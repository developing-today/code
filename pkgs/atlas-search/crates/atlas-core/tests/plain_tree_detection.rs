//! Plain-tree detector tests (#9980, design
//! `doc/2026-11-10T22-56-00Z_design_atlas_delta_detection_join` §2.2).
//!
//! Oracles: the files the test itself wrote, and `blake3::hash` of their
//! bytes. The #6028 guarantee is pinned the same way as for git sources:
//! an unchanged re-walk lands zero verdicts and leaves the landed
//! path→hash map identical. The §2.2 cost claims are pinned through
//! `bytes_read`: a stat hit is never opened, a suspect is read once.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use atlas_core::{DetectionLedger, Mechanism, PlainTreeOptions, Verdict, plain_tree_detection};

struct Tree {
    dir: PathBuf,
}

impl Tree {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "atlas-plain-{tag}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Tree { dir }
    }

    fn root(&self) -> &Path {
        &self.dir
    }

    fn write(&self, path: &str, content: &str) {
        let full = self.dir.join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(&full, content).unwrap();
    }

    /// Set a file's mtime to a fixed instant, so tests can make two
    /// writes look identical (or different) to the stat pass at will.
    fn set_mtime(&self, path: &str, secs: u64) {
        let file = File::options()
            .write(true)
            .open(self.dir.join(path))
            .unwrap();
        file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
            .unwrap();
    }

    fn remove(&self, path: &str) {
        std::fs::remove_file(self.dir.join(path)).unwrap();
    }

    fn rename(&self, from: &str, to: &str) {
        let target = self.dir.join(to);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::rename(self.dir.join(from), target).unwrap();
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn hash(content: &str) -> String {
    blake3::hash(content.as_bytes()).to_hex().to_string()
}

/// A tree with fixed mtimes, so a re-walk is a stat hit.
fn seeded(tag: &str) -> Tree {
    let tree = Tree::new(tag);
    tree.write("a.txt", "alpha\n");
    tree.write("docs/b.md", "bravo bravo\n");
    tree.write("docs/deep/c.rs", "fn c() {}\n");
    for (i, path) in ["a.txt", "docs/b.md", "docs/deep/c.rs"].iter().enumerate() {
        tree.set_mtime(path, 1_700_000_000 + i as u64);
    }
    tree
}

const SEEDED_BYTES: u64 = ("alpha\n".len() + "bravo bravo\n".len() + "fn c() {}\n".len()) as u64;

#[test]
fn first_ingest_hashes_every_file_and_lands_it_new() {
    let tree = seeded("first");
    let mut ledger = DetectionLedger::new();
    let pass = ledger
        .detect_plain_tree(tree.root(), "notes", &PlainTreeOptions::default())
        .unwrap();

    assert_eq!(pass.mechanism, Mechanism::PlainTreeHashWalk);
    assert_eq!(pass.from, None);
    assert_eq!(pass.counts.new, 3);
    assert_eq!(
        pass.counts.unchanged + pass.counts.changed + pass.counts.gone,
        0
    );
    assert_eq!(pass.bytes_read, SEEDED_BYTES);
    assert_eq!(pass.stat_presumption_rate, None);
    assert!(pass.to.starts_with("blake3:"));

    let landed = ledger.source("notes").unwrap().landed.as_ref().unwrap();
    assert_eq!(landed.commit, pass.to);
    assert_eq!(
        landed.objects.get("docs/b.md"),
        Some(&hash("bravo bravo\n"))
    );
    assert_eq!(landed.objects.len(), 3);
    assert_eq!(landed.stats.len(), 3);
    assert_eq!(landed.stats["a.txt"].size, 6);
}

#[test]
fn unchanged_rewalk_reads_nothing_and_lands_no_verdicts() {
    let tree = seeded("unchanged");
    let mut ledger = DetectionLedger::new();
    let first = ledger
        .detect_plain_tree(tree.root(), "notes", &PlainTreeOptions::default())
        .unwrap();
    let before = ledger.source("notes").unwrap().landed.clone();

    let pass = ledger
        .detect_plain_tree(tree.root(), "notes", &PlainTreeOptions::default())
        .unwrap();
    assert!(pass.verdicts.is_empty(), "{:?}", pass.verdicts);
    assert_eq!(pass.counts.unchanged, 3);
    assert_eq!(pass.bytes_read, 0, "a stat hit must not be opened");
    assert_eq!(pass.stat_presumption_rate, Some(1.0));
    assert_eq!(pass.from.as_deref(), Some(first.to.as_str()));
    assert_eq!(
        pass.to, first.to,
        "the tree digest is stable when nothing changed"
    );
    assert_eq!(ledger.source("notes").unwrap().landed, before);
    assert_eq!(ledger.source("notes").unwrap().passes.len(), 2);
}

#[test]
fn touch_without_edit_is_hashed_once_and_refreshes_only_the_stat() {
    let tree = seeded("touch");
    let mut ledger = DetectionLedger::new();
    ledger
        .detect_plain_tree(tree.root(), "notes", &PlainTreeOptions::default())
        .unwrap();
    let before = ledger.source("notes").unwrap().landed.clone().unwrap();

    tree.set_mtime("docs/b.md", 1_800_000_000);
    let pass = ledger
        .detect_plain_tree(tree.root(), "notes", &PlainTreeOptions::default())
        .unwrap();
    assert!(
        pass.verdicts.is_empty(),
        "a touch is not an edit: {:?}",
        pass.verdicts
    );
    assert_eq!(pass.counts.unchanged, 3);
    assert_eq!(pass.bytes_read, "bravo bravo\n".len() as u64);
    assert_eq!(pass.to, before.commit);

    let after = ledger.source("notes").unwrap().landed.clone().unwrap();
    assert_eq!(after.objects, before.objects);
    assert_ne!(after.stats["docs/b.md"], before.stats["docs/b.md"]);
    assert_eq!(after.stats["a.txt"], before.stats["a.txt"]);

    // The refreshed stat makes the next pass a stat hit again.
    let again = ledger
        .detect_plain_tree(tree.root(), "notes", &PlainTreeOptions::default())
        .unwrap();
    assert_eq!(again.bytes_read, 0);
}

#[test]
fn an_edit_lands_exactly_that_file_changed() {
    let tree = seeded("edit");
    let mut ledger = DetectionLedger::new();
    ledger
        .detect_plain_tree(tree.root(), "notes", &PlainTreeOptions::default())
        .unwrap();

    tree.write("docs/deep/c.rs", "fn c() { todo!() }\n");
    let pass = ledger
        .detect_plain_tree(tree.root(), "notes", &PlainTreeOptions::default())
        .unwrap();
    assert_eq!(pass.verdicts.len(), 1);
    let v = &pass.verdicts[0];
    assert_eq!(v.path, "docs/deep/c.rs");
    assert_eq!(v.verdict, Verdict::Changed);
    assert_eq!(
        v.blob.as_deref(),
        Some(hash("fn c() { todo!() }\n").as_str())
    );
    assert_eq!(
        v.previous_blob.as_deref(),
        Some(hash("fn c() {}\n").as_str())
    );
    assert_eq!(pass.counts.unchanged, 2);
    assert_eq!(pass.counts.changed, 1);
    assert_eq!(pass.bytes_read, "fn c() { todo!() }\n".len() as u64);
}

#[test]
fn adds_deletes_and_renames() {
    let tree = seeded("churn");
    let mut ledger = DetectionLedger::new();
    ledger
        .detect_plain_tree(tree.root(), "notes", &PlainTreeOptions::default())
        .unwrap();

    tree.write("new.txt", "fresh\n");
    tree.remove("a.txt");
    tree.rename("docs/b.md", "archive/b.md");
    let pass = ledger
        .detect_plain_tree(tree.root(), "notes", &PlainTreeOptions::default())
        .unwrap();

    let find = |path: &str| pass.verdicts.iter().find(|v| v.path == path).unwrap();
    assert_eq!(find("new.txt").verdict, Verdict::New);
    assert_eq!(find("a.txt").verdict, Verdict::Gone);
    assert_eq!(
        find("a.txt").previous_blob.as_deref(),
        Some(hash("alpha\n").as_str())
    );
    let moved = find("archive/b.md");
    assert_eq!(moved.verdict, Verdict::Changed);
    assert_eq!(moved.previous_path.as_deref(), Some("docs/b.md"));
    assert!(
        !pass.verdicts.iter().any(|v| v.path == "docs/b.md"),
        "the rename's delete half is coalesced away"
    );
    assert_eq!(pass.counts.new, 1);
    assert_eq!(pass.counts.gone, 1);
    assert_eq!(pass.counts.changed, 1);
    assert_eq!(pass.counts.unchanged, 1);

    let landed = ledger.source("notes").unwrap().landed.as_ref().unwrap();
    let mut paths: Vec<&str> = landed.objects.keys().map(String::as_str).collect();
    paths.sort_unstable();
    assert_eq!(paths, ["archive/b.md", "docs/deep/c.rs", "new.txt"]);
    assert_eq!(landed.stats.len(), 3);
}

#[test]
fn an_edit_that_keeps_size_and_mtime_needs_untrusted_mtime_to_be_seen() {
    let tree = seeded("sneaky");
    let mut ledger = DetectionLedger::new();
    ledger
        .detect_plain_tree(tree.root(), "notes", &PlainTreeOptions::default())
        .unwrap();

    // Same length, same mtime: the stat pass presumes it unchanged,
    // which is the documented price of the optimisation.
    tree.write("a.txt", "ALPHA\n");
    tree.set_mtime("a.txt", 1_700_000_000);
    let presumed = plain_tree_detection(
        tree.root(),
        "notes",
        ledger.source("notes").unwrap().landed.as_ref(),
        &PlainTreeOptions::default(),
    )
    .unwrap()
    .0;
    assert!(presumed.verdicts.is_empty());

    // With mtimes untrusted (a restore from backup), stage 2 decides.
    let untrusted = PlainTreeOptions {
        trust_mtime: false,
        ..PlainTreeOptions::default()
    };
    let pass = ledger
        .detect_plain_tree(tree.root(), "notes", &untrusted)
        .unwrap();
    assert_eq!(pass.verdicts.len(), 1);
    assert_eq!(pass.verdicts[0].path, "a.txt");
    assert_eq!(pass.verdicts[0].verdict, Verdict::Changed);
    assert_eq!(
        pass.bytes_read, SEEDED_BYTES,
        "untrusted means hash everything once"
    );
    assert_eq!(pass.stat_presumption_rate, Some(0.0));
}

#[test]
fn untrusted_prefixes_rehash_only_their_subtree() {
    let tree = seeded("prefix");
    let mut ledger = DetectionLedger::new();
    ledger
        .detect_plain_tree(tree.root(), "notes", &PlainTreeOptions::default())
        .unwrap();

    let options = PlainTreeOptions {
        trust_mtime: true,
        untrusted_prefixes: vec!["docs/deep/".to_string()],
    };
    let pass = ledger
        .detect_plain_tree(tree.root(), "notes", &options)
        .unwrap();
    assert!(pass.verdicts.is_empty());
    assert_eq!(pass.bytes_read, "fn c() {}\n".len() as u64);

    // A prefix is a directory boundary, not a string prefix.
    let near_miss = PlainTreeOptions {
        trust_mtime: true,
        untrusted_prefixes: vec!["doc".to_string()],
    };
    let pass = ledger
        .detect_plain_tree(tree.root(), "notes", &near_miss)
        .unwrap();
    assert_eq!(pass.bytes_read, 0);
}

#[cfg(unix)]
#[test]
fn symlinks_are_skipped_not_followed() {
    let tree = seeded("links");
    let outside = Tree::new("links-outside");
    outside.write("secret.txt", "outside the root\n");
    std::os::unix::fs::symlink(outside.root(), tree.root().join("escape")).unwrap();
    std::os::unix::fs::symlink(tree.root().join("a.txt"), tree.root().join("a-link")).unwrap();

    let (pass, landed) =
        plain_tree_detection(tree.root(), "notes", None, &PlainTreeOptions::default()).unwrap();
    assert_eq!(pass.counts.new, 3);
    assert!(
        landed
            .objects
            .keys()
            .all(|p| !p.starts_with("escape") && p != "a-link")
    );
}

#[test]
fn a_git_source_is_not_walked_as_a_plain_tree() {
    let tree = seeded("mixed");
    let mut ledger = DetectionLedger::new();
    ledger
        .detect_plain_tree(tree.root(), "notes", &PlainTreeOptions::default())
        .unwrap();
    ledger.sources.get_mut("notes").unwrap().passes[0].mechanism = Mechanism::GitTreeDiff;

    let err = ledger
        .detect_plain_tree(tree.root(), "notes", &PlainTreeOptions::default())
        .unwrap_err();
    assert!(err.to_string().contains("git_tree_diff"), "{err}");
}

#[test]
fn ledger_round_trips_stats_and_still_loads_git_only_ledgers() {
    let tree = seeded("roundtrip");
    let mut ledger = DetectionLedger::new();
    ledger
        .detect_plain_tree(tree.root(), "notes", &PlainTreeOptions::default())
        .unwrap();

    let path = tree.root().join("ledger/detection-ledger.json");
    ledger.save(&path).unwrap();
    let loaded = DetectionLedger::load(&path).unwrap();
    assert_eq!(loaded, ledger);

    // A ledger written before plain trees existed has no `stats`.
    let legacy = r#"{"schema":"atlas-detection-ledger/v1","sources":{"repo":{"landed":{"commit":"abc","objects":{"x":"0123"}},"passes":[]}}}"#;
    std::fs::write(&path, legacy).unwrap();
    let loaded = DetectionLedger::load(&path).unwrap();
    let landed = loaded.source("repo").unwrap().landed.as_ref().unwrap();
    assert!(landed.stats.is_empty());
    assert_eq!(landed.objects["x"], "0123");
}
