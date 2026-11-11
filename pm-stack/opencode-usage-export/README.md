# opencode-usage-export — OpenCode usage producer (#2072 slice 1)

The OpenCode half of the observability build (issue #2072, owner
direction 2026-10-29): a read-only **pull** exporter over OpenCode's
own history database, the same pull pattern as the Muse runtime-db
export. No push instrumentation is added to OpenCode/OpenChamber.
Slices 2–7 of #2072 (webhooks, views, rollups, projections) are
separate follow-ups; this is only the exporter.

```
~/atlas/opencode-usage-export/latest.json    atomic latest document
~/atlas/opencode-usage-export/history.jsonl  one line per tick, success or failure
~/atlas/opencode-usage-export/state.json     { storePath, lastTickAt } — move detection memory
```

## The live store — read this before touching the path

Verified 2026-11-02 (queue task 6610) against the running managed
instance on Drew's laptop:

- The live store file is **`~/.local/share/opencode/opencode.db`**
  (the prod `opencode serve --service` runs with
  `XDG_DATA_HOME=~/.local/share`).
- The live **tables** are `session_v2` (4,489 sessions at probe time,
  newest `time_updated` minutes old) and `session_message`
  (225,723 rows). Per-session cost/token totals are columns on
  `session_v2`; per-message model/cost/tokens are in
  `session_message.data` for `type = 'assistant'` rows
  (`model: { id, providerID }`).
- The legacy `session` / `message` / `part` tables **in the same
  file** froze on **2026-10-06** — that frozen legacy schema is the
  "stale default-path opencode.db" named in the #2072 direction. The
  file is live; the old tables are not. This exporter reads only the
  v2 tables, so it can never mistake the frozen legacy rows for data.

## Move detection

Candidates are probed in order (an explicit `--db` /
`OPENCODE_USAGE_DB` wins outright):

1. `~/.local/share/opencode/opencode.db`
2. `~/.local/share/openchamber/opencode.db`
3. `~/.local/state/opencode/opencode.db`

A candidate is **live** iff it has the v2 tables and its newest
`session_v2.time_updated` is within `--fresh-hours` (default 24). The
first live candidate wins. If the winner differs from the store path
recorded in `state.json`, the document carries
`"moved": { "from": ..., "to": ... }` — the move is surfaced on the
tick it happens. If **no** candidate is live, the tick writes a
failure document and exits non-zero; there is no silent fallback to a
stale store.

## Document

```json
{
  "service": "opencode",
  "source": "file-export/opencode-usage-laptop",
  "status": "ok",
  "measuredAt": "2026-11-02T18:00:00Z",
  "storePath": "/home/user/.local/share/opencode/opencode.db",
  "storeNewestAt": "2026-11-02T17:59:41Z",
  "windowHours": 24,
  "moved": null,
  "sessions": [{ "id": "ses_...", "parentId": null, "agent": "build", "directory": "/home/user/git/code", "model": { "id": "workhorse", "providerID": "opencode-route" }, "cost": 0.0, "tokens": { "input": 5937987, "output": 15558, "reasoning": 30432, "cacheRead": 1542479, "cacheWrite": 0 }, "messages": { "total": 89, "assistant": 85, "byModel": [{ "model": "workhorse", "providerID": "opencode-route", "count": 85, "cost": 0.0, "tokens": { "...": "..." } }] }, "timeCreated": "...", "timeUpdated": "..." }],
  "totals": { "sessions": 0, "childSessions": 0, "cost": 0.0, "tokens": { "...": "..." } }
}
```

Sessions are those with `time_updated` inside the trailing
`--window-hours` (default 24). Session rows carry parentage
(`parentId`), so child/subagent sessions attribute to their parent.

Failure documents carry `status` + `attemptedAt` + `detail` and **no**
sessions — a failed tick can never look like a reading:

| Status          | Exit | Meaning                                                                                                     |
| --------------- | ---- | ----------------------------------------------------------------------------------------------------------- |
| `store-missing` | 3    | no candidate DB file exists                                                                                 |
| `store-stale`   | 4    | candidates exist, none fresh — the store moved somewhere unknown, or OpenCode has been down past the window |
| `schema-drift`  | 5    | a candidate DB lacks the v2 tables                                                                          |
| `read-failed`   | 6    | any other read error                                                                                        |

## Run

```bash
node opencode-usage-export.mjs --out-dir ~/atlas/opencode-usage-export
node --test pm-stack/opencode-usage-export/opencode-usage-export.test.mjs   # offline: fixture SQLite stores
./install-laptop.sh                            # on the laptop: install + enable the 15-min timer
```

Reads use `node:sqlite` (Node ≥ 22) opened read-only with
`query_only` on; the exporter never writes to the store.
