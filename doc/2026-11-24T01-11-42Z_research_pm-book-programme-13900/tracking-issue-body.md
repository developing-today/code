# Project Management book — fifth house book, coverage audit, sidelines, Space

Programme tracking issue for the PM-BOOK lane (Drew's directive, 2026-11-23 ~18:29 CST). Built per the bookwright skill (v5): survey before structure, structure before prose, mechanical coverage audit before "done".

## The deliverable

A fifth book, **Project Management** (the project is still called "PM stack" in conversation; the book covers the wider concept), in `pkgs/pm-stack/docs/` beside the subject, matching the form of the four existing books (mcpx exemplar; fleet, atlas, id), joining the repo's single Pages deploy under its own subpath.

## Completion test (Drew's words)

> "You'll know the book is done when that's true": **every worker in the estate and every Turso database is covered in the PM book or in one or more of the four pre-existing books.**

The coverage ledger below is the test's evidence. It starts from the #13844 Turso→D1 audit estate list (50 DBs) and a fresh census of Cloudflare workers/doors from the repo, verified against live surfaces where the repo's tooling allows.

## Coverage ledger

_To be filled by the census slice: every worker and every Turso DB, each mapped to its covering book + chapter. Gaps close in the PM book by default, or in the owning book (fleet/atlas/id/mcpx) via the sidelines / follow-on chapter PRs._

## PM book scope (census the real systems; this list is the floor, not the ceiling)

- Everything the PM stack website has now, page by page: the aggregator + console (routes/pages, health/quota/usage views, the question sets).
- The half-used PM apps it integrates: Kaneo (confirmed — incl. the Kaneo PM bot) and every other app the census finds (Grist and the rest), what each is, how it's wired, how much is actually used.
- The Android app push-notification system end to end (the website pushes itself, event-driven at detection: outage + waiting-on-Drew classes, the section-risk class from #8882, the answer-writes-back mechanic).
- Roadmap/prioritisation mechanics: major initiatives, projects, epics, aggregators — what each type is, roll-up, prioritisation, answer-back flows.
- The queue systems and multiple dispatch methods from the PM perspective — overlap with the fleet book is fine; describe from this angle and link to the fleet book's documents.
- Bridge: light coverage here; its durable home is the fleet book + a `fleet` subcommand (follow-up issue filed by this programme). The standalone bridge build runs separately (pkgs/message-bridge) — coordinate, don't rebuild.
- The PM stack's own model dependencies (embedding models and any others).
- **OpenCode + OpenChamber section**: comprehensive, structured to be liftable into its own book later — OUR changes, goals, intentions for the two fork repos + their prioritised issue maps. About the forks programme, NOT about LLM plans.
- **LLM plans chapters** (separate from the OC section): the plans we use, rough metrics, which models we use and why, benchmarks + evidence from real use. All quota/plan figures carry their read dates; stale figures are never presented as current. Own prioritised list; cross-links to the OC section where relevant.
- Prioritised order lists + dependency maps for all PM items (issues, designs, details).

## Sidelines

- **mcpx book**: add coverage of the mcpx door Worker (mcpx.security.cab — gate/proxy, `?ns=` enforcement) with maps/lists for its related items.
- **fleet book**: add coverage of the commander worker (Desktop Commander door chain) AND bridge (service + the planned `fleet` subcommand follow-up), same treatment.

## Space

After the book lands: a **Project Management** Space in the same vein as the existing book spaces (mcpx-status-breakdown, fleet, atlas, id-project-status) — same depth/shape; tabs, widgets, dropdowns, images only where they genuinely explain. The book's substance (systems, roadmap mechanics, push notifications, OC programme, LLM plans, prioritised lists, dependency maps) carries into the Space.

## Rules

Laptop worktrees only for repo work; PR + merge when verified (CI on main only); GitHub via gh / fleet gh / gh-post, never the connector; bookwright process incl. verification passes (zero-warning build gate, mechanical coverage audit, H1/title checks); generic-first framing for ecosystem material; this issue updated at every slice boundary.

## Slice log

- 2026-11-23 ~18:30 CST — programme opened by lane PM-BOOK (coordinator). Census + surveys dispatched.
