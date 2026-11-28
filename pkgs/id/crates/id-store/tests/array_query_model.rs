//! Model test for max-items result paging with snapshot-bound
//! cursors (slice A9119-query-cursor, issue #10274; design
//! A-9119.2, S4.41–S4.44).
//!
//! Oracle: **model** — expected `Reference sorted snapshot
//! result; pages form disjoint ordered slices and cursor binds
//! scope/query/snapshot.` The reference model is independent of
//! the module: it sorts the snapshot's rows by the standard
//! `Ord` of `Vec<Option<Vec<u8>>>` (the order the A9118 design
//! states in plain form), filters by slotwise prefix, and cuts
//! the result into pages by hand. Generated runs page through
//! the module end to end and compare every page — rows,
//! truncation, snapshot token, terminal reason — against that
//! reference, so the concatenation of the pages must equal the
//! reference exactly (disjoint, ordered, complete).
//!
//! Fixed tests pin the design's named rules: the cap never
//! bounds the result domain, a changed scope/query refuses with
//! `cursor_context_changed`, a moved snapshot refuses with
//! `cursor_expired`, a tampered or foreign-key cursor fails
//! authentication, cancellation is observable as a terminal
//! page, and a duplicate address cannot enter a snapshot.
//!
//! Why the module is included by path: slice 9110-02 owns the
//! crate roots, so `array_query` is not exported from id-store
//! yet.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../src/array_query.rs"]
mod array_query;

use std::collections::BTreeMap;

use proptest::prelude::*;

use id_core::array_address::ArrayAddress;

use array_query::{
    ArrayQuery, ArraySnapshot, CancelFlag, CursorPager, QueryError, QueryRow, QueryScope,
    TerminalReason,
};

const KEY: [u8; 32] = [9u8; 32];
const OTHER_KEY: [u8; 32] = [4u8; 32];

type Slots = Vec<Option<Vec<u8>>>;

fn addr(slots: Slots) -> ArrayAddress {
    ArrayAddress::new(slots)
}

fn scope() -> QueryScope {
    QueryScope::new("tenant/a", "ns/alpha")
}

fn row(slots: Slots, payload: &[u8]) -> QueryRow {
    QueryRow::new(addr(slots), payload.to_vec())
}

/// The independent reference: unique addresses sorted by the
/// standard `Ord` of the slot vector, filtered by prefix.
fn reference(rows: &[(Slots, Vec<u8>)], prefix: &Slots) -> Vec<QueryRow> {
    let unique: BTreeMap<Slots, Vec<u8>> = rows.iter().cloned().collect();
    unique
        .into_iter()
        .filter(|(slots, _)| slots.len() >= prefix.len() && slots[..prefix.len()] == prefix[..])
        .map(|(slots, payload)| QueryRow::new(addr(slots), payload))
        .collect()
}

fn snapshot_of(rows: &[(Slots, Vec<u8>)]) -> ArraySnapshot {
    let unique: BTreeMap<Slots, Vec<u8>> = rows.iter().cloned().collect();
    ArraySnapshot::new(
        unique
            .into_iter()
            .map(|(slots, payload)| QueryRow::new(addr(slots), payload))
            .collect(),
    )
    .unwrap()
}

/// Page through the module until termination; returns every
/// page's rows plus the per-page (truncated, terminal) pairs.
fn page_through(
    pager: &CursorPager,
    snapshot: &ArraySnapshot,
    query: &ArrayQuery,
    max_items: usize,
) -> (Vec<QueryRow>, Vec<(bool, Option<TerminalReason>)>) {
    let mut rows = Vec::new();
    let mut marks = Vec::new();
    let mut page = pager
        .first_page(&scope(), query, snapshot, max_items, None)
        .unwrap();
    loop {
        rows.extend(page.rows().iter().cloned());
        marks.push((page.truncated(), page.terminal()));
        assert_eq!(page.snapshot(), snapshot.token());
        match page.next_cursor() {
            Some(cursor) => {
                let cursor = cursor.clone();
                page = pager
                    .next_page(&scope(), query, snapshot, &cursor, max_items, None)
                    .unwrap();
            }
            None => break,
        }
    }
    (rows, marks)
}

fn slot_strategy() -> impl Strategy<Value = Option<Vec<u8>>> {
    prop_oneof![
        Just(None),
        prop::collection::vec(0u8..3, 0..3).prop_map(Some),
    ]
}

