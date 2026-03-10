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
    /// synthetic transfers (2 per account: credit side + debit side).
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

        // Build genesis accounts.
        let genesis_accounts: Vec<Account> = groups
            .iter()
            .map(|group| Account {
                id: group.genesis_id,
                ledger: group.ledger,
                code: 0,                                 // Genesis account code (arbitrary).
                flags: tb_reader::AccountFlags::from(0), // No constraints.
                timestamp: 0,                            // Earliest possible timestamp.
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

        // Generate synthetic transfers.
        let mut synthetic_transfers = Vec::new();
        let mut transfer_id_counter: u128 = 1; // Start from 1 (0 is reserved).

        for group in &groups {
            for account in &group.accounts {
                // Credit side: if credits_posted > 0, create transfer:
                //   debit=GENESIS, credit=account, amount=credits_posted
                // This transfer happens FIRST to satisfy debits_must_not_exceed_credits.
                if account.credits_posted > 0 {
                    synthetic_transfers.push(SyntheticTransfer {
                        id: transfer_id_counter,
                        debit_account_id: group.genesis_id,
                        credit_account_id: account.id,
                        amount: account.credits_posted,
                        ledger: account.ledger,
                        code: account.code, // Inherit account's code.
                        timestamp: account.timestamp,
                    });
                    transfer_id_counter += 1;
                }

                // Debit side: if debits_posted > 0, create transfer:
                //   debit=account, credit=GENESIS, amount=debits_posted
                if account.debits_posted > 0 {
                    synthetic_transfers.push(SyntheticTransfer {
                        id: transfer_id_counter,
                        debit_account_id: account.id,
                        credit_account_id: group.genesis_id,
                        amount: account.debits_posted,
                        ledger: account.ledger,
                        code: account.code,
                        timestamp: account.timestamp,
                    });
                    transfer_id_counter += 1;
                }
            }
        }

        // Collect all regular accounts (flatten groups).
        let regular_accounts: Vec<Account> = groups.into_iter().flat_map(|g| g.accounts).collect();

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

        // Should have 1 regular account.
        assert_eq!(plan.regular_accounts.len(), 1);
        assert_eq!(plan.regular_accounts[0].id, 100);

        // Should have 2 synthetic transfers (credit + debit).
        assert_eq!(plan.synthetic_transfers.len(), 2);

        // First transfer: credit side (debit=GENESIS, credit=account, amount=200).
        let t0 = &plan.synthetic_transfers[0];
        assert_eq!(t0.debit_account_id, u128::MAX - 1);
        assert_eq!(t0.credit_account_id, 100);
        assert_eq!(t0.amount, 200);

        // Second transfer: debit side (debit=account, credit=GENESIS, amount=50).
        let t1 = &plan.synthetic_transfers[1];
        assert_eq!(t1.debit_account_id, 100);
        assert_eq!(t1.credit_account_id, u128::MAX - 1);
        assert_eq!(t1.amount, 50);
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

        // Should have 2 regular accounts.
        assert_eq!(plan.regular_accounts.len(), 2);

        // Should have 2 synthetic transfers (1 per account, since each has only debit OR credit).
        assert_eq!(plan.synthetic_transfers.len(), 2);
    }
}
