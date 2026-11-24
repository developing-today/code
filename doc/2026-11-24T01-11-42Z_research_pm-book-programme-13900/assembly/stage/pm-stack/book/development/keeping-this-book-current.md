# Keeping this book current

A book that drifts from the stack is worse than no book: readers script against what it says, and the PM stack's readers include the sessions that run the fleet. This chapter mirrors the fleet book's chapter of the same name in mechanism, and adapts its rule to this book's subject. The fleet book's contract is a documentation contract that predates its book; this book adopts the same shape from its first landing: the book is part of the change it describes.

## The rule

**A change to a pm-stack surface is not done until the book matches it in the same pull request.**

A pm-stack surface is any of: a page, a route, a Worker, a schema, a store. The list is the stack's own list of things a reader trusts this book to describe:

| Surface that changed | Chapter that changes with it |
|---|---|
| An aggregator page: a `PAGES` entry added, removed, or reworded in `pm-stack/aggregator/src/pages.ts` | [The aggregator website, page by page](../reference/website-pages.md), and [Add a page or projection](add-a-page.md) if the change alters the manifest mechanism |
| An aggregator route or API twin in `pm-stack/aggregator/src/server.ts`, including a route the manifest does not declare | [Aggregator HTTP routes and API](../reference/aggregator-routes.md). The manifest is not the route table (D7); a route change is a book change whether or not `pages.ts` moved |
| A console view in `pm-stack/console/public/app.js`, or an action in the console's audited action table | [The console, view by view](../reference/console-views.md) |
| A question set or an answer-back route: decision sets, needs-Drew answers, prompt threads, dashboard actions and comments, console actions, report and roadmap answer-backs | [The question sets, enumerated](../reference/question-sets.md) and [Question sets and the answer-back flow](../concepts/questions-and-answers.md) |
| A Worker: a wrangler configuration added or retired, a registry entry changed in `pm-stack/platform/workers.json`, a Worker's state changed (deployed, not deployed, in service) | The workers chapter in the reference part, and the coverage ledger contract below |
| A schema: `pm-stack/projector/schema.sql`, a shared-schema migration, a Worker's `schema.sql` | [The shared schema](../concepts/shared-schema.md) and the projector's chapters, as the schema's owner dictates |
| A store: a Turso or D1 database created, moved, frozen, or deleted; a local store file promoted, demoted to cache, or retired | The data-stores chapter in the reference part, and the coverage ledger contract below |
| A component added to or removed from `pm-stack/`, or a Nix package under `pkgs/pm-*` added or retired | [Repository layout](layout.md) and [Architecture](../concepts/architecture.md) |
| A build or test route: a `package.json` script, a suite adopted by `.github/workflows/pm-stack-tests.yml`, a package build shape | [Building and testing](building-and-testing.md) |
| A port, a unit, a deploy route, or this book's own deployment | The operations part, and [Repository layout](layout.md) for the port registry's location |
| A design decided, a programme reprioritised, an issue programme opened or closed | [PM stack research and implementation roadmap](program-roadmap.md) and [Project Management: prioritised issue map and dependency map](issue-map.md), with the issue map's snapshot line re-dated |

"Same pull request" means the same pull request. A follow-up issue promising to update the book later does not satisfy the rule. The survey for this book catalogued what the looser habit produces: a platform README counting eighteen Workers against a registry of twenty (D6), a decisions API the page manifest does not declare (D7), a detail page outside the render registry its own index feeds (D8), and a notification class count two behind the code (D14). Each of those was a surface that changed without its record changing in the same pass. If a disagreement of that kind surfaces while a change is in progress, the stale side is fixed inside the change's scope; a disagreement outside the scope gets an issue in the owning repository in the same pass and a disagreement number in this book's record.

## The coverage ledger is the sync contract

The book programme's coverage ledger — built for #13900 and kept in the programme's workspace — is the contract between this book and the estate's two censused sets: the Cloudflare Workers and doors, and the Turso databases. Its rule:

