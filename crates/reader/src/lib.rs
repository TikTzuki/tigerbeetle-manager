//! Read account and transfer records directly from TigerBeetle binary data files.
//!
//! This crate parses TigerBeetle's on-disk format without connecting to a
//! live cluster. It reads both the **checkpointed LSM** state and the
//! **pre-checkpoint WAL** entries.
//!
//! # LSM accounts (checkpointed)
//!
//! ```no_run
//! use tigerbeetle_manager_reader::DataFileReader;
//!
//! let mut reader = DataFileReader::open("cluster_0_replica_0.tigerbeetle")?;
//! for result in reader.iter_accounts()? {
//!     let account = result?;
//!     println!("LSM id={} ledger={} credits_posted={}", account.id, account.ledger, account.credits_posted);
//! }
//! # Ok::<(), tigerbeetle_manager_reader::ReaderError>(())
//! ```
//!
//! # WAL accounts (pre-checkpoint)
//!
//! ```no_run
//! use tigerbeetle_manager_reader::DataFileReader;
//!
//! let mut reader = DataFileReader::open("cluster_0_replica_0.tigerbeetle")?;
//! for result in reader.iter_wal_accounts()? {
//!     let account = result?;
//!     println!("WAL id={} ledger={}", account.id, account.ledger);
//! }
//! # Ok::<(), tigerbeetle_manager_reader::ReaderError>(())
//! ```

mod block;
mod error;
mod layout;
mod manifest;
mod reader;
mod superblock;
mod types;

pub use error::ReaderError;
pub use layout::TBConfig;
pub use reader::{AccountIter, DataFileReader, TransferIter, WalAccountIter, WalTransferIter};
pub use types::{Account, AccountFlags, Transfer, TransferFlags};
