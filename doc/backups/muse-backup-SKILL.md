# muse-backup

Status: tentative, first draft 2026-10-21.

## Purpose

Back up Muse's durable state — memory and avatar — so it can be restored after a sandbox loss or an accidental edit, and keep the locations/rules in one place.

## What lives where

- **Memory (source of truth):** `dezren39/hatch-personal` (private git repo). On disk it is `~/hatch-personal/`, symlinked back to `~/MEMORY.md`, `~/memory/`, `~/USER.md`, etc. Backup = commit + push the private repo (surgical staging — memory files only when that is the task). The full symlink restore map is in `hatch-personal/library/inventory/symlink-map.md`.
- **Avatar:** source image + generated renders under `~/workspace/avatars/`, tracked in the public repo `dezren39/hatch`. Drew wants the backup to clearly include the avatar *and its animations*, with liked animations restorable if an accidental source-image edit produces renders he dislikes — i.e. keep prior render sets, don't just overwrite them; a new source image gets a new render set alongside the old.
- **Too big for git:** session transcripts (`~/agents/`, ~1 GB) and similar bulk go as tarballs to the dedicated Google Drive **"Muse Backups"** folder, never into either git repo.
- **Secrets are never part of a backup.** They live in Bitwarden; backups reference names only. Credential files are SECRET-SKIP (paths + sizes in the private inventory classification) — never copied into a backup set.

## Procedure

1. `git -C ~/hatch-personal status` — review carefully; stage only the intended memory/personal files; commit; push. Never sweep unrelated dirty files into a backup commit.
2. Avatar: confirm the current source image and all render/animation files for it are present under `~/workspace/avatars/` and tracked in `dezren39/hatch`; if renders were regenerated, keep the previous set until Drew confirms the new one.
3. Bulk (only when asked): tar the target (e.g. `~/agents/`) with a dated name, upload to Drive "Muse Backups", record name + size + date in the private repo.
4. Report what was backed up, where, and the commit SHA / Drive file name. State explicitly what was skipped and why.

## Pitfalls

- `/home/hatch` itself is a git repo distinct from `~/hatch-personal` — commit memory with `git -C ~/hatch-personal`, not from home.
- `~/workspace/goals` is a symlink into hatch-personal — files there are private-repo files even though the path looks public-workspace.
- Deletion inside read-only (555) directories fails even as root; `chmod u+w` first, or keep backups as single tarballs instead of live trees.
