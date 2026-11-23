# Documentation: the fleet book is part of the change

`fleet` has a book: `pkgs/fleet/book/` (mdBook sources, `pkgs/fleet/book.toml`),
rendered by the flake output `.#fleet-docs` (`pkgs/fleet/docs.nix`) and
published inside the repository's GitHub Pages site at `/code/fleet/`.
The book is the complete, self-contained reference for the binary: every
domain, subcommand, flag, default, and operational procedure.

## The rule (DOC-01, extended)

The README's DOC-01 contract already requires the domain registry and
`pkgs/fleet/docs/verbs.md` to be updated in the same change as the code.
The book is part of that same contract:

- Any change to a command, subcommand, flag, default, exit code,
  capability, surface-registry entry, broker behaviour, or operational
  procedure **updates the matching book chapter in the same PR**.
- A new subcommand or flag is not done until its reference section
  exists in the book, following the worked traces in the book's
  development guide ("Add a subcommand", "Add a flag").
- Designed-but-unshipped spellings are never presented in the book as
  shipped behaviour (the DOC-01 discipline); where help text and source
  disagree, the book documents the source and calls out the discrepancy
  until the help is fixed.

## Gates

- `book.toml` sets `create-missing = false`: a `SUMMARY.md` entry with
  no file on disk fails the build instead of inventing a stub.
- `nix build .#fleet-docs` builds the whole book offline; the
  `fleet-docs` workflow runs it on every push to main.
- The usage↔parser parity test still guards the help text; the book is
  checked by review against the same sources.

## Building and previewing

```sh
cd pkgs/fleet
mdbook build            # renders into pkgs/fleet/rendered/
nix build .#fleet-docs  # the exact CI build, from the repo root
```

Deployment is combined with the mcpx book's (one repository, one Pages
site, whole-site deploys): see the book's "Deploying this book" chapter.
