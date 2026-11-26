//! Bake the source revision into atlas-core for the Atlas CLI
//! family's `--version` strings (#13279 part 2).
//!
//! Resolution: `$ATLAS_SOURCE_REV` (packaged builds pass the tree's
//! rev, the way pkgs/fleet passes `$FLEET_SOURCE_REV`) →
//! `git rev-parse` at the repo root (developer builds) → "unknown".
//! A build with no git history honestly reports "unknown" rather
//! than inventing a revision.

use std::process::Command;

fn git(root: &std::path::Path, args: &[&str]) -> Option<String> {
    Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn main() {
    println!("cargo:rerun-if-env-changed=ATLAS_SOURCE_REV");
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let root = std::path::Path::new(&manifest).join("../../../..");
    // Rebuild when the checked-out revision moves (the pkgs/fleet
    // build.rs pattern): HEAD is per-git-dir; branch refs and
    // packed-refs live in the common dir shared across worktrees.
    let resolve = |args: &[&str]| -> Option<std::path::PathBuf> {
        let dir = git(&root, args)?;
        let base = if std::path::Path::new(&dir).is_absolute() {
            std::path::PathBuf::from(dir)
        } else {
            root.join(dir)
        };
        Some(base)
    };
    let mut watched: Vec<std::path::PathBuf> = Vec::new();
    if let Some(gitdir) = resolve(&["rev-parse", "--git-dir"]) {
        watched.push(gitdir.join("HEAD"));
    }
    if let Some(common) = resolve(&["rev-parse", "--git-common-dir"]) {
        watched.push(common.join("packed-refs"));
        watched.push(common.join("refs/heads"));
        if let Some(branch_ref) = git(&root, &["symbolic-ref", "-q", "HEAD"]) {
            watched.push(common.join(branch_ref));
        }
    }
    for path in watched {
        if path.exists() {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
    let rev = std::env::var("ATLAS_SOURCE_REV")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            git(&root, &["rev-parse", "--short=12", "HEAD"])
                .unwrap_or_else(|| "unknown".to_string())
        });
    println!("cargo:rustc-env=ATLAS_SOURCE_REV={rev}");
}
