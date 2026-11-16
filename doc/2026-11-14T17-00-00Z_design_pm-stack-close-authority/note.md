# PM stack close authority — end-to-end review and target design

Status: DECIDED + implementation in flight · 2026-11-14 · Program: #10136 · Trigger: #7610 (closed) · Ordered by Drew 2026-11-14 ("review this entire system end to end… make something coherent that doesn't break this way")

## 1. What broke, in one paragraph

Kaneo's GitHub integration closed GitHub issues seconds after pull requests merged — including PRs that merely _referenced_ the issue ("Refs #n", or just "(#n)" in the title) with no closing keyword and work still open (#9106 sat wrongly closed ~14.75 h; #9078's elaboration task was consumed as already-done; #9060 is _still_ wrongfully closed). The config-level fix landed 2026-11-14 (`onPRMerge` → `in-review` in Kaneo's DB; `pm-stack/KANEO-GITHUB-SYNC.md`). This note is about why the system allowed that at all, and the design that prevents the whole class.

## 2. System inventory (verified live 2026-11-14)

**State stores**

| Store                                                                                 | Holds                                                                 | Owner of truth for                                                    |
| ------------------------------------------------------------------------------------- | --------------------------------------------------------------------- | --------------------------------------------------------------------- |
| GitHub issues/PRs (code, forks/opencode, forks/openchamber, dezren39/nix)             | issue state, `status:` + `status/` + `queue/` labels, sub-issue graph | Code-work item state (2026-10-24 design: "the non-negotiable centre") |
| Kaneo Postgres (compose, laptop; 4,425 tasks, 4,418 GitHub-linked)                    | tasks, columns, `integration.config`                                  | Execution board view                                                  |
| Queue DB (`agent.tasks`, live backend Turso via fleet crates; Postgres parity record) | claims, leases, lane ledger, progress/session bindings                | Execution/task state                                                  |
| Files on the laptop                                                                   | claimant `mode` file, pm-stack checkout, unit files                   | Local behaviour switches                                              |

**Writers of GitHub issue state (complete list, from code + live evidence)**

