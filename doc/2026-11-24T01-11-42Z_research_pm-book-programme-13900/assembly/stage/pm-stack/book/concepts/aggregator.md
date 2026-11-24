# The aggregator: one state, many sources

The aggregator is a Bun application on the laptop host `amd`, served as the systemd user unit `pm-aggregator` on port 4870. Port 4871 is its second, loopback-only listener — a separate `Bun.serve` on `127.0.0.1:4871` inside the aggregator process, commented in code: cross-process publishers get a separate loopback-only listener; it is never added to `PM_AGG_HOSTS` and never exposed through the Cloudflare tunnel (survey top-up 1, T5). Its only route is `POST /publish`, the only write seam for cross-process freshness publishers, which accepts a bounded (16 KiB) JSON freshness notice into the aggregator's `FreshnessStore` and replies 202 — no source payload is accepted or returned. The known publisher is the projector. Browser-facing freshness delivery is a different seam on the main 4870 listener (`GET /freshness/stream` SSE and `GET /api/freshness-events.json`). It is the most active component in the stack — its last commit in the survey is 2026-11-23 — and it is the stack's reader, publisher, question-raiser, and answer-recorder. Its own one-line account of itself stands: the one screen Drew lives in while the fleet works in GitHub, Kaneo, and Pad underneath (`pm-stack/aggregator/src/program.ts`).

This chapter states where its one state comes from, how it is published, and where the aggregator's authority stops. The page-by-page census is reference material; the question machinery is in [Question sets and the answer-back flow](questions-and-answers.md).

## The sources

The aggregator's sources are adapters in `pm-stack/aggregator/src/sources/`. Each source is read; none is written by the read.

