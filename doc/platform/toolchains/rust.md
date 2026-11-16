# Rust (rustup) — Setup Cycle

Verified 2026-10-21 **with the env vars below set**: rustup toolchain list shows `stable-x86_64-unknown-linux-gnu (active, default)` plus `1.99.0-x86_64-unknown-linux-gnu`; `cargo +stable --version` → cargo 1.98.1.

## Download / Install
- rustup home and cargo home live on persistent disk:
  - `RUSTUP_HOME=~/tools/rustup`
  - `CARGO_HOME=~/tools/cargo` — cargo binary at `~/tools/cargo/bin/cargo`, full tool suite alongside it (rustc, rustfmt, clippy-driver, rust-analyzer, rustdoc, rustup shim).
- Dispatcher stubs in `~/tools/bin/` (`cargo`, `rustc`, `rustfmt`, `rustup`) are tiny wrappers, not the real binaries.

## Gather secrets
- None for the toolchain itself.

## Configure
- **The env vars are not optional.** Every shell that touches Rust must have:
  ```bash
  export CARGO_HOME=$HOME/tools/cargo
  export RUSTUP_HOME=$HOME/tools/rustup
  ```
- Proof from the 2026-10-21 check: invoking `~/tools/cargo/bin/cargo --version` *without* that env failed with "rustup could not choose a version of cargo to run, because one wasn't specified explicitly, and no default is configured" — in a bare exec the rustup home wasn't found, so the (real, existing) default toolchain was invisible. With the env set, the same binary reports stable as active/default. If cargo "forgets" its toolchain, check env before reinstalling anything.

## Test
```bash
export CARGO_HOME=$HOME/tools/cargo RUSTUP_HOME=$HOME/tools/rustup
rustup toolchain list        # expected: stable-x86_64-unknown-linux-gnu (active, default)
cargo +stable --version      # expected: cargo 1.98.1
```

## Known quirks & what has NOT worked
- Attic follow-up (2026-10-05, Rust-adjacent): the Attic *server* (atticd 0.1.0) was built with cargo and runs, but the official Attic *client* could not be built here — it links nix C++ libraries (nix-main >= 2.24 via pkg-config) needing a deep transitive dev chain unavailable for our nix 2.35.2, and disabling the `nix_store` feature breaks the client (it uses the C++ bindings directly). Long cargo release builds are also slow on this box (20+ min observed for `cargo install --git` in release mode) — budget accordingly and don't mistake a long compile for a hang.
- Cargo git checkouts/registry live under `~/tools/cargo/` (persistent) — a recycle does not force a full re-fetch, unlike overlay-installed tooling.
