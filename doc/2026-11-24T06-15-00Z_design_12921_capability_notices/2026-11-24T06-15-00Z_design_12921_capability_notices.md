# #12921 — consolidated 9-point design record: capability-change notices (Part of #12921)

- Date: 2026-11-24
- Source issue: developing-today/code#12921 (open, `status:in-review`, `app/id`, `epic/id-program`)
- Task: repo-health-code-pilot 21679 (design slice, 45-min timebox)
- Design of record: #5959 (`doc/2026-11-23T05-02-30Z_design_id_capability_change_notices/`, PR #12920) §§1–4
- Contracts consumed: comparison semantics #5999 §6 (`doc/2026-11-08T10-12-00Z_design_id_capability_diff_semantics/`, CLOSED) + property invariants §7; registry schema #5958 §4 (CLOSED); remembered-grants composition #5960 §2 (CLOSED)
- Parent programme: #5955 (worlds design program)
- 9-point standard: `doc/2026-11-24T00-24-54Z_audit_design-gaps/2026-11-24T00-24-54Z_audit_design-gaps.md` § "The sufficient-design standard"

This record does not re-decide the #5959 design. It consolidates #12921 + #5959 + #5999/#5958/#5960 into the 9-point shape, with slices mapped to landed vs open work.

## 1. Problem + context

A world's capabilities can change between releases (shrink, grow, re-add). A client connecting with a stale baseline would silently exercise a different capability set than it accepted. #12921 implements the #5959 notice: a connect-stage comparison of the remembered baseline `B` against the newly resolved set `N` that holds the connection before any capability is exercised when the change is uncovered, and presents a delta-only notice with explicit choices. Prior art on main: comparison contract + invariants (#5999, closed), registry schema (#5958, closed), remembered grants (#5960, closed).

## 2. Goals / non-goals

Goals: per-user+world baseline store with accept/narrow/pin/grant-page-edit/yolo-auto-accept history; connect-stage covered/uncovered/uncomputable verdict; delta-only notice surface (client/server groups separate, server-side disclosure only), three choices with no default button, per-class denial composing per #5960 §2, grant-on-use honesty line where `CapsReport.policy` is grant-on-use.
Non-goals: (a) no class-table invention here — class assignment is #5999's own follow-up; unmapped names fall to `unknown` fail-closed so the pipeline lands first; (b) no silent narrowing — shrink narrows only with bookkeeping, re-addition after narrowing re-prompts; (c) this record adds no runtime behaviour.

## 3. Alternatives considered

1. Do nothing (connect without comparison): rejected — stale baselines silently exercise unaccepted capabilities; the whole #5959 programme exists because this failure mode is real.
2. Block-then-ask at every connect (no baseline memory): rejected — prompts on every connect train users to click through; the baseline store exists so the prompt fires iff the trigger fires.
3. Auto-accept everything except shrinks: rejected — re-addition after narrowing must re-prompt (or narrowing is meaningless); YOLO mode auto-accepts but records history and never leaves a stale baseline.
4. Chosen: #5959 §§1–4 — baseline store + connect-stage comparison + delta-only notice, fail-closed `unknown` on uncomputable.

## 4. Chosen approach

Per #12921 scope (authoritative detail in #5959 §§1–4, #5999 §6, #5958 §4): baseline record per user+world keyed over release identity with full choice history, stored with the user's ID state beside #5960's remembered grants and listed with history on the world's grant page; connect stage resolves release + instance records, verifies release identity, computes `N`, compares against `B` — covered ⇒ proceed with automatic narrowing bookkeeping; uncovered ⇒ hold before any capability is exercised and present the notice; uncomputable ⇒ fail-closed `unknown` notice; notice shows delta-only headline lines from the client class table, client-side/server-side groups separate.

## 5. Dependency-ordered slices with acceptance

1. Slice 1 (baseline comparison core) — DONE (PR #13079, merge f2a50a9cae6b). Acceptance: per its PR; #5999 §7 invariants 1–5, 8, 9 over `(B, N, ≼)` hold on main.
2. Notice-surface data side + grant-page listing data side — DONE (PR #14049 merged 9658b5c5; new `cap_notice.rs` + lib registration; laptop `cargo test -p id-caps` 267 passed; Fleet dispatch fp_5e9b2b2c3b14d11fb3913ec0a58a7634). Acceptance: prompt iff trigger; per-class denial excludes exactly that class; narrowing on shrink; re-prompt on re-addition; YOLO records history, no stale baseline; fail-closed on uncomputable.
3. Pure connect-stage verdict API (`ConnectVerdict`/`evaluate_connect` in id-caps) — OPEN (PR #13999 open, commit cf840a6d0ff; local `connect_verdict` tests 2 passed at last report; Fleet run 45307929204 execute step succeeded). Acceptance: merge + the two local tests green in CI + zero file overlap conflicts with slices 1–2.
4. Grant-page rendering (id-web) + live connect wiring — OPEN (follow-on #14050, refs #13999). Acceptance: listing renders from slice-2 data side; live pipeline holds the connection on uncovered verdict per §4.
5. Class-table fill-in (names → classes beyond `unknown`) — follow-on, non-blocking: pipeline lands first per the `unknown` fail-closed rule (#5999 §10 lineage).

## 6. Rollout + rollback

Rollout: slice order above (§5); each slice lands behind the existing fail-closed default (`unknown` ⇒ notice), so partial rollout degrades to more prompts, never silent exercise. Rollback: revert per slice; baseline store is additive beside #5960 grants (no migration to unwind); narrowing bookkeeping is history rows, not destructive edits — revert restores prompt-on-uncovered behaviour.

## 7. Verification plan

Property tests: #5999 §7 invariants 1–5, 8, 9 over `(B, N, ≼)` (already on main, not duplicated per #14049 record). Notice-level oracles (§2 goals): prompt-iff-trigger; per-class denial exactness; narrowing on shrink; re-prompt on re-addition; YOLO history with no stale baseline; fail-closed prompt on uncomputable comparison. Venue bar per slice: laptop `cargo test -p id-caps` + Fleet dispatch receipt + `cargo fmt` clean (the #14049 precedent).

## 8. Open questions (named deciders)

1. Grant-page rendering owner and venue (id-web side of #14050) — Decider: Drew / id-web owner.
2. Live-connect wiring order vs #13999 merge (does #14050 wait on the verdict API, or stub it?) — Decider: Drew.
3. Whether #12921 closes when slices 3–4 land, or stays open through class-table fill-in (slice 5) — Decider: Drew / issue owner.
4. Class-table fill-in sequencing across provider manifests — Decider: Drew (needs #5999 §10 owner).

## 9. Record + tracking + queue

- Record: this file (dated `doc/`, docs-only change, `Part of #12921`).
- Design depth homes: #5959 record (§§1–4), #5999 record (§§6–7, §10), #5958 §4, #5960 §2.
- Tracking: #12921 stays open (slices 3–4 + Q1–Q4); progress notes from oc-code-03 lanes already on the issue.
- Queue: task 21679 completes on merge of this record's PR; build slices live in #13999 (open PR) and #14050 (open follow-on) — no new packets needed.
