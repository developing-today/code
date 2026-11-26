//! Argv parity for `POST /v1/runs` (#10392 §6.1, #14224).
//!
//! The oracle is the CLI itself (design §7): for each argv, a run through
//! the REST run path and a direct invocation of the same `id` binary must
//! give the same stdout, stderr and exit code. Every top-level subcommand is
//! covered through its `--help` surface, which needs no store, plus the
//! version, the top-level help and a parse error.
//!
//! The HTTP layer (loopback gate, request validation, bounds) is covered by
//! the unit tests in `crates/id-web/src/runs.rs`; this test drives the run
//! path those handlers call, against the real binary.

#![cfg(feature = "web")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::process::Command;

use clap::CommandFactory;
use id::web::runs::{RunRequest, RunsConfig, StdinMode, execute, validate};

fn id_exe() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_id"))
}

/// Run `argv` both ways and assert the results match.
async fn assert_parity(argv: &[String]) {
    let request: RunRequest = serde_json::from_value(serde_json::json!({
        "argv": argv,
        "stdin": { "mode": "none" }
    }))
    .unwrap();
    assert_eq!(request.stdin.mode, StdinMode::None);
    let stdin = validate(&request).unwrap();
    let rest = execute(&RunsConfig::new(id_exe()), &request, stdin)
        .await
        .unwrap();

    let cli = Command::new(id_exe())
        .args(argv)
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();

    assert!(!rest.timed_out, "{argv:?} timed out over REST");
    assert_eq!(rest.exit_code, cli.status.code(), "exit code for {argv:?}");
    assert_eq!(rest.stdout.encoding, "utf8", "stdout encoding for {argv:?}");
    assert_eq!(
        rest.stdout.data,
        String::from_utf8(cli.stdout).unwrap(),
        "stdout for {argv:?}"
    );
    assert_eq!(rest.stderr.encoding, "utf8", "stderr encoding for {argv:?}");
    assert_eq!(
        rest.stderr.data,
        String::from_utf8(cli.stderr).unwrap(),
        "stderr for {argv:?}"
    );
}

#[tokio::test]
async fn every_subcommand_help_matches_the_cli() {
    let subcommands: Vec<String> = id::Cli::command()
        .get_subcommands()
        .filter(|sub| !sub.is_hide_set())
        .map(|sub| sub.get_name().to_owned())
        .collect();
    assert!(!subcommands.is_empty(), "the CLI lists no subcommands");
    for name in subcommands {
        assert_parity(&[name, "--help".to_owned()]).await;
    }
}

#[tokio::test]
async fn version_help_and_parse_errors_match_the_cli() {
    for argv in [
        vec!["--version"],
        vec!["--help"],
        vec!["--definitely-not-a-flag"],
        vec!["definitely-not-a-subcommand"],
    ] {
        let argv: Vec<String> = argv.into_iter().map(str::to_owned).collect();
        assert_parity(&argv).await;
    }
}

#[tokio::test]
async fn body_stdin_reaches_the_binary_like_a_pipe() {
    // A parse error does not read stdin; the run must still finish with the
    // same answer as the CLI when input is supplied and then closed.
    let request: RunRequest = serde_json::from_value(serde_json::json!({
        "argv": ["--definitely-not-a-flag"],
        "stdin": { "mode": "body", "data": "ignored input\n" }
    }))
    .unwrap();
    let stdin = validate(&request).unwrap();
    assert_eq!(stdin.as_deref(), Some(&b"ignored input\n"[..]));
    let rest = execute(&RunsConfig::new(id_exe()), &request, stdin)
        .await
        .unwrap();
    let cli = Command::new(id_exe())
        .arg("--definitely-not-a-flag")
        .output()
        .unwrap();
    assert_eq!(rest.exit_code, cli.status.code());
    assert_eq!(rest.stderr.data, String::from_utf8(cli.stderr).unwrap());
}
