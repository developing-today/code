# Pagination contract — closing inventory (#6622 Phase 4, #6623)

Date: 2026-11-12 · Base: origin/main `be6bf9a3af` · Worker: queue task 14524, lane code-b (recon: lane code-f)

The contract (design #6622): **a paged read is complete, bounded, or
says so.** At every read site: (1) the cursor is validated where read —
type first, then progress; (2) the volume is bounded, and breaching the
bound is an error naming the listing; (3) truncation is never silent —
the read completes, fails closed, or marks the truncation in its
output.

Dispositions below are one of: **conforms** (guard named),
**filed** (gap issue linked), **deliberately local/unbounded**
(reason named). Line numbers are at the base above.

## pkgs/id — host-page protocol (Phase 1 system)

| Loop | Evidence | Disposition |
|---|---|---|
| `record_pages` consumer | `pkgs/id/crates/id-worlds/src/world_net.rs:1836` | **Conforms** (#6578). Strict cursor typing (non-string `next_after` is a malformed-reply bail, never end-of-pages), last-key equality, strict-advance check, empty-page-with-cursor refusal, `RecordsBudget` volume bound. |
| `admin_event_history` consumer | `world_net.rs` (`parse_admin_history_cursor`) | **Conforms** (#6600). String-or-null cursor required; missing key refused; advance invariants enforced. Pins: `admin_event_history_cursor_requires_a_string_or_null_next_after` (world_net.rs:5020), property test `admin_event_history_cursor_next_after_invariants` (:5081). |
| SSH `/records` drain | `pkgs/id/crates/id-worlds/src/world_ssh.rs:1918` (`records_cursor`), drain at :1427 | **Conforms** (#6615). Cursor must be a string that strictly advances; truncation is surfaced as a notice in the drain output (the operator disposition). Pin: `records_cursor_must_be_a_string_that_advances` (world_ssh.rs:4286). |
| Sender side (disposition (c)) | `pkgs/id/crates/id-worlds/src/world_session/mod.rs:1154-1260` (host page producer); `pkgs/id/src/web/world_ws.rs:96,179` | **Conforms.** Both producers serialize `next_after` as `Option<String>` in the typed frame — always a JSON string or `null`, never another type and never an omitted key on a page that continues. The receiver-side refusals above are therefore compatible by construction. |
| Other `next_after` readers | repo-wide grep over `pkgs/id` | None exist. Remaining hits are the producers above, frame-size accounting in `pkgs/id/crates/id-caps/src/world_caps.rs` (not a page loop), and tests. `record_pages` also appears in `world/handle.rs` and `world_fixture.rs` as the host/fixture side of the same protocol, not separate consumers. |

Phase 1 pin (disposition (d), first half): cursor-type refusal is
pinned by `test_records_client_rejects_empty_page_with_cursor`
(world_net.rs:3050) plus the :5020/:5081/:4286 tests above.

## github-backup (Phase 2 system) — `infra/github-backup`

| Loop | Evidence | Disposition |
|---|---|---|
| Queue-events drain | `bin/drain_queue.py:119-148` | **Conforms** (#6572). `MAX_QUEUE_PAGES` / `MAX_QUEUE_EVENTS` bounds with named errors; seen-cursor guard; a non-string or empty cursor on an incomplete page fails closed before any event is acknowledged. |
| Owner inventory (GraphQL) | `bin/ghb_common.py:200-287` (`inventory_owner`) | **Conforms** (#6532). `MAX_INVENTORY_PAGES` bound (:276); `hasNextPage` with a malformed/non-advancing `endCursor` raises (:286). |
| Clone/fetch/reconcile loops | `bin/clone_all.py`, `bin/fetch_mirrors.py`, `bin/reconcile.py` | No page followers: clone/fetch are git operations; reconcile's `while True` loops are local parent-chain walks with cycle guards over an in-memory plan, and its clone-status reads are offset-based file tails. Not paginated reads. |
| App setup installations read | `bin/app_setup.py:259` | **Deliberately local.** One-shot setup-time GET; the population is the App's own installations. Not a loop; noted for completeness. |

## cold-store-reaper (Phase 2 system) — `pkgs/cold-store-reaper`

| Loop | Evidence | Disposition |
|---|---|---|
| Drive `files.list` for backups | `cold_store.py:1367` (`list_backups`), token guard :1422-1428 | **Conforms** (#6602, guard lineage #2426). Repeated-`nextPageToken` guard; `LIST_MAX_PAGES = 10_000` / `LIST_MAX_RESULTS = 1_000_000` bounds (`cold_store.py:78-79`) whose breach is an error naming the listing. Pins: `tests/test_cold_store.py:3676` ff. |
| S3 `ListObjects` | `cold_store.py:1978` (`s3_list_objects`), bound check :1993 | **Conforms** (#6602). Advancing-token guard plus the same page/result bounds with a named error. |
| Repair/drill scripts | `scripts/repair_dump.py`, `scripts/drill_compare.py` | No page loops of their own (no `pageToken` handling); they read through the client's listings above. |

## Aggregator / projector — `pm-stack`

| Loop | Evidence | Disposition |
|---|---|---|
| Projector GitHub list follower | `pm-stack/projector/src/github.ts:203` (`listResource`); consumers `reconcile.ts:59,78,102,149,161` | **Filed — #9010.** Follows `Link: rel="next"` in an unbounded `while (path)` loop accumulating all pages: rule 2 violated (no page/result bound, no breach error). |
| Aggregator project-pool fetch | `pm-stack/aggregator/src/projects.ts` (~:395) | **Conforms.** 10-page cap; truncation throws an error naming the pool `totalCount`. |
| Aggregator needs-Drew census | `pm-stack/aggregator/src/needs-drew.ts:198` | **Conforms.** `MAX_PAGES = 10` (:77); completeness read via `total_count`; GitHub `incomplete_results` is marked in the source status instead of hidden. |
| Aggregator event pages | `pm-stack/aggregator/src/events.ts:530-556` | **Deliberately local.** Server-side keyset pagination over a snapshot pinned by `snapshotAt`; the cursor is validated field-by-field (`decodeEventCursor`), and an invalid cursor restarts the complete snapshot read rather than filtering a partial one. The rendered page also marks windowed slices explicitly (`render.ts:6046`). |
| Aggregator incident cursor | `pm-stack/aggregator/src/alerts.ts:403-429` | **Deliberately local.** A stored (timestamp, id) progress cursor over the aggregator's own incident store; advances monotonically; not a network page loop. |

## Fleet crates — `pkgs/fleet`

| Loop | Evidence | Disposition |
|---|---|---|
| dispatch-test `scan` | `pkgs/fleet/crates/fleet-dispatch-test/src/lib.rs:380,449-485` | **Conforms** (#6605). `fetch_all_pages` for list reads; `total_count` for counts, per-site justification in the code comments. |
| Queue fallback marker read | `pkgs/fleet/crates/fleet-queue/src/fallback.rs:571` (`fetch_markers`) | **Filed — #9012.** Bounded (`page in 1..=10`, per_page=100) but exiting on the cap returns the partial marker set as if complete, and markers are claim/lease state — rule 3 violated at the cap. |
| Queue broker `/list` | `pkgs/fleet/crates/fleet-queue/src/queue_broker.rs:532-539,1169` | **Conforms.** `limit` validated to `1..=max` before use (#6419), explicit `offset`, and the PostgREST content-range is returned to the caller, so completeness is visible rather than assumed. |
| Placement run correlation | `pkgs/fleet/crates/fleet-queue/src/placement.rs:759` (`per_page=100`), :1322 (`per_page=1`) | **Deliberately bounded.** A snapshot-correlation read around this dispatch (reason recorded in the code comment, Codex P1 on PR #8325): it snapshots the workflow's recent run ids before/after submitting; runs outside the window are out of the correlation by design. The `per_page=1` read is an existence probe. |

## Operator scripts — `infra/`, `scripts/`

| Loop | Evidence | Disposition |
|---|---|---|
| cargo-target-janitor claimed-task read | `infra/cargo-target-janitor/bin/cargo_target_janitor.py:273-351` | **Conforms** (#6618). Keyset pagination (`id=gt.`) with `MAX_CLAIM_PAGES = 100`; exceeding it fails closed. |
| cargo-target-janitor task-states read | same file :366-383 | **Conforms** (#6618 disposition, recorded in code): chunked `id=in.(…)` reads sized so no chunk page can truncate. |
| cargo-target-janitor claimed-issue + open-PR reads | same file :481-595 | **Conforms** (#6618). Paginated to a short page with `MAX_PR_PAGES = 200`; a non-terminating read fails closed. Pins: `tests/test_cargo_target_janitor.py:271` (`test_queue_claims_pages_past_1000`), :302 (`…fails_closed_when_pages_never_end`), :398/:410 (open-PR pair). This is disposition (d), second half: #6618 landed fail-closed-or-complete, never page-as-set. |
| yard-reaper open-PR branches | `infra/yard-reaper/bin/yard_reaper.py:592-631` | **Conforms** (the #6618 pattern, cited in its docstring). `MAX_PR_PAGES = 200`; non-termination raises `SafetyError` rather than trusting a truncated never-delete set. |
| fleet-paperwork installations | `infra/fleet-paperwork/fleet_paperwork.py:253-285` | **Deliberately unbounded — reason named.** The endpoint returns a bare array with no total; termination is the short page. The population is the App's own installations (tens, not a growing listing), and the sibling loops in the same file are total-driven (:309) or capped (:403). Residual risk is a misbehaving GitHub endpoint, not data growth. |
| fleet-paperwork installation repositories | same file :309-345 (`assert_repo_in_scope`) | **Conforms.** Terminates on the response's own `total_count` (the count-by-total pattern) or an empty page; malformed shapes `die` loudly. |
| fleet-paperwork issue search | same file :395-415 | **Conforms.** `range(1, 11)` — the Search API's own 1000-result ceiling, named in the docstring. |
| program_tending roadmap state read | `scripts/program_tending.py:431` | **Conforms by degradation.** A single page, but refs it cannot see are reported `unverified` — never as a wrong state (the docstring names this). Truncation degrades the answer's certainty, visibly. |
| program_tending epics + open-PR count | `scripts/program_tending.py:285,670` | **Filed — #9011.** Single `per_page=100` page iterated as the epic set, and `len(prs)` presented as the open-PR count (the #6605 shape). |
| Coverage / repo-universe enumerations | `scripts/fleet/queue-reconcile/check_coverage.py:91-99`; `scripts/fleet/repo-universe/check_issues.py:151`; `scripts/fleet/repo-universe/generate.py` | **Conforms.** Complete reads via `gh api --paginate` (gh follows Link headers to the last page and fails loudly on error); the completeness intent is stated in their docs. |

## Required dispositions

- **(a) github-backup shape filings.** #6575 (drain queue-event shape)
  and #6606 (reconcile clone-status line shape) are both **CLOSED** —
  dispositioned under #3068's shape contract / #3282's estate, exactly
  as this contract's boundary requires. Nothing in this inventory or
  the contract claims them; their evidence is shape evidence, not
  pagination evidence.
- **(b) The #3283 cross-reference.** #3283 (Phase 3 audit of #3282)
  is **CLOSED**. The division: the pagination contract (#6622, this
  inventory) owns github-backup's page loops on the three pagination
  axes — the queue-events drain and the owner inventory, dispositioned
  above; #3282/#3283 own github-backup state access (validity,
  outcomes, alarms) including the parsed-shape defects in (a).
  Neither audit re-covers the other's ground, and no github-backup
  loop falls between them: the system's only page followers are the
  two loops above.
- **(c) Sender-side cursor emission.** For the Phase 1 protocol, the
  senders (host producer in `world_session`, web producer in
  `world_ws`) always emit `next_after` as a JSON string or `null` —
  see the pkgs/id table. A receiver-side refusal therefore cannot be
  triggered by a conforming sender; the compatibility break the
  contract warns about does not exist in-tree.
- **(d) Pin verification.** Phase 1 landed cursor-type refusal as
  recorded on #6622 — pinned by the world_net/world_ssh tests cited
  in the pkgs/id table. Phase 3 landed #6618 as
  fail-closed-or-complete — pinned by the janitor tests cited in the
  operator-scripts table (`test_queue_claims_pages_past_1000`,
  `test_queue_claims_fails_closed_when_pages_never_end`,
  `test_open_pr_branches_pages_past_500`,
  `test_open_pr_branches_fails_closed_when_pages_never_end`).

## Contract statement

The three-rule statement now also stands in the Phase 1–3 systems' own
docs (this PR): `world_net.rs` module docs (pkgs/id),
`infra/github-backup/README.md`, `pkgs/cold-store-reaper/README.md`,
`fleet-dispatch-test` crate docs, and
`infra/cargo-target-janitor/README.md` — in each case the same terms:
complete, bounded, or say so; cursor validated where read; volume
bounded with a named-listing error; truncation never silent.