fn address_strategy() -> impl Strategy<Value = Slots> {
    prop::collection::vec(slot_strategy(), 0..3)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(160))]

    #[test]
    fn generated_pages_match_the_reference_model(
        rows in prop::collection::vec(
            (address_strategy(), prop::collection::vec(0u8..4, 0..4)),
            0..40,
        ),
        prefix in address_strategy(),
        max_items in 1usize..8,
    ) {
        let snapshot = snapshot_of(&rows);
        let query = ArrayQuery::new(addr(prefix.clone()));
        let pager = CursorPager::new(KEY);
        let (paged, marks) = page_through(&pager, &snapshot, &query, max_items);
        let expected = reference(&rows, &prefix);
        // Pages concatenate to exactly the reference: disjoint,
        // ordered, complete — the cap never drops a row.
        prop_assert_eq!(&paged, &expected);
        // Every page respects the cap; only the final page is
        // terminal (Completed, not truncated); every earlier
        // page is truncated and non-terminal.
        for (index, (truncated, terminal)) in marks.iter().enumerate() {
            if index + 1 == marks.len() {
                prop_assert_eq!(*terminal, Some(TerminalReason::Completed));
                prop_assert!(!truncated);
            } else {
                prop_assert_eq!(*terminal, None);
                prop_assert!(*truncated);
            }
        }
        let page_count = marks.len();
        let expected_pages =
            if expected.is_empty() { 1 } else { expected.len().div_ceil(max_items) };
        prop_assert_eq!(page_count, expected_pages);
    }
}

#[test]
fn the_cap_never_bounds_the_result_domain() {
    let rows: Vec<(Slots, Vec<u8>)> = (0u8..10).map(|i| (vec![Some(vec![i])], vec![i])).collect();
    let snapshot = snapshot_of(&rows);
    let pager = CursorPager::new(KEY);
    let query = ArrayQuery::new(addr(vec![]));
    let (paged, _) = page_through(&pager, &snapshot, &query, 3);
    assert_eq!(paged.len(), 10);
    assert_eq!(paged, reference(&rows, &vec![]));
}

#[test]
fn empty_match_is_one_completed_page() {
    let snapshot = snapshot_of(&[(vec![Some(b"a".to_vec())], vec![1])]);
    let pager = CursorPager::new(KEY);
    let query = ArrayQuery::new(addr(vec![Some(b"zzz".to_vec())]));
    let page = pager
        .first_page(&scope(), &query, &snapshot, 5, None)
        .unwrap();
    assert!(page.rows().is_empty());
    assert!(!page.truncated());
    assert_eq!(page.terminal(), Some(TerminalReason::Completed));
    assert!(page.next_cursor().is_none());
}

#[test]
fn snapshot_token_ignores_input_order_and_tracks_membership() {
    let a = snapshot_of(&[(vec![Some(b"x".to_vec())], vec![1]), (vec![None], vec![2])]);
    let b = snapshot_of(&[(vec![None], vec![2]), (vec![Some(b"x".to_vec())], vec![1])]);
    assert_eq!(a.token(), b.token());
    let c = snapshot_of(&[
        (vec![Some(b"x".to_vec())], vec![1]),
        (vec![None], vec![2]),
        (vec![Some(b"y".to_vec())], vec![3]),
    ]);
    assert_ne!(a.token(), c.token());
}

#[test]
fn duplicate_address_cannot_enter_a_snapshot() {
    let dup = ArraySnapshot::new(vec![row(vec![None], b"one"), row(vec![None], b"two")]);
    assert_eq!(dup.unwrap_err(), QueryError::DuplicateAddress);
}

#[test]
fn zero_max_items_rejects() {
    let snapshot = snapshot_of(&[(vec![None], vec![1])]);
    let pager = CursorPager::new(KEY);
    let query = ArrayQuery::new(addr(vec![]));
    assert_eq!(
        pager.first_page(&scope(), &query, &snapshot, 0, None),
        Err(QueryError::InvalidMaxItems)
    );
}

