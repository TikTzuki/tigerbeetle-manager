//! Public data types and internal byte-parsing helpers.

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Bit-flags stored on a TigerBeetle [`Account`].
///
/// Flags are stored as a packed 16-bit integer on disk; this wrapper exposes
/// individual flags as boolean accessors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AccountFlags(u16);

impl AccountFlags {
    /// When set, this account is linked to the next account in the batch.
    pub fn linked(self) -> bool {
        self.0 & (1 << 0) != 0
    }

    /// Debits (pending + posted) must not exceed credits posted.
    pub fn debits_must_not_exceed_credits(self) -> bool {
        self.0 & (1 << 1) != 0
    }

    /// Credits (pending + posted) must not exceed debits posted.
    pub fn credits_must_not_exceed_debits(self) -> bool {
        self.0 & (1 << 2) != 0
    }

    /// Account balance history is retained for point-in-time queries.
    pub fn history(self) -> bool {
        self.0 & (1 << 3) != 0
    }

    /// Account was created with the `imported` flag (batch-import mode).
    pub fn imported(self) -> bool {
        self.0 & (1 << 4) != 0
    }

    /// Account has been closed and can no longer accept transfers.
    pub fn closed(self) -> bool {
        self.0 & (1 << 5) != 0
    }

    /// Raw 16-bit flag value as stored on disk.
    pub fn raw(self) -> u16 {
        self.0
    }
}

impl From<u16> for AccountFlags {
    fn from(v: u16) -> Self {
        AccountFlags(v)
    }
}

/// A TigerBeetle account record decoded from the data file.
///
/// The on-disk representation is exactly 128 bytes, little-endian, with no
/// padding (verified at compile time by TigerBeetle's Zig source).
///
/// | Byte offset | Field            | Type   |
/// |-------------|------------------|--------|
/// | 0           | `id`             | u128   |
/// | 16          | `debits_pending` | u128   |
/// | 32          | `debits_posted`  | u128   |
/// | 48          | `credits_pending`| u128   |
/// | 64          | `credits_posted` | u128   |
/// | 80          | `user_data_128`  | u128   |
/// | 96          | `user_data_64`   | u64    |
/// | 104         | `user_data_32`   | u32    |
/// | 108         | `reserved`       | u32    |
/// | 112         | `ledger`         | u32    |
/// | 116         | `code`           | u16    |
/// | 118         | `flags`          | u16    |
/// | 120         | `timestamp`      | u64    |
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    /// Unique account identifier (application-defined).
    pub id: u128,
    /// Sum of pending debit amounts that have not yet posted.
    pub debits_pending: u128,
    /// Sum of posted (settled) debit amounts.
    pub debits_posted: u128,
    /// Sum of pending credit amounts that have not yet posted.
    pub credits_pending: u128,
    /// Sum of posted (settled) credit amounts.
    pub credits_posted: u128,
    /// Application-defined 128-bit opaque reference.
    pub user_data_128: u128,
    /// Application-defined 64-bit opaque reference.
    pub user_data_64: u64,
    /// Application-defined 32-bit opaque reference.
    pub user_data_32: u32,
    /// Reserved for future accounting policy primitives; must be zero.
    pub reserved: u32,
    /// Ledger identifier (groups accounts into distinct balance spaces).
    pub ledger: u32,
    /// Chart-of-accounts code describing the account type.
    pub code: u16,
    /// Account behaviour flags.
    pub flags: AccountFlags,
    /// Creation timestamp assigned by TigerBeetle (nanoseconds since epoch).
    pub timestamp: u64,
}

