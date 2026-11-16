# FDR-PMO-006 — Transactional outbox and direct verified completion

- **Date:** 2026-11-13
- **Status:** accepted — local design contract only, not production authorization
- **Adopted:** Summit owner, task 15381, 2026-11-13; see `../reviews/component-contracts.md`. The bounded creation-outbox pilot is not expanded.
- **Owner:** Summit design owner, task 15381 / session `ses_e5e2d10e06effeEylSDDVaN0PL` for this proposal; future maintenance by Fleet maintainers through #9180.
- **Decision maker:** Delegated Summit architecture owner for this design pass; adoption follows independent final design review by the parent and Fleet owner recording through #9180. No human name is inferred.
- **Scope:** local atomic accepted intent, remote operation delivery/readback, partial recovery, completion finalization
- **Supersedes:** none
- **Superseded by:** none
- **Related records:** FDR-PMO-001, -003, -004, -005, -007

## Context and problem

Fleet state and GitHub/Project/session effects cannot share a transaction. The existing creation outbox is a bounded one-producer pilot, not a general lifecycle/mutation outbox. The charter requires remote merge/closure verification and direct queue completion without waiting for webhook/sweep, while recovering crashes, timeouts, partial effects, stale workers, duplicate delivery, and late activity. GitHub does not accept Fleet fences universally; retries cannot claim remote exactly-once.

