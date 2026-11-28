# Vendored crates

## p2panda-sync 0.7.1

**Origin.** `https://github.com/p2panda/p2panda`, rev `da80c05b95269926bf58121cf6a2d3dd5ca41dfc`, crate `p2panda-sync/`. The same rev as the other p2panda crates in `pkgs/id/Cargo.toml`. License `MIT OR Apache-2.0` (`p2panda-sync/LICENSES/`).

**Why.** p2panda-sync depends on p2panda-store with its default features, and the default includes `sqlite`, which pulls in `sqlx`. The store is redb (explainers section 14), so the copy removes that default. Vendoring was chosen over a fork, so nothing is pushed or published.

**The substantive change** (one line, in `p2panda-sync/Cargo.toml`):

```diff
-p2panda-store = { workspace = true, default-features = true }
+p2panda-store = { git = "https://github.com/p2panda/p2panda", rev = "da80c05b95269926bf58121cf6a2d3dd5ca41dfc", default-features = false }
```

**Mechanical changes**, needed because this copy is outside the p2panda workspace:

- Every `workspace = true` is replaced by the root's value (`[workspace.dependencies]`). `p2panda-core` and `p2panda-stream` become the same git source and rev.
- `[lints] workspace = true` is removed, so the copy takes no lints from this crate.
- `[dev-dependencies]` is removed. Its tests use the SQLite test helpers and are not built here.
- `examples/` is removed. The examples construct `SqliteStore`.
- `LICENSES/` is added.

**Verify.** The copy matches upstream except the files above:

```sh
SRC=$(ls -d ~/.cargo/git/checkouts/p2panda-*/da80c05/p2panda-sync)
diff -rq "$SRC" pkgs/id/vendor/p2panda-sync   # Cargo.toml differs; examples/ and LICENSES/ differ
diff -u "$SRC/Cargo.toml" pkgs/id/vendor/p2panda-sync/Cargo.toml
git -C "$SRC/.." rev-parse HEAD                # da80c05b95269926bf58121cf6a2d3dd5ca41dfc
```

`sqlx` is absent from the build. `cargo tree -i sqlx` reports no match, and `store_traits`'s `test_the_lock_has_no_sqlite_with_logsync_wired` fails if `Cargo.lock` names `sqlx`, `sqlite`, `rusqlite` or `libsqlite3-sys`, or if the `p2panda-store` line above loses `default-features = false`.

**Workspace exclusion.** Both vendored crates are listed in the root manifest's `[workspace] exclude`. Without that, Cargo treats path dependencies inside the workspace directory as members, and a member's feature graph joins the lock resolve: this crate's `test_utils` feature forwards to `p2panda-store/test_utils`, whose `sqlite` feature pulls the whole `sqlx` family into `Cargo.lock` even though no build here enables it. That is how `sqlx-sqlite` entered the lock at the #3447 workspace conversion (#4015); the exclusion plus a `cargo update -w` re-resolve removed it, with no version changes.

**To update.** Change the rev in the three places it appears (this file, the two git lines in `vendor/p2panda-sync/Cargo.toml`, and the git lines in `pkgs/id/Cargo.toml`). Then copy the crate again and reapply the changes above.

## p2panda-encryption 0.7.1 (DataScheme fork)

**Origin.** `https://github.com/p2panda/p2panda`, rev `da80c05b95269926bf58121cf6a2d3dd5ca41dfc`, crate `p2panda-encryption/`. The same rev as the other p2panda crates in `pkgs/id/Cargo.toml`. License `MIT OR Apache-2.0` (`p2panda-encryption/LICENSES/`).

**Why.** The crate's `process_welcome` overwrites the joiner's membership with the welcome's copy, and that copy lacks the joiner, so the joiner is never welcomed and reads nothing. The fix, and `add_from_join` (the welcome carries no earlier secret, so the group needs an immediate update after each join), are in our fork and not upstream. Decision (owner): keep the fix in this repo. Source: the spike `doc/2026-10-11T03-44-33Z_design_groups_events_delegation/spikes/shared/s3-history/` (`logs/patch.diff`, `RESULTS.md`).

**The changes** (the S3 patch, applied by hand):

- `src/data_scheme/dcgka.rs`: `process_add` skips `DGM::add` for the joiner. `process_welcome` takes the add's `seq`, calls `DGM::from_welcome`, then `DGM::add` for the joiner itself. Both are needed; the test is `joiner_is_welcomed_and_reads_messages_after_the_add` in `src/data_scheme/tests.rs`.
- `src/data_scheme/group.rs`: new public `EncryptionGroup::add_from_join` (25 lines). It calls `Dcgka::add` with an empty bundle and runs the same local steps as `add`. No existing item changes visibility.
- `src/data_scheme/tests.rs`: the regression test above, which fails on the stock crate (the joiner's roster is `[0, 1, 2]`).
- `Cargo.toml`: every `workspace = true` is replaced by the root's value, since this copy is outside the p2panda workspace. `[lints] workspace = true` is removed. `p2panda-core` is the same git source and rev, with `default-features = false`, as upstream's workspace declares it. `[dev-dependencies]` is unchanged (none).
- `LICENSES/` is added.

**Scheme v1 (P4.4).** The encoding this fork ships is DataScheme scheme v1, named in the group's signed policy (`src/groups/policy.rs`, `Scheme`). Three changes, all in the fork:

- `src/two_party/two_party.rs`: `#[serde(with = "serde_bytes")]` on `TwoPartyPlaintext.plaintext`. The stock layout is an array of integers, 146.5 bytes per epoch in the welcome. With it, 40.0 bytes per epoch.
- `src/two_party/x3dh.rs`: the same attribute on `X3dhCiphertext.ciphertext`. Together with the one above, the two fields are byte strings.
- `src/data_scheme/group_secret.rs`: `SecretBundleState` serializes its secrets sorted by (timestamp, ID), not in `HashMap` order. The bundle is encrypted inside the welcome, and sorting keeps its plaintext stable. The test is `bundle_bytes_do_not_depend_on_hash_map_order`.

v1 and the stock layout do not decode each other. The stock layout is not shipped, so a group whose policy names it is refused when a session starts (`pkgs/id/src/encryption/scheme.rs`, `negotiate`), and nothing is migrated. Source: `doc/2026-10-11T11-08-42Z_design_remaining_work/research-round2/scheme-serde/RESULTS.md`.

**Size.** `src/` differs from upstream in 6 files, 83 lines added and 4 removed. The S3 patch is 63 added and 2 removed in three files, and scheme v1 is the rest (20 added, 2 removed, in three files). `Cargo.toml` differs by 13 changed lines.

**Features.** `data_scheme` on, `test_utils` and `message_scheme` off. The `test_utils` tests in the fork use the crate's own `Network` and `TestDgm`, which the test cfg enables.

**Verify.**

```sh
C=$(ls -d ~/.cargo/git/checkouts/p2panda-*/da80c05)
diff -rq "$C/p2panda-encryption" pkgs/id/vendor/p2panda-encryption   # the files above differ, nothing else
git -C "$C/.." rev-parse HEAD                                         # da80c05b95269926bf58121cf6a2d3dd5ca41dfc
cd pkgs/id/vendor/p2panda-encryption && cargo test --lib data_scheme  # run standalone; delete the Cargo.lock it writes
```

**To update.** Change the rev in the three places it appears (this section, the `p2panda-encryption/Cargo.toml` git lines, and `pkgs/id/Cargo.toml`). Copy the crate again, then reapply the changes above and run the fork's test.
