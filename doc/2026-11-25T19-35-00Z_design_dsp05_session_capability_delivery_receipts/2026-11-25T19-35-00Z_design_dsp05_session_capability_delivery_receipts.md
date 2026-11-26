# DSP-05: Session capability, existing-binding lookup and message delivery receipts

Issue: [#9505](https://github.com/developing-today/code/issues/9505)
(slice `DSP-05` of [#9203](https://github.com/developing-today/code/issues/9203),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **V3 design proposal; not an authorization to implement.**

This record elaborates the DSP-05 slice of the merged design package
(`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`,
PR #9225) into four contracts an implementation issue can be filed
against: the authoritative binding of a task to a session, the
existing-binding lookup, the per-provider capability declaration, and
the message delivery receipt. It authorizes no code, schema,
migration, deployment, live send or live acceptance; those remain
separately gated stages (section 13).

The goal, from the slice text
(`issues/bodies/DSP-05.md:10-12`, component
`components/9203-direct-dispatch-slash-commands-event-routing.md:160-166`):
**deliver an instruction only to the task's authoritative
currently-bound session, and expose provider-specific capability and
acknowledgement states.** The oracle
(`issues/bodies/DSP-05.md:98`): "verified matching task/session sends
once; unbound shell sends never; unsupported adapter says
unsupported; Slack delivered is not ack; retry after timeout
reconciles."

The decision in one line: **the send target is read from the broker's
binding record at send time and nothing else; every delivery stage is
a separate, evidence-backed effect field; an adapter advertises only
the stages it can prove; and an ambiguous send is reconciled before
any retry or stays `uncertain`.**

All paths below are relative to the repository root unless they start
with `components/`, `decisions/`, `census/`, `sources/` or `issues/`,
which are relative to the design package directory. Line numbers are
at checkout HEAD `5bacbbf28` (2026-11-25T19:27Z) unless marked as the
package's pinned baseline `a038f084d`.

## 0. Governing decisions consumed, not re-opened

- **Component #9203 section 3.4, steps 2 to 4**
  (`components/9203-...md:78-80`): query the session adapter for the
  session bound to the task; "check task/session/worktree
  relationship, session health and actual supported `send`
  capability. Never use only issue labels, recently seen session
  name, or a free text session id"; send with operation id as
  correlation id; record adapter-accepted, delivered, ack, progress
  and error separately; "For Slack/Muse, 'message posted' is only
  transport delivery unless the session system returns an
  authenticated ack."
- **Component #9203 section 3.5** (`:85-97`): stages are effect
  detail, "not a second lifecycle/outcome enum"; ambiguous timeout is
  `uncertain`; known intent awaiting readback is `pending_sync`;
  effect-level reasons such as `send_failed` and `ack_timeout` are
  "not new top-level outcomes"; never record credentials.
- **Component #9203 section 3.7** (`:119`): "Session supports no
  send/resume: Show actual unsupported capability and documented
  alternate. No-go: Fake button, Slack post presented as session
  ack."
- **Component #9203 section 6** (`:233`, `:236`): reconcile before
  retry; "If no reliable reconciliation query exists, do not retry an
  ambiguous irreversible start automatically"; "Session/provider ack
  semantics vary. Default expose unsupported or delivery-only, not
  synthetic acknowledgment."
- **FDR-PMO-007** (`decisions/FDR-PMO-007-direct-dispatch-receipts.md:33`,
  `:37`): "For an active task, resolve authoritative current binding
  and supported adapter capability; send instruction with operation
  correlation ID and record delivery vs acknowledgment separately";
  "Provider adapter capability matrix advertises only verified
  operations. Slack/channel post means transport delivery, not
  session acknowledgment."
- **architecture.md, "Shared command and result interface"**
  (`architecture.md:101-160`): the `fleet.command/v1` and
  `fleet.result/v1` envelopes; the eight shared outcomes; "Opaque
  provider/session IDs stay opaque strings"; "A timeout returns a
  resumable operation receipt"; "Capability discovery advertises only
  the commands and guarantees that target actually supports."
- **architecture.md, "Identity and authority"** (`:91-95`): "provider
  owns actual session state"; existing session interfaces
  #9073 to #9076 "are consumed, not re-created".
- **Recovered original O section 14.5** (`sources/04_recovered_original_combined_design.md`,
  flattened onto one line; heading "## 14.5 Live-session adapters"):
  candidate targets OpenCode, Fleet lanes, other providers, Muse,
  Slack, GitHub comments; the abstraction `agent.session.find / send
/ status / start / resume / stop`; "Each adapter must honestly
  report supported capabilities. A Slack message being posted is not
  proof that an agent received and acted upon it."

Neighbour slices whose decisions this record reuses (section 12 lists
conflicts):

- **DSP-01** (#9445, closed): `work/operations.rs` already holds the
  acceptance ledger; refusal records nothing; the delivery GUID is
  provenance only.
- **REL-03** (#9513, open): owns `operation_id`, idempotency key,
  request digest and the stable effect receipt
  (`components/9209-leases-fencing-outbox-direct-completion.md:167-171`).
  This record consumes them and adds no second idempotency store.
- **DSP-04** (#9446, open): owns item resolution and the routing
  decision table, including `ambiguous_target`. DSP-05 starts from a
  resolved `(authority_id, work_id)`.
- **DSP-06** (#9506, open): owns direct launch, the initial prompt and
  the first bind. DSP-05 never launches.
- **DSP-07** (#9447, open): owns the `Result.effects[]` receipt
  verifier and the #6533 live fixture.
- **HYD-07** (#9509, record
  `doc/2026-11-23T12-19-51Z_design_hyd07_operation_session_attribution/`):
  `session_id` is opaque and never derived from an actor name;
  `operation_id` is REL-03's. DSP-05 records the
  `operation_id`/`session_id` pair on every send so HYD-07 can
  attribute what follows.
- **SEL-02** (record `doc/2026-11-25T01-18-23Z_design_sel02_claim_revalidation/`):
  live inputs are evaluated at commit, never taken from a snapshot;
  every non-success outcome wrote nothing. DSP-05 applies the same
  rule to the binding (section 4.4).
- **CLI-02** (#9438, closed; record
  `doc/2026-11-23T12-33-39Z_design_cli02_shared_command_contract_strict_parser/`):
  owns spelling and strict parsing; "DSP-05 to DSP-10 keep their own
  ownership" (`:504-507`). This record fixes semantics only.

## 1. Intent

An authorized person writes "please check the reconnect case before
merging" on a live issue, or types it into the PM board (O section
14.3; O "# 17. Agent Walkthrough C"; charter scenario C, C-R152). The
message must reach the one session that is working that task now, and
the person must be told honestly how far it got: queued, handed to
the provider, present in the session, acknowledged by the agent, or
unknown.

Three failures this slice prevents:

1. **Wrong target.** Sending to a session named by a label, a stale
   worktree marker, a free-text id in the comment, a Slack thread, or
   a binding the broker already cleared.
2. **False receipt.** Reporting "delivered" or "acknowledged" on the
   strength of an HTTP 2xx, a Slack post, a queue row or a session
   shell (H-R043; component `:19`).
3. **Duplicate instruction.** Sending twice after a timeout whose
   first attempt may have landed (C-R093 in spirit; component
   `:233`).

## 2. What exists today vs what this record proposes

### 2.1 Exists at HEAD (source facts, not deployment proof)

**Binding storage.** One task has at most one bound agent session,
held in two places that the broker writes together:

- `repo_task_lane_ledger.session_id`
  (`pkgs/fleet/crates/fleet-queue/src/turso_queue/core.rs:1098-1104`;
  `task_id` is `UNIQUE`);
- `tasks.progress.session_id` and `progress.launched_by`, merged by
  the bind verb (`turso_queue/lifecycle.rs:1513-1525`).

The two are written together by `verb_bind`, but not by every path:
`record_execution` (`turso_queue/core.rs:2719-2760`) stamps the ledger
`session_id` from an execution report without touching `progress`,
and the #8210 transfer writes `progress` always but the ledger only
when a ledger row already exists (`turso_queue/claim.rs:1074-1096`).
So "exactly one of the two is set" is a reachable, legitimate state
today, not by itself evidence of corruption.

**Bind.** `verb_bind` (`turso_queue/lifecycle.rs:1442-1560`) refuses a
malformed id before any write (`:1455-1465`, #8287), refuses a task
that is not `claimed` with a live `lease_until` (`:1474-1488`), refuses
rebinding to a different session (`:1494-1500`), then writes both
places and appends `session_bound` (#9046) and the legacy
`lane_session_bound` event. The one-trip `/claim-flow` claim+bind and
the #8210 session-to-session transfer (`turso_queue/claim.rs:916-937`)
bind the same way. CLI: `cmd_bind`
(`client/commands_lifecycle.rs:699-776`) and `claim --bind`
(`claim_bind_path`, `client/commands_query.rs:772-797`).

**Unbind.** `clear_binding` (`turso_queue/core.rs:2497-2518`) removes
`progress.session_id` and stamps `progress.session_cleared =
{session_id, cleared_at, via}`; `on_binding_cleared` (`:2520-2552`)
nulls the ledger `session_id` and appends `session_binding_cleared`
and `session_unbound`. Callers: claim (`lifecycle.rs:630`), park
(`:2094`), release (`:2263`), lease expiry (`core.rs:2681`), fail
(`admin.rs:1232`). The terminal verbs complete and cancel **keep** the
binding on the row as the execution record and only append
`session_unbound` (`claim.rs:633-640`, `:662-680`). The CLI mirrors
closes into the timeline with `unbound_append`
(`client/commands_query.rs:885-908`; called from
`commands_lifecycle.rs:537, 940, 1451, 1523, 1680`).

**Live-binding predicate.** `bound_session` and `cleared_session_id`
(`turso_queue/lifecycle.rs:112-139`, #8368): a ledger session id equal
to the broker's own `session_cleared` stamp is "not bound in any live
sense".

**Session-id shape.** The crate's one predicate `valid_session_id`
(`client/commands_query.rs:916-918`): `ses_` followed by ASCII
alphanumerics or `_`.

**Caller-side identity (#9075, closed).**
`pkgs/fleet/crates/fleet/src/session_state.rs`: the `.fleet-session-id`
worktree marker (`:11`), resolution order explicit > `FLEET_SESSION_ID`
/ `OPENCODE_SESSION_ID` > marker (`:119-145`), and a marker that cannot
be verified live is an error, never a silent fallback (`:140-144`).
Liveness asks the OpenCode server `GET /api/session/<id>` and treats
an archived or unreadable record as not live (`:246-268`,
`:316-329`); without the `opencode-probe` feature every marker is not
live (`:271-276`).

**OpenCode adapter.** `pm-stack/dispatch/src/opencode.ts`:
`getSession` returns `null` only on HTTP 404 and throws on any other
failure, so "gone" and "unknown" stay apart (`:165-181`);
`listSessions` fails closed at the 200 cap (`:150-163`); `sendPrompt`
is `POST /api/session/<id>/prompt {text}` and returns nothing
(`:142-144`); `deleteSession` deletes (`:146-148`). The only caller of
`sendPrompt` is the launch path, which sends the first prompt to a
just-created session **before** `claimTask` binds it
(`pm-stack/dispatch/src/server.ts:328-353`). The Rust side has a
fail-soft session read for telemetry
(`client/support.rs:1300-1330`).

**Dispatch queue path.** `pm-stack/dispatch/src/queue.ts:1-20`: all
queue access now goes through the broker (#9028, closed);
`reconcileClaimFlow` (`:244-275`) recovers a lost `/claim-flow`
response by reading the task and accepting it only when its live claim
names this session. `findTaskBySessionId` (`:288-325`) matches the live
binding first and then the **cleared** record, by design, for the
read-route repo gate.

**Command acceptance (DSP-01).** `pkgs/fleet/crates/fleet-queue/src/work/operations.rs:205-329`:
`Command` keyed by idempotency key, `AcceptanceLedger::accept`
authorizes first, records nothing on refusal, and returns the stored
receipt on a repeat. It has no request digest, no `operation_id`, and
no stage receipts.

**Shared envelope.** `work/contracts.rs`: `fleet.command/v1`,
`fleet.result/v1` (`:43-45`); `Outcome` with the eight shared values
(`:513-534`); `Effect {field, requested, observed, verified}`
(`:828-840`); `CommandResult` (`:862-891`); the `ReasonCode` registry
is the priority domain's fifteen codes and unknown codes are never
aliased (`:324-394`, module doc `:30-32`).

### 2.2 Absent at HEAD

- No send-to-existing-session path anywhere: `agent.session.send`,
  `agent.session.resume`, `agent.session.stop`, `agent.status` and
  `/fleet agent status` are all `absent` in `census/commands.jsonl`.
- No capability declaration for any adapter.
- No delivery receipt, no acknowledgement evidence, no reconciliation
  query for a sent message.
- No Slack or Muse session adapter in `pm-stack/dispatch/src/` or
  `fleet-queue/src/work/`.
- No `pm-stack/dispatch/test/session-delivery.test.ts`; no test named
  `pmo_dsp_05_*`.

### 2.3 Proposed by this record

A pure binding-view and capability model, a lookup contract, a
send operation whose receipt stages are `Effect` fields in the shared
result, and the fake-provider test corpus. No new table, column,
queue, scheduler, service, resolver, executable or provider adapter
(`issues/bodies/DSP-05.md:76-82`).

## 3. Data model

### 3.1 `BindingView` (derived, never stored)

One read-only projection of the existing records, computed by the
broker inside one read transaction:

| Field              | Source                                                                                         |
| ------------------ | ---------------------------------------------------------------------------------------------- |
| `work`             | `(authority_id, work_id)`; `work_id` is the task id as a canonical decimal string              |
| `task_status`      | `tasks.status`                                                                                 |
| `lease_until`      | `tasks.lease_until`, compared with the **store** clock                                         |
| `ledger_session`   | `repo_task_lane_ledger.session_id`                                                             |
| `progress_session` | `tasks.progress.session_id`                                                                    |
| `cleared_session`  | `tasks.progress.session_cleared.session_id`                                                    |
| `launched_by`      | `tasks.progress.launched_by`                                                                   |
| `binding_ref`      | id of the latest `session_bound` event for this task (the existing `events.id`)                |
| `provider`         | `opencode` for every binding written by today's bind paths (they only accept `ses_` ids)       |
| `directory`        | worktree directory when a provider read returns one (`sessionStatusView`, `server.ts:897-910`) |

`binding_ref` is the staleness token. It costs no new column: every
bind path already appends `session_bound`, and a rebind after a clear
appends a new one with a larger `events.id`. When REL-02 lands its
`fence_generation`, the token becomes the pair
`(fence_generation, binding_ref)`; until then the event id alone.

### 3.2 `SessionCapability` (static per adapter, plus a live probe)

```text
SessionCapability {
  adapter:   "opencode" | "fleet_lane" | "slack" | "muse" | "github_comment" | <future>,
  operations: { find, status, send, resume, stop }
              each: "supported" | "unsupported" | "unverified",
  max_evidence: "none" | "adapter_accepted" | "delivered" | "acknowledged" | "progressing",
  reconcile:  "by_correlation_marker" | "none",
  verified_by: [test or evidence reference]   // empty means not advertised
}
```

The static half is a typed constant compiled with the adapter; the
live half is the per-send provider status read (section 4.3). Only
`supported` with a non-empty `verified_by` is advertised (FDR-PMO-007
`:37`; architecture `:158-160`). `unverified` is reported to callers as
unsupported with the reason `capability_unverified`, so a caller can
tell "never possible" from "not yet proven". Nothing about capability
is persisted.

### 3.3 Delivery evidence ladder

Each step is a fact a named source can show; a higher step never
implies a lower one was observed unless its evidence covers it.

| Level              | Meaning                                                     | Evidence that may set it                                                              |
| ------------------ | ----------------------------------------------------------- | ------------------------------------------------------------------------------------- |
| `reserved`         | operation durably accepted, send intent recorded ("queued") | the operation journal row (REL-03 / DSP-03 home)                                      |
| `adapter_accepted` | the provider API returned success for the send request      | adapter response, e.g. OpenCode prompt 2xx                                            |
| `delivered`        | the message is readable inside the bound session            | a provider read-back that finds the correlation marker in that session                |
| `acknowledged`     | the agent in that session responded to this message         | an authenticated provider signal naming this message; **none is verified today**      |
| `progressing`      | the session did work after the message                      | provider activity for that session after `delivered`, e.g. #9046 `route_attempt` rows |

`progressing` is weaker than `acknowledged` for attribution: activity
after a message does not prove the activity answers it. It is
reported as its own field and never promoted to `acknowledged`.

### 3.4 Receipt states (the effect values)

`DeliveryState` is the `observed`/`verified` string of the
`message_delivery` effect (section 7). Values:

- `queued`: `reserved`, no send attempted yet.
- `adapter_accepted`, `delivered`, `acknowledged`, `progressing`:
  the ladder above.
- `rejected`: refused before any send; nothing left the broker.
- `send_failed`: the provider definitively refused (for OpenCode, a
  4xx other than 404, or 404 meaning the session is gone); no message
  was accepted.
- `unknown`: the send request may have left and no evidence shows
  whether it landed (connection reset, timeout, 5xx after the body was
  sent, broker crash after the call). **`unknown` means "not proven
  either way"; it is never read as "not sent".**

These are effect-level values under the shared outcome, not a second
outcome enum (component `:87`, `:95`).

## 4. Existing-binding lookup contract

### 4.1 Inputs

| Input                  | Required | Meaning                                                                                   |
| ---------------------- | -------- | ----------------------------------------------------------------------------------------- |
| `work`                 | yes      | resolved `(authority_id, work_id)` from DSP-04; never a bare issue number                 |
| `expected_session_ref` | no       | the session the caller believes is bound (`--session-id`, an env value, the #9075 marker) |
| `expected_binding_ref` | no       | `binding_ref` from an earlier lookup the caller is acting on                              |
| `probe_provider`       | no       | default true for `send`; false for a pure status read that must not touch the provider    |

`expected_session_ref` is an **expectation to compare, never the
target** (component `:78`). A caller's own identity from
`session_state.rs` answers "who am I", not "who owns this task".

### 4.2 Algorithm (broker side, one read transaction)

1. Read the task row and its ledger row.
2. If the task does not exist at this authority: `not_found`.
3. If `task_status` is not `claimed`: `no_binding` with
   `reason = task_not_live` and the status. This covers `parked`,
   `queued`, `handed_off`, `blocked` and the terminal statuses; for
   `done` and `cancelled` the row may still carry `session_id` as
   history (`claim.rs:633-640`), and that is reported as
   `last_session_ref`, never as a target.
4. If `lease_until` is at or before the store clock: `no_binding`,
   `reason = lease_expired`. This holds even before the expiry sweep
   (`core.rs:2681`) has cleared the row, matching the liveness rule
   `verb_bind` already applies (`lifecycle.rs:1474-1488`).
5. Compute `bound_session(ledger_session, cleared_session)`
   (`lifecycle.rs:120-131`). None: `no_binding`, `reason = never_bound`
   when no `session_bound` event exists, else `reason = cleared` with
   the `via` from the stamp.
6. If `progress_session` and `ledger_session` are both present and
   differ: `ambiguous`, `reason = binding_inconsistent`, both values
   returned; the lookup never picks one. If exactly one is present
   (reachable today, section 2.1), it is accepted only when the
   #9046 binding history agrees: the latest `session_bound` event for
   the task names that same session and no later `session_unbound`
   names it. Otherwise `ambiguous`, `reason = binding_partial`.
7. If the session id fails `valid_session_id`: `ambiguous`,
   `reason = binding_malformed`.
8. If `expected_binding_ref` is given and differs: `stale`,
   `reason = stale_binding`, current `binding_ref` returned.
9. If `expected_session_ref` is given and differs: `stale`,
   `reason = session_mismatch`, current session returned.
10. Otherwise `bound`.

When `probe_provider` is true and the result is `bound`, the adapter
reads the session (section 4.3) **after** the transaction; a broker
transaction is never held across HTTP (architecture `:345-347`).

### 4.3 Provider probe

For OpenCode, `GET /api/session/<id>` (`opencode.ts:170-181`):

- record with matching `id` and no `time.archived`: `live`
  (the rule in `session_state.rs:316-329`);
- record archived: `session_gone`;
- HTTP 404: `session_gone`;
- any other failure: `provider_unknown`.

When the lookup knows the task's worktree directory and the record
reports a different one, the result is `session_mismatch` with
`detail = directory`. A `session_gone` result does not clear the
binding; recovery of an orphaned binding stays with the existing
stuck-bound supervision and #9178 rules (component `:95`).

### 4.4 Outputs

```text
BindingLookup {
  status: "bound" | "no_binding" | "ambiguous" | "stale" | "not_found" | "unknown",
  reason: <code, section 8>,
  work, session_ref?, provider?, binding_ref?, lease_until?,
  launched_by?, last_session_ref?, provider_state?: "live" | "session_gone" | "provider_unknown" | "not_probed",
  observed_at
}
```

`unknown` is returned when the broker itself cannot be read; it fails
closed, as the dispatch read route already does for its repo gate
(`queue.ts:288-301`). A lookup is advisory: the send re-runs steps 1
to 10 at send time (section 5.2), as SEL-02 re-evaluates live inputs
at commit rather than trusting the chooser's view.

### 4.5 One session, several tasks

#9046 states one session may serve several tasks
(`claim.rs:633-640`), and #9178 keeps explicit multi-task same-session
claims possible (component `:17`). Lookup is task to session, so this
is not ambiguity. Each send names its `work` in the message marker and
receipt, and receipts stay per work item.

### 4.6 What is never a binding source

- issue labels, Project fields, board state;
- a session id inside comment text or a Slack thread;
- `findTaskBySessionId` (`queue.ts:288-325`), because it deliberately
  matches cleared bindings;
- the #9075 marker or environment of a process other than the bound
  session itself;
- placement sessions (`fp_` handles, `turso_queue/placement_sessions.rs:1-13`,
  `placement.rs:2399`), which are dispatch placement records, not
  agent sessions. The allocated `session.status`, `session.list` and
  `session.result` rows (`census/command-slice-map.jsonl`) are these
  placement commands; DSP-05 must not reuse their status as agent
  session status.

## 5. Send operation

### 5.1 Command

`agent.session.send` (final spelling: CLI-02 / CLI-13) carries the
shared envelope with `kind = "session_send"` and:

| Field                                           | Meaning                                                                                           |
| ----------------------------------------------- | ------------------------------------------------------------------------------------------------- |
| `operation_id`                                  | REL-03 operation identity; also the correlation id                                                |
| `idempotency_key`                               | for a webhook comment, the DSP-01 semantic key (`operations.rs:208-211`); else caller-supplied    |
| `request_digest`                                | REL-03 digest of the canonical request, body included                                             |
| `work`                                          | resolved pair                                                                                     |
| `session_ref`                                   | optional expectation (section 4.1)                                                                |
| `arguments.body_ref`                            | where the message text is held (the operation journal); the text itself is not in the receipt     |
| `arguments.require`                             | `delivered` (default) or `acknowledged`                                                           |
| `arguments.wait_seconds`                        | optional bound for waiting on `delivered` / `acknowledged`; zero returns after `adapter_accepted` |
| `source_ref`, `source_event_ref`, `delivery_id` | origin (comment, PM action, CLI) for audit                                                        |

### 5.2 Order of operations

1. Authenticate; authorize the principal for `session_send` on this
   work (DSP-01 policy; `actor_ref` is not proof, `contracts.rs:20-26`).
2. Idempotency lookup scoped to that authorization (component `:63`):
   same key and digest returns the stored receipt and sends nothing;
   same key, different digest is `conflict` /
   `idempotency_conflict`.
3. Capability gate: if the bound adapter's `send` is not advertised,
   or `require = acknowledged` and `max_evidence` is below it:
   `rejected` with `capability_unsupported` /
   `ack_unsupported`, and `next_action` naming the documented
   alternate where one is configured (component `:119`; O section
   14.7 "Use an authorized alternate route only where configured").
4. Binding check, steps 1 to 10 of section 4.2, in the same
   transaction as step 5. Any status other than `bound` is
   `rejected` (no binding, ambiguous) or `conflict` (stale), with no
   send.
5. Durable reservation: write the operation record with the
   `binding_ref`, `session_ref`, digest and state `queued`; commit.
   Nothing has left the broker yet.
6. Reauthorize and re-read the binding immediately before the call
   (architecture `:126-127`: "Re-check revocable permission and
   target preconditions before send"). A changed `binding_ref` or a
   revoked grant ends the operation as `conflict` or `rejected`, still
   with nothing sent.
7. Provider probe (section 4.3); `session_gone` ends as `rejected`
   with `session_gone`; `provider_unknown` ends as `rejected` with
   `provider_unreachable` (nothing was sent, so this is not
   `unknown`).
8. Send once, with the correlation marker (section 5.3). Record
   `adapter_accepted`, `send_failed` or `unknown` from the response.
9. If `wait_seconds` > 0: read back for `delivered`, then for the
   higher levels the adapter supports, until the bound. Record each
   level with its evidence reference and time.
10. Return the result (section 7).

No step holds the broker transaction across the provider call.

### 5.3 Correlation marker

The sent text is prefixed with a single machine-readable line naming
the operation and work, in the style the launch path already uses for
its first prompt (`server.ts:343-345`, `[dispatch] Queue task #...`).
The exact line format is an open question (section 11, Q3). The
marker is what read-back and reconciliation search for; without a
provider read that returns session messages, `delivered` cannot be
set (section 6).

### 5.4 Idempotency, retries and `unknown`

- **Replay** of a key whose operation is past `queued` returns the
  stored receipt, refreshed by a read-back if the adapter supports one;
  it never sends again.
- **Timeout or lost response** sets `unknown` and outcome `uncertain`
  (`contracts.rs:526-529`). The next attempt with the same key first
  reconciles: read the bound session's messages for the marker. Found:
  record `delivered` (and `adapter_accepted`), outcome as section 7.
  Not found after a read the adapter declares complete: the attempt
  may send again under the **same** `operation_id`. Not readable:
  stays `uncertain`.
- **No reconciliation query** (`reconcile = none`): an `unknown` send
  is never retried automatically (component `:233`). An authorized
  person may issue a **new** operation that names the old one in
  `supersedes`, and its preview states that the first message may
  also have arrived.
- **Binding changed between the attempt and the retry**: the retry
  does not follow the task to the new session. The old operation stays
  `uncertain` against the old `binding_ref`; a new send is a new
  operation (section 4.4: the target is the binding at send time).
- **Retry budget**: none is invented here; it is the provider policy
  REL-05 names (`components/9209-...md:73`, `:177`).

## 6. Provider capability matrix

Facts are stated only from the inspected sources; everything else is
marked **unverified**. Nothing below has been exercised against a live
provider in this pass.

| Adapter                               | find / status                                                                                       | send                                                                                                                                       | max_evidence today                                                                                                                                                                                                                                                      | resume / stop                                                                                                     | reconcile                                  |
| ------------------------------------- | --------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------- | ------------------------------------------ |
| OpenCode (`opencode.ts`)              | supported: `GET /api/session/<id>`, 404 is gone (`:170-181`); list fails closed at 200 (`:150-163`) | endpoint exists and is used at launch (`:142-144`, `server.ts:345`); its semantics for an **existing busy** session are **unverified**     | `adapter_accepted` (2xx). `delivered`: **unverified** (no message-read call in the client). `acknowledged`: **unverified**, not advertised. `progressing`: candidate via #9046 `route_attempt` join (`commands_query.rs:922-1000`), **unverified** as a delivery signal | resume: **unverified**. stop: **unsupported**; `deleteSession` deletes the session (`:146-148`) and is not a stop | **unverified** until a message read exists |
| Fleet execution lane (queue claimant) | supported via broker `/list` and the binding view                                                   | **unsupported**: a lane has no inbound message channel; the queue is not a message bus                                                     | `none`                                                                                                                                                                                                                                                                  | resume belongs to #9178 resume-by-session and DSP-06; stop **unsupported**                                        | n/a                                        |
| Slack                                 | none identified                                                                                     | no approved integration identified in `pm-stack/dispatch/src` or `fleet-queue/src/work`; **unsupported** until one is (`DSP-05.md:134`)    | if added: at most `delivered` meaning **transport** delivery; `acknowledged` never without an authenticated session ack (O 14.5; FDR-PMO-007 `:37`)                                                                                                                     | **unsupported**                                                                                                   | n/a                                        |
| Muse                                  | none identified                                                                                     | **unsupported** until an approved integration is identified (`DSP-05.md:134`)                                                              | `none`                                                                                                                                                                                                                                                                  | **unsupported**                                                                                                   | n/a                                        |
| GitHub comment fallback               | n/a                                                                                                 | not a session send. O 14.5 lists it "when direct session messaging is unavailable"; it is a remote write owned by the REL/AUTH outbox path | at most a created comment id with read-back; never `delivered` to a session or `acknowledged`                                                                                                                                                                           | n/a                                                                                                               | comment id read-back (REL-06)              |

Consequences for the oracle: "unsupported adapter says unsupported"
is the lane, Slack, Muse and stop rows; "Slack delivered is not ack"
holds by construction because the Slack row's ceiling is transport
`delivered`. Until OpenCode message read-back is verified, an OpenCode
send with `require = delivered` and `wait_seconds > 0` ends as
`pending_sync` with `delivered` unobserved, not as
`verified_complete`. That is an honest limit, and the first research
step for implementation (section 11, Q1).

## 7. JSON and error contract

### 7.1 Command (illustrative)

```json
{
  "schema_version": "fleet.command/v1",
  "operation_id": "op_01J...",
  "idempotency_key": "ghcmd:v1:developing-today/code#754:comment:92001:send",
  "request_digest": "sha256:...",
  "work": { "authority_id": "fleet-main", "work_id": "16056" },
  "kind": "session_send",
  "arguments": { "body_ref": "journal:...", "require": "delivered", "wait_seconds": "30" },
  "actor_ref": "github:dezren39",
  "source_ref": "github:developing-today/code#754",
  "source_event_ref": "comment:92001",
  "delivery_id": "d-1",
  "session_ref": null,
  "reason": { "code": "...", "evidence_refs": [] },
  "dry_run": false
}
```

The `reason.code` value is a gap: the `ReasonCode` registry
(`contracts.rs:324-394`) holds only priority-domain codes and refuses
unknown ones (section 11, Q4). Numeric values that may exceed the
JavaScript safe range are canonical decimal strings
(architecture `:146-147`).

### 7.2 Result

Outcome is one of the shared eight. Stage detail lives in three
`Effect` entries (`contracts.rs:828-840`):

| `effect.field`     | `requested`                       | `observed`                            | `verified`                                       |
| ------------------ | --------------------------------- | ------------------------------------- | ------------------------------------------------ |
| `session_binding`  | expected `binding_ref` or null    | `binding_ref` read at send            | same value when the post-send re-read matches    |
| `message_delivery` | `delivered`                       | highest `DeliveryState` with evidence | highest state confirmed by read-back, else null  |
| `session_ack`      | `acknowledged` or `not_requested` | `acknowledged` / `unsupported` / null | `acknowledged` only from an authenticated signal |

Outcome mapping:

| Situation                                                                                        | `outcome`           | `error.code`                                                |
| ------------------------------------------------------------------------------------------------ | ------------------- | ----------------------------------------------------------- |
| refused before reservation (auth, capability, no binding, ambiguity, gone, provider unreachable) | `rejected`          | section 8 code                                              |
| stale binding or session mismatch; same key, different digest                                    | `conflict`          | `stale_binding`, `session_mismatch`, `idempotency_conflict` |
| reserved, not yet sent                                                                           | `accepted`          | none                                                        |
| sent, waiting within `wait_seconds`                                                              | `running`           | none                                                        |
| `adapter_accepted`, requested level not yet confirmed                                            | `pending_sync`      | none; `next_action` names the read-back                     |
| provider definitively refused                                                                    | `rejected`          | `send_failed`                                               |
| `unknown`                                                                                        | `uncertain`         | `send_timeout`; `next_action: reconcile`                    |
| requested level confirmed; ack asked and timed out                                               | `partial`           | `ack_timeout`                                               |
| every requested effect verified                                                                  | `verified_complete` | none                                                        |

`verified_complete` here means "this send reached the verified level
asked for"; it never completes the work item (`contracts.rs:508-511`).

### 7.3 What the receipt must carry (proposed fields)

`operation_id`, `idempotency_key`, `work`, `session_ref`, `provider`,
`binding_ref`, adapter name and capability snapshot, per-stage
timestamp and evidence reference, adapter delivery id where the
provider returns one, the authenticated principal and policy version,
and source references. The shared `CommandResult` has no slot for
per-stage timestamps or evidence references (`contracts.rs:862-891`);
placing them is DSP-07's receipt contract and REL-03's stable receipt.
This record names the fields and leaves their envelope placement to
those owners (section 12, gap G4).

### 7.4 Redaction

The receipt, the events and every log line carry no message body, no
OpenCode password or `Authorization` header (the clients already never
log it: `opencode.ts:1-4`, `support.rs:1292-1299`), no bearer token,
and no private note text. The body is referenced by `body_ref` and
bounded by the journal's own visibility rules. Whether a body digest
may appear depends on a digest domain the architecture registry does
not yet have (`contracts.rs:52-63`; section 11, Q5).

## 8. Reason codes

Stable snake_case strings for `Result.error.code` and lookup
`reason`. They are not added to the priority `ReasonCode` registry by
this record.

| Code                                      | Raised by                        |
| ----------------------------------------- | -------------------------------- |
| `unauthorized`                            | 5.2 step 1, step 6               |
| `idempotency_conflict`                    | 5.2 step 2                       |
| `capability_unsupported`                  | 5.2 step 3                       |
| `capability_unverified`                   | 5.2 step 3                       |
| `ack_unsupported`                         | 5.2 step 3                       |
| `not_found`                               | 4.2 step 2                       |
| `task_not_live`                           | 4.2 step 3                       |
| `lease_expired`                           | 4.2 step 4                       |
| `never_bound`, `cleared`                  | 4.2 step 5                       |
| `binding_inconsistent`, `binding_partial` | 4.2 step 6                       |
| `binding_malformed`                       | 4.2 step 7                       |
| `stale_binding`                           | 4.2 step 8, 5.2 step 6           |
| `session_mismatch`                        | 4.2 step 9, 4.3                  |
| `session_gone`                            | 4.3, 5.2 step 7                  |
| `provider_unreachable`                    | 4.3, 5.2 step 7                  |
| `send_failed`                             | 5.2 step 8 (component `:95`)     |
| `send_timeout`                            | 5.2 step 8                       |
| `ack_timeout`                             | 5.2 step 9 (component `:95`)     |
| `ambiguous_target`                        | DSP-04, passed through unchanged |

## 9. Failure modes

| Scenario                                                             | Result                                                                                      |
| -------------------------------------------------------------------- | ------------------------------------------------------------------------------------------- |
| Task claimed, bound, lease live, session live                        | one send; receipt at the adapter's ceiling                                                  |
| Task queued or parked (binding cleared on park, `lifecycle.rs:2094`) | `rejected` / `task_not_live`; DSP-04 routes to DSP-06 or triage                             |
| Lease passed, expiry sweep not yet run                               | `rejected` / `lease_expired`; no send to a session that may be gone                         |
| Task `done`, row still carries `session_id`                          | `rejected` / `task_not_live`, `last_session_ref` reported                                   |
| Ledger and progress disagree                                         | `rejected` / `binding_inconsistent`; nothing chosen                                         |
| Comment names a session id                                           | ignored as a target; compared as `expected_session_ref` only if passed as one               |
| Rebind between lookup and send                                       | `conflict` / `stale_binding`; nothing sent                                                  |
| OpenCode session archived or 404                                     | `rejected` / `session_gone`; binding not auto-cleared                                       |
| OpenCode server down before send                                     | `rejected` / `provider_unreachable`; nothing sent                                           |
| Provider 2xx received, then the read-back wait times out             | `pending_sync`: `adapter_accepted` stands; `delivered` not yet observed                     |
| Request sent, connection lost                                        | `uncertain` / `unknown`; reconcile before any retry                                         |
| Duplicate webhook delivery of the same comment                       | stored receipt returned; one send                                                           |
| Same key, edited body                                                | `conflict` / `idempotency_conflict` (an edited comment is a new candidate, component `:71`) |
| Principal revoked after reservation                                  | `rejected` at step 6; nothing sent                                                          |
| Slack post succeeds (if an adapter is ever added)                    | `delivered` (transport) at most; `session_ack` stays `unsupported`                          |
| Broker crash after reservation, before send                          | operation stays `queued`; resumed by the operation owner, sent once                         |
| Broker crash after send, before recording                            | `unknown` on recovery; reconcile                                                            |

## 10. Commands allocated

The issue allocates 30 command keys (`issues/bodies/DSP-05.md`,
"Command allocation"). In `census/command-slice-map.jsonl` DSP-05 is
**primary** for seven and supporting for seven; the remaining sixteen
reach DSP-05 only through the merged `CLI-11` facade
(`DSP-05.md:139-141`). This record fixes semantics for the primary
rows; spelling is CLI-02's and registration is CLI-13's.

| Key                                                | Inventory                | DSP-05 semantics                                                                                                                     |
| -------------------------------------------------- | ------------------------ | ------------------------------------------------------------------------------------------------------------------------------------ |
| `agent.session.send`                               | absent                   | section 5                                                                                                                            |
| `agent.status`, `agent.slash.agent-status`         | absent                   | lookup (section 4) plus capability snapshot; read-only, `probe_provider` optional                                                    |
| `agent.session.resume`                             | absent                   | capability answer only: OpenCode resume unverified, so `rejected` / `capability_unverified`; resume of parked work is #9178 / DSP-06 |
| `agent.session.stop`                               | absent                   | `rejected` / `capability_unsupported` for every adapter in section 6                                                                 |
| `agent.slash.resume`                               | absent                   | lookup first: a live binding returns `bound` with no new session (H-R044); otherwise hand to DSP-04/DSP-06                           |
| `flag.session-id`                                  | implemented (claim mark) | on DSP-05 commands it is `expected_session_ref`, never the target                                                                    |
| `current.claim.bind`, `current.claim.from-session` | implemented              | V1 bind paths consumed unchanged (section 2.1)                                                                                       |
| `session.status`, `session.list`, `session.result` | implemented              | placement sessions; explicitly **not** agent session status (section 4.6)                                                            |
| `agent.dispatch`, `agent.slash.start`              | absent                   | DSP-06 primary; DSP-05 supplies the "already bound" check only                                                                       |
| the other `agent.slash.*` and `flag.*` rows        | mixed                    | owned by their primary slices (DSP-02, DSP-03, DSP-04, DSP-08, DSP-09); DSP-05 adds nothing                                          |

## 11. Open questions

- **Q1. OpenCode delivery read-back.** Does the OpenCode server expose
  a call that lists a session's messages, so the marker can be found?
  Until verified, `delivered` and reconciliation for OpenCode are
  unavailable and every timeout stays `uncertain`.
- **Q2. OpenCode prompt to a busy session.** What does
  `POST /api/session/<id>/prompt` do when the session is mid-turn:
  queue, interleave, or refuse? The launch path only calls it on a
  fresh session.
- **Q3. Marker format.** One line, machine-parsable, carrying
  `operation_id` and `work`; must not be confused with the launch
  prefix and must survive provider text normalization.
- **Q4. Reason-code home.** Extend the shared registry with a session
  domain, or keep these codes as `Result.error.code` only? The
  registry refuses unknown codes by design.
- **Q5. Body digest.** May a body digest appear in receipts, and under
  which registered digest domain?
- **Q6. Ack signal.** Is there any authenticated, message-specific
  acknowledgement any provider can emit? If not, `acknowledged` stays
  unadvertised everywhere and `require = acknowledged` is always
  rejected.
- **Q7. Session-id predicate split.** The broker accepts `_` after
  `ses_` (`commands_query.rs:916-918`); `fleet session bind` does not
  (`session_state.rs:106-110`). Which is canonical?
- **Q8. Where stage events live before REL-03 lands.** Proposed:
  additive `events` kinds on the task (`session_message_*`), no new
  table; needs the storage owner's review.

## 12. Conflicts and gaps with neighbouring sources

- **G1. Component V1 baseline is outdated on the queue path.**
  Component `:14` and `:24` say pm-dispatch enqueues through a
  Supabase-backed client; at HEAD `queue.ts:1-20` routes all access
  through the broker and #9028 is closed. This record uses the HEAD
  state. The component text is the package's pinned history and is not
  edited here.
- **G2. Gate state differs from the issue text.** The issue names
  REL-03 (#9513), #9075 and CLI-02 (#9438) as hard blockers
  (`DSP-05.md:86`). On 2026-11-25, #9075 and #9438 are closed and
  #9513 is open. The original slice text also names DSP-03/04
  (`DSP-05.md:137`); DSP-03 is normalized to REL-03 and DSP-04
  (#9446) is open but not a normalized hard blocker.
- **G3. O section 17.** The slice cites "O §14.3/14.5/17". The
  recovered source has two "# 17." headings: "Project and Epic
  Urgency" and "Agent Walkthrough C: Live User Instruction". Only the
  second bears on this slice; this record cites it.
- **G4. Receipt envelope has no stage slot.** `CommandResult` lacks
  per-stage timestamps and evidence references; DSP-07 and REL-03 must
  place them.
- **G5. Launch prompt precedes bind.** `server.ts:343-353` sends the
  first prompt before `claimTask` binds. That is DSP-06's initial
  prompt, not an instruction on existing work; DSP-05's "bound only"
  rule does not apply to it, and DSP-06/DSP-07 should record that
  ordering in the launch receipt.
- **G6. DSP-01 ledger lacks a digest.** `operations.rs:205-329` keys
  by idempotency key alone; "same key, different body is a conflict"
  needs REL-03's digest.
- **G7. Binding written unevenly.** `record_execution` and the
  transfer path can leave only one of ledger and progress set
  (section 2.1). Section 4.2 step 6 tolerates this through the #9046
  history; the storage owner may prefer to make all paths write both.
- **G8. Command allocation spread.** Sixteen of thirty allocated keys
  have another primary slice in the census map. No conflict in
  semantics was found; ownership is as in section 10.
- No contradiction with HYD-07, SEL-02 or CLI-02 decisions was found.

## 13. Gates and sequencing

Implementation of this slice starts only after:

- PR #9225 merged (done).
- REL-03 (#9513): `operation_id`, digest and stable receipt; also the
  shared-file predecessor on `work/operations.rs` (`DSP-05.md:143-147`).
- #9075 session state (closed; consumed as in section 2.1).
- CLI-02 (#9438) shared parser (closed; spellings from its record).
- DSP-04 (#9446) for resolution and routing, as a soft predecessor.
- Q1 and Q2 answered from the real OpenCode API before any OpenCode
  `delivered` claim is advertised.
- A separately authorized live run for any real send. The #6533
  fixture remains DSP-07's.

Owned writable file: `pkgs/fleet/crates/fleet-queue/src/work/operations.rs`.
Read-only consumers: `work/contracts.rs`,
`pm-stack/projector/src/server.ts` (`DSP-05.md:76-82`). Tests for the
fake PM adapter go in `pm-stack/dispatch/test/session-delivery.test.ts`.

Rollback: disable the send path only. Keep every reserved, `unknown`
and delivered receipt; do not clear bindings, stop sessions or delete
history (`DSP-05.md:108-115`).

## 14. Acceptance oracle, specified

Future tests, written failing first, each listed by exact name with
`-- --list` before running (a zero-test filter is not acceptance),
under
`cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker <name>`:

1. `pmo_dsp_05_send_requires_verified_task_binding`: the slice's first
   failing test. Claimed task, live lease, consistent binding, live
   fake session: exactly one provider call carrying the marker, receipt
   names the `binding_ref`. Same task with no binding: zero calls,
   `rejected` / `never_bound`.
2. `pmo_dsp_05_cleared_binding_never_sends`: ledger id equals the
   `session_cleared` stamp; zero calls.
3. `pmo_dsp_05_expired_lease_never_sends`: lease past, sweep not run;
   zero calls, `lease_expired`.
4. `pmo_dsp_05_terminal_binding_is_history`: `done` row with
   `session_id`; zero calls, `last_session_ref` set.
5. `pmo_dsp_05_inconsistent_binding_refuses`: ledger and progress
   differ; zero calls, both values returned. A ledger-only binding
   (the `record_execution` shape) with a matching latest
   `session_bound` event sends; without one it is `binding_partial`.
6. `pmo_dsp_05_stale_binding_ref_conflicts`: rebind between lookup and
   send; zero calls, `conflict` / `stale_binding`.
7. `pmo_dsp_05_caller_session_is_expectation_not_target`: a different
   `expected_session_ref` yields `session_mismatch`; the provider is
   never called with the caller's id.
8. `pmo_dsp_05_unsupported_adapter_says_unsupported`: lane, Slack,
   Muse adapters and `stop` return `capability_unsupported` with zero
   calls; an `unverified` operation returns `capability_unverified`.
9. `pmo_dsp_05_slack_delivered_is_not_ack`: a fake transport-only
   adapter reaching its ceiling yields `message_delivery = delivered`,
   `session_ack = unsupported`, never `acknowledged`.
10. `pmo_dsp_05_retry_after_timeout_reconciles`: fake provider applies
    then times out; the retry with the same key reads back, finds the
    marker, records `delivered`, and makes no second send call.
11. `pmo_dsp_05_unreconcilable_timeout_stays_uncertain`: adapter with
    `reconcile = none`; retry with the same key makes zero calls and
    stays `uncertain`.
12. `pmo_dsp_05_replay_returns_stored_receipt`: same key and digest
    after success; zero calls, identical receipt.
13. `pmo_dsp_05_same_key_different_body_conflicts`:
    `idempotency_conflict`, zero calls.
14. `pmo_dsp_05_reauthorizes_before_send`: grant revoked after
    reservation; zero calls, `unauthorized`.
15. `pmo_dsp_05_provider_unknown_fails_closed`: probe error other than
    404; zero send calls, `provider_unreachable`.
16. `pmo_dsp_05_capability_matrix_advertises_only_verified`: every
    advertised operation has a non-empty `verified_by`.
17. `pmo_dsp_05_receipt_redacts_body_and_credentials`: canary strings
    in body, password and token are absent from the serialized result,
    events and logs.
18. `pmo_dsp_05_operation_and_session_correlated`: every receipt
    carries `operation_id`, `session_ref` and `work`, for HYD-07.
19. `pmo_dsp_05_no_provider_call_inside_broker_transaction`:
    source-level or fake-clock check that the send happens after
    commit.

Fake PM adapter, `cd pm-stack/dispatch && bun test test/session-delivery.test.ts`
(file to be created), cases: `sends once to verified binding`,
`never sends when unbound`, `getSession 404 is session_gone and 500
is provider_unknown`, `prompt 2xx is adapter_accepted not delivered`,
`timeout reconciles by marker before retry`.

## 15. Requirement trace (design-level)

"Addresses" means this record defines the behavior for DSP-05's half;
"contributes" means DSP-05 supplies one input and another slice owns
the outcome. Statuses are from `census/requirements-crosswalk.jsonl`.

| ID     | Crosswalk line / status           | Outcome (abridged)                                                 | Here                                                                    |
| ------ | --------------------------------- | ------------------------------------------------------------------ | ----------------------------------------------------------------------- |
| C-R005 | L5, partially implemented         | cohesive Fleet CLI incl. `agent session status/send/resume`        | contributes: section 10 semantics; spelling CLI-02                      |
| C-R009 | L9, partially implemented (group) | discover, claim, execute, handle new activity, park, complete      | contributes: section 4 (new activity reaches the bound session)         |
| C-R087 | L87, open implementation          | authenticated slash grammar incl. `agent status`                   | contributes: section 10 (`agent.slash.agent-status`)                    |
| C-R088 | L88, design-only                  | authorized plain-language requests on live/parked items            | contributes: section 5 is the live-item delivery; classification DSP-04 |
| C-R089 | L89, partially implemented        | live-issue comment routes to current session, records delivery/ack | addresses: sections 4, 5, 7                                             |
| C-R090 | L90, partially implemented        | parked-item comment launches or triages, not merely queued         | contributes: section 4.2 step 3 hands parked work to DSP-06             |
| C-R091 | L91, partially implemented        | verify, authorize, resolve, dedupe, find session, send, record     | addresses "find live session, send, record": sections 4, 5              |
| C-R093 | L93, partially implemented        | repeated deliveries must not double-launch                         | contributes: section 5.4 (no double send); launch is DSP-06             |
| C-R107 | L107, partially implemented       | cohesive agent-facing CLI                                          | contributes: section 10                                                 |
| C-R108 | L108, partially implemented       | common flags incl. `--session-id`, `--idempotency-key`, `--wait`   | addresses `--session-id` semantics (4.1), `wait_seconds` (5.1)          |
| C-R150 | L150, partially implemented       | scenario A, standard agent                                         | contributes: binding view used by the scenario                          |
| C-R151 | L151, partially implemented       | scenario B, blocked/parked/resumed                                 | contributes: cleared binding on park is `no_binding` (4.2)              |
| C-R152 | L152, partially implemented       | scenario C, live comment to bound OpenCode session with ack        | addresses: sections 1, 4, 5, 6 (ack limits stated)                      |
| C-R153 | L153, open implementation         | scenario D, near-timeout checkpoint and park                       | contributes: post-park sends are refused (9, row 2)                     |
| H-R001 | L168, design-only                 | preserve the two coordinated tracks                                | contributes: DSP-05 stays in the dispatch track                         |
| H-R012 | L179, partially implemented       | live-session routing, durable receipts, idempotent webhooks        | addresses live-session routing and receipts: 4, 5, 7                    |
| H-R014 | L181, design-only                 | session attribution among lifecycle guarantees                     | contributes: 7.3, test 18 (HYD-07 owns attribution)                     |
| H-R016 | L183, design-only                 | late comment, duplicate webhook, stale lease scenarios             | addresses for sends: section 9                                          |
| H-R017 | L184, partially implemented       | preserve full command inventory                                    | contributes: section 10                                                 |
| H-R021 | L188, unverified                  | verify prompt delivery, session identity; reuse active sessions    | addresses for messages: 3.3, 4, 6                                       |
| H-R022 | L189, unverified                  | "continue" is not recovery until ack or resumed work               | addresses: 3.3 (`acknowledged` vs `progressing`), 6                     |
| H-R024 | L191, design-only                 | keep every command, error, receipt in coverage                     | contributes: sections 7, 8, 10                                          |
| H-R031 | L198, design-only                 | keep schema, errors, receipts, failures in coverage                | contributes: sections 7, 8, 9                                           |
| H-R034 | L201, design-only (history)       | verify task/session binding and durable handoff                    | addresses binding verification: 4.2                                     |
| H-R041 | L208, design-only                 | bind canonical task and launch authorized session                  | contributes: lookup only; launch DSP-06                                 |
| H-R042 | L209, design-only                 | verify prompt delivery, session identity, ownership/lease          | addresses for messages: 4.2 steps 3-9, 5.2, 6                           |
| H-R043 | L210, design-only                 | label, queue position, bare shell, pending outbox is not a receipt | addresses: 3.3, 7.2                                                     |
| H-R044 | L211, design-only                 | reuse a compatible active session, no duplicate claim              | addresses: section 10 `agent.slash.resume`, 4.4                         |
| H-R054 | L221, duplicate of H-R041         | as H-R041                                                          | as H-R041                                                               |
| H-R055 | L222, duplicate of H-R042         | as H-R042                                                          | as H-R042                                                               |
| H-R057 | L224, duplicate of H-R044         | as H-R044                                                          | as H-R044                                                               |
| O-R011 | L241, design-only candidate       | do-now routes to live session or dispatches, binds, acknowledges   | contributes: live-session half (5); directive DSP-08                    |
| O-R014 | L244, design-only candidate       | honest adapter delivery/ack, replay, audit, visible failure        | addresses: 5, 6, 7, 9                                                   |
| O-R021 | L251, design-only candidate       | coordinate discovery through completion                            | contributes: binding view                                               |
| O-R022 | L252, design-only candidate       | separate broker, adapters, direct-dispatch responsibilities        | addresses: 2.3 (no new service), 6 (adapter boundary)                   |
| O-R023 | L253, design-only candidate       | preserve CLI families, no parallel dispatch mechanism              | contributes: section 10                                                 |
| O-R026 | L256, design-only candidate       | stable Fleet identity, qualified external links                    | contributes: `work` pair in 4.1 and 5.1                                 |
| O-R033 | L263, design-only candidate       | lifecycle distinct from ownership; park preserves provenance       | contributes: 4.2 step 3 (history kept, not a target)                    |
| O-R040 | L270, design-only history         | preserve two-track history, current dispatch authority             | contributes: 2.1 uses current authority                                 |
| O-R047 | L277, design-only history         | bind issue/task/session, verify identity, report failure honestly  | addresses for messages: 4, 7                                            |
| O-R049 | L279, design-only example         | ordinary-worker walkthrough                                        | contributes: binding view                                               |
| O-R050 | L280, design-only example         | resume walkthrough with new activity                               | contributes: sends after resume target the new binding (5.4)            |
| O-R051 | L281, design-only example         | live-instruction walkthrough: route, record delivery, no queueing  | addresses: sections 1, 4, 5                                             |
| O-R052 | L282, design-only example         | near-timeout walkthrough                                           | contributes: 9, row 2                                                   |
| O-R063 | L293, design-only candidate       | comments on active work to bound session with delivery/ack         | addresses: 4, 5, 7                                                      |
| O-R064 | L294, design-only candidate       | capability-aware adapters; Slack post is not receipt               | addresses: 3.2, 6                                                       |
| O-R075 | L305, duplicate of O-R063         | as O-R063                                                          | as O-R063                                                               |
| O-R076 | L306, duplicate of O-R064         | as O-R064                                                          | as O-R064                                                               |
| P-R002 | L309, design-only candidate       | Fleet coordinates without replacing tools                          | contributes: 2.3 reuses existing adapter                                |
| P-R009 | L316, design-only candidate       | cohesive CLI incl. agent/session operations                        | contributes: section 10                                                 |
| P-R012 | L319, design-only candidate       | session flags, idempotency, wait, timeout                          | addresses: 4.1, 5.1, 5.4                                                |
| P-R013 | L320, design-only candidate       | work identity, ownership/lease fields                              | contributes: 3.1 (derived, no new field)                                |
| P-R027 | L334, design-only candidate       | realistic end-to-end acceptance scenarios                          | contributes: section 14                                                 |
| P-R035 | L342, design-only example         | worker walkthrough                                                 | contributes: binding view                                               |
| P-R036 | L343, design-only example         | resume/finish walkthrough with new feedback                        | contributes: 5.4                                                        |
| S-R004 | L381, partially implemented       | route comments to a live session or start one, not merely queue    | addresses live-session half: 4, 5; start is DSP-06                      |

Issue negative coverage (`DSP-05.md:106`): stale input (tests 3, 6),
duplicate/replay (12, 13), ambiguous resource (5; DSP-04's
`ambiguous_target`), unauthorized actor (14), partial remote effect
(10, 11; `partial` in 7.2), unknown version (shared envelope validation,
`contracts.rs:900`), no false verified result (8, 9, 11).

## 16. Out of scope

- Implementation of any of the above; gated per section 13.
- Direct launch, the initial prompt, and first bind (DSP-06); the
  launch receipt and #6533 live fixture (DSP-07); directives and
  owner authority (DSP-08); crash-cut MBT across boundaries (DSP-09);
  deployment, kill switch and dashboards (DSP-10).
- Item resolution and the routing table (DSP-04); grammar and
  spelling (CLI-02); registration (CLI-13).
- Operation identity, digest and stable receipt storage (REL-03);
  GitHub comment writes and their read-back (REL-04 to REL-06).
- Activity attribution after a send (HYD-07).
- Any new provider adapter, Slack or Muse integration, scheduler,
  queue, database, resolver or service.
- Clearing, stealing or stopping a binding or session; takeover stays
  with #9178.
- Changing `valid_session_id` in either crate (Q7 is a question, not a
  decision).
