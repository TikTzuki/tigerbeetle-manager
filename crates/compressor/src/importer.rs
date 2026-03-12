//! TigerBeetle client wrapper for importing accounts and transfers.

use crate::error::{CompressorError, Result};
use crate::plan::{BalancePlan, SyntheticTransfer};
use tb_reader::Account as ReaderAccount;
use tigerbeetle_unofficial::{
    Account, Client, Transfer, account::Flags as AccountFlags, transfer::Flags as TransferFlags,
};
use tokio::sync::mpsc;
use tracing::field::debug;
use tracing::{debug, info};

/// Maximum batch size for account/transfer creation.
///
/// TigerBeetle has a message size limit; batches must be split to stay under it.
/// Typical limit is ~8190 accounts or transfers per batch.
const BATCH_SIZE: usize = 8000;

/// Progress update emitted during import.
#[derive(Debug, Clone)]
pub struct ImportProgress {
    /// Current phase: `"genesis_accounts"`, `"accounts"`, or `"transfers"`.
    pub phase: String,
    /// Records imported so far in this phase.
    pub imported: u64,
    /// Total records to import in this phase.
    pub total: u64,
}

/// Wraps a TigerBeetle client and provides high-level import operations.
#[allow(missing_debug_implementations)]
pub struct Importer {
    client: Client,
}

impl Importer {
    /// Connect to a TigerBeetle cluster.
    ///
    /// # Arguments
    /// - `cluster_id`: The cluster identifier (0 for single-cluster setups).
    /// - `replica_addresses`: Comma-separated list of replica addresses (e.g., `"3000"` or `"3000,3001,3002"`).
    pub async fn connect(cluster_id: u128, replica_addresses: &str) -> Result<Self> {
        let client = Client::new(cluster_id, replica_addresses)
            .map_err(|e| CompressorError::Client(format!("failed to connect: {e:?}")))?;
        Ok(Importer { client })
    }

    /// Import all accounts from a balance plan.
    ///
    /// Creates genesis accounts first (with `imported` flag, timestamps `1..K`),
    /// then regular accounts (with `imported` flag, original timestamps), in batches.
    ///
    /// All accounts use the `imported` flag so that timestamps are strictly
    /// increasing and controlled by the importer rather than the cluster clock.
    pub async fn import_accounts(&self, plan: &BalancePlan) -> Result<()> {
        // Import genesis accounts first (they must exist before transfers reference them).
        println!(
            "Importing {} genesis account(s)...",
            plan.genesis_accounts.len()
        );
        self.create_accounts_batch(&plan.genesis_accounts, true)
            .await?;

        // Import regular accounts (preserve IDs and timestamps with `imported` flag).
        println!(
            "Importing {} regular account(s)...",
            plan.regular_accounts.len()
        );
        self.create_accounts_batch(&plan.regular_accounts, true)
            .await?;

        Ok(())
    }

    /// Import all synthetic transfers from a balance plan.
    ///
    /// Transfers are created in batches, preserving timestamp order.
    pub async fn import_transfers(&self, plan: &BalancePlan) -> Result<()> {
        println!(
            "Importing {} synthetic transfer(s)...",
            plan.synthetic_transfers.len()
        );
        self.create_transfers_batch(&plan.synthetic_transfers)
            .await?;
        Ok(())
    }

    /// Import the entire balance plan, streaming progress updates via `tx`.
    ///
    /// Phases (in order): `"genesis_accounts"`, `"accounts"`, `"transfers"`.
    /// After each batch, an [`ImportProgress`] is sent. If the receiver is
    /// dropped, the import continues silently (progress is best-effort).
    pub async fn import_all_with_progress(
        &self,
        plan: &BalancePlan,
        tx: mpsc::Sender<ImportProgress>,
    ) -> Result<()> {
        // Phase 1: genesis accounts.
        let genesis_total = plan.genesis_accounts.len() as u64;
        let mut genesis_imported = 0u64;
        debug!("Starting import of genesis accounts: total={genesis_total}");
        for chunk in plan.genesis_accounts.chunks(BATCH_SIZE) {
            let tb_accounts: Vec<Account> =
                chunk.iter().map(|acc| convert_account(acc, true)).collect();
            if let Err(e) = self.client.create_accounts(tb_accounts).await {
                tracing::error!("Error creating genesis accounts batch: {:?}", e);
                return Err(CompressorError::AccountCreationFailed(chunk.len()));
            }
            genesis_imported += chunk.len() as u64;
            let _ = tx
                .send(ImportProgress {
                    phase: "genesis_accounts".into(),
                    imported: genesis_imported,
                    total: genesis_total,
                })
                .await;
        }

        debug!(
            "Finished importing genesis accounts, starting regular accounts: total={}",
            plan.regular_accounts.len()
        );
        // Phase 2: regular accounts.
        let accounts_total = plan.regular_accounts.len() as u64;
        let mut accounts_imported = 0u64;
        for chunk in plan.regular_accounts.chunks(BATCH_SIZE) {
            let tb_accounts: Vec<Account> =
                chunk.iter().map(|acc| convert_account(acc, true)).collect();
            if let Err(e) = self.client.create_accounts(tb_accounts).await {
                tracing::error!("Error creating regular accounts batch: {:?}", e);
                return Err(CompressorError::AccountCreationFailed(chunk.len()));
            }
            accounts_imported += chunk.len() as u64;
            let _ = tx
                .send(ImportProgress {
                    phase: "accounts".into(),
                    imported: accounts_imported,
                    total: accounts_total,
                })
                .await;
        }

        // Phase 3: synthetic transfers.
        debug!(
            "Finished importing accounts, starting synthetic transfers: total={}",
            plan.synthetic_transfers.len()
        );
        let transfers_total = plan.synthetic_transfers.len() as u64;
        let mut transfers_imported = 0u64;
        for chunk in plan.synthetic_transfers.chunks(BATCH_SIZE) {
            let tb_transfers: Vec<Transfer> = chunk.iter().map(convert_transfer).collect();
            if let Err(e) = self.client.create_transfers(tb_transfers).await {
                tracing::error!("Error creating transfers batch: {:?}", e);
                return Err(CompressorError::TransferCreationFailed(chunk.len()));
            }
            transfers_imported += chunk.len() as u64;
            let _ = tx
                .send(ImportProgress {
                    phase: "transfers".into(),
                    imported: transfers_imported,
                    total: transfers_total,
                })
                .await;
        }

        Ok(())
    }

