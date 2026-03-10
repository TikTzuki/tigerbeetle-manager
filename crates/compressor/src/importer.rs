//! TigerBeetle client wrapper for importing accounts and transfers.

use crate::error::{CompressorError, Result};
use crate::plan::{BalancePlan, SyntheticTransfer};
use tb_reader::Account as ReaderAccount;
use tigerbeetle_unofficial::{
    Account, Client, Transfer, account::Flags as AccountFlags, transfer::Flags as TransferFlags,
};

/// Maximum batch size for account/transfer creation.
///
/// TigerBeetle has a message size limit; batches must be split to stay under it.
/// Typical limit is ~8190 accounts or transfers per batch.
const BATCH_SIZE: usize = 8000;

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
    /// Creates genesis accounts first, then regular accounts, in batches.
    pub async fn import_accounts(&self, plan: &BalancePlan) -> Result<()> {
        // Import genesis accounts first (they must exist before transfers reference them).
        println!(
            "Importing {} genesis account(s)...",
            plan.genesis_accounts.len()
        );
        self.create_accounts_batch(&plan.genesis_accounts, false)
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

    Account::new(acc.id, acc.ledger, acc.code)
        .with_flags(flags)
        .with_user_data_128(acc.user_data_128)
        .with_user_data_64(acc.user_data_64)
        .with_user_data_32(acc.user_data_32)
}

/// Convert our SyntheticTransfer to TigerBeetle's Transfer type.
fn convert_transfer(t: &SyntheticTransfer) -> Transfer {
    Transfer::new(t.id)
        .with_debit_account_id(t.debit_account_id)
        .with_credit_account_id(t.credit_account_id)
        .with_amount(t.amount)
        .with_ledger(t.ledger)
        .with_code(t.code)
        .with_flags(TransferFlags::IMPORTED)
}
