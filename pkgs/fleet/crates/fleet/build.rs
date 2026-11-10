//! Bake the source revision into the binary for `fleet version`.
//!
//! Resolution: `$FLEET_SOURCE_REV` (the nix build passes the flake's
//! rev) → `git rev-parse` in the repo root (developer builds) →
//! "unknown". The nix sandbox has no `.git`, so without the env var a
//! packaged build honestly reports "unknown" rather than inventing one.

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=FLEET_SOURCE_REV");
    let rev = std::env::var("FLEET_SOURCE_REV")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
            let root = std::path::Path::new(&manifest).join("../../..");
            Command::new("git")
                .args(["rev-parse", "--short=12", "HEAD"])
                .current_dir(root)
                .output()
                .ok()
                .filter(|o| o.status.success())
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "unknown".to_string())
        });
    println!("cargo:rustc-env=FLEET_SOURCE_REV={rev}");
}
