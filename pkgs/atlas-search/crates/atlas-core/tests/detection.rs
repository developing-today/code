//! Detection ledger + git tree-diff detector tests (#9978, design
//! `doc/2026-11-10T22-56-00Z_design_atlas_delta_detection_join` §2.1,
//! §2.4).
//!
//! Oracles: the git object database itself (a pass's verdicts are
//! checked against the blobs git says changed between two commits) and
//! the ledger after a save/load round trip. The #6028 guarantee is
//! pinned as: an unchanged re-ingest lands zero verdicts and leaves
//! the landed path→blob map byte-for-byte identical.

use std::path::{Path, PathBuf};
use std::process::Command;

use atlas_core::{
    DETECTION_LEDGER_SCHEMA, DetectionLedger, Verdict, git_tree_detection, resolve_commit,
};

struct Repo {
    dir: PathBuf,
}

impl Repo {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "atlas-detect-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let repo = Repo { dir };
        repo.git(&["init", "-q"]);
        repo.git(&["config", "user.name", "Atlas Test"]);
        repo.git(&["config", "user.email", "atlas@example.invalid"]);
        repo
    }

    fn path(&self) -> &Path {
        &self.dir
    }

    fn git(&self, args: &[&str]) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(&self.dir)
            .args(args)
            .output()
            .expect("git runs");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    fn write(&self, path: &str, content: &str) {
        let full = self.dir.join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(full, content).unwrap();
    }

    fn commit(&self, message: &str) -> String {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "-m", message]);
        self.git(&["rev-parse", "HEAD"]).trim().to_string()
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn corpus(repo: &Repo) {
    repo.write("README.md", "atlas\n");
    repo.write("src/a.rs", "fn a() {}\n");
    repo.write("src/b.rs", "fn b() {}\n");
    repo.write("src/nested/c.rs", "fn c() {}\n");
    repo.write("docs/design.md", "# design\n");
}

#[test]
fn first_ingest_marks_every_blob_new() {
    let repo = Repo::new("first-ingest");
    corpus(&repo);
    let c1 = repo.commit("c1");

    let mut ledger = DetectionLedger::new();
    let pass = ledger.detect_git(repo.path(), "corpus", &c1).unwrap();

    assert_eq!(pass.counts.new, 5, "all five blobs are new");
    assert_eq!(pass.counts.changed + pass.counts.gone + pass.counts.unchanged, 0);
    assert_eq!(pass.from, None);
    assert_eq!(pass.to, c1);
    assert_eq!(pass.bytes_read, 0, "detection reads tree objects, not blobs");
    let landed = ledger.source("corpus").unwrap().landed.as_ref().unwrap();
    assert_eq!(landed.objects.len(), 5);
}

#[test]
fn unchanged_reingest_lands_zero_changes() {
    let repo = Repo::new("unchanged");
    corpus(&repo);
    let c1 = repo.commit("c1");

    let mut ledger = DetectionLedger::new();
    ledger.detect_git(repo.path(), "corpus", &c1).unwrap();
    let landed_before = ledger
        .source("corpus")
        .unwrap()
        .landed
        .clone()
        .unwrap();

    // Re-ingest the SAME commit. git names no changed blobs, so the
    // pass must land zero changes and leave the landed map untouched.
    let pass = ledger.detect_git(repo.path(), "corpus", &c1).unwrap();

    assert!(pass.verdicts.is_empty(), "no object may be reported changed");
    assert_eq!(pass.counts.changed, 0);
    assert_eq!(pass.counts.new, 0);
    assert_eq!(pass.counts.gone, 0);
    assert_eq!(pass.counts.unchanged, 5);
    let landed_after = ledger.source("corpus").unwrap().landed.clone().unwrap();
    assert_eq!(
        landed_before.objects, landed_after.objects,
        "unchanged objects keep their exact object ids"
    );
    assert_eq!(ledger.source("corpus").unwrap().passes.len(), 2);
}

#[test]
fn single_file_edit_lands_only_that_delta() {
    let repo = Repo::new("single-edit");
    corpus(&repo);
    let c1 = repo.commit("c1");
    let mut ledger = DetectionLedger::new();
    ledger.detect_git(repo.path(), "corpus", &c1).unwrap();
    let a_before = ledger
        .source("corpus")
        .unwrap()
        .landed
        .as_ref()
        .unwrap()
        .objects["src/a.rs"]
        .clone();

    let objects_before = ledger
        .source("corpus")
        .unwrap()
        .landed
        .as_ref()
        .unwrap()
        .objects
        .clone();

    repo.write("src/a.rs", "fn a() { let _ = 1; }\n");
    let c2 = repo.commit("edit a");

    let pass = ledger.detect_git(repo.path(), "corpus", &c2).unwrap();

    assert_eq!(pass.verdicts.len(), 1, "exactly one changed path");
    assert_eq!(pass.verdicts[0].path, "src/a.rs");
    assert_eq!(pass.verdicts[0].verdict, Verdict::Changed);
    assert_ne!(pass.verdicts[0].blob.as_deref(), Some(a_before.as_str()));
    assert_eq!(pass.counts.changed, 1);
    assert_eq!(pass.counts.unchanged, 4);
    assert_eq!(pass.counts.new + pass.counts.gone, 0);

    let objects = &ledger.source("corpus").unwrap().landed.as_ref().unwrap().objects;
    for path in ["README.md", "src/b.rs", "src/nested/c.rs", "docs/design.md"] {
        assert_eq!(
            objects[path], objects_before[path],
            "{path} is untouched and keeps its object id"
        );
    }
    assert_ne!(objects["src/a.rs"], a_before, "the edited path advanced");
}

