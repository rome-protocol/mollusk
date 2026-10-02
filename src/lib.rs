pub mod error;

use {
    crate::error::{
        MolluskError::{
            Custom, ElfAccountNotFound, ProgramAccountNotFound, SimulateTransactionError,
        },
        Result,
    },
    mollusk_svm::Mollusk as MolluskSvm,
    mollusk_svm_result::types::{InstructionResult, ProgramResult},
    solana_account::Account,
    solana_bincode::limited_deserialize,
    solana_loader_v3_interface::state::UpgradeableLoaderState::{self, *},
    solana_program::{
        clock::Clock, instruction::Instruction, pubkey::Pubkey, rent::Rent, sysvar::Sysvar,
    },
    solana_sdk_ids::{bpf_loader, bpf_loader_deprecated, bpf_loader_upgradeable},
    solana_svm_log_collector::LogCollector,
    solana_system_interface::program as system_program,
    std::{
        cell::RefCell,
        collections::{HashMap, HashSet},
        iter::once,
        rc::Rc,
    },
};

pub const MOLLUSK_BUILTIN: [Pubkey; 1] = [system_program::ID];

/// Pick the Clock the SBF program observes: `Some(c)` pins the caller-supplied
/// clock (rome-sdk sizes a SENT leg at the next-block clock); `None` resolves
/// `Clock::get()`. Extracted so the selection is unit-testable without sysvar stubs.
fn resolve_clock(clock: Option<Clock>) -> Result<Clock> {
    match clock {
        Some(c) => Ok(c),
        None => Ok(Clock::get()?),
    }
}

thread_local! {
    static CACHED_SVM: RefCell<MolluskSvm> = RefCell::new(Mollusk::svm());
}

pub struct Mollusk<'a> {
    pub store: HashMap<Pubkey, Account>,
    pub upgradeable_elf: &'a HashMap<Pubkey, Account>,
}

impl<'a> Mollusk<'a> {
    pub fn svm() -> MolluskSvm {
        let mut mollusk = MolluskSvm::default();
        mollusk.compute_budget.heap_size = 256 * 1024;
        mollusk.compute_budget.compute_unit_limit = u64::MAX / 1000;
        mollusk.sysvars.rent = Rent::get().expect("rent expected");
        mollusk
    }
    pub fn new(
        store: HashMap<Pubkey, Account>,
        upgradeable_elf: &'a HashMap<Pubkey, Account>,
    ) -> Result<Self> {
        Ok(Self {
            store,
            upgradeable_elf,
        })
    }
    pub fn exec_keys(&self, ix: &Instruction) -> Result<HashSet<Pubkey>> {
        let mut set = HashSet::new();

        for meta in ix.accounts.iter() {
            let acc = self
                .store
                .get(&meta.pubkey)
                .ok_or(SimulateTransactionError(format!(
                    "expected account {}",
                    meta.pubkey
                )))?;

            if acc.executable {
                set.insert(meta.pubkey);
            }
        }
        set.insert(ix.program_id);

        Ok(set)
    }
    fn load_elf(&self, program: &Pubkey) -> Result<Vec<u8>> {
        let acc = &self
            .store
            .get(program)
            .ok_or(ProgramAccountNotFound(*program))?;

        if acc.owner == bpf_loader::id() || acc.owner == bpf_loader_deprecated::id() {
            return Ok(acc.data.clone());
        }

        if acc.owner != bpf_loader_upgradeable::id() {
            return Err(Custom(format!("{program} is not SBF program")));
        }

        match limited_deserialize(&acc.data, u64::MAX)? {
            Program {
                programdata_address: key,
            } => self.load_upgradeable_elf(&key),
            Buffer { .. } => {
                let offset = UpgradeableLoaderState::size_of_buffer_metadata();
                Ok(acc.data[offset..].to_vec())
            }
            _ => Err(Custom(format!(
                "{program} is not an upgradeable loader buffer or program account"
            ))),
        }
    }
    fn load_upgradeable_elf(&self, program: &Pubkey) -> Result<Vec<u8>> {
        let acc = self
            .upgradeable_elf
            .get(program)
            .ok_or(ElfAccountNotFound(*program))?;

        match limited_deserialize(&acc.data, u64::MAX)? {
            ProgramData { .. } => {
                let offset = UpgradeableLoaderState::size_of_programdata_metadata();
                Ok(acc.data[offset..].to_vec())
            }
            _ => Err(Custom(format!("Program {program} has been closed"))),
        }
    }
    pub fn execute_with_sysvar(
        &self,
        ix: &Instruction,
        logger: Option<Rc<RefCell<LogCollector>>>,
    ) -> Result<InstructionResult> {
        self.execute_with_sysvar_at(ix, logger, None)
    }

