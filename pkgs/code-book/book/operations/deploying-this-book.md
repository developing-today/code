# Deploying this book

This book is an mdBook that lives with the project it documents: sources in
`pkgs/code-book/book/`, configuration in `pkgs/code-book/book.toml`, build
definition in `pkgs/code-book/docs.nix`, published to the
`developing-today/code` GitHub Pages site under `/code/`. This chapter
describes exactly how that works — and, just as explicitly, the combined
deploy it joins and the custom-domain step that is not wired yet.

For the rule that keeps the book's content current, see [Keeping this book
current](../development/keeping-this-book-current.md).

## The pieces

- `pkgs/code-book/book.toml` — mirrors the fleet book's configuration:
  title "Code Book", `src = "book"` (a dedicated folder), build directory
  `rendered`, and `create-missing = false` — every chapter in `SUMMARY.md`
  must exist on disk; the build never invents stubs. The book is plain
  mdBook: no custom preprocessors.
- `pkgs/code-book/book/` — `SUMMARY.md` plus the chapters. `index.md` is
  the introduction.
- `pkgs/code-book/docs.nix` — mirrors `pkgs/fleet/docs.nix`. It takes a
  `siteUrl` argument (default `"/"`) that is baked into the rendered book
  as mdBook's `output.html.site-url`, injected at build time via the
  `MDBOOK_OUTPUT__HTML__SITE_URL` environment variable rather than stored
  in `book.toml`. Only the book inputs enter the store: `book.toml` plus
  the `book` tree.
- `lib/default.nix` — exposes the package next to its siblings:
  `codeDocs = pkgs.callPackage ../pkgs/code-book/docs.nix { };` and
  `code-docs = codeDocs;` in the packages set, so the build is:

  ```sh
  nix build .#code-docs
  ```

## The two workflows

Repository CI runs on `main` only, and the book follows the same policy.

### `code-book-docs.yml` — build only

`.github/workflows/code-book-docs.yml` mirrors the fleet book's build job:
on push to `main` and on `workflow_dispatch`, it builds `.#code-docs`,
stages the result, and verifies the rendered site (the index page exists).
It **never deploys**. Its job is to fail by name on `main` when a chapter
breaks the build — a missing `SUMMARY.md` target under
`create-missing = false`, a bad link, a broken derivation — before the
deploy path is involved.

### `mcpx-docs.yml` — the combined Pages deploy

The deploy lives inside the existing `.github/workflows/mcpx-docs.yml`,
whose deploy staging gains this book (that staging edit is landed
separately by the programme coordinator; this book's own change does not
touch that workflow). The reason is a hard constraint of GitHub Pages:

**One repository has one Pages site, and a Pages deploy replaces the whole
site.**

The mcpx book already occupies `https://developing-today.github.io/code/`.
A code-book-only deploy would publish an artifact containing only this book
— and wipe every sibling book off the site. So the deploy job builds all
the books and stages one combined site:

```text
site/          ← the mcpx book, built with siteUrl "/code/"
site/fleet/    ← the fleet book, built with siteUrl "/code/fleet/"
site/atlas/    ← the Atlas book, built with siteUrl "/code/atlas/"
site/id/       ← the ID book, built with siteUrl "/code/id/"
site/lab/      ← the Lab Book, built with siteUrl "/code/lab/"
site/code/     ← this book, built with siteUrl "/code/code/"
```

This book is therefore reached at
`https://developing-today.github.io/code/code/`, and its deploy `siteUrl`
is `/code/code/` — the `/code/` project-site prefix plus the `code/`
staging directory. The combined artifact then goes through the same
publish shape as before: a staging job (`configure-pages` plus
`upload-pages-artifact`) and a separate publish job with the `pages`
concurrency group and one retry.

GitHub Pages for `developing-today/code` is **already enabled** with
`build_type = "workflow"` (source: GitHub Actions). There is no manual
enable step, no `gh-pages` branch, and no generated HTML is ever committed.

## Preview the book locally

1. Build the packaged form from the repository root:

   ```sh
   nix build .#code-docs
   ```

2. Open or serve `result/index.html` and check the chapter order, internal
   links, and search.
3. Or, for a faster edit loop, build in place from `pkgs/code-book/`:

   ```sh
   mdbook build
   ```

   and preview the output directory named by `book.toml`. The Nix build is
   the authoritative check — it is the same derivation, mdBook version, and
   `siteUrl` handling the deploy uses.

Note the local default `siteUrl` is `"/"`. Links inside the book are
relative `.md` links, so local preview navigation works regardless; only
absolute site URLs differ between the local build and the deployed one.

## The future custom-domain step — not wired yet

Per the owner's steer, the project's books will eventually be served from a
custom domain instead of the `github.io` path. That step is **planned, not
wired**: there is no CNAME file anywhere in the repository today, no custom
domain is set on the Pages site, and the `siteUrl` values above still point
at the `/code/…` paths. Do not present the custom domain as live, and do
not "fix" URLs in anticipation of it.

When the step is taken, it has exactly three parts, done together:

1. **CNAME file** — add the CNAME file naming the chosen domain to the
   deployed site artifact (it must ride in the combined staging of
   `mcpx-docs.yml`, alongside every book, or the next deploy removes it —
   whole-site replacement cuts both ways).
2. **DNS record** — create the DNS record for the domain pointing at GitHub
   Pages, in the account that owns the domain's zone.
3. **Pages custom domain setting** — set the custom domain on the
   repository's Pages configuration so GitHub provisions the certificate
   and redirects the `github.io` address.

The same change must also move every book's deploy `siteUrl` value from the
`/code/…` paths to the domain-rooted paths, and update [Keeping this book
current](../development/keeping-this-book-current.md) and this chapter to
describe the wired state. Until that change lands, this section stands as
the record of intent.
