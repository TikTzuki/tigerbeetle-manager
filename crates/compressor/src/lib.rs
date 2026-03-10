//! Compress TigerBeetle data files by creating balance snapshots.
//!
//! This crate reads accounts from an existing TigerBeetle data file and
//! creates a minimal "compressed" representation in a fresh cluster — each
//! account gets at most 2 synthetic transfers that reconstruct its exact
//! `debits_posted` and `credits_posted` balances.
//!
//! # Quick start
//!
//! ```no_run
//! use tigerbeetle_manager_compressor::{BalancePlan, Importer};
//! use tb_reader::DataFileReader;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // 1. Read accounts from old file.
//! let mut reader = DataFileReader::open("old.tigerbeetle")?;
//! let accounts = reader.read_accounts()?;
//!
//! // 2. Build a compression plan.
//! let plan = BalancePlan::build(accounts);
//! println!("Plan: {} accounts → {} transfers", plan.total_accounts(), plan.total_transfers());
//!
//! // 3. Import into fresh cluster.
//! let importer = Importer::connect(0, "3000").await?;
//! importer.import_accounts(&plan).await?;
//! importer.import_transfers(&plan).await?;
//! # Ok(())
//! # }
//! ```

mod error;
mod importer;
mod plan;

pub use error::{CompressorError, Result};
pub use importer::Importer;
pub use plan::{AccountGroup, BalancePlan, SyntheticTransfer};
