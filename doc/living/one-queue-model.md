# One-queue model — gap analysis and designation (2026-11-10)

> Living document — committed 2026-11-11 from ~/workspace/goals/keep-the-four-repos-healthy/hidden_files/one-queue-gap-analysis-and-designation-2026-11-10.md as the tracking home for a completed design (60-day design register entry F4.8; register committed as pm-stack/design-register-2026-11-11.md in developing-today/code, tracking epic developing-today/code#8805). Status: shipped. Update this doc as the design shifts; the register is the frozen audit snapshot.

---

# One-queue lane model — gap analysis, designation, deploy record

Drew's spec: 2026-11-10 20:01–20:06 CST (three statements, quoted verbatim on code issue #8097).
Implementation: PR #8438 (merged 2026-11-11T02:28:30Z, merge commit 2efbeb292fb3658af10bc5db3df2f7e74567d017).
Registry before-image: `lanes-table-before-one-queue-2026-11-10.md` (same folder; 44 rows, 35 enabled).

## Gap analysis (wire-verified against broker source on main + live registry + claim path)

- **G1 — Claim was stream-scoped end to end (the central gap).** `tasks.queue` is a stored per-task stream label (22 distinct values live). The pick SELECT filtered `WHERE queue = ?`; the claim UPDATE re-checked the same predicate. `claim_preamble` REJECTED a lane whose registered `queue` ≠ the claim's queue ("lane X is for queue A, not B") — registry-side scoping of what a lane may see: exactly the binding the spec forbids by default. Priority ordering was global in form (`ORDER BY (priority IS NULL), priority ASC, created_at ASC`) but only ever applied WITHIN one stream.
- **G2 — `--queue` was effectively mandatory in the CLI.** `client.rs` `cmd_claim` always sent a queue: `--queue` flag, else env `REPO_QUEUE_DEFAULT_QUEUE`, else the built-in `DEFAULT_QUEUE = "repo-health-code-shadow"` (and `DEFAULT_WORKER = "repo-worker-code"`). Broker `verb_claim` read `queue` with no default. The ONLY claim filter was the exact stream match. No boolean expressions, no repo/priority-crossing claim existed.
- **G3 — The one-task-one-stream binding was created at enqueue** (`verb_enqueue` writes the packet's `queue`; client resolves queue = flag > packet > default). Per the spec the stored stream stays as descriptive metadata (it is the task's stream label / pool name in event marks), but it stopped being the routing mechanism: repo identity for filtering is the payload's `repo` (repo-task/v1 packets carry `"repo": "owner/name"`, a required field of `~/tools/repo-task-v1.schema.json`), falling back to the stream-derived repo (`repo-health-code-pilot` → `code`).
- **G4 — Claimer identity filtering (refinement 2 audit).** Places identity shaped what was offered beyond a task's own requirement: (a) the preamble lane↔queue registry check (identity scoping via registry); (b) the ledger platform limitation (legitimate — it is a TASK-side requirement, kept). The pick itself did NOT filter by platform/lane/worker (`family` stamped, never matched, #8187 by design). `lane_decision` (#8187) already implements "direct by default; routing only with a recorded reason" at assignment time — aligned, untouched.
- **G5 — Requirement machinery that already fit (kept, generalized).** `repo_task_lane_ledger.platform_limit` + `platform_limit_reason`: a task-scoped claimer-kind requirement enforced in the pick clause, claim-by-id, and transfer (#8217 semantics; a reason-less limit is invalid and cleared, never enforced). It was the ONLY per-task requirement dimension (platform family only). The PR generalizes the same shape to payload `requires` (`platform` / `lane` / `pool` / `model`), enforced in pick + by-id + transfer.
- **G6 — `lanes.queue` is `NOT NULL`.** Unbinding a registration needs a sentinel: the PR defines `"*"` (also `"all"`, `""`) as the recorded unbound state (`lane_queue_binds`); the preamble enforces a registration's binding only for stream-scoped claims against a bound registration.
- **G7 — FOUND DEFECT (pre-existing, not fixed by #8438): dead platform-limit enforcement in `claim_by_id`.** On current main the #8217 platform-limit block inside `claim_by_id` sits after a `return Ok(None)` inside the claim-guard block and is unreachable (rustc emits an `unreachable expression` warning at that block). The pick and transfer paths enforce the limitation live; the by-id path does not, despite the source-shape test `claim_by_id_enforces_platform_limit_like_the_pick_8217` asserting the code is present. The PR's new `task_requirements_unmet` check in `claim_by_id` was placed in reachable code (after `by_id_claimable`). The dead block itself was left untouched by #8438 to keep the change surgical; it needs its own fix (move the ledger block out of the guard block) and its own issue.

## What PR #8438 changed

- Broker: `queue` optional on claim (absent/empty/`*`/`all` = the whole queue, global priority order); new `filter` request field — composable boolean expressions (AND/OR/NOT/IN, parentheses) over repo, queue, pool, platform, label, kind, priority, lane, model — compiled into BOTH the pick SELECT and the claim UPDATE re-check; payload `requires` enforced on pick + by-id + transfer; new `pool` claim field (claimer pool identity, for `requires.pool` + stamps); preamble binding only for scoped claims against bound registrations; all-streams payload-guard sweep for unscoped claims. Claim stamps, pick predicate branches, batch ≤5, by-id/transfer special cases, #8097 rules: preserved (full existing suite green).
- Client: `--filter` and `--pool` flags; claim sends no queue when neither `--queue` nor the env names one (other commands keep the built-in default; scoped callers unchanged — additive).
- AGENTS.md (code repo): new "The one queue: claiming work" section.
- Tests: `cargo test -p fleet-queue --locked --all-features` = 187 passed, 0 failed (5 new one-queue tests). Plus a live-SQLite semantics check of the compiled scope SQL (8 scenarios: cross-stream priority, repo filters both identity forms, label membership, requires gating, ledger interplay, scoped compatibility) — all passed.

## Designated filtered sets (chosen from ENABLED registrations; disabled rows untouched)

### Muse surface (named lanes; designation = the filter expression in the lane's shift-job instructions)

- Code-only (5) — claim `--filter 'repo = code'`: **code-b, code-d, code-e, code-f, code-g**
- Not-code (3) — claim `--filter 'repo != code'`: **nix-a, opencode-a, code-h**
- Unfiltered (13 enabled): code-j, code-k, code-l, code-m, code-n, code-o, code-p, code-t, code-u, code-v, code-w, code-x, code-y
- Disabled, unchanged: code-a, code-c, code-i, code-q, code-r, code-s, code-z, nix-b, opencode-b

### OpenChamber surface (UUID registrations; designation = the filter expression in the OC scheduled-task definitions)

- Code-only (5) — `repo = code`: 02cafe76-eafd-4d0b-aaa6-2b7a737d68dc, 1a180ae5-384f-4e71-8d77-08f7b1652c75, 262eb403-3347-4a7c-ba6b-e7bc263ca853, 335a5d1f-bfc1-4831-99ea-850c8939a727, 7b47536b-c162-444c-be8a-3a9538cc4cb5
- Not-code (3) — `repo != code`: 46984602-51a3-438c-b3a5-f2857811adb1 (repo-worker-openchamber), 3fe37e8a-c61b-419c-9e97-769ecd264d5d (repo-worker-opencode), 8ac61616-3815-4e7b-b33d-840a3829154a (repo-worker-code UUID)
- Unfiltered (6 enabled): 9aca43b4-5439-4939-9f71-323f5d32521b, aea12c82-c5af-4e5a-9b66-f8e360f87177, bf419306-c4ef-470f-a553-beabf93239de, c29174f1-48e0-4b13-b4ff-9b73ab67e9ab, c8b9355e-319a-4640-8096-7ad19480d151, ec118cad-0fd3-4652-91c4-27432f2f2efd

## Registry reconciliation — state and the deploy-gated remainder

Registry values are UNCHANGED tonight (44 rows, 35 enabled, identical to the before-image). Reason: the live broker is still the pre-#8438 build, whose preamble rejects any claim whose queue ≠ the lane's registered queue. Rewriting registrations to the unbound `*` state before the broker deploy would break every live scoped claim those lanes make. Under the merged broker the registration is never consulted for unscoped claims, so no registry edit is needed for the model to work; the `*` rewrite is the recorded end-state formality, prepared here and executed in the deploy step:

```sql
-- DEPLOY STEP (run only after the #8438 broker build is live):
UPDATE lanes SET queue = '*';
-- Reversal: restore each row's queue from lanes-table-before-one-queue-2026-11-10.md.
```

## Pending (deploy step, separate)

1. Rebuild the queue-broker + repo-queue binaries from main (with #8438) and restart the queue broker (the switch-spec rules apply: changed code → explicit takeover).
2. Run the registry SQL above; read back the lanes table (44 rows, queue = `*`).
3. Flip the Muse shift jobs' claim lines from the interim scoped form to the one-queue form (designated lanes add their `--filter`; all others drop `--queue`), and rebuild/redistribute the client binary the shifts call.
4. Apply the OpenChamber designations in OC's scheduled-task definitions (filed as a queue task + recorded on #8097, since OC's task store is not editable from the Muse side).
5. Fix G7 (dead by-id platform-limit block) under its own issue.

## Completion addendum (2026-11-10 ~21:0x CST)

- Muse shift jobs: all 21 enabled claiming jobs updated with the ONE-QUEUE MODEL paragraph and read back (b, d, e, f, g code-only; h, nix, opencode not-code; j, k, l, m, n, o, p, t, u, v, w, x, y unfiltered).
- Shared runbooks: one-queue section added at the top of both `repo-queue-worker-instruction.md` and `lane-shift-instruction.md` (this folder).
- G7 filed as its own issue: https://github.com/developing-today/code/issues/8456 (the dead by-id platform-limit block).
- OpenChamber designation filed: queue task **14835** (openchamber stream, priority 30, queued 2026-11-11T02:45:49Z, packet for openchamber#2081) + issue https://github.com/developing-today-forks/openchamber/issues/2081.
- Record posted on #8097: comment 6785478676 (+ pointer comment 6785478985). Note: #8097 itself is CLOSED (its four rules were implemented 2026-11-09); the record comment stands as the one-queue addendum.
- Registry after: unchanged (44 rows, 35 enabled) — deploy-gated, per the section above.

## Completion addendum 2 (2026-11-10 ~21:36–21:47 CST) — OpenChamber designations applied directly

- Deploy step 4 is DONE, by direct application (Drew's 21:25 CST correction: Muse edits OC's task files on the laptop itself; the task-14835/#2081 filing is the record, not the mechanism — #2081 c.6786350684, #8097 c.6786356190).
- Edited: the 14 designated lane tasks in ~/.config/openchamber/projects/path_L2hvbWUvdXNlcg.json (2 of 3) + path_L2hvbWUvdXNlci9naXQvY29kZQ.json (12 of 24). Each gained adapter item 9 (ONE-QUEUE DESIGNATION) + an unscoped §1 claim command (`--filter 'repo = code'` / `'repo != code'` / none); gate + lane-identity wording aligned. Backups: *.bak-onequeue-20261111T033637Z. Deep-diff verified: only those 14 prompts changed.
- Landed POST-deploy (broker restarted 21:25:16 CST on the #8438 build; registry all `*`), so no interim inconsistency. Pickup proven at the 21:40 CST fire, no OC restart.
- Task 14835 left parked as the record. Open item for Drew: OC's "Queue lane (scheduled) - nix" task (8d133595) still exists/enabled/firing against the deleted broker registration — left untouched, disposition his call.
