//! CLI-level coverage for custom certificate profile scaffolding.
//!
//! Oracle: `fleet cert profile validate` accepts the scaffold emitted by
//! `fleet cert profile scaffold`, and rejects malformed JSON and profiles
//! that violate the custom-profile policy.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "fleet-cert-profile-cli-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("create temp directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fleet(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fleet"))
        .args(args)
        .output()
        .expect("run fleet")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn scaffold_output_is_accepted_by_profile_validate() {
    let temp = TempDir::new();
    let profile = temp.path().join("edge-server.json");
    let profile_arg = profile.to_str().expect("UTF-8 temp path");

    let scaffold = fleet(&[
        "cert",
        "profile",
        "scaffold",
        "--name",
        "edge-server",
        "--out",
        profile_arg,
    ]);
    assert_eq!(scaffold.status.code(), Some(0), "{}", stderr(&scaffold));
    assert!(
        profile.is_file(),
        "scaffold should write the requested file"
    );

    let validate = fleet(&["cert", "profile", "validate", "--json", profile_arg]);
    assert_eq!(validate.status.code(), Some(0), "{}", stderr(&validate));
    let result: serde_json::Value =
        serde_json::from_slice(&validate.stdout).expect("validate --json output");
    assert_eq!(result["valid"], true);
    assert_eq!(result["profile"]["name"], "edge-server");
}

#[test]
fn profile_validate_rejects_malformed_json() {
    let temp = TempDir::new();
    let profile = temp.path().join("malformed.json");
    std::fs::write(&profile, b"{ not json").expect("write malformed profile");

    let output = fleet(&[
        "cert",
        "profile",
        "validate",
        profile.to_str().expect("UTF-8 temp path"),
    ]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(stderr(&output).contains("malformed custom profile"));
}

#[test]
fn profile_validate_rejects_policy_invalid_profile() {
    let temp = TempDir::new();
    let profile = temp.path().join("overlong.json");
    let invalid = serde_json::json!({
        "schema": "fleet-cert-profile/v1",
        "name": "edge-server",
        "key_algorithm": "p256",
        "key_usages": ["digital-signature"],
        "extended_key_usages": ["server-auth"],
        "default_validity_days": 90,
        "max_validity_days": 1096,
        "san_defaults_to_cn": true,
        "notes": "exceeds the custom profile policy ceiling"
    });
    std::fs::write(&profile, serde_json::to_vec(&invalid).unwrap())
        .expect("write policy-invalid profile");

    let output = fleet(&[
        "cert",
        "profile",
        "validate",
        profile.to_str().expect("UTF-8 temp path"),
    ]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(stderr(&output).contains("policy ceiling"));
}

#[test]
fn scaffold_does_not_overwrite_an_existing_file() {
    let temp = TempDir::new();
    let profile = temp.path().join("existing.json");
    std::fs::write(&profile, b"keep this content").expect("write existing file");

    let output = fleet(&[
        "cert",
        "profile",
        "scaffold",
        "--name",
        "edge-server",
        "--out",
        profile.to_str().expect("UTF-8 temp path"),
    ]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert_eq!(std::fs::read(&profile).unwrap(), b"keep this content");
}
