# The web UI: pages and HTTP API

Source: https://github.com/developing-today/code/blob/main/pkgs/id/crates/id-web/src/routes.rs, https://github.com/developing-today/code/blob/main/pkgs/id/crates/id-web/src/security.rs, https://github.com/developing-today/code/blob/main/pkgs/id/crates/id-web/src/assets.rs, https://github.com/developing-today/code/blob/main/pkgs/id/web/package.json

The web UI is served by `id serve --web`. It lives in the `id-web` crate (`crates/id-web`) with a Bun + TypeScript frontend in `pkgs/id/web`. The main router has **54 distinct paths and 58 method bindings**. A second, program-only router has **10 paths**. Both are constructed in `crates/id-web/src/lib.rs` and `routes.rs`:

| Constructor             | Surface                                                                                                                                                                               |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `create_router`         | The main Explorer router below.                                                                                                                                                       |
| `create_router_for_app` | The Explorer router parameterised for one app.                                                                                                                                        |
| `create_program_router` | The program-only router: one listener pinned to one world.                                                                                                                            |
| Publication mounts      | A dynamic mount that rewrites a mounted path prefix into the program router inside the listener's security layer. Publications are covered in [serve and publications](cli-serve.md). |

Serving defaults, from the CLI: `--web` is off by default, `--port` defaults to `3000`, `--bind` defaults to `127.0.0.1`. The repeatable `--web-listen <IP:PORT>` flag replaces the `web_bindings` list from the embedded `defaults.json`; see [Configuration and environment](configuration.md). Multiple listeners can each be an Explorer listener, a program listener (`program:<world>`), or a document target.

## Authentication and the security layer (WEB-15)

Every router is wrapped by `security::protect` in `crates/id-web/src/security.rs`. Four controls apply to every route, in this order:

1. **Bind.** Loopback by default: the CLI `--bind` default is `127.0.0.1`.
2. **Host allow-list.** On loopback binds, the request `Host` must be `localhost`, `127.0.0.1` or `::1`, plus any operator-configured extras. Other hosts are refused.
3. **Origin check.** Any request carrying an `Origin` header must be same-origin: scheme, host and port must match, with no default-port expansion. The scheme counts as `https` only when the operator declared a TLS-terminating proxy via `--web-cookie-secure`.
4. **Token gate.** When a token is configured (`--web-token`, or the environment variable `ID_WEB_TOKEN`), it is accepted in three forms: the cookie `id_token`, an `Authorization: Bearer` header, or a `?token=` query parameter. The query form sets the cookie and then redirects to the clean path with the parameter removed. When no token is configured, this gate is open; `id serve` prints a warning at startup when a web listener is bound without `--web-token`.

Older documentation presents the web UI as an unauthenticated local tool and never mentions this layer. That is disagreement WEB-15: the Host allow-list, the Origin check and the token gate apply to every route in the code, and this chapter documents the code.

Body limits: a router-wide default body limit applies, and `/api/upload` overrides it with a larger route-scoped limit sized so that a file of exactly the maximum image size plus its multipart envelope fits.

## Pages

Full HTML pages in the main router:

| Method | Path                                      | Purpose                                                                                                                       |
| ------ | ----------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------- |
| GET    | `/`                                       | Index / home: world list, globally-tagged documents, and a global-chat iframe.                                                |
| GET    | `/.well-known/id-transport-advertisement` | The transport advertisement document.                                                                                         |
| GET    | `/files`                                  | File browser page.                                                                                                            |
| GET    | `/world`                                  | World page: the adapter surface for the hosted world. The path is the default; `AppState.web_route` parameterises it per app. |
| GET    | `/settings`                               | Settings page.                                                                                                                |
| GET    | `/peers`                                  | Discovered peers page.                                                                                                        |
| GET    | `/blocks`                                 | Operator block-list page.                                                                                                     |
| GET    | `/groups`                                 | Groups page.                                                                                                                  |
| GET    | `/edit/{*name}`                           | Editor/viewer for a file **by name** (wildcard).                                                                              |
| GET    | `/hash/{hash}`                            | Redirect from a content hash to its named file.                                                                               |
| GET    | `/view/{*name}`                           | Read-only view of a file by name.                                                                                             |
| GET    | `/blob/{hash}`                            | Raw blob bytes with a `Content-Type`.                                                                                         |

### Correct route spellings (WEB-1)

The editor and view routes take a **name**, not a hash, and the name is a wildcard segment:

- Edit: `/edit/{*name}`
- View: `/view/{*name}`
- Hash entry point: `/hash/{hash}`

`WEB.md` documents `GET /edit/{hash}` and `GET /file/{name}`. Neither route exists in the code. That is disagreement WEB-1; the spellings above are the routes `routes.rs` registers.

## HTTP API and partials

