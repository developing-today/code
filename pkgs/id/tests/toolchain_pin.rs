//! The Nix check must install the Rust version that `rust-toolchain.toml` pins.
//!
//! `default.nix` once ran `rustup default 1.89.0` while the pin said something
//! else, so the Nix check tested a different compiler from the one everyone
//! else builds with. These tests read both files and fail when they disagree.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::Path;
use std::process::Command;

const ROC_EXAMPLES: [&str; 7] = [
    "lounge",
    "roc-chat",
    "roc-counter",
    "roc-grid",
    "roc-screen",
    "territory",
    "tic-tac-toe",
];

/// The Nix side of the pin: a literal version, or a value read from the file.
#[derive(Debug, PartialEq, Eq)]
enum NixVersion {
    /// A hard-coded version, such as `rustup default 1.89.0`.
    Literal(String),
    /// Read from `rust-toolchain.toml`, so it cannot drift from the pin.
    FromToolchainFile,
}

/// The channel from the `[toolchain]` table, without the trailing comment.
fn pinned_channel(toml: &str) -> String {
    let line = toml
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("channel"))
        .expect("rust-toolchain.toml has a channel line");
    let value = line.split_once('=').expect("channel line has '='").1;
    let value = value.split('#').next().unwrap().trim();
    value.trim_matches('"').to_owned()
}

/// A line of Nix without its `#` comment, trimmed.
fn code_of(line: &str) -> &str {
    line.split('#').next().unwrap_or_default().trim()
}

/// The argument `default.nix` passes to `rustup default`. Only code counts, so
/// a commented-out line or binding is ignored.
fn nix_version(nix: &str) -> NixVersion {
    let code: Vec<&str> = nix.lines().map(code_of).collect();
    let line = code
        .iter()
        .find(|line| line.contains("rustup default"))
        .expect("default.nix runs `rustup default`");
    let arg = line.split("rustup default").nth(1).unwrap().trim();
    let Some(name) = arg
        .strip_prefix("${")
        .and_then(|rest| rest.strip_suffix('}'))
    else {
        return NixVersion::Literal(arg.trim_matches('"').to_owned());
    };
    let bound_from_file = code
        .iter()
        .any(|line| line.contains("builtins.fromTOML (builtins.readFile ./rust-toolchain.toml)"))
        && code.iter().any(|line| {
            line.starts_with(&format!("{name} =")) && line.contains(".toolchain.channel")
        });
    assert!(
        bound_from_file,
        "default.nix passes {arg} to rustup, but it is not bound to the channel in rust-toolchain.toml"
    );
    NixVersion::FromToolchainFile
}

fn read_pin_files() -> (String, String) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let toml = fs::read_to_string(root.join("rust-toolchain.toml")).unwrap();
    let nix = fs::read_to_string(root.join("default.nix")).unwrap();
    (toml, nix)
}

#[test]
fn test_roc_examples_and_nix_package_share_the_compiler_pin() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let pin = fs::read_to_string(root.join("nix/roc-version")).unwrap();
    let pinned_version = pin.trim();
    assert!(!pinned_version.is_empty(), "nix/roc-version is empty");

    let roc_nix = fs::read_to_string(root.join("nix/roc.nix")).unwrap();
    assert!(
        roc_nix.contains("tag = lib.removeSuffix \"\\n\" (builtins.readFile ./roc-version);")
            && roc_nix.contains("lib.removePrefix \"nightly-\" tag"),
        "nix/roc.nix must derive its download tag from nix/roc-version"
    );

    for example in ROC_EXAMPLES {
        let script = fs::read_to_string(root.join(format!("examples/{example}/build.sh"))).unwrap();
        assert!(
            script.contains("source ../roc-toolchain.sh"),
            "examples/{example}/build.sh must check the shared Roc compiler pin"
        );
    }
}

