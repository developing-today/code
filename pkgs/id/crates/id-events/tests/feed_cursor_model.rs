//! Model-based oracle for A-GRAPH-9128-02 (#10268).
//!
//! The independent model asserts the complete-feed projection ledger
//! over two source authorities: identical source epoch/sequence pairs
//! still yield distinct, ordered projection entries; latest/since/
//! before page by the stable `(projection_seq, entry_id)` order at a
//! pinned source-frontier vector; generation changes, resets and
//! retention gaps expire cursors instead of reporting "no changes";
//! and bad MACs plus cross-query/cross-tenant replays are typed
//! mismatches.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use id_core::graph::feed_cursor::{FeedCursorError, FeedLedger, FrontierPin, LedgerError};

const SERVER_KEY: [u8; 32] = [7; 32];
const OTHER_KEY: [u8; 32] = [9; 32];
const QUERY_COMMENTS: [u8; 32] = [1; 32];
const QUERY_OTHER: [u8; 32] = [2; 32];
const EXPIRES_AT: u64 = 9_000_000_000;
const NOW: u64 = 1_700_000_000;

fn pin(authority: &str, epoch: u64, sequence: u64) -> FrontierPin {
    FrontierPin {
        authority: authority.to_owned(),
        epoch,
        sequence,
    }
}

fn ledger() -> FeedLedger {
    FeedLedger::new(
        "server-a".to_owned(),
        "tenant-7".to_owned(),
        QUERY_COMMENTS,
        4,
        0,
    )
}

fn seeded() -> (FeedLedger, Vec<FrontierPin>) {
    let mut feed = ledger();
    let first = feed
        .append("authority-a".to_owned(), 3, 1, "object-a-1".to_owned())
        .unwrap();
    let second = feed
        .append("authority-b".to_owned(), 7, 1, "object-b-1".to_owned())
        .unwrap();
    let third = feed
        .append("authority-a".to_owned(), 3, 1, "object-a-2".to_owned())
        .unwrap();
    assert_eq!(
        [
            first.projection_seq,
            second.projection_seq,
            third.projection_seq
        ],
        [1, 2, 3]
    );
    let cutoff = vec![pin("authority-a", 3, 1), pin("authority-b", 7, 1)];
    (feed, cutoff)
}

#[test]
fn identical_source_positions_yield_distinct_ordered_entries() {
    let (mut feed, cutoff) = seeded();
    // Same authority, epoch and sequence but a different object still
    // allocates a fresh projection sequence.
    let fourth = feed
        .append("authority-a".to_owned(), 3, 1, "object-a-3".to_owned())
        .unwrap();
    assert_eq!(fourth.projection_seq, 4);
    // Exact source-key repeat is idempotent: no sequence is consumed.
    let repeat = feed
        .append("authority-a".to_owned(), 3, 1, "object-a-3".to_owned())
        .unwrap();
    assert_eq!(repeat, fourth);
    assert_eq!(feed.high_water(), 4);

    let snapshot = feed.snapshot(&cutoff);
    let ids: Vec<&str> = snapshot
        .iter()
        .map(|entry| entry.entry_id.as_str())
        .collect();
    assert_eq!(ids.len(), 4);
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), 4, "entry identities must be distinct");
    let seqs: Vec<u64> = snapshot.iter().map(|entry| entry.projection_seq).collect();
    assert_eq!(seqs, vec![1, 2, 3, 4]);
}

#[test]
fn complete_snapshot_holds_entries_past_the_pinned_cutoff() {
    let (mut feed, cutoff) = seeded();
    let beyond = feed
        .append("authority-b".to_owned(), 7, 2, "object-b-2".to_owned())
        .unwrap();
    assert_eq!(beyond.projection_seq, 4);
    let snapshot = feed.snapshot(&cutoff);
    assert_eq!(snapshot.len(), 3);
    assert!(
        !snapshot
            .iter()
            .any(|entry| entry.entry_id == beyond.entry_id),
        "entry beyond the pinned frontier must not leak into the snapshot"
    );
    // An authority with no pin contributes nothing.
    let unpinned = feed
        .append("authority-c".to_owned(), 1, 1, "object-c-1".to_owned())
        .unwrap();
    let snapshot = feed.snapshot(&cutoff);
    assert!(
        !snapshot
            .iter()
            .any(|entry| entry.entry_id == unpinned.entry_id),
        "unpinned authority must not leak into the snapshot"
    );
}

