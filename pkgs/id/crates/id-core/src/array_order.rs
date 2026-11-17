//! Typed lexicographic ordering for array addresses.
//!
//! Slice A9118-array-order (issue #10202), design
//! `part-a-addressing/versioned-design.md` section "#9118 —
//! Array-address core", statements S4.25–S4.28: the total order
//! over addresses whose slots are `Option<Bytes>`.
//!
//! The order is defined slotwise:
//!
//! - a null slot (`None`) sorts before any bytes slot (`Some`);
//! - two bytes slots compare as unsigned byte strings,
//!   lexicographically, with a strict prefix sorting before its
//!   extension (`Some([])` is empty bytes — distinct from null,
//!   and the smallest bytes value);
//! - addresses compare slot by slot; when every shared slot is
//!   equal, the shorter address (a strict prefix of the longer)
//!   sorts first.
//!
//! This is exactly the standard `Ord` of `Vec<Option<Vec<u8>>>`,
//! which the codec's `ArrayAddress` (slice A9118-array-codec)
//! delegates to; the functions here state the order explicitly,
//! in the typed form index and pagination consumers use, so the
//! ordering contract has one named home that the property tests
//! pin against independent references.
//!
//! This module is intentionally dependency-free (pure `std`): the
//! crate-root export and manifest wiring belong to slice 9110-02;
//! until that lands, the integration tests include this file by
//! path.

use std::cmp::Ordering;

/// Compare two slots: null before bytes; bytes unsigned
/// lexicographic with a strict prefix before its extension.
pub fn cmp_slot(a: &Option<Vec<u8>>, b: &Option<Vec<u8>>) -> Ordering {
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(x), Some(y)) => cmp_bytes(x, y),
    }
}

/// Compare two byte strings as unsigned lexicographic values: the
/// first differing byte decides (as an unsigned value); if one is
/// a strict prefix of the other, the shorter sorts first.
pub fn cmp_bytes(a: &[u8], b: &[u8]) -> Ordering {
    for (x, y) in a.iter().zip(b.iter()) {
        match x.cmp(y) {
            Ordering::Equal => {}
            ord => return ord,
        }
    }
    a.len().cmp(&b.len())
}

/// Compare two addresses slotwise under [`cmp_slot`]; when every
/// shared slot is equal, the shorter address sorts first.
pub fn cmp_address(a: &[Option<Vec<u8>>], b: &[Option<Vec<u8>>]) -> Ordering {
    for (x, y) in a.iter().zip(b.iter()) {
        match cmp_slot(x, y) {
            Ordering::Equal => {}
            ord => return ord,
        }
    }
    a.len().cmp(&b.len())
}