#[cfg(unix)]
#[test]
fn test_example_build_refuses_a_different_roc_before_building() {
    use std::os::unix::fs::PermissionsExt;

    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let temp = tempfile::tempdir().unwrap();
    let bash = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|directory| directory.join("bash"))
        .find(|candidate| candidate.is_file())
        .and_then(|candidate| fs::canonicalize(candidate).ok())
        .expect("bash is available on PATH");
    let roc_stub = temp.path().join("roc-stub");
    let build_marker = temp.path().join("build-was-run");
    fs::write(
        &roc_stub,
        format!(
            "#!{}\nif [[ \"${{1:-}}\" == version ]]; then\n  echo 'Roc compiler version unpinned-test'\nelse\n  : > \"$ROC_BUILD_MARKER\"\nfi\n",
            bash.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&roc_stub, fs::Permissions::from_mode(0o755)).unwrap();

    let output = Command::new("bash")
        .arg("examples/territory/build.sh")
        .current_dir(root)
        .env("ROC", &roc_stub)
        .env("ROC_BUILD_MARKER", &build_marker)
        .output()
        .unwrap();

    assert!(
        !output.status.success(),
        "an unpinned Roc compiler was accepted"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    let pinned_version = fs::read_to_string(root.join("nix/roc-version")).unwrap();
    assert!(stderr.contains(pinned_version.trim()), "{stderr}");
    assert!(
        stderr.contains("Roc compiler version unpinned-test"),
        "{stderr}"
    );
    assert!(
        !build_marker.exists(),
        "the build command ran before refusal"
    );
}

#[test]
fn nix_check_installs_the_channel_pinned_in_rust_toolchain_toml() {
    let (toml, nix) = read_pin_files();
    let channel = pinned_channel(&toml);
    if let NixVersion::Literal(version) = nix_version(&nix) {
        assert_eq!(
            version, channel,
            "default.nix runs `rustup default {version}`, but rust-toolchain.toml pins {channel}"
        );
    }
}

#[test]
fn pinned_channel_ignores_the_trailing_comment() {
    let toml =
        "[toolchain]\nchannel = \"1.99.0\" # WARNING: some note\ncomponents = [\"clippy\"]\n";
    assert_eq!(pinned_channel(toml), "1.99.0");
}

#[test]
fn literal_rustup_version_is_reported_as_a_literal() {
    let nix = "    rustup default 1.89.0\n    rustup component add clippy rustfmt\n";
    assert_eq!(nix_version(nix), NixVersion::Literal("1.89.0".to_owned()));
}

/// Each shape of `rustup default` line is read as the version it names. A
/// quoted version is the same version, and a commented-out line is not a line.
#[test]
fn quoted_and_commented_rustup_lines_are_read_as_the_version_they_name() {
    let cases = [
        ("    rustup default 1.99.0\n", "1.99.0"),
        ("    rustup default \"1.99.0\"\n", "1.99.0"),
        (
            "    # rustup default 1.89.0\n    rustup default 1.99.0\n",
            "1.99.0",
        ),
    ];
    let wrong: Vec<String> = cases
        .into_iter()
        .filter_map(|(nix, version)| {
            let got = nix_version(nix);
            (got != NixVersion::Literal(version.to_owned()))
                .then(|| format!("{nix:?} gave {got:?}"))
        })
        .collect();
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// A binding counts only when its code names the channel. A comment that
/// mentions the channel binds nothing, so the check must refuse it.
#[test]
fn a_binding_that_only_mentions_the_channel_in_a_comment_is_refused() {
    let bindings = [
        "rustVersion = \"1.89.0\"; # toolchainToml.toolchain.channel",
        "rustVersion = \"1.89.0\"; # builtins.readFile ./rust-toolchain.toml",
    ];
    for binding in bindings {
        let nix = format!(
            "  toolchainToml = builtins.fromTOML (builtins.readFile ./rust-toolchain.toml);\n  {binding}\n    rustup default ${{rustVersion}}\n"
        );
        let refused = std::panic::catch_unwind(|| nix_version(&nix)).is_err();
        assert!(refused, "accepted as bound to the pin: {binding:?}");
    }
}

/// The Rust version a line of prose names, such as `Rust 1.89.0`.
fn named_rust_version(line: &str) -> Option<&str> {
    let rest = line.split("Rust ").nth(1)?;
    let end = rest
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(rest.len());
    let version = rest[..end].trim_end_matches('.');
    let dots = version.matches('.').count();
    (dots == 2 && version.starts_with(|c: char| c.is_ascii_digit())).then_some(version)
}

/// The root docs describe the dev shell, so each Rust version they name must be
/// the pinned one. A doc written before the pin moved would otherwise go stale.
#[test]
fn the_root_docs_name_the_pinned_rust_version() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let pinned = pinned_channel(&fs::read_to_string(root.join("rust-toolchain.toml")).unwrap());
    let mut stale = Vec::new();
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap();
        for (index, line) in text.lines().enumerate() {
            if let Some(version) = named_rust_version(line)
                && version != pinned
            {
                stale.push(format!("{}:{}: Rust {version}", path.display(), index + 1));
            }
        }
    }
    assert!(
        stale.is_empty(),
        "docs name a Rust version other than the pin {pinned}:\n{}",
        stale.join("\n")
    );
}
