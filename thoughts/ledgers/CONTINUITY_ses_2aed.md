---
session: ses_2aed
updated: 2026-04-03T02:25:17.239Z
---



## Conversation Summary

### Overall Task
Transform `pkgs/id` web UI from a single-user markdown editor into a live collaborative platform. A 6-phase roadmap was created, approved, and implementation has begun.

### Completed Work
1. **Build fix**: Wrapped `libsql::Database` in `Arc` in `identity.rs` to fix `Clone` derive
2. **Feature verification**: All 10 features PASS (DaisyUI, identity, collab editing, peer discovery, etc.)
3. **Test documentation**: 8 screenshots + markdown report in `tests/results/2026-04-02T07-46-50Z/`
4. **Display name warning UI**: Soft warning at >8 chars in `templates.rs` + `main.ts`
5. **Lint fixes committed** as `e6a01637`: clippy pedantic, fmt, biome lint cleanups
6. **`just check` passes**: 549 unit + 74 integration + 343 TS tests
7. **Deep codebase analysis**: 7-area assessment of the web UI
8. **6-phase roadmap APPROVED** at `thoughts/shared/plans/pkgs-id-collaborative-web-platform-roadmap/README.md`
9. **Phase 1 plan APPROVED** at `thoughts/shared/plans/pkgs-id-collaborative-web-platform-roadmap/phase-1-fix-save-and-collab.md`

### The 6-Phase Roadmap
1. **Fix Save & Collab** (critical) ← CURRENTLY IMPLEMENTING
2. **Markdown Polish** (parallel)
3. **Identity & Auth**
4. **iroh-docs Versioning**
5. **p2panda Integration**
6. **UX Essentials**

### Phase 1 — Fix Save & Collab (In Progress)

**Root Problem**: Collab sessions are keyed by content hash. When save creates a new blob (new hash), the session breaks for all clients. Additionally, `NEW_VERSION` message type 7 has no client-side handler.

**Phase 1 has 4 parts, in order:**

#### Part 1: Add NEW_VERSION handler to collab.ts (← STARTING THIS NOW)
- Add `NEW_VERSION: 7` to MSG constants in `collab.ts` (between CURSOR_REMOVE=6 and AUTH=8)
- Add `case MSG.NEW_VERSION:` in `handleMessage` switch
- Handler extracts `hash` and `name` from decoded array, emits event/callback for `main.ts`
- **Files**: `pkgs/id/web/src/collab.ts`

#### Part 2: Re-key collab sessions by filename instead of hash
- Change `CollabState` document map key from hash to filename
- WebSocket endpoint `/ws/collab/{name}` instead of `/ws/collab/{hash}`
- Add `current_hash` tracking to `Document` struct
- **Files**: `pkgs/id/src/web/collab.rs`, `pkgs/id/src/web/routes.rs`

#### Part 3: Name-first URL scheme
- `/edit/{name}` (primary), `/view/{name}`, `/hash/{hash}` (redirect), backward compat redirects
- Client no longer changes URL on save (stays at `/edit/{name}`)
- **Files**: `pkgs/id/src/web/routes.rs`, `pkgs/id/src/web/templates.rs`, `pkgs/id/web/src/main.ts`

#### Part 4: Auto-save on idle
- Debounced 2s auto-save, save state indicator, rate limit handling
- Cancel pending auto-save on receiving NewVersion from other client
- **Files**: `pkgs/id/web/src/main.ts`

### Critical Architecture Details (from code analysis)
- **Server collab.rs**: `CollabState` has `documents: HashMap<String, Arc<Document>>` keyed by hash. MSG types: INIT=0, STEPS=1, UPDATE=2, ACK=3, CURSOR=4, ERROR=5, CURSOR_REMOVE=6, NEW_VERSION=7, AUTH=8, AUTH_OK=9. `notify_new_version(old_doc_id, new_hash, name)` broadcasts type 7.
- **Client collab.ts**: MSG consts skip 7 entirely (no NEW_VERSION). `handleMessage` switch has no `case 7`.
- **Save flow (routes.rs:1020-1139)**: Receives `SaveRequest{doc_id, name, doc}`, creates new blob → new hash, archives old tag, sets new tag, calls `notify_new_version`, returns `{hash, name, archive_name}`.
- **Client save (main.ts:1066-1139)**: POSTs to `/api/save`, updates `dataset.docId` and `replaceState` URL to `/edit/{new_hash}`. Does NOT reconnect WebSocket.

### Key Files
- `pkgs/id/src/web/collab.rs` — CollabState, MSG types, session management
- `pkgs/id/src/web/routes.rs` — HTTP handlers, save_handler, edit_handler
- `pkgs/id/src/web/templates.rs` — HTML rendering
- `pkgs/id/web/src/collab.ts` — Client WebSocket collab (missing NEW_VERSION=7)
- `pkgs/id/web/src/main.ts` — Client app logic, save, URL handling

### Constraints & Methodology
- Build with `just` and `nix flake`; keep generated artifacts synced
- Commit after every step; never revert/force/rebase
- Document everything including failed explorations
- Phase docs + validation docs with test specs before coding
- Evidence-based sign-off with screenshots
- If it builds and screenshots look right, proceed to next steps

### What to Do Next
1. **Implement Part 1**: Add `NEW_VERSION = 7` handler to `collab.ts` — was literally about to start this when conversation was compacted
2. Read current `collab.ts` MSG constants and `handleMessage` switch, add the constant and case handler
3. Commit, then proceed to Part 2
