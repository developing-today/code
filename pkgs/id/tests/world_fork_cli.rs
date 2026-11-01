//! F12.2 CLI surface: `id world fork` and `id world provenance`.
//!
//! The store-level fork is covered by `world_fork.rs`; this suite runs the
//! actual binary: a fork creates a world whose journal restores under the
//! new id and reports its provenance, `--template` is recorded and visible
//! through `id world provenance`, and the forked directory contains only
//! the journal and the modules — the F11.11 seed-inheritance pin (journal
//! state only; no seed secret or other source file is copied).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;

use id::world::{JournalEntry, WorldCore, WorldEvent, WorldEventKind, WorldJournal, WorldLimits};
use id::world_store::{FileJournal, ModuleDir};

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_id"))
}

fn run(args: &[&str]) -> std::process::Output {
    Command::new(binary()).args(args).output().unwrap()
}

fn run_ok(args: &[&str]) -> String {
    let output = run(args);
    assert!(
        output.status.success(),
        "id {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

/// A source world on disk: an installed program, one module file, and a
/// `seed.secret` file standing in for every non-journal file a world
/// directory might hold — none of them may reach the fork.
fn make_source(root: &Path) -> (PathBuf, String) {
    let src = root.join("src-world");
    std::fs::create_dir_all(&src).unwrap();
    let modules = ModuleDir::new(src.join("modules"));
    let hash = modules.save(b"wasm-bytes").unwrap();
    let (mut journal, _) = FileJournal::open(&src.join("journal.jsonl"), "src-world").unwrap();
    journal
        .append(&JournalEntry::Committed {
            event: WorldEvent {
                sequence: 1,
                participant_id: 0,
                kind: WorldEventKind::ProgramInstalled {
                    module_hash: hash.clone(),
                    seed: 7,
                    provenance: None,
                },
            },
        })
        .unwrap();
    drop(journal);
    std::fs::write(src.join("seed.secret"), b"never copy me").unwrap();
    (src, hash)
}

fn top_level_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

#[test]
fn cli_fork_creates_a_world_that_restores_and_reports_provenance() {
    let tmp = tempfile::tempdir().unwrap();
    let (src, hash) = make_source(tmp.path());
    let dst = tmp.path().join("dst-world");

    let stdout = run_ok(&[
        "world",
        "fork",
        src.to_str().unwrap(),
        dst.to_str().unwrap(),
    ]);
    assert!(stdout.contains("forked_from: src-world"), "{stdout}");
    assert!(stdout.contains("template: false"), "{stdout}");

    // The forked journal opens and the world restores under its new id
    // (the directory name is the default world id).
    let (_, entries) = FileJournal::open(&dst.join("journal.jsonl"), "dst-world").unwrap();
    assert!(matches!(
        &entries[0],
        JournalEntry::Created { world_id, .. } if world_id == "dst-world"
    ));
    let restored = WorldCore::restore("dst-world", WorldLimits::default(), entries).unwrap();
    assert_eq!(restored.core.world_id(), "dst-world");
    let program = restored.program.expect("the parent's install restores");
    assert_eq!(program.module_hash, hash);
    assert_eq!(program.seed, 7);

    // The module the parent's install names was copied under its hash.
    assert!(dst.join("modules").join(format!("{hash}.wasm")).is_file());

    // `id world provenance` identifies the fork and its origin.
    let stdout = run_ok(&["world", "provenance", dst.to_str().unwrap(), "--json"]);
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value["world_id"], "dst-world");
    assert_eq!(value["forked_from"], "src-world");
    assert_eq!(value["from_module_hash"], hash);
    assert_eq!(value["template"], false);
    assert_eq!(value["at_sequence"], 1);
}

#[test]
fn cli_fork_copies_only_the_journal_and_modules() {
    let tmp = tempfile::tempdir().unwrap();
    let (src, _hash) = make_source(tmp.path());
    let dst = tmp.path().join("dst-world");

    run_ok(&[
        "world",
        "fork",
        src.to_str().unwrap(),
        dst.to_str().unwrap(),
    ]);

    // F11.11 pin: journal state only. The source's seed.secret — and any
    // other file — is not inherited by the fork.
    assert_eq!(top_level_names(&dst), ["journal.jsonl", "modules"]);
}

#[test]
fn cli_fork_template_flag_is_recorded_and_identifiable() {
    let tmp = tempfile::tempdir().unwrap();
    let (src, _hash) = make_source(tmp.path());
    let dst = tmp.path().join("template-world");

    let stdout = run_ok(&[
        "world",
        "fork",
        src.to_str().unwrap(),
        dst.to_str().unwrap(),
        "--template",
        "--json",
    ]);
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value["world_id"], "template-world");
    assert_eq!(value["template"], true);

    // And the template marking stays identifiable after the fact.
    let stdout = run_ok(&["world", "provenance", dst.to_str().unwrap()]);
    assert!(stdout.contains("template: true"), "{stdout}");

    // The source itself is not a fork: provenance says so.
    let stdout = run_ok(&["world", "provenance", src.to_str().unwrap(), "--json"]);
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value["world_id"], "src-world");
    assert!(value["forked_from"].is_null());
    assert_eq!(value["template"], false);
}

#[test]
fn cli_fork_refuses_a_non_empty_destination_and_a_bad_name() {
    let tmp = tempfile::tempdir().unwrap();
    let (src, _hash) = make_source(tmp.path());
    let dst = tmp.path().join("dst-world");
    std::fs::create_dir_all(&dst).unwrap();
    std::fs::write(dst.join("occupied"), b"x").unwrap();

    let output = run(&[
        "world",
        "fork",
        src.to_str().unwrap(),
        dst.to_str().unwrap(),
    ]);
    assert!(!output.status.success());

    let output = run(&[
        "world",
        "fork",
        src.to_str().unwrap(),
        tmp.path().join("Other").to_str().unwrap(),
    ]);
    assert!(!output.status.success());
}
