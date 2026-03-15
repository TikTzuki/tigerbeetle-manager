//! Balance snapshot planning — generates synthetic transfers for compression.

use std::collections::HashMap;
use tb_reader::{Account, Transfer};

/// A group of accounts within a single ledger.
#[derive(Debug, Clone)]
pub struct AccountGroup {
    /// Ledger identifier.
    pub ledger: u32,
    /// All accounts in this ledger.
    pub accounts: Vec<Account>,
    /// Genesis credit account ID — the debit counterparty for credit-side transfers.
    ///
    /// After migration: `genesis_credit.debits_posted = Σ(acc.credits_posted)`.
    pub genesis_credit_id: u128,
    /// Genesis debit account ID — the credit counterparty for debit-side transfers.
    ///
    /// After migration: `genesis_debit.credits_posted = Σ(acc.debits_posted)`.
    /// Invariant: `genesis_credit.debits_posted == genesis_debit.credits_posted`
    /// iff the ledger is balanced (total credits == total debits).
    pub genesis_debit_id: u128,
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
    /// Genesis accounts (two per ledger): credit genesis + debit genesis.
    ///
    /// Credit genesis: debit counterparty for credit-side transfers.
    /// Debit genesis: credit counterparty for debit-side transfers.
    /// After migration, `credit_genesis.debits_posted == debit_genesis.credits_posted`
    /// iff the ledger is balanced.
    pub genesis_accounts: Vec<Account>,
    /// All regular accounts to import (preserving original IDs and flags).
    pub regular_accounts: Vec<Account>,
    /// Synthetic transfers that reconstruct balances.
    pub synthetic_transfers: Vec<SyntheticTransfer>,
    /// Actual transfers from the time window `[cutoff_ts, now]` to replay verbatim.
    ///
    /// Empty for pure snapshot migrations (`cutoff_ts = 0`). For windowed migrations,
    /// these are replayed after synthetic transfers to restore the full final balance.
    pub windowed_transfers: Vec<Transfer>,
}

