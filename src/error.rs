use {
    solana_program::{instruction::InstructionError, program_error::ProgramError, pubkey::Pubkey},
    thiserror::Error,
};

pub type Result<T> = std::result::Result<T, MolluskError>;

#[derive(Error, Debug)]
pub enum MolluskError {
    #[error("simulate transaction error: {0}")]
    SimulateTransactionError(String),

    #[error("program account not found: {0}")]
    ProgramAccountNotFound(Pubkey),

    #[error("elf account not found: {0}")]
    ElfAccountNotFound(Pubkey),

    #[error("{0}")]
    Custom(String),

    #[error(transparent)]
    Program(#[from] ProgramError),

    #[error(transparent)]
    Instruction(#[from] InstructionError),
}
