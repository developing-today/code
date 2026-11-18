# muse-dev2 recovery and Turso cutover receipt

Owner request, 2026-11-18: migrate Supabase uses and tables to Turso and
recover the records present in the backup. The deleted project is not
recreated. Existing Turso records, mirror checkpoints and backup artifacts
must not be destroyed or overwritten by an older snapshot.

## Source and preservation

The full November 17 23:00:48 UTC pg_dump archive was downloaded and its
SHA-256 checked against the backup manifest. The compressed file is
45,166,108 bytes. The complete lossless local recovery archive represents
130 tables and 623,584 records, including schema-qualified names, original
COPY field values, and NULLs. Preservation of all records is distinct from
making every archived PostgreSQL schema an active SQLite application schema.

The runtime recovery inserted 22,385 records and preserved 18 existing
parent records (22,403 total across the nineteen verified runtime tables).
Fresh-connection readback reported foreign-key checks OK, 82 pre-existing
schema entries preserved, and per-table value matches using the recorded
14-significant-digit normalization for REAL values. This is not a claim
of bit-for-bit equality for every PostgreSQL floating-point value.

A separate AA import inserted 160 records: three source registry rows and
157 sheet rows. New values were checked exactly. Forty-two existing source
rows were checked for exact preservation. Preservation of 85,293 existing
sheet rows is based on INSERT-only SQL and absence of target triggers, not
a full pre/post value hash. The resulting AA source/sheet counts are 45 and
85,450 respectively. All 130 archive tables / 623,584 records were also
verified in the Turso recovery archive; archive preservation and live
application availability remain separate claims.

## Config-store reconciliation

A read-only November 18 check found 54 records across the eight D1
`fleet-config` tables. The recovered November 17 backup contains 57 records
for those same tables. After normalizing timestamps, 53 of the 54 D1 rows
match their backup counterparts. The remaining `ainventory.sources` row
(id 41) has the entire D1 note plus a November 12 workbook-deprecation
addendum in the backup. The backup also contains source ids 47, 51 and 52.
No D1-only record or newer timestamp was observed. Preserve the fuller
backup state; D1 was not modified or removed.

## Code and verification

- PR #11915 ports active PM stores, alerts, feed/AA reads and the canonical
  table exporter to Turso.
- PR #11919 (merge `0809001c3b1681f0b6fff2dfa907588cfa286d51`) ports the
  orchestration and interpretation writers, packages their shared helper,
  wires PM service and backup exports to Turso, and retires the obsolete
  source mirror and source-dump timers in the host configuration.
- The Python client permits only the configured fleet-platform HTTPS
  origin, refuses redirects, validates per-statement and commit success
  evidence, and reports uncertain transport outcomes rather than claiming
  rollback. Orchestration replay compares JSON semantically and guards
  the observed stored document inside its transaction.
- Twenty-seven focused Python tests and the dashboard backup manifest
  suite passed. The changed Nix files parsed successfully.
- All eighteen append-only guards were independently exercised against
  actual Turso staging: six tables rejected UPDATE, DELETE and identity
  replacement; sampled records remained unchanged. This staging evidence
  does not by itself prove production guard installation.

## Activation and limits

The narrow cutover began **2026-11-18 21:46:10 UTC**, using merge
`0809001c3b1681f0b6fff2dfa907588cfa286d51`. The built PM package is
`qw96c6yk7yr7nc6r1dwkm4i7lmq9s0hp-pm-aggregator-0.1.0`. Five individually
generated Home Manager units were activated; old links/environment were
preserved in the local `turso-cutover-20261118T214610Z` rollback receipt.
Only `pm-aggregator.service` was restarted. No active oneshot was stopped,
and there was no full system switch or broad process kill.

Initial readback: `/api/health` returned HTTP 200 and `/api/alerts.json`
returned HTTP 200 with the restored 100-incident/event page. This is service
and store-read evidence, not phone-delivery evidence.

The old `supabase-backup.timer` and `supabase-turso-coord-mirror.timer` were
disabled/stopped at **21:47:24 UTC**. After failed-state reset and daemon
reload, both showed `LoadState=not-found`, `ActiveState=inactive`, and no
active timer listing. A second read independently confirmed those states.
Scripts, dumps, mirror data and checkpoints remain intact. The schedule
registry records their retirement in this same change.

The collector completed at **21:47:52 UTC**, exit 0. The generated feed
(21:46:58 UTC) reported AA status OK: 325 rankings, 122 configurations,
274 test runs and 110 prices. The dashboard backup completed at
**21:48:54 UTC**, exit 0, with zero missing required members. Local, R2 and
Drive copies were SHA-256 verified:
`18cbcdc35b04d1eebc8fbcf512062e87b16db5faeb74464f0ed642f82a29a8f7`.
Seven pre-existing optional members were missing and existing unclassified
scratch-file warnings remained; these are not concealed as a warning-free run.

The four previously active timers (`fleet-feed-collector`,
`pm-dashboard-backup`, `pm-digest-daily`, `pm-digest-weekly`) were restored
around **21:48 UTC** after health verification, retaining their existing
schedules. No digest was forced and no test push was sent. Normal scheduled
execution, phone registration and end-to-end notification acknowledgment
are separate from the completed collector/backup checks.

The backup has zero `alerts.push_subscriptions` rows. Alert history being
recovered is not evidence that phone delivery is restored. Delivery and
acknowledgment require their own end-to-end verification.

The bounded laptop-tool source inventory found the obsolete Supabase
backup and mirror producers. The CI worker/pruner was already Turso.
Cold-store Supabase labels refer to copying retained R2 backup artifacts,
not querying the deleted database; these preservation duties stay in place.
The inventory does not establish that every remote Muse sandbox consumer
has been discovered or migrated.
