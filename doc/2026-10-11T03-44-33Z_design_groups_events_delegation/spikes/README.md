# Spike code and results (2026-10-11)

Copied from `/home/user/work/spikes` (shared workspace) and `/home/user/work/spikes-s6` (storage workspace). Review: [design review](../2026-10-11T08-08-56Z_design_review.md).

C1 (connectivity) was named `s7-connect` in the workspace. It is renamed `c1-connect`, to avoid confusion with review S7 (range sync).

## Layout

- `shared/`: one Cargo workspace, members `s1-transport`, `s2-auth`, `s3-history`, `s4-blobs`, `s5-cbor`. Each member has `src/`, `Cargo.toml`, `RESULTS.md`, and small text outputs (`run-output.txt`, `logs/`). `Cargo.lock` is included so the exact graph can be rebuilt.
- `s6-index/`: its own workspace, because rusqlite 0.40 conflicts with p2panda-store's libsqlite3-sys. Same layout.
  | c1-connect | Connectivity: lookups (n0, mainline testnet, tickets), relay fallback, and a Tunnel-style relay with QAD off (t1–t8) |

## Pins

- p2panda crates: git rev `3e84cac48bba05c6857cce2ec433862b97f8d53c` (p2panda main, 2026-10-09).
- iroh 1.3.0, iroh-blobs =0.103.1, iroh-tickets 1.0.0, iroh-relay 1.3.0, cbor-core =0.10.1, redb =4.3.0.
- Toolchain: rustup 1.97.0. Use `"$(rustup which --toolchain 1.97.0 cargo)"`, not the Nix cargo on PATH.

## Rebuild

    cd shared && "$CARGO" build --workspace --locked
    cd ../s6-index && "$CARGO" build --locked

| c1-connect | Connectivity: lookups (n0, mainline testnet, tickets), relay fallback, and a Tunnel-style relay with QAD off (t1–t8) |

## Not included

- `s3-history/patched/`: the copied p2panda-encryption source. The change is kept as `s3-history/logs/patch.diff`.
- `target/` directories, and the SQLite run databases in `s6-index/run/`.
  | c1-connect | Connectivity: lookups (n0, mainline testnet, tickets), relay fallback, and a Tunnel-style relay with QAD off (t1–t8) |
- The large logs (over 40 KB). Each RESULTS.md cites the run that produced its numbers.

## What each member establishes

| Member       | Establishes                                                                                                                |
| ------------ | -------------------------------------------------------------------------------------------------------------------------- |
| s1-transport | Header format and sequence base, the verification trap, range versus LogSync bytes, ticket dialling, relay-only connection |
| s2-auth      | StrongRemove scenarios, the auth-frontier mapping, unknown-dependency behaviour, friendship and delegation prototypes      |
| s3-history   | Full history, PCS scope, from_join feasibility and patch, the ordering count, pre-key expiry                               |
| s4-blobs     | Body hash equals blob hash, fetch without a ticket, EventMask gating, verified streaming, yanks                            |
| s5-cbor      | cbor-core choice, decode-then-re-encode canonicality, four value rules, golden vectors, dcbor interop                      |
| s6-index     | redb versus SQLite load, durability under SIGKILL, concurrency, unique (author, seq), the prefix bug                       |
| c1-connect   | Connectivity: lookups (n0, mainline testnet, tickets), relay fallback, and a Tunnel-style relay with QAD off (t1–t8)       |
