//! Balance snapshot planning — generates synthetic transfers for compression.

use std::collections::HashMap;
use tb_reader::Account;

/// A group of accounts within a single ledger.
#[derive(Debug, Clone)]
pub struct AccountGroup {
    /// Ledger identifier.
    pub ledger: u32,
    /// All accounts in this ledger.
    pub accounts: Vec<Account>,
    /// GENESIS account ID for this ledger (reserved ID used as counterparty).
    pub genesis_id: u128,
}

/// A synthetic transfer that reconstructs an account's balance.
#[derive(Debug, Clone)]
pub struct SyntheticTransfer {
    /// Unique transfer ID.
    pub id: u128,
    /// Debit account ID.
    pub debit_account_id: u128,
    /// Credit account ID.
    pub credit_account_id: u128,
    /// Amount.
    pub amount: u128,
    /// Ledger.
    pub ledger: u32,
    /// Transfer code (inherited from account code).
    pub code: u16,
    /// Original account timestamp (for imported flag).
    pub timestamp: u64,
}

/// The complete compression plan — genesis accounts + synthetic transfers.
#[derive(Debug, Clone)]
pub struct BalancePlan {
    /// Genesis accounts (one per ledger) used as counterparties.
    pub genesis_accounts: Vec<Account>,
    /// All regular accounts to import (preserving original IDs and flags).
    pub regular_accounts: Vec<Account>,
    /// Synthetic transfers that reconstruct balances.
    pub synthetic_transfers: Vec<SyntheticTransfer>,
}

impl BalancePlan {
    /// Build a balance plan from a list of accounts.
    ///
    /// Groups accounts by ledger, creates genesis accounts, and generates
    /// synthetic transfers (≤ 2 per account: credit side + debit side).
    ///
    /// ## Timestamp strategy (TigerBeetle `imported` flag)
    ///
    /// All accounts and transfers are imported with the `imported` flag, which
    /// requires user-defined timestamps that are strictly increasing and in the
    /// past relative to the new cluster's clock.
    ///
    /// - **Genesis accounts**: timestamps `1, 2, …, K` (one per ledger, nanoseconds).
    /// - **Regular accounts**: sorted by original timestamp, then deduplicated so
    ///   each timestamp is strictly greater than the previous. The minimum is
    ///   `K + 1` (after all genesis accounts).
    /// - **Synthetic transfers**: sequential timestamps starting from
    ///   `max(regular_account_timestamps) + 1`.
    pub fn build(accounts: Vec<Account>) -> Self {
        // Group accounts by ledger.
        let mut groups_map: HashMap<u32, Vec<Account>> = HashMap::new();
        for account in accounts {
            groups_map.entry(account.ledger).or_default().push(account);
        }

        let mut groups: Vec<AccountGroup> = groups_map
            .into_iter()
            .map(|(ledger, accounts)| {
                // Genesis ID: u128::MAX - ledger (reserved range).
                let genesis_id = u128::MAX - u128::from(ledger);
                AccountGroup {
                    ledger,
                    accounts,
                    genesis_id,
                }
            })
            .collect();

        // Sort by ledger for deterministic output.
        groups.sort_by_key(|g| g.ledger);

        // Build genesis accounts with sequential timestamps starting at 1ns.
        // TigerBeetle requires imported timestamps > 0 and strictly increasing.
        let genesis_accounts: Vec<Account> = groups
            .iter()
            .enumerate()
            .map(|(i, group)| Account {
                id: group.genesis_id,
                ledger: group.ledger,
                code: 0,
                flags: tb_reader::AccountFlags::from(0),
                timestamp: (i as u64) + 1, // 1ns, 2ns, …, K ns
                debits_pending: 0,
                debits_posted: 0,
                credits_pending: 0,
                credits_posted: 0,
                user_data_128: 0,
                user_data_64: 0,
                user_data_32: 0,
                reserved: 0,
            })
            .collect();

        // The first regular account timestamp must be > last genesis timestamp.
        let genesis_max_ts = genesis_accounts.len() as u64; // == K

        // Flatten all regular accounts and sort by original timestamp.
        let mut regular_accounts: Vec<Account> = groups
            .iter()
            .flat_map(|g| g.accounts.iter().cloned())
            .collect();
        regular_accounts.sort_by_key(|a| a.timestamp);

        // Deduplicate timestamps: each must be strictly > previous.
        // Also ensure all are > genesis_max_ts.
        let mut prev_ts = genesis_max_ts;
        for acc in &mut regular_accounts {
            if acc.timestamp <= prev_ts {
                acc.timestamp = prev_ts + 1;
            }
            prev_ts = acc.timestamp;
        }

        let max_account_ts = regular_accounts
            .last()
            .map_or(genesis_max_ts, |a| a.timestamp);

        // Generate synthetic transfers.
        // Transfer timestamps must postdate both debit and credit account timestamps,
        // so we use sequential timestamps starting after the last account timestamp.
        let mut synthetic_transfers = Vec::new();
        let mut transfer_id_counter: u128 = 1; // Start from 1 (0 is reserved).
        let mut transfer_ts = max_account_ts;

        // Re-group by ledger for transfer generation (need genesis_id per ledger).
        let genesis_by_ledger: HashMap<u32, u128> =
            groups.iter().map(|g| (g.ledger, g.genesis_id)).collect();

        for acc in &regular_accounts {
            let genesis_id = genesis_by_ledger[&acc.ledger];

            // Credit side: if credits_posted > 0, create transfer:
            //   debit=GENESIS, credit=account, amount=credits_posted
            // This transfer happens FIRST to satisfy debits_must_not_exceed_credits.
            if acc.credits_posted > 0 {
                transfer_ts += 1;
                synthetic_transfers.push(SyntheticTransfer {
                    id: transfer_id_counter,
                    debit_account_id: genesis_id,
                    credit_account_id: acc.id,
                    amount: acc.credits_posted,
                    ledger: acc.ledger,
                    code: acc.code,
                    timestamp: transfer_ts,
                });
                transfer_id_counter += 1;
            }

            // Debit side: if debits_posted > 0, create transfer:
            //   debit=account, credit=GENESIS, amount=debits_posted
            if acc.debits_posted > 0 {
                transfer_ts += 1;
                synthetic_transfers.push(SyntheticTransfer {
                    id: transfer_id_counter,
                    debit_account_id: acc.id,
                    credit_account_id: genesis_id,
                    amount: acc.debits_posted,
                    ledger: acc.ledger,
                    code: acc.code,
                    timestamp: transfer_ts,
                });
                transfer_id_counter += 1;
            }
        }

        // Drop groups ownership (already consumed via iter above).
        drop(groups);

        BalancePlan {
            genesis_accounts,
            regular_accounts,
            synthetic_transfers,
        }
    }

