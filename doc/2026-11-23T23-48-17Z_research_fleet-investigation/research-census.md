# Fleet program — issue census (coordinator's own pass, 2026-11-23)

All counts: GitHub Search API via `gh api search/issues`, run 2026-11-23 ~12:40Z from the
sandbox seat. Labels overlap; per-label counts are not additive. The deduplicated union is
the headline.

## developing-today/code — fleet label space

Fleet labels present on the repo (labels API): `app/fleet`, `app/dispatch`,
`app/fleet-paperwork`, `app/codedb-bridge`, `app/jules-fleet` (Jules is retired — any open
items under it are stale by definition), `design/fleet-brokers`, `design/quota-plane`,
`dispatch/remediate`, `epic/fleet-apps`, `epic/fleet-ca-secrets`, `epic/fleet-expansion`,
`epic/fleet-maintenance`, `epic/fleet-orchestration`, `epic/fleet-tool`.

Open issues per label (Search API, `is:issue is:open`):

| label                    | open |
| ------------------------ | ---- |
| app/fleet                | 113  |
| epic/fleet-orchestration | 67   |
| epic/fleet-ca-secrets    | 12   |
| epic/fleet-tool          | 11   |
| design/quota-plane       | 5    |
| epic/fleet-apps          | 3    |
| design/fleet-brokers     | 2    |
| epic/fleet-expansion     | 1    |
| epic/fleet-maintenance   | 1    |
| app/dispatch             | 0    |

**Deduplicated union of the nine fleet labels above: 185 open issues, 406 all-state.**
`app/fleet` alone: 113 open / 251 all-state.

Status cross-section of the 113 open `app/fleet` issues (Search API):

| cross-label      | open |
| ---------------- | ---- |
| kind/design      | 98   |
| status/blocked   | 63   |
| queue/blocked    | 62   |
| queue/queued     | 8    |
| status/in-review | 0    |
| user/owned       | 1    |
| user/escalated   | 1    |
| user/review      | 0    |

**Census nuance (spot-checked):** the 98 `kind/design` count is real but dominated by the
pm-orchestration design DAG (the #9198–#9519 design series, e.g. #9514–#9519
`design(pm-orchestration): …`), which carries the `app/fleet` label. The design backlog is
the centre of gravity of the fleet label space, and a large share of it is that one DAG.
Repo-wide open `kind/design` is 212, so fleet-labelled issues are ~46% of all open design
work in the repo.

## dezren39/fleet (upstream)

Repo: default branch `main`, not a fork, pushed 2026-11-23T08:42Z.
**7 open issues, 1 open PR** (issues API, 2026-11-23):

- #88 fleet-build leg failing: build
- #87 sync-upstream cannot run on new org forks: app-id 5454422 vs fleet-app key, and the
  app isn't installed there (remainder lives in code#6645 per fleet records)
- #39 fleet-build leg failing: openchamber
- #38 fleet-build leg failing: zettel
- #37 fleet-build leg failing: code
- #36 fleet-build leg failing: opencode
- #35 fleet-build leg failing: nix
- PR #1 Configure Renovate (open)

## developing-today/fleet (org fork)

Repo: fork = true, default branch `main`, pushed 2026-11-23T08:43Z.
**0 open issues** (repo `open_issues_count` = 0, which includes PRs — so no open PRs either).

## Defect/program statuses — verified 2026-11-23 ~12:45Z (gh REST, second source)

These supersede any older status in briefs and memory; the ground moved today:

- **code#13091 (queue double-serve): CLOSED.** Fix
  [PR #13143](https://github.com/developing-today/code/pull/13143) MERGED (head
  `f7f45d67925e`), title: "fix(queue): broker never re-serves condition parks or
  completed work (#13091)". Lanes still observe stale serves occasionally — the
  investigation should verify whether observations postdate the merge/deploy or are
  broker-generation lag (the laptop queue-broker still runs gen 5 / b8bbcc0, which
  predates the fix).
- **code#13191 (packet-repair verb): CLOSED.**
  [PR #13218](https://github.com/developing-today/code/pull/13218) MERGED
  2026-11-23T11:14:22Z (`ab10437c6b7f`): "fleet queue: add repair verb for
  quarantined/malformed task packets". **But the verb is not yet in any deployed
  binary**: the sandbox seat binary `fleet-static-3cb241a` answers
  `fleet queue repair` with "unknown command" (first-hand check, 2026-11-23). Its
  queue verb list: validate, enqueue, list, claim, release, lane, bind, heartbeat,
  checkpoint, park, wake, block, unblock, dep-recheck, quarantine-recheck, cancel,
  revive, fail, complete, verify-disposition, timeline, history, show, models,
  health, signal, fallback, dispatch, session, surfaces, outbox, stack. The
  publish-fleet + bootstrap-pin rollout is the pending step; the fleet book's repair
  chapter rides it (DOC-01).
- **code#13146 (placement sessions): OPEN, slice 1 landed.** PR #13169 merged
  (`15b2ac834b0d`): `placement_sessions` in the Turso backend (SCHEMA in
  `turso_queue/core.rs`; `turso_queue/placement_sessions.rs`; routes
  /placement-put|get|list|sync in `stacks_outbox.rs`), bounded broker transport,
  client `placement/remote.rs`. Local JSON remains the only decision input. The issue
  stays open on criterion 1: a cross-seat `fleet queue session status` read, which
  completes when the post-#13191 rollout puts broker + two seats on the merged build.

## Structural facts for the prompt's ground-truth section

- The fleet _source_ lives in the monorepo: `pkgs/fleet/` in developing-today/code.
  dezren39/fleet is the umbrella/flake repo (submodules + sync origin), not the source home.
- The fleet book's tracking issue #13165 is CLOSED; the book is live at
  https://developing-today.github.io/code/fleet/ (55 chapters at build time).
- Shortlist tracking: code#12236 (refills recorded there; refill #5 landed 2026-11-23,
  8/8).
- The flag-hold set (issues prepared and awaiting Drew's item-specific approval, ~51 items
  as of 2026-11-23) is enumerated in the coordinator's brief and in lane records; the
  fleet-relevant subset: #13159 family (fleet-build container gaps + evidence records
  #13168/#13193/#13189/#13198/#13170), #13106 (Atlas D1 slice, fleet-adjacent), #6992+#13149
  (pm-dispatch), #6853 (oc-code lane fix), PR shepherds #12928/#12873/#12895/#13097/#12839.
  These are Drew's decisions — the investigation designs around them, never implements them.