    /// Create accounts in batches, handling TigerBeetle's batch size limit.
    async fn create_accounts_batch(
        &self,
        accounts: &[ReaderAccount],
        imported: bool,
    ) -> Result<()> {
        for (batch_idx, chunk) in accounts.chunks(BATCH_SIZE).enumerate() {
            let tb_accounts: Vec<Account> = chunk
                .iter()
                .map(|acc| convert_account(acc, imported))
                .collect();

            if let Err(e) = self.client.create_accounts(tb_accounts).await {
                eprintln!("Error creating accounts batch {}: {:?}", batch_idx, e);
                return Err(CompressorError::AccountCreationFailed(chunk.len()));
            }
        }
        Ok(())
    }

    /// Create transfers in batches.
    async fn create_transfers_batch(&self, transfers: &[SyntheticTransfer]) -> Result<()> {
        for (batch_idx, chunk) in transfers.chunks(BATCH_SIZE).enumerate() {
            let tb_transfers: Vec<Transfer> = chunk.iter().map(convert_transfer).collect();

            if let Err(e) = self.client.create_transfers(tb_transfers).await {
                eprintln!("Error creating transfers batch {}: {:?}", batch_idx, e);
                return Err(CompressorError::TransferCreationFailed(chunk.len()));
            }
        }
        Ok(())
    }
}

/// Convert our Account type to TigerBeetle's Account type.
///
/// When `imported` is true, sets `AccountFlags::IMPORTED` and copies the
/// account's timestamp into the raw struct (TigerBeetle requires non-zero,
/// strictly increasing timestamps for imported accounts).
fn convert_account(acc: &ReaderAccount, imported: bool) -> Account {
    let mut flags = AccountFlags::empty();
    if imported {
        flags |= AccountFlags::IMPORTED;
    }
    // Preserve original account flags.
    if acc.flags.linked() {
        flags |= AccountFlags::LINKED;
    }
    if acc.flags.debits_must_not_exceed_credits() {
        flags |= AccountFlags::DEBITS_MUST_NOT_EXCEED_CREDITS;
    }
    if acc.flags.credits_must_not_exceed_debits() {
        flags |= AccountFlags::CREDITS_MUST_NOT_EXCEED_DEBITS;
    }
    if acc.flags.history() {
        flags |= AccountFlags::HISTORY;
    }
    if acc.flags.closed() {
        flags |= AccountFlags::CLOSED;
    }

    info!(
        "Converting account {} with flags {:?} {:?} (imported={})",
        acc.id, acc.code, flags, imported
    );
    let mut account = Account::new(acc.id, acc.ledger, acc.code)
        .with_flags(flags)
        .with_user_data_128(acc.user_data_128)
        .with_user_data_64(acc.user_data_64)
        .with_user_data_32(acc.user_data_32);

    if imported {
        account.as_raw_mut().timestamp = acc.timestamp;
    }

    account
}

/// Convert our SyntheticTransfer to TigerBeetle's Transfer type.
///
/// All synthetic transfers use the `imported` flag with an explicit timestamp
/// that postdates both debit and credit account timestamps.
fn convert_transfer(t: &SyntheticTransfer) -> Transfer {
    let mut transfer = Transfer::new(t.id)
        .with_debit_account_id(t.debit_account_id)
        .with_credit_account_id(t.credit_account_id)
        .with_amount(t.amount)
        .with_ledger(t.ledger)
        .with_code(t.code)
        .with_flags(TransferFlags::IMPORTED);

    transfer.as_raw_mut().timestamp = t.timestamp;

    transfer
}