    /// Total number of accounts to import (genesis + regular).
    pub fn total_accounts(&self) -> usize {
        self.genesis_accounts.len() + self.regular_accounts.len()
    }

    /// Total number of synthetic transfers.
    pub fn total_transfers(&self) -> usize {
        self.synthetic_transfers.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_balance_plan_empty() {
        let plan = BalancePlan::build(vec![]);
        assert_eq!(plan.genesis_accounts.len(), 0);
        assert_eq!(plan.regular_accounts.len(), 0);
        assert_eq!(plan.synthetic_transfers.len(), 0);
    }

    #[test]
    fn test_balance_plan_single_account() {
        let account = Account {
            id: 100,
            ledger: 1,
            code: 10,
            flags: tb_reader::AccountFlags::from(0),
            timestamp: 1000,
            debits_pending: 0,
            debits_posted: 50,
            credits_pending: 0,
            credits_posted: 200,
            user_data_128: 0,
            user_data_64: 0,
            user_data_32: 0,
            reserved: 0,
        };

        let plan = BalancePlan::build(vec![account]);

        // Should have 1 genesis account for ledger 1.
        assert_eq!(plan.genesis_accounts.len(), 1);
        assert_eq!(plan.genesis_accounts[0].ledger, 1);
        assert_eq!(plan.genesis_accounts[0].id, u128::MAX - 1);
        assert_eq!(plan.genesis_accounts[0].timestamp, 1); // 1ns

        // Should have 1 regular account.
        assert_eq!(plan.regular_accounts.len(), 1);
        assert_eq!(plan.regular_accounts[0].id, 100);
        assert_eq!(plan.regular_accounts[0].timestamp, 1000); // Original, > genesis(1)

        // Should have 2 synthetic transfers (credit + debit).
        assert_eq!(plan.synthetic_transfers.len(), 2);

        // First transfer: credit side (debit=GENESIS, credit=account, amount=200).
        let t0 = &plan.synthetic_transfers[0];
        assert_eq!(t0.debit_account_id, u128::MAX - 1);
        assert_eq!(t0.credit_account_id, 100);
        assert_eq!(t0.amount, 200);
        assert_eq!(t0.timestamp, 1001); // > max account ts (1000)

        // Second transfer: debit side (debit=account, credit=GENESIS, amount=50).
        let t1 = &plan.synthetic_transfers[1];
        assert_eq!(t1.debit_account_id, 100);
        assert_eq!(t1.credit_account_id, u128::MAX - 1);
        assert_eq!(t1.amount, 50);
        assert_eq!(t1.timestamp, 1002); // Strictly increasing
    }

    #[test]
    fn test_balance_plan_multiple_ledgers() {
        let accounts = vec![
            Account {
                id: 1,
                ledger: 1,
                code: 10,
                flags: tb_reader::AccountFlags::from(0),
                timestamp: 1000,
                debits_posted: 100,
                credits_posted: 0,
                debits_pending: 0,
                credits_pending: 0,
                user_data_128: 0,
                user_data_64: 0,
                user_data_32: 0,
                reserved: 0,
            },
            Account {
                id: 2,
                ledger: 2,
                code: 20,
                flags: tb_reader::AccountFlags::from(0),
                timestamp: 2000,
                debits_posted: 0,
                credits_posted: 500,
                debits_pending: 0,
                credits_pending: 0,
                user_data_128: 0,
                user_data_64: 0,
                user_data_32: 0,
                reserved: 0,
            },
        ];

        let plan = BalancePlan::build(accounts);

        // Should have 2 genesis accounts (one per ledger).
        assert_eq!(plan.genesis_accounts.len(), 2);
        assert_eq!(plan.genesis_accounts[0].timestamp, 1); // 1ns
        assert_eq!(plan.genesis_accounts[1].timestamp, 2); // 2ns

        // Should have 2 regular accounts, sorted by timestamp.
        assert_eq!(plan.regular_accounts.len(), 2);
        assert!(plan.regular_accounts[0].timestamp < plan.regular_accounts[1].timestamp);

        // Should have 2 synthetic transfers (1 per account).
        assert_eq!(plan.synthetic_transfers.len(), 2);
        // Transfer timestamps must be > max account timestamp (2000).
        assert!(plan.synthetic_transfers[0].timestamp > 2000);
        assert!(plan.synthetic_transfers[0].timestamp < plan.synthetic_transfers[1].timestamp);
    }

    #[test]
    fn test_balance_plan_deduplicates_timestamps() {
        // Two accounts with the SAME timestamp — must be deduplicated.
        let accounts = vec![
            Account {
                id: 1,
                ledger: 1,
                code: 10,
                flags: tb_reader::AccountFlags::from(0),
                timestamp: 500,
                debits_posted: 100,
                credits_posted: 0,
                debits_pending: 0,
                credits_pending: 0,
                user_data_128: 0,
                user_data_64: 0,
                user_data_32: 0,
                reserved: 0,
            },
            Account {
                id: 2,
                ledger: 1,
                code: 10,
                flags: tb_reader::AccountFlags::from(0),
                timestamp: 500, // Same timestamp!
                debits_posted: 0,
                credits_posted: 200,
                debits_pending: 0,
                credits_pending: 0,
                user_data_128: 0,
                user_data_64: 0,
                user_data_32: 0,
                reserved: 0,
            },
        ];

        let plan = BalancePlan::build(accounts);

        // Timestamps must be strictly increasing.
        assert_eq!(plan.regular_accounts.len(), 2);
        assert!(plan.regular_accounts[0].timestamp < plan.regular_accounts[1].timestamp);
        // Both must be > genesis timestamp (1ns for 1 ledger).
        assert!(plan.regular_accounts[0].timestamp > 1);
    }
}
