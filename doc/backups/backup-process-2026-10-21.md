# Backup process + ledger — 2026-10-21

## The process

Backups here are layered, by what the thing is:

1. **Source-of-truth text** (memory, docs, scripts, plans) → git: `dezren39/hatch` (public-safe) and `dezren39/hatch-personal` (everything private, including "not secret but private-guidance" material). Commits are the change record; the push is the backup completing.
2. **Secrets** → Bitwarden Secrets Manager, by name; repos and docs reference names and locations only. Files that are themselves secrets get uploaded as secret values (file-level) — audited 2026-10-21.
3. **Bulky/opaque data** (session transcripts, tarballs) → Google Drive folders `Muse Backups` and `Muse Avatar Backup`, each with a manifest (date, source, sha256, restore instructions). Git is the wrong tool for these (see the chunk-drawer research in the library build notes).
4. **Databases** → native dumps stored in the private repo when tiny (a SQLite dump is text and diffs beautifully) plus independent exports for anything that matters.
5. **The nix store / toolchains** → reproducible from docs + the snapshot tarball (`~/tools/nix-snapshot.sh`); binaries that are NOT reproducible that way are listed in `library/toolchains/binary-backup.md`.

Rule of thumb: every backup run leaves a record a future agent can verify — a commit, a manifest with hashes, or an entry in this ledger.

## Ledger (most recent first)

- 2026-10-21 — Private DB backup (SQLite dump, verified by restore into a scratch DB) filed in hatch-personal. Details intentionally private-only.
- 2026-10-21 — Secret-file → Bitwarden coverage audit + upload (see private records).
- 2026-10-21 — Small-files/scripts sweep into hatch `archive/` + hatch-personal `junk/` with hash manifests.
- 2026-10-21 — Session logs: `session-logs-2026-10-21.tar.zst` (~/agents, 1.1 GB → 103 MiB) + manifest → Drive `Muse Backups`; MD5-verified after upload.
- 2026-10-18 — Avatar backup to Drive `Muse Avatar Backup` (12 files, sizes verified; restore test passed; daily job `avatar-backup-daily` continues it).
- Ongoing — memory/personality/instructions sync to hatch-personal with normal work (private).
