use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

fn empty_home(binary: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after Unix epoch")
        .as_nanos();
    let home = std::env::temp_dir().join(format!(
        "fleet-queue-{binary}-argv-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir_all(&home).expect("create isolated HOME");
    home
}

fn run(binary_path: &str, home: &std::path::Path, args: &[&str]) -> Output {
    Command::new(binary_path)
        .args(args)
        .env("HOME", home)
        .output()
        .expect("run standalone broker")
}

fn assert_argv_contract(binary: &str, binary_path: &str) {
    let home = empty_home(binary);

    for flag in ["--help", "-h"] {
        let output = run(binary_path, &home, &[flag]);
        assert!(output.status.success(), "{binary} {flag}: {output:?}");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains(&format!("usage: {binary}")), "{stdout}");
        assert!(!String::from_utf8_lossy(&output.stderr).contains("token file"));
    }

    for arg in ["--bogus-junk", "frobnicate"] {
        let output = run(binary_path, &home, &[arg]);
        assert_eq!(output.status.code(), Some(2), "{binary} {arg}: {output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(arg), "{stderr}");
        assert!(!stderr.contains("token file"), "{stderr}");
    }

    // With an empty HOME, bare invocation must continue into the existing
    // startup path, which fails at its first step while reading the token.
    let output = run(binary_path, &home, &[]);
    assert_eq!(output.status.code(), Some(3), "{binary} bare: {output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("token file"),
        "{output:?}"
    );

    std::fs::remove_dir_all(home).expect("remove isolated HOME");
}

#[test]
fn queue_broker_checks_arguments_before_startup() {
    assert_argv_contract("queue-broker", env!("CARGO_BIN_EXE_queue-broker"));
}

#[test]
fn secrets_broker_checks_arguments_before_startup() {
    assert_argv_contract("secrets-broker", env!("CARGO_BIN_EXE_secrets-broker"));
}
