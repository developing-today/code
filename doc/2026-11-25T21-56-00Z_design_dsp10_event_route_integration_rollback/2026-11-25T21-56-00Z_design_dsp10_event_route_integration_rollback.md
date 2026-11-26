# DSP-10 — Deployable source integration, observability and rollback

Issue: [#9450](https://github.com/developing-today/code/issues/9450)
(slice `DSP-10` of [#9203](https://github.com/developing-today/code/issues/9203),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **design proposal — not an authorization to implement.**
This record elaborates the DSP-10 row of the merged design package
(`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`,
PR #9225). The row asks to "wire supported entrypoints and dashboards
for operator inspection with kill switch and retention/retry policy",
and its oracle says: a disabled route emits zero effects; a stage
smoke with a signed test event and a fake provider reports shared
`Result` outcomes and a receipt with secret and prompt canaries
absent; rollback disables apply and preserves receipts. It never
says where the route enters pm-dispatch, what the switch is, what an
operator can see, or what is retried. This record _decides_ those
things against the service as it runs today. It authorizes no code,
deployment, schedule, production launch or live acceptance; those
remain separately gated stages (see §9).

Governing decisions consumed, not re-opened:

- **Component #9203 §3.2 and §3.7**: the processing order from raw
  body to notify; "no UI/webhook result acknowledges command
  acceptance until the Fleet request and idempotency record are
  durable"; a provider timeout after a remote effect is recorded as
  ambiguous and reconciled, never blindly retried; a producer on the
  wrong store fails closed.
- **DSP-01** (row and `work/operations.rs`): the projector's existing
  `POST /github/webhook` and HMAC helper are the only ingress; an
  in-process forwarder sends the event to the Fleet operation
  interface; projector delivery dedupe is never command acceptance.
- **DSP-04, DSP-05, DSP-06, DSP-07, DSP-08 records**: routing table,
  send and binding, reservation and launch, launch receipt and
  validator, policy and reauthorization checkpoints.
- **DSP-09 record**: no arbitrary retry caps; no blind compensation.
- **pm-dispatch as it runs** (`pm-stack/dispatch/README.md`): a user
  service on the laptop (`pm-dispatch.service`, NixOS pm-stack
  module), port 4879 on loopback and tailnet, public through the
  `amd-6-pm-stack` tunnel at `dispatch.security.cab`; env from
  `~/pm-stack/dispatch/.env` and `~/secrets/dispatch.env`, read at
  start; `/health` reports the deployed build version; deploy is a
  pull plus a NixOS switch, or `install.sh` as the fallback. #9028 is
  closed: dispatch reaches the queue only through the broker.

## 1. Intent

The event route can be turned off in one step, without a restart and
without a deploy, and when it is off nothing it would have done
happens and nothing it already recorded is lost. When it is on, an
operator can see every operation that is not finished, why, and the
next action, from the services that already exist. Merged, deployed
and live-verified are reported as separate facts.

## 2. Ownership

| Concern                                | Owner                                                 | DSP-10 role                                          |
| -------------------------------------- | ----------------------------------------------------- | ---------------------------------------------------- |
| Rollback and kill-switch tests         | `pm-stack/dispatch/test/event-route-rollback.test.ts` | writes                                               |
| Operator documentation and runbook     | `pm-stack/dispatch/README.md`                         | writes                                               |
| Route entry, mode read, health fields  | `pm-stack/dispatch/src/server.ts`, `src/config.ts`    | specifies; implemented by the service owner's change |
| Webhook ingress and forwarder          | `pm-stack/projector` (DSP-01)                         | consumes; adds no second receiver                    |
| Acceptance, routing, effects, receipts | DSP-01, DSP-04 to DSP-08, REL-03/REL-04               | consumes                                             |
| Operator panel                         | `pm-stack/aggregator` (existing pages)                | reads pm-dispatch; adds no service                   |
| Schedules                              | Fleet schedule registry                               | none introduced; registry unchanged                  |

## 3. Where the route enters

```text
GitHub ──► projector POST /github/webhook (HMAC, bounds, delivery dedupe)
              │ in-process forwarder (DSP-01)
              ▼
         Fleet operation acceptance (queue broker; REL-03 key)
              │
              ▼
         pm-dispatch POST /v1/operations/route   ◄── also: PM Stack actions, fleet CLI
              │ mode check (§4) → DSP-04 decision → DSP-08 checks
              ├─► send_to_session  (DSP-05)
              ├─► direct_start / resume (DSP-06, existing launch path)
              └─► status / triage / blocked_answer / reject (no external effect)
```

- **One new route on the existing service**, `POST /v1/operations/route`,
  taking an accepted operation id, with the same auth as today
  (bearer or GitHub OIDC). No new service, port, unit or tunnel
  mapping.
- **`GET /v1/operations/:id`** returns the operation's launch receipt
  (DSP-07 wrapper), the decision and the policy entries.
- **`GET /v1/operations?state=…`** lists operations by state for the
  operator (§5).
- `POST /v1/sessions` and `POST /v1/tasks` keep their contracts. A
  later change may move them onto the same path; this slice does not.

## 4. The kill switch

**Mode.** One value, `event_route_mode`: `off`, `shadow` or `apply`.

| Mode     | Effect                                                                                                                                                                                        |
| -------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `off`    | the route answers the shared `Result` `pending_sync`, reason `route_disabled`, `next_action` naming the mode; **zero external effects**; the accepted operation and its retry intent are kept |
| `shadow` | the DSP-04 decision and DSP-08 checks run and are written into the receipt as `would_route`; zero external effects                                                                            |
| `apply`  | effects run as DSP-05 and DSP-06 specify                                                                                                                                                      |

**Where it is read.** From a mode file beside the env files
(`~/pm-stack/dispatch/event-route-mode`), read on every request and
again at every DSP-08 checkpoint before an external effect. Env is
read only at start, so a switch held in env could not stop a running
service without a restart; a file read per request can. The file is
written by hand or by the runbook command; nothing else writes it.

**Fail closed.** A missing, unreadable or unrecognized file is `off`.
Two further narrowings, both default-empty: a per-verb list
(`event_route_verbs`) and the existing `DISPATCH_REPO_ALLOWLIST`. A
verb or repository not listed is `off` for that request. The most
restrictive value wins; nothing widens another.

**In flight.** Because the mode is re-read at each checkpoint, an
operation in `apply` that has not yet taken its next external effect
stops there when the mode drops to `off` or `shadow`, with
`pending_sync`, reason `route_disabled`, and every effect already
taken listed (DSP-09 I9). Nothing is undone because the switch moved.

**Shadow first.** The first deploy ships with the file absent (`off`).
`shadow` comes next and runs until the shadow receipts validate under
the DSP-07 validator with no violation; `apply` is a separate,
recorded owner step per verb (§9).

## 5. Operator inspection

From the services that exist, no new dashboard service:

| Where                                                       | What it shows                                                                                                                               |
| ----------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| `GET /health` (pm-dispatch)                                 | build version (as today) plus `event_route: {mode, mode_source, verbs, policy_version, counts}`; `counts` are outcomes since start by value |
| `GET /v1/operations?state=uncertain\|partial\|pending_sync` | each unfinished operation: work, stage reached, reason, next action, age                                                                    |
| `GET /v1/operations/:id`                                    | the whole receipt                                                                                                                           |
| journald, `pm-dispatch.service`                             | one structured line per decision and per effect: operation id, row, stage, outcome, reason; never a prompt, token or password               |
| pm-aggregator existing pages                                | a panel that reads the two pm-dispatch reads above through the aggregator's existing access; read only                                      |

The panel and the list never offer a "retry" button that mints a new
key: the only actions offered are the operation's own `next_action`
(for example a reconcile read), run under the same operation.

## 6. Retention and retry

- **Retention.** Operation records and receipts live where REL-03 and
  REL-04 put them and follow their retention; this slice deletes
  nothing and adds no store. Logs follow journald's existing
  retention. `/health` counts reset on restart and are labelled
  "since start".
- **Retry at the forwarder.** A retryable failure between the
  projector forwarder and acceptance (DSP-01) is retried with backoff
  under the **same** operation key until the operation's own deadline
  (the `do-now` directive's expiry when there is one, DSP-08 §4). No
  arbitrary attempt cap is introduced (DSP-09 §4.3); the deadline
  bounds it.
- **No retry at the effect.** pm-dispatch never re-runs a launch or a
  send with a new key. An `unknown` stage is listed as `uncertain`
  with its reconcile read; the reconcile is the retry.
- **Off is not failure.** An operation refused by `off` keeps its
  retry intent: when the mode returns to `apply` before its deadline,
  the same operation is routed once, from the start of the table,
  with fresh reads.

## 7. Stage smoke

A smoke that runs in CI and on a staged checkout, never against
production services:

1. Start pm-dispatch from the checkout with a fake OpenCode
   (accepts sessions, records prompts, returns marked checkpoints on
   request) and a fake broker (claim, `/bind`, timeline), both
   in-process doubles from DSP-09.
2. Sign a test `issue_comment` event with a test-only HMAC secret and
   send it to the projector's webhook handler with the forwarder
   pointed at the fake broker.
3. Run each mode: `off` and `shadow` must record zero calls on both
   fakes; `apply` must produce a receipt that validates under the
   DSP-07 validator.
4. Two canaries, a fake secret in the service env and a unique string
   in the prompt, must be absent from every response body, receipt,
   `/health` payload and captured log line.
5. Every response carries a shared `Result` outcome from the eight;
   any other value fails the smoke.

## 8. Rollback

| Step                        | How                                                                         | Keeps                                                                                                                     |
| --------------------------- | --------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------- |
| Stop effects now            | write `off` to the mode file, or delete it                                  | accepted operations, receipts, retry intent                                                                               |
| Stop one verb or repository | remove it from `event_route_verbs` or the allowlist                         | the same                                                                                                                  |
| Remove the code             | revert the merge, then the usual deploy (pull plus switch, or `install.sh`) | records written by the reverted code stay readable by `GET /v1/operations/:id` until REL-04's own rollback says otherwise |
| Verify                      | `/health` shows the expected build version and `mode: off`                  | —                                                                                                                         |

No rollback step deletes a receipt, cancels a task, deletes a session
or edits a GitHub comment. Effects already taken are reported, not
reversed.

## 9. Status is reported in separate facts

The README section for the event route keeps one line per fact, each
with its evidence: **designed** (this record), **implemented** (merged
code and its tests), **merged**, **deployed** (`/health` build version
on the laptop equals the merge commit), **mode** (`off`, `shadow`,
`apply` per verb, from `/health`), **live-verified** (a separately
authorized run, for example the #6533 fixture, with its receipt),
**blocked** and **pending**. A merge never implies deployed; deployed
never implies `apply`; `apply` never implies live-verified (H-R048).

## 10. Acceptance and gates

Persisted fields: none. The mode file is configuration, not state.

Tests, all in `pm-stack/dispatch/test/event-route-rollback.test.ts`
with in-process doubles (first red is the slice's starting test):

| Test                                                 | Proves                                                                                   |
| ---------------------------------------------------- | ---------------------------------------------------------------------------------------- |
| `pmo_dsp_10_disabled_route_has_zero_effects` (first) | `off`: zero calls on the fake provider and fake broker; `pending_sync`, `route_disabled` |
| `pmo_dsp_10_missing_or_bad_mode_file_is_off`         | absent, unreadable or unknown value behaves as `off`                                     |
| `pmo_dsp_10_unlisted_verb_or_repo_is_off`            | narrowing lists; most restrictive wins                                                   |
| `pmo_dsp_10_shadow_records_decision_without_effects` | `would_route` in the receipt; zero calls                                                 |
| `pmo_dsp_10_mode_drop_stops_before_next_effect`      | an in-flight `apply` stops at the next checkpoint and lists effects taken                |
| `pmo_dsp_10_rollback_preserves_receipts`             | after `off` and after a simulated revert, earlier receipts still read back               |
| `pmo_dsp_10_off_keeps_retry_intent`                  | back to `apply` before the deadline routes the same operation once                       |
| `pmo_dsp_10_forwarder_retry_keeps_operation_key`     | retries never mint a new key                                                             |
| `pmo_dsp_10_signed_smoke_reports_shared_outcomes`    | §7 steps 2, 3 and 5                                                                      |
| `pmo_dsp_10_canaries_absent`                         | §7 step 4                                                                                |
| `pmo_dsp_10_health_reports_mode_and_build`           | `/health` fields                                                                         |

Command: `cd pm-stack/dispatch && bun test test/event-route-rollback.test.ts`;
first show that the named tests are listed (a zero-test run is not a
pass). Runs on GitHub-hosted runners through the existing pm-stack
test workflow.

Gates:

- **Hard blockers:** DSP-01, CLI-02, REL-03, DSP-04, DSP-05, DSP-06,
  DSP-07. Records for DSP-04 to DSP-07 and REL-03 are on main; DSP-01
  has code on main.
- **Runtime gate:** #9196's safety gates for the program.
- **Stages each needing their own evidence and approval:** the
  service owner's change and deploy; moving any verb from `off` to
  `shadow`, and from `shadow` to `apply`; and the live #6533 run.
- **No schedule** is introduced, so the Fleet schedule registry is
  unchanged.

## 11. Requirement trace (design-level)

| ID     | Outcome (crosswalk, abridged)                                                                       | Where       |
| ------ | --------------------------------------------------------------------------------------------------- | ----------- |
| C-R160 | Acceptance covers claims, leases, external comments and failure cases                               | §7, §10     |
| C-R162 | Architecture map, gaps, decisions, migration plan, go/no-go gates                                   | §3, §4, §10 |
| H-R046 | Acceptance and migration/rollback tests in the design handoff                                       | §8, §10     |
| H-R048 | Report designed, implemented, merged, deployed, live-verified, blocked and pending separately       | §9          |
| O-R065 | Dedupe events, bind and report receipts; on failure preserve retry intent; never silently downgrade | §4, §6      |

## 12. Non-goals

- No second webhook receiver, service, port, unit or tunnel mapping.
- No new store and no deletion of any record.
- No schedule and no change to the Fleet schedule registry.
- No production launch and no live acceptance from this slice.
- No change to the contracts of `POST /v1/sessions` or `POST /v1/tasks`.
