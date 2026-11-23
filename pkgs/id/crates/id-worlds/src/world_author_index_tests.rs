//! Tests for the author-scoped local index projection (#13741). The
//! oracle is the fixture journal each test writes: programs, joins and
//! timestamps come from the entries it chose, not from a second
//! implementation of the query. Rebuild determinism is a property:
//! the same journals build the same index.

use std::io::{self, Cursor};

use proptest::prelude::*;

use super::*;
use crate::world::{JournalEntry, Participant, WorldEvent, WorldEventKind};

const BUILT_AT: u64 = 1_800_000_000_000;
const WORLD_A: &str = "w-alpha";
const WORLD_B: &str = "w-beta";

/// An in-memory [`WorldJournalSource`]: `journal: None` means the
/// world directory has no journal file.
struct TestSource {
    world_id: &'static str,
    journal: Option<Vec<u8>>,
    open_error: Option<io::ErrorKind>,
    modified_unix_ms: Option<u64>,
}

impl TestSource {
    fn journal(world_id: &'static str, journal: Vec<u8>) -> Self {
        Self {
            world_id,
            journal: Some(journal),
            open_error: None,
            modified_unix_ms: Some(1_799_000_000_000),
        }
    }

    fn no_journal(world_id: &'static str) -> Self {
        Self {
            world_id,
            journal: None,
            open_error: None,
            modified_unix_ms: Some(1_799_000_000_000),
        }
    }
}

impl WorldJournalSource for TestSource {
    fn world_id(&self) -> &str {
        self.world_id
    }

    fn open(&self) -> io::Result<Box<dyn BufRead + '_>> {
        if let Some(kind) = self.open_error {
            return Err(io::Error::new(kind, "test open failure"));
        }
        match &self.journal {
            Some(bytes) => Ok(Box::new(Cursor::new(bytes.clone()))),
            None => Err(io::Error::new(io::ErrorKind::NotFound, "no journal")),
        }
    }

    fn journal_modified_unix_ms(&self) -> Option<u64> {
        self.modified_unix_ms
    }
}

fn header(world: &str) -> JournalEntry {
    JournalEntry::Created {
        world_id: world.to_owned(),
        version: 12,
    }
}

fn issued(id: u64, issued_at: Option<u64>) -> JournalEntry {
    JournalEntry::Issued {
        participant: Participant {
            id,
            display_name: format!("name-{id}"),
        },
        digest: format!("{id:064x}"),
        scopes: 1,
        parent: None,
        expires_at: None,
        uses: None,
        used: 0,
        subject: None,
        issued_at,
    }
}

fn builder_installed(hash: &str) -> JournalEntry {
    JournalEntry::BuilderInstalled {
        by: 1,
        module_hash: hash.to_owned(),
        seed: 7,
    }
}

fn program_installed(hash: &str, sequence: u64) -> JournalEntry {
    JournalEntry::Committed {
        event: WorldEvent {
            sequence,
            participant_id: 0,
            kind: WorldEventKind::ProgramInstalled {
                module_hash: hash.to_owned(),
                seed: 7,
                provenance: None,
                trust: None,
            },
        },
    }
}

/// A checkpoint naming `hash` as the active program, as a raw JSON
/// line: the checkpoint struct is too wide to build by hand here.
fn checkpoint_line(hash: &str, sequence: u64) -> String {
    serde_json::json!({
        "entry": "checkpoint",
        "sequence": sequence,
        "module_hash": hash,
        "seed": 0,
        "snapshot": "",
        "events": [],
    })
    .to_string()
}

fn journal(entries: &[JournalEntry], extra_lines: &[String]) -> Vec<u8> {
    let mut text = String::new();
    for entry in entries {
        text.push_str(&serde_json::to_string(entry).unwrap());
        text.push('\n');
    }
    for line in extra_lines {
        text.push_str(line);
        text.push('\n');
    }
    text.into_bytes()
}

fn world_a_journal() -> Vec<u8> {
    journal(
        &[
            header(WORLD_A),
            issued(1, Some(1_000)),
            issued(2, Some(2_000)),
            builder_installed("hash-a"),
            program_installed("hash-b", 4),
        ],
        &[],
    )
}

