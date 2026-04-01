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

- **File Browser**: HTMX-powered listing with search, filtering, bulk operations
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
│  │   Axum      │    │   HTMX      │    │ ProseMirror │      │
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
│  - CSS: main.css (DaisyUI + Tailwind v4)                     │
│  - JS: main.js (bundled with Bun)                            │
└─────────────────────────────────────────────────────────────┘
```

## Development

### Prerequisites

The Nix dev shell provides all required tools:

- Rust 1.89.0
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
│   └── wrap.ts           # Word wrap toggle (Alt+Z)
└── dist/                 # Built assets (embedded in binary)

src/web/
├── mod.rs                # Module exports, AppState
├── routes.rs             # Axum route handlers (file CRUD, tags, rename, copy)
├── collab.rs             # WebSocket collaboration server
├── tags_ws.rs            # WebSocket tag change broadcast
├── templates.rs          # HTML template rendering
├── assets.rs             # rust-embed static serving
├── content_mode.rs       # Content type detection
└── markdown.rs           # Markdown rendering with syntax highlighting

e2e/
├── playwright.config.ts        # 3-mode config: local / nix sandbox / VM test
└── tests/
    ├── basic.spec.ts           # 19 UI fundamental tests
    ├── editor-features.spec.ts # 31 editor feature tests (syntax, lines, wrap, search, etc.)
    ├── file-operations.spec.ts # 18 tag CRUD, rename, copy, delete, search, theme tests
    ├── navigation.spec.ts      # 17 settings, peers, SPA nav, browser history tests
    └── websocket.spec.ts       # 19 WS + collaboration tests
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
{"type": "Set", "ns": "global", "subject": "readme.md", "key": "author", "value": "Jane"}
{"type": "Del", "ns": "global", "subject": "readme.md", "key": "author"}
{"type": "DelAll", "ns": "global", "subject": "readme.md"}
{"type": "Transfer", "ns": "global", "from_subject": "old.md", "to_subject": "new.md"}
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

| Route                 | Method | Description                         |
| --------------------- | ------ | ----------------------------------- |
| `/`                   | GET    | File browser (main page)            |
| `/peers`              | GET    | Discovered peers (auto-refreshes)   |
| `/settings`           | GET    | Settings page                       |
| `/edit/{hash}`        | GET    | Editor/viewer for file by hash      |
| `/file/{name}`        | GET    | Editor/viewer for file by name      |
| `/blob/{hash}`        | GET    | Raw blob content with Content-Type  |
| `/api/files`          | GET    | File list (HTMX partial)            |
| `/api/save`           | POST   | Save file content                   |
| `/api/new`            | POST   | Create new file                     |
| `/api/rename`         | POST   | Rename file (with optional archive) |
| `/api/copy`           | POST   | Copy file to new name               |
| `/api/download`       | POST   | Download file content               |
| `/api/delete`         | POST   | Soft-delete file                    |
| `/api/restore`        | POST   | Restore deleted file                |
| `/api/hard-delete`    | POST   | Permanently delete file             |
| `/api/tags`           | GET    | Get tags for a file                 |
| `/api/tags`           | POST   | Set a tag                           |
| `/api/tags`           | DELETE | Delete a tag                        |
| `/api/tags/search`    | GET    | Search tags                         |
| `/ws/collab/{doc_id}` | WS     | Collaboration WebSocket             |
| `/ws/tags`            | WS     | Tag change broadcast                |
| `/assets/*`           | GET    | Static assets                       |

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

Playwright tests run against both Chromium and Firefox (104 tests × 2 browsers = 208 total):

```bash
just test-e2e          # Both browsers (208 tests)
just test-e2e-chromium # Chromium only (104 tests)
just test-e2e-firefox  # Firefox only (104 tests)
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
