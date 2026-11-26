# MOB-07 — Session capability panel and session action receipts

Issue: #9474 (slice MOB-07 of #9201). Status: design record. It is not
an authorization to implement. It covers item operations 9 to 13 of
#9201 §3.3: request triage, request an agent, inspect current session
status, send a live-session instruction, and request a status report.

## 0. Governing decisions consumed

| Source                                          | What this record takes from it                                                                                                                                           |     |
| ----------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | --- |
| #9201 §3.2, §3.3, §3.5                          | The browser submits intents only; operations 9–13; "claimed item lacks verified session" shows `claimed / binding unknown`; merge, close and stop are not one-tap.       |     |
| #9201 MOB-07 slice text                         | Owned files; red test `session_action_requires_verified_task_binding`; oracle: per-adapter capability contract tests and a state test over fake adapter receipts.        |     |
| DSP-05 record (#9505)                           | `BindingView`, `SessionCapability`, the delivery evidence ladder, `DeliveryState`, the three effects `session_binding` / `message_delivery` / `session_ack`, the matrix. |     |
| DSP-06 record (#9506)                           | Direct dispatch reserves through the queue claim/bind protocol under one operation key; enqueue and direct launch are different operations.                              |     |
| MOB-05 (`src/work-action-contract.ts`, on main) | The envelope, `operationKey`, preview → accept → apply, replay returns the first receipt.                                                                                |     |
| MOB-09 (`src/work-operation-reconciliation.ts`) | Verified only from a source readback; `applyIfMutationsEnabled`.                                                                                                         |     |
| MOB-01 record D5                                | `claim` and `session` facts; `session` is `none` or `bound{verified, capabilities}`; both `absent/not_sourced` until real readers exist.                                 |     |
| MOB-03 record D7                                | The detail page's `capability-panel` region and its keyboard stop.                                                                                                       |     |
| #9075 (closed), #6533 (closed)                  | Session id in env and worktree pwd-file; #6533 stays the live acceptance fixture. MOB-07 files no duplicate live-proof issue.                                            |     |

## 1. Verified state on main `26fcce1d`

- `src/work-action-contract.ts` exports `createWorkAction`,
  `previewWorkAction`, `acceptWorkAction`, `applyWorkAction`,
  `receiptFor`, with `WorkActionRequest {operationKey, kind,
objectRef, action, payload, expectedWorkVersion}`.
- `src/work-operation-reconciliation.ts` exports `reconcileWorkAction`
  and `applyIfMutationsEnabled` (#15321).
- The aggregator has no session panel, no session send, and no
  capability model. `src/session-panel.ts` and the two owned tests do
  not exist.
- DSP-05's capability matrix (its §6): OpenCode find/status
  supported, send exists but its behaviour on a busy session is
  unverified, `delivered` and `acknowledged` unverified; the Fleet lane
  has no inbound channel; Slack and Muse unsupported; GitHub comment is
  not a session send. None of it is exercised against a live provider.
- DSP-05 and DSP-06 are records only; their broker code is not on
  main.

## 2. Intent

The panel shows, for one work item, what can be asked of its session
and why anything cannot, and then shows each request's progress as
separate proven stages. It never sends to a session that is not
verified as bound to this task, never shows a stage that has no
evidence, and never calls a session from the browser.

## 3. Ownership

- **Owned:** new `src/session-panel.ts`, new
  `test/session-capabilities.test.ts`, new
  `test/session-action-receipts.test.ts`, and their fixtures under
  `test/fixtures/session-panel/`.
- **Needed edit outside the owned list, named:** the render of the
  panel inside MOB-03's `capability-panel` region in `src/render.ts`.
- **Read-only:** `work-action-contract.ts`,
  `work-operation-reconciliation.ts`, `work-projection.ts`.
- **Not touched:** the broker, the OpenCode client, the dispatch app,
  `actions.ts`, any provider integration.
- **Added nowhere:** no database, queue, store, scheduler, provider
  client, Slack or Muse integration, or browser-side network call to
  a session or provider.

## 4. Decisions

### D1. Two pure functions

```ts
buildSessionPanel(input: SessionPanelInput): SessionPanel
sessionReceiptView(envelope: WorkActionEnvelope, effects: SessionEffects | null): SessionReceiptView
```

```ts
interface SessionPanelInput {
  view: PmWorkView; // MOB-01
  binding: SourceRead<BindingView>; // DSP-05 §3.1, read server-side
  capabilities: SessionCapability[]; // DSP-05 §3.2, static + live probe
  actor: WorkActionActor; // server-resolved; never from the client
  draft_id: string; // server-minted per panel render
  now: string;
}
interface PanelOperation {
  op: "request_triage" | "request_agent_enqueue" | "request_agent_direct" | "session_status" | "session_send" | "status_report";
  enabled: boolean;
  reason_code: string | null; // why disabled; null when enabled
  adapter: string | null;
  ceiling: "none" | "adapter_accepted" | "delivered" | "acknowledged" | "progressing";
  confirm: null | { target: string; effect: string }; // consequential ops name target and effect
  operation_key: string | null; // D4; null when disabled
}
interface SessionPanel {
  work: { authority_id: string; work_id: string };
  binding: "verified" | "claimed_binding_unknown" | "none" | "unavailable";
  session: null | { ref: string; provider: string; status: "active" | "idle" | "gone" | "unknown"; observed_at: string };
  operations: PanelOperation[]; // fixed order, all six always listed
}
```

No I/O, no clock beyond `now`, no module state. The server reads the
binding and capabilities (DSP-05 §4) and passes them in.

### D2. The binding rule

A session is targetable only when **all** hold:

1. `binding` is `ok` and its `binding_ref` is non-null;
2. the MOB-01 `session` fact is `present` and `bound{verified:true}`;
3. the session ref in (1) equals the one in (2);
4. `binding.task_status` is not terminal and the lease is not past
   `now` by the store clock DSP-05 names.

Otherwise:

| Situation                            | `binding`                 | `session_send`, `status_report` reason |
| ------------------------------------ | ------------------------- | -------------------------------------- |
| claim present, no verified binding   | `claimed_binding_unknown` | `session_binding_unverified`           |
| no claim, no binding                 | `none`                    | `no_bound_session`                     |
| binding read failed or `not_sourced` | `unavailable`             | `binding_unavailable`                  |
| (1) and (2) name different sessions  | `claimed_binding_unknown` | `session_mismatch`                     |
| session status `gone`                | `none`                    | `session_gone`                         |

A shell or session with no task binding is never shown as "current
owner" and is never a target. This is the red test.

### D3. Operations 9–13, each gated separately

| #   | `op`                    | Enabled when                                                                                                    | Ceiling                        | Confirm                                                  |
| --- | ----------------------- | --------------------------------------------------------------------------------------------------------------- | ------------------------------ | -------------------------------------------------------- |
| 9   | `request_triage`        | actor holds the triage role; work view is not `absent`; no session needed                                       | `none` (a Fleet queue request) | no                                                       |
| 10  | `request_agent_enqueue` | actor holds the dispatch role; `eligibility` present and `eligible`                                             | `none`                         | no                                                       |
| 10  | `request_agent_direct`  | as above, plus `workflow_state` not `parked` unless the actor confirms "start now"; no verified binding already | `none`                         | yes, when parked: names the item and "start now"         |
| 11  | `session_status`        | D2 targetable, or a claim exists (status read is safe); capability `status = supported`                         | n/a (a read)                   | no                                                       |
| 12  | `session_send`          | D2 targetable; capability `send = supported` with non-empty `verified_by`; actor holds the session role         | the adapter's `max_evidence`   | yes: names the session and that it is a live instruction |
| 13  | `status_report`         | same as 12                                                                                                      | the adapter's `max_evidence`   | no                                                       |

- Enqueue and direct dispatch are two listed operations, never one
  control with a hidden mode, per #9201 §3.3 item 10.
- `unverified` capability is disabled with `capability_unverified`, so
  the UI says "not proven yet", not "never possible".
- A stale or error facet that an operation depends on disables it with
  `facet_stale` / `facet_error` through MOB-01's
  `mutation_blockers`.
- `stop`, `resume`, merge and close are not on the panel. They are
  higher-risk and stay outside this slice.
- An actor without the role sees the operation disabled with
  `role_required` and no private detail about who holds it.

### D4. One operation key per intent

`operation_key = "mob07:" + work_id + ":" + op + ":" + draft_id`,
computed in `buildSessionPanel` and carried in the form. `draft_id` is
minted by the server once per panel render. Two clicks submit the same
key, and MOB-05's `claimOperation` returns the first receipt for the
second. A fresh render mints a new draft, so a deliberate second send
is a new key. The key never includes a client clock or a random value
minted in the browser.

### D5. Receipt stages are separate facts

`sessionReceiptView` turns an envelope plus DSP-05's three effects into
six display stages. Each stage is set only from its own evidence and is
never inferred from a later one:

| Stage                   | Set from                                                                 |
| ----------------------- | ------------------------------------------------------------------------ |
| `accepted_by_fleet`     | envelope state reached `accepted`                                        |
| `queued_to_adapter`     | `message_delivery.observed` ≥ `queued` (DSP-05 `reserved`)               |
| `delivered`             | `message_delivery.verified` ≥ `delivered`, readback only (MOB-09 rule)   |
| `acknowledged`          | `session_ack.verified = acknowledged` from an authenticated signal       |
| `progressing`           | provider activity after `delivered`, shown as its own line, never as ack |
| `task_binding_verified` | `session_binding.verified` equals `session_binding.requested`            |

Rules:

- A Slack (or any transport-only) adapter's ceiling is `delivered`.
  `acknowledged` is never shown for it, whatever the effect values say;
  a value above the ceiling is reported as `effect_above_ceiling` and
  ignored.
- `unknown` delivery renders as "Not proven either way" with the
  envelope's `uncertain` state and its reconcile next action. It is
  never rendered as "Not sent".
- `verified_complete` on a send means the send reached the level asked
  for. The panel says so and never marks the work item done.
- Stage order on screen is fixed; a missing stage shows "not yet" or
  "not supported by this adapter", by ceiling.

### D6. Where state comes from

All panel inputs are read on the server: the work view (MOB-01), the
binding and capability (DSP-05 broker read), and the envelope and
effects (MOB-05 and MOB-09). The browser posts only the operation key,
the op and the draft's free text to the existing actions route. No
browser script calls the broker, OpenCode, Slack or any provider.

### D7. Until DSP-05 and DSP-06 land

With no broker binding read and no capability probe available, the
server passes `binding: {ok:false, error_code:"not_sourced"}` and the
static capability constants only. The panel then shows
`binding: "unavailable"`, operations 11–13 disabled with
`binding_unavailable`, and operations 9 and 10 still listed and gated
by role and eligibility. Applying any of them stays behind MOB-09's
mutation switch.

## 5. Negative cases

| Case                  | Expected                                                                                      |
| --------------------- | --------------------------------------------------------------------------------------------- |
| stale input           | stale `session` or `claim` facet → 11–13 disabled with `facet_stale`                          |
| duplicate / replay    | two submits with one key → one effect, second returns the first receipt (D4)                  |
| ambiguous resource    | binding and session fact disagree → `session_mismatch` (D2)                                   |
| unauthorized actor    | missing role → `role_required`, no effect, no holder named (D3)                               |
| partial remote effect | delivered but ack timed out → envelope `partial`, `delivered` shown, `acknowledged` "not yet" |
| unknown version       | unknown `schema_version` of a capability record → adapter treated as `unverified`             |
| no false verified     | `adapter_accepted` without readback never shows `delivered`; Slack never shows `acknowledged` |

## 6. Oracles and tests

`test/session-action-receipts.test.ts`:

1. `session_action_requires_verified_task_binding` (red test). Each row
   of the D2 table disables 12 and 13 with its reason code; only the
   fully matching case enables them.
2. `session_action_two_clicks_one_operation_key`. Two submits of one
   draft produce one fake-adapter call and the same receipt.
3. `session_receipt_stages_never_infer_upward`. A fake adapter that
   reports only `adapter_accepted` yields `queued_to_adapter` set and
   `delivered` unset; one that reports `progressing` without ack leaves
   `acknowledged` unset.
4. `session_receipt_slack_delivered_is_not_ack`. A transport adapter
   with an `acknowledged` effect value shows `effect_above_ceiling` and
   no ack.
5. `session_receipt_unknown_is_not_not_sent`.
6. `session_receipt_verified_send_does_not_complete_work`.
7. `session_receipt_binding_changed_after_send_is_conflict`. A post-send
   binding re-read that differs leaves `task_binding_verified` unset
   and the envelope in `conflict`.

`test/session-capabilities.test.ts`:

1. `session_capability_matrix_matches_dsp05`. One case per adapter row
   of DSP-05 §6 (OpenCode, Fleet lane, Slack, Muse, GitHub comment)
   asserts each operation's enabled state and ceiling.
2. `session_capability_unverified_is_disabled_with_reason`.
3. `session_capability_requires_verified_by`. `supported` with an
   empty `verified_by` is treated as unverified.
4. `session_panel_lists_enqueue_and_direct_separately`.
5. `session_panel_direct_dispatch_while_parked_requires_confirm`.
6. `session_panel_stop_and_merge_are_absent`.
7. `session_panel_not_sourced_binding_disables_session_ops`. D7.

The fake adapter is a fixture object returning scripted DSP-05
effects. No test calls a network. Command:
`cd pm-stack/aggregator && bun test test/session-capabilities.test.ts
test/session-action-receipts.test.ts`, run remotely through
`pm-aggregator-tests.yml`. Tests are listed by name first; a filter
that matches zero tests fails. Live acceptance stays with #6533.

## 7. Compatibility and reversal

Additive. No existing page, action verb or store changes. Removing the
module and the render call removes the panel, and MOB-03's region shows
its "not connected yet" text again.

## 8. Requirement trace

Program requirements H-SURVIVE-A, C-7, C-8 and O §§13.2–14.5, as the
slice text names them:

| Requirement  | How this record serves it                                                                                 |
| ------------ | --------------------------------------------------------------------------------------------------------- |
| C-7          | Intents only, through MOB-05's envelope; no browser authority (D6).                                       |
| C-8          | Session operations only on a verified task binding (D2).                                                  |
| O §13.2–14.5 | Capability discovery separate from request state; delivery, ack and progress as separate stages (D3, D5). |
| H-SURVIVE-A  | Every disabled state carries a reason; unknown is never shown as not sent (D3, D5).                       |

## 9. Gates

1. This record merged.
2. MOB-01 code on main (#15188) for `PmWorkView`.
3. The pure module and both test files can then land with fixtures and
   the fake adapter; they need no broker.
4. Enabling 11–13 against a real session needs DSP-05's broker read
   and capability probe, then DSP-07's receipt placement. Enabling 10
   direct needs DSP-06. Each is its own slice and approval.
5. Any live send is gated by #6533's live acceptance, MOB-09's switch,
   and Drew's approval for production sessions.

## 10. Non-goals

- Stop, resume, merge, close.
- A Slack, Muse or new provider integration.
- Deciding the broker's binding algorithm or receipt fields (DSP-05,
  DSP-07).
- Defining roles; the roles named here are read from the existing
  authorization owner (#9180 / AUTH slices).
