# Vendored crates

## p2panda-sync 0.7.1

**Origin.** `https://github.com/p2panda/p2panda`, rev `3e84cac48bba05c6857cce2ec433862b97f8d53c`, crate `p2panda-sync/`. The same rev as the other p2panda crates in `pkgs/id/Cargo.toml`. License `MIT OR Apache-2.0` (`p2panda-sync/LICENSES/`).

**Why.** p2panda-sync depends on p2panda-store with its default features, and the default includes `sqlite`, which pulls in `sqlx`. The store is redb (explainers section 14), so the copy removes that default. Vendoring was chosen over a fork, so nothing is pushed or published.

**The substantive change** (one line, in `p2panda-sync/Cargo.toml`):

```diff
-p2panda-store = { workspace = true, default-features = true }
+p2panda-store = { git = "https://github.com/p2panda/p2panda", rev = "3e84cac48bba05c6857cce2ec433862b97f8d53c", default-features = false }
```

**Mechanical changes**, needed because this copy is outside the p2panda workspace:

- Every `workspace = true` is replaced by the root's value (`[workspace.dependencies]`). `p2panda-core` and `p2panda-stream` become the same git source and rev.
- `[lints] workspace = true` is removed, so the copy takes no lints from this crate.
- `[dev-dependencies]` is removed. Its tests use the SQLite test helpers and are not built here.
- `examples/` is removed. The examples construct `SqliteStore`.
- `LICENSES/` is added.

**Verify.** The copy matches upstream except the files above:

```sh
SRC=$(ls -d ~/.cargo/git/checkouts/p2panda-*/3e84cac/p2panda-sync)
diff -rq "$SRC" pkgs/id/vendor/p2panda-sync   # Cargo.toml differs; examples/ and LICENSES/ differ
diff -u "$SRC/Cargo.toml" pkgs/id/vendor/p2panda-sync/Cargo.toml
git -C "$SRC/.." rev-parse HEAD                # 3e84cac48bba05c6857cce2ec433862b97f8d53c
```

`sqlx` is absent from the build. `cargo tree -i sqlx` reports no match, and `store_traits`'s `test_the_lock_has_no_sqlite_with_logsync_wired` fails if `Cargo.lock` names `sqlx`, `sqlite`, `rusqlite` or `libsqlite3-sys`, or if the `p2panda-store` line above loses `default-features = false`.

**To update.** Change the rev in the three places it appears (this file, the two git lines in `vendor/p2panda-sync/Cargo.toml`, and the git lines in `pkgs/id/Cargo.toml`). Then copy the crate again and reapply the changes above.
