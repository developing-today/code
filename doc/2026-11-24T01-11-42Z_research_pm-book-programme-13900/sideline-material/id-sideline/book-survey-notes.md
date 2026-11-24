# Survey notes — the id book itself (`pkgs/id/book`)

## Layout and build facts (verified in the tree)

- mdBook config: `pkgs/id/book.toml` — title `id`, authors
  ["developing-today"], language en, `src = "book"`, `build-dir =
  "rendered"`, `[build] create-missing = false` ("Every chapter in
  SUMMARY.md must exist on disk; never invent stubs"), `[output.html]
  git-repository-url = "https://github.com/developing-today/code"`.
  The book root is `pkgs/id` itself so sources stay next to the system.
- Build derivation: `pkgs/id/docs.nix` — runCommand rendering book.toml +
  book/ with the flake-pinned mdbook, no network; rewrites links escaping
  the book tree to repo URLs in the rendered copy only (depth-dependent sed:
  maxdepth 1 vs mindepth 2); `siteUrl` argument baked as
  MDBOOK_OUTPUT__HTML__SITE_URL (default "/"; Pages deploy overrides with
  "/code/id/"). Exposed as flake output `.#id-docs`.
- Gate workflow: `.github/workflows/id-docs.yml` — on push to main +
  workflow_dispatch; `nix build .#id-docs --print-build-logs`; stages
  `cp -rL result site`; asserts `site/index.html`. Build only, never
  deploys (one Pages site per repo; combined deploy is mcpx-docs.yml,
  staging id under `id/` — per operations/deploying-this-book.md; live at
  https://developing-today.github.io/code/id/).
- Sections on disk: getting-started, concepts, reference, library,
  tutorials, design, development, operations, research + index.md.
  Reference chapters before this PR (16): cli, cli-files, cli-tags,
  cli-serve, cli-worlds, cli-worlds-admin, cli-gallery-walk,
  cli-groups-identity, repl, native-shell, web-ui, websockets,
  hosted-worker, mcp, configuration, data-stores.

## Second navigation / manifest gate check — NONE EXISTS

Question from the brief: does the id book have a second gate like mcpx's
compendium manifest (`TestEveryDocumentHasAChapter`)?
Evidence gathered:
- `grep -rn "compendium|TestEveryDocument|SUMMARY" pkgs/id/book/
  --include="*.md" --include="*.toml" --include="*.rs" --include="*.go"` →
  only prose mentions inside development chapters (layout.md's description
  of book/, keeping-this-book-current.md's statement of the two gates).
- `grep -rn "pkgs/id/book|id/book" --include="*.rs" --include="*.go"
  --include="*.nix" --include="*.yml" pkgs/id/ .github/workflows/id-docs.yml
  lib/` → only docs.nix, the workflow, and one unrelated Rust doc comment
  (an oracle path in examples/rust-colony pointing at a research evidence
  file). No Go/Rust test enumerates book files.
- keeping-this-book-current.md itself states "The two build gates" and that
  they are "the whole of its teeth today": (1) `create-missing = false`,
  (2) the `.#id-docs` Nix build on main. It also honestly records that
  neither gate checks chapter contents against code.
Conclusion: SUMMARY.md registration is the only registration. The PR says so
explicitly.

## The book's own form rules (from keeping-this-book-current.md)

- Every file starts with its exact chapter title (the SUMMARY.md text) as
  its single `# ` heading.
- Internal links are relative `.md` links whose text is the target chapter's
  title.
- The sync contract: "Any change to a command, a flag, a default, a config
  key, an MCP tool, a route, a schema or an operational procedure updates
  the matching chapter of this book in the same change." Item 2 names the
  hosted Worker route → hosted-worker.md mapping; this PR extends that item
  to name the mail Worker chapter (the book's own mechanism, extended in
  place — not a parallel mechanism).
- Designed-vs-shipped discipline: chapters never present designed behaviour
  as shipped; the hosted edition's local-only status is stated everywhere
  it is touched. Mirrored for the new chapters: mail = production-shaped but
  described from config/source; PoCs = explicitly not production.
- Source over help: where docs and code disagree, document the source and
  call out the discrepancy (book precedents: CLI-5, CLI-6, WEB-9).
- Voice: plain, exact, status-first; tables for surfaces; failure shapes
  enumerated; toy examples marked as toy data; secrets named, never valued;
  "Unverified" markers where a fact could not be checked against source
  (precedent in operations/services.md for operator-chosen ports).

## Existing coverage found by grep (pre-PR)

`grep -rn "worker-poc|id-worker-poc|d1-journal|id-mail|mail.security|
id-mail.security" pkgs/id/book/`:
- worker-poc / d1-journal-poc: passing summary treatment in
  concepts/hosted.md, reference/hosted-worker.md, concepts/architecture.md,
  getting-started/shape.md, design/worlds-runtime.md,
  design/hosted-editions.md, development/layout.md,
  development/release-and-rollout.md, operations/services.md,
  reference/data-stores.md — but NEVER under the exact Worker names
  `id-worker-poc` / `id-d1-journal-poc`, and with no SUMMARY chapter.
- mail worker: only development/release-and-rollout.md ("Two neighbouring
  Worker workflows are not the hosted edition…") and research/releases.md
  (workflow inventory line). No routes, no config, no chapter.
Census verdict stands: GAP for all three as named-Worker coverage.

## Issue-map / prioritised-list check — NONE EXISTS for workers

- design/roadmap-and-issue-map.md: its tracker table maps design-programme
  trackers (#5678, #5685, #9078, #3399→#3404, …) to design chapters; its
  design-record index maps the 408 doc/ records to design chapters. Neither
  is a worker list; adding Worker rows would break its stated construction
  rules (one row per design record; trackers named inside design records).
- research/issue-map.md, research/current-issue-map.md,
  research/issue-map-delta.md: dated census snapshots of open issues with
  evidence files under research/evidence/ — editing them would falsify the
  census record.
- Conclusion (reported in the PR and on #13900 instead of inventing a
  chapter): no worker prioritised list / issue-map chapter exists in the id
  book.

## Placement and cross-link decisions

- New chapters placed in `reference/` directly after "The hosted Worker
  API" in SUMMARY.md — the reference section is where the book puts Worker
  surfaces (hosted-worker.md is the precedent), and adjacency makes the
  mail/hosted distinction discoverable.
- Titles (SUMMARY text == H1):
  - "The mail Worker API" (mirrors "The hosted Worker API").
  - "id-worker-poc: the World Durable Object proof of concept".
  - "id-d1-journal-poc: the D1 journal proof of concept".
  (PoC titles carry the exact Worker names so the completion test — every
  worker covered by name — passes on SUMMARY text alone.)
- Files changed in the PR (8): SUMMARY.md; reference/mail-worker.md (new);
  reference/worker-poc.md (new); reference/d1-journal-poc.md (new);
  reference/hosted-worker.md (one closing paragraph linking all three);
  concepts/hosted.md (one sentence linking the two PoC chapters);
  operations/services.md (new "The mail Worker" section before "Exit
  behaviour for operators"); development/keeping-this-book-current.md
  (contract item 2 extended).
- NOT changed, deliberately: research census chapters (dated snapshots);
  roadmap tracker table / design-record index (construction rules above);
  development/layout.md (its worker-poc row is already accurate);
  reference/data-stores.md (already carries both PoC journal rows);
  configuration.md (mail-worker bindings are Worker bindings, not id
  process env; the hosted-worker precedent keeps Worker bindings in the
  Worker chapter — configuration.md's hosted-bindings section points at
  hosted-worker.md the same way).

## Chapter drafting notes

- Drafted in the sandbox first (per brief: drafting first, build last;
  laptop under heavy load), transferred as a base64 tar in 4 argument
  chunks through the laptop channel, sha256 verified both sides:
  b7b46952c5e81de9912c75827b6cc64a0c2853b95ee300d0d44629f04d0623dc.
  (Piping file content through the channel's stdin failed in the id book
  run's recorded experience; argument-passed chunks are the reliable route
  per bookwright v4.)
- Angle-bracket placeholders: all remaining `<…>` occurrences are inside
  code fences or backtick code spans (book precedent: hosted-worker.md uses
  `<64-hex>` the same way). The one literal error-template
  `to must have 1 to <maxRecipients> addresses` is a backticked code span
  quoting the transport's template string.
- Hedge pass: drafts grepped for `-shaped`, `reportedly`, `appears to`;
  three "shaped" phrasings reworded so no hedge-shaped word remains.
- Depth split per brief: mail worker = full chapter (what/routes/auth/
  config/failure shapes/operations/relationships/worked example); PoCs =
  shorter honest chapters (what each proved, what it was for, status,
  where its ideas landed, explicit not-a-product framing).
