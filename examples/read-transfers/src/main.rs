//! Read and print all transfers from a TigerBeetle data file.
//!
//! Usage:
//!   cargo run --bin read-transfers -- path/to/cluster_0_replica_0.tigerbeetle

use std::env;
use std::process;

use tb_reader::DataFileReader;

fn main() {
    let path = env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: read-transfers <path-to-.tigerbeetle-file>");
        process::exit(1);
    });

    let mut reader = match DataFileReader::open(&path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error opening {path}: {e}");
            process::exit(1);
        }
    };

    let iter = match reader.iter_transfers() {
        Ok(it) => it,
        Err(e) => {
            eprintln!("error reading superblock/manifest: {e}");
            process::exit(1);
        }
    };

    let mut total = 0u64;
    for result in iter {
        let transfer = match result {
            Ok(t) => t,
            Err(e) => {
                eprintln!("error decoding transfer: {e}");
                process::exit(1);
            }
        };

        total += 1;
        println!(
            "id={:<39} ledger={:<6} code={:<5} \
             debit_account_id={:<39} credit_account_id={:<39} \
             amount={:<20} flags=0x{:04x} timestamp={}",
            transfer.id,
            transfer.ledger,
            transfer.code,
            transfer.debit_account_id,
            transfer.credit_account_id,
            transfer.amount,
            transfer.flags.raw(),
            transfer.timestamp,
        );
    }

    println!("\nTotal: {total} transfer(s)");
}
