//! Exercise the production CLI across multiple runs using the synthetic gh fixture.
use std::process::Command;

#[test]
fn github_comments_checkpoint_preserves_eligible_records() {
    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/test_github_comments_checkpoint.py"
    );
    let status = Command::new("python3")
        .arg(fixture)
        .arg(env!("CARGO_BIN_EXE_atlas-github-comments"))
        .status()
        .expect("run Python 3 synthetic GitHub CLI fixture");
    assert!(status.success(), "checkpoint CLI oracle failed: {status}");
}
