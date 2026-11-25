# Web Interface for `id`

A browser-based UI for the `id` P2P file sharing CLI, featuring collaborative text editing, file management, and metadata tags with a terminal-inspired aesthetic.

## Quick Start

```bash
# Enter the Nix dev shell (includes Bun)
nix develop

# Build web assets
just web-build

# Build and run with web interface on port 3000
just serve
# or explicitly:
cargo run --features web -- serve --web
```

Open http://localhost:3000 in your browser (default port).

## Features

- **File Browser**: Server-rendered listing with client-side navigation (Datastar SDK + TypeScript), search, filtering, bulk operations
- **Collaborative Editor**: Real-time text editing using ProseMirror + prosemirror-collab
- **Syntax Highlighting**: Language-aware code coloring via Shiki (50+ languages)
- **Line Numbers**: Toggleable line numbers for code files (Alt+L)
- **Word Wrap**: Toggleable word wrap for long lines (Alt+Z)
- **Find & Replace**: In-editor search with regex, match navigation (Ctrl+F / Ctrl+H)
- **Go-to-Line**: Jump to specific line numbers (Ctrl+G)
- **Active Line Highlight**: Visual indicator of current cursor line
- **Tab Indentation**: Smart tab/shift-tab with 2-space indent
- **Metadata Tags**: Attach, view, search key-value tags on files via inline UI and WebSocket
- **File Management**: Rename, copy, delete, restore with archive tracking
- **Content Viewers**: Smart rendering for text, markdown, images, video, audio, PDF, and binary files
- **Themes**: Switchable DaisyUI terminal themes (sneak, arch, mech)
- **Single Binary**: All JS/CSS embedded via rust-embed

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      Web Interface                          │
├─────────────────────────────────────────────────────────────┤
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐      │
│  │   Axum      │    │  Datastar   │    │ ProseMirror │      │
│  │   Router    │───►│   Views     │───►│   Editor    │      │
│  └─────────────┘    └─────────────┘    └─────────────┘      │
│         │                  │                  │             │
│         │           ┌──────┴──────┐    ┌─────┴─────┐        │
│         │           │             │    │           │        │
│         │     ┌─────▼─────┐ ┌─────▼────▼┐                   │
│         │     │   HTML    │ │ WebSocket │                   │
│         │     │ Templates │ │  Collab   │                   │
│         │     └───────────┘ └───────────┘                   │
│         ▼                                                    │
│  Embedded Assets (rust-embed)                                │
│  - CSS: styles.<hash>.css (DaisyUI + Tailwind v4)            │
│  - JS: main.<hash>.js (bundled with Bun)                      │
└─────────────────────────────────────────────────────────────┘
```

## Development

### Prerequisites

The Nix dev shell provides all required tools:

- Rust 1.99.1
- Bun (for TypeScript bundling)
- TypeScript
- Chromium and Firefox (for E2E tests)

### Building Assets

```bash
# Build for production (minified)
just web-build

# Watch mode for development
just web-dev
```

### Project Structure

```
web/
├── package.json          # Bun/npm dependencies
├── bun.nix               # Offline npm deps for nix sandbox
├── tsconfig.json         # TypeScript config
├── src/
│   ├── main.ts           # Entry point, SPA navigation, file operations
│   ├── input.css         # TailwindCSS v4 + DaisyUI entry (themes, CRT effects, all styles)
│   ├── editor.ts         # ProseMirror editor setup, schema, menu
│   ├── collab.ts         # WebSocket collaboration client, MessagePack protocol
│   ├── cursors.ts        # Cursor/selection plugin with opacity fading
│   ├── theme.ts          # Theme switching (sneak/arch/mech)
│   ├── search-panel.ts   # Find & replace panel (Ctrl+F / Ctrl+H)
│   ├── highlight.ts      # Syntax highlighting via Shiki
│   ├── cursor-utils.ts   # Cursor opacity, color, tooltip clustering
│   ├── goto-line.ts      # Go-to-line dialog (Ctrl+G)
│   ├── indent.ts         # Tab indentation (2-space)
│   ├── active-line.ts    # Active line highlight plugin
│   ├── wrap.ts           # Word wrap toggle (Alt+Z)
│   ├── home.ts           # Home page (world list, tagged documents, global chat)
│   ├── navigation.ts     # SPA navigation
│   ├── world.ts          # World surface (bridge client, forms, records)
│   ├── world-*.ts        # World helpers (http, keyboard, seed, surface)
│   └── *.test.ts         # Unit tests alongside the modules they cover
└── dist/                 # Built assets (embedded in binary; contents git-ignored,
                          # the directory itself is tracked with a .gitkeep)

