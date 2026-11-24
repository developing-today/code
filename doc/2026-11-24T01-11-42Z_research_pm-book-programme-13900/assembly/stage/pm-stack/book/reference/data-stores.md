# Data stores: the Turso estate

The estate holds exactly 50 Turso databases. The count is settled: a live `turso db list` on the laptop on 2026-11-23 returned exactly 50 rows, and every name in this chapter is copied from that list. Every database sits in the single Turso group `great-star-lord`; hostnames are all `…-dezren39.aws-us-east-1.turso.io`. The #13844 Turso audit's headline and group arithmetic (Atlas 33 + fleet plane 4 + `muse-meta` 1 + small product stores 4 + frozen 8 = 50) match the live list name for name outside the Atlas shard families.

This book covers 15 of the 50 in full: the seven live PM stores and the frozen eight as one grouped entry. The other 35 are named in the table at the end: 34 with their covering book and chapter from the coverage ledger (the 33 Atlas databases and `fleet-queue`), and `fleet-dispatch-jobs`, the one gap the ledger assigns to the fleet book. Settled ledger arithmetic: 34 covered + 15 PM gaps + 1 fleet gap = 50. Three further stores named in the ledger are D1, not Turso, and close the chapter.

Every full entry follows the same shape: what the store is, its shape, its behaviour, and its failure shapes where the survey records them.

## `fleet-platform`

- **What it is:** The PM and dashboard plane store: the `agent_mirror`, quota, aggregator, and ingest families, alerts, and decision events. Per the #13844 audit, this store takes 87% of the estate's writes.
- **Shape:** Named table families in the survey record: the alerts tables (`incidents`, `push_subscriptions`, `notification_sends`) behind the alert throat, and `decision_set_events` for decision-set activity.
- **Behaviour:** The aggregator writes it through the Turso HTTP API — from the laptop, not from a Worker: the #13844 audit named the aggregator and the dashboard as Workers, and both names are the one laptop service `pm-aggregator` (see [The workers](workers.md), D18). The dashboard — that same aggregator UI — reads and writes its ingest families. The quota-check helper reads it over the libsql HTTP protocol with a per-database token fetched by name. Orchestration state is written to it one transaction per pass by the shared-schema writer (`pm-stack/shared-schema/orchestration-writer.py`, #5005, #3142).
- **Failure shapes:** None recorded in the survey.

## `fleet-mirror`

- **What it is:** The lagging second copy of the `agent_mirror` dataset, 175 MB.
- **Behaviour:** A mirror, not a primary: it trails `fleet-platform`'s mirror dataset. The quota-check helper reads it with its own per-database token, fetched by name.
- **Failure shapes:** None recorded in the survey. Its frozen recovery copy is `fleet-mirror-recovery-stage-20261118`, in the grouped entry below.

## `muse-meta`

- **What it is:** The coordination store.
- **Shape:** Tables named in the ledger: `coord_events`, `coord_tasks`, `coord_workers`, `meta_notes`, and `mail_messages`.
- **Behaviour:** The store the fleet's coordination records live in: events, tasks, worker records, notes, and mail messages in one database.
- **Failure shapes:** None recorded in the survey.

## `notes-submissions`

- **What it is:** The notes submissions product store.
- **Shape:** Under 250 rows at audit time.
- **Behaviour:** A small product store; the audit records its size and its product role and nothing further.
- **Failure shapes:** None recorded in the survey.

## `bedside-and-away`

- **What it is:** A small product store.
- **Shape:** Under 15 rows at audit time.
- **Behaviour:** The audit records its size only; no writer or reader is named for it in the survey record.
- **Failure shapes:** None recorded in the survey.

## `voice-line-ledger`

- **What it is:** The voice-line Worker's ledger.
- **Shape:** 4 rows at audit time.
- **Behaviour:** Owned by the `voice-line` Worker (see [The workers](workers.md)); the ledger records the voice line's state alongside its Durable Object sessions.
- **Failure shapes:** None recorded in the survey.

## `great-star-lord`

- **What it is:** A third-party application's database (Charm/Wish), held in the same Turso estate.
- **Behaviour:** Standing keep-by-name, zero usage. Per the audit it STAYS on Turso: it is not a migration candidate and no PM component reads or writes it.
- **Note:** `great-star-lord` is also the name of the Turso group that holds all 50 databases in this chapter. The group name and this database's name are the same string; they are not the same thing.

## The frozen eight (grouped entry)

