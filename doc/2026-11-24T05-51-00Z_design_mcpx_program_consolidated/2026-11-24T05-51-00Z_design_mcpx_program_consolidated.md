# mcpx program — consolidated 9-point design record (Part of #13264)

- Date: 2026-11-24
- Tracking issue: developing-today/code#13264 (open, `status:in-review`; programme tracking home)
- Task: repo-health-code-pilot 21663 (design slice, 45-min timebox)
- Prior records: `doc/2026-11-23T12-40-29Z_design_mcpx_program_investigation_prompt/` (full prompt), `doc/2026-11-23T18-00-00Z_design_mcpx_program_delivery/` (pointer)
- Live book: `pkgs/mcpx/docs/program/` (`overview.md`, `lifecycle.md`, `policy.md`, `publication.md`, `runtime.md`, `interop.md`, `libraries-models.md`, `issue-map.md`, `high-level-roadmap.md`)
- 9-point standard: `doc/2026-11-24T00-24-54Z_audit_design-gaps/2026-11-24T00-24-54Z_audit_design-gaps.md` § "The sufficient-design standard"

This record does not re-argue the book. It consolidates the book + #13264 prompt/threads into the 9-point shape so the audit gap ("all 9 sections absent from the issue body") is closed by pointer.

## 1. Problem + context

mcpx is the fleet's single MCP gateway (Go daemon + CLI in `pkgs/mcpx/`): owns every MCP server process, serves every harness. Prior to the #13264 programme, designs were scattered across per-issue threads (supervision #9963/#1384, policy, Atlas #8625, config exemption tail, publication coverage) with no single statement of purpose, roadmap, dependency order, or acceptance bar. Evidence: book `overview.md` implementation-order table; `issue-map.md` census (48 labelled + 124 broad matches at survey time, 130 after filing); runtime/publication audits with dated receipts (e.g. d24c19c 241-setting inventory: 236 named probes + 5 exemptions; 402 oracle #13544/#13548 merged b853df8).

## 2. Goals / non-goals

Goals: one gateway with supervised server lifecycle, per-client policy ceilings + audit, versioned remote/hosted deployments, config-system completion (exemption tail to zero), observability, library/model reuse, and a verified mcpx↔fleet↔Atlas↔ID surface — built to a fully-featured platform, not an MVP.
Non-goals: (a) no implementation of flag-held items without Drew's item-specific word — #8472 `plugin.disabledTools` + `plugin.instructions` remainders, #8473 `protoTasks.cancelGrace`; (b) no silent decisions on Drew's reserved calls — #8809 server logins, #8810 answers, #1943 CNAME, #12900 thread 2 (surfaced, never decided here); (c) this record adds no new runtime behaviour — it ratifies the book.

## 3. Alternatives considered

1. Do nothing (leave designs per-thread): rejected — audit #13873 scores #13264 body 9/9 absent; per-thread designs cannot express cross-cutting order (supervision → policy → interop).
2. Single mega-design doc duplicating the book: rejected — duplication rots; the book already carries approach detail per chapter.
3. Status quo chosen: keep depth in the versioned book chapters, close the audit gap with this thin dated consolidation record + issue links (this file).

## 4. Chosen approach

Programme structure = book chapters (contracts/acceptance per slice) + `issue-map.md` (membership/dependencies/evidence split) + dated `doc/` receipts + #13264 as tracking home. Chapter map: onboarding/ownership → `lifecycle.md`; policy/audit → `policy.md`; docs/config → `settings.md` + `publication.md`; verification venues → `runtime.md`; fleet/Atlas/ID/harness interop → `interop.md`; libraries/models → `libraries-models.md`; order + acceptance → `overview.md`.

## 5. Dependency-ordered slices with acceptance

Per `overview.md` (prerequisite order); each slice is one lane/session with the listed oracle:

1. Payment refusal (#13544 — DONE, merged #13548 b853df8): loopback 402 → one request to refusing provider, zero to successor; 503 still advances.
2. Docs and configuration: every registered setting maps to one named probe or exemption; staged HTML + agent Markdown links resolve under deployment base; all public book roots compared after publish.
3. Server ownership (`lifecycle.md`): same-name instances keep distinct sentinels across upgrade; removal leaves neighbour callable; interrupted mutation never silently replayed.
4. Policy and audit (`policy.md`): denial → zero upstream calls + payload-free decision record; revoked grant fails before queued execution.
5. Fleet/Atlas/ID/harnesses (`interop.md`): each client supplies attributable allowed call + denial; corpus filtering at Atlas, capability attenuation at ID; missing receipts recorded individually.
6. Libraries/model ops (`libraries-models.md`): version-pair comparison on same fixture; cancellation/repair preserve caller operation budget.

## 6. Rollout + rollback

Rollout: book-first (land design deltas as chapter updates with `Part of #13264`, never `Closes`), then slice PRs in overview order, each updating its chapter reference in the same change that alters a command/flag/setting/transport/route. Rollback: per-slice revert (design PRs are docs-only, safe to revert); runtime slices roll back by restoring prior declaration generation (compare-and-swap at commit; removals/permission-reductions take precedence on failure — `lifecycle.md`).

## 7. Verification plan

Named-test + execution-receipt bar per `overview.md` "Maintaining the evidence": for each implementation save Fleet handle, commit, runner, non-skipped assertions, outcome; distinguish merge vs passing checks vs observed deployment. Standing fixtures: 402/503 oracle; supervised-restart fault table (`lifecycle.md`); policy fixture table (`policy.md`); publication anchor/link audit (`publication.md`); live-unit sampling procedure (`runtime.md`). Venues carry their own gates (#3016 stress gates #13371/#13349; #2686 sandbox exclusions; #1983/#13386–#13388 remote/matrix legs).

## 8. Open questions (named decider: Drew)

1. Flag-held remainders (#8472, #8473) — implement or keep held? Decider: Drew, item-specific word required.
2. Reserved product calls (#8809 logins, #8810, #1943 CNAME, #12900-t2 multi-client demand) — Decider: Drew.
3. Supervised-by-default flip (#9963 Phase 3, opt-in pause) — Decider: Drew on attach-path evidence.
4. Atlas four-harness acceptance (#8625) sequencing vs policy slice — Decider: Drew.
5. Exemption-tail-to-zero priority vs feature slices — Decider: Drew.

## 9. Record + tracking + queue

- Record: this file (dated `doc/`, docs-only change).
- Tracking: #13264 stays open as programme home (2/3 sub-issues complete at time of writing).
- Queue: task 21663 completes on merge of this record's PR; follow-on build slices already have live homes (issue-map rows + queue receipts 21396–21398 for #13573/#13575/#13577; maintenance epic #12459; platform tracker #1900).
