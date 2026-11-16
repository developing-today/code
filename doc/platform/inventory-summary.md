# Inventory summary — /home/hatch (public, redacted)

Generated 2026-10-21. Companion files in the private repo (`dezren39/hatch-personal`, `library/inventory/`): `inventory.md` (sizes/counts), `symlink-map.md` (897 symlinks with restore targets), `classification.md` (full sensitivity rules), and the unredacted version of this summary. **Inventory only — nothing was copied, moved, committed, or pushed by the inventory itself. No secret file contents were read.**

This public copy has the SECRET-SKIP path table and the UNSURE items that name credential paths removed — details in private repo.

## Headline numbers

- Disk: 100 GB volume, **48 GB used / 51 GB free** (`df -h`).
- Files walked: **1,720,278** (excl. nix store contents). Symlinks (depth ≤4): **897**. Files >50 MB: **165**.
- Apparent-size total exceeds disk use (~115 GB apparent vs 48 GB used) due to hardlinks and duplicated build artifacts across clones/worktrees/caches.

## Totals per class (apparent, overlapping classes — do not sum)

| class | approx. size | what dominates it |
|---|---|---|
| PUBLIC-SAFE | ~100–150 MB of unique Drew-authored material | `tools/*.sh` + service units, `notes/architecture` + `notes/research`, root `README/TODO/TOOLS.md`, `workspace/avatars` (59.1 MB), `workspace/custom-skills` |
| PRIVATE | ~1.3 GB | session transcripts ~1.0 GB, `hatch-personal/` 59.3 MB (+ `.git` 29.7 MB), imports ~99.1 MB, plus goals/documents workspaces (details in private repo) |
| SECRET-SKIP | ~7 KB of actual credential files (15 curated paths) + whole-directory holds for credential/profile stores — details in private repo | Paths + byte sizes only, in private repo |
| LARGE-SKIP | ~95 GB apparent | nix store 31.4 GB, `git/` clones 44.9 GB, `.cache` 9.0 GB, `tmp` 8.3 GB, `.bun` 4.6 GB, toolchains in `tools` ~3.9 GB, `go/pkg` 3.0 GB |

## Biggest skipped items (LARGE-SKIP, with sizes)

1. `~/nix-persist/store` — **31.4 GB** — reproducible via nix, never copy into git.
2. `~/git/devtoday-code` — **36.1 GB** apparent — own remote; bulk is Rust `target/` build output.
3. `~/.cache/go-build` — 6.3 GB; `~/.bun/install` — 4.9 GB; `~/git/devtoday-opencode` — 4.5 GB.
4. `~/tmp/oc199` — 3.1 GB; `~/go/pkg` — 3.2 GB; `~/nix-persist/var` — 3.3 GB.
5. `~/git/devtoday-openchamber` — 2.7 GB; `~/tools/rustup` — 2.5 GB; `~/.cache/opencode` — 2.4 GB; two OpenChamber worktrees — 2.3 GB each.
6. `~/opt-backup.tar.zst` — **1.08 GB single file** (contents not inspected; classification unresolved — details in private repo).
7. `~/agents/` — **1.0 GB** (also PRIVATE — Google Drive tarball candidate, not git).
8. `~/.nix-portable` — 989 MB; `~/.local` — 937 MB; `~/.rustup` — 1.4 GB; `~/.npm/_cacache` — 1.3 GB; root-repo `~/.git/objects` — 1.16 GB.

## Files/dirs classified UNSURE

The full UNSURE list is in the private repo. In public terms, the unresolved categories are: one opaque 1.08 GB archive at the home root (contents not inspected); working directories that mix public-safe code with account-specific configuration and need a per-file scan before any copy; platform-provided content that is not Drew-authored and likely belongs in neither repo; platform runtime state whose ownership/purpose is unclear; and a cache-shaped directory whose origin app is unknown. Items that name credential paths have been redacted from this copy — details in private repo.

## Method / caveats

- Sizes are apparent (`lstat` sums from a full walk, no symlink following), except the nix store (`du -sb`).
- Symlink map excludes `nix-persist/` and `.git/` internals; 897 links at depth ≤4 — the bulk are toolchain shims and wrappers. The 13 load-bearing restore links are the root symlinks into `hatch-personal/` plus a handful of workspace/notes links (full map in private repo).
- A raw name-scan for secret-pattern filenames produced ~12,223 hits, overwhelmingly library/cache false positives (e.g. `tokenizer*` files in node_modules); they were classified under their parent cache/repo class, not as individual secrets.
