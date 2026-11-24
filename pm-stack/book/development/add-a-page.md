# Add a page or projection

Two extension points cover new read surfaces in the PM stack. A new dashboard page registers in the aggregator's page manifest and renders from the aggregator's one state. A new projection adds a read model in the projector, or a source reader in the aggregator, and the page consumes it. This chapter gives both procedures as the code is shaped today. Adding either surface changes a chapter of this book in the same pull request; see [Keeping this book current](keeping-this-book-current.md).

## Add a page

The manifest is `pm-stack/aggregator/src/pages.ts`. Its header states the mechanism: every dashboard page is a registered entry there; the navigation bar and the home-page dropdown render from this list, never hard-coded per view; and nothing else registers the page — nav, dropdown, and manifest are the same list. The header calls the recipe a ~30-minute recipe. The steps:

1. **Add one entry to `PAGES` in `pages.ts`.** An entry is a `PageDef`: `id`, `title`, `route`, `api` (the page's `/api/*.json` twin, when one exists), `blurb`, `group`, and `nav`. The `group` value names one of the four `PAGE_GROUPS` ids in the same file: `overview`, `work`, `ops`, `system`. The `nav` value is `primary` or `dropdown`, under the #1497 curation recorded in the file: primary pages earn a top-nav slot; dropdown pages are reachable from the Pages dropdown and their drill-down parents.
2. **Add the render function in `pm-stack/aggregator/src/render.ts`.**
3. **Add the route branch in `pm-stack/aggregator/src/server.ts`, plus the `/api` twin route** when the entry declares one. The twin serves the same assembly as the page; the health assembly behind `/health` and `/api/health` is the standing example of one assembly feeding both forms.
4. **Wire the data.** If the page needs data the aggregator does not already hold, add it to the `Source` interface in `pm-stack/aggregator/src/source.ts`, with a live reader and a fixture fallback. Live readers live in `pm-stack/aggregator/src/sources/` — one file per source (`live.ts`, `projector.ts`, `ready-for-agent.ts`, `repo-roadmap.ts`, `opencode.ts`).
5. **Register any detail route.** A page with a per-item detail form adds an entry to `SUBROUTE_PARENTS` in `pages.ts`: `name`, `prefix`, `parent`. That entry puts the detail renders inside the #1147 render registry that `/api/health` reports. A detail page missing from the map renders outside the registry — the decision-set detail page is the standing example of that gap (D8): `/decisions/&lt;setId&gt;` exists, its parent is `/decisions` by navigation, and `SUBROUTE_PARENTS` does not list it, so `manifestRouteFor("/decisions/…")` returns null for it.
6. **Test and document in the same change.** Add the page's tests under `pm-stack/aggregator/test/` in the suite's existing shape (see [Building and testing](building-and-testing.md)), and update [The aggregator website, page by page](../reference/website-pages.md) — that chapter is a 1:1 census of the manifest, so a new page without a new entry there leaves the book wrong.

Three manifest facts bound the recipe:

- **The `api` field is not automatic.** Three surveyed pages declare no twin. The `decisions` entry is one of them, and the server nevertheless implements a decisions API — `/api/decisions/&lt;setId&gt;/submit`, `/defer`, `/request-info`, `/events`, and `/api/decision-sets` (D7). A manifest entry records the page's declared twin; it is not the server's route table. Check `server.ts` before stating that a page has no API.
- **A redirect is a manifest outcome, not a page.** `/projects` is not a page: its manifest comment and its route branch redirect to the projects-pool section of `/program`, preserving the view parameters, while `/api/projects.json` still serves. A merged page follows that shape — redirect entry in the manifest, machine twin kept.
- **The All pages page is the manifest rendered.** `/pages` shows every registered view in one place. A correct entry appears there with no further work; an entry that does not appear there is not registered.

### The console is a separate surface

A console view is not a manifest entry. Console views are the `VIEWS` map in `pm-stack/console/public/app.js`, served by `pm-stack/console/src/server.ts` under `/console`, with server-side data in `pm-stack/console/src/` (`work.ts`, `ops.ts`, `plan.ts`). The surveyed console has 21 hash views. Adding one is a console change — view function in `app.js`, data and actions on the server side, tests in `pm-stack/console/test/` — and it updates [The console, view by view](../reference/console-views.md) in the same change. Do not register a console view in `pages.ts`, and do not register an aggregator page in `VIEWS`.

## Add a projection

A projection is a derived read model: data reshaped from a source of truth the projection does not own. Two homes exist, and the choice is the first step of the procedure.

### In the projector

The projector (`pm-stack/projector/`) is the stack's projection service. Its contract, in its own README: it links and projects; it never becomes a fifth source of truth and never writes back to GitHub, Kaneo, Pad, or Grist. To add a projection there:

1. **State the source event or poll that proves each field.** The projector's rule is exact: a projection writes only what the event proves; an absent field proves nothing. Webhook intake is `POST /github/webhook` (HMAC-checked, deduplicated by delivery GUID); reconcile polls in `src/reconcile.ts` and `src/cli.ts` self-heal the projection beyond webhook delivery.
2. **Add the storage.** Projector storage is SQL: schema in `pm-stack/projector/schema.sql` (the `pmp_` tables), read views in the same file (the standing views are `pmp_v_links_items` and `pmp_v_subject_activity`). Idempotent upserts key the writes.
3. **Expose the read.** The standing read shapes are `GET /items`, `GET /comments`, `GET /subjects/&lt;subject&gt;`, and the link registry `GET|POST|DELETE /links`. A new projection follows those shapes or adds a route in the same style in the projector's server.
4. **Classify rebuildability before writing.** The shared-schema map draws the line this step applies: the projector's item and comment projections are rebuildable from GitHub; `pmp_links` — the human- and agent-written subject-to-GitHub links — is the one non-rebuildable cross-service relationship in the projector schema, and it carries the registry backup legs for that reason. A new projection that stores a non-rebuildable relationship states that fact and its backup route in the same change. See [The shared schema](../concepts/shared-schema.md).
5. **Consume it from the aggregator through its source reader.** The aggregator reads the projector through `pm-stack/aggregator/src/sources/projector.ts` (links and latest comments per subject). A projector projection the dashboard shows is wired there, under the `Source` interface, not read from the projector's database by the aggregator directly.
6. **Test in the projector's suite** (`pm-stack/projector/test/`, run by the stack workflow) and update [The projector and projections](../concepts/projector.md) in the same change.

### In the aggregator

A projection that only the dashboard consumes, over data the aggregator already polls, lives in the aggregator as a source reader plus a derived view:

1. **Add the reader** as a file in `pm-stack/aggregator/src/sources/`, registered on the `Source` interface with a live reader and a fixture fallback (step 4 of the page recipe).
2. **Name the subject scheme.** Aggregator items join on registry subjects: `gh:…`, `kaneo:&lt;slug&gt;/&lt;id&gt;`, `grist:&lt;docId&gt;/Roadmap/&lt;recId&gt;`, `session:…`, `orch:…`. A projection over a new source states its subject form in the reader.
3. **State the merge rule.** Sources disagree, and the aggregator surfaces disagreement instead of hiding it: the `/conflicts` page lists every live source disagreement with both values and its age. A projection that merges two sources states which value wins and where the losing value shows. The roadmap merge is the standing example: on a title collision between a repository roadmap file and a Grist row, the repository file wins, the Grist duplicate is noted in provenance, and its links are unioned in (#1738).
4. **Record provenance.** Every aggregator surface carries a provenance string stating whether its data is live and from which source. A new projection produces one; the provenance-matrix check (`bun run provenance:check`) is part of the test route.

### What neither procedure does

Neither procedure creates a new source of truth inside the stack. The aggregator reads its sources; the projector links and projects; Project fields have one writer, the orchestrator, and a dashboard status change travels as a `status-request` action for the orchestration pass to apply. A change that needs a new durable store is a shared-schema change, not a page or projection change; it starts from [The shared schema](../concepts/shared-schema.md) and the store record in the reference part's data-stores chapter.

Sources. `pm-stack/aggregator/src/pages.ts` (file header, `PageDef`, `PAGE_GROUPS`, `SUBROUTE_PARENTS`, `manifestRouteFor`), read on the laptop clone at `44aaf73`; survey files `~/workspace/pm-book/survey/website-pages.md` (§A, §B), `~/workspace/pm-book/survey/apps.md` (§5, §7), `~/workspace/pm-book/survey/roadmap.md` (§2), `~/workspace/pm-book/survey/disagreements.md` (D7, D8); repo paths `pm-stack/projector/README.md`, `pm-stack/projector/schema.sql`, `pm-stack/aggregator/src/source.ts`, `pm-stack/aggregator/src/sources/projector.ts`.
