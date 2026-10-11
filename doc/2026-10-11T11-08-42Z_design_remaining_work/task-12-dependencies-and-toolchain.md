# Task 12: update all dependencies and the Rust toolchain

- Date: 2026-10-11 (UTC 12:20)
- Status: Open. Start after task 01 (the tuple fix is in; pull pages, MSRV and dial deadlines remain), and before task 02, so the storage work starts on current crates.
- Owner request: "update all dependencies and update rust toolchain to latest version in nixpkgs (or latest nightly only if required)".

## Where things stand (checked 2026-10-11)

| Item | Current in `pkgs/id` | Latest found | Notes |
| --- | --- | --- | --- |
| Rust toolchain | 1.97.0 (`rust-toolchain.toml`) | 1.99.0 (stable, 2026-09-28) | nixpkgs store on this machine has rustc 1.98.1 and 1.99.0. |
| Nightly | none | not needed | `#![feature` found nowhere in `pkgs/`. |
| MSRV | declared 1.91 in `pkgs/id/Cargo.toml` | p2panda and the spikes need 1.96 | Discrepancy, also in task 01. |
| p2panda | git rev `3e84cac` (2026-10-09) | crates.io 0.7.1 (2026-08-21) for core, store, stream, sync | We are ahead of 0.7.1 on purpose. |
| iroh stack | iroh 1.3, iroh-relay 1.3, iroh-tickets 1.0, iroh-blobs =0.103.1, iroh-docs 0.101, iroh-gossip 0.101 | (check) | iroh-blobs 0.97.0, 0.99.0, 0.100.0 to 0.103.0 and 0.28.0 are yanked. 0.103.1 is not. |
| redb | =4.3.0 | (check) | Also used by iroh-blobs' FsStore. |
| CBOR | cbor-core =0.10.1; dcbor =0.25.2 (dev) | (check) | cbor-core is the canonical encoder the headers use. |
| ed25519 | ed25519-dalek 3.0 in the product; p2panda-core uses 2.2 | (check) | Index decision 19. |
| SQLite family | libsql 0.9.30, optional behind `web` | Turso 0.8.2 (candidate) | Depends on the storage decision (task 02). |
| distributed-topic-tracker | 0.3.5 | (check) | Pulled in by iroh-related code. |

## Procedure

1. Inventory: `cargo tree -d` for duplicate versions, and `cargo update --dry-run` for what would move. Record the output in a dated section of this file.
2. Change one group at a time, and commit each group separately:
   1. Toolchain 1.97.0 to 1.99.0. Change `rust-toolchain.toml` and the flake's rust-overlay input if needed. Build with `nix build` before committing, as the comment in `rust-toolchain.toml` asks.
   2. The iroh stack (iroh, iroh-relay, iroh-tickets, iroh-blobs, iroh-docs, iroh-gossip, iroh-mdns). Watch for API changes in the connectivity module and the dial helper.
   3. The p2panda rev (see decision A).
   4. redb, cbor-core, dcbor, ed25519-dalek.
   5. Everything else, with a minimal `cargo update`.
3. For each group run the same checks:
   - `cargo test --lib -- --skip world_compile --skip test_worlds_compile_roc_sources_on_the_fly --skip test_world_module_shared_over_iroh_and_web_presentations` (the three are pre-existing failures; see the feature doc).
   - `cargo test --doc`.
   - `rustfmt --edition 2024 --check` on changed files.
   - `cargo clippy --lib --tests -- -D warnings`. Use one toolchain and one target dir for all of it. Mixing rustc builds in one target dir gives E0514 errors, which is what happened on 2026-10-11.
   - When iroh changes: `cargo test -- --ignored test_n0_endpoints_dial_by_id` (a live n0 dial).
4. Record the versions before and after, and the Cargo.lock diff size, in a dated section.

## Acceptance

- The same tests pass as before, and clippy reports nothing new.
- `nix build` succeeds on the new toolchain.
- No yanked crate in `Cargo.lock`.
- Each group is a separate commit, with its version table in the message body.

## Decisions for the owner

- **A. p2panda source.** Stay on the git rev (preferred: we need fixes that are not in 0.7.1), or move to crates.io 0.7.x and carry patches (alternative).
- **B. Toolchain step.** Jump to 1.99.0 in one step (preferred), or stay at 1.97.0 until the next dependency pass (alternative).
- **C. ed25519-dalek.** Align with p2panda-core's 2.2 (preferred, one copy in the tree), or keep 3.0 and shim at the boundary (alternative). Index decision 19.
- **D. libsql.** Keep for web identity (alternative: remove it when storage moves to redb), decided together with the storage question in task 02.

## Risks

- iroh API changes since 1.3. The connectivity module and the dial helper are the first to notice.
- redb 4.x format and API changes. Any store opened by an older version needs a rebuild, which is acceptable because the data can be discarded.
- libsql builds C code. Check it on the new toolchain.
- Feature unification could pull in dependencies we don't want. Check `cargo tree -e features` after each group.
