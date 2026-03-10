//! On-disk layout constants and address translation.
//!
//! All size values are derived from TigerBeetle's **default production cluster
//! configuration** (`src/config.zig`, `configs.default_production`):
//!
//! | Parameter               | Value     |
//! |-------------------------|-----------|
//! | `block_size`            | 512 KiB   |
//! | `superblock_copies`     | 4         |
//! | `clients_max`           | 64        |
//! | `pipeline_prepare_queue_max` | 8    |
//! | `journal_slot_count`    | 1024      |
//! | `message_size_max`      | 1 MiB     |
//! | `sector_size`           | 4096 B    |
//!
//! Derived file zone offsets:
//!
//! | Zone            | Offset            | Size             |
//! |-----------------|-------------------|------------------|
//! | superblock      | 0                 | 98,304 B         |
//! | wal_headers     | 98,304            | 262,144 B        |
//! | wal_prepares    | 360,448           | 1,073,741,824 B  |
//! | client_replies  | 1,074,102,272     | 67,108,864 B     |
//! | grid_padding    | 1,141,211,136     | 163,840 B        |
//! | **grid**        | **1,141,374,976** | unbounded        |

/// Configuration describing the on-disk layout of a TigerBeetle data file.
///
/// The [`Default`] implementation matches TigerBeetle's default production
/// cluster configuration. If the cluster was started with custom flags such as
/// `--clients-max` or `--journal-slot-count`, construct a custom `TBConfig`
/// with the matching values.
#[derive(Debug, Clone)]
pub struct TBConfig {
    /// Size of each grid block in bytes. Default: `512 * 1024` (512 KiB).
    pub block_size: u64,
    /// Size of each superblock copy (header + padding) in bytes. Default: `24,576`.
    pub superblock_copy_size: u64,
    /// Number of redundant superblock copies. Default: `4`.
    pub superblock_copies: u64,
    /// Byte offset of the grid zone within the data file. Default: `1,141,374,976`.
    pub grid_zone_start: u64,
    /// Number of WAL journal slots. Default: `1024`.
    pub journal_slot_count: u64,
    /// Maximum message size (header + body) in bytes. Default: `1,048,576` (1 MiB).
    pub message_size_max: u64,
}

impl Default for TBConfig {
    fn default() -> Self {
        TBConfig {
            block_size: 512 * 1024, // 524,288
            superblock_copy_size: 24_576,
            superblock_copies: 4,
            grid_zone_start: 1_141_374_976,
            journal_slot_count: 1_024,
            message_size_max: 1_048_576,
        }
    }
}

impl TBConfig {
    /// Convert a 1-indexed block address to its byte offset within the data file.
    ///
    /// TigerBeetle block addresses start at 1; address 0 is the null sentinel.
    pub fn block_offset(&self, address: u64) -> u64 {
        debug_assert!(
            address > 0,
            "block addresses are 1-indexed; 0 is a null sentinel"
        );
        self.grid_zone_start + (address - 1) * self.block_size
    }

    /// Byte offset of superblock copy `n` (0-indexed) within the data file.
    pub fn superblock_copy_offset(&self, n: u64) -> u64 {
        debug_assert!(n < self.superblock_copies);
        n * self.superblock_copy_size
    }

    /// Byte offset of the WAL headers zone (256-byte header per slot).
    pub(crate) fn wal_headers_start(&self) -> u64 {
        self.superblock_copies * self.superblock_copy_size
    }

    /// Byte offset of the WAL prepares zone (full message per slot, up to `message_size_max`).
    pub(crate) fn wal_prepares_start(&self) -> u64 {
        self.wal_headers_start() + self.journal_slot_count * 256
    }
}
