# pkgs/id — Collaborative Web Platform Roadmap

## Overview

Transform `pkgs/id` web UI from a single-user markdown editor into a live collaborative platform with persistent identity, versioned documents, and peer-to-peer sync.

## Implementation Methodology

- One phase/part at a time
- Phase docs + validation docs with test specs **before** coding
- Evidence-based sign-off with screenshots
- Commit after every step
- Never revert/force/rebase
- Document everything including failed explorations

---

## Phase 1 — Fix Save & Collab (Critical)

**Doc**: `thoughts/shared/plans/pkgs-id-collaborative-web-platform-roadmap/phase-1-fix-save-and-collab.md`

**Priority**: Critical — current save is broken when collaboration is active

### Goals
- Decouple sessions from content hashes (sessions persist across edits)
- Fix `NewVersion` MSG type 7 in `collab.ts` (currently unhandled)
- Name-first URL scheme: `/edit/{name}`, `/view/{name}`, `/hash/{hash}`, `/user/{pubkey}`
- Hash becomes a fallback identifier, not the primary one
- Auto-save on idle (debounced ~2s after last edit)

### Key Files
- `pkgs/id/src/web/collab.rs` — server-side collab state
- `pkgs/id/src/web/routes.rs` — HTTP route handlers, `save_handler`
- `pkgs/id/web/src/collab.ts` — client-side WebSocket collab
- `pkgs/id/web/src/main.ts` — app initialization, save logic

---

## Phase 2 — Markdown Polish (Parallel with Phase 1)

**Doc**: `thoughts/shared/plans/pkgs-id-collaborative-web-platform-roadmap/phase-2-markdown-polish.md`

**Priority**: High — improves everyday editing experience

### Goals
- GFM extensions: tables, strikethrough, task lists
- Image alt-text support in rendered markdown
- Image browser (list/pick from uploaded images)
- Resize handles for images (future: pretext + 2D canvas)

### Key Files
- `pkgs/id/src/web/templates.rs` — HTML rendering
- `pkgs/id/web/src/main.ts` — editor UI

---

## Phase 3 — Identity & Auth

**Doc**: `thoughts/shared/plans/pkgs-id-collaborative-web-platform-roadmap/phase-3-identity-and-auth.md`

**Priority**: High — foundation for ownership and permissions

### Goals
- Binary key-value tags: null-separated pairs (explore escaping strategies)
- Ownership model: first-created-wins, user namespaces
- CLI flags: `--tags`, `--tags-json`, `--tags-json-file`
- Challenge-response one-time tokens with permission levels:
  - `read` / `write` / `manage` / `manage-no-self-remove`
- QR code generation: terminal, image file, and web display

### Key Files
- `pkgs/id/src/web/identity.rs` — IdentityStore
- `pkgs/id/src/identity.rs` — core identity logic
- `pkgs/id/src/cli.rs` — CLI argument handling

---

## Phase 4 — iroh-docs Versioning

**Doc**: `thoughts/shared/plans/pkgs-id-collaborative-web-platform-roadmap/phase-4-iroh-docs-versioning.md`

**Priority**: Medium — enables version history and document namespaces

### Goals
- Client-scoped namespaces (not per-doc) for iroh-docs
- ProseMirror as canonical storage format (upgrade on first edit, export back to markdown)
- Version DAG with fork and merge support
- Replace archive tags with proper versioning

### Key Files
- `pkgs/id/src/web/collab.rs` — collab state management
- iroh-docs integration files (TBD during phase doc creation)

---

## Phase 5 — p2panda Integration

**Doc**: `thoughts/shared/plans/pkgs-id-collaborative-web-platform-roadmap/phase-5-p2panda-integration.md`

**Priority**: Medium — adds p2p sync, groups, and advanced collaboration

### Goals
- Integrate p2panda core crates: core/net/auth/sync/encryption/spaces
- Native groups with RBAC (role-based access control)
- Streams: chatrooms, line-comments-in-PM-blobs, firehose
- Cross-node sync via p2panda LogSync (evaluate diamond types/automerge if needed)
- RBAC-scoped tokens
- Offline editing with eventual consistency

### Key Files
- New integration layer (TBD during phase doc creation)
- `pkgs/id/src/web/collab.rs` — collab architecture updates

---

## Phase 6 — UX Essentials

**Doc**: `thoughts/shared/plans/pkgs-id-collaborative-web-platform-roadmap/phase-6-ux-essentials.md`

**Priority**: Medium — polish and usability

### Goals
- Sidebar tree navigation for documents
- Drag-drop file upload
- Folders via tags (virtual folder structure)
- Keyboard shortcuts
- Mobile responsive layout

### Key Files
- `pkgs/id/web/src/main.ts` — UI components
- `pkgs/id/src/web/templates.rs` — layout templates
- `pkgs/id/web/styles/` — CSS/styling

---

## Working Approach

Focus on one phase at a time. Complete it thoroughly before moving on. Within each phase, break work into small parts and commit after every meaningful step.

If a parallel task would unblock or accelerate the current phase, pull it in — but only when it concretely helps what's being worked on right now. Don't start future phases speculatively.

The numbered order reflects natural dependencies (save must work before auth makes sense, etc.) but isn't rigid. Let the work guide the sequence.