- **What they are:** Eight frozen staging and recovery copies from 2026-11-17 and 2026-11-18, named in the audit and confirmed name-for-name in the live list:
  - `fleet-supabase-recovery-20261117` — the frozen 2026-11-17 Supabase recovery copy, 422 MB.
  - `fleet-supabase-runtime-stage-20261118` — frozen 2026-11-18 staging copy.
  - `fleet-supabase-runtime-legacy-stage-20261118` — frozen 2026-11-18 staging copy.
  - `fleet-supabase-runtime-tx-stage-20261118` — frozen 2026-11-18 staging copy.
  - `fleet-platform-reconcile-stage-20261118` — frozen 2026-11-18 staging copy.
  - `fleet-platform-batched-stage-20261118` — frozen 2026-11-18 staging copy.
  - `fleet-platform-short-tx-stage-20261118` — frozen 2026-11-18 staging copy.
  - `fleet-mirror-recovery-stage-20261118` — frozen 2026-11-18 staging copy.
- **Behaviour:** Frozen: no writer or reader is recorded for any of the eight. They are point-in-time copies of the Supabase recovery and of the `fleet-platform` and `fleet-mirror` staging runs.
- **Disposition:** The audit recommends export-to-object-storage and delete for the eight copies rather than migration. They are assigned to this book so the estate's completion test holds until that decision is taken.
- **Failure shapes:** None recorded in the survey.

## The other 35

