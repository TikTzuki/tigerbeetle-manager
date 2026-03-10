//! Error type for the reader crate.

/// Errors that can occur while reading a TigerBeetle data file.
#[derive(Debug, thiserror::Error)]
pub enum ReaderError {
    /// An I/O error occurred while reading the file.
    #[error("I/O error: {0}")]
    Io(String),

    /// No valid superblock was found (file is not a TigerBeetle data file,
    /// or it has never been formatted / written to).
    #[error("invalid superblock: {0}")]
    InvalidSuperblock(String),

    /// A grid block failed structural validation.
    #[error("invalid block: {0}")]
    InvalidBlock(String),

    /// The cluster has started (superblock sequence > 0) but has never
    /// triggered a checkpoint, so no LSM data exists on disk yet.
    ///
    /// TigerBeetle only checkpoints after `vsr_checkpoint_ops` committed
    /// operations (≈960 for the default production config). Data committed
    /// before that lives only in memory/WAL and is NOT visible to the reader.
    #[error(
        "no checkpoint yet (superblock sequence={sequence}): \
         the cluster has started but has not committed enough operations \
         to trigger its first LSM checkpoint (~960 ops for production config); \
         query the live cluster via the TigerBeetle client instead"
    )]
    NotCheckpointed {
        /// The superblock sequence number (confirms the file is active).
        sequence: u64,
    },
}

impl From<std::io::Error> for ReaderError {
    fn from(e: std::io::Error) -> Self {
        ReaderError::Io(e.to_string())
    }
}
