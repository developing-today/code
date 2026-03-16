# Web Interface for `id`

A browser-based UI for the `id` P2P file sharing CLI, featuring collaborative text editing with a "computery/hacker" aesthetic.

## Quick Start

```bash
# Enter the Nix dev shell (includes Bun)
nix develop

# Build web assets
just web-build

# Build and run with web interface on port 3000
cargo run --features web -- serve --web 3000
```

Open http://localhost:3000 in your browser.

## Features

- **File Browser**: HTMX-powered listing of stored files
- **Collaborative Editor**: Real-time text editing using ProseMirror + prosemirror-collab
- **Themes**: Switchable terminal themes:
  - **Terminal** (default): Classic green-on-black
  - **Matrix**: Bright green with glow effects
  - **Evangelion**: Orange/purple NERV-inspired
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
├── tsconfig.json         # TypeScript config
├── src/
│   ├── main.ts           # Entry point, HTMX init
│   ├── editor.ts         # ProseMirror editor setup
│   ├── collab.ts         # WebSocket collaboration client
│   └── theme.ts          # Theme switching
├── styles/
│   ├── terminal.css      # Base terminal styles
│   ├── themes.css        # Matrix/Evangelion themes
│   └── editor.css        # ProseMirror styles
└── dist/                 # Built assets (embedded in binary)

src/web/
├── mod.rs                # Module exports, AppState
├── routes.rs             # Axum route handlers
├── collab.rs             # WebSocket collaboration server
├── templates.rs          # HTML template rendering
└── assets.rs             # rust-embed static serving
```

### Justfile Commands

```bash
just web-build     # Build web assets with Bun
just web-dev       # Watch mode for development
just build-web     # Build Rust with web feature
just serve-web     # Build and run with web on port 3000
```

## Collaboration Protocol

The editor uses prosemirror-collab for real-time collaboration via WebSocket:

1. **Connect**: Client connects to `/ws/collab/{doc_id}`
2. **Init**: Server sends current document state and version
3. **Steps**: Client sends local changes as ProseMirror steps
4. **Broadcast**: Server validates, applies, and broadcasts to other clients
5. **Ack**: Server acknowledges applied steps with new version

### Message Types

```typescript
// Server → Client
{ type: "init", version: number, doc: ProseMirrorDoc }
{ type: "update", steps: Step[], clientIDs: string[] }
{ type: "ack", version: number }
{ type: "error", error: string }

// Client → Server
{ type: "steps", version: number, steps: Step[], clientID: string }
```

## Themes

Switch themes via the settings page or keyboard shortcut `Ctrl+T`.

### Terminal (Default)
Classic terminal aesthetic with:
- Monospace font (JetBrains Mono, Fira Code, etc.)
- Green text on black background
- Scanline effect

### Matrix
Enhanced terminal theme with:
- Bright green (#00FF00)
- Text glow effects
- "Digital rain" animations (CSS)

### Evangelion
NERV-inspired theme with:
- Warning orange (#FF6600)
- Deep purple accents (#6600CC)
- Angular UI elements
- Status indicators

## API Routes

| Route | Method | Description |
|-------|--------|-------------|
| `/` | GET | File browser (main page) |
| `/settings` | GET | Settings page |
| `/edit/{hash}` | GET | Editor for file |
| `/api/files` | GET | File list (HTMX partial) |
| `/ws/collab/{doc_id}` | WS | Collaboration WebSocket |
| `/assets/*` | GET | Static assets |

## Configuration

The web server runs on the same `id serve` process:

```bash
# Start with web UI on port 3000
id serve --web 3000

# Ephemeral mode (no persistence)
id serve --web 3000 --ephemeral
```

## Troubleshooting

### Assets not found (404)

Ensure web assets are built before compiling Rust:

```bash
just web-build && cargo build --features web
```

### WebSocket connection failed

Check that the server is running and the port is accessible. The WebSocket endpoint is at `ws://localhost:PORT/ws/collab/{doc_id}`.

### Theme not applying

Clear browser cache or hard refresh (Ctrl+Shift+R). Theme preference is stored in localStorage.
