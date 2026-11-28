# Handoff: the research-to-delivery pipeline and platform backlog

- Date: 2026-10-22T15:25:00Z
- For: a fresh agent with no context. Everything you need is here or linked.
- Your role: finish the workflow foundation, then drive the `id` platform backlog through it. Subagents implement, review and validate; you decide, merge (merge commits only) and restart stalled work.
- Evidence base: the reviews in `/tmp/pipeline-review/` (this machine), the discovery JSON in `/tmp/pipeline-review/discovery/`, and `origin/main` of `developing-today/code`.

---

## 1. Where things stand

| Item                                           | State                                                                                                                                                         |
| ---------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Workflow framework (tool, contracts, 9 skills) | Merged into `main` (PRs #337, #486, #487, #488, #489, #490, #518, #537, #566)                                                                                 |
| Cairn library copy                             | Merged in Cairn (PR #1); **out of date vs `main`, sync not done yet**                                                                                         |
| Intake of the original request                 | Package `doc/2026-10-19T06-09-02Z_research_research-intake-to-implementation/` exists at stage S0 (prepared), 14 concepts; only C001 has had any prepare work |
| Issue backlog filing                           | 31 issues created from the docs discovery (`/tmp/pipeline-review/discovery/filed.json`)                                                                       |
| Issue close sweep                              | 5 closed with evidence (#182, #164, #230, #292, #141); 7 more close candidates listed in §4                                                                   |
| `main` verification                            | workflow tests 73+, `research_workflow.py check` ok, prettier clean                                                                                           |

## 2. Open pull requests (verify CI before merging; merge commits only)

| PR       | Head                           | CI                                     | What it is                                                                                                                                    |
| -------- | ------------------------------ | -------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------- |
| **#594** | `fix/545-549-ssh-screen-queue` | pending (`nix flake check`, `treefmt`) | SSH alternate screen + visible dropped input. 21 tests pass locally. Hand-written; check the diff yourself before merging. Closes #545, #549. |
| **#582** | `wf2/skills-3b`                | green                                  | `feature-plan` and `implementation-orchestrate` skills. Worktree has 3 uncommitted files: finish or drop them.                                |
| **#581** | `wf2/tool-live`                | green                                  | ready-wave exclusions, race-free note receipts, leases. 2 uncommitted files remain (status --live, remote-notes scan, CI update).             |
| —        | `docs/intake-platform`         | no PR yet                              | S0 intake package for SSH/transport/clients/native-client ideas; 2 commits pushed, 1 dirty file. Finish then open the PR.                     |
| —        | `docs-architecture`            | gone                                   | deleted earlier (content was in `main`)                                                                                                       |

Pre-merge check, for every PR: `gh pr view <n> --json headRefOid` must equal `git ls-remote origin refs/heads/<branch>`; CI green; the PR touches only files it owns (the ownership table is in `/tmp/pipeline-review/WAVE2.md`).

## 3. Planned but not started (code)

Worktrees already exist for the first two, pushed with no commits:

| Item                                                                                                  | Issue      | Worktree                                                             | Notes                                                                                                        |
| ----------------------------------------------------------------------------------------------------- | ---------- | -------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------ |
| Root flake evaluation broken (`clanModules` removed from clan-core)                                   | #159       | `/home/user/work/wt/fix-159` (if missing, create from `origin/main`) | Evaluate only; never `nixos-rebuild`. Evidence and pointers in `/tmp/pipeline-review/discovery/d2-issues.md` |
| mcpx orphaned-session test flake                                                                      | #290       | `/home/user/work/wt/fix-290`                                         | Failing run 39507103235; cause visible in its diagnostics                                                    |
| A view trap must not poison the world (Phase 11 F11.17)                                               | #547       | `/home/user/work/wt/f11-17`                                          | Tests first; `sandbox.rs` + `world.rs`                                                                       |
| Bytes-per-frame baseline table (F11.9)                                                                | #542       | none                                                                 | small; measure 80x24 and 200x60 per example                                                                  |
| Injected clock (F11.5)                                                                                | #552       | none                                                                 | removes `sleep` from schedule tests                                                                          |
| Split the large modules (F11.10, moves only)                                                          | #540       | none                                                                 | `world.rs` 6,120 lines; now unblocked                                                                        |
| `id world verify` replay self-check (F11.16)                                                          | #544       | none                                                                 |                                                                                                              |
| Smaller modules, wasm-opt (F11.15)                                                                    | #555       | none                                                                 |                                                                                                              |
| Pinned Roc toolchain for checked-in examples (F11.18)                                                 | #541       | none                                                                 | behavior comparison, never hashes                                                                            |
| #211 reconciliation gaps: total records cap, document output through `send`, per-world document store | #548       | none                                                                 | small follow-up after #211 merged                                                                            |
| Alternate screen, dropped input                                                                       | #545, #549 | **PR #594 open**                                                     |                                                                                                              |

Full issue numbers by area: `/tmp/pipeline-review/discovery/filed.json` and the table in `/tmp/pipeline-review/discovery/d1-docs.md` (D1-0NN → issue). Tracking issues for design work: #534 (Phase 12 designs), #535 (SSH input and presentation), #536 (transport), #538 (native clients and client capabilities).

## 4. Close or decide candidates (from `/tmp/pipeline-review/discovery/d2-issues.md`)

- **Already fixed, close with evidence:** #178 (read path budget verified in `range.rs`).
- **Needs one check first:** #190 (`cli_integration.rs` stall), #257 (ENOSPC — check recent `nix ci` runs).
- **Needs an owner decision, then close:** #161, #169, #170, #171, #185, #195, #157, #437, #434 (each has a stated default; see the report).
- **Duplicate:** #176 (same suite as #229).
- **Partly done, edit the issue:** #311 (says #211 has conflicts; it merged 2026-10-20), #288.

## 5. Research packages and designs (not started)

Issue clusters the discovery recommends running through the workflow rather than as single PRs: groups fold at ingest (#168, #165, #163, #171, #162), Phase 4 delivery liveness (#187, #189, #186), sync bounds (#178, #179, #180), LogSync (#241 + PR #255), identity event (#160), journal format (#227), mcpx backlog move (#309), Seitan V2 (#194), Firefox E2E (#229), clan-core migration (#159).

Design-sized items already collected into tracking issues #534/#535/#536/#538; they need the S0→S6 run.

## 6. Discovered but deliberately not yet acted on

- **Report documents:** `platform-world-apps-migration` worktree holds two uncommitted dated reports (`2026-10-18T00-00-00Z_reference_platform_state/`, `2026-10-18T18-40-00Z_report_platform_world_state/`). The intake branch copies them into `doc/` — keep them (they are the source for the intake package).
- **Validation trial** (zero-context run of the merged skills on real and garbled input): `/tmp/pipeline-review/validation/REPORT.md`, with handoff notes in `/tmp/pipeline-review/handoffs/`. Not yet triaged.
- **Un-merged discovery:** `d3-prs` (PR follow-ups) and `d4-messages` (owner's messages about `id`) never produced reports. See §7.
- **Repo notes:** `AGENTS.md` carries the workflow routing and parallel-writer rules; the Pocock skills are vendored under `.opencode/skills` with `POCOCK-UPSTREAM.md`; `planning/workflow/*` is the contract and `planning/DOCUMENTATION_ARCHITECTURE.md` the docs map.
- **Local worktrees are disposable:** `/home/user/work/wt/wf2-*`, `fix-*`, `f11-17`, `intake-platform`. Git writes need `source /home/user/.local/share/plaza-tools/git-env.sh`.

## 7. Next steps, in order

1. Merge #582 and #581 (CI green; check SHA and ownership first), finishing their uncommitted work on fresh branches if anything remains.
2. Merge #594 after `nix flake check` and `treefmt` pass.
3. Finish and PR `docs/intake-platform`; then run S1 (`research-prepare`) in parallel on its concepts.
4. Implement #159, #290, #547, #542 — four independent worktrees, safe to run at once.
5. Re-run the discovery passes `d3-prs` and `d4-messages`, then triage `/tmp/pipeline-review/validation/REPORT.md`.
6. Sync the workflow library from `code` into Cairn (`scripts/sync_research_workflow.py --write`), and close the loop on the issue clusters in §5.

## 8. Not verified / risks

- `nix flake check` on #594 was still running when this handoff was written.
- `d2-issues` did not run tests locally for #257, #190, #162, #179, #178-read.
- The account hits a subagent session limit every ~2 hours; resume stopped subagents by session id, or start fresh agents with short briefs (resuming long ones burns the window on context).
- Cairn `main` has diverged from `code`'s workflow copy; do not sync until the open PRs in §2 are merged.
