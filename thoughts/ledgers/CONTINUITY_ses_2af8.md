---
session: ses_2af8
updated: 2026-04-02T23:29:36.876Z
---



## Summary of Phase 1 Part 2 Steps 1-5: Re-key Collab Sessions by Filename

### Task
Re-key collaborative editing sessions from content hash to filename in the Rust backend, so WebSocket sessions persist across file saves (which change the hash).

### What Was Done — All 5 Steps Completed

**Step 1: Add `hash` field to Document struct (collab.rs)**
- Added `pub hash: RwLock<String>` field to `Document` struct
- Added `hash: RwLock::new(String::new())` initialization in `with_doc_and_mode`

**Step 2: Change CollabState to key by filename (collab.rs)**
- Changed `get_or_create` signature from `(doc_id, initial_content, filename: Option<&str>)` to `(filename, hash, initial_content)`
- Key is now `filename` instead of `doc_id`; creates `Document::with_content(initial_content, Some(filename))`
- After creation, sets `*doc.hash.write().await = hash.to_owned()`
- Changed `notify_new_version` signature from `(old_doc_id, new_hash, filename)` to `(filename, new_hash)`
- Looks up by `filename` instead of `old_doc_id`, updates stored hash after broadcast

**Step 3: Update handle_collab_socket (collab.rs)**
- `doc_id` now represents filename; resolves filename→hash via `super::routes::get_hash_for_name`
- Loads content using resolved hash, calls `get_or_create` with new signature
- Updated log message to remove `filename` parameter (doc_id IS the filename now)
- **Note:** `filename: Option<String>` parameter is now unused but still in signature (caller passes it)

**Step 4: Update edit_handler and file_by_name_handler (routes.rs)**
- Made `get_hash_for_name` → `pub(crate) async fn get_hash_for_name`
- edit_handler: Changed all `render_editor(&hash, &name, ...)` → `render_editor(&name, &name, ..., &hash)` and `render_editor_page(&hash, &name, ..., &state.assets)` → `render_editor_page(&name, &name, ..., &hash, &state.assets)` (both editable and error cases)
- file_by_name_handler: Same changes for not-found (empty hash), editable, and error cases

**Step 5: Update save_handler (routes.rs)**
- Changed `.notify_new_version(&req.doc_id, &new_hash_str, &req.name)` → `.notify_new_version(&req.name, &new_hash_str)`

**Templates (templates.rs)**
- `render_editor`: Added `hash: &str` param, `hash_escaped = html_escape(hash)`, added `data-hash="{}"` to editor container div, changed blob download link from `doc_id_escaped` to `hash_escaped`
- `render_editor_page`: Added `hash: &str` param, passes it through to inner `render_editor` call

### Remaining Work
1. **Handle unused `filename` parameter** in `handle_collab_socket` — needs underscore prefix `_filename` to avoid clippy warning
2. **Run `cargo fmt`** in pkgs/id/
3. **Run `cargo clippy`** in pkgs/id/ to catch warnings
4. **Verify compilation** succeeds

### Key Files Modified
- `pkgs/id/src/web/collab.rs` — Document struct, CollabState methods, handle_collab_socket
- `pkgs/id/src/web/routes.rs` — get_hash_for_name visibility, edit_handler, file_by_name_handler, save_handler
- `pkgs/id/src/web/templates.rs` — render_editor, render_editor_page signatures and HTML output

### Key Constraints
- No `unwrap()`, `expect()`, `panic!()`, `todo!()`, `unimplemented!()`, or `dbg!()` allowed (clippy denies them)
- The `edit_handler` still receives hash in URL (`/edit/{hash}`) — route change is Phase 1 Part 3
- `edit_handler` resolves hash→name, then passes **name** as doc_id and **hash** as separate data attribute
