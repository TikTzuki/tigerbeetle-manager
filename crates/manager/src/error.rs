//! Error types for the TigerBeetle manager.

use thiserror::Error;

/// Errors that can occur during TigerBeetle process management.
#[derive(Error, Debug)]
pub enum ManagerError {
    /// Error spawning or managing the child process.
    #[error("process error: {0}")]
    Process(String),

    /// Error during backup operation.
    #[error("backup error: {0}")]
    Backup(String),

    /// I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Compression error.
    #[error("compression error: {0}")]
    Compression(String),

    /// AWS S3 error.
    #[error("S3 error: {0}")]
    S3(String),
}

/// A specialized `Result` type for manager operations.
pub type Result<T> = std::result::Result<T, ManagerError>;
