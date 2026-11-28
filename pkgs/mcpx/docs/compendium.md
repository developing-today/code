# The compendium: structure and mapping

```
created:      2026-10-29T19:00:00-05:00
last-updated: 2026-10-29T19:00:00-05:00
increment:    1
status:       standard
tags:         area:docs, area:compendium
description:  what the compendium is, the chapters in order, which file is the
              source of each, and the rule that keeps a file in exactly one.
```

The compendium is level 3 of the book: the encyclopedia. Level 1 is the
overview, level 2 is the guides, and level 3 is where "basically everything
that is built can be described here in as much detail as you want". It is
deliberately not limited to mcpx: note 88 asks it to catalogue MCP servers,
resources, indexes, URLs and projects as well (chapter 7).

This page is the structure. The per-file mapping is
[`compendium.manifest.json`](compendium.manifest.json), next to it, and a test
keeps the two in step (see [Keeping this true](#keeping-this-true)).

## Approach: the docs are the book source

Approach C, as recommended on #1867 and not reopened here:

- **Source root** `pkgs/mcpx/docs`. Files render where they already are.
- **Navigation** is generated from `compendium.manifest.json`, not written by
  hand and not kept in a second tree.
- **`README.md` stays a symlink to `docs/story.md`** (git mode 120000), and the
  `<!-- table-of-contents-marker -->` stays where it is. The runbook's check
  `test -L README.md && readlink README.md` keeps passing.
- **One home per document.** Other levels link to a chapter or excerpt it; they
  do not copy it. That rule is what makes a level-3 page safe to be as long as
  it needs to be.

Approaches A (move the files under a book directory) and B (one stub page per
document) were rejected on #1867 for the reasons recorded there: A breaks every
relative link, `internal/settings/docs_test.go`, and the symlink; B needs a stub
per document plus a check that the stubs have not drifted, and breaks links a
different way.

## Chapters, in order

A chapter's source is a file or a **directory**. When it is a directory, its
`README.md` is the chapter index and the rest of the directory is the chapter's
pages; a generated navigation therefore has to accept a directory, which is the
concrete requirement the three in-flight directories from the 2026-09-30
inventory turned into.

| #   | Chapter                 | Source                                                                                                                                                | Holds                                                                                                                                                                                              |
| --- | ----------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | Features                | `docs/story.md`, below `<!-- table-of-contents-marker -->`                                                                                            | the feature list: one page per key-block entry or `area:` tag, split at build time at section IDs (#151). The walk above the marker is the level-1 overview (#157), not a chapter                  |
| 2   | Designs                 | `docs/elicitation.md`, `exec.md`, `consumer.md`, `opencode-plugin.md`, `plugin-discovery.md`, `environment.md`, `runtimes.md`, `schema-validation.md` | the "why" documents: a design kept as the record of a decision, at full depth                                                                                                                      |
| 3   | Decision records        | `docs/decisions/` (directory) + `declared-but-unread.md`, `landing.md`, `git-discovery.md`                                                            | one record per collision, following the template in `decisions/README.md`. The two files at `docs/` root record a decision each and say so in their own front matter; the chapter index links them |
| 4   | Protocol                | `docs/protocol.md` + `docs/spec/` (directory)                                                                                                         | `protocol.md` is the design and the entry point; `spec/README.md` is the chapter index for the records under it: what each revision asks of each side, what mcpx sends, and how it is measured     |
| 5   | Build and packaging     | `docs/container.md`, `dependencies.md`                                                                                                                | what the binary and the image are made of, and what they deliberately are not                                                                                                                      |
| 6   | Proposed                | `docs/proposals.md`                                                                                                                                   | open designs, badged `proposed`. A reader deciding whether mcpx will do something next starts here                                                                                                 |
| 7   | Catalogue               | `docs/catalogue/` (directory)                                                                                                                         | things outside mcpx: MCP servers, resources, indexes and registries, URLs, projects. Structure below; the entries are seeded by the follow-up task                                                 |
| 8   | Comparisons and history | `docs/compare/` (directory) + `ASSESSMENT.md`, `OPENCODE-V2.md`                                                                                       | the living comparison and the historical documents, kept as history with their text intact (#153)                                                                                                  |
| 9   | Research and findings   | `docs/audit/` (directory) + `in-name-only.md`, `work-strategies.md`, `research-and-findings.md`                                                       | audits and lessons: what was found, not what was designed. The research chapter (#1871) has joined; its Nix section is still a later slice                                                         |
| 10  | Maintaining these docs  | `docs/release-runbook.md`, `compendium.md`                                                                                                            | how `story.md` stays true, the key-block rules, and this structure                                                                                                                                 |
| 11  | Document history        | `docs/archive.md`                                                                                                                                     | retired entries and refactored-out text, append-only and unordered (#150)                                                                                                                          |

## The migration mapping

Every one of the 96 markdown files under `pkgs/mcpx` at `origin/main` is in the
manifest, with a chapter or an explicit reason not to have one. The 2026-09-30
inventory on #1867 covered 20-odd of them; the seven it never saw now land as:

| file                            | lands in                                                                                       |
| ------------------------------- | ---------------------------------------------------------------------------------------------- |
| `docs/archive.md`               | 11 Document history                                                                            |
| `docs/environment.md`           | 2 Designs (what mcpx tells a program it starts, #172)                                          |
| `docs/git-discovery.md`         | 3 Decision records (it calls itself a decision record, and says why it is not in `decisions/`) |
| `docs/in-name-only.md`          | 9 Research and findings (the curated list of what only exists on paper)                        |
| `docs/runtimes.md`              | 2 Designs                                                                                      |
| `docs/schema-validation.md`     | 2 Designs                                                                                      |
| `docs/work-strategies.md`       | 9 Research and findings (lessons learned)                                                      |
| `docs/research-and-findings.md` | 9 Research and findings (the #1871 chapter: findings by topic, provenance-tagged)              |
| `docs/audit/` (3 files)         | 9 Research and findings                                                                        |
| `docs/compare/` (18 files)      | 8 Comparisons and history                                                                      |
| `docs/decisions/` (5 files)     | 3 Decision records                                                                             |
| `docs/spec/` (14 files)         | 4 Protocol                                                                                     |

### Explicitly not compendium, and why

These are in the manifest with `role: not-compendium`, so the mapping is
complete rather than merely plausible:

| file(s)                                | why not                                                                                                                                       |
| -------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------- |
| `docs/ideas.md`                        | raw unvetted intake ("some of it is probably wrong"). The proposed chapter publishes the index-of-issues version instead; see open question 3 |
| `plugin/opencode/skills/**` (12 files) | skill text shipped inside the plugin and rendered as level-2 pages (#148). Restating it in the compendium would make two sources for one text |
| `daemon/README.md`                     | an operational how-to for the systemd user unit: level 2                                                                                      |
| `internal/mcpspec/schema/README.md`    | a provenance note for vendored schemas; it documents a tree, not a design                                                                     |
| `TODO.md`                              | a scratch list                                                                                                                                |

Four more are `role: included`: `AGENTS.md` (the agent instruction file, linked
and included but chapterless, per #1867), `docs/configuration.md` and
`docs/parity.md` (reference, #144, and level 2, #146 — the compendium links to
them; `internal/settings/docs_test.go` reads `configuration.md`), and
`plugin/opencode/README.md` (install, level 2, agent twin #120).

`README.md` is in the manifest as a `symlink` entry rather than a source, so the
generator has an explicit instruction about it instead of a rule it has to
remember.

## The catalogue chapter

Structure only in this change; the entries are a follow-up so that this page can
be reviewed as a structure rather than as a directory listing.

- **Source** `docs/catalogue/`, with a chapter `README.md` and one page per
  section: `servers.md`, `registries.md`, `resources.md`, `urls.md`,
  `projects.md`.
- **Generated or written: C.** A hand-written list says _which_ servers,
  registries, resources, URLs and projects are in the catalogue and _why_; the
  facts per server (`mcpx --json ls` tool counts, `mcpx types` tool lists,
  `mcpx --json registry search` publisher and version) are captured into a
  committed snapshot by a refresh script. Nothing at docs build time starts a
  server or calls the network, because CI must not have to start seven MCP
  servers to render a page.
- **First entries** are the seven servers configured on this machine —
  `codebase-memory`, `codedb`, `fff`, `fff-nix`, `fff-worktree`,
  `chrome-devtools` (the `session`-mode case) and `context7` — plus the official
  registry `registry.modelcontextprotocol.io`, the recorded rejection (HAPI), and
  the projects list from #1867. Scope beyond that machine is open question 5.

Approach A (hand-written entries only) and B (mirror `mcpx registry search`)
stay rejected for the reasons on #1867; C is recorded here so the follow-up does
not have to re-argue it.

## Open questions on #1867, answered

1. **Split `story.md` per entry, or one long page?** Split, at build time, at
   section IDs (#151). One page with good anchors is readable, but the feature
   list is also what level-2 pages excerpt (#146-#148) and what the catalogue
   and the decision records link to by name; a stable page per entry gives those
   links a target that survives an edit elsewhere in the list. The split pages
   are generated and not committed, so `story.md` and the symlinked `README.md`
   stay one file on GitHub.
2. **Move `ASSESSMENT.md` and `OPENCODE-V2.md` into `docs/`, or include them
   from the root?** Move them into `docs/`. Both are chapter sources for chapter
   8, both are linked by relative path from `story.md` and `proposals.md`, and a
   chapter whose sources live in two trees needs every link rewritten twice.
   `AGENTS.md` stays at the root: it is the agent instruction file and the name
   is the convention.
3. **`ideas.md`: unpublished, raw, or an index of issues?** Publish the
   index-of-issues version in the proposed chapter; keep `ideas.md` itself as
   raw intake and do not publish it. It says of itself that it anticipates
   becoming "an index of links" once issues exist, and they do.
4. **Which "resources" and which "indexes" does note 88 mean?** All of them, and
   the chapter separates them instead of choosing: `resources.md` covers MCP
   resources (the protocol feature, and what the configured servers actually
   expose) and reading material as its own section, while `registries.md` covers
   registries and the code indexes the configured servers maintain (`fff`,
   `codedb`, `codebase-memory`). A reader looking for either arrives at the same
   page.
5. **Catalogue scope: this machine's servers, or also the ones mcpx was tested
   against?** This machine's seven first, because each has a recorded reason to
   be configured. Servers mcpx was merely tested against (the Swagger petstore
   in `OPENCODE-V2.md`, `io.github.microsoft/playwright-mcp` in `story.md`) go in
   a clearly marked second section, since the reason they are listed is
   different: evidence, not configuration.
6. **Do stale facts in living documents get fixed before the compendium
   publishes them?** Yes, as a separate small fix, not inside this structure
   change: `dependencies.md` ("**One.**"), the `story.md` "Nix packaging" entry
   (`vendorHash = null`, "one third-party Go dependency"), and the missing
   `nix/darwin-module.nix` that `ASSESSMENT.md` points at. Publishing them is
   what turns a known staleness into a citable error. Tracked as its own
   follow-up, and deferred to #1907 if it is not picked up first.

## Constraint this structure keeps

The one test that pins a doc path is `internal/settings/docs_test.go`, which
reads `../../docs/configuration.md`. Nothing here moves a file, so it is
untouched. No relative link changes either: this change adds two files under
`docs/` and edits nothing else, which is also why the `story.md` links written
relative to `docs/` (`../AGENTS.md`, `../ASSESSMENT.md`, `../OPENCODE-V2.md`,
`./release-runbook.md`, `../scripts/stress.sh`) keep resolving. When open
question 2 is implemented and the two root documents move into `docs/`, those
links change in the same pull request.

## Keeping this true

`internal/settings/compendium_test.go` walks every markdown file under
`pkgs/mcpx` and fails when:

- a file is not in the manifest at all (a new document must be placed, and the
  failure names it);
- a manifest path does not exist (a moved or deleted document leaves a chapter
  pointing at nothing);
- an entry's `chapter` is not one of the chapters listed here;
- a `chapter-source` entry has no chapter, or a `not-compendium` entry has one
  without saying why.

That is the whole mechanism: adding a document to `pkgs/mcpx` is not finished
until it has a chapter, or a reason for not having one.

**Related:** #1867 (this structure), #1907 (level-3 tracker), #117 (generator
source root), #150/#151 (section IDs and split), #153 (chapters 8),
#154/#1871 (chapter 9), #157 (level-1 overview), #144/#146-#148 (reference and
level 2), #120 (agent twin), #24 (elicitation as a design).