| Source | How it is read | What it contributes |
|---|---|---|
| Kaneo | Sign-in at `/api/auth/sign-in/email`, then workspace, project, and paginated task reads in `live.ts` | Board items with subject `kaneo:&lt;slug&gt;/&lt;taskId&gt;` and deep links built by `kaneoTaskUrl()`; poll diffs feed activity and freshness, with gap and recovery tracking |
| GitHub Project #4 | Read verbatim through the fleet feed in `projects.ts` | Project pool items carrying Status, Area, Horizon, and Kind (`Task` or `Epic`) as single-select fields |
| Grist | `GET &lt;grist&gt;/api/docs/&lt;docId&gt;/tables/Roadmap/records` with a bearer API key, in `live.ts` | Roadmap rows with subject `grist:&lt;docId&gt;/Roadmap/&lt;recId&gt;` |
| Repository roadmap files | `planning/roadmaps/*.md` in `developing-today/code` and `ROADMAP.md` in `dezren39/darwin-nix`, in `repo-roadmap.ts` | Roadmap rows that win over Grist on a title collision; the Grist duplicate is noted in provenance and its links unioned in (#1738) |
| Projector | `aggregator/src/sources/projector.ts` | Links and latest comments per subject. See [The projector and projections](projector.md) |
| ready-for-agent | `aggregator/src/sources/ready-for-agent.ts`, configured by `PM_AGG_RFA_URL` | Dispatch UI state for the four configured repositories |
| The fleet feed and queue | Feed collector scripts and queue reads | Session identity, orchestration state, queue position |
| Quota and billing sources | About 25 quota modules and the gather and billing scripts | Pool quotas, PAYG dollars, SaaS usage against limits, charge candidates |

Two sources the reader expects are absent, deliberately:

- **Pad contributes no data.** There is no source adapter for Pad content. The aggregator carries a Pad base URL in configuration (default `http://127.0.0.1:7777`), a Pad service link, a health and topology probe, and accepts `pad` as a stack-token target alongside `kaneo` and `grist`. The README sentence "the aggregator reads no Pad data" is broader than the code: the reconciled statement is that the aggregator reads no Pad *planning data* and does probe Pad for health (D3).
- **The aggregator makes no model calls.** No LLM calls and no embedding calls exist in its source. Its `/search` page is a title and reference search over its own projections — board, roadmap, registry sessions, DNS, services, and tier items by title or linked issue or pull request reference — not a vector search.

## From sources to one state

Reads become typed items in `pm-stack/aggregator/src/model.ts`. Subjects join the sources: `gh:` for GitHub, `kaneo:`, `grist:`, `session:`, and `orch:` for orchestration items. Around the items the aggregator builds the structures the pages render:

- **The board**, curated cross-repository work in progress. Kaneo columns map to canonical statuses in `status-taxonomy.ts` (to-do, in-progress, in-flight, in-review, done), and board policy in `board-policy.ts` filters retired cards. When the Kaneo poll succeeds, the board's provenance string records that the data is live from Kaneo, with done items aged out after 7 days.
- **The roadmap**, merged from Grist, repository files, and Project pool and program data, with view filtering shared between the page and its twin. Two horizon vocabularies meet here and are not unified: repository and Grist rows use the buckets Now, Next, Later, Ideas, Ongoing, while aggregator tier items use `today`, `tomorrow`, `next-3-days`, `longer`, `incoming` (D11). The console's roadmap is a second, independent implementation, not a view of this one (D12).
- **Epics and programs.** A pool item with Kind equal to Epic gains an `epicRef` resolved through the native sub-issue parent of its children. Rollups group children by `epicRef` and count done against total; progress is weight, not counts — effort sizes xs, s, m, l carry weights 1, 4, 12, 40, unsized items contribute no weight and are counted visibly as unsized. An epic closure gate refuses closure while its subtree has open children or unmet conditions.
- **Conflicts.** Where sources disagree — a Project Status against derived state is the standing example — the aggregator surfaces both values with the disagreement's age on `/conflicts`, a read-only queue of every live source disagreement. It does not pick a winner silently.
- **Freshness.** Polls and feeds carry ages. The health assembly reports feed section ages and per-page render state; the same assembly feeds the `/health` page and its twin `/api/health`.

## Publication: pages and twins

The page manifest in `pm-stack/aggregator/src/pages.ts` registers 23 pages in four nav groups — Overview, Work, Ops, System — with a primary-or-dropdown curation per page. Six sub-route and detail pages bring the aggregator total to 29 routes: item detail, discuss, one project page, one quota service page, alert preferences, and the decision-set page. Most pages declare a machine twin (`/api/overview.json`, `/api/board.json`, `/api/roadmap.json`, and so on) serving the same data; Datastar fragments under `/frag/` serve partial renders, and `/freshness/stream` is a server-sent-events stream.

Two manifest gaps are part of the reconciled behaviour, not errors to smooth over:

- The decisions page declares no API twin in the manifest, one of only three pages without one — yet the server implements the full decisions write API (`/api/decisions/&lt;setId&gt;/submit`, `/defer`, `/request-info`, `/events`, and `/api/decision-sets`). A reader of the manifest alone would miss the entire write surface (D7).
- The decision-set page `/decisions/&lt;setId&gt;` is missing from the manifest's sub-route parent map, so `manifestRouteFor` returns null for it and its renders sit outside the render registry that `/api/health` reports (D8). The page exists; the health accounting does not cover it.

In public, the dashboard is reached only behind the console gate at `pm.security.cab`: the console proxies non-`/console` requests to the dashboard for a signed-in owner.

## Questions, actions, and the limits of its authority

The aggregator raises the stack's batched questions. Decision sets mirror groups of items from the Muse-side decisions ledger; a set's items carry 1–16 questions each, and resolutions — answered, deferred, request-info — are recorded in the aggregator's decisions store, exported for the ledger, and synced out to the decision-events Worker. Single needs-Drew questions ride notification candidates and are answered at `POST /api/needs-drew/&lt;ledgerId&gt;/answer`, where the per-question token in the URL is the authority. Which file a single-question answer ultimately writes is UNVERIFIED: the code names the question store in `needs-drew.ts` and routes the answer into the same resolution machinery, without unambiguously documenting the store (D9).

Structured answers on tier items go through dashboard actions (`POST /api/actions`, with a batch form): triage verbs including answer, snooze, escalate, and dismiss; status requests; session verbs; and preference settings. Consumers poll for new actions and report outcomes back. The aggregator records; it does not perform the source change.

That limit is the chapter's central rule:

- The aggregator never writes GitHub labels. Its needs-Drew census header states the rule for its own service, and the code holds it. The stack as a whole does write labels — through the console's audited actions — so "the aggregator never writes" and "the stack writes through the console" are both true, about different services (D4).
- The aggregator does not edit Project fields. The orchestrator is the Project's single writer; a dashboard status change is a `status-request` action that the orchestration pass consumes and performs.
- The aggregator never writes GitHub at all from its comment surface: a dashboard comment is applied by a fleet job, which reports the GitHub result back.
- What the aggregator does own outright: its decisions store, its comments store, its roadmap annotations, its alert and notification ledgers, and its published state. Within those stores it is the writer, and the pages are the record.

One text inside the aggregator disagrees with the deployment it describes: its program page says the execution board runs the fork at `developing-today-forks/kaneo`. The compose file pins upstream v2.47.0. The deployment code wins, and this book states the board as upstream wherever the board's version matters (D1).

Sources. Survey files `~/workspace/pm-book/survey/apps.md` (§1, §2, §5, §7), `~/workspace/pm-book/survey/website-pages.md` (§A–§D, §F1, §F2, §F5, §F6), `~/workspace/pm-book/survey/roadmap.md` (§1–§4), `~/workspace/pm-book/survey/disagreements.md` (D1, D3, D4, D7, D8, D9, D11, D12); repo paths `pm-stack/aggregator/src/pages.ts`, `pm-stack/aggregator/src/server.ts`, `pm-stack/aggregator/src/model.ts`, `pm-stack/aggregator/src/sources/live.ts`, `pm-stack/aggregator/src/sources/projector.ts`, `pm-stack/aggregator/src/decisions.ts`, `pm-stack/aggregator/src/needs-drew.ts`, `pm-stack/compose.yml`.