- **Every worker in the estate and every Turso database is covered in this book or in one of the four pre-existing books** — the fleet book, the Atlas book, the ID book, and the mcpx book. That is the programme's completion test, and it does not lapse when the programme closes; it becomes this chapter's standing test.
- A new Worker, door, or database enters the ledger in the change that creates it, with its covering book and chapter named, or with its gap assigned to a book in that same change. An unassigned gap is a failing ledger.
- A Worker or database that moves books — a sideline chapter landing in an owning book, a store migrating under a programme such as #13844 — updates the ledger's covering entry in the same change. Coverage that points at a chapter that no longer carries the item is drift of the same kind as a stale count.
- The ledger's counts are census figures, and census figures follow the rule in the next section: they are re-derived from the source — the wrangler configurations and workflows for workers, a live database list for the Turso estate — never restated from the previous ledger.

At this book's first census the ledger stood at 22 workers (3 covered, 19 assigned as gaps) and 50 Turso databases, confirmed by a live list (34 covered, 16 assigned as gaps). Those figures are the census of 2026-11-23. A later edition of this chapter quotes a later census or none; it does not repeat these as current.

## Census figures are re-derived, never restated

Every count in this book came from a count: 50 primary pages and routes from the manifest and the console's view map, 36 components from the tree, 20 Workers from the registry file, 8 notification classes from the class defaults in code, 50 databases from a live list. The rule for keeping them true:

1. **Re-derive from the named source.** A page count is recounted from `pages.ts`, `server.ts`, and the console view map. A Worker count is recounted from `workers.json` and the wrangler files its test checks. A database count is recounted from a live `turso db list` or from the governing programme's audit, read that day.
2. **Date the figure in the sentence that carries it**, or in the chapter's snapshot line where the chapter has one, as [Project Management: prioritised issue map and dependency map](issue-map.md) does. An undated count in this book is a defect.
3. **Never carry a figure forward because it was true in the last edition.** The estate's own record shows why: the Turso audit's headline of 50 and its internal enumeration disagreed by two until a live list settled which figure was wrong — the enumeration, not the headline. A restated figure imports the error and hides the settlement.
4. **A figure that cannot be re-derived at edit time is marked UNVERIFIED or removed.** It is not left standing on the strength of the previous text.

## The build gates

Two mechanical gates keep the book honest, in the fleet book's shape:

1. **`create-missing = false`.** The book's `book.toml` sets `create-missing = false`: every chapter in `SUMMARY.md` exists on disk; no stubs are invented. Adding a `SUMMARY.md` entry without the file fails the build instead of publishing an empty page.
2. **The packaged build.** The book builds as a flake output with its own build workflow, in the shape of the fleet book's `.#fleet-docs` package and `fleet-docs.yml` workflow, and the combined Pages deploy rebuilds it under its subpath. A chapter that breaks mdBook — a bad link target, a missing file — fails the build by name rather than shipping broken. The zero-warning build is the release gate.

Form is part of the contract, because the gates check it: every chapter's sole H1 is its `SUMMARY.md` entry text; internal links are relative `.md` links whose text is the target chapter's title; angle-bracket placeholders are escaped. A change that breaks the form breaks the build or the census chapters' 1:1 shape, and both are release defects, not style notes.

## Where the rule is recorded

The rule lives in three places, which must agree:

1. This chapter — the book stating its own maintenance rule, so a reader who arrived here first finds the whole contract.
2. The programme record on #13900 — the completion test and the coverage ledger that this chapter turns into a standing contract.
3. The fleet book's chapter of the same name — the mechanism this chapter mirrors, and the precedent for the estate's other books.

If the three disagree, this chapter wins for this book's scope, the fleet chapter wins for the fleet book's scope, and the disagreement itself is fixed in the same pass it is noticed — the same rule, applied to the rule.

Sources. The fleet book's keeping-current chapter (`pkgs/fleet/book/development/keeping-this-book-current.md`, read through `fleet gh api` on 2026-11-23) for the mechanism mirrored here; `~/workspace/pm-book/coverage-ledger.md` (census date 2026-11-23, worker and Turso counts, the settled 50-database count); `~/workspace/pm-book/tracking-issue-body.md` (the completion test in Drew's words); `~/workspace/pm-book/survey/disagreements.md` (D6, D7, D8, D14); `~/workspace/pm-book/skeleton.md` (placement at `pm-stack/book/` and `pm-stack/book.toml`, the deploy shape).