crates/id-web/src/
├── lib.rs                # Router construction (Explorer, per-app, program-only)
├── routes.rs, routes/    # Axum route handlers (file CRUD, tags, rename, copy)
├── security.rs           # Host allow-list, Origin check, token gate (every route)
├── collab.rs             # WebSocket collaboration server
├── tags_ws.rs            # WebSocket tag change broadcast
├── world_ws.rs           # World session bridge WebSocket
├── templates.rs          # HTML template rendering
├── assets.rs             # rust-embed static serving
├── content_mode.rs       # Content type detection
├── markdown.rs, markdown/ # Markdown rendering with syntax highlighting
├── home.rs               # Home page and feed
├── explore.rs            # Script-free directory explorer (/explore)
├── gallery.rs            # Program gallery routes
├── runs.rs               # POST /v1/runs: argv parity, SSE output, app loader
├── sessions.rs           # /v1/sessions: event log, input frames, replay, socket
├── groups.rs             # Groups page
├── identity.rs           # Web client identity routes
├── seitan.rs             # Seitan admission (invite / admit)
├── transport_ad.rs       # Transport advertisement document
└── app_interface.rs      # Per-app interface surface

e2e/
├── playwright.config.ts        # 3-mode config: local / nix sandbox / VM test
└── tests/
    ├── autosave.spec.ts        # 7 autosave tests
    ├── basic.spec.ts           # 20 UI fundamental tests
    ├── editor-features.spec.ts # 47 editor feature tests (syntax, lines, wrap, search, etc.)
    ├── file-operations.spec.ts # 20 tag CRUD, rename, copy, delete, search, theme tests
    ├── navigation.spec.ts      # 17 settings, peers, SPA nav, browser history tests
    ├── websocket.spec.ts       # 22 WS + collaboration tests
    └── world-surface.spec.ts   # 5 world surface tests
