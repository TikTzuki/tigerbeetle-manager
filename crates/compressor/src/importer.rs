//! TigerBeetle client wrapper for importing accounts and transfers.

use crate::error::{CompressorError, Result};
use crate::plan::{BalancePlan, SyntheticTransfer};
use tb_reader::Account as ReaderAccount;
use tb_reader::Transfer as ReaderTransfer;
use tigerbeetle_client::{
    Account, AccountFlags, Client, Transfer, TransferFlags,
};
use tokio::sync::mpsc;
use tracing::info;

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
            .map_err(|e| CompressorError::Client(format!("failed to connect: {e}")))?;

        // Probe connectivity with a 30s timeout. Client::new() is lazy —
        // a bad address won't error until the first real request.
        let probe = client.lookup_accounts(&[0u128]);
        match tokio::time::timeout(std::time::Duration::from_secs(30), probe).await {
            Ok(Ok(_)) => {} // connected (account not found is fine)
            Ok(Err(e)) => {
                return Err(CompressorError::Client(format!(
                    "target cluster rejected probe request: {e}"
                )));
            }
            Err(_) => {
                return Err(CompressorError::Client(
                    "connection to target cluster timed out after 30s".into(),
                ));
            }
        }

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
            plan.synthetic_transfers.len(),
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
        info!("Starting import of genesis accounts: total={genesis_total}");
        for chunk in plan.genesis_accounts.chunks(BATCH_SIZE) {
            let tb_accounts: Vec<Account> =
                chunk.iter().map(|acc| convert_account(acc, true)).collect();
            let results = self.client.create_accounts(&tb_accounts).await
                .map_err(|e| CompressorError::Client(format!("create_accounts: {e}")))?;
            if !results.is_empty() {
                tracing::error!("Error creating genesis accounts batch: {:?}", results);
                return Err(CompressorError::AccountCreationFailed(results.len()));
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

        info!(
            "Finished importing genesis accounts, starting regular accounts: total={}",
            plan.regular_accounts.len()
        );
        // Phase 2: regular accounts.
        let accounts_total = plan.regular_accounts.len() as u64;
        let mut accounts_imported = 0u64;
        for chunk in plan.regular_accounts.chunks(BATCH_SIZE) {
            let tb_accounts: Vec<Account> =
                chunk.iter().map(|acc| convert_account(acc, true)).collect();
            let results = self.client.create_accounts(&tb_accounts).await
                .map_err(|e| CompressorError::Client(format!("create_accounts: {e}")))?;
            if !results.is_empty() {
                tracing::error!("Error creating regular accounts batch: {:?}", results);
                return Err(CompressorError::AccountCreationFailed(results.len()));
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
        info!(
            "Finished importing accounts, starting synthetic transfers: total={}",
            plan.synthetic_transfers.len(),
        );
        let transfers_total = plan.synthetic_transfers.len() as u64;
        let mut transfers_imported = 0u64;
        for chunk in plan.synthetic_transfers.chunks(BATCH_SIZE) {
            let tb_transfers: Vec<Transfer> =
                chunk.iter().map(|t| convert_transfer(t)).collect();
            let results = self.client.create_transfers(&tb_transfers).await
                .map_err(|e| CompressorError::Client(format!("create_transfers: {e}")))?;
            if !results.is_empty() {
                tracing::error!("Error creating synthetic transfers batch: {:?}", results);
                return Err(CompressorError::TransferCreationFailed(results.len()));
            }
            transfers_imported += chunk.len() as u64;
            let _ = tx
                .send(ImportProgress {
                    phase: "synthetic_transfers".into(),
                    imported: transfers_imported,
                    total: transfers_total,
                })
                .await;
        }

        // Phase 4: windowed transfers (actual transfers from the time window).
        // Only present for time-window migrations (cutoff_ts > 0).
        if !plan.windowed_transfers.is_empty() {
            info!(
                "Finished importing synthetic transfers, starting windowed transfers: total={}",
                plan.windowed_transfers.len()
            );
            let windowed_total = plan.windowed_transfers.len() as u64;
            let mut windowed_imported = 0u64;
            for chunk in plan.windowed_transfers.chunks(BATCH_SIZE) {
                let tb_transfers: Vec<Transfer> =
                    chunk.iter().map(|t| convert_windowed_transfer(t)).collect();
                let results = self.client.create_transfers(&tb_transfers).await
                    .map_err(|e| CompressorError::Client(format!("create_transfers: {e}")))?;
                if !results.is_empty() {
                    tracing::error!("Error creating windowed transfers batch: {:?}", results);
                    return Err(CompressorError::TransferCreationFailed(results.len()));
                }
                windowed_imported += chunk.len() as u64;
                let _ = tx
                    .send(ImportProgress {
                        phase: "windowed_transfers".into(),
                        imported: windowed_imported,
                        total: windowed_total,
                    })
                    .await;
            }
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

            let results = self.client.create_accounts(&tb_accounts).await
                .map_err(|e| CompressorError::Client(format!("create_accounts: {e}")))?;
            if !results.is_empty() {
                eprintln!("Error creating accounts batch {}: {:?}", batch_idx, results);
                return Err(CompressorError::AccountCreationFailed(results.len()));
            }
        }
        Ok(())
    }

    /// Create transfers in batches.
    async fn create_transfers_batch(
        &self,
        transfers: &[SyntheticTransfer],
    ) -> Result<()> {
        for (batch_idx, chunk) in transfers.chunks(BATCH_SIZE).enumerate() {
            let tb_transfers: Vec<Transfer> =
                chunk.iter().map(|t| convert_transfer(t)).collect();

            let results = self.client.create_transfers(&tb_transfers).await
                .map_err(|e| CompressorError::Client(format!("create_transfers: {e}")))?;
            if !results.is_empty() {
                eprintln!("Error creating transfers batch {}: {:?}", batch_idx, results);
                return Err(CompressorError::TransferCreationFailed(results.len()));
            }
        }
        Ok(())
    }
}

/// Convert our Account type to TigerBeetle's Account type.
///
/// When `imported` is true, sets `AccountFlags::Imported` and copies the
/// account's timestamp into the raw struct (TigerBeetle requires non-zero,
/// strictly increasing timestamps for imported accounts).
fn convert_account(acc: &ReaderAccount, imported: bool) -> Account {
    let mut flags = AccountFlags::empty();
    if imported {
        flags |= AccountFlags::Imported;
    }
    // Preserve original account flags.
    if acc.flags.linked() {
        flags |= AccountFlags::Linked;
    }
    if acc.flags.debits_must_not_exceed_credits() {
        flags |= AccountFlags::DebitsMustNotExceedCredits;
    }
    if acc.flags.credits_must_not_exceed_debits() {
        flags |= AccountFlags::CreditsMustNotExceedDebits;
    }
    if acc.flags.history() {
        flags |= AccountFlags::History;
    }
    if acc.flags.closed() {
        flags |= AccountFlags::Closed;
    }

    Account {
        id: acc.id,
        ledger: acc.ledger,
        code: acc.code,
        flags,
        user_data_128: acc.user_data_128,
        user_data_64: acc.user_data_64,
        user_data_32: acc.user_data_32,
        timestamp: if imported { acc.timestamp } else { 0 },
        ..Default::default()
    }
}

/// Convert our SyntheticTransfer to TigerBeetle's Transfer type.
///
/// All synthetic transfers use the `imported` flag with an explicit timestamp
/// that postdates both debit and credit account timestamps.
fn convert_transfer(t: &SyntheticTransfer) -> Transfer {
    Transfer {
        id: t.id,
        debit_account_id: t.debit_account_id,
        credit_account_id: t.credit_account_id,
        amount: t.amount,
        ledger: t.ledger,
        code: t.code,
        flags: TransferFlags::Imported,
        timestamp: t.timestamp,
        ..Default::default()
    }
}

/// Convert a reader Transfer to TigerBeetle's Transfer type for windowed replay.
///
/// Preserves all original fields (ID, accounts, amount, ledger, code, pending_id,
/// user data, timeout) and all original flags, adding the `imported` flag so the
/// caller can set an explicit timestamp.
fn convert_windowed_transfer(t: &ReaderTransfer) -> Transfer {
    let mut flags = TransferFlags::Imported;
    // Preserve original transfer flags.
    if t.flags.linked() {
        flags |= TransferFlags::Linked;
    }
    if t.flags.pending() {
        flags |= TransferFlags::Pending;
    }
    if t.flags.post_pending_transfer() {
        flags |= TransferFlags::PostPendingTransfer;
    }
    if t.flags.void_pending_transfer() {
        flags |= TransferFlags::VoidPendingTransfer;
    }
    if t.flags.balancing_debit() {
        flags |= TransferFlags::BalancingDebit;
    }
    if t.flags.balancing_credit() {
        flags |= TransferFlags::BalancingCredit;
    }
    if t.flags.closing_debit() {
        flags |= TransferFlags::ClosingDebit;
    }
    if t.flags.closing_credit() {
        flags |= TransferFlags::ClosingCredit;
    }

    Transfer {
        id: t.id,
        debit_account_id: t.debit_account_id,
        credit_account_id: t.credit_account_id,
        amount: t.amount,
        pending_id: t.pending_id,
        user_data_128: t.user_data_128,
        user_data_64: t.user_data_64,
        user_data_32: t.user_data_32,
        timeout: t.timeout,
        ledger: t.ledger,
        code: t.code,
        flags,
        timestamp: t.timestamp,
    }
}
