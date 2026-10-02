use mollusk::{Mollusk, MOLLUSK_BUILTIN};
use solana_program::pubkey::Pubkey;
use solana_system_interface::program as system_program;

#[test]
fn system_program_is_builtin() {
    assert!(Mollusk::builtin(&system_program::ID));
    assert_eq!(MOLLUSK_BUILTIN, [system_program::ID]);
}

#[test]
fn unknown_key_is_not_builtin() {
    assert!(!Mollusk::builtin(&Pubkey::new_unique()));
}