```

### Justfile Commands

```bash
just web-build           # Build web assets with Bun
just web-dev             # Watch mode for development
just build               # Build Rust with embedded web UI
just serve               # Build and run with web on port 3000
just test-e2e            # Run Playwright tests (chromium + firefox)
just test-e2e-chromium   # Run Playwright tests (chromium only)
just test-e2e-firefox    # Run Playwright tests (firefox only)
just test-e2e-report     # Show Playwright HTML report
```

## Security Layer

Every router is wrapped by the security layer (`crates/id-web/src/security.rs`),
which applies to every route below, in this order:

1. **Bind**: loopback by default (`--bind` defaults to `127.0.0.1`).
2. **Host allow-list**: on loopback binds, the request `Host` must be
   `localhost`, `127.0.0.1` or `::1`, plus any operator-configured extras.
3. **Origin check**: any request carrying an `Origin` header must be
   same-origin (scheme, host and port must match).
4. **Token gate**: when a token is configured (`--web-token` or the
   `ID_WEB_TOKEN` environment variable), it is accepted as the `id_token`
   cookie, an `Authorization: Bearer` header, or a `?token=` query parameter
   (the query form sets the cookie and redirects to the clean path). When no
   token is configured this gate is open, and `id serve` prints a warning at
   startup when a web listener is bound without `--web-token`.

## Pages

### File List (Home)

The home page displays all stored files with:

- **New file form**: Create files with a name input
- **Search/filter**: Real-time filtering by name
- **Visibility toggles**: Show/hide auto-generated and archived files, show deleted files
- **Bulk operations**: Select multiple files for bulk tag operations
- **File items**: Name, classification badge, tag pills, date, short hash

### Editor

The editor page renders text/markdown files with ProseMirror:

- **Collaborative editing**: Real-time multi-user editing via WebSocket
- **Syntax highlighting**: Language-aware coloring for 50+ languages via Shiki
- **Line numbers**: Toggleable gutter line numbers (Alt+L)
- **Word wrap**: Toggleable soft wrap for long lines (Alt+Z)
- **Find & replace**: Search panel with match highlighting, navigation (Ctrl+F), replace (Ctrl+H)
- **Go-to-line**: Jump to line number dialog (Ctrl+G)
- **Active line**: Visual highlight of the current cursor line
- **Tab indentation**: Tab inserts 2 spaces, Shift+Tab dedents
- **Save**: Manual save (Ctrl+S) writes content back to blob store
- **Download**: Raw text, ProseMirror JSON, or original format
- **Rename/Copy**: Inline rename and copy with archive support
- **Tag panel**: View, add, and remove metadata tags inline

### Media Viewer

Images, video, audio, and PDF files render with native browser elements:

- **Inline display**: `<img>`, `<video>`, `<audio>`, or `<iframe>` embed
- **Download**: Direct download link
- **Rename/Copy**: Same rename and copy buttons as editor

### Binary Viewer

Unknown binary files show a download-only view:

- **Download link**: Direct download
- **Rename/Copy**: Same rename and copy buttons as editor

## Collaboration Protocol

The editor uses prosemirror-collab for real-time collaboration via WebSocket:

1. **Connect**: Client connects to `/ws/collab/{doc_id}`
2. **Init**: Server sends current document state and version
3. **Steps**: Client sends local changes as ProseMirror steps
4. **Broadcast**: Server validates, applies, and broadcasts to other clients
5. **Ack**: Server acknowledges applied steps with new version

### Wire Protocol

Messages are binary MessagePack arrays. See [web/README.md](web/README.md) for the complete wire protocol specification including message types, cursor position sharing, and timeout behavior.

## Metadata Tags

Tags are key-value pairs attached to files. The web UI provides:

- **Tag panel** (editor page): Inline display of current tags with add/remove
- **Tag pills** (file list): Visual tag indicators on each file
- **WebSocket updates**: Tag changes broadcast to all connected clients via `/ws/tags`
- **Bulk tagging**: Select multiple files and add tags in bulk

### Tag WebSocket

The `/ws/tags` endpoint broadcasts `TagEvent` messages (JSON) when tags change:

```json
{"type": "set", "ns": "global", "subject": "readme.md", "key": "author", "value": "Jane"}
{"type": "del", "ns": "global", "subject": "readme.md", "key": "author"}
{"type": "del_all", "ns": "global", "subject": "readme.md"}
{"type": "transfer", "ns": "global", "from": "old.md", "to": "new.md", "count": 3}
```

## Themes

Three DaisyUI terminal-inspired themes with `#000000` black backgrounds. Switch via the footer toggle or keyboard shortcut `Alt+T`.

### sneak (default, blue)

- Accent: `#00aaff`
- Monospace font, scanline effect

### arch (green)

- Accent: `#00ff00`
- Text glow effects

### mech (orange)

- Accent: `#ff6600`
- Angular UI elements

## API Routes

The main router registers 54 paths with 58 method bindings; a second,
program-only router (one listener pinned to one world) registers 10 more.

### Pages

