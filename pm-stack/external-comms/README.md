# External-comms ledger

The send-time ledger for Drew's ratified rule **EXTERNAL COMMS ARE
NEVER SILENT** (2026-11-05; rule text: `~/AGENTS.md`,
`~/memory/topics/external-comms-rule-2026-11-05.md`; origin: the
external-comms audit of 2026-11-05).

Every send outside our own repos/channels under Drew's identity or a
fleet identity — third-party email, issues/PRs/comments/reviews on
repos we don't own, public posts — is recorded here at send time,
with its approval basis: (a) per-item approval, (b) surfaced-with-link
(the narrow #176 shape), or (c) a written allowlist entry.

## Where things live

| Piece | Location |
|---|---|
| Table | `agent_external_comms` in the Turso database **fleet-platform** (repointed 2026-11-07; before that `agent.external_comms` in Supabase project muse-dev2 — the 58 pre-repoint rows stay in Postgres until the finalize top-up) |
| Migration | `migration-0001-agent-external-comms.sql` (original Postgres DDL) + `migration-0002-agent-external-comms-turso.sql` (Turso DDL, executed 2026-11-07) |
| Writer CLI + pusher | `record-external-send` (this directory; deployed from the primary clone `~/git/code` on the laptop) |
| Outbox (spill) | `~/.local/state/external-comms/outbox.jsonl` on the laptop |
| Pusher health | `~/.local/state/external-comms/pusher-health.json` on the laptop |
| Pusher supervision | systemd **user** units `external-comms-pusher.service` / `.timer` (this directory; installed to `~/.config/systemd/user/`, timer every minute) |
| Backfill | `backfill-2026-11-05.jsonl` — the 68 sends enumerated by the 2026-11-05 audit, loaded once via `record-many` |

## Write pattern: database-first, spill to outbox, pusher drains

Drew's direction (2026-11-05): *"use database spill to outbox or use
outbox always ensure pusher. all on laptop."*

Chosen: **database-first with spill to outbox** — the proven pm-stack
pattern (the aggregator's `events.ts` retry outbox: write to the store
first; on failure append to a durable JSONL outbox; flush merged by
stable id on the next tick). It fits this ledger because the direct
path lands most records instantly (audits and dashboards read the DB,
not files), while no record can be lost to a DB/network outage at
send time:

1. `record-external-send …` builds a record with a stable id and
   tries a direct libSQL pipeline insert (INSERT OR IGNORE on the
   id) into `agent_external_comms` in Turso fleet-platform
   (DB token resolved **by name** through the secrets broker:
   `turso.fleet-platform-rw-token`; values never touch disk or logs).
2. On any failure it appends the record to the local outbox
   (fsync'd) and still exits 0 — recording never blocks the send it
   logs. Exit is non-zero only when *both* paths fail.
3. The **pusher** (`record-external-send push`, timer every minute)
   drains the outbox: insert with INSERT OR IGNORE on
   the id PK (safe retry after ambiguous failures), drop landed
   lines, keep failures with `_attempts` / `_last_error`. Every run
   rewrites `pusher-health.json` (`last_run_at`, `last_drain_ok_at`,
   `pending_count`, `last_error`, `total_landed`) — a stuck pusher is
   a stale health file plus a non-zero pending count, readable by the
   stewardship visibility pass.

Retention follows the #1258 durability tiers: the database is the
record; the laptop outbox is local spill only; the laptop state dir
rides the laptop's existing backup story.

## Recording contract

```sh
record-external-send \
  --identity drew|fleet:<name> \
  --channel github|gmail|slack|facebook|... \
  --destination <repo | addressee | channel> \
  --kind issue|pr|comment|review|email|post|dm \
  --approval-basis "per-item approval" | "surfaced-with-link" | "allowlist:<name>" \
  --summary "one line: what was sent" \
  [--item-ref <url>] [--approver <who>] [--approval-evidence <ref>] \
  [--sent-at <iso8601>] [--payload '<json>']
# -> {"id": "…", "landed": "db"|"outbox"}

record-external-send status   # health + outbox depth + DB row count
record-external-send push     # manual drain (normally the timer's job)
```

Email approvals are recorded explicitly: `--approver` names who
approved and `--approval-evidence` points at where (chat line, memory
entry, issue comment) — the gap the Oct 19 Crunchyroll 06:19 send
exposed.

## Install (laptop)

```sh
cp external-comms-pusher.{service,timer} ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now external-comms-pusher.timer
```

The units execute the script from the deployed primary clone
(`%h/git/code/pm-stack/external-comms/record-external-send`), so a
merge to main + pm-git-sync is the deploy.
