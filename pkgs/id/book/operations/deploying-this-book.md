# Deploying this book

This book is one of four books on **one** GitHub Pages site. That single fact decides everything about how it deploys: the id book is never deployed on its own. Its build is checked on its own; its deployment happens only as one staging step inside the combined deploy that publishes the whole site at once. The live book is at **https://developing-today.github.io/code/id/**.

## The pieces

| Piece                             | What it is                                                                                                                                                                                                                                                                    |
| --------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `pkgs/id/book/`                   | The book sources — the Markdown chapters, in the layout the summary file names                                                                                                                                                                                                |
| `pkgs/id/book.toml`               | The mdBook configuration: title `id`, `src = "book"`, `build-dir = "rendered"`, `create-missing = false` — a chapter named in the summary but missing on disk is a build failure, never a silently generated stub                                                             |
| `pkgs/id/docs.nix`                | The build: an mdBook build of the sources above, sandboxed, with the flake-pinned mdBook from nixpkgs and no network access. It rewrites links that escape the book tree to their repository URLs in the rendered copy only, and takes the base URL as its `siteUrl` argument |
| `lib/default.nix`                 | Exposes that build to the flake as **`.#id-docs`**                                                                                                                                                                                                                            |
| `.github/workflows/id-docs.yml`   | The build-only workflow for this book (below)                                                                                                                                                                                                                                 |
| `.github/workflows/mcpx-docs.yml` | The combined deploy workflow for the whole Pages site (below)                                                                                                                                                                                                                 |

Build the book locally exactly as CI does:

```text
nix build .#id-docs
```

The flake output uses the default `siteUrl` of `/`. The Pages deploy overrides it with `/code/id/` — the id book's path inside the repository's project site. mdBook needs the real hosting path, or its generated 404 page points assets at the domain root.

## The two workflows

### `id-docs.yml` — build only

This workflow runs on pushes to `main` and on manual dispatch. It installs Nix, builds `.#id-docs` with `nix build .#id-docs --print-build-logs`, then stages the result — `cp -rL result site`, dereferencing the store symlink so the staged tree is a plain static site — and asserts `site/index.html` exists. It is the gate: a broken chapter, a missing chapter (`create-missing = false`), or a failed book check fails the build here, before anything can deploy.

It publishes nothing. Its permissions are `contents: read` only; there is no Pages deploy step in it, and adding one would be the standalone-deploy mistake described below. Its own header states the reason in full: a repository has exactly one GitHub Pages site and every Pages deployment replaces the whole site, so the id book is deployed by the `mcpx-docs` workflow's deploy job, staged under `id/` with `siteUrl` `/code/id/`.

### `mcpx-docs.yml` — the combined deploy

This is the repository's one Pages deployment. It builds each book and stages them into **one site tree**, then deploys that tree as the site's single artifact. The mcpx book is the site root; the other three books are staged under it:

| Staged path         | Book           | Site URL                                         |
| ------------------- | -------------- | ------------------------------------------------ |
| Site root (`site/`) | The mcpx book  | `https://developing-today.github.io/code/`       |
| `site/fleet/`       | The fleet book | `https://developing-today.github.io/code/fleet/` |
| `site/atlas/`       | The atlas book | `https://developing-today.github.io/code/atlas/` |
| `site/id/`          | This book      | `https://developing-today.github.io/code/id/`    |

Each book is built with its own `siteUrl` override — `/code/` for the root mcpx book, `/code/fleet/`, `/code/atlas/` and `/code/id/` for the staged books — the path prefix the rendered links and assets must assume, because those books do not live at the site root. The deploy then publishes the assembled `site/` tree once, through the Pages artifact flow: `actions/configure-pages`, `actions/upload-pages-artifact`, and `actions/deploy-pages` in a separate publish job, serialised so two deploys never interleave.

## The one-site rule

GitHub Pages serves **one site per repository**, and a Pages deployment replaces that site with the artifact it is given. The rule follows directly:

> **Never deploy the id book on its own.** A workflow that uploaded only the id build output as the Pages artifact would publish a site containing only id — and **wipe the mcpx book at the root, the fleet book at `/code/fleet/` and the atlas book at `/code/atlas/`** until the next combined deploy restored them.

Every id deploy therefore goes through `mcpx-docs.yml`'s combined staging, even when only id chapters changed. The same rule protects id in the other direction: no other book's change can deploy without staging id alongside it. If you find a draft workflow, a manual `gh` Pages call, or a helpful script that deploys `.#id-docs` output directly, that is the bug — do not run it.

## The writability fix when staging

Built book output arrives from the Nix store **read-only**, and a site tree assembled by copying store output inherits that. A later staging step that copies the next book into — or over — a read-only part of the tree then fails with permission errors, typically on the second or third book staged, which makes it look like that book's build is at fault. It is not: it is the tree.

The fix, applied in the workflow whenever a built book is copied into the copied site tree, is to make the staged tree writable before the next staging step touches it:

```text
cp -rL <built-book-output> site/id
chmod u+w site
```

The workflow applies `chmod u+w site` to the site root before each staged book is copied in. Keep the `chmod` with the copy it belongs to. Removing it because one run happened to pass reintroduces an order-dependent failure that will return the first time the staging order or the store paths change. The id book's own build does the same thing internally for a different reason: `pkgs/id/docs.nix` copies its sources into a writable work directory (`chmod -R u+w work`) before running mdBook, because the store copy it starts from is read-only.

## Verification after a deploy

A green combined deploy is not the verification. Check all four books, every time, because the failure this deploy shape risks is another book disappearing:

1. **id is live.** `https://developing-today.github.io/code/id/` returns the book's front page, and a chapter page under it (this chapter, for instance) renders with working links — which also proves the `/code/id/` site URL was staged correctly.
2. **The mcpx root still serves.** `https://developing-today.github.io/code/` returns the mcpx book, not a 404 and not the id book.
3. **The fleet book still serves.** `https://developing-today.github.io/code/fleet/` returns the fleet book.
4. **The atlas book still serves.** `https://developing-today.github.io/code/atlas/` returns the atlas book.

If check 1 passes and any of checks 2 to 4 fails, the deploy was not combined — treat it as the one-site rule having been broken, and re-run the combined `mcpx-docs.yml` deploy to restore the full tree. If check 1 fails while 2 to 4 pass, the id staging step is the suspect: rebuild `.#id-docs` locally, confirm the output exists, and check the staging copy and the writability fix above before re-deploying.

## Custom domain: not wired

There is **no custom domain** for this book, or for the site it lives on. The canonical address is the `github.io` URL above; no `CNAME` is set, and no DNS record points a fleet domain at the Pages site.

A later DNS step — choosing a hostname, adding its DNS records, and configuring the domain on the Pages site — is documented as future work and is **not wired**. Until that step is deliberately done, do not add a `CNAME` file to any book's output, do not set a custom domain in the repository's Pages settings as part of a book change, and do not publish links under any domain other than `developing-today.github.io`. A half-wired domain would move all four books at once, which is exactly the blast radius the one-site rule exists to keep visible. The standing rule for keeping the chapters themselves current as the system changes is in [Keeping this book current](../development/keeping-this-book-current.md).
