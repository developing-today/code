# Deploying this book

This book is an mdBook that lives with the code it documents: sources in `pm-stack/book/`, configuration in `pm-stack/book.toml`, published to the `developing-today/code` GitHub Pages site under `/pm/`. This chapter mirrors the fleet book's chapter of the same name, because the PM book follows the fleet book's build and deploy shape. It describes the pieces, the two workflows, and the local preview.

## The pieces

- `pm-stack/book.toml` — mirrors the fleet book's configuration: `src = "book"` (a dedicated folder), build directory `rendered`, and `create-missing = false` — every chapter in `SUMMARY.md` must exist on disk; the build never invents stubs.
- `pm-stack/book/` — `SUMMARY.md` plus the chapters. `index.md` is the introduction.
- The flake output `.#pm-docs` — the Nix build of the book (`pm-stack/docs.nix`, wired in `lib/default.nix` as `pmDocs`), taking a `siteUrl` argument (default `"/"`) that is baked into the rendered book as mdBook's `output.html.site-url`, injected at build time via the `MDBOOK_OUTPUT__HTML__SITE_URL` environment variable rather than stored in `book.toml`. It mirrors the fleet precedent, `.#fleet-docs` (`pkgs/fleet/docs.nix`).
- Links inside the book are relative `.md` links. Local preview navigation works regardless of `siteUrl`; only absolute site URLs differ between a local build and the deployed one.

## The two workflows

Repository CI runs on `main` only, and the book follows the same policy.

### The build-only workflow

The build-only workflow is `.github/workflows/pm-docs.yml`, mirroring `.github/workflows/fleet-docs.yml`: on push to `main` and on `workflow_dispatch`, it builds `.#pm-docs`, stages the result, and verifies the rendered site (the index page exists). It **never deploys**. Its job is to fail by name on `main` when a chapter breaks the build — a missing `SUMMARY.md` target under `create-missing = false`, a bad link, a broken derivation — before the deploy path is involved.

### `mcpx-docs.yml` — the shared Pages deploy

The deploy lives inside the existing `.github/workflows/mcpx-docs.yml`, whose deploy job is the shared deploy job for the repository's books. The reason is a hard constraint of GitHub Pages:

**One repository has one Pages site, and a Pages deploy replaces the whole site.**

The mcpx book already occupies `https://developing-today.github.io/code/`, and the fleet book deploys through the same job under `fleet/`. A PM-only deploy would publish an artifact containing only the PM book — and wipe the other books off the site. So the shared deploy job builds every book it carries and stages one combined site:

```text
site/          ← the mcpx book, built with siteUrl "/code/"
site/fleet/    ← the fleet book, built with siteUrl "/code/fleet/"
site/atlas/    ← the atlas book, built with siteUrl "/code/atlas/"
site/id/       ← the id book, built with siteUrl "/code/id/"
site/pm/       ← this book, built with siteUrl "/code/pm/"
```

This book is therefore reached at `https://developing-today.github.io/code/pm/`, and its `siteUrl` is `/code/pm/` — the `/code/` project-site prefix plus the `pm/` staging directory.

Adding this book to that deploy job is the high-blast-radius change in the book's setup: a staging error there removes the other books from the site on the next deploy. Parse the workflow YAML and build the deploy variant locally before merge. That check is part of the change, not a follow-up.

GitHub Pages for `developing-today/code` is already enabled with `build_type = "workflow"` (source: GitHub Actions), per the fleet book's record. There is no manual enable step, no `gh-pages` branch, and no generated HTML is ever committed.

## Preview the book locally

1. Build the packaged form from the repository root: `nix build .#pm-docs`.
2. Open or serve the result's `index.html` and check the chapter order, internal links, and search.
3. Or, for a faster edit loop, build in place from `pm-stack/`:

   ```sh
   mdbook build
   ```

   and preview the output directory named by `book.toml`. The Nix build is the authoritative check — it is the same derivation, mdBook version, and `siteUrl` handling the deploy uses.

Note the local default `siteUrl` is `"/"`.

## Custom domains — out of scope

Custom domains are out of scope until asked. No CNAME file for this book is part of its setup, no custom domain is set for it, and its `siteUrl` stays `/code/pm/`. Do not present a custom domain as live, and do not change URLs in anticipation of one. A future custom-domain change for the repository's Pages site is a whole-site change: it touches every book the shared deploy job stages, in the same pass.

Sources. The fleet book's "Deploying this book" chapter (`pkgs/fleet/book/operations/deploying-this-book.md`, read 2026-11-23); skeleton.md, "Deploying this book" placement notes for this programme; the assembly record on #13900 (programme PM-BOOK), which created `pm-stack/docs.nix`, the `.#pm-docs` flake output, `.github/workflows/pm-docs.yml`, and this book's two steps in the `mcpx-docs.yml` deploy job.
