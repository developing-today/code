# PM-stack current-state ownership map

**Inventory date:** 2026-11-05 (laptop-local checks; all paths below are host-local unless noted).

**Scope:** #3989 acceptance criterion 1 only. This records observed homes, writers, host, and backup path; it does not move data, change deployments, fence writers, or run restore/failover drills.

## Ownership map

| Service | Canonical data home today | Single active writer observed | Current host | Backup method and home today | Target data home per latest #1258 ruling |
|---|---|---|---|---|---|
| **Aggregator** | Split today: aggregator-owned JSON/state files under `/home/user/.local/state/pm-aggregator/` (including comments, actions, registry, feed, usage snapshots, and other runtime state); the current repo/schema map also documents some alert/usage/session records in Supabase `muse-dev2` (remote rows were not queried for this inventory). GitHub remains canonical for GitHub-owned issues, PRs, comments, and Project records. | One active `pm-aggregator.service` instance for aggregator API writes. The feed collector and quota gatherer write their separate producer-owned files; `pm-projector.service` is a separate writer for projector/link state, not a second aggregator API instance. | Laptop `amd` | Daily `pm-dashboard-backup` creates a dated local archive under `~/backups/dashboard/`, then copies the same verified archive to R2 and Drive. The 2026-11-04 run record says `complete`, exit 0, and local/R2/Drive `ok`. Core aggregator files are included, but this is an allowlist, not a snapshot of the whole state directory; see coverage gaps below. | Supabase project `muse-dev2` for PM-stack state. GitHub-owned records remain in GitHub. |
| **Kaneo / PostgreSQL** | Kaneo application state is in the `kaneo` database served by `pm-stack-postgres-1`; PostgreSQL data is in Docker volume `pm-stack_postgres_data`, mounted at `/var/lib/postgresql/data`. The Kaneo app container has no separate data volume. | One `pm-stack-kaneo-1` app container, backed by one `pm-stack-postgres-1` database container; both were healthy when checked. | Laptop `amd` | The daily external PM-stack dump helper produces a dated archive at `~/backups/pm-stack/YYYY-MM-DD/pm-stack-YYYY-MM-DD.tar.zst`. The 2026-11-04 archive contains a Kaneo `pg_dump` and globals dump; it is included in the dashboard archive whose latest recorded local/R2/Drive destinations were all `ok`. | Supabase project `muse-dev2` for PM-stack state. The ruling does not prescribe a specific compatible Kaneo/PostgreSQL representation. |
| **Grist** | Docker volume `grist_grist_data`, mounted at `/persist` in the `pm-grist` container; this contains the Grist home/session SQLite databases and native `.grist` documents. | One active `pm-grist` container managed by `docker-pm-grist.service`. | Laptop `amd` | The PM-stack archive uses online SQLite/document backups. The 2026-11-04 archive contains the Grist home database, session database, and document store; it rides the dashboard archive's local/R2/Drive legs. | Supabase project `muse-dev2` for PM-stack state. The physical mapping for Grist's native documents is still unresolved; do not silently assume a Supabase Postgres table or a persistent volume. |
| **Pad** | Docker volume `pad_pad_data`, mounted at `/data` in `pm-pad`; the database is `/data/pad.db`, with attachments and required app configuration/key material in the backup set. | One active `pm-pad` container managed by `docker-pm-pad.service`. | Laptop `amd` | The PM-stack archive uses a SQLite backup plus attachments/configuration and required key material. The 2026-11-04 archive contains the Pad database, attachments, and configuration/key files; it rides the dashboard archive's local/R2/Drive legs. | Supabase project `muse-dev2` for PM-stack state. The physical mapping for Pad's SQLite database and attachments is still unresolved; no native-store migration path is assumed here. |

### Observation details and backup limits