impl Account {
    /// Parse an [`Account`] from a 128-byte little-endian slice.
    pub(crate) fn from_bytes(b: &[u8]) -> Self {
        debug_assert!(b.len() >= 128, "account slice must be at least 128 bytes");
        Account {
            id: read_u128(b, 0),
            debits_pending: read_u128(b, 16),
            debits_posted: read_u128(b, 32),
            credits_pending: read_u128(b, 48),
            credits_posted: read_u128(b, 64),
            user_data_128: read_u128(b, 80),
            user_data_64: read_u64(b, 96),
            user_data_32: read_u32(b, 104),
            reserved: read_u32(b, 108),
            ledger: read_u32(b, 112),
            code: read_u16(b, 116),
            flags: AccountFlags(read_u16(b, 118)),
            timestamp: read_u64(b, 120),
        }
    }
}

/// Bit-flags stored on a TigerBeetle [`Transfer`].
///
/// Flags are stored as a packed 16-bit integer on disk; this wrapper exposes
/// individual flags as boolean accessors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TransferFlags(u16);

impl TransferFlags {
    /// When set, this transfer is linked to the next transfer in the batch.
    pub fn linked(self) -> bool {
        self.0 & (1 << 0) != 0
    }

    /// Transfer is pending — it reserves funds but does not post them.
    pub fn pending(self) -> bool {
        self.0 & (1 << 1) != 0
    }

    /// Transfer posts a previously created pending transfer.
    pub fn post_pending_transfer(self) -> bool {
        self.0 & (1 << 2) != 0
    }

    /// Transfer voids a previously created pending transfer.
    pub fn void_pending_transfer(self) -> bool {
        self.0 & (1 << 3) != 0
    }

    /// The transfer amount is capped by the debit account's available balance.
    pub fn balancing_debit(self) -> bool {
        self.0 & (1 << 4) != 0
    }

    /// The transfer amount is capped by the credit account's available balance.
    pub fn balancing_credit(self) -> bool {
        self.0 & (1 << 5) != 0
    }

    /// The debit account is closed after this transfer settles.
    pub fn closing_debit(self) -> bool {
        self.0 & (1 << 6) != 0
    }

    /// The credit account is closed after this transfer settles.
    pub fn closing_credit(self) -> bool {
        self.0 & (1 << 7) != 0
    }

    /// Transfer was created with the `imported` flag (batch-import mode).
    pub fn imported(self) -> bool {
        self.0 & (1 << 8) != 0
    }

    /// Raw 16-bit flag value as stored on disk.
    pub fn raw(self) -> u16 {
        self.0
    }
}

impl From<u16> for TransferFlags {
    fn from(v: u16) -> Self {
        TransferFlags(v)
    }
}

/// A TigerBeetle transfer record decoded from the data file.
///
/// The on-disk representation is exactly 128 bytes, little-endian, with no
/// padding (verified at compile time by TigerBeetle's Zig source).
///
/// | Byte offset | Field               | Type   |
/// |-------------|---------------------|--------|
/// | 0           | `id`                | u128   |
/// | 16          | `debit_account_id`  | u128   |
/// | 32          | `credit_account_id` | u128   |
/// | 48          | `amount`            | u128   |
/// | 64          | `pending_id`        | u128   |
/// | 80          | `user_data_128`     | u128   |
/// | 96          | `user_data_64`      | u64    |
/// | 104         | `user_data_32`      | u32    |
/// | 108         | `timeout`           | u32    |
/// | 112         | `ledger`            | u32    |
/// | 116         | `code`              | u16    |
/// | 118         | `flags`             | u16    |
/// | 120         | `timestamp`         | u64    |
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transfer {
    /// Unique transfer identifier (application-defined).
    pub id: u128,
    /// Account to debit (funds flow out of this account).
    pub debit_account_id: u128,
    /// Account to credit (funds flow into this account).
    pub credit_account_id: u128,
    /// Amount transferred (in the unit of the ledger).
    pub amount: u128,
    /// If posting or voiding a pending transfer, the id of that transfer.
    pub pending_id: u128,
    /// Application-defined 128-bit opaque reference.
    pub user_data_128: u128,
    /// Application-defined 64-bit opaque reference.
    pub user_data_64: u64,
    /// Application-defined 32-bit opaque reference.
    pub user_data_32: u32,
    /// Timeout in seconds for pending transfers (0 = no timeout).
    pub timeout: u32,
    /// Ledger identifier — must match both account ledgers.
    pub ledger: u32,
    /// Chart-of-accounts code describing the reason for the transfer.
    pub code: u16,
    /// Transfer behaviour flags.
    pub flags: TransferFlags,
    /// Creation timestamp assigned by TigerBeetle (nanoseconds since epoch).
    pub timestamp: u64,
}

