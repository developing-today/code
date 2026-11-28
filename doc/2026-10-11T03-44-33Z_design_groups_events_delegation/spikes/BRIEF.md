# Shared brief for the design spikes (2026-10-11)

You are running one spike for the owner's identity/P2P system (`pkgs/id`, Rust). The design is still open. Your result feeds a decision list, so report numbers you actually observed, and say plainly what you did not test.

## Where things are

- Remaining-work index: `/home/user/work/integrated/doc/2026-10-11T11-08-42Z_design_remaining_work/2026-10-11T11-08-42Z_design_remaining_work.md`
- Task docs: `/home/user/work/integrated/doc/2026-10-11T11-08-42Z_design_remaining_work/task-NN-*.md`
- Main design: `/home/user/work/integrated/doc/2026-10-11T03-44-33Z_design_groups_events_delegation/2026-10-11T03-44-33Z_design_groups_events_delegation.md`
- Research findings, design review, connectivity design: the same folder (`2026-10-11T04-20-17Z_research_findings.md`, `2026-10-11T08-08-56Z_design_review.md`, `2026-10-11T10-46-58Z_design_connectivity.md`). Relay kit: `.../connectivity/relay/`.
- Existing spike code (shared workspace, pinned versions): `/home/user/work/spikes` (members s1-transport, s2-auth, s3-history, s4-blobs, s5-cbor, s6-index, c1-connect). `/home/user/work/spikes/Cargo.toml` has the pins. Read the RESULTS.md in each member first.
- Patched p2panda-encryption (serde_bytes + from_join membership fix): `/home/user/work/spikes/s3-history/patched/p2panda-encryption`. The diff is `.../spikes/shared/s3-history/logs/patch.diff` in the design folder.
- Storage spike (redb vs SQLite): `/home/user/work/spikes-s6` (its own workspace).
- Crate sources extracted for reading: `/tmp/research/q/core/p2panda-core-0.7.1`, `/tmp/research/q/enc/p2panda-encryption-0.7.1`, `/tmp/research/q/spaces/...`, `/tmp/research/p2p-store-*/`, `/tmp/research/p2p-stream-*/`, `/tmp/research/p2p-sync-*/` (all 0.7.1). The cargo registry at `~/.cargo/registry/src/` has the rest.
- The p2panda crates are pinned to git rev `3e84cac48bba05c6857cce2ec433862b97f8d53c`. iroh 1.3.0, iroh-blobs =0.103.1, iroh-relay 1.3.0, cbor-core =0.10.1, redb =4.3.0.

## Rules

1. Write only inside your own folder: `/home/user/work/spikes-next/<your-name>/`. Read anything, but do not modify `/home/user/code`, `/home/user/work/integrated`, `/home/user/work/spikes/*/src` or the p2panda sources.
2. Do not push, open issues or PRs, publish packages, or deploy anything. Do not contact any service except what your task names.
3. Do not read or copy the large image files in the repo root (`*.iso`, `*.qcow2`). Do not run `nix`, `./rebuild`, or anything that changes the host's network config (outside a network namespace).
4. Toolchain: `CARGO="$(rustup which --toolchain 1.97.0 cargo)"`. Do not use the Nix cargo on PATH.
5. Target dir: unless your task says otherwise, `export CARGO_TARGET_DIR=/home/user/work/spikes/target` (warm, shared). Builds serialize on that lock, which is expected. Run every build and test in the foreground and wait for it. Never claim a result you did not see in output.
6. Standalone crate: add an empty `[workspace]` table to your `Cargo.toml` so cargo does not join a parent workspace. Copy `/home/user/work/spikes/Cargo.lock` into your folder so versions match. Copy dependency pins from `/home/user/work/spikes/Cargo.toml`.
7. Known traps (from earlier spikes):
   - `Header::verify` in p2panda-core returns true in non-test builds. Verify signatures with `AnyHeader::decode`, not the typed `verify`.
   - Enabling p2panda-encryption's `test_utils` feature turns on p2panda-core/test_utils, which enables the signature check. Keep that in mind when mixing features (see `s1-transport/Cargo.toml`).
   - `pkgs/id/src/tuple.rs` has a NUL-terminator bug: a full-field prefix can match a longer value containing NUL. A fix is in progress; if you use tuple keys, say which encoding you used.
   - `RelayMode::custom` turns QUIC address discovery on for every relay. Tunnel-style relays have no UDP.
8. Deliverables, all in your folder:
   - `RESULTS.md`: question, method (commands), results (tables with raw numbers), what was NOT tested and why, verdicts, and anything surprising.
   - Code: small and readable. Comments only where non-obvious. No unrelated refactors.
   - Logs: keep only small text logs (under 40 KB each).
9. Final message to the parent: at most 300 words. Give one verdict per question, the key numbers, the path to RESULTS.md, and any other findings.
10. Time budget: about two hours. If you are blocked, stop, write down the blocker with the exact command and error, what you already measured, and what would unblock you. A partial result with a clear blocker is more useful than a guess.
