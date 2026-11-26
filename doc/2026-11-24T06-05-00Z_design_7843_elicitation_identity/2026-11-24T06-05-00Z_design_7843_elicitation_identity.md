# #7843 — consolidated 9-point design record: elicitation binds no user identity (Part of #7843)

- Date: 2026-11-24
- Source issue: developing-today/code#7843 (open, `status/triaged`; split of #1957 per task 13636 packet rule: design before implementation)
- Task: repo-health-code-pilot 21675 (design slice, 45-min timebox)
- Decided design: `pkgs/mcpx/docs/decisions/0007-elicitation-identity.md` (accepted 2026-11-17, PR #11199) — refuse-identity
- Implementation follow-on: #11198 (closed `status:done` 2026-11-18)
- 9-point standard: `doc/2026-11-24T00-24-54Z_audit_design-gaps/2026-11-24T00-24-54Z_audit_design-gaps.md` § "The sufficient-design standard"

This record does not re-decide anything. Decision 0007 is accepted and its implementation issue is closed. This file consolidates 0007 + #7843 + #11198 into the 9-point shape so the audit gap is closed by pointer, and surfaces the one loose end found while writing it (matrix rows still reading `gap` on main, §8 Q1).

## 1. Problem + context

Conformance-matrix rows `elicitation-server-state-bound-to-user` (2025-11-25, MUST) and `elicitation-server-no-trust-client-user-id` (2026-07-28, MUST NOT) were both marked gap (#257): mcpx had no authenticated user; elicitation state was bound to the request while the session id was whatever the caller sent (`internal/elicit/elicit.go` `Session` string stored verbatim in `store.go`; no user field anywhere in schema; grep-verified for 0007). Task 13636's packet rule forbade ad-hoc implementation before the identity decision landed. Evidence: 0007 §Context; matrix rows at `pkgs/mcpx/docs/spec/conformance-matrix.md:1330,1358,2028`.

## 2. Goals / non-goals

Goal: decide, as a recorded rule, what identity (if any) elicitation/sampling state binds to and what client-supplied values must never be treated as — then pin it in code comments, audit answer paths, and conformance tests.
Non-goals: (a) no new identity layer — no accounts, tokens, or IdP binding for one feature (0007 option A, rejected); (b) no implementation code changes under #7843 itself — the decision describes what the code already does, plus guard work in #11198; (c) this record adds no runtime behaviour.

## 3. Alternatives considered

1. Do nothing (leave the gap rows un-dispositioned): rejected — the MUST/MUST NOT rows' threat (a server trusting a client-asserted user id) stays unanswered, and task 13636 blocks all elicitation follow-ons until decided.
2. Authenticated identity (0007 option A): grow accounts/tokens/IdP so state can bind to a user. Rejected — invents a principal the rest of mcpx does not have; every other subsystem authorizes by daemon access, so an elicitation-only identity would be a second, weaker authorization model, and bolting on assertions does not answer the spec rows' threat.
3. Refuse identity (0007 option B, taken): keep exactly the principals that exist — the request and the daemon owner — and record the absence of a user as a designed property with rules.

## 4. Chosen approach

Decision 0007's four rules (authoritative text in the decision record): (1) no user identity in mcpx — state binds to the elicitation record (id, trace, parent task) and the daemon principal (OS owner of daemon, socket, owner-only store); this is the conformance position for `elicitation-server-state-bound-to-user`. (2) Client-supplied identifiers (`Session`, any user-id-shaped value) are correlation labels only — stored/logged/searchable, never identity or authorization inputs; conformance position for `elicitation-server-no-trust-client-user-id` and `elicitation-state-not-session-id-alone`. (3) Answering authority is daemon access (shipped broker model, `docs/elicitation.md` §5.2). (4) Future identity enters at the transport (socket peer credentials / authenticated bearer) under a superseding record, never from MCP message fields.

## 5. Dependency-ordered slices with acceptance

Single-threaded by nature (decision first, guards second); each sized for one lane/session:

1. Decision record 0007 — DONE (PR #11199, accepted). Acceptance: four matrix rows dispositioned to the decision; `docs/elicitation.md` broker-model pointer; decisions README + compendium manifest entries (TestEveryDocumentHasAChapter passes).
2. Guard work #11198 — DONE/CLOSED (`status:done`). Acceptance was its four boxes: `Session`/`session` documented correlation-only; answer-path audit (clean or findings fixed) recorded; conformance tests for the two MUST NOT rows in elicit/daemon suites passing; `elicitation.md` identity pointer added.

## 6. Rollout + rollback

Rollout: decision-first (0007 merged before any guard code), guards as normal small-fix PRs. Rollback: both changes are docs/comments/tests-only — revert is safe and restores the prior gap-marked state; no data migration, no config change, no multi-principal state to unwind. Downgrade path needs none beyond revert.

## 7. Verification plan

Per #11198 acceptance: conformance tests `elicit.TestAClientSuppliedSessionGrantsNoAuthority@2025-11-25,2026-07-28` (matrix rows 1358/2028 now read `tested`) and `elicit.TestQuestionsSharingASessionStayAddressableById`-class coverage for `elicitation-state-not-session-id-alone` (row 1331 reads `tested`); answer-path audit result recorded on #11198. Oracle throughout: the MCP spec sections the rows cite (client/elicitation statefulness; identifying-the-user).

## 8. Open questions (named deciders)

1. Matrix rows 1330 (`elicitation-server-state-bound-to-user`, MUST) and 1359/2029 (`elicitation-server-follow-best-practices`, SHOULD) still read `**gap** #257/#255` on origin/main despite 0007's consequence clause ("re-dispositioned from gap to this decision in the same PR"). Is the remaining `gap` text intentional (accepted residual: no authenticated user to bind to) with wording to fix, or did the re-disposition miss rows? Decider: Drew, with matrix owner. (This record's worktree is docs-only by task scope — `owned_paths: doc/` — so the fix, if any, is a follow-on, not this PR.)
2. If mcpx ever runs multi-user or remote, transport-level identity per 0007 rule 4 requires a superseding decision record. Decider: Drew (reserved product call; cf. #13264 standing constraints #8809/#8810).
3. Whether #7843 itself should now close (its acceptance — design landed, rows re-dispositioned, implementation issues filed/closed — reads met modulo Q1). Decider: Drew / issue owner.

## 9. Record + tracking + queue

- Record: this file (dated `doc/`, docs-only change, `Part of #7843`).
- Design depth home: decision 0007 (accepted); broker model: `pkgs/mcpx/docs/elicitation.md` §5.2; matrix: `pkgs/mcpx/docs/spec/conformance-matrix.md`.
- Tracking: #7843 stays open pending §8 Q1–Q3 (per its acceptance structure: design first, implementation second).
- Queue: task 21675 completes on merge of this record's PR; no new implementation slices — #11198 is closed.
