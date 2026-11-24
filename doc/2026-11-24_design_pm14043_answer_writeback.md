# Design record: pm-stack push answer write-back to the queue broker (#14043)

- **Date:** 2026-11-24
- **Status:** Design record (queue task 21648, slice 1 of #14043). Implements nothing. Unmerged; Archer merges from the #14043 record.
- **Issue:** [#14043](https://github.com/developing-today/code/issues/14043)
- **Reads:** [#13774](https://github.com/developing-today/code/issues/13774) (endpoint list, tiered extension policy), [#13902](https://github.com/developing-today/code/issues/13902) (alerts throat 401), [#13469](https://github.com/developing-today/code/issues/13469) (edge answer intake), [#8569](https://github.com/developing-today/code/issues/8569), and `doc/2026-11-17_design_pm8569_notifications_phase2.md`.
- **Code read for this record:** `pm-stack/aggregator/src/needs-drew.ts` (`recordAnswer`, `listAnswers`, the answer endpoint and the feed `answers` publication), `pm-stack/aggregator/src/decisions.ts` (resolution store, `listDecisionResolutions`), `pkgs/fleet/crates/fleet-queue/src/needs_drew.rs` (`build_extension_alert_body`), `pkgs/fleet/crates/fleet-queue/src/client/actions_exec.rs` (`ae_extend_decide`), and `pkgs/fleet/crates/fleet-queue/src/turso_queue/actions_exec.rs` (`verb_actions_exec_extend_decide`).

## Summary

Today a phone tap on a broker-raised extension push lands in the PM ledger, and nothing applies it. A seat notices and runs `/actions-exec-extend-decide` by hand. This design adds an **applier** that carries a recorded answer for a `fleet-actions-exec` alert to the broker, once, with failures raised as alerts. The ledger stays the only place of record for the answer. The broker's decide verb keeps its meaning and gains one new, narrowly scoped way to authenticate.

| Seam                           | Chosen option                                                                                                                                  | Rejected                                                                           |
| ------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------- |
| 1. Applier placement           | **Aggregator-side applier** that runs when an answer is recorded for a broker-sourced alert                                                    | Broker-side consumer reading the aggregator's exported feed                        |
| 2. Applier-to-broker authority | **Aggregator signs the decide envelope** with a secret held by name; the broker accepts a signed decide only for `/actions-exec-extend-decide` | Reusing seat auth (lets any seat decide without Drew's answer); trusting localhost |
| 3. Exactly-once and races      | **Applier ledger keyed by `answerId`**, plus the broker's existing settled-state outcomes read as terminal                                     | Applying on every feed read; relying on the broker alone to dedupe                 |
| 4. Failure behaviour           | **Each failed application raises its own alert**, with retry until terminal                                                                    | Logging the failure in the broker only (the #13902 pattern)                        |

## 1. Applier placement (decided: aggregator-side)

The aggregator already owns the tap. `recordAnswer` in `needs-drew.ts` is the first-answer-wins write, and `listAnswers` is the read. The answer's per-question token is authority only at the aggregator, so the aggregator is the only process that can prove a tap was genuine.

Placing the applier aggregator-side means:

- The broker's decide verb does not learn about PM schemas. It keeps receiving `{handle, approve}` and gains only a way to check who sent it.
- The applier reads one store. Answers from the laptop tap and answers drained from the edge (#13469) both reach `recordAnswer`, so the applier sees one stream.
- There is no second reader of the feed. Feed publication (`collect-fleet-feed.ts`, the `answers` array) stays for the fleet lane's resume; the applier does not depend on it.

The rejected broker-side option would have the broker parse the aggregator's exported answers. That makes the broker a second consumer of the PM record, couples it to the ledger's shape, and gives it a reason to read answers it cannot authenticate.

**Mapping.** An answer must reach its broker handle. `build_extension_alert_body` sets `source: "fleet-actions-exec"` and `key: actions-exec-ext:<handle>` on the push, and the needs-drew question carries the ledger id. The build slice must record `ledgerId → {source, key, handle}` when the raise is stored, so the applier can tell a broker-sourced answer from any other needs-drew answer. Answers without that mapping are left alone and are never applied. This mapping is the one fact the build must verify first (see slice S2).

## 2. Applier-to-broker authority (decided: signed envelope)

The decide route is seat-shaped today. A seat acting on Drew's recorded answer runs the verb. Any caller that can reach the route can therefore decide, and the route cannot tell Drew's answer from anyone else's.

The applier is the one party that holds a recorded answer. So the decide body carries an envelope the aggregator signs:

```json
{
  "handle": "<session handle>",
  "approve": true,
  "answer_id": "<ledger answer id>",
  "answered_at": "<RFC 3339>",
  "sig": "<HMAC-SHA256 over handle|approve|answer_id|answered_at>"
}
```

- The HMAC key is a secret by name (for example `pm-aggregator.actions-exec-apply-key`), in Bitwarden Secrets, read by the aggregator and by the broker through fleet-secrets. The value appears in neither the repo nor the record.
- The broker accepts the signed form only on `/actions-exec-extend-decide`. A signature proves the call came from the aggregator's applier, and the applier only signs a recorded answer. Seat calls without `sig` keep their current behaviour.
- The broker checks `answered_at` against a bounded window, which limits replay of an old envelope.

Rejected: a dedicated applier seat with its own seat credential. It would authenticate the process but not the answer, so a compromised or confused applier could still decide with no answer behind it. Rejected: trusting the loopback connection. The throat is localhost-only, so loopback proves only where a call came from, not what it carries.

## 3. Exactly-once application and races (decided)

**Idempotency key.** The applier's ledger records each `answer_id` with one of: `pending`, `applied`, `superseded`, `rejected`, `failed`. An `answer_id` reaches the broker at most once in the `applied` state. Retries of a `pending` or `failed` answer reuse the same `answer_id`, so the broker can recognise a repeat.

**Broker outcomes, read as terminal.** The broker's existing decide verb already answers each race:

| Broker state at arrival                | Verb outcome today                                                                         | Applier state                                                |
| -------------------------------------- | ------------------------------------------------------------------------------------------ | ------------------------------------------------------------ |
| Pending extension request              | Approves or denies; `decided: true`                                                        | `applied`                                                    |
| Tier-2 window already silence-approved | `decided: false`, `approved: true`, `outcome: silence-approved`                            | `superseded` (raise a note, do not re-decide)                |
| Session settled, or no pending request | fail code 5 "no extension request is pending"                                              | `superseded` if the session is settled; `rejected` otherwise |
| Expired decision grace                 | `require_session_settled` settles the session first; the late deny does not read as landed | `superseded`, with the note                                  |

Only the first two rows are successes, and the applier treats them as final. A late tap after silence approval is not an error: it is recorded and raised as an informational note, so Drew can see his answer did not land.

**Double taps and retries.** Intake already dedupes double-taps by answer id (`recordAnswer` returns current state on repeats). The applier adds the second guard: a second answer for the same handle with a different `answer_id` is recorded as `superseded` and never sent.

## 4. Failure behaviour (decided: visible alert, never log-only)

The lesson from #13902 is that a failed raise was only a broker log line, so Drew saw nothing. A failed application is therefore an alert of its own:

- Source `fleet-actions-exec`, key `actions-exec-apply:<answer_id>`, severity high. It names the handle, the answer, and the broker's error. It is deduped by key, like the other throat alerts.
- The applier retries with bounded backoff while the failure is transient (broker unreachable, throat unreachable). A transient failure stays `failed` until a retry succeeds or a terminal state is reached. A rejected signature, or a settled session that is not a supersede, is terminal at once.
- If the alerts throat is itself down (today's #13902 state), the applier keeps the failure in its own ledger and re-raises it when the throat answers. The failure is then visible in the board feed as well as the push, so it is never only in a log.

## 5. Composition with #13469 (one answer path)

Answers that arrive while the laptop is down come through the edge answer intake of #13469. Its drain goes into the same needs-drew answer store through the Phase-1 catcher, which stays the single writer. The applier reads only that store. So there is one place of record and one applier, whichever transport the answer came in on. No second answer path is created, and #13469's scope is unchanged: it still excludes application.

## 6. Build slices (dependency order, on #14043)

Each slice names its scope and a done check that a command can run.

- **S1 — broker: signed decide envelope.** Owned: `pkgs/fleet/crates/fleet-queue/src/turso_queue/actions_exec.rs`, `pkgs/fleet/crates/fleet-queue/src/client/actions_exec.rs`. Accept `sig` on `/actions-exec-extend-decide`, verify against the named key, enforce the `answered_at` window. Done check: `cargo test -p fleet-queue` covers a valid signature, a bad signature, an expired `answered_at`, and an unsigned seat call that still works.
- **S2 — aggregator: answer-to-handle mapping.** Owned: `pm-stack/aggregator/src/needs-drew.ts`, `pm-stack/aggregator/src/alerts.ts`. At raise, store `ledgerId → {source, key, handle}` for `fleet-actions-exec` questions. First step of the build: verify the raise stores the key, since this design infers it from the push body. Done check: unit test shows an answered broker-sourced question yields its handle, and a non-broker question yields none.
- **S3 — aggregator: applier loop.** Owned: new `pm-stack/aggregator/src/actions-exec-apply.ts`, plus its wiring in the aggregator. Reads answers, signs, calls the decide verb, runs the `answer_id` state machine. Done check: tests for each row of the race table in §3, and a double-tap test showing one decide call.
- **S4 — applier failure alerts and retry.** Owned: `actions-exec-apply.ts`, `alerts.ts`. Raises `actions-exec-apply:<answer_id>` on failure, retries transient failures, re-raises when the throat returns. Done check: a forced-broker-failure test shows one alert, and a forced-throat-failure test shows the alert on recovery.
- **S5 — live proof.** Owned: a proof script and record under `doc/`. Depends on S1–S4 and on #13902 landing. Done check: a tier-3 extension on a live Actions exec session, answered by tap, shows `ext_state` approved and `max_until` moved, with the applier's ledger row at `applied`.

S1 and S2 are independent and can run in parallel. S3 needs both. S4 needs S3. S5 is the last and cannot pass until #13902 is fixed.

## 7. Live-proof plan (ACTIONS-EXEC-3 proof shape)

The proof follows the ACTIONS-EXEC-3 shape already used on #13774: a live exec session, a tier-3 extension request, a push, a tap, and a readback of the broker session row. The new readback is the applier ledger row at `applied`, plus one decide call in the broker log for that `answer_id`.

**Dependency.** Until #13902 lands, the push is dropped at the throat (401) and no tap can happen. The design does not depend on #13902; the live proof does. S1–S4 can be built and unit-tested without it.

## Invariants

1. **One place of record:** the needs-drew answer store is the only record of Drew's answer. The applier keeps its own state about application, not about the answer.
2. **Only a recorded answer is sent:** the broker accepts a signed decide only for an answer the aggregator recorded.
3. **Nothing applies twice:** `answer_id` is the key; a second answer for the same handle is superseded.
4. **No silent failure:** every failed application is an alert, and it is re-raised if the alerts throat was down.
5. **Secrets by name:** the signing key lives in Bitwarden Secrets and is read by name, never written into code, docs or the ledger.

## Open items recorded (not decisions for this slice)

- The raise path's stored key for `fleet-actions-exec` questions is inferred from the push body. S2 verifies it before S3 relies on it.
- The signing-key secret name is proposed here; the owner of the Bitwarden entry is recorded on the build slice, not decided in this design.
