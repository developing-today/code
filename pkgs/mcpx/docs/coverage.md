# Coverage: the book against the binary

This chapter is the book's mechanical coverage proof: the whole
surface of mcpx, dumped from the binary itself, set against the
whole text of the book, with every difference named. It was
first run on 2026-11-23 against the binary built from commit
`0d29cdad5cf`, and the gaps it found were closed in the same
change that added this chapter. The method is the point: anyone
can re-run it, and the gates listed at the end hold most of it
true between runs.

## Method

The surface was dumped from the subject, never from memory or
from older docs:

- `mcpx --help` and the `--help` of every command, recursively;
- `mcpx config --schema --plumbing` — the full settings
  registry, public and internal, with types, defaults, flag
  names and environment names;
- `mcpx openapi` — the /v1 document the binary prints with no
  daemon running (94 paths, 109 operations);
- `go doc` for the public library package, and the source tree
  for the settings registry, the internal packages and the
  design records.

Each dimension below is a set difference between that dump and
the book's text, computed mechanically. Literal matching has
known false-positive classes — list-form tables that name a
command once and its verbs in the next cell, a route the book
documents as a template, a usage line whose bracket holds a
flag rather than a verb — and every audit failure was
hand-classified before it was called a gap. The classifications
are stated with the numbers.

## Commands and verbs

The binary declares **75 top-level commands** in six groups
(running code, finding tools, the daemon, configuration,
inspection, daemon operations). All 75 are in the generated
command table in [Parity](./parity.md), and the equality of
the declaration list with the binary's dispatch table is a
test: `TestEveryDispatchedCommandIsDeclared`. The prose group
tables in [The CLI](./cli.md) now also name all 75 — the audit
found five missing there (`cache` and `stdio-proxy` from the
daemon group, `plugin` from configuration, `hook` and `want`
from inspection) and they are added.

**64 verb forms** sit under those commands (`adapter add`,
`settings set`, `task cancel`, …). Every family's verbs are
now described in prose in [The CLI](./cli.md)'s verb-set
table. The literal audit matches 54 of 64 two-word forms in
the text; of the ten it cannot see, six are in that table in
split-cell form (the command in one cell, its verbs in the
next), two are `stats` modes named in the story chapter's
inventory, and two are extraction artifacts — a flag in a
usage line, and the generic `mcpx help <command>` topic form.

Three commands had no prose anywhere before this run:
`skills` (now in the verb-set table and the running group),
`supervise` and `upgrade` (parity rows and group-table
entries, with the mechanism in the [Glossary](./glossary.md)
under *supervise* and *takeover*).

## Flags

The help dump carries **332 distinct `--flags`**. 215 of them
are setting flags — every setting's flag spelling is in the
book through its setting's row in
[Configuration](./configuration.md), with none missing. The
other 117 are command-local. 75 of those appear in the book's
prose; the remaining **42 are documented only in the generated
surfaces** — `mcpx help <command>`, the man page and the shell
completions, all printed from the same declarations in
`internal/cli/manual.go`, so they cannot drift from the
binary. They are named here so the set is exact rather than
approximate:

`--args`, `--argument`, `--arguments`, `--base-url`,
`--changed`, `--context`, `--deprecated`, `--dir`, `--dump`,
`--exclude`, `--execution-score`, `--feedback`, `--install`,
`--kind`, `--kinds`, `--max-score`, `--min-score`,
`--namespace`, `--notes`, `--overwrite`, `--parallel`,
`--ref`, `--replace`, `--retrieval-score`, `--safe-prefix`,
`--score`, `--scores`, `--semantic`, `--sharing`, `--socket`,
`--sort-by`, `--stop-on-error`, `--takeover`, `--target`,
`--tools`, `--trace`, `--ttl`, `--until`, `--uri`,
`--validate`, `--wait-ms`, `--yes`.

This is the book's one deliberate non-duplication: per-command
flags live in the generated reference, and this chapter is
where the book says so, with the list.

## Settings and environment

The registry declares **241 settings**: 193 public and 48
internal (the plumbing half, listed by
`mcpx config --schema --plumbing`). All 241 appear in
[Configuration](./configuration.md)'s generated inventory
with type, default, scope, whether they take effect hot, flag
name and environment name — held byte-equal with the registry
by `TestTheDocumentationListsEverySetting` and the shared
golden test below. The book states no other count; an earlier
programme census said 252, which counted `Path:` literals
across the whole settings tree, test files included, rather
than the registry the binary itself prints.