#[test]
fn since_and_before_page_by_stable_projection_order() {
    let (feed, cutoff) = seeded();
    let (cursor, first_page) = feed.latest(&SERVER_KEY, &cutoff, EXPIRES_AT);
    assert_eq!(first_page.len(), 3);

    let after_first = feed.since(&SERVER_KEY, cursor.as_bytes(), NOW).unwrap();
    assert!(
        after_first.1.is_empty(),
        "latest cursor is already at the end"
    );

    // Page back from the end: before returns the two older entries in
    // stable ascending order.
    let (older_cursor, older) = feed.before(&SERVER_KEY, cursor.as_bytes(), NOW).unwrap();
    let older_ids: Vec<&str> = older.iter().map(|entry| entry.entry_id.as_str()).collect();
    assert_eq!(older.len(), 2);
    assert_eq!(
        older_ids,
        vec![
            first_page[0].entry_id.as_str(),
            first_page[1].entry_id.as_str()
        ]
    );
    // Continuing backward from the oldest returned entry is empty:
    // pages concatenate without omission or duplication.
    let (_, oldest) = feed
        .before(&SERVER_KEY, older_cursor.as_bytes(), NOW)
        .unwrap();
    assert!(oldest.is_empty());

    // since from the middle cursor returns exactly the tail.
    let (_, tail) = feed
        .since(&SERVER_KEY, older_cursor.as_bytes(), NOW)
        .unwrap();
    let tail_ids: Vec<&str> = tail.iter().map(|entry| entry.entry_id.as_str()).collect();
    assert_eq!(
        tail_ids,
        vec![
            first_page[1].entry_id.as_str(),
            first_page[2].entry_id.as_str()
        ]
    );
}

#[test]
fn restart_restores_the_durable_high_water_mark() {
    let (feed, _) = seeded();
    let durable = feed.high_water();
    let entries = feed.entries().to_vec();
    let mut restarted = FeedLedger::restore(
        "server-a".to_owned(),
        "tenant-7".to_owned(),
        QUERY_COMMENTS,
        4,
        durable,
        entries,
    );
    let next = restarted
        .append("authority-a".to_owned(), 3, 2, "object-a-2".to_owned())
        .unwrap();
    assert_eq!(
        next.projection_seq, 4,
        "restart must continue the durable sequence, never reuse it"
    );

    // Even if the persisted mark is inconsistent with the restored
    // entries, restoration must not allocate an already-used sequence.
    let mut repaired = FeedLedger::restore(
        "server-a".to_owned(),
        "tenant-7".to_owned(),
        QUERY_COMMENTS,
        4,
        1,
        feed.entries().to_vec(),
    );
    let after_short_mark = repaired
        .append("authority-c".to_owned(), 1, 1, "object-c-1".to_owned())
        .unwrap();
    assert_eq!(after_short_mark.projection_seq, 4);
}

#[test]
fn generation_change_invalidates_prior_cursors() {
    let (mut feed, cutoff) = seeded();
    let (cursor, _) = feed.latest(&SERVER_KEY, &cutoff, EXPIRES_AT);
    feed.advance_generation(5);
    assert_eq!(
        feed.since(&SERVER_KEY, cursor.as_bytes(), NOW).unwrap_err(),
        FeedCursorError::Expired
    );
    assert_eq!(
        feed.before(&SERVER_KEY, cursor.as_bytes(), NOW)
            .unwrap_err(),
        FeedCursorError::Expired
    );
    // The feed-wide high-water mark stays monotonic across the
    // generation change: no projection sequence is reused.
    let next = feed
        .append("authority-b".to_owned(), 7, 2, "object-b-2".to_owned())
        .unwrap();
    assert_eq!(next.projection_seq, 4);
}

#[test]
fn reset_expires_cursors_instead_of_claiming_no_changes() {
    let (mut feed, cutoff) = seeded();
    let (cursor, _) = feed.latest(&SERVER_KEY, &cutoff, EXPIRES_AT);
    feed.reset(5);
    assert_eq!(
        feed.since(&SERVER_KEY, cursor.as_bytes(), NOW).unwrap_err(),
        FeedCursorError::Expired
    );
    let after_reset = feed
        .append("authority-a".to_owned(), 3, 2, "object-a-2".to_owned())
        .unwrap();
    assert_eq!(
        after_reset.projection_seq, 4,
        "reset must preserve the high-water mark across generations"
    );
}

