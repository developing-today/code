//! The `fleet serve search` role through the real multicall binary
//! (#5604): routing, the `atlas-search serve` argv0 spelling that keeps
//! the single-step rollback working, and the failure postures that must
//! hold before any backend, token or socket is touched.
//!
//! Oracle: process exit codes and stderr/stdout text the dispatch layer
//! and the role fix (2 usage/config, 0 help, 69 capability stub).
#![cfg(feature = "search-serve")]

use std::path::PathBuf;
use std::process::{Command, Output};

fn fleet_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_fleet"))
}

/// A scratch dir with the binary linked under an argv0 alias.
fn linked(alias: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "fleet-search-serve-{}-{}",
        std::process::id(),
        alias
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let link = dir.join(alias);
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(fleet_bin(), &link).unwrap();
    (dir, link)
}

fn run(bin: &PathBuf, args: &[&str]) -> Output {
    Command::new(bin)
        .args(args)
        // Never let an ambient config or token steer a contract test.
        .env_remove("ATLAS_TURSO_TOKEN")
        .env(
            "ATLAS_SEARCH_SERVICE_CONFIG",
            "/nonexistent/atlas-service.json",
        )
        .output()
        .unwrap()
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

#[test]
fn serve_search_help_prints_the_roles_own_usage() {
    for flag in ["--help", "-h"] {
        let out = run(&fleet_bin(), &["serve", "search", flag]);
        assert_eq!(out.status.code(), Some(0), "{flag}");
        let o = text(&out.stdout);
        assert!(o.contains("usage: fleet serve search"), "{o}");
        assert!(o.contains("--config"), "{o}");
    }
}

#[test]
fn serve_search_with_a_missing_config_is_a_usage_failure_before_any_backend() {
    let out = run(&fleet_bin(), &["serve", "search"]);
    assert_eq!(out.status.code(), Some(2), "{}", text(&out.stderr));
    let e = text(&out.stderr);
    assert!(e.contains("fleet serve search:"), "{e}");
    assert!(e.contains("cannot read service config"), "{e}");
}

#[test]
fn serve_search_rejects_unknown_flags_including_any_backend_selector() {
    for bad in [
        &["--bogus"][..],
        &["--backend", "local"][..],
        &["--db", "x"][..],
    ] {
        let mut args = vec!["serve", "search"];
        args.extend_from_slice(bad);
        let out = run(&fleet_bin(), &args);
        assert_eq!(out.status.code(), Some(2), "{bad:?}: {}", text(&out.stderr));
        assert!(text(&out.stderr).contains("unexpected argument"));
    }
}

#[test]
fn the_atlas_search_argv0_serve_spelling_runs_the_same_role() {
    // The rollback story: `atlas-search serve` through the fleet binary
    // is the same role as `fleet serve search`.
    let (dir, link) = linked("atlas-search");
    let out = run(&link, &["serve"]);
    assert_eq!(out.status.code(), Some(2), "{}", text(&out.stderr));
    assert!(
        text(&out.stderr).contains("cannot read service config"),
        "{}",
        text(&out.stderr)
    );
    let help = run(&link, &["serve", "--help"]);
    assert_eq!(help.status.code(), Some(0));
    assert!(text(&help.stdout).contains("usage: fleet serve search"));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn the_fleet_serve_name_keeps_the_role_in_its_profile() {
    let (dir, link) = linked("fleet-serve");
    let out = run(&link, &["serve", "search"]);
    assert_eq!(out.status.code(), Some(2), "{}", text(&out.stderr));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn serve_without_a_role_names_search_among_the_roles() {
    let out = run(&fleet_bin(), &["serve"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(text(&out.stderr).contains("search"));
}

#[test]
fn the_registry_no_longer_calls_the_search_role_a_later_step() {
    let out = run(&fleet_bin(), &["help", "serve"]);
    assert_eq!(out.status.code(), Some(0));
    let o = text(&out.stdout);
    assert!(!o.contains("lands in a later step"), "{o}");
    assert!(o.contains("search"), "{o}");
}
