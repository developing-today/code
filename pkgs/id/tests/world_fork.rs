//! F12.2: world forking and templates.
//!
//! `id world fork` copies a world's module and snapshot into a new world with
//! provenance, and templates are forks marked reusable. This suite exercises
//! the store-level fork: the journal header is rewritten, the parent's
//! entries are preserved, a `Forked` provenance entry is appended, and the
//! module files are copied.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use id::world::{JournalEntry, WorldEvent, WorldEventKind, WorldJournal};
use id::world_store::{FileJournal, ForkSpec, ModuleDir, fork_world};

fn read_entries(dir: &std::path::Path) -> Vec<JournalEntry> {
    let bytes = std::fs::read(dir.join("journal.jsonl")).unwrap();
    bytes
        .split(|b| *b == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice(line).unwrap())
        .collect()
}

#[test]
fn a_fork_copies_the_parent_and_records_provenance() {
    let root = std::env::temp_dir().join(format!("id-fork-{}", std::process::id()));
    let src = root.join("src");
    let dst = root.join("dst");
    std::fs::create_dir_all(&src).unwrap();

    // A source world with an installed program and one module.
    let modules = ModuleDir::new(src.join("modules"));
    let hash = modules.save(b"wasm-bytes").unwrap();
    let (mut journal, _) = FileJournal::open(&src.join("journal.jsonl"), "src-world").unwrap();
    let installed = JournalEntry::Committed {
        event: WorldEvent {
            sequence: 1,
            participant_id: 0,
            kind: WorldEventKind::ProgramInstalled {
                module_hash: hash.clone(),
                seed: 7,
                provenance: None,
            },
        },
    };
    journal.append(&installed).unwrap();

    fork_world(
        &src,
        &dst,
        &ForkSpec {
            from_world_id: "src-world".to_owned(),
            to_world_id: "dst-world".to_owned(),
            template: true,
        },
    )
    .unwrap();

    // The destination journal has the new header, the copied install, and the
    // fork provenance.
    let entries = read_entries(&dst);
    assert!(matches!(
        &entries[0],
        JournalEntry::Created { world_id, .. } if world_id == "dst-world"
    ));
    assert!(
        entries.iter().any(|entry| matches!(
            entry,
            JournalEntry::Committed { event } if matches!(&event.kind, WorldEventKind::ProgramInstalled { module_hash, seed, .. } if module_hash == &hash && *seed == 7)
        )),
        "the parent's install is copied: {entries:?}"
    );
    assert!(entries.iter().any(|entry| matches!(
        entry,
        JournalEntry::Forked { from_world_id, from_module_hash, template, at_sequence }
            if from_world_id == "src-world" && from_module_hash == &hash && *template && *at_sequence == 1
    )));

    // The module file was copied under its hash.
    assert!(dst.join("modules").join(format!("{hash}.wasm")).is_file());

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn a_fork_from_a_different_world_is_refused() {
    let root = std::env::temp_dir().join(format!("id-fork-mismatch-{}", std::process::id()));
    let src = root.join("src");
    let dst = root.join("dst");
    std::fs::create_dir_all(&src).unwrap();
    FileJournal::open(&src.join("journal.jsonl"), "real-world").unwrap();

    let error = fork_world(
        &src,
        &dst,
        &ForkSpec {
            from_world_id: "other-world".to_owned(),
            to_world_id: "dst-world".to_owned(),
            template: false,
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("belongs to"), "{error}");

    std::fs::remove_dir_all(&root).ok();
}
