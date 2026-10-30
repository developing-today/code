# supabase-export — Supabase usage producer (#1050)

The Supabase half of the usage hub's file-export family (#1151 gatherer,
#1048 Cloudflare adapter shape). Runs on the laptop tick, probes the Supabase
Management API for project `muse-dev`, and writes the document the aggregator
prefers over `quota-view.md` prose on the Supabase card.

```
~/atlas/supabase-export/latest.json    atomic latest document
~/atlas/supabase-export/history.jsonl  one line per tick, success or failure
```

## Document

```json
{
  "service": "supabase",
  "source": "file-export/supabase",
  "status": "ok | partial",
  "measuredAt": "2026-10-29T15:55:00Z",
  "project": "muse-dev",
  "ref": "nltgxorqwxhgfzseiirt",
  "metrics": [
    { "id": "db-size", "used": 1.154, "limit": 0.5, "unit": "gb",
      "source": "management-api:/database/query pg_database_size",
      "flag": "approaching-paid-trigger" }
  ],
  "billing": { "plan": "free", "billedUsd": 0, "measured": true,
               "source": "management-api:/billing/addons (no selected paid addons)" },
  "scope": [ { "endpoint": "organizations/:slug", "status": 403, "note": "scope gap: …" } ]
}
```

Failure documents carry `status` + `attemptedAt` + `detail` and **no** metrics
or billing — a failed tick can never look like a reading. Statuses: `no-token`
(exit 3), `scope-denied` (4), `fetch-failed` (5), `unparseable` (6).

`status: "partial"` means at least one probed endpoint answered >= 400; the
`.scope` array records exactly which, so the card can state the gap instead of
guessing around it.

## Metrics and limits

| Metric | Source | Free limit |
|---|---|---|
| `db-size` | `POST /database/query` → `pg_database_size(current_database())` | 0.5 GB |
| `egress` | **no endpoint reports bytes** — gap recorded, request counts as context | 5 GB |
| `storage` | `GET /storage/buckets` (0 buckets on muse-dev → 0 GB) | 1 GB |

Limits are the Supabase Free-plan allowances recorded on 2026-10-29
(supabase.com/pricing); the API measures usage, it does not publish the numbers.
Any metric at >= 80% of its Free limit carries `flag: "approaching-paid-trigger"`
— the signal for Drew's standing $30/mo spend ceiling.

## Scope probe (2026-10-29, the existing `supabase.*` management token)

Recorded endpoint-by-endpoint in `.scope` on every run:

| Endpoint | Status |
|---|---|
| `GET /projects`, `GET /projects/:ref` | 200 |
| `POST /projects/:ref/database/query` | 201 |
| `GET /projects/:ref/analytics/endpoints/usage.api-counts` | 200 |
| `GET /projects/:ref/analytics/endpoints/usage.api-requests-count` | 200 |
| `GET /projects/:ref/billing/addons` | 200 (`selected_addons: []`) |
| `GET /projects/:ref/storage/buckets` | 200 |
| `GET /projects/:ref/health?services=db` | 200 |
| `GET /organizations/:slug` | **403** — token is project-scoped; org plan/billing reads need an org token (Drew's account action) |
| `GET /organizations/:slug/{billing,usage}` | 403 / 404 |
| `GET /projects/:ref/usage`, `…/database/size` | 404 — no such endpoint |
| egress bytes | no endpoint exists; only request counts |

Creating a token or granting org consent is out of scope for the producer; the
gap is recorded, not worked around.

## Token

Resolved by **name** from Bitwarden Secrets Manager at runtime
(`supabase.access-token-muse-cli`, override `SUPABASE_TOKEN_SECRET_NAME`;
`SUPABASE_MANAGEMENT_TOKEN` wins if set). The value is sent only to
`api.supabase.com` and is never written to the document, a log line, a fixture,
or argv.

## Run

```bash
bun supabase-quota-export.mjs --out-dir ~/atlas/supabase-export
bun test   # offline: the API is an injected fake fetch
```

Aggregator side: `saas-extract.ts` `applySupabaseExport` prefers a document
younger than 30 minutes (the #1151 tick); older or missing falls back to the
quota-view prose exactly as before.
