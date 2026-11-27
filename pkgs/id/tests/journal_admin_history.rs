//! External-construction coverage for `JournalAdminHistory` (#13421).
//!
//! Benches, integration tests and downstream tooling live outside
//! `id-worlds`; before #13421 they could not build the current-form
//! checkpoint admin history at all, because the type's fields were
//! crate-private and it had no public constructor — the `stress_join`
//! bench fixtures could only write the legacy inline-checkpoint form
//! (`admin_history: None`). This test uses only the `id` facade's
//! public surface — the same surface a bench writes journal fixtures
//! through — to build a current-form checkpoint, serialize it in the
//! journal's line format, and read it back unchanged.
//!
//! Oracle: the public constructor and accessors round-trip the exact
//! events and completeness flag, and a `JournalEntry::Checkpoint`
//! carrying the built history survives the journal's on-disk line
//! format byte-for-value.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use id::world::{JournalAdminHistory, JournalEntry, WorldEvent, WorldEventKind};

fn chat(sequence: u64, text: &str) -> WorldEvent {
    WorldEvent {
        sequence,
        participant_id: 7,
        kind: WorldEventKind::Chat(text.to_owned()),
    }
}

#[test]
fn external_code_can_build_a_current_form_checkpoint_admin_history() {
    let admin_events = vec![chat(1, "first"), chat(2, "second")];
    let history = JournalAdminHistory::new(admin_events.clone(), true);
    assert_eq!(history.events(), admin_events.as_slice());
    assert!(history.is_complete());
    assert_eq!(history.clone().into_events(), admin_events);

    let entry = JournalEntry::Checkpoint {
        sequence: 2,
        module_hash: "fixture-program".to_owned(),
        seed: 42,
        provenance: None,
        execution_profile: None,
        caps_ceiling: None,
        schedules: None,
        kv: None,
        blobs: None,
        transfers: None,
        lease: None,
        snapshot: "2".to_owned(),
        events: admin_events.clone(),
        admin_history: Some(history),
        open_presence: Some(Vec::new()),
    };

    // The journal's on-disk line format: one JSON value per entry.
    let line = serde_json::to_string(&entry).unwrap();
    let decoded: JournalEntry = serde_json::from_str(&line).unwrap();
    assert_eq!(decoded, entry);
    let JournalEntry::Checkpoint {
        admin_history: Some(decoded_history),
        ..
    } = decoded
    else {
        panic!("checkpoint lost its admin history on the wire");
    };
    assert_eq!(decoded_history.events(), admin_events.as_slice());
    assert!(decoded_history.is_complete());
}

#[test]
fn incomplete_admin_history_round_trips_as_incomplete() {
    let history = JournalAdminHistory::new(Vec::new(), false);
    assert_eq!(history.events(), [] as [WorldEvent; 0]);
    assert!(!history.is_complete());
    let line = serde_json::to_string(&history).unwrap();
    let decoded: JournalAdminHistory = serde_json::from_str(&line).unwrap();
    assert_eq!(decoded, history);
}
