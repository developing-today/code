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
cargo run --features web -- serve --web 3000
```

Open http://localhost:3000 in your browser.

## Features

- **File Browser**: HTMX-powered listing with search, filtering, bulk operations
- **Collaborative Editor**: Real-time text editing using ProseMirror + prosemirror-collab
- **Metadata Tags**: Attach, view, search key-value tags on files via inline UI and WebSocket
- **File Management**: Rename, copy, delete, restore with archive tracking
- **Content Viewers**: Smart rendering for text, markdown, images, video, audio, PDF, and binary files
- **Themes**: Switchable terminal themes (sneak, arch, mech)
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
│  - CSS: terminal.css, themes.css, editor.css                 │
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
│   ├── main.ts           # Entry point, HTMX init, file operations
│   ├── editor.ts         # ProseMirror editor setup
│   ├── collab.ts         # WebSocket collaboration client
│   ├── cursors.ts        # Cursor/selection plugin with fading
│   └── theme.ts          # Theme switching
├── styles/
│   ├── terminal.css      # Base terminal styles, file list
│   ├── themes.css        # sneak/arch/mech theme variables
│   └── editor.css        # ProseMirror styles, viewer buttons
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
├── playwright.config.ts  # Chromium + Firefox config
└── tests/
    └── basic.spec.ts     # 15 E2E tests
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

Three terminal-inspired themes with `#000000` black backgrounds. Switch via the footer toggle or keyboard shortcut `Ctrl+T`.

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
# Start with web UI on port 3000
id serve --web 3000

# Ephemeral mode (no persistence between restarts)
id serve --web 3000 --ephemeral
```

## Testing

### Unit Tests

Rust unit tests cover routes, templates, and content mode logic:

```bash
just test-unit
```

### E2E Tests

Playwright tests run against both Chromium and Firefox:

```bash
just test-e2e          # Both browsers
just test-e2e-chromium # Chromium only
just test-e2e-firefox  # Firefox only
```

Tests cover: home page elements, file creation, editor features, navigation, themes.

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