    /// Like `execute_with_sysvar`, but pins the Clock the SBF program observes:
    /// `Some(clock)` sets `svm.sysvars.clock` directly; `None` keeps the original
    /// `Clock::get()` behavior. Additive — `execute_with_sysvar` delegates with `None`.
    pub fn execute_with_sysvar_at(
        &self,
        ix: &Instruction,
        logger: Option<Rc<RefCell<LogCollector>>>,
        clock: Option<Clock>,
    ) -> Result<InstructionResult> {
        let accounts = ix
            .accounts
            .iter()
            .map(|a| a.pubkey)
            .chain(once(ix.program_id))
            .map(|pubkey| {
                let account = self
                    .store
                    .get(&pubkey)
                    .ok_or_else(|| SimulateTransactionError(format!("expected account {pubkey}")))?
                    .clone();
                Ok((pubkey, account))
            })
            .collect::<Result<HashMap<Pubkey, Account>>>()?
            .into_iter()
            .collect::<Vec<_>>();

        let keys = self.exec_keys(ix)?;

        CACHED_SVM.with(|cell| -> Result<InstructionResult> {
            let mut svm = cell.borrow_mut();
            svm.logger = logger;
            svm.sysvars.clock = resolve_clock(clock)?;

            for program in keys {
                if Mollusk::builtin(&program) {
                    continue;
                }
                let cached = svm.program_cache.load_program(&program).is_some();
                if !cached {
                    let elf = self.load_elf(&program)?;
                    svm.add_program_with_loader_and_elf(
                        &program,
                        &bpf_loader_upgradeable::id(),
                        &elf,
                    );
                }
            }
            Ok(svm.process_instruction(ix, &accounts))
        })
    }
    pub fn builtin(key: &Pubkey) -> bool {
        MOLLUSK_BUILTIN.contains(key)
    }
    pub fn is_success(res: &InstructionResult) -> bool {
        matches!(res.program_result, ProgramResult::Success)
    }
}

#[cfg(test)]
mod clock_inject_tests {
    use super::*;

    // `Some` returns the injected clock verbatim without calling `Clock::get()`,
    // so it holds with no sysvar stubs installed.
    #[test]
    fn some_injects_the_supplied_clock() {
        let c = Clock {
            unix_timestamp: 1_777_000_123,
            slot: 42,
            ..Default::default()
        };

        let got = resolve_clock(Some(c.clone())).expect("Some path must not error");
        assert_eq!(got.unix_timestamp, c.unix_timestamp);
        assert_eq!(got.slot, c.slot);
    }

    // `None` routes through `Clock::get()`, which fails closed with no stubs
    // installed — so an Err here proves the fallback path, not an injected value.
    // (E2E "SBF reads the injected clock" is the live same-second race gate.)
    #[test]
    fn none_falls_back_to_clock_get() {
        assert!(
            resolve_clock(None).is_err(),
            "None path must call Clock::get(), which errors with no stubs installed"
        );
    }
}

#[cfg(test)]
mod account_store_tests {
    use super::*;
    use solana_program::instruction::AccountMeta;
    use std::panic::{catch_unwind, AssertUnwindSafe};

    #[test]
    fn missing_instruction_account_is_reported_not_panicked() {
        let present = Pubkey::new_unique();
        let missing = Pubkey::new_unique();
        let ix = Instruction {
            program_id: system_program::ID,
            accounts: vec![
                AccountMeta::new_readonly(present, false),
                AccountMeta::new_readonly(missing, false),
            ],
            data: vec![],
        };
        let mut store = HashMap::new();
        store.insert(present, Account::default());
        store.insert(system_program::ID, Account::default());
        let elfs = HashMap::new();
        let mollusk = Mollusk::new(store, &elfs).expect("construct mollusk");

        let outcome = catch_unwind(AssertUnwindSafe(|| {
            mollusk.execute_with_sysvar_at(&ix, None, Some(Clock::default()))
        }));

        let result = outcome.expect("missing account must not panic");
        let error = result.expect_err("missing account must fail composition");
        assert!(
            error.to_string().contains(&missing.to_string()),
            "error must identify the missing account: {error}"
        );
    }
}
