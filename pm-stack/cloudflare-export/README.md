# Cloudflare quota exporter — code#1048

Laptop-side live collector for Cloudflare usage, delivered as the
quota-gatherer's file-export adapter on the #1151 15-minute tick
(amended cadence, 2026-10-27 — not a standalone hourly job). Part of
epic #1047; triage verdict + pinned shape: task 2309 on #1048.

## Why laptop-side, why Bun

The day-to-day Cloudflare API token lives in Bitwarden Secrets
Manager (`cloudflare.api-token-day-to-day`). The wrapper
(`cloudflare-quota-export.sh`) fetches it by name into the process
environment only — the value never lands in a file, log, or the
export document. The exporter runs under **Bun**: Node's fetch/https
egress times out on this laptop (observed 2026-10-29; raw TCP, curl,
and Bun connect fine).

## What it reads (current cycle = calendar month UTC)

One GraphQL analytics POST (`/client/v4/graphql`) plus REST probes:

| metric id | dataset |
| --- | --- |
| `workers-requests`, `workers-cpu-ms` | `workersInvocationsAdaptive` sum (requests, cpuTimeUs ÷ 1000) |
| `kv-reads`, `kv-writes` | `kvOperationsAdaptiveGroups` by actionType (writes+deletes+lists fold, the card's grouping) |
| `d1-rows-read`, `d1-rows-written` | `d1AnalyticsAdaptiveGroups` sum |
| `queues-ops` | `queueMessageOperationsAdaptiveGroups` sum billableOperations |
| `r2-class-a`, `r2-class-b` | `r2OperationsAdaptiveGroups` by actionType, classified per Cloudflare's published pricing classes (deletes free; unknown actions reported in `r2UnclassifiedActions`, never guessed) |
| `r2-storage` | `r2StorageAdaptiveGroups` max payloadSize → GB; bucket names enumerated directly via `GET /accounts/{id}/r2/buckets` |
| `workers-ai-neurons` | `aiInferenceAdaptiveGroups` sum totalNeurons, as a per-day average over the cycle to date (cycle total in the metric's `detail`) |

Metric ids match the aggregator's Cloudflare card
(`pm-stack/aggregator/src/saas-extract.ts`), which prefers a fresh
(< 30 min) export for the card's `used` values and falls back to
tracker prose when the export is missing or stale.

## Billing: invoice-only (settled)

Probed live 2026-10-29: `GET /accounts/{id}/billing/profile` and
`GET /accounts/{id}/subscriptions` both return **403** with the
day-to-day token. Per the issue's sanctioned fallback, the export
document records `billing: { status: "invoice-only", probes: [...] }`
— billed state comes from the monthly invoice only, and the metrics
are explicitly **not-yet-billed** current-cycle consumption
(billed vs not-yet-billed both represented). quota-view Part (b)
states the same. Creating a scoped billing token is Drew's account
action and out of scope.

## Export format

`~/atlas/cloudflare-export/latest.json` (atomic tmp + rename) plus
append-only `history.jsonl`:

```json
{
  "service": "cloudflare",
  "source": "file-export/cloudflare",
  "status": "ok",
  "measuredAt": "<ISO>",
  "cycleStart": "<ISO first of month UTC>",
  "accountId": "...",
  "billing": { "status": "invoice-only", "probes": [{ "endpoint": "...", "status": 403 }], "detail": "..." },
  "r2Buckets": ["..."],
  "metrics": [{ "id": "workers-requests", "used": 75225, "unit": "count", "source": "cloudflare-graphql-analytics" }]
}
```

## Failure states (fail-closed)

A failed tick writes a failure document with `status`,
`attemptedAt`, `detail`, `measuredAt: null`, and **no** metrics:

| status | meaning | exit |
| --- | --- | --- |
| `no-token` | token not in environment (wrapper/bws problem) | 2 |
| `token-scope` | API answered 401/403; `missingScopes` names the scopes needed | 3 |
| `unparseable` | an endpoint answered an unrecognised shape | 6 |
| `fetch-failed` | network/other error | 7 |

## Install / schedule (laptop)

```sh
bash pm-stack/cloudflare-export/install-laptop.sh   # run ON the laptop
```

Installs exporter + wrapper to `~/atlas/cloudflare-export/`, the
systemd user service + 15-minute timer to `~/.config/systemd/user/`,
and starts one run immediately.

Manual run / debugging:

```sh
~/atlas/cloudflare-export/cloudflare-quota-export.sh
journalctl --user -u cloudflare-quota-export.service
```

## Tests

```sh
node --test pm-stack/cloudflare-export/cloudflare-quota-export.test.mjs
```
