# Deploying this book

This book is an mdBook that lives with the subject it documents:
sources in `pkgs/lab/book/`, configuration in
`pkgs/lab/book.toml`, build definition in `pkgs/lab/docs.nix`,
published to the `developing-today/code` GitHub Pages site under
`/lab/`. This chapter describes exactly how that works.

For the rule that keeps the book's content current, see
[Keeping this book current](../development/keeping-this-book-current.md).

## The pieces

- `pkgs/lab/book.toml` — mirrors the fleet book's configuration:
  title "Lab Book", `src = "book"` (a dedicated folder), build
  directory `rendered`, and `create-missing = false` — every
  chapter in `SUMMARY.md` must exist on disk; the build never
  invents stubs.
- `pkgs/lab/book/` — `SUMMARY.md` plus the chapters. `index.md` is
  the introduction.
- `pkgs/lab/docs.nix` — mirrors `pkgs/fleet/docs.nix`. It takes a
  `siteUrl` argument (default `"/"`) that is baked into the
  rendered book as mdBook's `output.html.site-url`, injected at
  build time via the `MDBOOK_OUTPUT__HTML__SITE_URL` environment
  variable rather than stored in `book.toml`. It also rewrites
  links that escape the book source tree to the file's repository
  URL (`https://github.com/developing-today/code/blob/main/pkgs/lab/`)
  in the copied pages only — the sources keep their relative
  links for GitHub.
- `lib/default.nix` — exposes the package next to its sibling
  books: `labDocs = pkgs.callPackage ../pkgs/lab/docs.nix { };`
  and `lab-docs = labDocs;` in the packages set, so the build is:

  ```sh
  nix build .#lab-docs
  ```

## The two workflows

Repository CI runs on `main` only, and the book follows the same
policy.

### `lab-docs.yml` — build only

`.github/workflows/lab-docs.yml` mirrors the fleet book's build
job: on push to `main` and on `workflow_dispatch`, it builds
`.#lab-docs`, stages the result, and verifies the rendered site
(the index page exists). It **never deploys**. Its job is to fail
by name on `main` when a chapter breaks the build — a missing
`SUMMARY.md` target under `create-missing = false`, a bad link, a
broken derivation — before the deploy path is involved.

### `mcpx-docs.yml` — the combined Pages deploy

The deploy lives inside the existing
`.github/workflows/mcpx-docs.yml`, whose deploy staging gains the
Lab Book. The reason is a hard constraint of GitHub Pages:

**One repository has one Pages site, and a Pages deploy replaces
the whole site.**

The mcpx book already occupies
`https://developing-today.github.io/code/`, and the sibling books
sit under it (the fleet book under `/code/fleet/`, the Code Book
under `/code/code/`). A lab-only deploy would publish an artifact
containing only the Lab Book — and wipe every sibling book off
the site. So the deploy job builds all the books and stages one
combined site:

```text
site/          ← the mcpx book, built with siteUrl "/code/"
site/lab/      ← the Lab Book, built with siteUrl "/code/lab/"
```

The Lab Book is therefore reached at
`https://developing-today.github.io/code/lab/`, and its `siteUrl`
is `/code/lab/` — the `/code/` project-site prefix plus the
`lab/` staging directory. The combined artifact then goes
through the same publish shape as the sibling books: a staging
job (`configure-pages` + `upload-pages-artifact`) and a separate
publish job with the `pages` concurrency group.

The combined-deploy staging for this book is landed by the
programme coordinator; this book's own change deliberately does
not edit `mcpx-docs.yml`.

GitHub Pages for `developing-today/code` is **already enabled**
with `build_type = "workflow"` (source: GitHub Actions). There
is no manual enable step, no `gh-pages` branch, and no generated
HTML is ever committed.

## Preview the book locally

1. Build the packaged form from the repository root:

   ```sh
   nix build .#lab-docs
   ```

2. Open or serve `result/index.html` and check the chapter order,
   internal links, and search.
3. Or, for a faster edit loop, build in place from `pkgs/lab/`:

   ```sh
   mdbook build
   ```

   and preview the output directory named by `book.toml`. The Nix
   build is the authoritative check — it is the same derivation,
   mdBook version, and `siteUrl` handling the deploy uses.

Note the local default `siteUrl` is `"/"`. Links inside the book
are relative `.md` links, so local preview navigation works
regardless; only absolute site URLs differ between the local
build and the deployed one.
