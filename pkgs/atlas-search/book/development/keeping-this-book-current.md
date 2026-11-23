# Keeping this book current

Source: `pkgs/atlas-search/book.toml` and `pkgs/atlas-search/README.md` in https://github.com/developing-today/code/blob/main/pkgs/atlas-search/book.toml, and the contract this chapter mirrors, `pkgs/fleet/README.md` §"Operator and contributor contract (DOC-01, #10728)" with `pkgs/fleet/book/development/keeping-this-book-current.md` — provenance only; this chapter is complete without following it.

A book that drifts from the system is worse than no book: agents and
operators script against what it says. Atlas, unlike fleet, had **no**
documentation contract before this book existed — verified for this
book's research: `pkgs/atlas-search/` had no package README and no
`AGENTS.md`, so a contributor's contracts lived scattered across the
repository-root `doc/` design archive, the three package notes in
`docs/`, module-doc headers in the Rust sources, the schema DDL headers,
and the Worker and eval READMEs. This book replaces that scatter with
one reference, and this chapter states the contract that keeps it true.
The contract is modelled on fleet's DOC-01 mechanism; where Atlas's
version is weaker than fleet's, this chapter says so.

## The contract

**Any change to a command, a flag, a default, a config key, an MCP
tool, a REST route, a schema, a model, or an operational procedure
updates the matching chapter of this book in the same change.**

Concretely, in the same change as the code:

1. A new or changed verb or flag → its reference chapter, plus
   [Add a command or a tool](add-a-verb.md) if the change alters the
   pattern that chapter teaches.
2. A new or changed MCP tool → [The Atlas MCP server](../reference/mcp.md);
   a new or changed REST route → [The service HTTP API](../reference/service-api.md)
   or [The Worker HTTP API](../reference/worker-api.md), whichever serves it.
3. A changed default → **every** chapter that states the old default.
   Search the book for the value; defaults are stated in more places
   than you remember.
4. A new or changed config key or environment variable →
   [Configuration and environment](../reference/configuration.md).
5. A schema change → [Data stores and schemas](../reference/data-stores.md),
   and [Add an ingest source](add-a-source.md) if the staging or gate
   route moved. A new schema version is a new `vN` artifact, never an
   edit to an applied one — the book's schema chapter follows the same
   rule and describes the new version beside the old.
6. A model change → [Embeddings: Snowflake Arctic and the parity contract](../concepts/embeddings.md),
   [Data stores and schemas](../reference/data-stores.md) for the new
   vector table, and [Change the embedding model](change-the-model.md)
   if the migration discipline itself changed.
7. A changed operational procedure (ports, service shape, deploy or
   rollback order) → the matching operations chapter:
   [Running the service and the Worker](../operations/services.md) or
   [Release and rollout](release-and-rollout.md).
8. A changed deployment of the book itself →
   [Deploying this book](../operations/deploying-this-book.md).

"Same change" means the same pull request (or the same run, for non-PR
work). A follow-up issue promising to update the book later does not
satisfy the rule — that is exactly how the pre-book documentation
drifted into the disagreements this book's research catalogued: a
design tree reprinted as a layout, a workflow named for pull requests
that never runs on one, a Python header pointing at a README that was
not there.

Two disciplines ride with the rule:

- **Designed versus shipped.** Chapters in this book's design section
  are marked as designs, name their tracking issues, and never present
  a designed behaviour as if it works today. A spelling that is
  designed but not shipped is documented *as contract, not surface*.
- **Source over help.** Where help text and source disagree, the book
  documents the source behaviour and calls out the discrepancy until
  the stale side is fixed. [Building and testing](building.md) carries
  three live examples (the CI triggers, the worker-tests header, the
  toolchain channel); do not "fix" a discrepancy by silently copying
  the wrong side into the book.

## The two build gates

The contract is enforced by two gates, and they are the whole of its
teeth today:

1. **`create-missing = false`.** The book's configuration,
   `pkgs/atlas-search/book.toml` (title `atlas`, sources in
   `pkgs/atlas-search/book/`), sets `create-missing = false`: a chapter
   listed in `SUMMARY.md` that does not exist is a **build failure,
   never a generated stub**. A new chapter is therefore a real file,
   written by its author, or the build is red.
2. **The `.#atlas-docs` Nix build.** From the repository root,
   `nix build .#atlas-docs` builds the rendered site exactly as it
   will be deployed. The `atlas-docs.yml` workflow runs that build, and
   the book ships through the combined GitHub Pages deploy alongside
   the repository's other books, at
   https://developing-today.github.io/code/atlas/.

An honest limitation, recorded so nobody over-trusts the gates: fleet's
contract has a third part Atlas does not have yet — a machine check (a
test) tying the written contract to a registered inventory of the
surface. Atlas has no single registry its help renders from, and no
equivalent test; its nearest machinery is atlas-proto's versioned
query-parameters file and the fleet-search byte-parity guard, which is
dispatch-only. Until an inventory check exists, the same-change rule
above is enforced by the build gates and by review — which is why this
chapter exists in the book itself, where a contributor changing a
surface will meet it.

## Where the rule is recorded

The rule lives in two places, which must agree:

1. `pkgs/atlas-search/README.md` — the package README, added with this
   book (the package had none before it). Its contract section is the
   source of truth a contributor meets at the package root, before
   they open the book.
2. This chapter — the book stating its own maintenance rule, so a
   reader who arrived here first can find the whole contract.

(Fleet's version of the rule lives in three places — its README, its
package's agent documentation, and its book chapter. Atlas has no
package-local agent documentation today; if one is added, it becomes
the third recording, in the same change that adds it.)

If the two recordings disagree, the README wins for package-scope
questions and this chapter wins for book-scope questions — and the
disagreement itself gets fixed in the same pass it is noticed.

The book's own form rules are part of the contract: every file starts
with its exact chapter title (the `SUMMARY.md` text) as its single `# `
heading, and internal links are relative `.md` links whose text is the
target chapter's title. Both keep `SUMMARY.md`, the file and the
rendered page in agreement, and keep the book readable in the
repository as well as on the rendered site.

## Build and preview the book locally

1. From `pkgs/atlas-search/`, with mdBook available, build in place:

   ```sh
   mdbook build
   ```

   The rendered site lands in the build directory named by
   `book.toml`. Open its `index.html` to preview.

2. Or build the packaged form, exactly as the deploy does, from the
   repository root:

   ```sh
   nix build .#atlas-docs
   ```

   The result is the rendered site; serve or open `result/index.html`
   to check links, search and chapter order before merging.

Either route is a valid pre-merge check. The Nix route is the stronger
one: it is the same derivation the deploy uses, with the same mdBook
version.