| Writer                                                   | Can close?                                                                                                       | Notes                                                                                                      |
| -------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| Kaneo (upstream, via our GitHub App)                     | **YES** — task → final column sync closes the issue; labels `status:*` both ways                                 | The #7610 actor. App permission `issues: write` is monolithic: GitHub offers no label-but-not-close scope. |
| GitHub native (closing keywords, web UI, API as a human) | YES                                                                                                              | The legitimate paths.                                                                                      |
| pm-dispatch (`dispatch/src/github.ts`)                   | Only its own just-created issue, as compensation when enqueue fails (#4917)                                      | Deliberate, scoped, grandfathered in the design (§4).                                                      |
| GitHub Actions (9 workflows hold `issues: write`)        | 3 close — each only its own tracking issue                                                                       | `bws-freshness`, `dispatch-health-probe`, `flake-inputs-update`. No workflow closes on PR merge.           |
| opencode-claimant / lanes tooling                        | No (comments via `gh`, label _reads_ only)                                                                       |                                                                                                            |
| projector / aggregator                                   | No — read-only by construction (projector keeps a delivery log + reconcile poll; aggregator "never writes back") | The healthy pattern.                                                                                       |
| Muse closure watchdog (#10105)                           | Reopens only                                                                                                     | The safety net, §5.                                                                                        |

**Sync paths**: GitHub App webhook → Kaneo (`issues`, `issue_comment`, `pull_request`, `push`); Kaneo → GitHub status sync; projector webhook + 15-min reconcile → its own DB only; pm-git-sync (repo → laptop checkout, the deploy path — **found FAILED**, §3.4); claimant poll → queue broker; dispatch → queue + OpenCode.

## 3. Design flaws that made #7610 inevitable

1. **Close authority was never assigned.** Issue state is mirrored into three stores with multiple writers, and the most dangerous transition (close) was a _side effect_ of a board column move. Any path that moved a task to Done — a PR event, a UI drag, an API call — closed the issue. Nobody owned the question "who may close an issue?"
2. **Safety-critical config lived only in a vendor DB.** `integration.config.statusTransitions` is set by nothing in this repo. The connect flow recreates it with the upstream default `onPRMerge: "done"`, and in v2.47.0 deleting the rule in the UI also falls back to `"done"`. A config that can silently revert to dangerous is not a fix; it's a reprieve.
3. **Loose linking × keyword-blind automation.** PR→task linking accepts any `#n` in a title; the merge handler never inspected title/body for a closing keyword. Two individually "convenient" behaviours compose into wrongful closes at machine speed (7 s for #9106).
4. **Silent defaults and silent failures.** Missing claimant `mode` file ⇒ "shadow": safe direction, zero signal — summit dispatch stalled on it (#9106) and the file was _still missing_ at review time. `pm-git-sync.service` failed ⇒ repo changes silently stop reaching the laptop. `pm-projector-reconcile` observed stuck in `activating`. Each is individually small; the pattern — _absence of a thing looks exactly like health_ — is the flaw.
5. **No invariant enforcement in the sync layer.** GitHub cannot hard-block closing a parent with open children (#9060's own design concluded "the monitor is the enforcement surface"), yet until #10105 there was no monitor, and v1 of it had holes: no nix coverage, a 2 h horizon under an advancing watermark (permanent misses), no pagination, a keyword veto that also vetoed the open-sub-issues rule, and no visibility of `status:done` without a close.
6. **Two label families for one fact.** `status:` (Kaneo, 3 values) and `status/` (fleet, 18 values) coexist, with semantic duplicates (`status:done` vs `status/done`). Tolerable once ownership is explicit (§4); confusing and drift-prone while it isn't.

Also recorded, not flaws of design but of deployment hygiene: Kaneo is compose-managed while Grist/Pad are NixOS oci-containers (#1752 unfinished); Kaneo's Postgres — the only copy of 4,400+ tasks — has no backup routine (#1514).

## 4. Target design (decided)

**P1 — One source of truth per fact.** GitHub for code-work issue state; Kaneo for its board view; the queue DB for execution state; the repo for _configuration_. Projectors and aggregators stay read-only, forever.

**P2 — Close authority, enumerated.** A GitHub issue closes ONLY via:
(i) a GitHub-native closing keyword on a merged PR,
(ii) a human closing it on GitHub,
(iii) a human deliberately moving the linked Kaneo task to Done (the documented, wanted board sync — proven at setup, 2026-10-24).
Automation may label and comment; it must never _cause_ a close. Sole exception, grandfathered: pm-dispatch may close an issue it created itself, in the same request, when enqueue fails (compensation, #4917) — it may never close an attach target.
Because the App's `issues: write` cannot be scoped down, P2 is enforced by construction + monitoring (P3), not by permissions. The alternative — replacing Kaneo's integration with a first-party sync service — was considered and rejected for now: Kaneo's remaining close path after P3 is exactly (iii), which we _want_; a bespoke sync layer would re-own comments/labels/two-way status for marginal gain over P3 + the watchdog. If Kaneo ever gains another automated path into Done, this decision is revisited.

**P3 — No automated transition may target a closing state.** The transition map is declared in the repo and continuously asserted: `pm-stack/kaneo-sync-guard.py` (slice #10137) checks every GitHub integration's `statusTransitions` — no event-driven transition targets `done`; `onPRMerge`/`onPROpen` = `in-review`, `onBranchPush` = `in-progress` — at deploy time (deploy.sh) and hourly (systemd timer), healing drift and failing loudly. This converts the DB-only setting into repo-owned, self-healing config and kills both silent-revert paths in §3.2. With P3 in force, loose linking (§3.3) is _contained by construction_: a stray link's worst case is a task moving to in-review/in-progress — visible, reversible, never a close.

**P4 — The epic invariant is enforced, not hoped for.** An issue with open sub-issues is never done. Enforced by the closure watchdog v2 (#10138: the open-sub-issues rule fires even against a closing keyword; full 4-repo coverage; watermark catch-up with pagination; `status:done`-on-open-issue sweep) and a periodic sweeper (#10139) that inventories closed-with-open-children across all repos — the #9060 class — with a report-first, then auto-reopen policy.

**P5 — Fail-safe means no-mutation + a signal.** Missing config defaults to the non-mutating behaviour (claimant shadow is correct as a _safety_ default) AND raises a visible health item (#10141): claimant mode and pm-git-sync/projector-reconcile unit states surface on the aggregator's needs-attention tier. Silence is the bug, not the default.

**P6 — Label ownership.** `status:` is Kaneo's mirror (3 values, bot-owned); `status/` + `queue/` are the fleet's (human/automation-owned per their existing writers). No new family may be introduced; cross-family duplicates are documented here rather than migrated (migration risk > confusion cost).

**P7 — Auditability.** Every automated state change already leaves a GitHub timeline event (actor + timestamp); the watchdog's evidence comments and the guard's drift log lines are the audit records for the enforcement layer. The projector's delivery log (`pmp_deliveries`, idempotent by delivery id) is the replayable event record #9568's C2 pipeline builds on.

## 5. What was implemented in this pass (2026-11-14)

- **Watchdog v2** (#10138) — live on the existing 15-min cron: 4-repo coverage, paginated catch-up (no age horizon above the watermark), epic-invariant precedence over keywords, per-issue/per-repo error isolation, repeat-close counter, open-issue `status:done` sweep. Dry-run-verified against #9106, #9078, #9060 (all fire both rules).
- **Kaneo sync guard** (#10137) — script in `pm-stack/`, installed on the laptop, hourly timer enabled, deploy.sh hook in this PR. Live-verified: clean run conforms on all 4 integrations; an induced `onPRMerge="done"` drift on the nix integration was detected, healed, and reported (exit 1).
- **Program filed** — epic #10136 (sub-issue of #9196) with slices #10137–#10142 and #10105 attached; deployment hygiene tracked to #1752 rather than duplicated (#10142).

## 6. Migration safety & rollback

No cutover: every change is additive or config-asserting. The guard only ever writes the documented safe map; reverting = disable the timer + restore a prior `integration.config` from Kaneo's DB (the pre-guard state is exactly today's verified map). The watchdog reopens/labels-strips only; its state file can be re-initialised (`--init`) to halt catch-up. Queue tasks, Kaneo data, and lane workflows are untouched. Kaneo upgrades and integration re-connects are the two events that can reintroduce drift — both are now covered by the hourly guard, and the fixture battery (#10140) is the re-verification procedure after either.

## 7. Open items handed to the slices

#10139 sweeper build + first inventory (expected headliner: #9060 — reopen itself belongs to the closure-audit pass already running in parallel). #10140 fixture battery, incl. the in-Kaneo human Done move. #10141 health surfacing (and the pm-git-sync repair). #10142/#1752 manager end-state + Kaneo Postgres backups (#1514) before any cutover.