| Route                                     | Method | Description                                             |
| ----------------------------------------- | ------ | ------------------------------------------------------- |
| `/`                                       | GET    | Index / home: world list, tagged documents, global chat |
| `/.well-known/id-transport-advertisement` | GET    | Transport advertisement document                        |
| `/files`                                  | GET    | File browser page                                       |
| `/world`                                  | GET    | World page (the hosted world's adapter surface)         |
| `/settings`                               | GET    | Settings page                                           |
| `/peers`                                  | GET    | Discovered peers (auto-refreshes)                       |
| `/blocks`                                 | GET    | Operator block-list page                                |
| `/groups`                                 | GET    | Groups page                                             |
| `/edit/{*name}`                           | GET    | Editor/viewer for a file by name (wildcard)             |
| `/hash/{hash}`                            | GET    | Redirect from a content hash to its named file          |
| `/view/{*name}`                           | GET    | Read-only view of a file by name                        |
| `/blob/{hash}`                            | GET    | Raw blob content with Content-Type                      |

### HTTP API and partials

| Route                    | Method    | Description                                               |
| ------------------------ | --------- | --------------------------------------------------------- |
| `/api/files`             | GET       | File list (HTML partial)                                  |
| `/api/home/feed`         | GET       | Home feed partial                                         |
| `/api/peers`             | GET       | Peers partial                                             |
| `/api/blocks`            | GET, POST | List blocks / create a block                              |
| `/api/blocks/remove`     | POST      | Remove a block                                            |
| `/api/save`              | POST      | Save file content                                         |
| `/api/new`               | POST      | Create new file                                           |
| `/api/rename`            | POST      | Rename file (with optional archive)                       |
| `/api/copy`              | POST      | Copy file to new name                                     |
| `/api/files/send`        | POST      | Send file to another server (server-side transfer, #5346) |
| `/api/download`          | POST      | Download file content                                     |
| `/api/delete`            | POST      | Soft-delete file                                          |
| `/api/restore`           | POST      | Restore deleted file (404 if missing, 409 if not deleted) |
| `/api/hard-delete`       | POST      | Permanently delete file                                   |
| `/api/upload`            | POST      | Image upload (multipart, route-scoped body limit)         |
| `/api/images`            | GET       | Image listing, paged                                      |
| `/api/tags`              | GET       | Get tags for a file                                       |
| `/api/tags`              | POST      | Set a tag                                                 |
| `/api/tags`              | DELETE    | Delete a tag                                              |
| `/api/tags/search`       | GET       | Search tags                                               |
| `/api/identity/register` | POST      | Register a web client identity                            |
| `/api/identity/me`       | GET       | The current identity                                      |
| `/api/identity/name`     | POST      | Update the identity display name                          |
| `/api/seitan/invite`     | POST      | Seitan admission: create an invite                        |
| `/api/seitan/admit`      | POST      | Seitan admission: admit with an invite                    |
| `/assets/{*path}`        | GET       | Static assets                                             |

### World bridge

| Route                  | Method    | Description                                              |
| ---------------------- | --------- | -------------------------------------------------------- |
| `/api/world/invite`    | POST      | Mint a world guest invite                                |
| `/api/world/host-view` | GET       | Host view: snapshot, surface, and paged records          |
| `/api/world/metrics`   | GET       | World host metrics                                       |
| `/api/world/directory` | GET, POST | Directory read / signed directory write action           |
| `/explore`             | GET       | Script-free HTML directory explorer (forms only)         |
| `/explore/login`       | POST      | Explorer login (admin token for the `id_explore` cookie) |
| `/explore/act`         | POST      | Explorer directory action                                |
| `/invite`              | GET       | Invite landing page                                      |
| `/ws/world`            | WS        | World session bridge (default socket path)               |

### Program gallery

Every gallery route answers 404 unless a gallery was wired in with
`--gallery-dir` or `ID_GALLERY_DIR`.

| Route                                        | Method | Description                           |
| -------------------------------------------- | ------ | ------------------------------------- |
| `/api/gallery/records`                       | GET    | Signed bundle records list            |
| `/api/gallery/bundles/{content_hash}`        | GET    | Bundle bytes, immutable cache         |
| `/api/gallery/search`                        | GET    | Search bundles                        |
| `/api/gallery/authors/{author}`              | GET    | Author page by key                    |
| `/api/gallery/collections`                   | GET    | Operator-signed featured collections  |
| `/api/gallery/collections/{operator}/{name}` | GET    | One collection                        |
| `/api/gallery/transparency`                  | GET    | Removals, refusals and reinstatements |

### Runs

`POST /v1/runs` runs one subcommand of the serving binary with the request's
argv, allowlisted env and stdin (`none`, or `body` with `data` or
`data_base64`), and answers with its exit code, stdout and stderr: the same
result as the CLI with that argv (#10392 §6.1). Loopback clients only; a
request forwarded by a proxy is refused.

With `Accept: text/event-stream` the run answers as server-sent events while
it runs (§6.2): `id: <seq>` from 1, `event: app | out | err | exit`, and a
JSON `data:` object. `out` and `err` carry `{encoding, data}` chunks; `exit`
comes last with the exit code, signal, timeout and truncation flags. The
events of a run with `stdin.mode = none` add up to the plain JSON answer.

App runs name an `app` from the served `app.toml` manifests, announce a
`client` (`kind`, `features`) and list the `capabilities` they grant (§6.4).
A run missing a required capability is refused with 403 naming it, and never
starts.

Sessions (§6.3) hold a run's numbered event log. `POST /v1/sessions` answers
`{id}` at once; a run joins one by naming `session`, and a streamed run with
input gets one (its first event is `app` with `{session}`). With
`stdin.mode = session`, input arrives as frames `{seq, bytes, close}` (`bytes`
base64, `seq` from 1): in order, a gap or a conflicting repeat refused with
`kind` `gap` or `conflict`, an identical repeat answered `repeated`. Readers
attach with `since`, `after`, `before` + `after` or `all=1`; when retention
(16 MiB per session, one hour idle) dropped events below the cursor, an `app`
event `{truncated, first_retained_seq}` comes first. The socket carries the
same event and input objects.

| Route                      | Method | Description                                  |
| -------------------------- | ------ | -------------------------------------------- |
| `/v1/runs`                 | POST   | Run one subcommand; JSON when done, or SSE   |
| `/v1/apps/{app}/manifest`  | GET    | An app's run manifest (capabilities, assets) |
| `/v1/sessions`             | POST   | Create a session; answers `{id}`             |
| `/v1/sessions/{id}/input`  | POST   | One numbered input frame                     |
| `/v1/sessions/{id}/events` | GET    | Attach or replay as SSE                      |
| `/v1/sessions/{id}/socket` | GET    | Attach over a WebSocket (events and input)   |

### WebSocket

| Route                 | Method | Description             |
| --------------------- | ------ | ----------------------- |
| `/ws/collab/{doc_id}` | WS     | Collaboration WebSocket |
| `/ws/tags`            | WS     | Tag change broadcast    |

## Configuration

```bash
# Start with web UI (default port 3000)
id serve --web

# Custom port
id serve --web --port 8080

# Ephemeral mode (no persistence between restarts)
id serve --web --ephemeral

# Custom data directory
id serve --web --data-dir /path/to/data

# Fresh node (new identity, ignores existing data)
id serve --web --new
```

## Testing

### Unit Tests

Rust unit tests cover routes, templates, and content mode logic:

```bash
just test-unit
```

### E2E Tests

Playwright tests run against both Chromium and Firefox (138 tests × 2 browsers = 276 total):

```bash
just test-e2e          # Both browsers (276 tests)
just test-e2e-chromium # Chromium only (138 tests)
just test-e2e-firefox  # Firefox only (138 tests)
```

Tests cover: home page elements, file creation, editor features (syntax highlighting, line numbers, word wrap, find/replace, go-to-line, active line, tab indentation), navigation, themes, WebSocket connection/disconnect/reconnect, collaborative editing, multi-user scenarios, tag CRUD, file rename/copy/delete, search filtering.

Browser paths are configured via environment variables for nix compatibility:

- `PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH`
- `PLAYWRIGHT_FIREFOX_EXECUTABLE_PATH`

## Troubleshooting

### Assets not found (404)

Ensure web assets are built before compiling Rust:

```bash
just web-build && cargo build --features web
```

### WebSocket connection failed

Check that the server is running and the port is accessible. WebSocket endpoints:

- Collaboration: `ws://localhost:PORT/ws/collab/{doc_id}`
- Tags: `ws://localhost:PORT/ws/tags`

### Theme not applying

Clear browser cache or hard refresh (Ctrl+Shift+R). Theme preference is stored in localStorage.

### E2E tests fail with browser not found

Ensure you're in the nix dev shell (`nix develop`), which sets browser paths automatically. Outside nix, install Playwright browsers:

```bash
cd e2e && bunx playwright install chromium firefox
```