fn world_b_journal() -> Vec<u8> {
    journal(
        &[header(WORLD_B), issued(1, None)],
        &[checkpoint_line("hash-c", 9)],
    )
}

fn build_two_worlds() -> AuthorIndex {
    build(
        &[
            TestSource::journal(WORLD_A, world_a_journal()),
            TestSource::journal(WORLD_B, world_b_journal()),
        ],
        BUILT_AT,
        AuthorIndexLimits::default(),
    )
}

#[test]
fn test_indexes_programs_joins_and_timestamps_from_journals() {
    let index = build_two_worlds();

    assert_eq!(index.schema_version, AUTHOR_INDEX_SCHEMA_VERSION);
    assert_eq!(index.built_at_unix_ms, BUILT_AT);
    assert_eq!(index.duplicates_skipped, 0);
    assert_eq!(index.worlds.len(), 2);

    let a = &index.worlds[0];
    assert_eq!(a.world_id, WORLD_A);
    assert_eq!(a.source, IndexSource::Journal);
    assert_eq!(
        a.journal,
        Some(JournalSummary {
            header_version: Some(12),
            entries: 5,
            checkpointed: false,
            torn_tail: false,
        })
    );
    assert_eq!(
        a.programs,
        Some(vec![
            ProgramEntry {
                module_hash: "hash-a".to_owned(),
                first_seen_entry: 4,
            },
            ProgramEntry {
                module_hash: "hash-b".to_owned(),
                first_seen_entry: 5,
            },
        ])
    );
    assert_eq!(
        a.joins,
        Some(JoinsSummary {
            count: 2,
            first_issued_at_unix_ms: Some(1_000),
            last_issued_at_unix_ms: Some(2_000),
            untimed: 0,
            complete: true,
        })
    );
    assert_eq!(a.journal_modified_unix_ms, Some(1_799_000_000_000));

    let b = &index.worlds[1];
    assert_eq!(b.world_id, WORLD_B);
    assert_eq!(b.source, IndexSource::Journal);
    // The checkpoint's active module is a program the world holds,
    // even though no install event survives in this journal.
    assert_eq!(
        b.programs,
        Some(vec![ProgramEntry {
            module_hash: "hash-c".to_owned(),
            first_seen_entry: 3,
        }])
    );
    assert_eq!(
        b.joins,
        Some(JoinsSummary {
            count: 1,
            first_issued_at_unix_ms: None,
            last_issued_at_unix_ms: None,
            untimed: 1,
            complete: true,
        })
    );
    assert!(b.journal.is_some_and(|summary| summary.checkpointed));
}

#[test]
fn test_rebuild_from_same_journals_is_deterministic() {
    let first = build_two_worlds();
    let second = build_two_worlds();
    // Build time differs between runs; the projection does not.
    assert_eq!(first.worlds, second.worlds);
    assert_eq!(first.built_at_unix_ms, second.built_at_unix_ms);
    assert_eq!(first.duplicates_skipped, second.duplicates_skipped);
}

#[test]
fn test_stale_when_journal_modified_after_build() {
    let index = build_two_worlds();
    assert_eq!(index.is_stale(WORLD_A, BUILT_AT), Some(false));
    assert_eq!(index.is_stale(WORLD_A, BUILT_AT + 1), Some(true));
    assert_eq!(index.is_stale(WORLD_A, BUILT_AT - 1), Some(false));
    assert_eq!(index.is_stale("w-unknown", BUILT_AT + 1), None);
}

#[test]
fn test_missing_journal_degrades_to_directory_only() {
    let index = build(
        &[
            TestSource::no_journal(WORLD_A),
            TestSource::journal(WORLD_B, world_b_journal()),
        ],
        BUILT_AT,
        AuthorIndexLimits::default(),
    );

    assert_eq!(index.worlds.len(), 2);
    let a = &index.worlds[0];
    assert_eq!(a.world_id, WORLD_A);
    assert_eq!(
        a.source,
        IndexSource::DirectoryOnly {
            reason: DirectoryOnlyReason::JournalMissing,
        }
    );
    // Nothing is guessed about a world with no journal.
    assert_eq!(a.journal, None);
    assert_eq!(a.programs, None);
    assert_eq!(a.joins, None);
    // The other world still indexes.
    assert_eq!(index.worlds[1].source, IndexSource::Journal);
}

