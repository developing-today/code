//! The Nix check must install the Rust version that `rust-toolchain.toml` pins.
//!
//! `default.nix` once ran `rustup default 1.89.0` while the pin said something
//! else, so the Nix check tested a different compiler from the one everyone
//! else builds with. These tests read both files and fail when they disagree.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::Path;

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

/// The argument `default.nix` passes to `rustup default`.
fn nix_version(nix: &str) -> NixVersion {
    let line = nix
        .lines()
        .find(|line| line.contains("rustup default"))
        .expect("default.nix runs `rustup default`");
    let arg = line.split("rustup default").nth(1).unwrap().trim();
    let Some(name) = arg
        .strip_prefix("${")
        .and_then(|rest| rest.strip_suffix('}'))
    else {
        return NixVersion::Literal(arg.to_owned());
    };
    let bound_from_file = nix
        .contains("builtins.fromTOML (builtins.readFile ./rust-toolchain.toml)")
        && nix.lines().any(|line| {
            let line = line.trim();
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
