# HYD-04 — Full bounded hydration adapter and navigable summary

Issue: [#9461](https://github.com/developing-today/code/issues/9461)
(slice `HYD-04` of [#9207](https://github.com/developing-today/code/issues/9207),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **design proposal — not an authorization to implement.**
This record elaborates the adapter-and-summary slice of the merged
design package (`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`,
PR #9225) into the source plan, fetch orchestration, status mapping,
head binding, budgets, trust boundary and `summary.md` contract a
separately scoped implementation issue can be filed against. HYD-01
freezes what a manifest _says_; HYD-03 publishes a generation; HYD-05
walks pages. HYD-04 is the slice that decides **what to fetch for a
piece of work, in what order, how each outcome becomes a per-source
status, and what the agent reads first**. It authorizes no code,
schema, migration, deployment or live acceptance; those remain
separately gated stages (§11).

Governing decisions consumed, not re-opened:

- **Component #9207** (`components/9207-context-hydration-incremental-activity.md`):
  §2.1 identity and safe paths (canonical `WorkId`, repo-scoped source
  refs so `alpha/repo#10` and `beta/repo#10` never collide); §2.2 the
  required content list (queue fields, issue/PR detail, comments with
  create/update state, timeline, linked PRs, head/base SHA, checks,
  reviews and review threads, commits with author/committer identity
  and times and parents, changed paths with rename source, diff hunks
  with old/new ranges, typed dependency refs, previous notes,
  checkpoints and cursor snapshot "with privacy boundaries"); §2.3
  one captured `head_sha` per PR; §3 retrieval bounds (`Link` header is
  continuation authority, page size 30 default / 100 max, PR commits
  capped at 250 and files at 3000, hitting a cap is `truncated`,
  budgets for bytes, diff bytes, time and requests, streaming with
  bounded memory); §4 "Trust boundary" (GitHub text is untrusted data
  labelled by source, actor and time; it cannot authorize anything;
  private notes are exported only by an explicit note-mirror
  operation); §5 failure matrix rows for same-number-cross-repo, huge
  diff, private-note leak ("immediate release blocker") and malicious
  body; and the HYD-04 slice row: orchestration in `work/context.rs`,
  a read-policy call into `work/authority.rs`, the GitHub adapter
  staying in `pkgs/fleet/crates/fleet/src/gh.rs` ("no `fleet-gh`
  crate"), an oracle fixture with an unavailable review endpoint and a
  renamed diff with author/committer timestamps and hunk ranges, and
  rollback as an artifact-only read path whose disabling leaves the
  queue intact.
- **FDR-PMO-004** (immutable hydration, per-source cursors,
  attribution): partial or truncated state is never presented as
  complete; cursors advance only after durable processing.
- **HYD-01 record** (PR #15184,
  `doc/2026-11-25T18-52-28Z_design_hyd01_versioned_manifest_result_envelope/`,
  **open at writing**): the frozen `fleet.context-manifest/v1` shape,
  the per-source status set (`complete | partial | unavailable |
forbidden | not_found | truncated`, plus `not_applicable` for
  optional resources), the rule that generation status is _derived_
  from per-source statuses and never asserted, the sibling artifact
  set (`summary.md`, `queue.json`, `issues.json`, `pull_requests.json`,
  `comments.json`, `reviews.json`, `checks.json`, `timeline.json`,
  `commits.json`, `dependencies.json`, `activity-cursors.json`,
  `diffs/pr-<n>.patch`), and the result envelope. This record fills
  those fields and files; it adds no manifest key. If PR #15184 changes
  before merging, this record follows it (§11).
- **HYD-05 record** (`doc/2026-11-23T13-02-35Z_design_hyd05_pagination_api_caps_resumable_durable_reads/`):
  a walk ends in exactly one of `complete`, `truncated` or `incomplete`;
  the continuation is the verbatim `Link rel="next"`; a malformed
  `Link` header is a hard error reported as `incomplete`; source caps
  and Fleet caps both end as `truncated`. That record names "the
  HYD-04 hydration adapter" as a caller that drives walks; HYD-04 is
  that caller and implements no pagination of its own.
- **HYD-06 / HYD-07 records**: revision identity and the four times
  (`occurred_at`, `source_updated_at`, `observed_at`,
  `first_observed_at`); origin and confidence attribution. HYD-04
  reports what those slices produce in the summary's activity section
  and computes neither.
- **CLI-03** (#9439, CLOSED; PR #13396): `resolve_work_inspection_by_id`
  / `_by_ref` and `WorkInspection { identity, workflow, readiness,
ownership_execution, remote_resources }` in `work/lifecycle.rs`,
  with `WorkInspectionLookupError::Ambiguous` for a cross-repo
  reference. HYD-04 starts from that resolution and never from a bare
  number.
- **CEN-04 record** (`doc/2026-11-23T13-51-45Z_design_cen04_schema_owner_migration_compatibility/`):
  a persisted field with no matrix row may not be written; cursors are
  pre-filed in §3.8 as `absent — gated`.
- **`work/contracts.rs` on main**: the `fleet.context-manifest/v1`
  digest domain, `digest`, `canonical_json`, `Outcome::Partial`, and
  the `fleet.result/v1` `CommandResult`.

Verified state at writing (2026-11-25, origin/main `60e2e2d7f`):

- `pkgs/fleet/crates/fleet-queue/src/work/context.rs` and
  `work/activity.rs` **do not exist**. `work/mod.rs` declares
  `authority`, `completion`, `contracts`, `degraded`, `journal`,
  `lifecycle`, `operations`, `priority`, `routing`, `selection` and
  (broker builds) `sqlite_replica`. No code path writes a hydration
  manifest or a `summary.md`.
- `work/authority.rs` is AUTH-04's managed-body-section module; it has
  **no read-policy function** for HYD-04 to call. AUTH-01 (#9429), the
  field-ownership and scoped-authorization contract, is OPEN with no
  design record.
- `work/journal.rs` carries `HydrationRef { generation,
manifest_digest }` on checkpoints with `generation` required to be a
  canonical decimal string, while the component and HYD-01 spell
  `generation_id` as `g-<stable unique id>`. This record does not
  resolve that (it is JRN-03's and HYD-01's), but §9 names it as an
  implementation-blocking mismatch.
- GitHub reads today: `fleet gh` (`fleet/src/gh.rs`: `run`,
  `run_with_provider_capturing`, `api_call`, `is_read_call`) is a
  pass-through to `gh` with token rotation; `fleet-queue/src/fallback.rs`
  (`gh_json`, `fetch_markers`) and `client/fallback_stack.rs` page issue
  comments; `queue_broker.rs` probes issue/PR state for dependency
  rechecks. **Nothing fetches reviews, review threads, check runs,
  the GitHub timeline, commits or diffs for context.**
- Gate state: PR #9225 MERGED. HYD-01 #9458, HYD-02 #9459, HYD-03
  #9460, HYD-05 #9507, HYD-06 #9508, HYD-07 #9509, HYD-08 #9510 and
  HYD-09 #9462 OPEN (HYD-05/06/07 have merged records; HYD-01's record
  is PR #15184, open). CLI-03 #9439 CLOSED. AUTH-01 #9429 OPEN.
  CEN-04 #9434 OPEN with record merged. CEN-07 #9437 OPEN. Per the
  sibling pattern (HYD-05/06/07, PRI-03/04/07, SEL-02 landed as
  records while their implementation prerequisites stood open), these
  gate the implementation issue (§11), not this record.

## 1. Intent

An agent that claims work should be able to open one short file and
know, without reading everything: what the work is, what state its
issue and PR are in, what is blocking it, what reviewers and checks
said, what changed in the code and how to read it, what happened
since it last looked — and, above all, **what is missing from that
picture and why**. HYD-04 is the slice that produces that, by:

1. deciding the **source plan** — the required and optional sources
   for this work's shape (§2);
2. **orchestrating** the fetches through the existing adapter seam,
   under one budget, with PR artifacts bound to one head (§3, §4, §5);
3. **mapping every outcome** to exactly one per-source status, so
   "missing", "forbidden" and "empty" never blur (§3.3);
4. **rendering `summary.md`** deterministically from the published
   artifacts, stating missing data as missing (§7);
5. doing all of that while treating GitHub text as data and never
   letting private notes into the bundle (§6).

## 2. Ownership: who writes what

| Concern                                                                         | Owner                   | HYD-04's relation                                                      |
| ------------------------------------------------------------------------------- | ----------------------- | ---------------------------------------------------------------------- |
| Manifest schema, per-source status set, derived status, result envelope         | HYD-01 (#9458)          | Fills them; adds no key or status.                                     |
| Source keys and artifact paths (repo-scoped, traversal-safe)                    | HYD-02 (#9459)          | Asks for a key per source; builds no path itself.                      |
| Staging, manifest-last write, atomic `current` swap                             | HYD-03 (#9460)          | Hands a complete staged set to the publisher; never touches `current`. |
| Page walks, continuation, caps, cursor commits                                  | HYD-05 (#9507)          | Starts walks and reads their end state; no pagination code.            |
| Revisions, tombstones, attribution                                              | HYD-06/07 (#9508/#9509) | Reports their results in the summary; computes neither.                |
| Commands, flags, refresh/watch, timeouts as CLI outcomes                        | HYD-08 (#9510)          | Exposes one library entry point; registers no command.                 |
| Work identity resolution                                                        | CLI-03 (landed)         | Starts from `WorkInspection`; never from a bare number.                |
| Which fields a principal may read; private-note boundary                        | AUTH-01 (#9429)         | Calls its read-policy decision per field class (§6.2).                 |
| HTTP, auth, token rotation, rate-limit handling                                 | `fleet/src/gh.rs`       | Calls it; adds endpoints there, no new client or crate.                |
| **Source plan, orchestration, outcome mapping, head binding, budgets, summary** | **HYD-04 (this)**       | Owns them, in `work/context.rs`.                                       |

## 3. The source plan

### 3.1 Starting point

Hydration takes a resolved `WorkInspection` (CLI-03). An `Ambiguous`
or not-found lookup is the HYD-01 envelope's top-level identity error
and fetches nothing. `remote_resources` gives the linked issue and
PRs, each as a provider-native identity (owner, repo, number, stable
remote id). A PR discovered later (from the issue timeline) joins the
plan only if its repository and number are read from the timeline
event's own `source.issue.repository` and `number` fields, never by
matching a number in text.

### 3.2 Sources

Every row is one manifest `sources[]` entry. **Required** sources
decide generation status (HYD-01 §4); **optional** ones are recorded
as `not_applicable` when the work has nothing for them, never left
out.

| Source (`endpoint` value)      | Request (REST, via `gh.rs`)                                                                            | Required when         | Artifact                |
| ------------------------------ | ------------------------------------------------------------------------------------------------------ | --------------------- | ----------------------- |
| `queue-row`                    | the queue store's own read of the work row (every field, custom columns, status, reason, owner, lease) | always                | `queue.json`            |
| `issue-detail`                 | `GET /repos/{o}/{r}/issues/{n}`                                                                        | work has an issue     | `issues.json`           |
| `issue-comments`               | `GET /repos/{o}/{r}/issues/{n}/comments` (walked)                                                      | work has an issue     | `comments.json`         |
| `issue-timeline`               | `GET /repos/{o}/{r}/issues/{n}/timeline` (walked)                                                      | work has an issue     | `timeline.json`         |
| `pull-request-detail`          | `GET /repos/{o}/{r}/pulls/{n}`                                                                         | per linked PR         | `pull_requests.json`    |
| `pull-request-commits`         | `GET /repos/{o}/{r}/pulls/{n}/commits` (walked; source cap 250)                                        | per linked PR         | `commits.json`          |
| `pull-request-files`           | `GET /repos/{o}/{r}/pulls/{n}/files` (walked; source cap 3000)                                         | per linked PR         | `pull_requests.json`    |
| `pull-request-diff`            | `GET /repos/{o}/{r}/compare/{base_sha}...{head_sha}` with the diff media type                          | per linked PR         | `diffs/pr-<n>.patch`    |
| `pull-request-reviews`         | `GET /repos/{o}/{r}/pulls/{n}/reviews` (walked)                                                        | per linked PR         | `reviews.json`          |
| `pull-request-review-comments` | `GET /repos/{o}/{r}/pulls/{n}/comments` (walked)                                                       | per linked PR         | `reviews.json`          |
| `pull-request-conversation`    | `GET /repos/{o}/{r}/issues/{n}/comments` for the PR (walked)                                           | per linked PR         | `comments.json`         |
| `checks`                       | `GET /repos/{o}/{r}/commits/{head_sha}/check-runs` and `.../status`                                    | per linked PR         | `checks.json`           |
| `dependencies`                 | the queue's typed dependency refs (#8276 `blocked_deps`, journal `BlockerEdgeRef`)                     | always (may be empty) | `dependencies.json`     |
| `checkpoint`                   | the latest journal checkpoint's `HydrationRef` and `CursorRef`s                                        | optional              | `activity-cursors.json` |
| `activity-cursors`             | HYD-05's cursor snapshot for this work                                                                 | optional until HYD-05 | `activity-cursors.json` |

The diff is fetched from the **compare** endpoint pinned by both SHAs
rather than the PR's own diff media type, because the PR form follows
whatever the head is at request time and cannot be bound to a
captured head (§4). Review threads' resolved state is not in the REST
review-comments payload; until an adapter path for it exists, threads
are recorded with `resolved: unknown` and the summary says so — never
"no unresolved threads".

### 3.3 Outcome → per-source status

Each fetch ends in exactly one status; the rule is applied by HYD-04,
not by the adapter, so every slice sees one mapping:

| Adapter outcome                                                    | Status        | `truncation_reason` / `error`                                    |
| ------------------------------------------------------------------ | ------------- | ---------------------------------------------------------------- |
| 200, walk `complete`, any record count including **zero**          | `complete`    | —                                                                |
| 404, or 410                                                        | `not_found`   | `error: not_found` / `gone`                                      |
| 403 or 401 that is not a rate limit                                | `forbidden`   | `error: forbidden`                                               |
| 403/429 rate limit, secondary limit, 5xx, timeout, transport error | `unavailable` | `error: rate_limited` / `server_error` / `timeout`               |
| walk `truncated` at a source cap (250 commits, 3000 files)         | `truncated`   | `source_cap`                                                     |
| walk `truncated` at a Fleet budget (§5)                            | `truncated`   | `request_budget` / `byte_budget` / `diff_budget` / `time_budget` |
| walk `incomplete` (malformed `Link`, page lost after retry)        | `partial`     | `incomplete_walk`                                                |
| response identity ≠ requested identity (§3.4)                      | `partial`     | `identity_mismatch`; nothing persisted                           |
| 301/307 to another repository                                      | `partial`     | `moved`; the redirect is not followed                            |
| PR head moved and the bounded retry did not settle (§4)            | `partial`     | `stale_head`                                                     |
| policy withheld the whole source (§6.2)                            | `forbidden`   | `error: policy`                                                  |

**Missing vs empty** is the first row against the second: an issue with
no comments is `complete` with `records_seen: 0`; a comments endpoint
that 404s is `not_found`. The summary renders the two differently
(§7.2).

### 3.4 Same number, other repository

A response is persisted only if it proves it is the thing asked for:
an issue or PR's `repository_url` / `base.repo.full_name` and `number`
must equal the requested ref, and its stable remote id must equal
`remote_resources`' id where one was recorded. A mismatch is
`identity_mismatch`: nothing from that response is written, and the
manifest says why. Artifact keys come from HYD-02 and include the
repository, so two repositories' `#10` cannot share a file even when
both are legitimately in one bundle.

## 4. Head binding

For each linked PR:

1. Read `pull-request-detail` first and capture `head.sha` and
   `base.sha`. These go in every PR source's
   `remote_version.head_sha`.
2. Fetch the pinned sources by SHA (`checks` by `head_sha`, the diff
   by `base_sha...head_sha`).
3. Fetch the unpinnable sources (`commits`, `files`, reviews, review
   comments, conversation).
4. Re-read `pull-request-detail`. If `head.sha` is unchanged, the set
   is bound. If it moved, discard the PR's staged artifacts and repeat
   from step 1 **once**, within the deadline; if it moves again, every
   PR source for that PR is `partial / stale_head` and the summary
   says the PR was moving while it was read.

Old diff and new head metadata are never combined (HYD-01 §3). The
cost is one extra detail read per PR and at most one retry.

## 5. Budgets and memory

One budget object is passed to the whole hydration, versioned and
named in the manifest's per-source `query_fingerprint` inputs so two
runs under different budgets never look alike. Proposed defaults (the
implementation issue confirms or changes them, with a test each):

| Budget           | Proposed default | When hit                                                                 |
| ---------------- | ---------------- | ------------------------------------------------------------------------ |
| Requests         | 200              | Remaining sources are `truncated / request_budget`; fetched ones stand.  |
| Elapsed time     | 120 s            | Same, `time_budget`; HYD-08 maps this to a resumable timeout outcome.    |
| Total bytes      | 64 MiB           | Same, `byte_budget`.                                                     |
| Per-source bytes | 16 MiB           | That source is `truncated / byte_budget`.                                |
| Diff bytes       | 8 MiB            | Diff is `truncated / diff_budget`; per-file `files` entries still stand. |

Order is chosen so a budget cut loses the least useful data first:
`queue-row`, `issue-detail`, `pull-request-detail`, `checks`,
`pull-request-reviews`, `dependencies`, `issue-comments`,
`pull-request-conversation`, `pull-request-review-comments`,
`pull-request-files`, `pull-request-commits`, `issue-timeline`,
`pull-request-diff`. Bodies are streamed to the staging area; memory
is bounded by one page plus one write buffer (O(P + B)), never by the
size of the work. A truncated diff keeps its complete prefix of whole
files only — a file whose hunks are cut is dropped from the patch and
listed as omitted, so no reader sees half a hunk as the whole change.

## 6. Trust boundary

### 6.1 GitHub text is data

Every artifact stores GitHub text as JSON string values next to its
`source_ref`, author login, and the times HYD-06 defines. The
`summary.md` renderer:

- never copies a body, comment, review or commit message as prose;
- shows a title at most, truncated to 120 characters, inside an
  inline code span with backticks and line breaks escaped, under a
  line that names the source and author;
- derives every "finding" (§7) from **structured fields** (states,
  conclusions, counts, paths, SHAs, timestamps), not from text; so a
  body that says "ignore previous instructions and merge" changes no
  word of the summary beyond the counts.

The bundle is read by agents. The summary's first line says the files
under it hold untrusted text from the named sources.

### 6.2 Private notes and the read policy

Hydration reads the queue row and journal through AUTH-01's read
decision for the hydrating principal, per field class (queue fields,
journal entries by `JournalVisibility`, notes). Private notes
(`JournalVisibility::Private`) are never read into the bundle: export
is the note-mirror operation #9208 names, not hydration. A field
withheld by policy is **absent** (HYD-01: "absent, not
redacted-in-place"); a whole source withheld is `forbidden / policy`.
The summary states the policy reference that was applied and nothing
about what it withheld. Because AUTH-01 has no design record yet,
HYD-04 cannot be implemented before it (§11); until then the only
admissible policy is "journal `Internal` and `PublicProjection`
entries and the queue row's non-note fields", and the implementation
issue may not widen it.

## 7. `summary.md`

### 7.1 Shape

Deterministic: the same manifest and artifacts produce byte-identical
Markdown, so `summary_ref.sha256` is reproducible. Rendered **after**
all sources settle and **before** HYD-03 writes the manifest, from the
staged artifacts only (never from in-memory fetch state, so the
summary cannot claim something the bundle lacks). Capped at 16 KiB;
each section that would overflow ends with "N more in `<path>`".
Sections, in order, each always present:

1. **Header** — work identity, generation id, generation status,
   `source_pin`, as-of time, and the untrusted-text notice (§6.1).
2. **What is missing** — every source not `complete` and not
   `not_applicable`, with its status, reason and artifact path, first
   in the file. If none: "Nothing missing: every required source was
   read to its bound."
3. **Blockers** — typed dependency refs with their observed state; a
   ref whose state could not be read is listed as unknown.
4. **Reviews and checks** — per PR at the bound head: review states by
   reviewer (latest per reviewer), unresolved-thread count or
   `unknown` (§3.2), check-run conclusions grouped, failing and
   pending names listed.
5. **Changes** — per PR: base and head SHA, commit count (with
   `truncated at 250` when so), files changed, and per file
   `old_path → new_path` for renames, additions/deletions, hunk count
   with their `@@ -a,b +c,d @@` ranges, and the diff path; commits
   listed with short SHA, author and committer identity and both
   timestamps.
6. **Activity since checkpoint** — counts HYD-06/07 provide (new,
   revised, tombstoned; by origin), or "not available: HYD-05/06
   cursors absent".
7. **Files** — every artifact path with bytes and SHA-256.
8. **Next investigations** — mechanical only: refetch commands for
   each missing source, "read diff of the N largest files", "PR head
   moved during hydration". No advice drawn from text.

### 7.2 Missing is never empty

For every section, a required source that is not `complete` is
rendered as "not available (`<status>`: `<reason>`)", never as an
empty list, a zero count or "none". The test fixture of the slice
row — reviews unavailable — must render "Reviews: not available
(`unavailable`: `server_error`)" and must not contain "no reviews".

## 8. Entry point, outcomes, rollback

- **One library entry point** in `work/context.rs`:
  `hydrate(inspection, plan_options, budget, adapter, read_policy,
publisher) -> HydrationResult`, where `HydrationResult` is HYD-01's
  envelope. HYD-08 wraps it in commands; HYD-04 registers none.
- **Outcomes** follow HYD-01 §5: generation `complete` or `partial` is
  a published result with per-source detail (`partial` maps to the
  shared `Outcome::Partial`); `failed` names the still-current prior
  generation. Identity failure, an unknown manifest version on read,
  and an invalid digest or path are the only top-level errors.
- **Writes.** HYD-04 writes only into HYD-03's staging area. It writes
  no queue field, no journal entry and no cursor (HYD-05 commits
  cursors). It performs no GitHub write: every request in §3.2 is a
  `GET`, and the implementation asserts each call is classified a read
  by `gh.rs`'s `is_read_call` / API-method check before sending it.
- **Rollback.** Disable the entry point (feature flag or command
  removal): published generations remain readable, the queue is
  untouched, and no other module depends on a fresh hydration to
  function. Nothing is deleted.

## 9. Persisted fields

**No queue or journal field.** The bundle is files whose every byte is
covered by HYD-01's manifest; CEN-04's field-admission gate is engaged
only through HYD-05's cursor row (§3.8, `absent — gated`), which this
slice does not write. One conflict must be settled before
implementation and is not this slice's to settle: `journal.rs`
`HydrationRef.generation` requires a canonical decimal string, while
HYD-01's `generation_id` is `g-<stable unique id>`. Either JRN-03 or
HYD-01 changes; HYD-04's implementation issue cites whichever record
resolves it.

## 10. Acceptance oracle, specified

All tests live in `work/context.rs`'s test module and run with the
slice body's command form,
`cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker <name>`.
Each name must first appear in that command's `-- --list` output; a
filter matching zero tests is not acceptance. Builds and test runs go
through remote dispatch, per the repository's remote-build rule. All
fixtures are recorded adapter responses replayed through a fake
adapter at the `gh.rs` seam; a fixture passing is not live GitHub
evidence (component §6), and live acceptance is HYD-09's.

| Test                                                               | Oracle                                                                                                                                                                                                                                                                                                                                                          |
| ------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `pmo_hyd_summary_reports_partial_and_diff_provenance` (first, red) | The slice row's fixture: reviews endpoint returns 502; the PR renames `a/old.rs → a/new.rs` with two hunks; commits have distinct author/committer identities and times. Generation `partial`; reviews `unavailable / server_error`; summary lists reviews as not available, shows the rename, both hunk ranges, base/head SHA, and both timestamps per commit. |
| `pmo_hyd_missing_vs_empty`                                         | Issue with zero comments → `complete`, `records_seen 0`, summary "0 comments"; comments endpoint 404 → `not_found`, summary "not available (`not_found`)". Never "no comments" for the second.                                                                                                                                                                  |
| `pmo_hyd_forbidden_source_is_forbidden_not_empty`                  | 403 on checks → `forbidden`; generation `partial`; no checks artifact bytes beyond the manifest entry; summary names it in "What is missing".                                                                                                                                                                                                                   |
| `pmo_hyd_same_number_cross_repo_never_conflated`                   | Work linked to `alpha/repo#10`; adapter answers with `beta/repo#10`'s payload → `identity_mismatch`, nothing persisted; a work linked to both keeps two distinct artifact keys.                                                                                                                                                                                 |
| `pmo_hyd_head_moved_retries_once_then_stale`                       | Head changes between first and last detail read → one retry; changes again → every PR source `partial / stale_head`; diff and commit artifacts from different heads never coexist.                                                                                                                                                                              |
| `pmo_hyd_caps_and_budgets_truncate_explicitly`                     | 260 commits → `truncated / source_cap`; diff over 8 MiB → whole-file prefix kept, cut files listed as omitted, `truncated / diff_budget`; request budget hit → remaining sources `truncated / request_budget` in §5's order.                                                                                                                                    |
| `pmo_hyd_hostile_text_does_not_reach_summary`                      | Bodies, comments and a commit message containing instructions, Markdown links, HTML and backtick runs: the summary contains none of those strings except an escaped, 120-character-capped title in a code span; findings unchanged vs a benign fixture.                                                                                                         |
| `pmo_hyd_private_note_never_in_bundle`                             | A private journal note holding a canary string: no artifact, manifest or summary byte contains it; the summary names the policy reference only.                                                                                                                                                                                                                 |
| `pmo_hyd_summary_is_deterministic`                                 | Same staged artifacts twice → byte-identical `summary.md` and `summary_ref.sha256`; the summary is rendered from staging, so a fetched-but-not-staged record never appears in it.                                                                                                                                                                               |
| `pmo_hyd_reads_only`                                               | Every request the fake adapter receives is a `GET` classified as a read by `gh.rs`; a non-read request fails the test.                                                                                                                                                                                                                                          |
| `pmo_hyd_disable_leaves_queue_and_generations`                     | With the entry point disabled, the queue store's full-table fingerprint and the published generations are byte-identical to before.                                                                                                                                                                                                                             |

Negative coverage from the slice body, mapped: 403 →
`forbidden_source_is_forbidden_not_empty`; missing vs empty →
`missing_vs_empty`; same PR number cross-repo →
`same_number_cross_repo_never_conflated`; hostile instructions →
`hostile_text_does_not_reach_summary`; private note leakage →
`private_note_never_in_bundle`.

## 11. Gates and sequencing

Implementation against this record starts only when **all** of the
following hold:

1. This record is merged, and **HYD-01's record (PR #15184) is merged**
   — HYD-04 fills its schema; if the merged HYD-01 differs from §3–§8's
   reading of it, this record is amended first.
2. A separately scoped implementation issue is filed against it,
   carrying §10's oracles as its acceptance. #9461 itself authorizes
   no code; it stays open as the slice tracker.
3. The slice body's hard blockers: **HYD-01** implemented (manifest
   type, status derivation); **CLI-03** (met: #9439 closed, PR #13396);
   **AUTH-01** (#9429) designed and implemented far enough to supply
   the read decision of §6.2; **CEN-04** (met for design: record
   merged).
4. HYD-02 (keys/paths) and HYD-03 (publisher) are implemented — HYD-04
   calls both — and HYD-05's walk is available for every "walked"
   source in §3.2. The component lists HYD-01/03 as HYD-04's
   prerequisites; HYD-02 and HYD-05 are the concrete calls this record
   adds to that list.
5. The `HydrationRef.generation` mismatch of §9 is resolved.
6. CEN-07 (#9437), which HYD-01's record names as the remaining #9198
   implementation gate, is met or explicitly waived for this slice.

Live GitHub acceptance, the performance envelope and migration and
rollback proof at scale are HYD-09's; commands and timeouts are
HYD-08's.

**Schedules:** this record creates, changes, and retires no schedule
on any surface.

## 12. Requirement trace (design-level)

Full per-occurrence text for every allocated ID is in
`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/census/requirements-crosswalk.jsonl`;
this table records which part of this design speaks to each group —
design coverage, not implementation.

| IDs                                                            | Where addressed                                                                                                        |
| -------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------- |
| C-R006, C-R113, P-R016, O-R029                                 | §3.2, §7 — the bundle's sources and artifacts, and the navigable summary with provenance and freshness                 |
| C-R115, P-R050, P-R065                                         | §3.2, §4, §7.1 — source versions and hashes, linked issues/PRs, reviews/checks, commit and diff metadata               |
| C-R116                                                         | §3.3, §7.2, `missing_vs_empty`, first oracle — incomplete never presented as complete or empty                         |
| C-R009, C-R110, C-R150, C-R151                                 | §1, §8 — `hydrate` as the step scenario A/B's `--hydrate full` and `resume --hydrate incremental` call                 |
| H-R016, P-R027                                                 | §10 — hydration scenarios (fresh work, partial, stale head, budgets) as named oracles; the rest are other slices'      |
| C-R005, C-R107, C-R108, H-R017, H-R024, H-R031, O-R023, P-R009 | §2, §8 — one library entry point; the command family, flags and their compatibility coverage stay with HYD-08 / CLI-13 |
| P-R051, P-R066                                                 | §7.1 item 6 — activity counts reported from HYD-05/06/07; their acceptance is those slices'                            |

## 13. Non-goals

- No manifest key, status value or envelope field beyond HYD-01's.
- No path or key construction (HYD-02), publication or `current`
  pointer (HYD-03), pagination or cursor storage (HYD-05), revision or
  attribution logic (HYD-06/07), command registration, refresh or
  watch (HYD-08), live acceptance or performance proof (HYD-09).
- No read-policy design (AUTH-01) and no note-mirror export (#9208).
- No GitHub write of any kind, no new GitHub client or crate, no new
  queue, database, scheduler, service, resolver or executable, and no
  persisted queue or journal field.
- No summarization of text: the summary is derived from structured
  fields only.
- No change to #6533 or its live acceptance.
