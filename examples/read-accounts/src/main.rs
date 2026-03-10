//! Read and print all accounts from a TigerBeetle data file (LSM + WAL).
//!
//! Usage:
//!   cargo run -p read-accounts -- path/to/cluster_0_replica_0.tigerbeetle

use std::env;
use std::process;

use tb_reader::DataFileReader;

fn main() {
    let path = env::args().nth(1).unwrap_or_else(|| {
        "/Users/tiktuzki/Desktop/repos/ewallet/core-ledger-ms/compose/data/tigerbeetle-data/0_0.tigerbeetle".into()
    });

    let mut reader = match DataFileReader::open(&path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error opening {path}: {e}");
            process::exit(1);
        }
    };

    println!("=== LSM (checkpointed) accounts ===");
    match reader.iter_accounts() {
        Ok(iter) => {
            let mut n = 0u64;
            for result in iter {
                let a = result.unwrap_or_else(|e| {
                    eprintln!("error: {e}");
                    process::exit(1);
                });
                n += 1;
                print_account(&a);
            }
            println!("Total LSM: {n}\n");
        }
        Err(e) => println!("(none — {e})\n"),
    }

    println!("=== WAL (pre-checkpoint) accounts ===");
    match reader.iter_wal_accounts() {
        Ok(iter) => {
            let mut n = 0u64;
            for result in iter {
                let a = result.unwrap_or_else(|e| {
                    eprintln!("error: {e}");
                    process::exit(1);
                });
                n += 1;
                print_account(&a);
            }
            println!("Total WAL: {n}");
        }
        Err(e) => {
            eprintln!("error reading WAL: {e}");
            process::exit(1);
        }
    }
}

fn print_account(a: &tb_reader::Account) {
    println!(
        "id={:<39} ledger={:<6} code={:<5} \
         debits_posted={:<20} credits_posted={:<20} \
         flags=0x{:04x} timestamp={}",
        a.id,
        a.ledger,
        a.code,
        a.debits_posted,
        a.credits_posted,
        a.flags.raw(),
        a.timestamp,
    );
}
