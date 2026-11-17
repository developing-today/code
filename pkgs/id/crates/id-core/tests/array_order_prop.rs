//! Property tests for the typed lexicographic ordering of array
//! addresses (slice A9118-array-order, issue #10202; design
//! statements S4.25–S4.28).
//!
//! Oracle: **invariant** — `None` sorts before `Some(bytes)`; byte
//! strings compare unsigned lexicographically with shorter equal
//! prefixes first; tuple comparison proceeds slotwise then shorter
//! equal tuples first, yielding a total antisymmetric transitive
//! order. Verified against two independent references: an explicit
//! sort-key model (the tuple reference: null maps to a key below
//! every bytes key; a bytes key is its byte sequence) and the
//! standard `Ord` of `Vec<Option<Vec<u8>>>` the design declares
//! equivalent.
//!
//! The crate-root export of `array_order` belongs to slice
//! 9110-02, so until that lands this test includes the module by
//! path — the same file the export will name.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../src/array_order.rs"]
mod array_order;

use array_order::{cmp_address, cmp_bytes, cmp_slot};
use proptest::prelude::*;
use std::cmp::Ordering;

/// Independent reference #1 — the tuple sort-key model. A slot's
/// key is `(0, [])` for null and `(1, bytes)` for a bytes slot; an
/// address's key is the list of its slot keys. Keys compare with
/// the ordinary lexicographic list order (first difference
/// decides; a strict prefix sorts first), which is the semantics
/// the issue's Python tuple reference computes.
fn key_of(addr: &[Option<Vec<u8>>]) -> Vec<(u8, Vec<u8>)> {
    addr.iter()
        .map(|slot| match slot {
            None => (0, Vec::new()),
            Some(bytes) => (1, bytes.clone()),
        })
        .collect()
}

fn ref_cmp(a: &[Option<Vec<u8>>], b: &[Option<Vec<u8>>]) -> Ordering {
    key_of(a).cmp(&key_of(b))
}

fn slot_strategy() -> impl Strategy<Value = Option<Vec<u8>>> {
    prop_oneof![
        1 => Just(None),
        3 => prop::collection::vec(any::<u8>(), 0..=4).prop_map(Some),
    ]
}

fn addr_strategy() -> impl Strategy<Value = Vec<Option<Vec<u8>>>> {
    prop::collection::vec(slot_strategy(), 0..=4)
}

#[test]
fn golden_ordering_cases() {
    let null: Option<Vec<u8>> = None;
    let empty = Some(Vec::new());
    let a = Some(vec![0x61]);
    let a_ext = Some(vec![0x61, 0x00]);
    let b = Some(vec![0x62]);
    // Null before bytes; empty bytes is the smallest bytes value
    // and distinct from null.
    assert_eq!(cmp_slot(&null, &empty), Ordering::Less);
    assert_eq!(cmp_slot(&empty, &a), Ordering::Less);
    // Unsigned lexicographic: first differing byte decides.
    assert_eq!(cmp_slot(&a, &b), Ordering::Less);
    assert_eq!(cmp_bytes(&[0xFF], &[0x00, 0x00]), Ordering::Greater);
    // A strict prefix sorts before its extension, slot and
    // address level alike.
    assert_eq!(cmp_slot(&a, &a_ext), Ordering::Less);
    assert_eq!(
        cmp_address(&[a.clone()], &[a.clone(), null.clone()]),
        Ordering::Less
    );
    // The empty address sorts before every non-empty one.
    assert_eq!(cmp_address(&[], &[null.clone()]), Ordering::Less);
    assert_eq!(cmp_address(&[], &[]), Ordering::Equal);
    // Slotwise: the first differing slot decides, later slots do
    // not matter.
    assert_eq!(
        cmp_address(&[null.clone(), b.clone()], &[empty.clone(), null.clone()]),
        Ordering::Less
    );
}

proptest! {
    #[test]
    fn ordering_matches_both_references(
        a in addr_strategy(),
        b in addr_strategy(),
    ) {
        let ord = cmp_address(&a, &b);
        // Independent reference #1: the tuple sort-key model.
        prop_assert_eq!(ord, ref_cmp(&a, &b));
        // Independent reference #2: the standard Ord the design
        // declares this order equivalent to.
        prop_assert_eq!(ord, a.cmp(&b));
    }

    #[test]
    fn ordering_is_total_antisymmetric_and_reflexive(
        a in addr_strategy(),
        b in addr_strategy(),
    ) {
        prop_assert_eq!(cmp_address(&a, &b), cmp_address(&b, &a).reverse());
        prop_assert_eq!(cmp_address(&a, &a), Ordering::Equal);
        // Totality: equal under the order means identical slots.
        prop_assert_eq!(cmp_address(&a, &b) == Ordering::Equal, a == b);
    }

    #[test]
    fn ordering_is_transitive(
        a in addr_strategy(),
        b in addr_strategy(),
        c in addr_strategy(),
    ) {
        let ab = cmp_address(&a, &b);
        let bc = cmp_address(&b, &c);
        let ac = cmp_address(&a, &c);
        if ab != Ordering::Greater && bc != Ordering::Greater {
            prop_assert!(ac != Ordering::Greater, "a<=b, b<=c but a>c");
        }
        if ab == Ordering::Equal && bc == Ordering::Equal {
            prop_assert_eq!(ac, Ordering::Equal);
        }
        if ab == Ordering::Less && bc == Ordering::Less {
            prop_assert_eq!(ac, Ordering::Less);
        }
    }

    #[test]
    fn sorting_by_the_order_matches_reference_sort(
        mut addrs in prop::collection::vec(addr_strategy(), 0..=8),
    ) {
        let mut expected = addrs.clone();
        expected.sort_by(|x, y| ref_cmp(x, y));
        addrs.sort_by(|x, y| cmp_address(x, y));
        prop_assert_eq!(addrs, expected);
    }
}
