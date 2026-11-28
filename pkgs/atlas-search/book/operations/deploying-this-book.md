# Deploying this book

Source: `pkgs/atlas-search/book/`, `pkgs/atlas-search/book.toml`, `pkgs/atlas-search/docs.nix`, `lib/default.nix`, `.github/workflows/atlas-docs.yml`, and `.github/workflows/mcpx-docs.yml` under https://github.com/developing-today/code/blob/main/pkgs/atlas-search/book.toml — provenance only; this chapter is complete without following it.

This book is one of three books on **one** GitHub Pages site. That single fact decides everything about how it deploys: the Atlas book is never deployed on its own. Its build is checked on its own; its deployment happens only as one staging step inside the combined deploy that publishes the whole site at once. The live book is at **https://developing-today.github.io/code/atlas/**.

## The pieces

| Piece | What it is |
|---|---|
| `pkgs/atlas-search/book/` | The book sources — the Markdown chapters, in the layout the summary file names |
| `pkgs/atlas-search/book.toml` | The mdBook configuration: title `atlas`, `src = "book"`, `create-missing = false` — a chapter named in the summary but missing on disk is a build failure, never a silently generated stub |
| `pkgs/atlas-search/docs.nix` | The build: an mdBook build of the sources above, offline, with the book's checks |
| `lib/default.nix` | Exposes that build to the flake as **`.#atlas-docs`** |
| `.github/workflows/atlas-docs.yml` | The build-only workflow for this book (below) |
| `.github/workflows/mcpx-docs.yml` | The combined deploy workflow for the whole Pages site (below) |

Build the book locally exactly as CI does:

```
$ nix build .#atlas-docs
```

## The two workflows

### `atlas-docs.yml` — build only

This workflow builds `.#atlas-docs` and nothing more. It is the gate: a broken chapter, a missing chapter (`create-missing = false`), or a failed book check fails the build here, before anything can deploy. It publishes nothing — there is no Pages deploy step in it, and adding one would be the standalone-deploy mistake described below.

### `mcpx-docs.yml` — the combined deploy

This is the repository's one Pages deployment. It builds each book and stages them into **one site tree**, then deploys that tree as the site's single artifact:

| Staged path | Book | Site URL |
|---|---|---|
| Site root (`site/`) | The mcpx book | `https://developing-today.github.io/code/` |
| `site/fleet/` | The fleet book | `https://developing-today.github.io/code/fleet/` |
| `site/atlas/` | This book | `https://developing-today.github.io/code/atlas/` |

The Atlas staging step builds `.#atlas-docs` and copies the result into `site/atlas`, with the book's site URL configured as **`/code/atlas/`** — the path prefix the rendered links and assets must assume, because the book does not live at the site root. The deploy step then publishes the assembled `site/` tree once.

## The one-site rule

GitHub Pages serves **one site per repository**, and a Pages deployment replaces that site with the artifact it is given. The rule follows directly:

> **Never deploy the Atlas book on its own.** A workflow that uploaded only the Atlas build output as the Pages artifact would publish a site containing only Atlas — and **wipe the mcpx book at the root and the fleet book at `/code/fleet/`** until the next combined deploy restored them.

Every Atlas deploy therefore goes through `mcpx-docs.yml`'s combined staging, even when only Atlas chapters changed. The same rule protects Atlas in the other direction: no other book's change can deploy without staging Atlas alongside it. If you find a draft workflow, a manual `gh` Pages call, or a helpful script that deploys `.#atlas-docs` output directly, that is the bug — do not run it.

## The writability fix when staging

Built book output arrives from the Nix store **read-only**, and a site tree assembled by copying store output inherits that. A later staging step that copies the next book into — or over — a read-only part of the tree then fails with permission errors, typically on the second or third book staged, which makes it look like that book's build is at fault. It is not: it is the tree.

The fix, applied in the workflow whenever a built book is copied into the copied site tree, is to make the staged tree writable before the next staging step touches it:

```
$ cp -r &lt;built-book-output&gt; site/atlas
$ chmod -R u+w site
```

Keep the `chmod` with the copy it belongs to. Removing it because one run happened to pass reintroduces an order-dependent failure that will return the first time the staging order or the store paths change.

## Verification after a deploy

A green combined deploy is not the verification. Check all three books, every time, because the failure this deploy shape risks is another book disappearing:

1. **Atlas is live.** `https://developing-today.github.io/code/atlas/` returns the book's front page, and a chapter page under it (this chapter, for instance) renders with working links — which also proves the `/code/atlas/` site URL was staged correctly.
2. **The mcpx root still serves.** `https://developing-today.github.io/code/` returns the mcpx book, not a 404 and not the Atlas book.
3. **The fleet book still serves.** `https://developing-today.github.io/code/fleet/` returns the fleet book.

If check 1 passes and either of checks 2 or 3 fails, the deploy was not combined — treat it as the one-site rule having been broken, and re-run the combined `mcpx-docs.yml` deploy to restore the full tree. If check 1 fails while 2 and 3 pass, the Atlas staging step is the suspect: rebuild `.#atlas-docs` locally, confirm the output exists, and check the staging copy and the writability fix above before re-deploying.

## Custom domain: not wired

There is **no custom domain** for this book, or for the site it lives on. The canonical address is the `github.io` URL above; no `CNAME` is set, and no DNS record points a fleet domain at the Pages site.

A later DNS step — choosing a hostname, adding its DNS records, and configuring the domain on the Pages site — is documented as future work and is **not wired**. Until that step is deliberately done, do not add a `CNAME` file to any book's output, do not set a custom domain in the repository's Pages settings as part of a book change, and do not publish links under any domain other than `developing-today.github.io`. A half-wired domain would move all three books at once, which is exactly the blast radius the one-site rule exists to keep visible. The standing rule for keeping the chapters themselves current as the system changes is in [Keeping this book current](../development/keeping-this-book-current.md).
