# Keeping this book current

Source: `pkgs/id/book.toml`, `pkgs/id/docs.nix` and `.github/workflows/id-docs.yml` as supplied with this book, and the contract form this chapter mirrors in `pkgs/atlas-search/book/development/keeping-this-book-current.md` — provenance only; this chapter is complete without following it.

A book that drifts from the system is worse than no book: agents and operators script against what it says. Before this book, ID's documentation lived in the package documents `pkgs/id/README.md`, `pkgs/id/ARCHITECTURE.md`, `pkgs/id/WEB.md` and `pkgs/id/TODO.md`, in module-doc headers across the crates, and in the design-record archive at the repository root. That scatter drifted: the book's research catalogued 56 numbered disagreements across its four surveys — a command table that omits a command its own file documents in a full section, a quick-start that equates two different builds, a lock filename spelled three ways. This book replaces the scatter with one reference, and this chapter states the contract that keeps it true.

## The contract

**Any change to a command, a flag, a default, a config key, an MCP tool, a route, a schema or an operational procedure updates the matching chapter of this book in the same change.**

Concretely, in the same change as the code:

1. A new or changed command or flag → its reference chapter, plus [Add a command](add-a-verb.md) if the change alters the pattern that chapter teaches.
2. A new or changed MCP tool → [The MCP bridge](../reference/mcp.md). A new or changed web route → [The web UI: pages and HTTP API](../reference/web-ui.md) or [WebSocket protocols](../reference/websockets.md), whichever serves it. A new or changed hosted Worker route → [The hosted Worker API](../reference/hosted-worker.md). A new or changed mail Worker route, caller grant, template, limit or binding (in `nixos/services/id-mail/worker/config.json` and its sources) → [The mail Worker API](../reference/mail-worker.md).
3. A changed default → **every** chapter that states the old default. Search the book for the value; defaults are stated in more places than you remember.
4. A new or changed config key or environment variable → [Configuration and environment](../reference/configuration.md).
5. A schema or data-format change → [Data stores and schemas](../reference/data-stores.md). A new format version is described beside the old one, never written over it: the old version's readers are still in the field during a rollout, per [Release and rollout](release-and-rollout.md).
6. A change to the crate layout, the workspace membership or the facade surface → [Repository layout](layout.md) and [The crates](../library/crates.md).
7. A change to the platform package, the Plaza ABI or the capability protocol → [Add a Roc guest app](add-a-guest.md) and [Worlds and the Roc guest runtime](../concepts/worlds.md).
8. A changed build, test or toolchain fact → [Building and testing](building.md).
9. A changed operational procedure (ports, service shape, deploy or rollback order) → the matching operations chapter: [Running a node: serve, worlds and the hosted edition](../operations/services.md) or [Release and rollout](release-and-rollout.md).
10. A changed deployment of the book itself → [Deploying this book](../operations/deploying-this-book.md).

"Same change" means the same pull request (or the same run, for non-PR work). A follow-up issue promising to update the book later does not satisfy the rule — that is exactly how the pre-book documentation drifted into the disagreements named above.

Two disciplines ride with the rule:

- **Designed versus shipped.** Chapters in this book's design section are marked as designs, name their tracking issues, and never present a designed behaviour as if it works today. The discipline applies outside the design section too: the hosted edition is a local-only scaffold, and every chapter that touches it — [The hosted edition](../concepts/hosted.md), [The hosted Worker API](../reference/hosted-worker.md), [Release and rollout](release-and-rollout.md) — states that status instead of describing a live service. A spelling that is designed but not shipped is documented as contract, not surface.
- **Source over help.** Where help text and source disagree, the book documents the source behaviour and calls out the discrepancy until the stale side is fixed. Worked examples already in the book: `id list` has no `ls` alias in the parser although the package README's command table gives it one — the alias exists only inside the REPL and on `tag list` — and the `serve --topic` help presents the hardcoded fallback constants as the defaults, although the operative defaults normally come from the embedded `defaults.json`. Do not "fix" a discrepancy by silently copying the wrong side into the book.

A third, smaller discipline governs numbers: **test counts are dated measurements.** A count is stated with the run, the date and the commit it was measured at, or it is marked unverified. Standing totals copied from older documents are how the pre-book counts — repeated identically across three package documents, substantiated by none of them — survived for months.

## The two build gates

The contract is enforced by two gates, and they are the whole of its teeth today:

1. **`create-missing = false`.** The book's configuration, `pkgs/id/book.toml` (title `id`, sources in `pkgs/id/book/`), sets `create-missing = false` under `[build]`: a chapter listed in `SUMMARY.md` that does not exist is a **build failure, never a generated stub**. A new chapter is therefore a real file, written by its author, or the build is red.
2. **The `.#id-docs` Nix build on `main`.** The workflow `.github/workflows/id-docs.yml` runs `nix build .#id-docs --print-build-logs` on every push to `main` and on manual dispatch, then stages the result and checks that the staged tree contains an `index.html`. The derivation is `pkgs/id/docs.nix`: it renders `book.toml` plus the `book/` tree with the flake-pinned mdBook, with no network access, exactly as the deploy will render it. The workflow builds only; it never deploys — deployment is the combined Pages deploy described in [Deploying this book](../operations/deploying-this-book.md).

An honest limitation, recorded so nobody over-trusts the gates: neither gate checks that a chapter's *contents* still match the code. They prove the book is complete and renders; the same-change rule above, enforced by review, is what keeps it true — which is why this chapter exists in the book itself, where a contributor changing a surface will meet it.

## Where the rule is recorded

The rule lives in two places, which must agree:

1. This chapter — the book stating its own maintenance rule, so a reader who arrived here first can find the whole contract.
2. The DOC-01 contract in `pkgs/id/README.md` — a contract section in the package readme, naming this book as the package's documentation of record and restating the same-change rule for a contributor who meets the package root before they open the book.

The second recording is stated here as the contract's requirement, in the future tense where it belongs: at the time this chapter was written, `pkgs/id/README.md` contained no DOC-01 section — its documentation pointer still described the `docs/` symlink as feature documentation, which the design survey records as disagreement DES-5. Adding the DOC-01 section to the readme is part of landing this book, and until it lands, this chapter is the rule's only recording. If the two recordings later disagree, the readme wins for package-scope questions and this chapter wins for book-scope questions — and the disagreement itself gets fixed in the same pass it is noticed.

The book's own form rules are part of the contract: every file starts with its exact chapter title (the `SUMMARY.md` text) as its single `# ` heading, and internal links are relative `.md` links whose text is the target chapter's title. Both keep `SUMMARY.md`, the file and the rendered page in agreement, and keep the book readable in the repository as well as on the rendered site.

## Build and preview the book locally

1. From `pkgs/id/`, with mdBook available, build in place:

   ```sh
   mdbook build
   ```

   The rendered site lands in the build directory named by `book.toml`, which is `rendered`. Open its `index.html` to preview.

2. Or build the packaged form, exactly as the gate and the deploy build it, from the repository root:

   ```sh
   nix build .#id-docs
   ```

   The result is the rendered site; serve or open `result/index.html` to check links, search and chapter order before merging.

Either route is a valid pre-merge check. The Nix route is the stronger one: it is the same derivation the gate and the deploy use, with the same mdBook version, and it applies the link rewriting for pages outside the book tree that a plain mdBook build does not.
