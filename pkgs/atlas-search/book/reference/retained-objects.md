# Retained objects and retention classes

Source: [worker/src/retained_object.ts](https://github.com/developing-today/code/blob/main/pkgs/atlas-search/worker/src/retained_object.ts), [worker/src/retention_report.ts](https://github.com/developing-today/code/blob/main/pkgs/atlas-search/worker/src/retention_report.ts), [worker/src/audit_report.ts](https://github.com/developing-today/code/blob/main/pkgs/atlas-search/worker/src/audit_report.ts), [worker/src/index.ts](https://github.com/developing-today/code/blob/main/pkgs/atlas-search/worker/src/index.ts) — provenance only; this chapter is complete without following it.

Recommendations, search history and hot-data snapshots are stored in the Worker as versioned, timestamped snapshots that share one envelope and each sit under a retention class (#9140, build slices 1–7, landed 2026-11-26). They live in the D1 database bound as `RETAINED`, in the `atlas_retained_objects` and `atlas_retention_classes` tables (migrations `0003_retained_objects.sql`, `0004_retained_objects_retirement.sql`, `0005_retention_modes_and_reports.sql`). With no `RETAINED` binding the admin routes below answer 503 `retained_off`, and every writer stays off.

## The envelope

One version of one retained object carries exactly these fields:

| Field       | Meaning                                                                 |
| ----------- | ----------------------------------------------------------------------- |
| `objectId`  | The logical object the versions belong to; non-empty.                   |
| `family`    | One of `recommendation`, `search_history`, `hot_snapshot`.              |
| `class`     | The retention class holding it (below).                                 |
| `capturedAt` | RFC 3339 time the snapshot was taken.                                  |
| `version`   | Monotonic per `objectId`, from 1. Appends may name an expected current version; a mismatch is refused as `version_conflict`, so two writers cannot both land the same version. |
| `inputsRef` | What produced it: query ids, a parent snapshot id, a retrieval-stats version id. Non-empty. |
| `payload`   | The family payload, stored as JSON.                                     |

A draft whose family its class does not hold is refused as `family_class_mismatch`; a malformed draft as `malformed_envelope`. One payload rule is applied at write time: a `search_history` payload loses its `query` text unless its class allows query text — by default no class does.

## The four retention classes

The design defaults are a floor, not a starting suggestion: a class rule may lengthen the window or raise the keep-newest count, never go below the default (`retention_below_default`). Every class starts disabled, and a store append refuses while its class is disabled (`retention_class_disabled`), so each family's writer lands behind its class's `enabled` flag.

| Class            | Holds           | Window | Keep newest | Notes |
| ---------------- | --------------- | ------ | ----------- | ----- |
| `hot-short`      | `hot_snapshot`  | 24 h   | 2           | The short class for hot snapshots (slice 4). |
| `recommendation` | `recommendation` | 30 days | 10          | One version per recommendation recompute (slice 2); the envelope's `inputsRef` names the parent snapshot, and the payload is the ordered item refs with scores and the producing rule id. |
| `search-history` | `search_history` | 90 days | 20          | One version per answered search in a named session (slice 3); keeping history never changes or fails the search itself. |
| `snapshot-audit` | any family      | 365 days | 50         | The audit set (slice 5, below). |

Hot snapshots (slice 4) hold at most the top 10 items of the recommendation order (`HOT_WINDOW`), captured each time the recommendation accumulator pins a new snapshot for a tenant and subject.

## The admin audit set

An admin pins a stored version of any retained object into the audit set: a copy under `audit:<object id>` in the `snapshot-audit` class, whose `inputsRef` names the source version (`version:<n>`). Pinning the same version again returns the existing pin. Because the report reads only audit envelopes and payloads, a before/after comparison still works after the source version has been retired from its own, shorter class. Recommendation scores are compared only within one rule id: when the two pinned versions name different rules, the report still lists added, removed and moved items but no score changes.

The routes are part of [The Worker HTTP API](worker-api.md) and sit behind its shared gate:

```http
POST /v1/admin/retained/audit   {object, version}
GET  /v1/admin/retained/compare?object=<id>&from=<version>&to=<version>
POST /v1/admin/retained/report
POST /v1/admin/retained/promote {class}
```

## Modes, reports and promotion

Each class is in one of two modes: `dry-run` (the default) or `live-cold`. No mode deletes anything: retiring a version is the cold step, marking `retired_at_ms`; the row stays stored.

A retention report (engine `atlas-retained-report-v1`) lists, per class, every live version its rule would let go: older than the class window (the age trigger) and outside the keep-newest floor. Versions the age trigger fired on but the floor kept are counted as protected. In `dry-run` every candidate's action is `report-only`; in `live-cold` it is `cold-tier`. Reports name versions by object id and version only, never payload content, record the rule set they ran under, and carry deltas against the previous report of the same kind.

Promotion is the one reviewed step, and it is a route, not a flag flip:

- `POST /v1/admin/retained/promote {class}` moves a class from `dry-run` to `live-cold` only once the class appears unchanged — same window, keep-newest and mode — in the two latest **scheduled** reports; otherwise it refuses `not_earned`. On-demand reports (from `POST /v1/admin/retained/report`) never count toward promotion.
- Changing a class's window or keep-newest floor returns the class to `dry-run` and clears its promotion record: a changed rule is a new rule.
- The successful promote response names the class, the new mode and the two scheduled report run ids that earned it (`promoted_from`).

## The scheduled retention cycle

The Worker's cron trigger (`17 */6 * * *` in `wrangler.toml`) runs one retention cycle per scheduled event (slice 7), and does nothing when no `RETAINED` binding exists. One cycle stores the scheduled report for that cron time — the reports promotion counts — then retires what the report lists in `live-cold` classes only: versions older than their class window and outside the keep-newest floor. A `dry-run` class is reported and never touched. A cycle retires at most 200 objects (`MAX_RETIRE_OBJECTS_PER_CYCLE`); the rest are deferred to the next cycle, whose report still lists them. A second delivery of the same cron time finds its report already stored and does nothing.
