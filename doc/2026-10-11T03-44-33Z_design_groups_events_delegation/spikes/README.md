# Spike code and results (2026-10-11)

Copied from `/home/user/work/spikes` (shared workspace) and `/home/user/work/spikes-s6` (storage workspace). Review: [design review](../2026-10-11T08-08-56Z_design_review.md).

C1 (connectivity) was named `s7-connect` in the workspace. It is renamed `c1-connect`, to avoid confusion with review S7 (range sync).

## Layout

- `shared/`: one Cargo workspace, members `s1-transport`, `s2-auth`, `s3-history`, `s4-blobs`, `s5-cbor`. Each member has `src/`, `Cargo.toml`, `RESULTS.md`, and small text outputs (`run-output.txt`, `logs/`). `Cargo.lock` is included so the exact graph can be rebuilt.
- `s6-index/`: its own workspace, because rusqlite 0.40 conflicts with p2panda-store's libsqlite3-sys. Same layout.

## Pins

- p2panda crates: git rev `3e84cac48bba05c6857cce2ec433862b97f8d53c` (p2panda main, 2026-10-09).
- iroh 1.3.0, iroh-blobs =0.103.1, iroh-tickets 1.0.0, iroh-relay 1.3.0, cbor-core =0.10.1, redb =4.3.0.
- Toolchain: rustup 1.97.0. Use `"$(rustup which --toolchain 1.97.0 cargo)"`, not the Nix cargo on PATH.

## Rebuild

    cd shared && "$CARGO" build --workspace --locked
    cd ../s6-index && "$CARGO" build --locked

## Not included

- `s3-history/patched/`: the copied p2panda-encryption source. The change is kept as `s3-history/logs/patch.diff`.
- `target/` directories, and the SQLite run databases in `s6-index/run/`.
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

## 2026-10-11T12-53-32Z Spike results from the 2026-10-11 round

Each folder has a `RESULTS.md` with the commands, raw numbers, and what was not tested. Build output is not committed. Shared brief: `BRIEF.md` in this folder.

| Folder | Question | Verdict |
| --- | --- | --- |
| `s7-range-real/` | Range sync on real redb stores over iroh | Works and matches S1. Keep K=16, T=16. Crossover: range wins control bytes up to about 33 new events at 1,000 chains |
| `s8-tags/` | Observed-remove tags | Hold. The clock must cover the causal past. Compaction needs a snapshot |
| `s9-seitan/` | Seitan V2 derivations, freshness, admin path | Derivations match Keybase's V2 vectors. A ticket cannot carry the token |
| `s10-heads/` | Heads orderer in a 40-member group | Cap 32 needs the merge rule and the author's own latest event named every time |
| `s11-relay/` | Relay restart, keys, certificates, idle | Restart survives a short outage. A 35 s outage loses frames. Use `KillSignal=SIGINT` |
| `s12-welcome/` | Welcome size per epoch | 146.6 B stock, 40.0 B with serde_bytes. The serde_bytes change needs a new scheme version. `variants/*.patch` hold the diffs |
| `c2-holepunch/` | Holepunching through NAT in namespaces | Full cone and port-restricted hold-punch. Symmetric against symmetric falls back. QAD is required |
| `beekem/` | BeeKEM costs, concurrency, join, migration | Not adoptable now. DataScheme stays. See task 05 |
| `c1-rerun/` | C1 t2 and t4 on current infrastructure | Both pass |
