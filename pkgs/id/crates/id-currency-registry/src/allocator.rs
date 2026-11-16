//! The ledger-u32 allocator (design sections 2.1 and 6).
//!
//! Rules, all structural rather than conventional:
//!
//! - **Monotonic.** The next candidate is one past the highest id
//!   ever allocated in the kind's range, so ids increase even when
//!   specific ids were reserved out of band.
//! - **Never recycled.** There is deliberately no release or free
//!   operation on this API, and the store refuses to insert an id
//!   that is already allocated. A spent u32 stays spent; exhaustion
//!   means a new cluster, not reuse (design section 6).
//! - **Non-zero.** Ledger 0 is invalid in `TigerBeetle`; both ranges
//!   start at or above 1, so 0 can never be handed out.
//! - **Range-split.** Currencies allocate from
//!   `0x00100000..=0xFFFFFFFF`; tenancy/system from
//!   `0x00000001..=0x000FFFFF` (ratified, decision 2 on issue #2317).

use std::fmt;

use crate::LedgerKind;
use crate::store::{Allocation, Store, StoreError};

/// An allocation failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AllocError {
    /// The kind's range has no unspent id left.
    Exhausted {
        /// The kind whose range is exhausted.
        kind: LedgerKind,
    },
    /// A specific-id reservation fell outside the kind's range.
    OutOfRange {
        /// The kind the reservation was requested for.
        kind: LedgerKind,
        /// The offending ledger id.
        ledger_id: u32,
    },
    /// The store rejected the allocation (e.g. already allocated).
    Store(StoreError),
}

impl fmt::Display for AllocError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exhausted { kind } => {
                write!(f, "ledger range for kind {} is exhausted", kind.as_str())
            }
            Self::OutOfRange { kind, ledger_id } => {
                write!(
                    f,
                    "ledger {ledger_id} is outside the {} range",
                    kind.as_str()
                )
            }
            Self::Store(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for AllocError {}

impl From<StoreError> for AllocError {
    fn from(err: StoreError) -> Self {
        Self::Store(err)
    }
}

/// The allocator, over any registry [`Store`].
///
/// In production the store is Postgres and `allocate_ledger`
/// (migration `0001`) is the same algorithm over sequences; this
/// type states the semantics the SQL implements and is what tests
/// exercise.
#[derive(Debug)]
pub struct Allocator<S: Store> {
    store: S,
}

impl<S: Store> Allocator<S> {
    /// Create an allocator over `store`.
    pub const fn new(store: S) -> Self {
        Self { store }
    }

    /// Access the underlying store.
    pub const fn store(&self) -> &S {
        &self.store
    }

    /// Mutably access the underlying store.
    pub const fn store_mut(&mut self) -> &mut S {
        &mut self.store
    }

    /// Consume the allocator and return the store.
    pub fn into_store(self) -> S {
        self.store
    }

    /// Allocate the next ledger id for `kind`, recorded against
    /// `owner_ref`. Hands out non-zero ids monotonically inside the
    /// kind's range and never reissues an id.
    pub fn allocate(
        &mut self,
        kind: LedgerKind,
        owner_ref: impl Into<String>,
    ) -> Result<u32, AllocError> {
        let (start, end) = kind.range();
        let next = match self.store.highest_in_range(kind) {
            Some(highest) => highest
                .checked_add(1)
                .ok_or(AllocError::Exhausted { kind })?,
            None => start,
        };
        if next > end || next == 0 {
            return Err(AllocError::Exhausted { kind });
        }
        self.store.insert_allocation(Allocation {
            ledger_id: next,
            kind,
            owner_ref: owner_ref.into(),
        })?;
        Ok(next)
    }

    /// Reserve a specific ledger id for `kind` — the up-front range
    /// reservation the design calls for (e.g. a tenancy block). The
    /// id must lie inside the kind's range and be unspent; reserving
    /// an allocated id fails rather than recycling it.
    pub fn reserve_specific(
        &mut self,
        kind: LedgerKind,
        ledger_id: u32,
        owner_ref: impl Into<String>,
    ) -> Result<(), AllocError> {
        if !kind.contains(ledger_id) {
            return Err(AllocError::OutOfRange { kind, ledger_id });
        }
        self.store.insert_allocation(Allocation {
            ledger_id,
            kind,
            owner_ref: owner_ref.into(),
        })?;
        Ok(())
    }
}
