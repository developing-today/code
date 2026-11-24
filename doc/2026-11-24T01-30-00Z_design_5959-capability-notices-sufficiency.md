# Sufficiency record: capability-change notices on reconnect (#5959)

- **Date:** 2026-11-24
- **Source issue:** [#5959](https://github.com/developing-today/code/issues/5959) (worlds program #5955)
- **Design of record:** `doc/2026-11-23T05-02-30Z_design_id_capability_change_notices/` (PR [#12920](https://github.com/developing-today/code/pull/12920), on main)
- **Queue:** task 21555, `repo-health-code-pilot`, AUDIT-7 Lane 6 (tracking [#13873](https://github.com/developing-today/code/issues/13873))
- **Lane:** oc-code-08
- **Kind:** design-only gap fill. No implementation. The design of record is cited, not rewritten.

This record judges #5959 against the nine-point sufficient-design standard. The design of record covers points 1, 3, 4 and parts of 7 and 8. The gaps below are filled here.

## 1. Problem and context

The design of record already states the problem: a world a user has connected to before can gain capabilities, by expansion or by sideways change, and the user must be warned before connecting. Evidence in the design of record: the comparison contract in #5999 §6, the registry schema in #5958 §4, and the permission-UX contract in #5960 §2.

What breaks today is that a known world can change its capability footprint silently. #5959 names this directly ("no silent upgrades of what a known world can do"). Implementation #12921 is open and waiting on this design's notice contract, so the gap blocks a shipped feature.

## 2. Goals and explicit non-goals

**Goals**
- G1. A notice fires before any capability is exercised when the new release's class set is not covered by the user's accepted baseline (design of record §2, §5).
- G2. Decreases never prompt; the baseline narrows automatically (§5).
- G3. An uncomputable comparison fails closed to an `unknown` notice, never to silent proceed (§3.2).
- G4. Invites may pin the baseline as issued, so an expansion after a pinned join prompts like a reconnect (§7).

**Non-goals**
- N1. Re-deriving comparison semantics. #5999 §6 is consumed unchanged.
- N2. Re-deriving the registry schema. #5958 is consumed unchanged.
- N3. Grant-on-use policy changes. The notice carries an honesty line and does not cover grant-on-use (§3.3).
- N4. Notifying end users about used-set growth. That is an author declaration defect (§6).

## 3. Alternatives, including do-nothing

- **A0. Do nothing.** Known worlds gain capabilities silently. This is the defect #5959 exists to prevent, so it is rejected.
- **A1. Prompt on every change, decreases included.** Rejected in the design of record §9: fatigue, and it contradicts #5999 rule 2.
- **A2. Whole-set accept, no per-class denial.** Rejected in §9: it breaks the #5960 §2 per-capability denial contract.
- **A3. Chosen: prompt on uncovered expansion only, per-class denial, narrowing on decrease, optional invite pin** (design of record §§2–7).

## 4. Chosen approach

Pointers only; the design of record is authoritative.
- Baseline store per (user, world), keyed by release identity (§1).
- Connect-stage comparison between release resolution and connection completion (§2).
- Three choices with no default button: connect once, connect and remember, don't connect (§4).
- Re-baselining only by explicit choice, first connect, invite pin, automatic narrowing, or grant-page edit (§5).

## 5. Dependency-ordered build slices

Each slice fits one lane or session and has its own acceptance criteria.

1. **S0, prerequisite (gate): the `class` field on `ProviderManifest` and the reserved-class table** (#5999 §10). Acceptance: every registered capability name maps to a class; unmapped names map to `unknown`, which fails closed.
2. **S1, baseline store.** Per-user, per-world baseline keyed by release identity, with history, stored beside #5960 remembered grants. Acceptance: round-trip test; history records accept, narrow, pin, grant-page edit and YOLO auto-accept; no stale baseline after YOLO auto-accept.
3. **S2, pure comparison wiring.** Call #5999 §6 functions over `(B, N, ≼)`. Acceptance: property tests for #5999 §7 invariants 1–5, 8, 9 pass.
4. **S3, connect-stage gate.** Covered proceeds with narrowing bookkeeping; uncovered holds before any capability is exercised; uncomputable yields `unknown`. Acceptance: prompt iff trigger; no capability exercised before the decision; fail-closed prompt on an uncomputable comparison.
5. **S4, notice surface.** Delta-only headline, client-side and server-side groups separate, per-class denial composing per #5960 §2, grant-on-use line where applicable. Acceptance: per-class denial removes exactly that class from the remembered set; re-addition after narrowing re-prompts.
6. **S5, invite pin** (optional, §7). Acceptance: a pinned join with an uncovered expansion prompts; an unfetchable pinned record degrades gracefully and never hard-fails the public path.

S0 gates S2. S1 and S2 can run in parallel. S3 needs S1 and S2. S4 needs S3. S5 needs S3.

## 6. Rollout and rollback

- **Rollout:** ship S1 and S2 dark (no UI, no gating). Enable S3 for new connections first, then for known worlds. Enable S4 with S3. Enable S5 last, as an optional field.
- **Rollback:** disable S3 to return to the pre-notice connect flow. Stored baselines stay in place and are inert. S4 can be disabled independently. No stored state is rewritten in either direction.
- **Safety under rollback:** while S3 is off, the connect flow must not treat a missing decision as acceptance. Known worlds keep their last recorded baseline.

## 7. Verification plan

- **Oracles:** the reference comparison in #5999 §6, and the state machine of §§4–5 of the design of record. The oracle for each test is named.
- **Property tests (S2):** #5999 §7 invariants 1–5, 8, 9.
- **Model/state tests (S1, S3):** generated sequences of connect, release, decrease, expand, pin and remember; expected outcome from the §5 re-baselining rule.
- **Live proof:** on a test world, expand its declared set, reconnect, and confirm the notice fires before the first capability call, and a decrease narrows silently.

## 8. Open questions with named deciders

1. Final notice wording. **Decider:** #5960 wording review. **Default:** client class table text as written in the design of record §3.1.
2. Whether pinned invites are offered by default. **Decider:** #5751 owner. **Default:** optional, off unless the inviter opts in (§7 of the design of record).
3. Whether server admins see the new server-side class prompt or only a grant-log entry. **Decider:** #5960 §3 mode owner. **Default:** admin decides at serve time under the §3 modes; users see disclosure only.
4. Whether `unknown` notices may be dismissed permanently. **Decider:** owner, Drew. **Default:** not dismissible; each connect shows it until the class table maps the name.

None of these blocks S0 or S1.

## 9. Record and tracking

- Design of record: PR #12920, on main.
- This record: the gap fill, linked from #5959 by an agent note.
- Implementation: #12921 is open and is the vehicle for S1–S4. S5 files as its own issue against the #5751 invite terms.
- Queue task 21555 completes on merge of this record's PR.
