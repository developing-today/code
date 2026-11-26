//! Oracle: #13279 part 2 — the clap-based Atlas binaries report the
//! same `<version>+g<source revision>` shape as the hand-rolled
//! family members, not clap's bare CARGO_PKG_VERSION.

use std::process::Command;

fn version_output(binary: &str) -> String {
    let output = Command::new(binary)
        .arg("--version")
        .output()
        .expect("run --version");
    assert!(output.status.success(), "--version exits 0");
    String::from_utf8(output.stdout).expect("--version prints UTF-8")
}

#[test]
fn atlas_search_binaries_report_source_revision() {
    let expected = atlas_core::version::version_string(env!("CARGO_PKG_VERSION"));
    for (name, binary) in [
        ("atlas-search", env!("CARGO_BIN_EXE_atlas-search")),
        ("atlas-stage", env!("CARGO_BIN_EXE_atlas-stage")),
    ] {
        assert_eq!(version_output(binary), format!("{name} {expected}\n"));
    }
}
