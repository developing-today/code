# Project Management

The Project Management stack (the PM stack) is the project's own coordination
surface: the services that turn fleet state — queue, quota, issues, decisions
— into pages a human can act on and notifications a phone can answer. Its
parts, per the PM census of 2026-11-23:

- **The aggregator** reads fleet, queue and GitHub state and serves it as
  JSON and pages: live needs-you and quota panels first (PRs #892, #900,
  #901, #920 in W43), growing to 29 routes and 21 console views at census.
- **The console** is the operator's web surface over the aggregator.
- **pm-dispatch** launches and enqueues work into the fleet queue; it was
  built and deployed 2026-10-30.
- **The notify path** — aggregator to Web Push to phone to answer — is the
  stack's end-to-end test of whether the project can reach Drew and take his
  answer back into the queue.
- **The estate it watches:** the PM census live-verified 50 Turso databases
  and 22 Cloudflare Workers on 2026-11-23. That census is the estate of
  record for the storage-placement item (merged-list item 11) and the backup
  item (item 7).

Costs live in the PM book, by Drew's ruling of 2026-11-23, and are covered
lightly there: monthly usage across the core quotas (Workers, Turso, each
LLM provider), pay-as-you-go spend covered separately, and plan arrangements
covered as existing without detail. This book does not reproduce cost
figures.

## Current state (2026-11-23)

- The aggregator and console are live. The notify relay is built but not
  deployed: its answer routes are unrouted, recorded as disagreement D5 in
  the PM survey.
- Completion is merged-list item 26, in flight. Its pacing verdict is
  recorded on #5007: backlog state derived from database plus GitHub, with
  markdown as a view, is correctly blocked on phase ordering — do not start
  yet. Decision sets v2 (#9335) landed its acceptance and was live-verified
  2026-11-13; at census it awaited close only.
- The PM stack is also the forks programme's home in this book's map: the
  OpenCode/OpenChamber convergence item (item 27) is carried as a liftable
  section of the PM book. See [The OpenCode and OpenChamber
  forks](forks.md).

## Where its detail lives

- Book: the PM book, in preparation under `pm-stack/book/`. Its skeleton was
  reconciled on 2026-11-23 (about 48 chapters). Its published subpath in the
  combined Pages deploy is **not yet final** — this book says so plainly
  rather than printing a URL that does not exist yet.
- Programme tracker: [#13900](https://github.com/developing-today/code/issues/13900)
  (PM-BOOK programme); platform tracker
  [#13504](https://github.com/developing-today/code/issues/13504).
- This programme's slice: the PM book carries items 26, 27, 29 and 30 in
  full when its high-level roadmap chapter lands, per the assignment table in
  Census B.
- Merged-list items: 26, 27, 29 and 30 are PM-primary; PM also carries
  cross-links to items 1, 2, 7, 9, 11, 15, 17, 22, 23 and 28.
