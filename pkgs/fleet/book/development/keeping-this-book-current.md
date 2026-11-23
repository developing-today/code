# Keeping this book current

A book that drifts from the binary is worse than no book: operators script against what it says. Fleet already had a documentation contract before this book existed — DOC-01, in the fleet README — and this chapter states that contract, then extends it to the book itself.

## The DOC-01 contract

DOC-01 is the "Operator and contributor contract" section of `pkgs/fleet/README.md` (#10728). Its contributor paragraph, in substance:

> Keep the command contract together: update `crates/fleet-registry/src/lib.rs` when a top-level help inventory changes, and update `docs/verbs.md` for every changed or added command, including nested leaves and examples, in the same change. Docs must not present a designed spelling as shipped: the runtime dispatch is the authority on what a binary can run. And a change to any settled semantic the section records (claim/park/resume, supervision and stop authority, cap policy, attribution/logging) requires a decision record in the same body of work.

The supporting rules in the same section:

- **The inventory is the registry.** `fleet --help` renders the full registry on every variant and `fleet capabilities` prints it as JSON. The registered inventory lives in one place — `crates/fleet-registry/src/lib.rs` — so no build can know a domain another build forgot. See [Register a domain or capability](register-a-domain.md).
- **Designed-versus-shipped discipline.** Spellings that are designed but not yet shipped are documented *as contract, not surface*, explicitly marked as not in the registered inventory and not dispatchable today, each with its shipped equivalent. Invoking one answers `unknown command`. Never write a designed spelling into a procedure as if it works.
- **Decision records for settled semantics.** A change to claim/park/resume semantics, supervision or stop authority, cap policy, or attribution/logging requires a fleet decision record (FDR, in repository-root `doc/decisions/`, indexed by `doc/decisions/README.md`) in the same body of work — the same pull request, or the same run for non-PR work.
- **A machine check enforces the inventory half.** The test `pmo_operator_docs_match_registered_help_inventory` in `crates/fleet-registry/tests/pmo_operator_docs.rs` ties the README's operator contract to the registered help inventory: a verb added to the registry without its contract docs, or vice versa, fails the workspace test suite. See [Testing](testing.md).

## The extension: the book is part of the same change

This book is now one of the surfaces DOC-01 covers. The rule:

**Any change to a command, a flag, a default, a capability, a surface, or an operational procedure updates the matching chapter of this book in the same pull request.**

Concretely, in the same change as the code:

1. A new or changed verb or flag → its command-reference chapter, plus [Add a subcommand](add-a-subcommand.md) or [Add a flag](add-a-flag.md) if the change alters the pattern those chapters teach, plus `pkgs/fleet/docs/verbs.md` per DOC-01.
2. A new domain or capability → [Register a domain or capability](register-a-domain.md) and [Repository layout](layout.md).
3. A changed default → every chapter that states the old default. Search the book for the value; defaults are stated in more places than you remember.
4. A changed operational procedure (ports, profile names, heal order, rollout order) → the matching operations chapter: [Running the brokers as services](../operations/services.md), [Health checks and the heal stack](../operations/health-and-healing.md), or [Release and rollout](release-and-rollout.md).
5. A changed deployment of the book itself → [Deploying this book](../operations/deploying-this-book.md).

"Same change" means same pull request. A follow-up issue promising to update the book later does not satisfy the rule — that is exactly how the pre-book docs drifted into the disagreements the research for this book catalogued (a README package count that no longer matched `default.nix`, usage strings that disagreed with each other, help text describing an earlier epic step as current). If you find such a disagreement while writing, fix the stale side if it is in your change's scope; otherwise file an issue in the owning repository in the same pass and note it in your change.

## The two build gates

Two mechanical gates keep the book honest:

1. **`create-missing = false`.** The book's `book.toml` sets `create-missing = false`, with the standing rule inherited from the mcpx exemplar: every chapter in `SUMMARY.md` must exist on disk; never invent stubs. Adding a `SUMMARY.md` entry without the file fails the build instead of silently publishing an empty page.
2. **The Nix build.** The book builds as the `.#fleet-docs` package (see [Deploying this book](../operations/deploying-this-book.md)). The build-only workflow `fleet-docs.yml` runs that build on `main`, and the combined Pages deploy rebuilds it again. A chapter that breaks mdBook — a bad link target, a missing file — fails the build by name rather than shipping broken.

Internal links in this book are relative `.md` links only, and every file starts with its exact chapter title as its single `# ` heading. Both are part of the contract: relative links keep the book readable in the repository as well as on the rendered site, and the title rule keeps `SUMMARY.md`, the file, and the rendered page in agreement.

## Where the rule is recorded

The rule lives in three places, which must agree:

1. `pkgs/fleet/README.md` — the DOC-01 contract itself, the source of truth for the registry and `verbs.md` halves.
2. `pkgs/fleet/doc/agents/documentation.md` — the agent-facing documentation rules inside the package, which carry the book rule for sessions working in this tree.
3. This chapter — the book stating its own maintenance rule, so a reader who arrived here first can find the whole contract.

If the three disagree, the README's DOC-01 text wins for command-contract questions and this chapter wins for book-scope questions — and the disagreement itself gets fixed in the same pass it is noticed.

## Build and preview the book locally

1. From `pkgs/fleet/`, with mdBook available, build in place:

   ```sh
   mdbook build
   ```

   The rendered site lands in the build directory named by `book.toml`. Open its `index.html` to preview.
2. Or build the packaged form, exactly as CI does, from the repository root:

   ```sh
   nix build .#fleet-docs
   ```

   The result is the rendered site; serve or open `result/index.html` to check links, search, and chapter order before merging.

Either route is a valid pre-merge check. The Nix route is the stronger one: it is the same derivation the deploy uses, with the same mdBook version.