#[test]
fn retention_gap_expires_cursors_positioned_in_pruned_range() {
    let (mut feed, cutoff) = seeded();
    let (old_cursor, _) = feed.latest(&SERVER_KEY, &cutoff, EXPIRES_AT);
    // Cursor at the first entry, before pruning.
    let (first_cursor, _) = feed
        .before(&SERVER_KEY, old_cursor.as_bytes(), NOW)
        .unwrap();
    let (head_cursor, head) = feed
        .before(&SERVER_KEY, first_cursor.as_bytes(), NOW)
        .unwrap();
    assert!(head.is_empty());
    feed.prune_below(3);
    assert_eq!(
        feed.since(&SERVER_KEY, head_cursor.as_bytes(), NOW)
            .unwrap_err(),
        FeedCursorError::Expired,
        "cursor inside the retention gap must expire"
    );
    // Entries above the horizon still page.
    let (_, tail) = feed.since(&SERVER_KEY, old_cursor.as_bytes(), NOW).unwrap();
    assert!(tail.is_empty(), "cursor already at the end stays empty");
    let snapshot = feed.snapshot(&cutoff);
    assert!(
        snapshot.iter().all(|entry| entry.projection_seq >= 3),
        "pruned entries must leave the snapshot"
    );
}

#[test]
fn bad_mac_is_a_mismatch() {
    let (feed, cutoff) = seeded();
    let (cursor, _) = feed.latest(&SERVER_KEY, &cutoff, EXPIRES_AT);
    // A cursor validated under the wrong server key fails closed.
    assert_eq!(
        feed.since(&OTHER_KEY, cursor.as_bytes(), NOW).unwrap_err(),
        FeedCursorError::Mismatch
    );
    // Tampered bytes fail closed.
    let mut tampered = cursor.as_bytes().to_vec();
    let last = tampered.len() - 1;
    tampered[last] ^= 0xff;
    assert_eq!(
        feed.since(&SERVER_KEY, &tampered, NOW).unwrap_err(),
        FeedCursorError::Mismatch
    );
    // Non-cursor bytes fail closed.
    assert_eq!(
        feed.since(&SERVER_KEY, b"not-a-cursor", NOW).unwrap_err(),
        FeedCursorError::Mismatch
    );
}

#[test]
fn cross_query_and_cross_tenant_replay_are_mismatches() {
    let (feed, cutoff) = seeded();
    let (cursor, _) = feed.latest(&SERVER_KEY, &cutoff, EXPIRES_AT);
    // Same server key, but the cursor was minted for another query.
    let other_query = FeedLedger::new(
        "server-a".to_owned(),
        "tenant-7".to_owned(),
        QUERY_OTHER,
        4,
        0,
    );
    assert_eq!(
        other_query
            .since(&SERVER_KEY, cursor.as_bytes(), NOW)
            .unwrap_err(),
        FeedCursorError::Mismatch
    );
    // Same query fingerprint, but another tenant.
    let other_tenant = FeedLedger::new(
        "server-a".to_owned(),
        "tenant-9".to_owned(),
        QUERY_COMMENTS,
        4,
        0,
    );
    assert_eq!(
        other_tenant
            .since(&SERVER_KEY, cursor.as_bytes(), NOW)
            .unwrap_err(),
        FeedCursorError::Mismatch
    );
}

#[test]
fn past_expiry_is_expired() {
    let (feed, cutoff) = seeded();
    let (cursor, _) = feed.latest(&SERVER_KEY, &cutoff, EXPIRES_AT);
    assert_eq!(
        feed.since(&SERVER_KEY, cursor.as_bytes(), EXPIRES_AT + 1)
            .unwrap_err(),
        FeedCursorError::Expired
    );
}

#[test]
fn sequence_exhaustion_stops_allocation_without_wrapping() {
    let mut feed = FeedLedger::new(
        "server-a".to_owned(),
        "tenant-7".to_owned(),
        QUERY_COMMENTS,
        4,
        u64::MAX - 1,
    );
    let last = feed
        .append("authority-a".to_owned(), 1, 1, "object-a-1".to_owned())
        .unwrap();
    assert_eq!(last.projection_seq, u64::MAX);
    assert_eq!(
        feed.append("authority-b".to_owned(), 1, 1, "object-b-1".to_owned())
            .unwrap_err(),
        LedgerError::SequenceExhausted
    );
    assert_eq!(feed.high_water(), u64::MAX);
}