#[test]
fn untouched_metadata_survives_a_ledger_round_trip() {
    let repo = Repo::new("round-trip");
    corpus(&repo);
    let c1 = repo.commit("c1");
    let mut ledger = DetectionLedger::new();
    ledger.detect_git(repo.path(), "corpus", &c1).unwrap();

    repo.write("src/b.rs", "fn b() { let _ = 2; }\n");
    let c2 = repo.commit("edit b");
    ledger.detect_git(repo.path(), "corpus", &c2).unwrap();

    let before = ledger
        .source("corpus")
        .unwrap()
        .landed
        .as_ref()
        .unwrap()
        .objects
        .clone();

    let path = repo.path().join("state/detection-ledger.json");
    ledger.save(&path).unwrap();
    let loaded = DetectionLedger::load(&path).unwrap();

    assert_eq!(loaded.schema, DETECTION_LEDGER_SCHEMA);
    let after = &loaded.source("corpus").unwrap().landed.as_ref().unwrap().objects;
    assert_eq!(&before, after, "the ledger round-trips unchanged objects exactly");
    assert_eq!(
        loaded.source("corpus").unwrap().passes.len(),
        2,
        "every pass is retained for audit"
    );
}

#[test]
fn deletion_reports_gone_without_reembedding() {
    let repo = Repo::new("delete");
    corpus(&repo);
    let c1 = repo.commit("c1");
    let mut ledger = DetectionLedger::new();
    ledger.detect_git(repo.path(), "corpus", &c1).unwrap();

    std::fs::remove_file(repo.path().join("docs/design.md")).unwrap();
    let c2 = repo.commit("delete design");

    let pass = ledger.detect_git(repo.path(), "corpus", &c2).unwrap();
    assert_eq!(pass.verdicts.len(), 1);
    assert_eq!(pass.verdicts[0].path, "docs/design.md");
    assert_eq!(pass.verdicts[0].verdict, Verdict::Gone);
    assert!(pass.verdicts[0].blob.is_none());
    assert_eq!(pass.counts.gone, 1);
    let objects = &ledger.source("corpus").unwrap().landed.as_ref().unwrap().objects;
    assert!(!objects.contains_key("docs/design.md"));
}

#[test]
fn rename_is_one_changed_object_second_pass() {
    let repo = Repo::new("rename");
    corpus(&repo);
    let c1 = repo.commit("c1");
    let mut ledger = DetectionLedger::new();
    ledger.detect_git(repo.path(), "corpus", &c1).unwrap();

    repo.git(&["mv", "src/nested/c.rs", "src/c.rs"]);
    let c2 = repo.commit("rename c");

    let pass = ledger.detect_git(repo.path(), "corpus", &c2).unwrap();
    let renamed: Vec<_> = pass
        .verdicts
        .iter()
        .filter(|v| v.previous_path.is_some())
        .collect();
    assert_eq!(renamed.len(), 1, "a pure rename is one object");
    assert_eq!(renamed[0].path, "src/c.rs");
    assert_eq!(renamed[0].previous_path.as_deref(), Some("src/nested/c.rs"));
    assert_eq!(renamed[0].verdict, Verdict::Changed);
    let objects = &ledger.source("corpus").unwrap().landed.as_ref().unwrap().objects;
    assert!(!objects.contains_key("src/nested/c.rs"));
    assert!(objects.contains_key("src/c.rs"));
}

#[test]
fn every_rename_in_one_commit_is_one_changed_object() {
    let repo = Repo::new("rename-two");
    corpus(&repo);
    let c1 = repo.commit("c1");
    let mut ledger = DetectionLedger::new();
    ledger.detect_git(repo.path(), "corpus", &c1).unwrap();

    repo.git(&["mv", "src/a.rs", "src/a2.rs"]);
    repo.git(&["mv", "src/b.rs", "src/b2.rs"]);
    let c2 = repo.commit("rename a and b");

    let pass = ledger.detect_git(repo.path(), "corpus", &c2).unwrap();
    let mut renamed: Vec<_> = pass
        .verdicts
        .iter()
        .filter(|v| v.previous_path.is_some())
        .map(|v| (v.path.as_str(), v.previous_path.as_deref(), v.verdict))
        .collect();
    renamed.sort_by_key(|r| r.0);
    assert_eq!(
        renamed,
        vec![
            ("src/a2.rs", Some("src/a.rs"), Verdict::Changed),
            ("src/b2.rs", Some("src/b.rs"), Verdict::Changed),
        ],
        "both renames coalesce: {:?}",
        pass.verdicts
    );
    assert!(
        !pass
            .verdicts
            .iter()
            .any(|v| v.verdict == Verdict::Gone || v.verdict == Verdict::New),
        "no rename may surface as Gone + New: {:?}",
        pass.verdicts
    );
}

#[test]
fn resolve_commit_returns_full_oid() {
    let repo = Repo::new("resolve");
    corpus(&repo);
    let c1 = repo.commit("c1");
    let resolved = resolve_commit(repo.path(), "HEAD").unwrap();
    assert_eq!(resolved.len(), 40);
    assert_eq!(resolved, c1);
    let _ = git_tree_detection(repo.path(), "corpus", None, &c1, None).unwrap();
}
