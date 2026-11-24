//! Android dependency-boundary check for the `id-client` facade.
//!
//! Slice 1 of #7938 (child #13403): the Android `arm64-v8a` graph must
//! contain the client-only facade and none of the server/runtime crates
//! (`wasmtime`, web/`axum`, SSH/`russh`) or server/listener owners
//! (`id-worlds`, `id-web`, `id-cli`). This test is the named automated
//! check that enforces the boundary: it fails when the facade gains a
//! forbidden edge and passes while the facade stays client-only.
//!
//! The check reads the facade's own `Cargo.toml` (compile-time
//! `include_str!`, so it cannot drift from the built manifest) and
//! asserts the forbidden names are absent. `cargo tree -p id-client`
//! remains the manual audit; this test is the gate.

/// Crates that must never appear in the `id-client` dependency closure.
const FORBIDDEN: &[&str] = &[
    "wasmtime",
    "axum",
    "russh",
    "hyper",
    "id-worlds",
    "id-web",
    "id-cli",
    "id-native-shell",
    "id-hw-shell",
];

/// Manifest dependency text of the facade under test: non-comment lines
/// only, so doc comments naming the excluded crates cannot trip the gate.
fn manifest_deps() -> String {
    include_str!("../Cargo.toml")
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

/// True when `manifest` names `crate_name` as a dependency edge: the name
/// occurs with a non-name character (or edge) on both sides, so `id-cli`
/// does not match `id-client`.
fn names_crate(manifest: &str, crate_name: &str) -> bool {
    let mut search_from = 0;
    while let Some(rel) = manifest[search_from..].find(crate_name) {
        let start = search_from + rel;
        let end = start + crate_name.len();
        let before_ok = start == 0 || !is_name_char(manifest.as_bytes()[start - 1]);
        let after_ok = end == manifest.len() || !is_name_char(manifest.as_bytes()[end]);
        if before_ok && after_ok {
            return true;
        }
        search_from = end;
    }
    false
}

#[allow(clippy::needless_pass_by_value)]
const fn is_name_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
}

#[test]
fn android_dependency_boundary_holds() {
    let manifest = manifest_deps();
    for forbidden in FORBIDDEN {
        assert!(
            !names_crate(&manifest, forbidden),
            "id-client must not depend on `{forbidden}` (Android client-only boundary)"
        );
    }
    // The facade must actually wire the client-safe leaves it claims.
    for required in ["id-core", "id-sync", "id-directory"] {
        assert!(
            manifest.contains(required),
            "id-client is expected to facade `{required}`"
        );
    }
}
