//! Triage probe for issue #1523 (queue task 1569) — NOT for commit.
//! Bug expectation: fork_world refuses a crash-torn journal that
//! FileJournal::open repairs fine.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use id::world::{JournalEntry, WorldEvent, WorldEventKind};
use id::world_store::{FileJournal, ForkSpec, ModuleDir, fork_world};

fn build_src(root: &std::path::Path) -> std::path::PathBuf {
    let src = root.join("src");
    std::fs::create_dir_all(&src).unwrap();
    let modules = ModuleDir::new(src.join("modules"));
    let hash = modules.save(b"wasm-bytes").unwrap();
    let (mut journal, _) = FileJournal::open(&src.join("journal.jsonl"), "src-world").unwrap();
    journal
        .append(&JournalEntry::Committed {
            event: WorldEvent {
                sequence: 1,
                participant_id: 0,
                kind: WorldEventKind::ProgramInstalled { module_hash: hash, seed: 7, provenance: None },
            },
        })
        .unwrap();
    drop(journal);
    src
}

fn torn_fragment() -> Vec<u8> {
    // First half of a would-be next entry, no trailing newline: a crash
    // mid-append leaves exactly this shape.
    let entry = JournalEntry::Committed {
        event: WorldEvent { sequence: 2, participant_id: 0, kind: WorldEventKind::ProgramInstalled { module_hash: "x".into(), seed: 9, provenance: None } },
    };
    let full = serde_json::to_vec(&entry).unwrap();
    full[..full.len() / 2].to_vec()
}

fn spec() -> ForkSpec {
    ForkSpec { from_world_id: "src-world".to_owned(), to_world_id: "dst-world".to_owned(), template: false }
}

#[test]
fn probe_torn_tail_open_repairs_but_fork_refuses() {
    let root = std::env::temp_dir().join(format!("id-probe1523-{}", std::process::id()));
    let src = build_src(&root);
    // Tear the journal: append the fragment without a newline.
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().append(true).open(src.join("journal.jsonl")).unwrap();
        f.write_all(&torn_fragment()).unwrap();
    }
    // Control: open on a copy repairs the torn tail fine.
    let copy = root.join("copy");
    std::fs::create_dir_all(&copy).unwrap();
    std::fs::copy(src.join("journal.jsonl"), copy.join("journal.jsonl")).unwrap();
    let open_res = FileJournal::open(&copy.join("journal.jsonl"), "src-world");
    println!("open(torn copy) ok = {}", open_res.is_ok());
    assert!(open_res.is_ok(), "open should repair the torn tail");

    // The bug: fork on the torn source refuses, calling it corrupt.
    let dst = root.join("dst");
    let fork_res = fork_world(&src, &dst, &spec());
    match &fork_res {
        Ok(()) => println!("fork(torn src) = OK (bug NOT reproduced)"),
        Err(e) => println!("fork(torn src) = Err: {e:#}"),
    }
    assert!(fork_res.is_err(), "BUG EXPECTED: fork should refuse the torn journal in this probe");
    assert!(format!("{:#}", fork_res.unwrap_err()).contains("is corrupt"), "error should misdiagnose the torn append as corruption");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn probe_complete_corrupt_line_stays_fatal_for_fork() {
    let root = std::env::temp_dir().join(format!("id-probe1523c-{}", std::process::id()));
    let src = build_src(&root);
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().append(true).open(src.join("journal.jsonl")).unwrap();
        f.write_all(b"{\"not\":\"an entry\"}\n").unwrap();
    }
    let dst = root.join("dst");
    let fork_res = fork_world(&src, &dst, &spec());
    println!("fork(corrupt complete line) err = {:?}", fork_res.as_ref().map_err(|e| format!("{e:#}")));
    assert!(fork_res.is_err(), "newline-terminated corruption must stay fatal");
    std::fs::remove_dir_all(&root).ok();
}
