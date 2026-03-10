//! Error types for the compressor.

use thiserror::Error;

/// Errors that can occur during compression and import.
#[derive(Error, Debug)]
pub enum CompressorError {
    /// Error reading from the source data file.
    #[error("reader error: {0}")]
    Reader(#[from] tb_reader::ReaderError),

    /// Error communicating with TigerBeetle cluster.
    #[error("TigerBeetle client error: {0}")]
    Client(String),

    /// Account creation failed for some accounts.
    #[error("failed to create {0} account(s)")]
    AccountCreationFailed(usize),

    /// Transfer creation failed for some transfers.
    #[error("failed to create {0} transfer(s)")]
    TransferCreationFailed(usize),

    /// Invalid configuration or plan.
    #[error("invalid plan: {0}")]
    InvalidPlan(String),
}

/// A specialized `Result` type for compressor operations.
pub type Result<T> = std::result::Result<T, CompressorError>;
