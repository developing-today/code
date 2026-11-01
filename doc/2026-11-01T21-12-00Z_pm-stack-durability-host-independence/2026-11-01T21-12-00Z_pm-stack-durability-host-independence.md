# PM-stack durability and host independence

Status: implementation design proposed for Drew's review; the numeric targets below are planning targets, not a claim that the current stack meets them.

Issue: [#1258](https://github.com/developing-today/code/issues/1258) · parent: [#1184](https://github.com/developing-today/code/issues/1184) · related state-migration direction: [#3142](https://github.com/developing-today/code/issues/3142).

## Decision summary

Treat the laptop as a development/administration client, not the PM stack's only durable host. Keep each service's authoritative state under that service's ownership, but put mutable aggregator records in the existing Supabase direction, run the stateful services from the cluster when its deployment is ready, and keep verified off-machine backups until each move has passed a restore and a laptop-off drill. Do not make the aggregator's JSON cache, a Docker volume on the laptop, or a copied secret file the only copy of anything that cannot be regenerated.

The older one-local-libSQL/Turso-shaped shared-store proposal in code#1744 is superseded as the default for new PM-stack state by the 2026-10-31 decision recorded on code#3142: start with Supabase `muse-dev2`, domain-appropriate schemas, and phased migrations. Keep service-owned databases (Kaneo, Grist, Pad) separate unless their maintainers prove a supported shared-database mode; a common backup/restore and host lifecycle do not require merging their schemas.

## Current evidence and ownership

This is a design inventory, not a live-data census. Recheck the inventory and backup age in the implementation work before cutover.

- `pm-stack/aggregator/src/config.ts` defines file-backed comments, actions, the identity registry, fleet feed, and usage snapshots; comments/actions/registry default to JSON under the process working directory. `sources/live.ts` merges session records into the registry file on reads.
- The 2026-11-01 triage of #1258 observed the live aggregator state under `~/.local/state/pm-aggregator/` on the laptop: actions, comments, registry, fleet feed, and snapshots.
- `pm-stack/compose.yml` runs Kaneo with PostgreSQL 16 and a named local Docker volume. Grist and Pad have separate deployments and service-owned data (Grist SQLite/document files; Pad SQLite plus attachments and encryption key).
- The repo's `pm-stack/README.md` has a manual Kaneo `pg_dump` instruction. Issue #1514 records a manually proven multi-store archive and restore; #3869 records that a later scheduled run silently left empty date directories after container-name changes. Thus a backup script or a dated directory alone is not evidence of a good backup.
- code#1459 describes the dashboard state-file backup legs (local, Drive, R2); code#1516 records the wider laptop-backup direction. The current live schedule, freshness, destination, and restore status must be verified rather than inferred from those historical comments.
- code#3142 records the current Supabase-first direction for aggregator state, with comments/actions and identity/provenance first; #941's 2026-10-31 decision says to stamp provenance server-side and store the new identity foundation in Supabase. The session mirror and usage snapshot ingestion already have Supabase-backed pieces according to #3142.
- The GitHub Project remains GitHub-owned. code#1218 (already a child of #1184) is the audit-export work; an export is a reconstruction aid, not a replacement source of truth.

## Store inventory and proposed homes

“Acceptable loss” is the proposed maximum loss after a single failure, assuming the service reports backup freshness honestly. It is not a promise about a provider's current configuration.

| Store / data | Owner and current home | Proposed canonical target | Backup / recovery | Proposed acceptable loss |
|---|---|---|---|---|
| GitHub issues, PRs, comments, reviews | GitHub; GitHub-hosted | Stay in GitHub; do not mirror-write from the PM stack | GitHub's own service and retention; use repo exports only as audit/reconstruction aids | 0 acknowledged GitHub writes under GitHub's service assumptions; no independent RPO is under our control |
| GitHub Project #4 fields, views, items | GitHub Project; export is tracked by #1218 | Stay in GitHub | Daily secret-free schema/field/view/item export under #1218; reconstruct from export plus GitHub issues if needed | ≤24h for the audit export; Project itself remains canonical |
| Aggregator comments and actions, including provenance | Aggregator user writes; presently local JSON | Supabase `muse-dev2`, PM/aggregator-owned tables, following #941 and #3142; local files become cache/export only | Existing Supabase backup path plus a verified offsite export; confirm provider PITR/backup configuration and exercise point-in-time restore | ≤15 minutes for acknowledged writes (target; must be proven by the chosen Supabase recovery mode) |
| Aggregator identity/session registry, operator-token metadata | Aggregator/auth service; registry presently local JSON, with session mirror already in Supabase | Supabase, with identity/provenance tables owned by the auth/aggregator service; secrets themselves remain in Bitwarden/SOPS, never in data rows | Database backup plus secret-free schema/data export; restore identity records before enabling privileged writes | ≤15 minutes for registry/provenance changes; secret material is reissued from vault, not restored from an archive |
| Kaneo boards, workflows, mappings, non-GitHub task metadata | Kaneo owns its application DB; currently PostgreSQL in a laptop Docker volume | PostgreSQL on the basement cluster, in a dedicated Kaneo database/role (not mixed into aggregator tables) | Nightly consistent `pg_dump` + globals/config bundle, manifest/hash verification, copied to laptop backup storage and off-machine R2 + Drive; retain a tested previous generation | ≤1 hour (target; use PITR or increase dump cadence if hourly recovery cannot be demonstrated) |
| Grist home DB, documents and roadmap/register data | Grist owns SQLite/document files in its laptop deployment | Grist service with persistent cluster storage; keep its supported native document format, do not make Supabase its implicit owner | Online SQLite/document export using the supported backup API, plus home/config metadata; encrypted off-machine archive; restore into an isolated Grist instance | ≤24 hours; roadmap/register edits are recoverable from the daily generation |
| Pad plans, database, attachments and encryption key | Pad owns its local-first store and attachments on laptop | Pad service with persistent cluster storage; preserve the app's native store and keep encryption key in the secret flow | Consistent SQLite backup API plus attachments/config and a manifest; encrypt archive; restore key from vault before validation | ≤24 hours; validate readable plans and attachments, not just archive existence |
| Aggregator fleet-feed JSON | Collector pipeline; presently copied to laptop | Derived export/cache; collectors and canonical source rows are the source of truth. Serve from whichever active host runs the aggregator | Rebuild from GitHub/Supabase/session sources; retain the existing dashboard backup only as short-term diagnostics | ≤10 minutes of stale display; regenerate within 10 minutes after host recovery |
| Usage/quota snapshots and histories | Usage collectors/aggregator; local files for some paths, Supabase-backed ingestion for others per #3142 | Supabase canonical snapshot tables where already migrated; finish remaining writers through #3142. JSONL/JSON files are exports/cache | Database backup plus daily verified export; preserve source timestamp and provenance; monitor collector lag | ≤1 hour for canonical snapshot records; local display cache may be lost and rebuilt |
| Projector/link registry and derived projections | Projector owns link writes; currently SQLite plus projections | Put non-rebuildable link/identity writes in the PM schema in Supabase as the #3142 schema map settles; projections remain rebuildable and may be local cache | Back up link writes; regenerate projections from GitHub events/reconciliation; test foreign-key/link continuity | ≤1 hour for explicit link writes; derived projections may be rebuilt from source |
| Secrets, service identities and tunnel credentials | Bitwarden is the secret source; repo SOPS file is encrypted configuration; host age keys are per-host | Remain in Bitwarden/SOPS, not in database dumps or GitHub exports. Add each approved host's age recipient before deploy | Restore by name from Bitwarden at runtime; decrypt SOPS only on the target host; rotate/reissue host-bound credentials when needed | No plaintext-secret recovery from backup; restore/reissue access within the 4-hour service RTO |
| Host-local runtime state, temp files, caches and generated exports | Individual services/laptop | Keep ephemeral by contract; canonical data must be elsewhere | Recreate from canonical DBs and source APIs; exclude transient secrets and caches from authoritative backups | 0 data loss required because this category must contain no unique record |

## Deployment, failover, and service levels

1. **Normal path:** deploy aggregator, Kaneo, Grist, and Pad on the basement cluster as their durable homes are ready. Keep the laptop available for development and operator access, not as the only writer or storage host. The aggregator should read canonical Supabase state and service APIs; host-local JSON is a disposable cache/export.
2. **Cold failover:** if the cluster is unavailable, restore service data to the designated alternate host from the latest verified offsite archive, restore secrets by name from Bitwarden, decrypt the repo config with that host's age key, and start the app units. The goal is a documented, repeatable runbook, not automatic multi-writer failover.
3. **Single writer:** one active writer per service/database. Do not run two Kaneo/Pad/Grist writers against copied state. Do not run two Cloudflare Tunnel connectors for the same hostname unless the tunnel configuration explicitly supports that topology. On failover, fence the old writer before promoting the restore.
4. **Public entry:** keep the stable Cloudflare Access hostname and policy as the public entry. Change the tunnel origin/connector to the active host; keep tunnel credentials in Bitwarden and host-specific configuration in SOPS. Do not require DNS edits or a new user-facing URL for a host move. Validate Access identity propagation separately from application identity/session behavior.
5. **Proposed stack objective:** core PM dashboard available again within **4 hours (RTO)** of losing the primary host; **15 minutes RPO** for aggregator actions/comments/identity and **1 hour RPO** for Kaneo and canonical usage/link records; **24 hours RPO** for Grist, Pad, and Project audit export. These are deliberately proposed targets for Drew to accept or adjust. They require tested backup frequencies and must not be claimed until verified.
6. **Degraded mode:** GitHub remains usable directly if the PM UI is down. The aggregator may serve a visibly stale read-only cache while a database is unavailable, but must never silently accept writes it cannot durably commit. Health must report backup age, restore point, and active host so a green page cannot mask stale or empty archives.

## Restore-drill design

- **Every scheduled backup:** write an atomic archive only after each required source export succeeds; include a manifest with per-file size and SHA-256; verify the finished archive and each destination. A missing/empty archive is a failure, not a successful day. Keep the last known-good archive until a newer generation passes verification.
- **Monthly:** restore one rotating store class (aggregator/Supabase, Kaneo, Grist, Pad, Project export) into an isolated namespace, validate record counts and application-level reads, then remove the scratch restore. Rotate so every class is restored at least once per quarter.
- **Quarterly:** restore the full PM stack into an isolated cluster namespace from off-machine copies, verify the URL/access path in a non-public test route, and record measured RTO/RPO, restore point, checks, and failures.
- **Annually and after a host-move design change:** perform a laptop-off drill: stop relying on the laptop, bring up the cluster-hosted services, confirm the stable public entry and one-writer fencing, then restore the laptop role without changing canonical data.
- **Evidence location:** add one secret-free dated report under `doc/pm-stack/restore-drills/YYYY-MM.md` per drill, linked from the active backup/restore tracker (currently #1516 and #1514). Record archive digests, source timestamps, counts/integrity checks, start/end UTC times, measured RTO/RPO, and follow-up issue links. Never include credentials, raw private records, or copied archives in the report.
- **Failure path:** a failed export, hash check, destination copy, stale backup, or restore is visible in the existing stack health/watchdog path and opens or updates a specific issue; do not count a run as successful merely because its directory or timer exists. #3869 is the current failure-mode reference.

## Implementation issues and boundaries

The design avoids duplicating active work. These existing issues already cover slices and should be treated as implementation work, not re-filed under new numbers:

- [#941 / queued follow-up #4642](https://github.com/developing-today/code/issues/941): server-side provenance and identity foundation; the recorded Drew decision says Supabase first.
- [#3142](https://github.com/developing-today/code/issues/3142): phased Supabase migration and schema map for aggregator state, feeds, and backlogs. Keep file paths working through verified cutover; no big-bang rewrite.
- [#1514](https://github.com/developing-today/code/issues/1514): automated, restorable backup of Kaneo/Grist/Pad data; its manual proof is not completion of the recurring schedule.
- [#3869](https://github.com/developing-today/code/issues/3869): make stale container names / empty day directories observable and prevent an unverified generation from appearing successful.
- [#1218](https://github.com/developing-today/code/issues/1218): auditable GitHub Project export; already a sub-issue of #1184.

Implementation links: [#3989](https://github.com/developing-today/code/issues/3989) covers the remaining host-independence cutover; existing [#1514](https://github.com/developing-today/code/issues/1514) covers application-database backups and [#3869](https://github.com/developing-today/code/issues/3869) covers empty/stale backup generations. All three are attached as native sub-issues of #1258, which is already a sub-issue of #1184. GitHub reports that #1184 already has its maximum 100 direct sub-issues and rejects any additional direct child. We preserved all existing relationships rather than moving a child out of the full epic; this creates an explicit nested implementation group under #1258 and keeps the work in #1184's native descendant tree. #1514 and #3869 had no prior parent. The new #3989 is deliberately separate from #3142 (data-schema migration) and #941/#4642 (provenance/auth foundation).

## Questions for Drew

1. **RTO/RPO:** Are the proposed targets acceptable: 4h core-stack RTO; 15m for aggregator actions/comments/identity; 1h for Kaneo and canonical usage/link records; 24h for Grist, Pad, and Project audit export?
2. **Canonical application DBs:** Approve dedicated cluster PostgreSQL for Kaneo and cluster persistent volumes for Grist/Pad, while aggregator-owned structured state moves to Supabase `muse-dev2`? Or should any of Kaneo/Grist/Pad also become Supabase-hosted after compatibility and ownership review?
3. **Backup destinations and retention:** Keep the current local + R2 + Drive pattern for every stateful service, and what retention should apply off-machine? The per-service implementation owner must confirm actual credentials, bucket/folder scope, and cost boundaries.
4. **Failover posture:** Is a tested cold restore within 4h sufficient, or is a warm standby worth its operational and single-writer complexity?
5. **Restore evidence:** Approve `doc/pm-stack/restore-drills/YYYY-MM.md` as the durable, secret-free report location, with a rotating monthly store restore, quarterly full-stack restore, and annual laptop-off drill?
6. **Project export:** Should #1218's daily export be retained off-machine for a specific retention window, or is the export for audit/reconstruction only?
7. **Laptop-local exceptions:** Is any remaining store intentionally laptop-local after the above moves? If so, name its owner and the data-loss consequence explicitly; otherwise the default is that no unique PM record lives only on the laptop.

## References

- #1258 and epic #1184 (design-series acceptance).
- #3142 (Supabase-first phased migration direction), #1744 (older shared-libSQL proposal to reconcile), #941/#4642 (identity/provenance decision and follow-up).
- #1459 and #1516 (dashboard/laptop backup history), #1514 (Kaneo/Grist/Pad backup), #3869 (silent empty backup days), #1218 (Project export).
- `pm-stack/README.md`, `pm-stack/compose.yml`, `pm-stack/aggregator/src/config.ts`, `pm-stack/aggregator/src/sources/live.ts`.
