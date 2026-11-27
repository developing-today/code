//! Exit-code family contract (#15766): standalone Atlas binaries
//! exit 0 on success, 2 on usage / invalid invocation, and 1 on
//! execution failure. Fleet-level codes (search 3, capability 69)
//! are out of scope here.
//!
//! Oracle: the recorded decision on #15766 — atlas-classify and
//! atlas-sweep must not report runtime failures as 2, and
//! atlas-github-comments must not report argument errors as 1.

use std::process::{Command, Output};

fn run(binary: &str, args: &[&str]) -> Output {
    Command::new(binary).args(args).output().unwrap()
}

fn code(out: &Output) -> i32 {
    out.status.code().unwrap()
}

#[test]
fn atlas_classify_exit_codes() {
    let bin = env!("CARGO_BIN_EXE_atlas-classify");
    // Usage: bare invocation, unknown action, invalid flag value.
    assert_eq!(code(&run(bin, &[])), 2);
    assert_eq!(code(&run(bin, &["--db", "x.db", "bogus"])), 2);
    assert_eq!(
        code(&run(bin, &["--db", "x.db", "run", "--window-days", "abc"])),
        2
    );
    // Execution: the corpus cannot be opened.
    assert_eq!(
        code(&run(bin, &["--db", "/nonexistent-dir-15766/x.db", "run"])),
        1
    );
    // Usage wins over an unreadable input when a required flag is absent.
    // Oracle: the #15766 decision classifies invalid invocations as 2.
    assert_eq!(
        code(&run(
            bin,
            &[
                "--db",
                "x.db",
                "schema-review",
                "--old-schema",
                "/nonexistent-dir-15766/old.json",
            ]
        )),
        2
    );
    assert_eq!(
        code(&run(
            bin,
            &[
                "--db",
                "x.db",
                "schema-apply",
                "--review",
                "/nonexistent-dir-15766/review.json",
            ]
        )),
        2
    );
}

#[test]
fn atlas_sweep_exit_codes() {
    let bin = env!("CARGO_BIN_EXE_atlas-sweep");
    // Usage: missing --state-dir, unknown action, bad source class.
    assert_eq!(code(&run(bin, &[])), 2);
    assert_eq!(code(&run(bin, &["--state-dir", "/tmp", "bogus-action"])), 2);
    assert_eq!(
        code(&run(
            bin,
            &[
                "--state-dir",
                "/tmp",
                "pause-class",
                "--class",
                "nope",
                "--host",
                "h"
            ]
        )),
        2
    );
    // Execution: the request file cannot be read.
    assert_eq!(
        code(&run(
            bin,
            &[
                "--state-dir",
                "/tmp",
                "run-once",
                "/nonexistent-dir-15766/req.json"
            ]
        )),
        1
    );
}

#[test]
fn atlas_github_comments_exit_codes() {
    let bin = env!("CARGO_BIN_EXE_atlas-github-comments");
    // Usage: missing required flags, unknown argument.
    assert_eq!(code(&run(bin, &[])), 2);
    assert_eq!(code(&run(bin, &["--bogus"])), 2);
    // Execution: the watermark lock cannot be opened.
    assert_eq!(
        code(&run(
            bin,
            &[
                "--state",
                "/nonexistent-dir-15766/state.json",
                "--out",
                "/nonexistent-dir-15766/out.jsonl",
                "--repo",
                "developing-today/code",
            ]
        )),
        1
    );
}