- On 2026-11-05, `docker ps` showed one Kaneo container, one PostgreSQL container, one Grist container, and one Pad container on host `amd`; all four app/database containers were running. `docker inspect` confirmed the three named-volume mounts above. The user systemd units showed one active `pm-aggregator.service`, one active `pm-projector.service`, and active Grist/Pad service units. These are observations of the current single-host deployment, not a claim that failover or restore has been tested.
- The local aggregator state directory contains `activity-ingestion.json` and `freshness.json`; both are absent from the readable 2026-11-04 dashboard archive. Nix also configures a `roadmap-annotations.json` path in that directory; no such file was present at inspection, and the backup allowlist omits it. The dashboard script's required/optional set includes core files such as actions, comments, feed, registry, events, coverage, and snapshots, but does not cover every deployed runtime path. Classify/cover these stores before cutover; this inventory does not change the backup job.
- The checked-in laptop backup declaration describes its separate Supabase dump job as backing up project `muse-dev`, while the current PM-state target is `muse-dev2`. This inventory did not establish that `muse-dev2` has an equivalent verified off-machine backup/restore path. Do not count the current dashboard archive as proof that the future Supabase target is protected.
- The daily PM-stack archive is generated by a helper under `~/tools/` outside this repository and is conditionally added to the dashboard bundle. The 2026-11-04 dashboard run record is current positive evidence for local/R2/Drive copies, and the local archive was readable with the expected service-store categories. It is not a restore drill. Issue [#1514](https://github.com/developing-today/code/issues/1514) remains open; its latest update says the remaining acceptance item is a Kaneo restore into a scratch container. No restore was performed for this inventory.
- `pm-dashboard-backup` is a daily user timer. The checked-in job declaration specifies 30-day retention for its verified local/R2/Drive copies. #1258's per-service off-machine retention decision remains open despite the current job's configured retention.

## #1258 decisions and remaining questions

The design note's original proposal described Kaneo on cluster PostgreSQL and Grist/Pad on cluster persistent storage. Later owner rulings recorded in #1258 on 2026-11-03 supersede that split: **all PM-stack state is to have its canonical home in Supabase `muse-dev2`**, with no cluster PostgreSQL or cluster persistent volume treated as the system of record. This map follows the later ruling, not the older proposal or the still-unreconciled wording in #3989 and `pm-stack/SHARED-SCHEMA-MAP.md`.

The following items are settled in #1258, but are goals and have not been measured by this inventory:

- **Recovery objectives accepted:** 4-hour core-stack RTO; 15-minute RPO for aggregator actions/comments/identity; 1-hour RPO for Kaneo plus canonical usage/link records; 24-hour RPO for Grist/Pad and Project export.
- **Failover posture accepted:** tested cold restore within 4 hours, with GitHub as degraded mode; no warm standby or automatic multi-writer failover.
- **Evidence cadence/location approved:** monthly rotating store restores, quarterly full-stack restores, annual laptop-off drill; reports under `doc/pm-stack/restore-drills/YYYY-MM.md`.
- **Laptop-only exceptions:** none; no unique PM record is intended to remain laptop-only.

Explicit questions still needing resolution under #1258 before the dependent cutover work proceeds:

1. **Native-store mapping:** Drew's all-in-Supabase ruling names the logical home, but what supported representation and restore path will Kaneo/PostgreSQL, Grist's native documents, and Pad's SQLite database/attachments use in `muse-dev2`? #1258 explicitly says any store that cannot literally live in Supabase Postgres must be mapped in the cutover plan; this inventory does not invent that mapping.
2. **Backup target and retention (Q3):** what verified off-machine backup/restore path and retention applies to `muse-dev2` and each migrated store? The checked-in Supabase backup declaration names `muse-dev`, not `muse-dev2`; confirm the real target and recovery coverage before relying on it.
3. **Project export retention (Q6):** how long should the GitHub Project audit export be retained off-machine?
4. **Runtime host and stale target language:** which exact non-laptop host will run the services under the accepted cold-restore posture, and should #3989's current cluster/alternate-host wording and the older design/schema-map target text be reconciled to the all-in-Supabase ruling before implementation?

## Relationship to #3989

This document supplies the current-state inventory for **#3989 acceptance criterion 1** only. It does not claim laptop-off availability, a cutover, writer fencing, measured RTO/RPO, or a successful restore. The remaining #3989 criteria still wait on the unresolved #1258 mapping/backup decisions and on #1514's remaining restore evidence. #3142 remains the owner of aggregator schema migration; #3869 remains the owner of silent/empty backup-failure handling.

## Evidence references

Live checks on 2026-11-05 were read-only: host name; running container list and volume mounts; systemd service/timer state; aggregator state filenames (not file contents); the 2026-11-04 backup run record; and archive member categories. GitHub was freshly queried for #3989, #1258, and #1514, including #1258's 2026-11-03 owner rulings and #1514's 2026-11-04 backup-run update.

Relevant checked-in declarations:

- Aggregator state directory and service: [`nixos/services/pm-stack/default.nix`](../../nixos/services/pm-stack/default.nix#L75-L79) and [aggregator unit/config](../../nixos/services/pm-stack/default.nix#L125-L169); [installed file paths](../../pm-stack/aggregator/install.sh#L99-L113).
- Kaneo/PostgreSQL deployment: [`pm-stack/compose.yml`](../../pm-stack/compose.yml#L14-L41). Grist/Pad OCI containers and mounts: [`nixos/services/pm-apps/default.nix`](../../nixos/services/pm-apps/default.nix#L84-L111).
- Dashboard backup member set and DB-dump inclusion: [`nixos/home/pm-dashboard-backup.sh`](../../nixos/home/pm-dashboard-backup.sh#L329-L380); schedule/destinations and separate `muse-dev` Supabase dump declaration: [`nixos/home/laptop-backups.nix`](../../nixos/home/laptop-backups.nix#L12-L39).
- The older proposal and original design questions remain in the [#1258 design note](../2026-11-01T21-12-00Z_pm-stack-durability-host-independence/2026-11-01T21-12-00Z_pm-stack-durability-host-independence.md#store-inventory-and-proposed-homes); its target table predates the later owner rulings recorded in [issue #1258](https://github.com/developing-today/code/issues/1258).
- Related issues: [#3989](https://github.com/developing-today/code/issues/3989), [#1514](https://github.com/developing-today/code/issues/1514), [#3142](https://github.com/developing-today/code/issues/3142), and [#3869](https://github.com/developing-today/code/issues/3869).