| Database                                  | Covering book and chapter (per the ledger)                                                                                                                                                                                                                                                                                                                                   |
| ----------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `atlas`                                   | Atlas book — `reference/data-stores.md`, `concepts/corpus-and-provenance.md`. The Atlas monolith: sessions, messages, chunks, native vector and FTS5 store, 7.28 GB. Published at [https://developing-today.github.io/code/atlas/](https://developing-today.github.io/code/atlas/)                                                                                           |
| `atlas-ask-ledger`                        | Atlas book — `reference/data-stores.md`. Ask exports, queries, usage counters (20 rows).                                                                                                                                                                                                                                                                                     |
| `atlas-core-internal-content`             | Atlas book — `reference/data-stores.md`. The shards' content store: session and message text, no vectors.                                                                                                                                                                                                                                                                    |
| `atlas-core-secret-content`               | Atlas book — `concepts/sensitivity.md`, `reference/data-stores.md`. Secret-tier shard.                                                                                                                                                                                                                                                                                       |
| `atlas-core-secret-main-01`               | Atlas book — `concepts/sensitivity.md`, `reference/data-stores.md`. Secret-tier shard.                                                                                                                                                                                                                                                                                       |
| `atlas-core-internal-agent-tooling-01`    | Atlas book — `reference/data-stores.md`, `concepts/corpus-and-provenance.md`. Content shard.                                                                                                                                                                                                                                                                                 |
| `atlas-core-internal-agent-tooling-02`    | Atlas book — `reference/data-stores.md`, `concepts/corpus-and-provenance.md`. Content shard.                                                                                                                                                                                                                                                                                 |
| `atlas-core-internal-agent-tooling-03`    | Atlas book — `reference/data-stores.md`, `concepts/corpus-and-provenance.md`. Content shard.                                                                                                                                                                                                                                                                                 |
| `atlas-core-internal-agent-tooling-04`    | Atlas book — `reference/data-stores.md`, `concepts/corpus-and-provenance.md`. Content shard.                                                                                                                                                                                                                                                                                 |
| `atlas-core-internal-cloudflare-01`       | Atlas book — `reference/data-stores.md`, `concepts/corpus-and-provenance.md`. Content shard.                                                                                                                                                                                                                                                                                 |
| `atlas-core-internal-fleet-repos-01`      | Atlas book — `reference/data-stores.md`, `concepts/corpus-and-provenance.md`. Content shard.                                                                                                                                                                                                                                                                                 |
| `atlas-core-internal-fleet-repos-02`      | Atlas book — `reference/data-stores.md`, `concepts/corpus-and-provenance.md`. Content shard.                                                                                                                                                                                                                                                                                 |
| `atlas-core-internal-fleet-repos-03`      | Atlas book — `reference/data-stores.md`, `concepts/corpus-and-provenance.md`. Content shard.                                                                                                                                                                                                                                                                                 |
| `atlas-core-internal-home-lab-01`         | Atlas book — `reference/data-stores.md`, `concepts/corpus-and-provenance.md`. Content shard.                                                                                                                                                                                                                                                                                 |
| `atlas-core-internal-home-lab-02`         | Atlas book — `reference/data-stores.md`, `concepts/corpus-and-provenance.md`. Content shard.                                                                                                                                                                                                                                                                                 |
| `atlas-core-internal-infra-nix-01`        | Atlas book — `reference/data-stores.md`, `concepts/corpus-and-provenance.md`. Content shard.                                                                                                                                                                                                                                                                                 |
| `atlas-core-internal-infra-nix-02`        | Atlas book — `reference/data-stores.md`, `concepts/corpus-and-provenance.md`. Content shard.                                                                                                                                                                                                                                                                                 |
| `atlas-core-internal-media-01`            | Atlas book — `reference/data-stores.md`, `concepts/corpus-and-provenance.md`. Content shard.                                                                                                                                                                                                                                                                                 |
| `atlas-core-internal-misc-01`             | Atlas book — `reference/data-stores.md`, `concepts/corpus-and-provenance.md`. Content shard.                                                                                                                                                                                                                                                                                 |
| `atlas-core-internal-models-and-plans-01` | Atlas book — `reference/data-stores.md`, `concepts/corpus-and-provenance.md`. Content shard.                                                                                                                                                                                                                                                                                 |
| `atlas-core-internal-models-and-plans-02` | Atlas book — `reference/data-stores.md`, `concepts/corpus-and-provenance.md`. Content shard.                                                                                                                                                                                                                                                                                 |
| `atlas-core-internal-people-pets-01`      | Atlas book — `reference/data-stores.md`, `concepts/corpus-and-provenance.md`. Content shard.                                                                                                                                                                                                                                                                                 |
| `atlas-core-internal-personal-admin-01`   | Atlas book — `reference/data-stores.md`, `concepts/corpus-and-provenance.md`. Content shard.                                                                                                                                                                                                                                                                                 |
| `atlas-core-internal-personal-admin-02`   | Atlas book — `reference/data-stores.md`, `concepts/corpus-and-provenance.md`. Content shard.                                                                                                                                                                                                                                                                                 |
| `atlas-core-internal-personal-admin-03`   | Atlas book — `reference/data-stores.md`, `concepts/corpus-and-provenance.md`. Content shard.                                                                                                                                                                                                                                                                                 |
| `atlas-core-internal-personal-admin-04`   | Atlas book — `reference/data-stores.md`, `concepts/corpus-and-provenance.md`. Content shard.                                                                                                                                                                                                                                                                                 |
| `atlas-core-internal-security-01`         | Atlas book — `reference/data-stores.md`, `concepts/corpus-and-provenance.md`. Content shard.                                                                                                                                                                                                                                                                                 |
| `atlas-core-internal-work-engineering-01` | Atlas book — `reference/data-stores.md`, `concepts/corpus-and-provenance.md`. Content shard.                                                                                                                                                                                                                                                                                 |
| `atlas-core-internal-work-engineering-02` | Atlas book — `reference/data-stores.md`, `concepts/corpus-and-provenance.md`. Content shard.                                                                                                                                                                                                                                                                                 |
| `atlas-core-internal-work-engineering-03` | Atlas book — `reference/data-stores.md`, `concepts/corpus-and-provenance.md`. Content shard.                                                                                                                                                                                                                                                                                 |
| `atlas-core-internal-work-engineering-04` | Atlas book — `reference/data-stores.md`, `concepts/corpus-and-provenance.md`. Content shard.                                                                                                                                                                                                                                                                                 |
| `atlas-core-internal-work-engineering-05` | Atlas book — `reference/data-stores.md`, `concepts/corpus-and-provenance.md`. Content shard.                                                                                                                                                                                                                                                                                 |
| `atlas-core-internal-work-engineering-06` | Atlas book — `reference/data-stores.md`, `concepts/corpus-and-provenance.md`. Content shard.                                                                                                                                                                                                                                                                                 |
| `fleet-queue`                             | Fleet book — `concepts/queue.md`, `library/queue-client.md`, `tutorials/run-the-brokers.md`. The queue broker core: tasks, events, leases, placement, actions-exec tables. Published at [https://developing-today.github.io/code/fleet/](https://developing-today.github.io/code/fleet/)                                                                                     |
| `fleet-dispatch-jobs`                     | Fleet book — this database is the fleet book's gap, not this book's: the fleet book is its home, and its fleet chapter is landing in the same programme (lane FLEET-SIDELINE-2, unmerged at this book's build time). The dispatch jobs store (`jobs`, 6 rows). Published at [https://developing-today.github.io/code/fleet/](https://developing-today.github.io/code/fleet/) |

## Not Turso: the D1 stores

Three stores named alongside the estate in the ledger and audit are Cloudflare D1 databases, not Turso databases, and are excluded from the 50:

- `decision-events` — the decision-events Worker's sibling D1 database; its schema is the Worker's `schema.sql`. See [The workers](workers.md).
- `labops-db` — the lab-ops-hub Worker's D1 database, alongside its R2 bucket `labops-files`.
- `atlas-hot-cache` — the Atlas search Worker's D1 hot cache (#9134), in front of the Turso corpus; covered by the Atlas book.

Sources: PM-CENSUS coverage ledger Table B and gap summary; the settled estate list (live `turso db list`, laptop, 2026-11-23, lane TURSO-SETTLE); the #13844 Turso audit (PR #13845); PM-SURVEY survey file `apps.md` (§23, §25), census date 2026-11-23.
