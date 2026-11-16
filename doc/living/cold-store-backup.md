# Cold-store backup system

- Living document · Status: shipped / operating (nightly) · Register entry X6 · Tracking epic: developing-today/code#8805

## Purpose

One nightly cold-store backup across the fleet's data stores — Turso databases,
Supabase, and D1 — with content-addressed artifacts filed to Google Drive, so a
lost laptop or a dead cloud database is a restore, not a loss.

## What landed

- The nightly cold-store run itself: exports each source store, packages the
  artifacts, and files them per the R2 layout design committed beside this doc
  (`cold-store-r2-layout-design-2026-10-25.md`, the original 2026-10-25 design,
  previously only in the lab-cluster goal's files).
- The 2026-10-27 failure fix chain (PRs #1373/#1383 in developing-today/code):
  the Turso step failed when the ~1.13 GB atlas DB dump hit caps; the fix is a
  compressed streaming rebuild plus a repair filter for libsql's malformed
  `/dump` schema rows. The run went "Turso full-house" clean on 2026-10-28,
  and a Supabase S3 gap found that night was closed.

## Rough architecture

Nightly job → per-source export (Turso `.dump`, Supabase, D1) → streaming
compression → content-addressed cold-store layout (see the R2 layout design
doc) → Google Drive filing. The repair filter sits in the Turso export path
(`export-turso.sh`) and rewrites malformed schema rows emitted by libsql's
`/dump` so the dump stays loadable.

## Status

Shipped and operating nightly. Designed 2026-10-25; first recorded failure
2026-10-27, fixed in place; clean full-house run 2026-10-28.

## Next steps

- Immediate: none recorded.
- Near-term: confirm the permanent streaming repair filter in
  `export-turso.sh` is the landed form (it was "in flight" in the
  2026-10-27 record).
- Long-term: read with the wider backups family (register X19): cold-store
  nightly, Supabase DIY backups (X7), fleet-backups provisioning (#8584),
  hash-tree scoping, Backblaze — the register notes backups is the most
  fragmented domain, with no single owning epic.

## Open follow-ups

- The libsql `/dump` malformed-schema-rows quirk owed an upstream filing
  since 2026-10-27 — now filed as developing-today/code#8811.

## Future ideas not yet specced

- A single owning epic for the whole backups family (register X19 note).

## Links

- Register entry X6 (pm-stack/design-register-2026-11-11.md, developing-today/code)
- Design: `cold-store-r2-layout-design-2026-10-25.md` (this directory)
- Fix PRs: developing-today/code#1373, #1383
- Upstream-quirk filing: developing-today/code#8811
