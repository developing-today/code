# Orphan tree `~/worktrees/12247-qb` — history-level audit

Dated record for [issue #14353](https://github.com/developing-today/code/issues/14353)
(split out of #13919 §7). Audit run 2026-11-25 by lane item-work-289.

## Verdict

Every content-bearing entry in the retained orphan snapshot is accounted for:
its exact content is either identical to `origin/main`, present in the code
repo's git history, or cargo build output reproducible from tracked sources.
**Unlanded files: 0.** Nothing needs preservation, so the tree is cleared for
deletion under the issue's acceptance criteria; the deletion itself is
recorded on #14353.

## The tree

- Laptop directory `~/worktrees/12247-qb`: 1.2 GB, no `.git`, not a registered
  worktree of any clone. A code-repo-shaped snapshot (files dated
  2026-11-21 19:52), including `pkgs/fleet/` with its cargo target dirs and a
  282-entry `doc/` tree.
- Comparison base: `origin/main` at `80063a004dbf361fd88d670ce77d6e07f861a9b5`
  (the laptop primary clone's `origin/main` at audit time).

## Method

For every entry (regular file or symlink) the audit computed the git blob
SHA-1 of its exact content (for a symlink, the blob of its target string) and
classified it:

- **current** — the same path on `origin/main` carries the identical blob.
- **history** — the blob exists in the clone's object database
  (`git cat-file --batch-all-objects`), i.e. the exact content exists in the
  repo's git history on some ref.
- **generated** — the blob is in neither, and the entry is compiler output
  (see below).

The full per-file accounting is `accounting.csv` in this folder (9,160 rows:
path, kind, size, blob SHA-1, class, present-on-main flag). Empty directories
(58) carry no content and are listed in `empty-dirs.txt`.

## Accounting (computed from the audit data)

| Class     |   Entries | Meaning                                 |
| --------- | --------: | --------------------------------------- |
| current   |     4,529 | identical to `origin/main`              |
| history   |     1,536 | exact content in git history            |
| generated |     3,095 | cargo build/clippy output, reproducible |
| **total** | **9,160** | 9,120 files + 40 symlinks               |

- Differing vs `origin/main`: 4,631 entries; not present on `origin/main` at
  all: 4,165.
- Build output (`pkgs/fleet/target/` + `pkgs/fleet/target-clippy/`): 4,127
  entries — all 3,095 generated entries live there, plus 1,032 whose bytes
  happen to coincide with a history blob. `target` dirs are git-ignored by
  design (`.gitignore`: `target`, `**/target`); the clippy dir is the same
  cargo output under a `CARGO_TARGET_DIR` name.
- Non-build differing entries: **504, and every one of them is class
  "history"** — older document versions, the top-level slice-1 completion
  reports, and the certificate artifacts below.
- Certificate artifacts under
  `doc/2026-10-11T03-44-33Z_design_groups_events_delegation/spikes/s11-relay/certs/`:
  20 entries — 9 current, 11 history (including the `gen/` `.key` and `.srl`
  files). Their exact bytes are in git history; no unlanded key material.
- Empty directories: 58 — 57 inside the build dirs, 1 outside
  (`pkgs/map/migrations`). Git tracks no empty directories; nothing to land.

### Why these counts differ from the #13919 §7 pass

§7 reported 367 differing files and 23 orphan-only entries. That pass ran a
recursive diff against the primary clone's _working tree of the same era_
(HEAD `9e6b4adf64e5`), which collapses whole unique directories into single
"only in" lines and shares the era's build output. This audit counts per file
against current `origin/main` and counts build output individually; the
comparable non-build differing figure is 504 (above), all history-matched.

## The fragment-named entries, explained

The top-level fragments (`aut`, `f`, `fu`, `ful`, `n`, `nv`, `offli`, `r`,
`re`, `rebu`, `si`) are **symlinks**, not files. Each is a prefix-truncation
alias of one of the `lib/` helper scripts:

- `aut`, `f`, `fu`, `ful`, `full`, `full.sh`, `auth`, `auth-rebuild.sh`,
  `re`, `rebu`, `r`, `rebuild.sh` → `./lib/auth-rebuild-full.sh`
- `offli`, `offline.sh`, `rebuild-offline.sh`, `auth-offline.sh` →
  `./lib/auth-rebuild-offline.sh`
- `si`, `simple`, `rebuild-simple`, `auth-simple` → `./lib/auth-rebuild-simple.sh`
- `n`, `nv`, `nvim`, `nvim.sh`, `vim` → `lib/nvim.sh`

They are the residue of progressively typed `ln -s` commands (every prefix
of the target's distinguishing word got its own link). All 40 symlinks in the
tree are accounted: 29 exist identically on `origin/main`, and the 11
fragment ones carry target strings whose exact blobs exist in git history —
the links hold no content beyond the target path, and every target script is
itself tracked.

## Verification

The history classification rests on object-database membership. Two samples
were additionally verified server-side through the GitHub API (a blob match
at a commit that carried that exact path version):

- `FINAL_SLICE1_COMPLETION_REPORT.md`, blob
  `0aee17a98261e1dafe378326cb587520b8767fb6` — matches at commit
  `1858531205ab98b5857eeaac94c11a8a67baf8f1`.
- `doc/2026-10-30T22-45-00Z_design_hosted_provisioning_sso_binding.md`, blob
  `755accaf8ed75a32b0472eb4a8911f8cc846373c` — matches at commit
  `c6564aa2f0fded7c68bcd7d24b13fbcb5163b5b3`.

## Disposition

- Preservation: none required — zero unlanded files.
- Deletion: with the accounting complete, the tree may be deleted; the
  deletion (path, realpath check, freed space) is recorded on #14353, which
  closes on that record.
