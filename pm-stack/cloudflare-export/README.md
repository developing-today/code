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
| neurons/day alert | `aiInferenceAdaptiveGroups` grouped by `date` + `modelId` (limit 10000, sized for cycle-days × models) — see the alert section below |

Metric ids match the aggregator's Cloudflare card
(`pm-stack/aggregator/src/saas-extract.ts`), which prefers a fresh
(< 30 min) export for the card's `used` values and falls back to
tracker prose when the export is missing or stale.

## Workers AI neurons/day alert (code#2287)

The `workers-ai-neurons` metric is a per-day **average** over the
cycle, which dilutes a one-day burn past visibility — the Oct 24–25
2026 burn (4.11M neurons, ~$45 PAYG) averaged to ~145k/day and was
invisible for days. The exporter therefore also runs the grouped
per-day/per-model query and evaluates a threshold rule on every tick
(the 15-minute cadence surfaces a crossing the same day):

- **Rule:** any UTC day in the cycle-to-date whose total neurons
  **exceed** the threshold fires an alert.
- **Threshold:** env `CLOUDFLARE_AI_NEURONS_DAY_ALERT_THRESHOLD`,
  default **50000** (5× the 10k free daily allowance). Unset, empty,
  non-numeric, or negative values fall back to the default, so a typo
  never silently disables the alert. Retune via the env — no code
  change.
- **Surfacing:** the success document gains an `alerts` array,
  present **only** when at least one day is firing:

  ```json
  "alerts": [{
    "kind": "workers-ai-neurons-day",
    "date": "2026-10-24",
    "totalNeurons": 3090000,
    "threshold": 50000,
    "topModels": [{ "model": "@cf/deepseek-ai/deepseek-v4-pro-0813", "neurons": 2990000 }]
  }]
  ```

  The aggregator (`pm-stack/aggregator/src/saas-extract.ts`,
  `applyCloudflareExport`) appends each alert loudly to the
  Cloudflare card's `sourceNote`. `history.jsonl` already persists
  every tick's document, so firings are auditable after the fact. A
  document without `alerts` renders exactly as before.
- **Saturation:** the grouped query's limit is 10000 (cycle-days ×
  models). If a result ever hits it, day totals are lower bounds:
  alerts carry `lowerBound: true` and the document carries
  `aiNeuronsByDaySaturated: true` — flagged, never silently truncated
  (the same discipline as `r2-storage`).

The average metric is unchanged (card compatibility); the alert is
additive. The prevention side (config guard) is code#2288, separate.

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
