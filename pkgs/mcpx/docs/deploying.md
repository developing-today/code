# Deploying this book

This book is the root of the repository's single GitHub Pages
site, and it shares that site with three sibling books. This
chapter is how the deploy works, so a change to any of the four
books can be landed without wiping the other three.

## One repository, one site

GitHub Pages gives a repository exactly one site, and a
deployment replaces the whole of it. Four books live in this
repository — this one, the fleet book (`pkgs/fleet/book`), the
Atlas book (`pkgs/atlas-search/book`) and the ID book
(`pkgs/id/book`) — so they deploy together, in one workflow, or
not at all: a deploy that published only one book would delete
the other three from the site.

The workflow is `.github/workflows/mcpx-docs.yml`. It has three
jobs:

1. **build** — `nix build .#mcpx-docs`, on every push to main
   and every manual dispatch. Nothing deploys from this job;
   it is the same build the deploy job repeats with the
   deploy `siteUrl`.
2. **deploy** — main only. Builds all four books and stages them
   into one site tree, then uploads a single Pages artifact.
3. **publish** — a separate job that runs `deploy-pages` against
   the uploaded artifact. It is separate on purpose (#10956):
   when publish ran in the same job, it could fetch the artifact
   metadata before the upload had propagated and fail with
   "Found 0 artifact(s)". Deploys are serialised and never
   cancelled by a later one (#1941) — two deploys must never
   interleave.

## Where each book sits

Each book is built with its own `siteUrl`, because mdBook bakes
the base path into its generated pages (with the wrong one, the
404 page points its assets at the domain root and breaks):

| book  | sources                  | siteUrl       | served at                                  |
| ----- | ------------------------ | ------------- | ------------------------------------------ |
| mcpx  | `pkgs/mcpx/docs`         | `/code/`      | `https://developing-today.github.io/code/` |
| fleet | `pkgs/fleet/book`        | `/code/fleet/`  | `…/code/fleet/`  |
| Atlas | `pkgs/atlas-search/book` | `/code/atlas/`  | `…/code/atlas/`  |
| ID    | `pkgs/id/book`           | `/code/id/`     | `…/code/id/`     |

The mcpx book sits at the root because it was there first — it
is the site's original book (#1939, #1941), and the others
joined under subpaths as they were written. The staging step
copies the mcpx build into the site root, then adds `fleet/`,
`atlas/` and `id/` beside it; the copied tree comes out of the
Nix store read-only, so the staging step makes the root
writable before adding the subdirectories (#13225).

Each sibling book also has a build-only workflow of its own
(`fleet-docs.yml`, `atlas-docs.yml`, `id-docs.yml`) that builds
it on every push to main. Those workflows never deploy — deploying
is this workflow's job alone, precisely so no second deploy can
replace the shared site with a partial one.

## What the mcpx build itself does

`.#mcpx-docs` (`pkgs/mcpx/docs.nix`) renders these sources with
mdBook, with two gates inside the build:

- the **docs-history** preprocessor renders release metadata
  and history disclosures into the pages;
- **docs-twins** then emits the agent surface — a `.html.md`
  twin for every page, plus `llms.txt` and `llms-full.txt` —
  and the build *fails* if any page lacks a current twin
  (#1947). The deploy job asserts the twins are present in the
  staged site as well.

`book.toml` sets `create-missing = false`: every chapter in
`SUMMARY.md` must exist on disk, and a summary entry without a
file fails the build instead of publishing a stub.

## After a book change lands

The deploy runs on the push to main. Verify it the way the
failure mode demands — not just the book you changed:

1. The workflow's publish job is green.
2. Fetch the root and every subpath live — `/`, `/fleet/`,
   `/atlas/`, `/id/` — and confirm each serves its book. The
   failure this guards against is a deploy that succeeds by
   wiping a sibling.

## The custom domain

The site is the `github.io` address above. A custom subdomain
for the mcpx docs is a recorded, pending decision (#1943) —
applying the CNAME is the user's call, and until it is made
this chapter describes the site as it is.
