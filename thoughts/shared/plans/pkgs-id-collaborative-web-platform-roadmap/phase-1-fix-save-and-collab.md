# Phase 1 — Fix Save & Collab

## Problem Statement

Saving a document breaks all active collaboration sessions. The root cause is that collab sessions are keyed by content hash. When a save creates a new blob (new hash), the old session becomes stale, the saving client updates its URL but doesn't reconnect, and other clients are stuck on a dead session because the `NEW_VERSION` message (type 7) has no client-side handler.

## Architecture Changes

### Current Flow (broken)
1. Client opens `/edit/{hash}` → WebSocket connects to `/ws/collab/{hash}`
2. User edits → collab steps sync between peers
3. User saves → POST `/api/save` → new blob → new hash
4. Server broadcasts `NewVersion(7, new_hash, name)` to old session
5. Client ignores type 7 (no handler), updates URL to `/edit/{new_hash}`
6. WebSocket still connected to old hash session → collab dead
7. Other clients still on old hash → they're stranded

### Target Flow (fixed)
1. Client opens `/edit/{name}` → server resolves name → hash → content
2. WebSocket connects to `/ws/collab/{name}` (name-keyed session)
3. User edits → collab steps sync (same as before)
4. User saves → POST `/api/save` → new blob → new hash
5. Server updates internal hash reference, session stays alive (same name)
6. Server broadcasts `NewVersion(7, new_hash, name)` on name-keyed session
7. All clients receive NewVersion, update their hash reference (no reconnect needed)
8. URL stays `/edit/{name}` — no change needed

---

## Parts

### Part 1: Add NEW_VERSION handler to collab.ts

**What**: Add `MSG.NEW_VERSION = 7` constant and handle it in `handleMessage`.

**Files**:
- `pkgs/id/web/src/collab.ts`

**Changes**:
- Add `NEW_VERSION: 7` to MSG constants (between CURSOR_REMOVE=6 and AUTH=8)
- Add `case MSG.NEW_VERSION:` in `handleMessage` switch
- Handler should: extract `hash` and `name` from decoded array, emit a custom event or callback so `main.ts` can update `dataset.docId` and optionally show a toast

**Test spec**:
- Unit test: construct a msgpack-encoded `[7, "newhash123", "test.md"]` buffer, feed to `handleMessage`, verify the hash/name are extracted
- Integration: with server running, save from one client, verify second client receives NewVersion

---

### Part 2: Re-key collab sessions by filename instead of hash

**What**: Change `CollabState` document map key from hash to filename.

**Files**:
- `pkgs/id/src/web/collab.rs`
- `pkgs/id/src/web/routes.rs`

**Changes in collab.rs**:
- `documents: HashMap<String, Arc<Document>>` — key becomes filename (already a String, just different semantics)
- `get_or_create(doc_id, ...)` — `doc_id` parameter becomes filename
- `notify_new_version` — no longer needs `old_doc_id` param, just `filename` and `new_hash`
- Add `current_hash` field to `Document` struct (or a parallel `HashMap<String, String>` for name→hash)
- WebSocket endpoint changes from `/ws/collab/{hash}` to `/ws/collab/{name}`

**Changes in routes.rs**:
- `edit_handler`: resolve name from hash (existing `get_file_name`), pass name to collab
- `save_handler`: use filename as session key, not old hash
- `ws_collab_handler`: extract name from path instead of hash

**Test spec**:
- Save a document, verify the collab session is still alive (same session object)
- Two clients connected to same filename, one saves → both still connected
- Verify `Document` tracks current hash correctly after save

---

### Part 3: Name-first URL scheme

**What**: Add routes `/edit/{name}`, `/view/{name}`. Keep `/edit/{hash}` as fallback that redirects to name-based URL.

**Files**:
- `pkgs/id/src/web/routes.rs`
- `pkgs/id/src/web/templates.rs` (update links in HTML)
- `pkgs/id/web/src/main.ts` (update URL handling after save)

**New routes**:
- `GET /edit/{name}` — resolve name→hash→content, render editor (primary)
- `GET /view/{name}` — resolve name→hash→content, render viewer (primary)
- `GET /hash/{hash}` — resolve hash→name, redirect to `/edit/{name}` (fallback)
- `GET /edit/:hash` — keep for backward compat, redirect to name URL
- `GET /user/{pubkey}` — list documents by user (future, stub only)

**Changes**:
- `edit_handler` now accepts name, resolves to hash internally
- New `edit_by_hash_handler` that resolves hash→name, redirects
- `save_handler` response no longer needs hash for URL — client stays on `/edit/{name}`
- `main.ts`: after save, don't update URL (it's already correct)
- Templates: links use `/edit/{name}` format

**Test spec**:
- `GET /edit/test.md` → 200, renders editor
- `GET /hash/{some_hash}` → 302 redirect to `/edit/test.md`
- `GET /edit/{old_hash}` → 302 redirect to `/edit/{name}` (backward compat)
- Save from `/edit/test.md` → URL stays `/edit/test.md`, no navigation

---

### Part 4: Auto-save on idle

**What**: Debounced auto-save 2 seconds after last edit. Visual indicator shows save state.

**Files**:
- `pkgs/id/web/src/main.ts`

**Changes**:
- Add debounce timer: on each ProseMirror transaction that changes content, reset a 2s timer
- When timer fires, call the existing save function
- Add save state indicator: "Saved" / "Saving..." / "Unsaved changes" in the UI
- Respect the server's 5s rate limit — if save returns rate-limited, retry after cooldown
- On NewVersion from another client, cancel pending auto-save (their version is newer)

**Test spec**:
- Edit text, wait 2s, verify save API is called
- Edit text rapidly for 5s, verify only 1 save at the end (debounce works)
- Edit, save fires, edit again within 5s → second save waits for rate limit
- Receive NewVersion → pending auto-save cancelled

---

## Implementation Order

1. Part 1 (NEW_VERSION handler) — smallest change, immediately useful
2. Part 2 (re-key by filename) — core architectural fix
3. Part 3 (name-first URLs) — depends on Part 2
4. Part 4 (auto-save) — depends on Parts 1-3 working

## Validation Criteria

- [ ] Two browsers editing same doc, one saves → both keep editing without interruption
- [ ] URL shows `/edit/{filename}` not `/edit/{hash}`
- [ ] Old hash URLs redirect to name URLs
- [ ] Auto-save triggers after 2s idle
- [ ] `just check` passes (all existing tests)
- [ ] Screenshots of: save with two clients, URL scheme, auto-save indicator