| Method            | Path                     | Purpose                                                                                                                                                                                         |
| ----------------- | ------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| GET               | `/api/files`             | File list, as an HTML partial for SPA navigation.                                                                                                                                               |
| GET               | `/api/home/feed`         | Home feed partial.                                                                                                                                                                              |
| GET               | `/api/peers`             | Peers partial.                                                                                                                                                                                  |
| GET, POST         | `/api/blocks`            | List blocks / create a block.                                                                                                                                                                   |
| POST              | `/api/blocks/remove`     | Remove a block.                                                                                                                                                                                 |
| POST              | `/api/save`              | Save editor content to the blob store.                                                                                                                                                          |
| POST              | `/api/new`               | Create a new named file.                                                                                                                                                                        |
| POST              | `/api/rename`            | Rename, with optional archive of the original or replaced file.                                                                                                                                 |
| POST              | `/api/copy`              | Copy a name to a new name.                                                                                                                                                                      |
| POST              | `/api/files/send`        | Server-side send of file references to another server.                                                                                                                                          |
| POST              | `/api/download`          | Download file content.                                                                                                                                                                          |
| POST              | `/api/delete`            | Soft-delete a file.                                                                                                                                                                             |
| POST              | `/api/restore`           | Restore a soft-deleted file. Returns `404` when the file is missing and `409` when it is not deleted.                                                                                           |
| POST              | `/api/hard-delete`       | Permanently delete a file.                                                                                                                                                                      |
| POST              | `/api/upload`            | Image upload, multipart, with the route-scoped body limit.                                                                                                                                      |
| GET               | `/api/images`            | Image listing, paged, with a cursor clamped to the result set.                                                                                                                                  |
| GET, POST, DELETE | `/api/tags`              | Get, set or delete tags. DELETE modes: exact match, `all: true`, and `all_values: true`. Binary values travel as `value_b64`. The tag wire protocol is in [WebSocket protocols](websockets.md). |
| GET               | `/api/tags/search`       | Structured tag search: `q` and `ns` parameters; `key:`, `:value`, `key:value`, quoted and bare terms, ANDed.                                                                                    |
| POST              | `/api/identity/register` | Register a web client identity. Bounded by `ID_WEB_MAX_IDENTITIES`; see [Configuration and environment](configuration.md).                                                                      |
| GET               | `/api/identity/me`       | The current identity.                                                                                                                                                                           |
| POST              | `/api/identity/name`     | Update the identity display name.                                                                                                                                                               |
| POST              | `/api/seitan/invite`     | Seitan admission: create an invite.                                                                                                                                                             |
| POST              | `/api/seitan/admit`      | Seitan admission: admit with an invite.                                                                                                                                                         |
| GET               | `/assets/{*path}`        | Embedded static assets; see "Assets" below.                                                                                                                                                     |

The delete pair is a web-layer construction: `/api/delete` and `/api/restore` implement soft delete on top of the FileOps hard delete described in [Data stores and schemas](data-stores.md).

## World bridge HTTP routes

These 9 paths are merged into the main router always. The WebSocket socket path comes from `[interfaces.web]` in the app's `app.toml` and defaults to `/ws/world`; see [Configuration and environment](configuration.md).

| Method    | Path                         | Purpose                                                                                                                         |
| --------- | ---------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| POST      | `/api/world/invite`          | Mint a world guest invite. Fields: `display_name`, optional `world`, `expires_in_secs`, `uses`.                                 |
| GET       | `/api/world/host-view`       | Host view: snapshot, surface, and paged records. Paging uses an `after` cursor; the records revision is an FNV-1a 53-bit value. |
| GET       | `/api/world/metrics`         | World host metrics.                                                                                                             |
| GET, POST | `/api/world/directory`       | Directory read / signed directory write action.                                                                                 |
| GET       | `/explore`                   | Script-free HTML directory explorer: forms only, strict CSP `default-src 'none'`.                                               |
| POST      | `/explore/login`             | Explorer login: an admin token is exchanged for the `id_explore` session cookie, scoped to path `/explore`.                     |
| POST      | `/explore/act`               | Explorer directory action.                                                                                                      |
| GET       | `/invite`                    | Invite landing page.                                                                                                            |
| WS        | `/ws/world` (default socket) | World session bridge; the frame protocol is in [WebSocket protocols](websockets.md).                                            |

## Program gallery routes

These 7 paths are merged into the main router always. Every one of them answers `404` unless a gallery was wired in with `--gallery-dir` or `ID_GALLERY_DIR`. The operator commands behind them are in [Gallery, match-audit and walk](cli-gallery-walk.md).

