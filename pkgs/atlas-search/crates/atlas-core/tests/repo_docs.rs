//! Oracles: explicit approved-path model, byte-for-byte DB snapshots, atomic rollback.
use atlas_core::repo_docs::{OWN_REPOS, RepoRoot, ingest_repo_docs};
use atlas_core::shards::Tier;
use atlas_core::stage::{STAGE_V1_DDL, list_staged_items};
use rusqlite::Connection;
use std::path::{Path, PathBuf};

const NOW: &str = "2026-11-24T00:00:00Z";
struct Fixture(PathBuf);
impl Fixture {
    fn new(tag: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("atlas-repo-docs-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }
    fn write(&self, path: &str, text: &str) {
        let path = self.0.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    fn roots(&self) -> Vec<RepoRoot> {
        OWN_REPOS
            .iter()
            .enumerate()
            .map(|(i, repo)| {
                let root = self.0.join(i.to_string());
                std::fs::create_dir_all(&root).unwrap();
                RepoRoot {
                    repo: repo.to_string(),
                    root,
                }
            })
            .collect()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn db() -> Connection {
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch(STAGE_V1_DDL).unwrap();
    c
}
fn snapshot(c: &Connection, except: i64) -> Vec<String> {
    let mut s = c.prepare("SELECT quote(c.id)||'|'||quote(c.item_id)||'|'||quote(c.idx)||'|'||quote(c.text)||'|'||quote(c.content_hash)||'|'||quote(c.created_at)||'|'||quote(i.meta)||'|'||quote(i.text)||'|'||quote(i.sensitivity) FROM chunks_raw c JOIN items i ON i.id=c.item_id WHERE i.id != ?1 ORDER BY c.id").unwrap();
    s.query_map([except], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

#[test]
fn four_roots_provenance_replay_and_edit_match_snapshot_oracle() {
    let f = Fixture::new("replay");
    let roots = f.roots();
    let c = db();
    let long = "paragraph of fleet documentation\n".repeat(5000);
    for i in 0..4 {
        for path in [
            "README.md",
            "doc/runbook.md",
            "docs/design.MD",
            "pkgs/a/docs/api.md",
            "packages/b/README.md",
        ] {
            f.write(
                &format!("{i}/{path}"),
                if path == "doc/runbook.md" {
                    &long
                } else {
                    "owned document"
                },
            );
        }
        for path in [
            "src/private.md",
            "pkgs/a/src/private.md",
            "docs/data.txt",
            ".git/README.md",
        ] {
            f.write(&format!("{i}/{path}"), "excluded");
        }
    }
    assert_eq!(
        ingest_repo_docs(&c, &roots, Some("muse"), Tier::Internal, NOW)
            .unwrap()
            .len(),
        20
    );
    let items = list_staged_items(&c, None).unwrap();
    let sources: std::collections::BTreeSet<_> = items.iter().map(|i| i.source.clone()).collect();
    assert_eq!(
        sources.len(),
        20,
        "repo identity disambiguates equal relative paths"
    );
    for item in &items {
        let meta: serde_json::Value = serde_json::from_str(item.meta.as_ref().unwrap()).unwrap();
        assert_eq!(meta["corpus"], "repo-docs");
        assert_eq!(meta["agent"], "muse");
        assert_eq!(meta["source_ref"].as_str(), item.source.as_deref());
        assert_eq!(meta["zone"], "fleet");
        assert_eq!(meta["sensitivity"], "internal");
        assert!(meta["updated_at"].as_str().unwrap().ends_with('Z'));
    }
    let before = snapshot(&c, -1);
    assert!(
        ingest_repo_docs(&c, &roots, Some("muse"), Tier::Internal, "later")
            .unwrap()
            .is_empty()
    );
    assert_eq!(snapshot(&c, -1), before);
    let edited = items
        .iter()
        .find(|i| i.source.as_deref() == Some("developing-today/code:doc/runbook.md"))
        .unwrap();
    assert!(edited.chunk_count > 1);
    let untouched = snapshot(&c, edited.id);
    f.write("0/doc/runbook.md", &(long + "edited\n"));
    assert_eq!(
        ingest_repo_docs(&c, &roots, Some("muse"), Tier::Internal, "later").unwrap(),
        vec![edited.id]
    );
    assert_eq!(snapshot(&c, edited.id), untouched);
    let mut stmt = c
        .prepare("SELECT text FROM chunks_raw WHERE item_id=?1 ORDER BY idx")
        .unwrap();
    let chunks: Vec<String> = stmt
        .query_map([edited.id], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(
        chunks.concat(),
        std::fs::read_to_string(roots[0].root.join("doc/runbook.md")).unwrap()
    );
}

#[test]
fn configuration_and_boundary_negatives_use_empty_db_oracle() {
    let f = Fixture::new("negative");
    let roots = f.roots();
    let c = db();
    f.write("0/README.md", "valid");
    for bad in [
        vec![],
        vec![RepoRoot {
            repo: "external/repo".into(),
            root: roots[0].root.clone(),
        }],
        vec![roots[0].clone(), roots[0].clone()],
        vec![RepoRoot {
            repo: OWN_REPOS[0].into(),
            root: Path::new("relative").into(),
        }],
        vec![RepoRoot {
            repo: OWN_REPOS[0].into(),
            root: roots[0].root.join("../0"),
        }],
    ] {
        assert!(ingest_repo_docs(&c, &bad, None, Tier::Internal, NOW).is_err());
        assert!(list_staged_items(&c, None).unwrap().is_empty());
    }
    assert!(
        serde_json::from_str::<Vec<RepoRoot>>(
            r#"[{"repo":"developing-today/code","root":"/x","extra":true}]"#
        )
        .is_err()
    );
}

#[cfg(unix)]
#[test]
fn symlinks_and_partial_failure_do_not_escape_or_write_oracle() {
    let f = Fixture::new("links");
    let roots = f.roots();
    let c = db();
    f.write("0/README.md", "valid");
    f.write("outside.md", "must not ingest");
    std::os::unix::fs::symlink(f.0.join("outside.md"), roots[1].root.join("README.md")).unwrap();
    assert!(ingest_repo_docs(&c, &roots, None, Tier::Internal, NOW).is_err());
    assert!(list_staged_items(&c, None).unwrap().is_empty());
    std::fs::remove_file(roots[1].root.join("README.md")).unwrap();
    f.write("0/docs/a.md", "first");
    ingest_repo_docs(&c, &roots, None, Tier::Internal, NOW).unwrap();
    let before = snapshot(&c, -1);
    c.execute_batch("CREATE TEMP TRIGGER fail_chunks BEFORE INSERT ON chunks_raw BEGIN SELECT RAISE(ABORT, 'oracle failure'); END;").unwrap();
    f.write("0/docs/a.md", "changed");
    assert!(ingest_repo_docs(&c, &roots, None, Tier::Internal, NOW).is_err());
    assert_eq!(
        snapshot(&c, -1),
        before,
        "failed restage rolls back deletes and item updates"
    );
}