impl Transfer {
    /// Parse a [`Transfer`] from a 128-byte little-endian slice.
    pub(crate) fn from_bytes(b: &[u8]) -> Self {
        debug_assert!(b.len() >= 128, "transfer slice must be at least 128 bytes");
        Transfer {
            id: read_u128(b, 0),
            debit_account_id: read_u128(b, 16),
            credit_account_id: read_u128(b, 32),
            amount: read_u128(b, 48),
            pending_id: read_u128(b, 64),
            user_data_128: read_u128(b, 80),
            user_data_64: read_u64(b, 96),
            user_data_32: read_u32(b, 104),
            timeout: read_u32(b, 108),
            ledger: read_u32(b, 112),
            code: read_u16(b, 116),
            flags: TransferFlags(read_u16(b, 118)),
            timestamp: read_u64(b, 120),
        }
    }
}

// ---------------------------------------------------------------------------
// Internal types (manifest log parsing)
// ---------------------------------------------------------------------------

/// A row from the manifest log describing one LSM table.
///
/// Corresponds to `schema.ManifestNode.TableInfo` in the Zig source.
#[derive(Debug, Clone)]
pub(crate) struct TableInfo {
    /// 1-indexed grid block address of the table's index block.
    pub address: u64,
    /// LSM tree identifier. `7` = Account objects tree (keyed by timestamp).
    pub tree_id: u16,
    /// Whether this manifest entry creates, updates, or removes the table.
    pub event: ManifestEvent,
}

/// The kind of event recorded in a manifest log entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ManifestEvent {
    /// Table was compacted into this level (live).
    Insert,
    /// Table metadata was updated (still live).
    Update,
    /// Table was compacted away (no longer live).
    Remove,
}

impl TableInfo {
    /// Parse a `TableInfo` from a 128-byte slice of a manifest block body.
    ///
    /// Returns `None` if the label event bits are the reserved value 0.
    pub(crate) fn from_bytes(b: &[u8]) -> Option<Self> {
        debug_assert!(b.len() >= 128, "TableInfo slice must be at least 128 bytes");
        let address = read_u64(b, 96);
        let tree_id = read_u16(b, 124);

        // label is a packed u8: bits[5:0] = level (u6), bits[7:6] = event (u2).
        // Zig packed structs lay fields from LSB to MSB.
        let label = b[126];
        let event_bits = (label >> 6) & 0b11;
        let event = match event_bits {
            1 => ManifestEvent::Insert,
            2 => ManifestEvent::Update,
            3 => ManifestEvent::Remove,
            _ => return None, // 0 = reserved; skip malformed or zero-padded entries
        };

        Some(TableInfo {
            address,
            tree_id,
            event,
        })
    }
}

// ---------------------------------------------------------------------------
// Little-endian byte helpers (used across all modules)
// ---------------------------------------------------------------------------

#[inline]
pub(crate) fn read_u16(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes(b[off..off + 2].try_into().unwrap())
}

#[inline]
pub(crate) fn read_u32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(b[off..off + 4].try_into().unwrap())
}

#[inline]
pub(crate) fn read_u64(b: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(b[off..off + 8].try_into().unwrap())
}

#[inline]
pub(crate) fn read_u128(b: &[u8], off: usize) -> u128 {
    u128::from_le_bytes(b[off..off + 16].try_into().unwrap())
}
