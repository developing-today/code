# Task 11: Dependencies, pins, and upstream reports

- Status: Open. Most of this is needed by tasks 02 to 05.
- Index: [what is left](./2026-10-11T11-08-42Z_design_remaining_work.md). Depends on: [task 01](./task-01-phase0-fixes.md).
- Design: [review §7](../2026-10-11T03-44-33Z_design_groups_events_delegation/2026-10-11T08-08-56Z_design_review.md). Reporting rule: `pkgs/id/AGENTS.md` (name, version, reason, feature, for each new dependency).

## 1. Crates to adopt

| Crate                        | Action                                                                          | Why                      | Notes                                             |
| ---------------------------- | ------------------------------------------------------------------------------- | ------------------------ | ------------------------------------------------- |
| p2panda-core                 | Adopt at git rev `3e84cac`. Bumps reviewed                                      | Header format, ChainExt  | Needs Rust 1.96                                   |
| p2panda-auth                 | Adopt at the same rev                                                           | StrongRemove, membership | Needs Rust 1.96                                   |
| p2panda-encryption           | Adopt at the same rev, with `data_scheme` only                                  | E2EE                     | Never `test_utils` in product code                |
| cbor-core                    | Adopt, `=0.10.1` with `serde`                                                   | Canonical form           | The same version as p2panda-core                  |
| unicode-normalization        | Add                                                                             | NFC value rule           | New                                               |
| redb                         | Keep, `=4.3.0` (already in the tree)                                            | Events and index         |                                                   |
| iroh-blobs                   | Keep, `=0.103.1`                                                                | Payloads                 | Not yanked                                        |
| iroh-tickets                 | Keep, `1.0.0`                                                                   | Addresses                |                                                   |
| iroh-relay                   | Add when our relay is deployed (task 08)                                        | Our relay                | Server feature only for the host                  |
| iroh-mainline-address-lookup | Add behind a feature when the DHT is wired (task 08)                            | Opt-in DHT               | Brings `n0-mainline`                              |
| iroh-docs                    | Keep for records only. Disable its default `redb-v2-migration` (drops redb 3.x) | Records                  | Verify in a build. Existing data may be discarded |
| dcbor                        | Dev-only, for interop tests                                                     | S5                       |                                                   |
| hmac, scrypt                 | Add with task 07                                                                | Seitan V2                | `scrypt` is transitive today                      |

Do not add p2panda-store, p2panda-sync, or p2panda-stream. Range sync and redb replace them, and p2panda-store brings sqlx, which conflicts with rusqlite.

## 2. Pins and versions

- p2panda crates pinned to a git rev. Each bump is reviewed against the spike tests, not taken blindly.
- `ed25519-dalek`: p2panda-core uses 2.2, and our code and iroh use 3.0. Keys cross as bytes, and both sides use `verify_strict`. Decide whether to align (the owner decision).
- MSRV: 1.96 for the p2panda crates (task 01).

## 3. Upstream reports

File these against p2panda (and attach the patch from `spikes/shared/s3-history/logs/patch.diff` where it applies):

1. The joiner membership overwrite in `dcgka.rs` `process_welcome`. The joiner applies its add, then the welcome overwrites its membership, which lacks the joiner. Seen with stock `TestDgm`. The fix is 8 lines added and 2 changed.
2. Missing `serde_bytes` on `TwoPartyPlaintext.plaintext` and `X3dhCiphertext.ciphertext`. Each byte costs one or two in CBOR. The welcome grows about 3.6 times with the fix (estimated).
3. The orderer names the per-author latest and never prunes, so a 40-member group names 40 dependencies.
4. `receive` returns no state on error (`group.rs:464`), so a joiner that processes a pre-join application message loses its state. Upstream should return an undecryptable output instead.
5. Unknown-dependency and group-absent cases in p2panda-auth: an unknown dependency is an error, but a group absent at its dependencies panics at `group/crdt/mod.rs:822`.
6. `MemberDgm` and `TestDgm` in test utilities: test-only, so note it, but the production DGM must not copy them.

## 4. Upstream policy

Carry the patch in our tree until upstream merges it. Keep the patch small and separate, so it can be dropped cleanly. Revisit at each pin bump.

## 5. Known gaps in the crates

- p2panda-auth 0.7.1 has no member cap, no dissolve, no cascade, and no delegation. Ours (task 03).
- p2panda-auth's promote and demote check does not verify direction (issue #783), and it has a silent depth cap (#799).
- p2panda-spaces 0.7.1 panics on Promote and Demote, and does not rotate on add.
- p2panda-encryption has no audit (its README says so), no test vectors, and open issue #807 (a concurrent add can miss a secret).

These are risks to record, not blockers for the design.

## Acceptance

Each crate in the table has a line in `Cargo.toml` with its pin, a reporting entry, and a build in CI or in the local check. The upstream reports are filed, with links recorded here.

## 2026-10-11T12-20-38Z Update: upstream decisions

- Patches stay in our fork (developing-today). Nothing goes upstream, and no upstream issues or PRs are opened, per the owner.
- serde_bytes: local only. Upstream: #770 (closed) and #1059 (closed, fixed by PR #1058 for `Topic`). None covers the DataScheme `Vec<u8>` fields.
- The membership bug is in upstream code (see task 05). It is not reported upstream.
- The toolchain and dependency update moved to task 12.
