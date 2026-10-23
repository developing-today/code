---
session: ses_2e53
updated: 2026-03-23T12:56:50.651Z
---

## Summary

### Task

Read-only exploration of the web UI codebase in `/home/user/code/pkgs/id` to understand its complete architecture. **No modifications were made.**

### What Was Done

Every relevant file in the codebase was read in full. Complete architectural understanding was established:

### Architecture Overview

**Backend (Rust/Axum):**

- **`src/web/mod.rs`** (216 lines) — `AppState` struct (store, collab, assets, peers, node_id), `web_router()`, asset URL loading
- **`src/web/routes.rs`** (728 lines) — All HTTP handlers. Routes: `/` (file list), `/settings`, `/peers`, `/edit/:hash`, `/blob/:hash`, `/api/files`, `/api/save`, `/api/new`, `/api/download`, `/ws/collab/:doc_id`, `/assets/*path`. All handlers check `HX-Request` header for full page vs HTMX partial.
- **`src/web/templates.rs`** (638 lines) — All HTML built inline via `String::push_str()`/`write!()`. No template engine. Key functions: `render_page()`, `render_file_list()`, `render_editor()`, `render_media_viewer()`, `render_settings()`, `render_peers()`
- **`src/web/collab.rs`** (1243 lines) — WebSocket collaborative editing. MessagePack wire protocol (tags 0-6). Per-document state management. Timeouts: ping 30s, WS close 30m, cursor removal 5m, doc cleanup 1h.
- **`src/web/content_mode.rs`** (446 lines) — Content type detection by extension: Rich (.pm.json), Markdown (.md), Plain (.txt), Raw (code), Media (image/video/audio/pdf), Binary
- **`src/web/markdown.rs`** (970 lines) — Markdown↔ProseMirror JSON conversion via comrak
- **`src/web/assets.rs`** (203 lines) — rust-embed static file serving with cache-busted URLs
- **`src/commands/serve.rs`** (662 lines) — Server startup, default port 3000
- **`src/store.rs`** (414 lines) — iroh-blobs storage. Tags map filename→hash. Persistent (SQLite) or ephemeral (memory).

**Key design details:**

- File names = iroh-blobs tag names. Tags map name→content hash.
- **No dates tracked** — only timestamps in archive tag names
- **File size hardcoded to 0** in file list (TODO exists at routes.rs:265)
- Archive/backup: `{name}.archive.{unix_timestamp}` tags created on save (no auto-save timer)
- 3 themes: sneak (blue), arch (green), mech (orange) — all on #000 background

**Frontend (TypeScript/Bun):**

- `web/src/main.ts`, `editor.ts` (ProseMirror), `collab.ts` (WebSocket client), `cursors.ts`, `cursor-utils.ts`, `theme.ts`
- `web/styles/terminal.css`, `themes.css`, `editor.css`
- Build: Bun bundles JS, concatenates CSS, generates content-hashed filenames + manifest.json, embedded via rust-embed

### Current State

Exploration is **complete**. No modifications were made, no tasks are in progress, and no next steps were defined by the user.

### What's Needed

The user has not specified what to do next. Awaiting instructions on what to build, fix, or modify in this codebase.
