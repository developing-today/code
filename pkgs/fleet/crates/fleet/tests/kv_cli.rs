//! `fleet kv` through the real binary (code#13314): strict parsing before
//! session resolution, fail-closed launch identity, and a local round trip.
#![cfg(feature = "kv")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const SESSION: &str = "ses_kvclitest";

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fleet-kv-cli-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn fleet(dir: &Path, args: &[&str], stdin: Option<&[u8]>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_fleet"));
    command
        .args(args)
        .current_dir(dir)
        .env("HOME", dir)
        .env("XDG_STATE_HOME", dir.join("state"))
        .env("FLEET_INVOCATION_LEDGER", dir.join("ledger.jsonl"))
        .env("FLEET_SESSION_ID", SESSION)
        .env_remove("OPENCODE_SESSION_ID")
        .env("FLEET_CTX_USER", "spoofed-user");
    match stdin {
        None => command.output().unwrap(),
        Some(bytes) => {
            use std::io::Write;
            command
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
            let mut child = command.spawn().unwrap();
            child.stdin.take().unwrap().write_all(bytes).unwrap();
            child.wait_with_output().unwrap()
        }
    }
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn bad_arguments_and_global_are_answered_before_anything_is_created() {
    let dir = scratch("strict");
    let usage = fleet(&dir, &["kv", "get", "--namespace", "n", "k", "extra"], None);
    assert_eq!(usage.status.code(), Some(2), "{}", stderr(&usage));
    let global = fleet(&dir, &["kv", "list", "--global", "--namespace", "n"], None);
    assert_eq!(global.status.code(), Some(69));
    assert!(stderr(&global).contains("unimplemented_central_in_phase1"));
    let help = fleet(&dir, &["kv", "--help"], None);
    assert_eq!(help.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&help.stdout).contains("clear-override"));
    assert!(!dir.join("state").join("fleet").join("kv").exists());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn no_launch_record_fails_closed_even_with_ctx_overrides() {
    let dir = scratch("closed");
    let output = fleet(&dir, &["kv", "list", "--namespace", "n"], None);
    assert_eq!(output.status.code(), Some(69));
    assert!(
        stderr(&output).contains("identity_unavailable"),
        "{}",
        stderr(&output)
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn a_launch_record_enables_a_local_round_trip() {
    let dir = scratch("round");
    fleet_kv::launch::write(&dir.join("state").join("fleet"), SESSION, "user-1", "ctx-1").unwrap();
    let set = fleet(
        &dir,
        &["kv", "set", "--namespace", "notes", "--stdin", "a/b"],
        Some(b"bytes\x00here"),
    );
    assert_eq!(set.status.code(), Some(0), "{}", stderr(&set));
    assert_eq!(set.stdout, b"1\n");
    let get = fleet(&dir, &["kv", "get", "--namespace", "notes", "a/b"], None);
    assert_eq!(get.status.code(), Some(0), "{}", stderr(&get));
    assert_eq!(get.stdout, b"bytes\x00here");
    let delete = fleet(&dir, &["kv", "delete", "--namespace", "notes", "a/b"], None);
    assert_eq!(delete.status.code(), Some(0));
    let list = fleet(
        &dir,
        &["kv", "list", "--namespace", "notes", "--include-tombstones"],
        None,
    );
    assert_eq!(list.stdout, b"a/b\t2\tmasked\n");
    let _ = fs::remove_dir_all(dir);
}