| Method | Path                                         | Purpose                                                                                                         |
| ------ | -------------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| GET    | `/api/gallery/records`                       | Signed bundle records list. Filters: `?author=` and `?name=`.                                                   |
| GET    | `/api/gallery/bundles/{content_hash}`        | Bundle bytes, immutable cache. Answers `410 Gone`, with a transparency entry, when the bundle is unlisted here. |
| GET    | `/api/gallery/search`                        | Search with `?q=`, `?category=`, `?limit=`. Result order: exact-name, then category, then most-recent-version.  |
| GET    | `/api/gallery/authors/{author}`              | Author page by key.                                                                                             |
| GET    | `/api/gallery/collections`                   | Operator-signed featured collections.                                                                           |
| GET    | `/api/gallery/collections/{operator}/{name}` | One collection.                                                                                                 |
| GET    | `/api/gallery/transparency`                  | Removals, refusals and reinstatements, plus the admission posture. Paging: `?after=` and `?limit=`.             |

## WebSocket routes

| Path                  | Protocol                                                |
| --------------------- | ------------------------------------------------------- |
| `/ws/collab/{doc_id}` | Collaborative editing, binary MessagePack.              |
| `/ws/tags`            | Tag-change broadcast, JSON text, server to client only. |

Both protocols are specified in full in [WebSocket protocols](websockets.md).

## The program-only router

`create_program_router` serves one listener pinned to one world. It has 10 paths:

- `GET /` — the program page.
- `GET /assets/{*path}` — embedded static assets.
- The full world bridge surface above (8 HTTP paths plus the world socket), bound at that world's socket.

A publication mount serves exactly this router under a mounted prefix.

## Content modes and themes

A file's content mode is decided from its extension, and for text modes from its content:

| Mode                              | Selected for                                                                                  | Wire name  |
| --------------------------------- | --------------------------------------------------------------------------------------------- | ---------- |
| `Media(Image\|Video\|Audio\|Pdf)` | Media and PDF extensions                                                                      | `media`    |
| `Rich`                            | `.pm.json` ProseMirror JSON                                                                   | `rich`     |
| `Markdown`                        | `.md`, `.markdown`                                                                            | `markdown` |
| `Plain`                           | `.txt`                                                                                        | `plain`    |
| `Raw`                             | A long list of code and config extensions; also the default for unknown or missing extensions | `raw`      |
| `Binary`                          | Only when a text-mode file is not valid UTF-8                                                 | `binary`   |

Editable modes: Rich, Markdown, Plain, Raw. Toolbar modes: Rich, Markdown, Plain. `get_content_type` maps extensions to MIME types and defaults to `application/octet-stream`.

The frontend ships exactly three DaisyUI themes, all on a `#000000` background: `sneak` (default, blue `#00aaff`), `arch` (green `#00ff00`) and `mech` (orange `#ff6600`). The choice is stored in the `localStorage` key `id-theme`, applied as the `data-theme` attribute, cycled with Alt+T, and announced with a `theme:change` CustomEvent.

## Assets: how they are built and served

Build, from `web/package.json`:

```text
bun build src/main.ts --outdir=dist --minify --sourcemap=external --entry-naming=[name].[hash].js
```

CSS is built with the Tailwind v4 CLI from `src/input.css` to `dist/styles.css`. A manifest script then renames the stylesheet to its hashed name and writes the manifest the templates consume (`AssetUrls`).

Serving: `web/dist` is embedded into the binary at compile time with `rust-embed` (`#[folder = "../../web/dist"]`, struct `Assets`) and served at `/assets/{*path}`. The checkout's `web/dist` contains only a tracked `.gitkeep`; the assets are a build product, not source.

### Asset naming and cache contract (WEB-4)

The cache contract is derived from the emitted file name:

| File                                   | Cache header                          |
| -------------------------------------- | ------------------------------------- |
| `main.<8 base36>.js` and its `.js.map` | `public, max-age=31536000, immutable` |
| `styles.<8 lower-hex>.css`             | `public, max-age=31536000, immutable` |
| Everything else                        | `no-cache`                            |

`WEB.md` names the assets `main.css` and `main.js`. The build never emits either name. That is disagreement WEB-4; the hashed names above are what `assets.rs` recognises.

## Worked example

Toy data: a node serving on the default port, with a web token set, and a file named `shopping-list.txt`.

Start the server (flags are specified in [serve and publications](cli-serve.md)):

```text
$ id serve --web --port 3000 --web-token TOKEN
```

First visit with the token in the query string; the server sets the `id_token` cookie and redirects to the clean path:

```text
GET /?token=TOKEN        → 302 → GET /
GET /                    → 200  (index page, cookie now set)
```

Read and edit the file by name:

```text
GET  /view/shopping-list.txt   → 200  (read-only view)
GET  /edit/shopping-list.txt   → 200  (editor)
POST /api/save                 → 200  (editor content written to the blob store)
GET  /hash/0123abcd…           → 302  (redirect to the named file for that hash)
GET  /blob/0123abcd…           → 200  (raw bytes, Content-Type set)
```

A request from a disallowed host, or a cross-origin POST, is refused by the security layer before any handler runs — that refusal is the Host allow-list and Origin check above, not a per-route check.