Every setting carries an effect probe or a written exemption.
The exemption count is pinned at **5** (`EffectExemptionPin`)
and can only shrink: `daemon.inlineStartPoll`,
`daemon.tsnet.hostname`, `daemon.tsnetUpTimeout`,
`http.streamBufferInit`, `embeddings.wasmPath` — each named,
with its reason, in the registry and in the configuration
chapter's table.

The schema names **246 `MCPX_*` environment variables** (some
settings carry an alias); all 246 are in the book through the
same inventory rows. One environment variable is deliberately
*not* a setting: `TS_AUTHKEY`, the Tailscale auth key for the
tsnet transport, kept out of the settings system so it can
never land in a config file or a settings dump. It is
documented with the transport, in [Reaching a daemon on
another machine](./remote.md).

## The /v1 API

The printed OpenAPI document has **94 paths and 109
operations**. 82 paths appear literally in the book (in
[The /v1 HTTP API](./http-api.md) and the parity table). The
other 12 are the instances of the one templated family the
API chapter names, `POST /v1/tools/{tool}` — one path per
tool mcpx itself serves — and the chapter now enumerates the
instances. The daemon's own served document additionally
carries one path per tool in the *live* catalogue, generated
per request; those exist by construction, not by enumeration,
and the API chapter says so.

## The library surface

`pkg/client` exports **14 symbols**; all 14 are now documented
in [Public Go client for /v1 (`pkg/client`)](./public-client.md),
with the call shapes and error types, and an example from the
package's own tests. Nothing else in the repository calls the
package yet — the chapter says that too.

Of the **62 internal packages**, 59 are named somewhere in
the book. The three that are not — `homeutil`,
`pluginsession`, `statsorder` — are internal helpers with no
public surface; they are recorded here rather than padded
into chapters.

## Design records and the book's own inventory

mcpx keeps its design records in the book tree, not in the
repository-root `doc/` archive. The inventory: 7 decision
records (`decisions/0001`–`0007`) under their index, 13 spec
chapters, 12 comparison chapters and 23 register entries, 3
audit chapters, the catalogue, and the testing set. Every
markdown file under `pkgs/mcpx` has a compendium manifest
entry with a chapter or a stated reason — a test
(`TestEveryDocumentHasAChapter`) walks the tree and fails on
an unplaced file.

Every file under `docs/` is in `SUMMARY.md` except the eight
`changes/` fragments, which are docs-history metadata —
consumed by the build's preprocessor, not chapters. Before
this run, six placed chapters were missing from the summary
and so never rendered: the testing set (4 files), the public
client chapter, and decision record 0007. All are listed now.

Every summary title equals its chapter's sole `# ` heading
(the one exception is `index.md`, the styled home page, whose
title is its HTML `<h1>` by design), and the audit checked the relative links
in the source tree: links that escape `docs/` are rewritten
to repository URLs by the build, and the two genuinely dead
links the audit found — a release-runbook link to a
`tty.md` that does not exist, and a story-chapter link that
only resolved by URL normalisation — are fixed.

## The build

The book builds as `.#mcpx-docs` with the docs-twins gate
inside it. Before this run the build carried 19 warnings —
bare `<placeholder>` tokens in prose being read as HTML tags
and silently vanishing from the rendered pages — across ten
files; all 19 are escaped now, one of them at its source (the
`script.devshell` description in the settings registry, whose
generated table row carried the token). One warning remains
by nature: the search index is large (12.8 MB), because
the book is about 405,000 words. It is named here so it reads
as a known quantity, not a defect.

## The gates that hold this

- `TestCommittedRegionsAreCurrent` (`internal/refdoc`) — the
  generated regions of `configuration.md`, `parity.md` and
  `protocol.md` are byte-equal with the declarations they are
  generated from. Regenerate with the same test's `-update`.
- `TestTheDocumentationListsEverySetting` and
  `TestEveryDocumentHasAChapter` (`internal/settings`) — the
  settings inventory and the manifest placement, above.
- `TestEveryDispatchedCommandIsDeclared` (`internal/cli`) —
  declaration and dispatch, both directions.
- The effect-coverage check (`internal/settings`) — every
  setting probed or exempt, the pin only shrinking.
- `create-missing = false` and the docs-twins gate, inside
  the `.#mcpx-docs` build itself.

The rule that follows, and the one this book shares with the
fleet, Atlas and ID books: a change to a command, a flag, a
setting, a transport or a route updates its chapter in the
same change. A follow-up promise is how the gaps this chapter
closed got there.
