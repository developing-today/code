# File List UI Improvements

See [original plan](../../.opencode/plans/) (ad-hoc implementation).

## Overview

Comprehensive improvements to the web UI file list at `/` (served by `id serve`), including file classification, search/filter, date display, URL-by-name routing, and smaller editor buttons.

## Features

### 1. File Classification (FileKind)

Files are classified into three kinds based on their tag name patterns:

- **Primary**: Normal user-created files (e.g., `notes.md`, `config.toml`)
- **Auto**: Auto-backup files matching `auto-{ISO_DATE}` pattern (e.g., `auto-2026-03-12T06:42:30.015Z`)
- **Archive**: Archive files matching `{name}.archive.{unix_timestamp}` pattern, created on save

Classification happens server-side in `get_file_list()` which returns `Vec<FileInfo>` instead of the old `Vec<(String, String, u64)>`.

### 2. FileInfo Struct

```rust
pub struct FileInfo {
    pub name: String,
    pub hash: String,
    pub size: u64,
    pub kind: FileKind,
    pub parent_name: Option<String>,  // For auto/archive: the primary file they relate to
    pub timestamp: Option<u64>,       // Unix timestamp parsed from name
}
```

### 3. File Sorting

Files are sorted with primary files first, then by name. Within each group, files sharing a hash are grouped together so related auto/archive files appear near their parent.

### 4. Search and Filter

- Search input at top of file list for filtering by name (client-side JS)
- Toggle button to show/hide auto and archive files (hidden by default)
- Uses `data-kind` attributes on `<li>` elements for JS filtering
- Filter state persists via `data-show-all` attribute on the file list container

### 5. Date Display

- Timestamps parsed from auto-backup ISO dates and archive unix timestamps
- Displayed as `YYYY-MM-DD HH:MM` UTC in a `.file-date` column
- Primary files show no date (no timestamp available in store metadata)

### 6. URL-by-Name Routing (`/file/:name`)

New route `/file/*name` resolves a filename to its hash and serves the editor/viewer. This enables bookmarkable URLs like `/file/notes.md` instead of `/edit/{64-char-hash}`.

### 7. Smaller Editor Buttons

The `.header-btn` class gets reduced padding (`2px 6px` instead of base button `8px 16px`) to match the compact header aesthetic.

### 8. New File Form Position

The "New File" form is moved above the file list for easier access.

### 9. get_file_name Improvement

`get_file_name()` now prefers primary file names over auto/archive names when multiple tags share the same hash. This ensures the editor header shows "notes.md" not "auto-2026-03-12T...".

### 10. File Rename

The editor header includes a "rename" button that allows renaming files with configurable archiving behavior.

#### API Endpoint

`POST /api/rename` with JSON body:

```rust
struct RenameRequest {
    name: String,      // Current file name
    new_name: String,  // Desired new name
    archive: bool,     // Whether to keep old name as archive
}
```

Response:

```rust
struct RenameResponse {
    name: String,                      // New file name
    hash: String,                      // Content hash (unchanged)
    archived_original: Option<String>, // Archive tag for old name (if archive=true)
    archived_replaced: Option<String>, // Archive tag for replaced file (if target existed)
}
```

#### Rename Behavior

Three scenarios are handled:

1. **Simple rename** (`archive=false`, target doesn't exist): Creates new tag `new_name` → hash, deletes old tag `name`.
2. **Rename with archive** (`archive=true`, target doesn't exist): Creates new tag `new_name` → hash, creates archive tag `{name}.archive.{timestamp}` → hash, deletes old tag `name`.
3. **Rename to existing** (target name already exists): Archives the existing file at target as `{new_name}.archive.{timestamp}`, then proceeds as #1 or #2. This prevents data loss when overwriting.

#### Frontend Flow

1. User clicks "rename" button in editor header
2. Browser `prompt()` asks for new name (pre-filled with current name)
3. Browser `confirm()` asks whether to archive the original name
4. POST to `/api/rename` with the request
5. On success, navigates to `/file/{new_name}` via HTMX or `window.location`

## Architecture

### Data Flow

```
Store (iroh-blobs tags) 
  → get_file_list() classifies each tag into FileInfo
  → render_file_list() generates HTML with data-kind attributes
  → main.ts initFileFilter() adds client-side search/toggle

Rename:
  POST /api/rename → rename_handler
  → Looks up hash for current name
  → If target exists: archives existing as {target}.archive.{ts}
  → Creates new tag: new_name → hash
  → If archive=true: creates {old_name}.archive.{ts}
  → Deletes original tag
  → Returns RenameResponse JSON
```

### File Changes

1. **`src/web/routes.rs`**: FileInfo, FileKind, get_file_list(), /file/*name route, get_file_name(), RenameRequest/RenameResponse, rename_handler
2. **`src/web/templates.rs`**: render_file_list(&[FileInfo]), search/filter UI, date column, rename button in editor header
3. **`web/styles/terminal.css`**: .header-btn, .file-filter, .file-date, .file-badge, .dropdown-menu.show
4. **`web/src/main.ts`**: initFileFilter(), search/toggle, renameFile() method on IdApp

## References

- `src/web/routes.rs` - Backend route handlers (rename_handler at line ~800)
- `src/web/templates.rs` - HTML template rendering (rename button in render_editor)
- `web/src/main.ts` - Frontend JavaScript (renameFile method)
- `web/styles/terminal.css` - Base styles
