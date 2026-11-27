//! Oracle: #13279 part 2 — every Atlas binary's `--version` names
//! its own package version plus the baked source revision
//! (`<version>+g<rev>`), never a bare `0.1.0` that collides across
//! packages and cannot be traced to a source tree.

use std::process::Command;

use atlas_core::version::version_string;

fn version_output(binary: &str) -> String {
    let output = Command::new(binary)
        .arg("--version")
        .output()
        .expect("run --version");
    assert!(output.status.success(), "--version exits 0");
    String::from_utf8(output.stdout).expect("--version prints UTF-8")
}

#[test]
fn atlas_core_binaries_report_source_revision() {
    let expected = version_string(env!("CARGO_PKG_VERSION"));
    for (name, binary) in [
        ("atlas-classify", env!("CARGO_BIN_EXE_atlas-classify")),
        ("atlas-eval", env!("CARGO_BIN_EXE_atlas-eval")),
        (
            "atlas-github-comments",
            env!("CARGO_BIN_EXE_atlas-github-comments"),
        ),
        ("atlas-sweep", env!("CARGO_BIN_EXE_atlas-sweep")),
    ] {
        assert_eq!(version_output(binary), format!("{name} {expected}\n"));
    }
}