#[test]
fn test_unreadable_journal_degrades_to_directory_only() {
    let mut source = TestSource::no_journal(WORLD_A);
    source.open_error = Some(io::ErrorKind::PermissionDenied);
    let index = build(&[source], BUILT_AT, AuthorIndexLimits::default());

    assert_eq!(
        index.worlds[0].source,
        IndexSource::DirectoryOnly {
            reason: DirectoryOnlyReason::JournalUnreadable,
        }
    );
    assert_eq!(index.worlds[0].programs, None);
}

#[test]
fn test_malformed_journal_degrades_that_world_only() {
    let mut bad = world_a_journal();
    bad.extend_from_slice(b"this is not a journal entry\n");
    let index = build(
        &[
            TestSource::journal(WORLD_A, bad),
            TestSource::journal(WORLD_B, world_b_journal()),
        ],
        BUILT_AT,
        AuthorIndexLimits::default(),
    );

    assert_eq!(
        index.worlds[0].source,
        IndexSource::DirectoryOnly {
            reason: DirectoryOnlyReason::JournalMalformed,
        }
    );
    assert_eq!(index.worlds[0].programs, None);
    assert_eq!(index.worlds[1].source, IndexSource::Journal);
}

#[test]
fn test_wrong_world_journal_is_labelled() {
    let index = build(
        &[TestSource::journal(WORLD_A, world_b_journal())],
        BUILT_AT,
        AuthorIndexLimits::default(),
    );

    assert_eq!(
        index.worlds[0].source,
        IndexSource::DirectoryOnly {
            reason: DirectoryOnlyReason::WrongWorld,
        }
    );
}

#[test]
fn test_duplicate_world_ids_are_skipped_and_counted() {
    let index = build(
        &[
            TestSource::journal(WORLD_A, world_a_journal()),
            TestSource::journal(WORLD_A, world_b_journal()),
        ],
        BUILT_AT,
        AuthorIndexLimits::default(),
    );

    assert_eq!(index.worlds.len(), 1);
    assert_eq!(index.duplicates_skipped, 1);
    // The first source wins.
    assert_eq!(
        index.worlds[0].programs,
        Some(vec![
            ProgramEntry {
                module_hash: "hash-a".to_owned(),
                first_seen_entry: 4,
            },
            ProgramEntry {
                module_hash: "hash-b".to_owned(),
                first_seen_entry: 5,
            },
        ])
    );
}

#[test]
fn test_schema_version_check() {
    assert_eq!(
        check_schema_version(&serde_json::json!({ "schema_version": 1 })),
        Ok(AUTHOR_INDEX_SCHEMA_VERSION)
    );
    assert_eq!(
        check_schema_version(&serde_json::json!({ "schema_version": 2 })),
        Err(Some(2))
    );
    assert_eq!(check_schema_version(&serde_json::json!({})), Err(None));
}

fn journal_entry_strategy() -> impl Strategy<Value = JournalEntry> {
    (0..2u8, 1..=3u64, 0..=5_000u64).prop_map(|(kind, id, at)| {
        if kind == 0 {
            issued(id, Some(at))
        } else {
            builder_installed(if id % 2 == 0 { "h1" } else { "h2" })
        }
    })
}

proptest! {
    #[test]
    fn test_rebuild_determinism_over_generated_journals(
        entries in prop::collection::vec(journal_entry_strategy(), 0..=8),
    ) {
        let mut with_header = vec![header(WORLD_A)];
        with_header.extend(entries);
        let bytes = journal(&with_header, &[]);
        let sources = [TestSource::journal(WORLD_A, bytes)];
        let first = build(&sources, BUILT_AT, AuthorIndexLimits::default());
        let second = build(&sources, BUILT_AT, AuthorIndexLimits::default());
        prop_assert_eq!(first.worlds, second.worlds);
        prop_assert_eq!(first.duplicates_skipped, second.duplicates_skipped);
    }
}
