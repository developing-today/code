# Inventory: person-name identifiers in code, and the role terms to replace them (#13507)

Status: inventory and proposal (scope items 1 and 2 of #13507). No code is renamed here.
Counted against main at 63e741c1, 2026-11-25. Counts exclude Markdown; dated `doc/` records are history and stay as written (out of scope per #13507).

## Summary

The names sit in four places: the PM stack's "needs-drew" surface (aggregator, console, notify-relay, exporters), the shared orchestration schema, the fleet queue's block and person-wait path, and three identity values (the Atlas requester kind, the search client name and the bridge address). Roughly two thirds of the occurrences are plain source identifiers that can be renamed in one pass. The rest are wire or stored values; each of those needs read-old, write-new handling, listed below with the store it lives in.

The bridge already shows the pattern to follow: `pkgs/message-bridge/worker/src/relay.js` treats `drew`, `owner` and `central` as one address. "owner" is therefore already a live role term in the code, and this proposal reuses it where the name means "the person who runs the fleet", and "user" where the existing prose sweep (#13392) used "the user".

## Counts (non-Markdown lines, outside dated docs)

| Form | Lines | Where |
|---|---|---|
| `needs-drew` | 273 | pm-stack/aggregator 172, pkgs/fleet 24, notify-relay 14, console 7, nixos/home 5 |
| `needsDrew` | 277 | aggregator 225, shared-schema 16, pkgs/fleet 16, notify-relay 11 |
| `NeedsDrew` | 269 | aggregator 242, pkgs/fleet 17, console 6, notify-relay 1 |
| `needs_drew` | 66 | pkgs/fleet 45, aggregator 1 |
| `NEEDS_DREW` | 36 | aggregator 22, exporters 7, console 2, notify-relay 1 |
| `drew-cli` | 12 | pkgs/fleet 8, pkgs/atlas-search 2 |
| `drew_decision` | 4 | pkgs/fleet 2 (one in a SQL migration) |
| `awaiting drew` | 2 | pkgs/fleet 1 |
| bare `"drew"` value | ~40 | atlas-core requester kind, message-bridge relay/wake, fleet-kv and bridge test fixtures |

## A. Wire and stored values (rename only with aliases)

Each row: what the value is, where it is stored or sent, and the alias rule the rename needs.

1. **Fleet block payload key `needs_drew`** (`fleet-queue/src/client/commands_lifecycle.rs`). Written into the queue task's `progress` JSON in the fleet database, and sent as `body.needs_drew` on the block request. Rule: write `needs_user`; read both keys, preferring the new one; keep reading `needs_drew` until no open task row carries it.
2. **Alert intake extension `needsDrew`** (`fleet-queue/src/needs_drew.rs` posts it; `pm-stack/aggregator/src/server-alert-routes.ts` validates it, and treats it as mutually exclusive with `stackHealth` and `decisionSet`). The wire contract in `needs_drew.rs` says the aggregator side owns it. Rule: the aggregator accepts `needsUser` and `needsDrew` (one, not both) first; the fleet producer switches to `needsUser` only after that aggregator is deployed.
3. **Alert and incident key `needs-drew:<ledgerId>`** (`needs-drew.ts` incident key, `notifications.ts` objectRef and actedRefs, `notify-relay.ts` prefix match, `needs_drew.rs` key). Stored in `notifications.jsonl` and the incident store, and the throat dedupes on it. Rule: keep matching the old prefix for existing refs; new keys can move to `needs-user:` only together with the prefix match in notify-relay, or the dedupe and the close-on-answer path miss the old ones.
4. **Block reason class `drew_decision`** (`fleet-queue/src/person_wait.rs` `PERSON_WAIT_CLASSES`; `pkgs/fleet/migrations/0021_park_revalidation.sql` view). Stored as `result.reason_class` on task rows. Rule: add `user_decision` to the class list and to the view in a new migration, keep `drew_decision` in both; never edit migration 0021 in place.
5. **Aggregator state file `needs-drew.json` and env `PM_AGG_NEEDS_DREW_FILE`** (`aggregator/src/config.ts`, `needs-drew.ts`; the backup set in `nixos/home/pm-dashboard-backup.sh` and its manifest test). Rule: read the new path if present, else the old one; keep the old env name as a fallback; update the backup manifest to accept either name for one cycle, since backups of the old name already exist.
6. **HTTP routes** `/api/needs-drew.json`, `/api/needs-drew/:ledgerId/answer`, the `#needs-drew-census` anchor (`aggregator/src/server.ts`, `render.ts`, `server-pages.ts`), read by the console (`console/src/work.ts`, tier `"needs-drew"`). Answer links already sent to the phone point at the old path. Rule: serve both paths; switch the console and newly sent links to the new path; keep the old path as an alias.
7. **Atlas requester kind `drew`** (`atlas-core/src/feedback.rs` `REQUESTER_KINDS`, weights in `trace_stats.rs`, normalising in `feedback_normalise.rs`). Stored in feedback and trace records. Rule: accept `drew` on read and normalise it to the new kind; write the new kind. Note `user` is already a distinct kind with the same weight (1.0), so the new term cannot be `user`.
8. **Search client name `drew-cli`** (`fleet-search` and `atlas-search` `server.rs` sensitivity table, `serve_contract.rs`, `smoke.sh`). It names a seat in client.json configs on machines, with the secret-bearing ceiling. Rule: add the new name to the same ceiling, keep `drew-cli` as an alias until configs are moved.
9. **Bridge address `drew`** (`message-bridge/worker/src/relay.js`, `wake.js`). Already aliased with `owner` and `central`. Rule: make the shown default reply address `owner` instead of `drew`; keep `drew` in the accepted set.

10. **Orchestration tier `needsDrew`** (`pm-stack/shared-schema/migrations/20261111_0001_orchestration_state.up.sql` CHECK constraint `tier IN ('needsDrew', 'needsOrchestrator')`, the `tiers` key in the orchestration document, `orchestration-writer.py`, `ORCHESTRATION.md`, which calls a new tier "a contract change, not data"). Stored in `orch_items.tier`. Rule: a new migration that widens the CHECK to accept `needsUser`, the writer and readers accepting both keys, then a data update of stored rows; never edit the applied migration.
11. **Notify-relay edge mirror** (`pm-stack/notify-relay`: `POST /api/needs-drew/:ledgerId/answer`, `payload.needsDrew`, withdraw by `needs-drew:<ledgerId>`). The deployed Worker sees the same values as A2, A3 and A6, so it moves with the aggregator in the same slice, old forms still accepted.

## B. Free-text matchers (keep the old phrases)

`person_wait.rs` `OWNER_WAIT_PHRASES` includes `awaiting drew`, `waiting on drew`, `waiting for drew` and `needs drew`. These match what lanes have written in free text, so removing them would let old wording slip past the rule. Keep them; the role forms (`awaiting owner`, `needs the owner`) are already in the list.

## C. Source-only identifiers (one mechanical rename per package)

No stored or wire value depends on these: the Rust module `fleet_queue::needs_drew` and its types (`NeedsDrewOption`, `BlockNeedsDrew`); the TypeScript files `needs-drew.ts` and `needs-drew-answer.ts`; and the functions and types `NeedsDrewItem`, `NeedsDrewQuestion`, `NeedsDrewCensus`, `buildNeedsDrewCensus`, `extractNeedsDrewReason`, `raiseNeedsDrew`, `validateNeedsDrew`, `createNeedsDrewSweeper`, `NEEDS_DREW_LABELS`, `ESCALATION_NEEDS_DREW_AFTER_MS` and the rest. The console's parity test imports aggregator names (`console/test/needs.test.ts`), so both packages move in the same change. Test fixtures that use `"drew"` as a sample user id (fleet-kv, fleet-bridge, message-bridge tests) are data, not the surface; they can stay or move to a neutral id.

## Proposed role terms

| Old | Proposed | Why |
|---|---|---|
| needs-drew / needsDrew / NeedsDrew / needs_drew / NEEDS_DREW | needs-user / needsUser / NeedsUser / needs_user / NEEDS_USER | Matches the #13392 prose term "needs user" and the existing labels `user/review`, `user/escalated` that the census reads. |
| `drew_decision` | `user_decision` | Sits beside `escalated-to-user`. |
| `awaiting drew` (text) | keep; `awaiting owner` already matches | Free-text matcher, see B. |
| Atlas kind `drew` | `owner` | `user` is taken by another kind; "owner" is the bridge's existing alias. |
| `drew-cli` | `owner-cli` | Same reasoning as the Atlas kind. |
| bridge address `drew` | `owner` (already accepted) | Only the default shown address changes. |

## Suggested order of slices

1. Aggregator and notify-relay: accept the new alert extension, routes, mirror payload and state-file name alongside the old ones (A2, A3 read side, A5, A6, A11), and rename their source identifiers (C), with tests for old-value reads.
2. Shared schema: migration widening the orchestration tier check, writer and readers dual-keyed (A10).
3. Fleet queue: new migration adding `user_decision`; read both payload keys; rename the module and types; switch the producer to `needsUser` once slice 1 is deployed (A1, A2 write side, A4).
4. Atlas and search: new requester kind and client name with aliases (A7, A8).
5. Bridge default address (A9).
6. Books and living references that quote the old identifiers, as #13507 says, after the code lands.