#[test]
fn cursor_binds_scope_query_and_snapshot() {
    let rows: Vec<(Slots, Vec<u8>)> = (0u8..6).map(|i| (vec![Some(vec![i])], vec![i])).collect();
    let snapshot = snapshot_of(&rows);
    let pager = CursorPager::new(KEY);
    let query = ArrayQuery::new(addr(vec![]));
    let first = pager
        .first_page(&scope(), &query, &snapshot, 2, None)
        .unwrap();
    let cursor = first.next_cursor().unwrap().clone();

    // Same everything: the cursor resumes.
    let second = pager
        .next_page(&scope(), &query, &snapshot, &cursor, 2, None)
        .unwrap();
    assert_eq!(second.rows().len(), 2);

    // Changed tenant: context changed.
    let other_scope = QueryScope::new("tenant/b", "ns/alpha");
    assert_eq!(
        pager.next_page(&other_scope, &query, &snapshot, &cursor, 2, None),
        Err(QueryError::CursorContextChanged)
    );
    // Changed namespace: context changed.
    let other_scope = QueryScope::new("tenant/a", "ns/beta");
    assert_eq!(
        pager.next_page(&other_scope, &query, &snapshot, &cursor, 2, None),
        Err(QueryError::CursorContextChanged)
    );
    // Changed query: context changed.
    let other_query = ArrayQuery::new(addr(vec![Some(vec![1u8])]));
    assert_eq!(
        pager.next_page(&scope(), &other_query, &snapshot, &cursor, 2, None),
        Err(QueryError::CursorContextChanged)
    );
    // Moved snapshot: expired.
    let mut more = rows.clone();
    more.push((vec![Some(vec![99u8])], vec![99]));
    let moved = snapshot_of(&more);
    assert_eq!(
        pager.next_page(&scope(), &query, &moved, &cursor, 2, None),
        Err(QueryError::CursorExpired)
    );
    // Foreign key: authentication fails before any binding read.
    let foreign = CursorPager::new(OTHER_KEY);
    assert_eq!(
        foreign.next_page(&scope(), &query, &snapshot, &cursor, 2, None),
        Err(QueryError::UnauthenticatedCursor)
    );
}

#[test]
fn tampered_and_garbage_cursors_refuse() {
    let rows: Vec<(Slots, Vec<u8>)> = (0u8..4).map(|i| (vec![Some(vec![i])], vec![i])).collect();
    let snapshot = snapshot_of(&rows);
    let pager = CursorPager::new(KEY);
    let query = ArrayQuery::new(addr(vec![]));
    let first = pager
        .first_page(&scope(), &query, &snapshot, 2, None)
        .unwrap();
    let issued = first.next_cursor().unwrap().as_str().to_string();

    // Flip one hex digit in the middle: the tag no longer
    // verifies.
    let mut tampered = issued.clone();
    let mid = tampered.len() / 2;
    let replacement = if tampered.as_bytes()[mid] == b'0' {
        "1"
    } else {
        "0"
    };
    tampered.replace_range(mid..mid + 1, replacement);
    let tampered_cursor = array_query::Cursor::from_string(tampered);
    assert_eq!(
        pager.next_page(&scope(), &query, &snapshot, &tampered_cursor, 2, None),
        Err(QueryError::UnauthenticatedCursor)
    );

    // Not hex at all, and empty: malformed.
    for garbage in ["not-a-cursor", ""] {
        let cursor = array_query::Cursor::from_string(garbage.to_string());
        assert_eq!(
            pager.next_page(&scope(), &query, &snapshot, &cursor, 2, None),
            Err(QueryError::MalformedCursor)
        );
    }
}

#[test]
fn oversized_cursor_is_malformed_before_decoding() {
    let rows: Vec<(Slots, Vec<u8>)> = (0u8..4).map(|i| (vec![Some(vec![i])], vec![i])).collect();
    let snapshot = snapshot_of(&rows);
    let pager = CursorPager::new(KEY);
    let query = ArrayQuery::new(addr(vec![]));

    // One hex character over the documented maximum: refused as
    // malformed by the length check, before hex decoding can
    // allocate storage proportional to the input size.
    let oversized = "a".repeat(array_query::MAX_CURSOR_LEN + 1);
    let cursor = array_query::Cursor::from_string(oversized);
    assert_eq!(
        pager.next_page(&scope(), &query, &snapshot, &cursor, 2, None),
        Err(QueryError::MalformedCursor)
    );

    // Exactly at the maximum, the length check passes the string
    // on to decoding and authentication, which refuse it on
    // their own terms (here the tag does not verify).
    let at_max = "a".repeat(array_query::MAX_CURSOR_LEN);
    let cursor = array_query::Cursor::from_string(at_max);
    assert_eq!(
        pager.next_page(&scope(), &query, &snapshot, &cursor, 2, None),
        Err(QueryError::UnauthenticatedCursor)
    );
}

#[test]
fn cancellation_is_an_observable_terminal_page() {
    let rows: Vec<(Slots, Vec<u8>)> = (0u8..5).map(|i| (vec![Some(vec![i])], vec![i])).collect();
    let snapshot = snapshot_of(&rows);
    let pager = CursorPager::new(KEY);
    let query = ArrayQuery::new(addr(vec![]));
    let flag = CancelFlag::new();
    flag.cancel();
    let page = pager
        .first_page(&scope(), &query, &snapshot, 3, Some(&flag))
        .unwrap();
    assert!(page.rows().is_empty());
    assert_eq!(page.terminal(), Some(TerminalReason::Cancelled));
    assert!(page.truncated());
    assert!(page.next_cursor().is_none());
}