impl BalancePlan {
    /// Build a pure balance snapshot plan from a list of accounts.
    ///
    /// Groups accounts by ledger, creates genesis accounts, and generates
    /// synthetic transfers (≤ 2 per account: credit side + debit side).
    /// No windowed transfer replay — `windowed_transfers` will be empty.
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
        Self::build_snapshot_impl(accounts, false)
    }

    /// Build a time-window migration plan.
    ///
    /// Accounts are adjusted so their balances reflect the state at `cutoff_ts`:
    /// - Accounts created **before** `cutoff_ts`: balances are reduced by the sum
    ///   of windowed transfer amounts they participated in.
    /// - Accounts created **during** the window (`timestamp >= cutoff_ts`): balances
    ///   are zeroed out (they didn't exist at cutoff; windowed transfer replay will
    ///   restore them).
    ///
    /// The synthetic transfers reconstruct the adjusted (at-cutoff) balances.
    /// The windowed transfers are stored verbatim and replayed in Phase 4, restoring
    /// the full final balance.
    ///
    /// ## Timestamp ordering guarantee
    ///
    /// ```text
    /// max_active_ts  <  synthetic_ts range  <  cutoff_ts  ≤  windowed_transfer_ts
    /// ```
    ///
    /// Where `max_active_ts` is the maximum timestamp among accounts with a non-zero
    /// balance at `cutoff_ts` (all such accounts have `timestamp < cutoff_ts`).
    pub fn build_windowed(
        accounts: Vec<Account>,
        windowed_transfers: Vec<Transfer>,
        cutoff_ts: u64,
    ) -> Self {
        // Compute per-account debit/credit deltas from windowed transfers.
        let mut debit_delta: HashMap<u128, u128> = HashMap::new();
        let mut credit_delta: HashMap<u128, u128> = HashMap::new();
        for t in &windowed_transfers {
            *debit_delta.entry(t.debit_account_id).or_insert(0) += t.amount;
            *credit_delta.entry(t.credit_account_id).or_insert(0) += t.amount;
        }

        // Adjust each account's balance to reflect state at cutoff_ts.
        let adjusted: Vec<Account> = accounts
            .into_iter()
            .map(|mut a| {
                if a.timestamp >= cutoff_ts {
                    // Account was created during the window — balance was 0 at cutoff.
                    a.debits_posted = 0;
                    a.credits_posted = 0;
                } else {
                    // Subtract windowed deltas: balance_at_cutoff = final − Σ(window deltas).
                    a.debits_posted = a
                        .debits_posted
                        .saturating_sub(*debit_delta.get(&a.id).unwrap_or(&0));
                    a.credits_posted = a
                        .credits_posted
                        .saturating_sub(*credit_delta.get(&a.id).unwrap_or(&0));
                }
                a
            })
            .collect();

        let mut plan = Self::build_snapshot_impl(adjusted, true);
        plan.windowed_transfers = windowed_transfers;
        plan
    }

    /// Total number of accounts to import (genesis + regular).
    pub fn total_accounts(&self) -> usize {
        self.genesis_accounts.len() + self.regular_accounts.len()
    }

    /// Total number of synthetic transfers.
    pub fn total_transfers(&self) -> usize {
        self.synthetic_transfers.len()
    }

    /// Total number of windowed transfers to replay.
    pub fn total_windowed_transfers(&self) -> usize {
        self.windowed_transfers.len()
    }

    /// Core snapshot building logic.
    ///
    /// When `windowed_mode` is `true`, synthetic transfer timestamps start from the
    /// maximum timestamp of accounts with non-zero balance (all < `cutoff_ts`), rather
    /// than the maximum timestamp of all accounts. This keeps synthetic transfers in the
    /// slot `(max_active_ts, cutoff_ts)` without colliding with windowed transfers.
    fn build_snapshot_impl(accounts: Vec<Account>, windowed_mode: bool) -> Self {
        // Group accounts by ledger.
        let mut groups_map: HashMap<u32, Vec<Account>> = HashMap::new();
        for account in accounts {
            groups_map.entry(account.ledger).or_default().push(account);
        }

        let mut groups: Vec<AccountGroup> = groups_map
            .into_iter()
            .map(|(ledger, accounts)| {
                // Two genesis IDs per ledger in the reserved u128::MAX range.
                // genesis_credit_id: debit counterparty for credit-side transfers.
                // genesis_debit_id:  credit counterparty for debit-side transfers.
                let genesis_credit_id = u128::MAX - u128::from(ledger) * 2;
                let genesis_debit_id = u128::MAX - u128::from(ledger) * 2 - 1;
                AccountGroup {
                    ledger,
                    accounts,
                    genesis_credit_id,
                    genesis_debit_id,
                }
            })
            .collect();

        // Sort by ledger for deterministic output.
        groups.sort_by_key(|g| g.ledger);

        // Build two genesis accounts per ledger with sequential timestamps starting at 1ns.
        // Order: credit genesis (2*i+1), debit genesis (2*i+2) for ledger at index i.
        // TigerBeetle requires imported timestamps > 0 and strictly increasing.
        let mut genesis_accounts: Vec<Account> = Vec::with_capacity(groups.len() * 2);
        for (i, group) in groups.iter().enumerate() {
            let base_ts = (i as u64) * 2 + 1;
            // Credit genesis account (debit counterparty for credit-side transfers).
            genesis_accounts.push(Account {
                id: group.genesis_credit_id,
                ledger: group.ledger,
                code: 1,
                flags: tb_reader::AccountFlags::from(0),
                timestamp: base_ts,
                debits_pending: 0,
                debits_posted: 0,
                credits_pending: 0,
                credits_posted: 0,
                user_data_128: 0,
                user_data_64: 0,
                user_data_32: 0,
                reserved: 0,
            });
            // Debit genesis account (credit counterparty for debit-side transfers).
            genesis_accounts.push(Account {
                id: group.genesis_debit_id,
                ledger: group.ledger,
                code: 1,
                flags: tb_reader::AccountFlags::from(0),
                timestamp: base_ts + 1,
                debits_pending: 0,
                debits_posted: 0,
                credits_pending: 0,
                credits_posted: 0,
                user_data_128: 0,
                user_data_64: 0,
                user_data_32: 0,
                reserved: 0,
            });
        }

        // The first regular account timestamp must be > last genesis timestamp.
        let genesis_max_ts = genesis_accounts.len() as u64; // == 2*K

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

        // Determine the starting timestamp floor for synthetic transfers.
        //
        // In windowed mode: use the max timestamp among accounts with non-zero balance
        // at cutoff_ts (i.e., those that will actually have synthetic transfers). This
        // keeps synthetic timestamps in (max_active_ts, cutoff_ts), safely below the
        // first windowed transfer's timestamp (>= cutoff_ts).
        //
        // In snapshot mode: start after all regular accounts (max_account_ts).
        let transfer_ts_floor = if windowed_mode {
            regular_accounts
                .iter()
                .filter(|a| a.credits_posted > 0 || a.debits_posted > 0)
                .map(|a| a.timestamp)
                .max()
                .unwrap_or(genesis_max_ts)
        } else {
            max_account_ts
        };

        // Generate synthetic transfers.
        // Transfer timestamps must postdate both debit and credit account timestamps,
        // so we use sequential timestamps starting after the floor.
        let mut synthetic_transfers = Vec::new();
        let mut transfer_id_counter: u128 = 1; // Start from 1 (0 is reserved).
        let mut transfer_ts = transfer_ts_floor;

        // Re-group by ledger for transfer generation (need both genesis IDs per ledger).
        let genesis_by_ledger: HashMap<u32, (u128, u128)> = groups
            .iter()
            .map(|g| (g.ledger, (g.genesis_credit_id, g.genesis_debit_id)))
            .collect();

        for acc in &regular_accounts {
            let (genesis_credit_id, genesis_debit_id) = genesis_by_ledger[&acc.ledger];

            // Credit side: if credits_posted > 0, create transfer:
            //   debit=genesis_credit, credit=account, amount=credits_posted
            // genesis_credit.debits_posted accumulates Σ(acc.credits_posted).
            // This transfer happens FIRST to satisfy debits_must_not_exceed_credits.
            if acc.credits_posted > 0 {
                transfer_ts += 1;
                synthetic_transfers.push(SyntheticTransfer {
                    id: transfer_id_counter,
                    debit_account_id: genesis_credit_id,
                    credit_account_id: acc.id,
                    amount: acc.credits_posted,
                    ledger: acc.ledger,
                    code: acc.code,
                    timestamp: transfer_ts,
                });
                transfer_id_counter += 1;
            }

            // Debit side: if debits_posted > 0, create transfer:
            //   debit=account, credit=genesis_debit, amount=debits_posted
            // genesis_debit.credits_posted accumulates Σ(acc.debits_posted).
            // Invariant after full migration: genesis_credit.debits_posted == genesis_debit.credits_posted
            if acc.debits_posted > 0 {
                transfer_ts += 1;
                synthetic_transfers.push(SyntheticTransfer {
                    id: transfer_id_counter,
                    debit_account_id: acc.id,
                    credit_account_id: genesis_debit_id,
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
            windowed_transfers: vec![],
        }
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
        assert_eq!(plan.windowed_transfers.len(), 0);
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

        // ledger=1: genesis_credit_id = u128::MAX - 1*2 = u128::MAX - 2
        //           genesis_debit_id  = u128::MAX - 1*2 - 1 = u128::MAX - 3
        let expected_credit_genesis = u128::MAX - 2;
        let expected_debit_genesis = u128::MAX - 3;

        // Should have 2 genesis accounts for ledger 1 (credit + debit).
        assert_eq!(plan.genesis_accounts.len(), 2);
        assert_eq!(plan.genesis_accounts[0].ledger, 1);
        assert_eq!(plan.genesis_accounts[0].id, expected_credit_genesis);
        assert_eq!(plan.genesis_accounts[0].timestamp, 1); // 1ns
        assert_eq!(plan.genesis_accounts[1].id, expected_debit_genesis);
        assert_eq!(plan.genesis_accounts[1].timestamp, 2); // 2ns

        // Should have 1 regular account.
        assert_eq!(plan.regular_accounts.len(), 1);
        assert_eq!(plan.regular_accounts[0].id, 100);
        assert_eq!(plan.regular_accounts[0].timestamp, 1000); // Original, > genesis(2)

        // Should have 2 synthetic transfers (credit + debit).
        assert_eq!(plan.synthetic_transfers.len(), 2);

        // First transfer: credit side (debit=genesis_credit, credit=account, amount=200).
        let t0 = &plan.synthetic_transfers[0];
        assert_eq!(t0.debit_account_id, expected_credit_genesis);
        assert_eq!(t0.credit_account_id, 100);
        assert_eq!(t0.amount, 200);
        assert_eq!(t0.timestamp, 1001); // > max account ts (1000)

        // Second transfer: debit side (debit=account, credit=genesis_debit, amount=50).
        let t1 = &plan.synthetic_transfers[1];
        assert_eq!(t1.debit_account_id, 100);
        assert_eq!(t1.credit_account_id, expected_debit_genesis);
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

        // Should have 4 genesis accounts (2 per ledger: credit + debit).
        assert_eq!(plan.genesis_accounts.len(), 4);
        assert_eq!(plan.genesis_accounts[0].timestamp, 1); // ledger 1 credit genesis
        assert_eq!(plan.genesis_accounts[1].timestamp, 2); // ledger 1 debit genesis
        assert_eq!(plan.genesis_accounts[2].timestamp, 3); // ledger 2 credit genesis
        assert_eq!(plan.genesis_accounts[3].timestamp, 4); // ledger 2 debit genesis

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
        // Both must be > last genesis timestamp (2ns for 1 ledger: credit=1, debit=2).
        assert!(plan.regular_accounts[0].timestamp > 2);
    }

    #[test]
    fn test_build_windowed_adjusts_balances() {
        // Account with final balance: debits_posted=300, credits_posted=500.
        // Windowed transfers: debit 100 out, credit 200 in.
        // Expected balance at cutoff: debits_posted=200, credits_posted=300.
        let account = Account {
            id: 10,
            ledger: 1,
            code: 5,
            flags: tb_reader::AccountFlags::from(0),
            timestamp: 1000,
            debits_posted: 300,
            credits_posted: 500,
            debits_pending: 0,
            credits_pending: 0,
            user_data_128: 0,
            user_data_64: 0,
            user_data_32: 0,
            reserved: 0,
        };

        let windowed = vec![
            Transfer {
                id: 1001,
                debit_account_id: 10,
                credit_account_id: 99,
                amount: 100,
                pending_id: 0,
                user_data_128: 0,
                user_data_64: 0,
                user_data_32: 0,
                timeout: 0,
                ledger: 1,
                code: 5,
                flags: tb_reader::TransferFlags::from(0),
                timestamp: 2000,
            },
            Transfer {
                id: 1002,
                debit_account_id: 99,
                credit_account_id: 10,
                amount: 200,
                pending_id: 0,
                user_data_128: 0,
                user_data_64: 0,
                user_data_32: 0,
                timeout: 0,
                ledger: 1,
                code: 5,
                flags: tb_reader::TransferFlags::from(0),
                timestamp: 2001,
            },
        ];

        let cutoff_ts = 1500u64;
        let plan = BalancePlan::build_windowed(vec![account], windowed.clone(), cutoff_ts);

        // Regular account should have adjusted balances.
        assert_eq!(plan.regular_accounts.len(), 1);
        let adj = &plan.regular_accounts[0];
        assert_eq!(adj.debits_posted, 200); // 300 - 100
        assert_eq!(adj.credits_posted, 300); // 500 - 200

        // Synthetic transfers reconstruct the cutoff balances.
        assert_eq!(plan.synthetic_transfers.len(), 2);

        // Windowed transfers stored verbatim.
        assert_eq!(plan.windowed_transfers.len(), 2);
        assert_eq!(plan.windowed_transfers[0].id, 1001);
        assert_eq!(plan.windowed_transfers[1].id, 1002);
    }

    #[test]
    fn test_build_windowed_zeros_window_accounts() {
        // Account created AFTER cutoff_ts — balance should be zeroed out.
        let account = Account {
            id: 20,
            ledger: 1,
            code: 5,
            flags: tb_reader::AccountFlags::from(0),
            timestamp: 3000, // > cutoff_ts
            debits_posted: 50,
            credits_posted: 75,
            debits_pending: 0,
            credits_pending: 0,
            user_data_128: 0,
            user_data_64: 0,
            user_data_32: 0,
            reserved: 0,
        };

        let cutoff_ts = 2000u64;
        let plan = BalancePlan::build_windowed(vec![account], vec![], cutoff_ts);

        // Account balance should be zeroed.
        assert_eq!(plan.regular_accounts.len(), 1);
        assert_eq!(plan.regular_accounts[0].debits_posted, 0);
        assert_eq!(plan.regular_accounts[0].credits_posted, 0);

        // No synthetic transfers (zero balance → no transfers needed).
        assert_eq!(plan.synthetic_transfers.len(), 0);
    }
}
