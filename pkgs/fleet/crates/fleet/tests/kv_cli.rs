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
        .env_remove("FLEET_KV_URL")
        .env_remove("FLEET_KV_TOKEN_FILE")
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
fn bad_arguments_and_unconfigured_central_are_answered_before_anything_is_created() {
    let dir = scratch("strict");
    let usage = fleet(&dir, &["kv", "get", "--namespace", "n", "k", "extra"], None);
    assert_eq!(usage.status.code(), Some(2), "{}", stderr(&usage));
    // With no central store configured, --global, sync, backup and
    // restore --archive exit 69 before the session's identity or the local
    // store is touched.
    for words in [
        &["kv", "list", "--global", "--namespace", "n"][..],
        &["kv", "sync"][..],
        &["kv", "backup"][..],
        &["kv", "restore", "--archive"][..],
    ] {
        let global = fleet(&dir, words, None);
        assert_eq!(global.status.code(), Some(69), "{}", stderr(&global));
        assert!(
            stderr(&global).contains("central_not_configured"),
            "{}",
            stderr(&global)
        );
    }
    let blob = fleet(
        &dir,
        &["kv", "set", "--force-blob", "--namespace", "n", "k", "v"],
        None,
    );
    assert_eq!(blob.status.code(), Some(69));
    assert!(stderr(&blob).contains("pending_needs_phase3"));
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

#[test]
fn a_snapshot_restores_on_a_fresh_host_through_the_binary() {
    let source = scratch("snap-source");
    let fresh = scratch("snap-fresh");
    for host in [&source, &fresh] {
        fleet_kv::launch::write(
            &host.join("state").join("fleet"),
            SESSION,
            "user-1",
            "ctx-1",
        )
        .unwrap();
    }
    let set = fleet(
        &source,
        &["kv", "set", "--namespace", "notes", "k", "kept"],
        None,
    );
    assert_eq!(set.status.code(), Some(0), "{}", stderr(&set));
    let snapshot = source.join("snap.db");
    let snapshot_arg = snapshot.to_str().unwrap();
    let taken = fleet(&source, &["kv", "snapshot", snapshot_arg], None);
    assert_eq!(taken.status.code(), Some(0), "{}", stderr(&taken));
    assert!(
        String::from_utf8_lossy(&taken.stdout).contains(" overlay_rows=1 "),
        "{}",
        String::from_utf8_lossy(&taken.stdout)
    );

    let restored = fleet(&fresh, &["kv", "restore", snapshot_arg], None);
    assert_eq!(restored.status.code(), Some(0), "{}", stderr(&restored));
    assert_eq!(restored.stdout, taken.stdout);
    let get = fleet(&fresh, &["kv", "get", "--namespace", "notes", "k"], None);
    assert_eq!(get.status.code(), Some(0), "{}", stderr(&get));
    assert_eq!(get.stdout, b"kept");

    let again = fleet(&fresh, &["kv", "restore", snapshot_arg], None);
    assert_eq!(again.status.code(), Some(1));
    assert!(
        stderr(&again).contains("backup_target_exists"),
        "{}",
        stderr(&again)
    );
    let _ = fs::remove_dir_all(source);
    let _ = fs::remove_dir_all(fresh);
}

#[test]
fn host_verbs_fail_closed_without_a_launch_record() {
    let dir = scratch("host-closed");
    for verb in ["snapshot", "restore"] {
        let output = fleet(&dir, &["kv", verb, "x.db"], None);
        assert_eq!(output.status.code(), Some(69), "{verb}");
        assert!(stderr(&output).contains("identity_unavailable"), "{verb}");
    }
    assert!(!dir.join("x.db").exists());
    assert!(
        !dir.join("state")
            .join("fleet")
            .join("kv")
            .join("local.db")
            .exists()
    );
    let _ = fs::remove_dir_all(dir);
}
