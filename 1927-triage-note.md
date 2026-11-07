<!-- agent-note:v1 agent=repo-worker-code state=done at=2026-11-03T15:52:00Z -->

🤖 **Agent note** · done (triage)
- **Agent:** repo-worker-code, opencode execution lane (`build`, `opencode-route/workhorse`)
- **Session / ID:** session `ses_e5e5df699f1ffeTb5Z3gWDNsYI`, queue task **7787**
- **Found via:** queue claim (task 7787, triage packet)
- **Task:** triage #1927 at current `main`, confirm it is still live, scope the remaining ladder or close it with evidence.
- **Expectation:** done — **still live, partially implemented; keep open and narrow to the unbuilt rungs below.** No code written by this task (triage only).

## Triage verdict — keep open; roughly half the ladder has landed since the 2026-10-29 triage

Checked against `developing-today/code` `main` at **`0467ae0f9`** (2026-11-03). Issue is OPEN, no hard-stop labels, no open PR implementing it (only #2615, merged).

### Landed (verified in the tree, not just claimed by other issues)

| Piece | Evidence at `0467ae0f9` |
| --- | --- |
| **Rung 0** — upstream `-32602` diagnosed | `internal/daemon/callerr.go:168` `explainCall` → `diagnose.CallError` at `:201`; called from `internal/daemon/registry.go:2280` and `internal/daemon/input_required.go:87`. `CallErrorBody` carries structured `diagnostics` **and** the raw `upstream` error, so pass-through still has the bytes. |
| **The dial + ceiling** | `repair.autonomy` at call scope, clamped by `autonomy.max` (`internal/daemon/callerr.go:80-88`, `internal/settings/consumer.go:16`, `internal/daemon/routes_consumer.go:543-573`). Vocabulary `off / advise / ask / propose / run` is settled in `docs/decisions/0002-autonomy-dial.md`; **#1902 closed**. |
| **Rung 1 (first half)** — shared near-miss matcher | `internal/nearmiss` (one definition: normalise + Damerau-Levenshtein + length-relative threshold), consumed by `diagnose.nearest` (`internal/diagnose/diagnose.go:422-433`); **#1904 closed**. |
| **Approach B item 2** — per-call `call` frames | `execsvc.FrameCall` (`internal/execsvc/execsvc.go:298`), `CallInfo` `:309`, emitted `:414`; documented order and documented-vs-emitted test in `docs/exec.md:177` + `internal/execsvc/call_frames_test.go`. Landed by **PR #2615** (merge `d37f22595`). |
| **Approach B item 4** — argument validation before the wire | `internal/mcpclient/schemacheck.go` compiles every tool's input schema and `CallTool` enforces it client-side with `-32602` (`InvalidParamsError`), Phase 2 of #1481 / **#1483 closed**. |

### Still not built — this issue remains the owner

1. **Rung 1, second half — argument-shape fit for substitutions.** `diagnose.CallError` (`internal/diagnose/diagnose.go:468`) handles missing-required, a field named in the server message, and unknown tool/namespace. It does **not** do the issue's `search → query` / `olderThan → olderThanDays` case (argument names satisfying exactly one other tool's schema, or a prefix of exactly one declared parameter). No code in `pkgs/mcpx` implements this today. Deterministic, no blocker — the natural next piece.
2. **Rungs 4–5 — repair propose / repair-and-retry.** `repair.autonomy` is read in exactly one place, `explainCall`'s `advise` gate. Nothing consumes `ask`/`propose`/`run` for repair, and no artifact-backed repair proposal exists (`POST /v1/diagnose` returns diagnostics only; `artifact()` is not wired to repair).
3. **Rung 3 (model-backed "you did this and that" warning).** The deterministic half exists (`elicit.confirmDestructive`, unanswered refuses). The model-backed `onError` version is **#1929** (open).
4. **Dry run** (`ExecOptions.dryRun`) — absent; the only "dryRun" in the tree is a comment in `internal/recipes/recipes.go:15`.
5. **Safe-prefix execution** — absent (no `destructiveHint`/`readOnlyHint` batched stop).
6. **Structured refusal reason codes** — partial only: elicitation answers carry `action` + `reason` (`internal/elicit/elicit.go:121`), but a refused *call* has no machine-readable reason code.

### Blockers and ordering (recorded here per the packet's acceptance)

- **Deterministic now, independent:** (a) argument-shape substitution in `diagnose.CallError`; (b) `ExecOptions.dryRun`; (c) safe-prefix execution. Each can be its own `small_fix`; dry run should go first because it also serves the prompt and hook features.
- **Blocked on a model:** rungs 4–5 need the provider layer **#1925** (open) and the `onError` hook **#1929** (open). Rung 6 needs no new work: `/v1/intent` already exists, so "reinterpret" is composition, not a new feature.
- **Rung 0 residual:** none found — the `CallAsk` gap noted on 2026-10-29 is closed (`internal/daemon/input_required.go:87` now calls `explainCall`).

### Caveat on the earlier triage's dedupe list

The 2026-10-29 triage cited **#1769 / #2083 / #2173 / #2165** as related mcpx issues. At today's state those numbers resolve to unrelated work (`id(directory_auth)`, `proxy-supervise`, quota projections, a main-broken flake check) — all closed. Treat that list as stale; the live related set is **#1902 / #1904 (closed, landed)**, **#1925 / #1929 / #1891 / #1900 (open)**, plus this issue.

### Recommendation

Keep #1927 open as the ladder's tracker with the scope narrowed to items 1–6 above, and split the deterministic remainder into individual tasks in that order. The issue body's cross-references still use the historical `dezren39/nix` numbering (noted in the relocation preamble) — a docs gap, not a blocker.