Sources: [architecture, “Shared command and result interface” and “Lifecycle and cross-system reliability”](../architecture.md); [source 02 §§13–16](../sources/02_charter_v1_1.md); [Track B census, “Finding” and “Compatibility and ownership”](../census/track-b.md); [#9209, “Outbox operation model,” “Direct completion protocol and receipt,” and “Failure matrix”](../components/9209-leases-fencing-outbox-direct-completion.md); [#9207, “Incremental activity and cursor correctness”](../components/9207-context-hydration-incremental-activity.md); [#9208, “Field authority and external synchronization”](../components/9208-typed-dependencies-notes-journal-field-authority.md); [prior art B, “Transactional outbox and completion” and “Failure matrix”](../research/track-b-state-of-art.md).

## Options considered

1. **Local transactional intent + effect-specific adapter + remote observation/reconciliation + direct local finalization (recommended).** Atomic local durability, honest remote uncertainty, immediate normal success.
2. **Dual-write Fleet and GitHub.** Apparent simplicity but a crash between writes loses track of one side; no shared transaction exists.
3. **Webhook-only close/reconciliation.** Useful trigger/recovery but duplicate, delayed, or absent delivery cannot satisfy direct successful completion.
4. **Expand existing creation outbox in place without scope review.** Reuses code superficially but wrongly assumes generic transactional semantics and adds unrelated producer risk.
5. **New workflow service.** Durable replay can help, but remote effects still need idempotency/reconciliation and service adoption adds complexity absent measured need.
6. **Do nothing.** Leaves accepted mutations vulnerable to lost intent and normal completion dependent on eventual external notification.

**Tradeoff comparison:** A local outbox is modest implementation complexity with strong local durability; throughput is bounded by existing store/remote API and should be measured. Dual-write has lower apparent latency but poor crash reliability. Webhook-only scales reads cheaply but weakens completion latency and liveness. A workflow service offers retry/visibility extensibility at material platform and operational cost while not improving remote atomicity. The existing pilot is cheapest only if its narrower semantics are retained; expanding it without design is a hidden reliability cost.

## Proposed operational decision

For each accepted consequential mutation, commit canonical state change, journal fact, and required immutable operation intent in one local transaction, after the existing store's transaction semantics are verified. Never hold the local transaction over HTTP. Use stable operation/idempotency identity and request digest; preserve attempt history and per-effect requested/observed/verified states. Serialize operations by target where causal order requires it. A relay fence protects local relay state only; it does not fence a remote API.

Before delayed send, revalidate revocable permission and effect-specific preconditions. Revocation prevents future sends; it does not cancel an already-sent request. On timeout after send, status is unknown until readback; do not blind-repeat non-idempotent effects. If a stale worker's authorized remote request later succeeds, an authenticated broker recorder may append independently verified evidence to that exact operation; it cannot create a new operation or alter current lifecycle/owner state.

Direct completion explicitly names authorized PR/issue effects and required-vs-optional policy. Prepare a recoverable `completing` operation; refresh activity and preconditions; merge using expected head SHA where supported; distinguish queued/accepted from merged; verify merge and designated issue closures; then transactionally finalize canonical work and receipts. No webhook/sweep is needed on the normal verified success path. Partial success remains recoverable; never undo a successful merge to simulate rollback.

Completion reports final refresh watermark and residual read→merge race. Changed expected head, required checks/reviews/branch policy failure, newly unresolved hard blocker, authorization revocation, or explicit hold/cancel stops further consequential effects. An authorized actionable comment/review observed before finalization holds for policy evaluation; informational activity need not. Post-completion activity appends evidence and invokes explicit triage/reopen policy; history is not rewritten.

## Rationale and evidence boundary

**Facts:** census confirms existing queue/outbox primitives but explicitly does not establish general lifecycle outbox or composed completion; creation outbox has one producer. Research supports at-least-once relay, remote API uncertainty, merge SHA precondition, asynchronous merge state, and lack of a cross-system transaction. These do not prove current deployment behavior.

**Inference:** outbox plus direct verify gives the best balance of durable intent and synchronous normal-success semantics without new infrastructure. “Effectively once” is scoped to operations with recognizable state/idempotency and retention; never universal exactly-once.

## Consequences, compatibility, and migration

- General mutation outbox is its own scoped implementation issue and must not enroll as another creation-outbox producer by implication.
- Verify actual DB locking/isolation/clock and migration ownership; QTI/#3424/#8575/#8579/#9241 schema gate applies. Keep pending intents and remote receipts through rollback; disabling producer cannot erase external effects.
- Remote operation support is effect-specific. GitHub close/readback may not prove actor attribution; report confidence. Comments/body writes without remote idempotency/CAS remain uncertain/best-effort under FDR-PMO-005.
- Direct completion can finish an already-authorized operation after lease expiry only via restricted operation-scoped path and current lifecycle/version checks; it cannot authorize unrelated writes.
- No new scheduler, service, deployment, remote mutation, exact-once guarantee, or migration authorized.

## Negative cases and test oracles

- **Crash-cut MBT:** crash around every local commit/network send/response/readback/finalize boundary; pending operation remains inspectable and converges or stays explicit unknown/conflict.
- **Atomicity oracle:** accepted local state without required durable intent is impossible; intent without committed corresponding state is impossible under the same local transaction.
- **Lost-ack oracle:** fake remote applies then times out; readback confirms exact effect without duplicate POST; unresolved result stays unknown.
- **Completion oracle:** accepted/enqueued merge is not merged; closed PR is not merged; all required effects must be verified and Fleet finalization committed before completed.
- **Partial oracle:** merge success + issue-close failure preserves merge receipt and only retries authorized close; no auto-reopen.
- **Late activity oracle:** interleave requirement comment before finalization → hold; after completion → append and apply explicit policy; never claim global lock.
- **Authorization/fence oracle:** revoked actor blocks future send; in-flight remote effect may be recorded only against immutable operation evidence and cannot mutate new owner state.

## Revisit triggers

Reopen if verified storage cannot atomically commit state+journal+intent, target API adds/removes conditional/idempotent guarantees, completion policy changes, observed pending-operation volume exceeds existing operator capacity, or live tests show unacceptable remote uncertainty. New workflow infrastructure needs measured reliability/scale justification.

## Implementation, research, and source links

- Existing: [#9209](https://github.com/developing-today/code/issues/9209), [#9207](https://github.com/developing-today/code/issues/9207), [#9208](https://github.com/developing-today/code/issues/9208), [#9198](https://github.com/developing-today/code/issues/9198), [#9241](https://github.com/developing-today/code/issues/9241), [#6533](https://github.com/developing-today/code/issues/6533) (separate live dispatch proof, not completion proof).
- Existing granular component keys: #9209 `REL-03`–`REL-11`; #9207 `HYD-08` and `HYD-09`; #9208 `AUTH-02`–`AUTH-04`; #9206 `CLI-10`. No third backlog is created here.
- Pinned baseline `a038f084dbeb525494d7bb928ad91f597db2ac51`; see `census/track-b.md`, `doc/fleet/creation-outbox.md`, and #9209.
