//! Bake the source revision and the derived version into the
//! binary for `fleet version`.
//!
//! Source rev resolution: `$FLEET_SOURCE_REV` (the nix build passes
//! the flake's rev) → `git rev-parse` in the repo root (developer
//! builds) → "unknown". The nix sandbox has no `.git`, so without
//! the env var a packaged build honestly reports "unknown" rather
//! than inventing one.
//!
//! Derived version resolution (#8566; policy #5790): the version is
//! derived, never hand-maintained —
//! `$FLEET_DERIVED_VERSION` (release tooling / packaged builds,
//! computed where the git history exists, exactly the way
//! `$FLEET_SOURCE_REV` already flows) → `git describe --tags
//! --match '<app>/v*'` transformed by the shared derivation in
//! `src/derived_version.rs` → the pre-first-tag fallback
//! `0.1.0-dev` for a build with no history at all. See the
//! "Versioning" section of pkgs/fleet/README.md for the tag and
//! release procedure this implements.

use std::process::Command;

#[path = "src/derived_version.rs"]
mod derived_version;

use derived_version::{ReleaseVersion, branch_class, derive, parse_app_tag, parse_describe};

/// The app's short name — its tag prefix and deploy name (#5790:
/// one short-name policy for all apps).
const APP_NAME: &str = "fleet";

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

/// Fallback tag selection (PR #8630 Codex finding): `git describe`
/// considers every tag matching `fleet/v*`, including malformed
/// ones (`fleet/v0.4.3-rc.1`, `fleet/v01.2.3`) that parse_describe
/// rightly rejects — and a rejected NEAREST tag must not hide an
/// older valid release tag behind it. Scan the reachable tags,
/// keep only the ones that validate as `<app>/vX.Y.Z`, and pick
/// the nearest (smallest commit distance; ties go to the highest
/// version).
fn from_validated_tags(root: &std::path::Path, branch: Option<&str>) -> Option<String> {
    let tags = git(
        root,
        &[
            "tag",
            "--list",
            &format!("{APP_NAME}/v*"),
            "--merged",
            "HEAD",
        ],
    )?;
    let hash = git(root, &["rev-parse", "--short=7", "HEAD"]);
    let mut best: Option<(u64, ReleaseVersion)> = None;
    for line in tags.lines() {
        let tag = line.trim();
        let Some(version) = parse_app_tag(APP_NAME, tag) else {
            continue;
        };
        let Some(distance) = git(root, &["rev-list", "--count", &format!("{tag}..HEAD")])
            .and_then(|s| s.parse::<u64>().ok())
        else {
            continue;
        };
        let wins = match best {
            None => true,
            Some((bd, bv)) => {
                distance < bd
                    || (distance == bd
                        && (version.major, version.minor, version.patch)
                            > (bv.major, bv.minor, bv.patch))
            }
        };
        if wins {
            best = Some((distance, version));
        }
    }
    let (distance, version) = best?;
    let class = branch_class(branch, distance == 0);
    Some(derive(Some(version), distance, hash.as_deref(), &class))
}

fn derived_version(root: &std::path::Path) -> String {
    if let Some(v) = std::env::var("FLEET_DERIVED_VERSION")
        .ok()
        .filter(|s| !s.is_empty())
    {
        return v;
    }
    let branch = git(root, &["branch", "--show-current"]);
    if let Some(described) = git(
        root,
        &[
            "describe",
            "--tags",
            "--long",
            "--match",
            &format!("{APP_NAME}/v*"),
        ],
    )
    .and_then(|out| parse_describe(APP_NAME, &out))
    {
        let class = branch_class(branch.as_deref(), described.distance == 0);
        return derive(
            Some(described.tag),
            described.distance,
            described.hash.as_deref(),
            &class,
        );
    }
    if let Some(v) = from_validated_tags(root, branch.as_deref()) {
        return v;
    }
    // No `<app>/v*` tag yet: the 0.1.0-dev fallback family, with the
    // repo-wide commit count as the (monotonic) distance and the
    // HEAD hash as the uniqueness suffix, when git can prove them.
    let hash = git(root, &["rev-parse", "--short=7", "HEAD"]);
    let distance = git(root, &["rev-list", "--count", "HEAD"])
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    let class = branch_class(branch.as_deref(), false);
    derive(None, distance, hash.as_deref(), &class)
}

/// Register the git state the bake reads as build-script inputs
/// (PR #8630 Codex finding): without this, cutting a tag on an
/// already-built tree changes no env var and no source file, and
/// cargo keeps the stale baked version. Tag refs live in the
/// common git dir (shared across worktrees); HEAD is per-git-dir.
fn rerun_on_git_refs(root: &std::path::Path) {
    for (args, leaf) in [
        (vec!["rev-parse", "--git-common-dir"], "refs/tags"),
        (vec!["rev-parse", "--git-common-dir"], "packed-refs"),
        (vec!["rev-parse", "--git-dir"], "HEAD"),
    ] {
        if let Some(dir) = git(root, &args) {
            let base = if std::path::Path::new(&dir).is_absolute() {
                std::path::PathBuf::from(dir)
            } else {
                root.join(dir)
            };
            let watched = base.join(leaf);
            if watched.exists() {
                println!("cargo:rerun-if-changed={}", watched.display());
            }
        }
    }
}

fn main() {
    println!("cargo:rerun-if-env-changed=FLEET_SOURCE_REV");
    println!("cargo:rerun-if-env-changed=FLEET_DERIVED_VERSION");
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let root = std::path::Path::new(&manifest).join("../../..");
    rerun_on_git_refs(&root);
    let rev = std::env::var("FLEET_SOURCE_REV")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            git(&root, &["rev-parse", "--short=12", "HEAD"])
                .unwrap_or_else(|| "unknown".to_string())
        });
    println!("cargo:rustc-env=FLEET_SOURCE_REV={rev}");
    let derived = derived_version(&root);
    println!("cargo:rustc-env=FLEET_DERIVED_VERSION={derived}");
}
