# HTMX to Datastar + DaisyUI/Tailwind Migration Plan

## Summary

Migrate the `pkgs/id` web UI from **HTMX 1.9.10 to Datastar** for reactive interactions and from **custom terminal CSS (~1200 lines) to DaisyUI v5 + Tailwind CSS v4** for styling. WebSocket endpoints (`/ws/collab/{doc_id}` and `/ws/tags`) remain unchanged. The migration follows a 4-phase CSS-first approach ensuring the application stays functional at the end of every phase.

**Key architectural shifts:**
- Server responses for Datastar-triggered requests become SSE (`text/event-stream`) with `datastar-merge-fragments` events, via a typed `DatastarResponse` builder in Rust
- Full-page loads remain normal HTML; only Datastar-initiated requests return SSE fragments
- Tailwind CSS v4 with DaisyUI v5 replaces all custom CSS, using CSS-first configuration
- Three CRT terminal themes (sneak/arch/mech) are recreated as custom DaisyUI themes with monospace fonts, zero border-radius, and glow effects
- Build pipeline switches from CSS concatenation to Tailwind CLI processing via Bun

---

## Original Prompt

> update the website to use datastar instead of htmx and use daisyui/tailwind for styling and other features. keep the websockets as-is we dont want to use sse for the current uses of websockets. design a plan with a summary, then this exact prompt as new section followed by subsections for each feedback/response appended as the plan is developed. then go into details for design/intent/architecture, and then low level implementation details. create a phased todo task list with checkboxes going into small steps that must be done. then load the tasks and begin once approved.

### Brainstorm Feedback & Decisions

#### Branch 1: Server Response Architecture
**Decision: Full SSE with Typed Builder Pattern**
- Every Datastar-triggered route returns SSE (`text/event-stream`) with `datastar-merge-fragments` events
- Create a `DatastarResponse` builder in Rust with methods: `.merge_fragment()`, `.merge_signals()`, `.remove_fragments()`, `.execute_script()`
- Initial page load (`GET /`) serves a full HTML shell; subsequent navigations are SSE fragment updates targeting `#main`
- The `HX-Request` header detection is removed entirely; replaced by distinguishing initial page load vs Datastar-triggered request (presence of `datastar` query param or accept header)
- WebSocket endpoints remain completely untouched

#### Branch 2: Build Pipeline
**Decision: Tailwind CLI v4 via Bun**
- Install `@tailwindcss/cli` and `daisyui` as Bun dev dependencies
- Single Tailwind input file (`web/src/input.css`) replaces the old `build-css.ts` concatenation script
- Build command: `bunx @tailwindcss/cli -i src/input.css -o dist/styles.css --minify`
- Add `@source "../src/web/templates.rs"` directive so Tailwind scans Rust template strings for class names
- Existing `build-manifest.ts` continues handling content-hash filenames
- Bun orchestrates all build steps; unified dev/watch mode

#### Branch 3: Theme Strategy
**Decision: CRT Terminal Aesthetic on DaisyUI**
- 3 custom themes extending DaisyUI dark base:
  - **sneak**: primary = blue (`oklch(0.65 0.20 250)`)
  - **arch**: primary = green (`oklch(0.75 0.25 145)`)
  - **mech**: primary = orange (`oklch(0.70 0.20 55)`)
- All themes share: `base-100` = `#000000`, `base-200` = `#0a0a0a`, `base-300` = `#111111`
- Zero border-radius: `--rounded-btn: 0; --rounded-box: 0; --rounded-badge: 0`
- Global monospace font stack via Tailwind `fontFamily` extend
- CRT effects (text-shadow glow, scanline overlay, flicker animation) in custom `@layer components`, not in theme config
- Theme switching via `data-theme` attribute on `<html>` element (compatible with both DaisyUI and Datastar signals)

#### Branch 4: Migration Phasing
**Decision: CSS-First, 4-Phase Approach**
- Phase 1: Build pipeline + theme setup (HTMX stays working)
- Phase 2: CSS/component migration page-by-page (HTMX still working, visual layer migrated)
- Phase 3: HTMX to Datastar conversion (SSE builder, route-by-route)
- Phase 4: Cleanup & polish (remove HTMX, dead CSS, visual QA)
- Application remains fully functional at the end of every phase

---

## Design / Intent / Architecture

### Current Architecture

```
Browser                     Axum Server
  |                              |
  |-- GET / (full page) -------->|-- renders full HTML (render_page + render_main_page_wrapper)
  |<--- HTML document -----------|
  |                              |
  |-- GET /file/x (HX-Request) ->|-- detects HX-Request header
  |<--- HTML fragment -----------|-- returns partial HTML (just #main content)
  |                              |
  |-- WS /ws/collab/{id} ------->|-- binary MessagePack (ProseMirror collab)
  |-- WS /ws/tags -------------->|-- JSON text (tag change notifications)
```

### Target Architecture

```
Browser (Datastar SDK)      Axum Server
  |                              |
  |-- GET / (initial) ---------->|-- returns full HTML shell with data-signals, data-on-load
  |<--- HTML document -----------|
  |                              |
  |-- @get('/files') ----------->|-- returns SSE: datastar-merge-fragments
  |<--- text/event-stream -------|-- fragment targets #main via data-merge-mode
  |                              |
  |-- @get('/file/x') ---------->|-- returns SSE: merge-fragments + merge-signals
  |<--- text/event-stream -------|
  |                              |
  |-- WS /ws/collab/{id} ------->|-- UNCHANGED: binary MessagePack
  |-- WS /ws/tags -------------->|-- UNCHANGED: JSON text
```

### DatastarResponse Builder (Rust)

```rust
pub struct DatastarResponse {
    fragments: Vec<Fragment>,
    signals: Option<serde_json::Value>,
    scripts: Vec<String>,
    removals: Vec<String>,
}

impl DatastarResponse {
    pub fn new() -> Self { ... }

    /// Add an HTML fragment to merge into the DOM
    pub fn merge_fragment(mut self, selector: &str, html: &str, mode: MergeMode) -> Self { ... }

    /// Update reactive signals (client-side state)
    pub fn merge_signals(mut self, signals: serde_json::Value) -> Self { ... }

    /// Remove elements matching selector
    pub fn remove_fragments(mut self, selector: &str) -> Self { ... }

    /// Execute JavaScript on the client
    pub fn execute_script(mut self, script: &str) -> Self { ... }

    /// Convert to Axum SSE response
    pub fn into_response(self) -> axum::response::Sse<impl Stream<Item = ...>> { ... }
}

pub enum MergeMode {
    Morph,       // default - intelligent DOM diffing
    Inner,       // replace innerHTML
    Outer,       // replace outerHTML
    Prepend,
    Append,
    Before,
    After,
    DeleteElement,
}
```

### Template Refactoring

**Before (templates.rs):**
```rust
pub fn render_file_list(files: &[FileInfo], search: &str, page: u32) -> String {
    let mut html = String::with_capacity(8192);
    write!(html, r#"<div id="file-list">"#);
    write!(html, r#"<input hx-get="/api/files" hx-trigger="keyup changed delay:300ms"
                           hx-target="#file-list-content" hx-include="[name='search']">"#);
    // ... renders full page or partial based on context
    html
}
```

**After (templates.rs):**
```rust
pub fn render_file_list(files: &[FileInfo], search: &str, page: u32) -> String {
    let mut html = String::with_capacity(8192);
    write!(html, r#"<div id="file-list" data-signals-search="{search}">"#);
    write!(html, r#"<input class="input input-bordered font-mono"
                           data-model="search"
                           data-on-keyup__debounce.300ms="@get('/api/files')">"#);
    // ... same template logic, different attributes
    html
}
```

### Route Handler Refactoring

**Before (routes.rs):**
```rust
async fn file_list(headers: HeaderMap, query: Query<FileListParams>) -> impl IntoResponse {
    let is_htmx = headers.get("HX-Request").is_some();
    let html = if is_htmx {
        templates::render_file_list_content(&files, &query.search, query.page)
    } else {
        templates::render_page(&templates::render_file_list(&files, &query.search, query.page))
    };
    Html(html)
}
```

**After (routes.rs):**
```rust
// Initial page load - returns full HTML
async fn index(state: State<AppState>) -> impl IntoResponse {
    let files = state.list_files(1, 50, "").await;
    let html = templates::render_page(&templates::render_file_list(&files, "", 1));
    Html(html)
}

// Datastar-triggered - returns SSE fragments
async fn file_list_fragment(query: Query<FileListParams>) -> impl IntoResponse {
    let files = state.list_files(query.page, 50, &query.search).await;
    DatastarResponse::new()
        .merge_fragment("#file-list-content",
            &templates::render_file_list_content(&files, &query.search, query.page),
            MergeMode::Morph)
        .merge_signals(json!({ "page": query.page, "search": query.search }))
        .into_response()
}
```

### CSS Architecture

**Input file (`web/src/input.css`):**
```css
@import "tailwindcss";
@plugin "daisyui";
@source "../../src/web/templates.rs";

/* === Custom DaisyUI Themes === */
@plugin "daisyui/theme" {
  name: "sneak";
  default: true;
  prefersdark: true;
  color-scheme: dark;
  --color-primary: oklch(0.65 0.20 250);
  --color-secondary: oklch(0.50 0.15 250);
  --color-accent: oklch(0.70 0.22 250);
  --color-base-100: #000000;
  --color-base-200: #0a0a0a;
  --color-base-300: #111111;
  --color-base-content: oklch(0.85 0.02 250);
  --color-info: oklch(0.65 0.20 250);
  --color-success: oklch(0.72 0.22 145);
  --color-warning: oklch(0.75 0.18 85);
  --color-error: oklch(0.65 0.25 25);
  --rounded-btn: 0;
  --rounded-box: 0;
  --rounded-badge: 0;
  --border: 1px;
  --depth: 0;
}

@plugin "daisyui/theme" {
  name: "arch";
  prefersdark: true;
  color-scheme: dark;
  --color-primary: oklch(0.75 0.25 145);
  /* ... similar structure, green primary ... */
}

@plugin "daisyui/theme" {
  name: "mech";
  prefersdark: true;
  color-scheme: dark;
  --color-primary: oklch(0.70 0.20 55);
  /* ... similar structure, orange primary ... */
}

/* === CRT Terminal Effects === */
@layer components {
  .crt-glow {
    text-shadow: 0 0 5px currentColor, 0 0 10px currentColor;
  }

  .crt-scanlines::after {
    content: "";
    position: fixed;
    inset: 0;
    background: repeating-linear-gradient(
      0deg,
      rgba(0, 0, 0, 0.15) 0px,
      rgba(0, 0, 0, 0.15) 1px,
      transparent 1px,
      transparent 2px
    );
    pointer-events: none;
    z-index: 9999;
  }

  @keyframes crt-flicker {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.98; }
  }
}

/* === ProseMirror Editor Styles (preserved) === */
@import "./editor.css";
```

### Theme Mapping

| Current CSS Property | DaisyUI Equivalent | Notes |
|---|---|---|
| `--bg-primary` (#000) | `base-100` | Main background |
| `--bg-secondary` (#0a0a0a) | `base-200` | Card/section background |
| `--bg-tertiary` (#111) | `base-300` | Elevated surfaces |
| `--text-primary` (theme color) | `base-content` | Main text |
| `--text-muted` | `base-content/50` | Opacity modifier |
| `--accent` (per theme) | `primary` | Theme accent color |
| `--accent-hover` | `primary-focus` | Hover state |
| `--success/warning/error` | `success/warning/error` | Direct mapping |
| `--border` | `base-content/20` | Border color |
| `--glow-color` | Custom `crt-glow` class | Via text-shadow |

### Component Mapping

| Current Custom CSS | DaisyUI Component | Tailwind Utilities |
|---|---|---|
| `.card` | `card bg-base-200` | - |
| `.btn` | `btn btn-primary btn-outline` | `font-mono uppercase` |
| `.file-item` | `card bg-base-200 hover:bg-base-300` | `p-3 border border-base-content/20` |
| `.search-input` | `input input-bordered` | `w-full font-mono bg-base-100` |
| `.pagination` | `join` + `btn btn-sm` | `font-mono` |
| `.header` | `navbar bg-base-200` | `border-b border-base-content/20` |
| `.footer` | `footer bg-base-200` | `border-t border-base-content/20` |
| `.tag` | `badge badge-outline` | `font-mono text-xs` |
| `.dropdown` | `dropdown` + `menu` | - |
| `.theme-btn` | `btn btn-circle btn-sm` | Custom gradient still needed |
| `.toggle-checkbox` | `toggle toggle-primary` | `toggle-sm` |
| Modal/dialog | `modal` | `modal-box bg-base-200` |
| Tabs (if any) | `tabs tabs-bordered` | - |

### Datastar Attribute Mapping

| HTMX Pattern | Datastar Equivalent |
|---|---|
| `hx-get="/url"` | `data-on-click="@get('/url')"` |
| `hx-post="/url"` | `data-on-click="@post('/url')"` |
| `hx-target="#el"` | Server-side: fragment targets `#el` in SSE |
| `hx-swap="innerHTML"` | Server-side: `MergeMode::Inner` |
| `hx-swap="outerHTML"` | Server-side: `MergeMode::Outer` |
| `hx-push-url="true"` | `data-on-click="@get('/url', {push: true})"` or use `window.history.pushState` in execute_script |
| `hx-trigger="keyup changed delay:300ms"` | `data-on-keyup__debounce.300ms="@get('/url')"` |
| `hx-trigger="every 10s"` | `data-on-load__interval.10000ms="@get('/url')"` |
| `hx-trigger="change"` | `data-on-change="@get('/url')"` |
| `hx-include="[name='search']"` | Automatic via `data-signals` (signals sent with every request) |
| `hx-select="#content"` | Server returns only the needed fragment |
| `htmx.ajax("GET", url, opts)` | `@get(url)` from JS or Datastar action |
| `htmx:afterSwap` event | `data-on-load` on fragment, or `execute_script` in SSE |
| `htmx:beforeRequest` event | `data-on-click` handler before `@get` |
| `htmx.process(el)` | Not needed (Datastar auto-processes new fragments) |

### URL History / SPA Navigation

Datastar doesn't have built-in URL push like HTMX's `hx-push-url`. Options:
1. Use `execute_script` in SSE response to call `window.history.pushState()`
2. Client-side wrapper: `data-on-click="$$pushUrl('/path'); @get('/path')"`
3. Custom Datastar plugin for URL management

Recommended: Option 1 - server controls URL updates via `execute_script` in the SSE response, keeping all routing logic server-side.

---

## Low-Level Implementation Details

### Phase 1: Build Pipeline + Theme Setup

#### 1.1 Install Dependencies
```bash
cd pkgs/id/web
bun add -d @tailwindcss/cli daisyui@latest
bun add @starfederation/datastar  # add now, use in Phase 3
bun remove htmx.org               # defer to Phase 4
```

#### 1.2 Create Tailwind Input File
Create `web/src/input.css` with:
- `@import "tailwindcss"`
- `@plugin "daisyui"`
- `@source "../../src/web/templates.rs"` (scan Rust templates for classes)
- `@source "./main.ts"` (scan TypeScript for classes)
- Three `@plugin "daisyui/theme"` blocks (sneak, arch, mech)
- CRT effect classes in `@layer components`
- ProseMirror styles imported from `./editor-compat.css` (extracted from current editor.css, stripped of redundant resets)

#### 1.3 Update Build Scripts
Replace `build-css.ts` with Tailwind CLI invocation:
```json
{
  "scripts": {
    "build:css": "bunx @tailwindcss/cli -i src/input.css -o dist/styles.css --minify",
    "build:js": "bun build src/main.ts --outdir dist --entry-naming [name].[hash].js",
    "build:manifest": "bun run scripts/build-manifest.ts",
    "build": "bun run build:js && bun run build:css && bun run build:manifest",
    "dev": "bun run build --watch"
  }
}
```

Note: `build-manifest.ts` needs a small update - the CSS output is now `styles.css` (no hash from Tailwind CLI). Options:
- Post-process: read `dist/styles.css`, compute hash, rename to `styles.{hash}.css`, write manifest
- Or: modify `build-manifest.ts` to handle the unhashed CSS file by computing hash itself

#### 1.4 Verify HTMX Still Works
At this point HTMX is still loaded and all `hx-*` attributes are intact. The only change is styling. Run the app and verify all pages render correctly with new Tailwind/DaisyUI classes (they won't yet - this is just pipeline verification).

### Phase 2: CSS/Component Migration

#### 2.1 Template Class Replacement Strategy
Work page-by-page through `templates.rs`, replacing custom CSS classes with Tailwind/DaisyUI equivalents. The HTMX attributes (`hx-get`, `hx-target`, etc.) remain untouched.

**Order of pages:**
1. Layout shell (header, footer, page wrapper) - affects all pages
2. File list page - most complex, search + pagination + file items
3. Editor page - ProseMirror container, toolbar
4. Media viewer - relatively simple
5. Binary viewer - relatively simple
6. Peers page - auto-refresh table
7. Settings page - theme switcher, toggles

#### 2.2 Layout Shell Migration
```rust
// Before:
write!(html, r#"<header class="header">"#);

// After:
write!(html, r#"<header class="navbar bg-base-200 border-b border-base-content/20 px-4 font-mono">"#);
```

#### 2.3 Custom Utilities Preserved
Some custom utilities may not have direct DaisyUI equivalents:
- Scrollbar styling → keep as custom CSS in `@layer utilities`
- ProseMirror-specific classes → keep in `editor-compat.css`
- CRT scanline overlay → custom `crt-scanlines` component class
- Phosphor glow on hover → `crt-glow` component class

#### 2.4 Theme Switcher Update
Current: `.theme-btn` with custom gradient backgrounds
New: DaisyUI `swap` or `btn` with inline style for the gradient preview. Theme switching JS (`theme.ts`) updated to set `document.documentElement.setAttribute('data-theme', name)`.

### Phase 3: HTMX to Datastar Conversion

#### 3.1 Create DatastarResponse Module
New file: `src/web/datastar.rs`

```rust
use axum::response::{IntoResponse, Response, Sse};
use axum::http::header;
use futures::stream;
use std::convert::Infallible;

pub struct DatastarResponse { /* ... */ }

impl IntoResponse for DatastarResponse {
    fn into_response(self) -> Response {
        let events: Vec<String> = Vec::new();

        for fragment in &self.fragments {
            events.push(format!(
                "event: datastar-merge-fragments\ndata: selector {}\ndata: merge {}\ndata: fragments {}\n\n",
                fragment.selector, fragment.mode.as_str(), fragment.html
            ));
        }

        if let Some(ref signals) = self.signals {
            events.push(format!(
                "event: datastar-merge-signals\ndata: signals {}\n\n",
                serde_json::to_string(signals).unwrap()
            ));
        }

        for script in &self.scripts {
            events.push(format!(
                "event: datastar-execute-script\ndata: script {}\n\n",
                script
            ));
        }

        let body = events.join("");
        Response::builder()
            .header(header::CONTENT_TYPE, "text/event-stream")
            .header(header::CACHE_CONTROL, "no-cache")
            .body(body.into())
            .unwrap()
    }
}
```

#### 3.2 Route Migration Order
1. **File list search/pagination** (`GET /api/files`) - most exercised, good test case
2. **Navigation** (file list → editor → back) - SPA navigation core
3. **File operations** (create, rename, copy, save) - POST endpoints
4. **Peers page** (auto-refresh via `data-on-load__interval`)
5. **Settings page** - simple form
6. **Download/export** - may need special handling (non-HTML response)

#### 3.3 JavaScript Migration (main.ts)
- Remove `import htmx` and all `htmx.*` calls
- Import `@starfederation/datastar` SDK
- Replace `htmx.ajax()` calls with Datastar `@get`/`@post` actions or direct `fetch()` + manual SSE parsing
- Replace `htmx:afterSwap` event listeners with `data-on-load` attributes on fragments, or `MutationObserver` for editor initialization
- The `IdApp` API object stays but its methods change internally:
  - `IdApp.navigate(url)` → triggers `@get(url)` or dispatches custom event
  - `IdApp.createFile()` → `@post('/api/new')` then navigate to result
  - `IdApp.renameFile()` → `@post('/api/rename')` then navigate
  - etc.

#### 3.4 Signals Architecture
```html
<html data-signals-theme="'sneak'"
      data-signals-search="''"
      data-signals-page="1"
      data-signals-show_deleted="false"
      data-signals-current_file="''">
```

These signals are automatically sent with every Datastar request, eliminating the need for `hx-include`.

### Phase 4: Cleanup & Polish

#### 4.1 Remove Dead Code
- Delete `web/styles/terminal.css`, `web/styles/themes.css` (replaced by input.css)
- Remove `htmx.org` from `package.json`
- Remove `build-css.ts` (replaced by Tailwind CLI)
- Remove any leftover `hx-*` attribute references in templates
- Remove `is_htmx_request()` helper and `HX-Request` header checks

#### 4.2 Visual QA
- Test all 3 themes across all pages
- Verify CRT effects (glow, scanlines) render correctly
- Check ProseMirror editor styling is intact
- Test responsive layout (if applicable)
- Verify WebSocket collab still works (cursor display, step sync)
- Verify tag WebSocket still works (real-time updates)

#### 4.3 Performance
- Compare bundle sizes (before/after)
- Verify Tailwind purges unused classes
- Check SSE response sizes vs old HTML fragment sizes
- Verify no memory leaks from SSE connections

---

## Phased Task List

### Phase 1: Build Pipeline + Theme Setup
- [ ] 1.1 Install Tailwind CLI and DaisyUI as Bun dev dependencies
- [ ] 1.2 Install Datastar SDK as runtime dependency (used in Phase 3)
- [ ] 1.3 Create `web/src/input.css` with Tailwind imports and DaisyUI plugin
- [ ] 1.4 Define sneak theme (blue primary, black backgrounds, zero border-radius)
- [ ] 1.5 Define arch theme (green primary, same base structure)
- [ ] 1.6 Define mech theme (orange primary, same base structure)
- [ ] 1.7 Add CRT effect classes in `@layer components` (glow, scanlines, flicker)
- [ ] 1.8 Extract ProseMirror-compatible styles into `web/src/editor-compat.css`
- [ ] 1.9 Add `@source` directive for Rust template class scanning
- [ ] 1.10 Update `package.json` build:css script to use Tailwind CLI
- [ ] 1.11 Update `build-manifest.ts` to handle Tailwind CSS output (hash + rename)
- [ ] 1.12 Run build, verify CSS output contains DaisyUI classes and themes
- [ ] 1.13 Verify HTMX still works with new CSS (visual may differ, interactions intact)

### Phase 2: CSS/Component Migration (HTMX stays)
- [ ] 2.1 Migrate layout shell: `render_page()` - html, head, body wrapper
- [ ] 2.2 Migrate header: `render_main_page_wrapper()` - navbar, nav links
- [ ] 2.3 Migrate footer: render_main_page_wrapper footer section
- [ ] 2.4 Migrate file list page: search input, show-deleted toggle, file grid
- [ ] 2.5 Migrate file list items: file cards with name, size, date, tags
- [ ] 2.6 Migrate pagination: prev/next buttons, page info
- [ ] 2.7 Migrate editor page: toolbar, ProseMirror container, save/export buttons
- [ ] 2.8 Migrate media viewer: image/video/audio display
- [ ] 2.9 Migrate binary viewer: hex dump or download prompt
- [ ] 2.10 Migrate peers page: peer list/table with status indicators
- [ ] 2.11 Migrate settings page: theme switcher, preference toggles
- [ ] 2.12 Migrate tag components: tag badges, tag input, tag search dropdown
- [ ] 2.13 Update `theme.ts` to work with DaisyUI `data-theme` attribute
- [ ] 2.14 Delete `terminal.css` and `themes.css` (fully replaced)
- [ ] 2.15 Visual QA: verify all pages render correctly with new styling
- [ ] 2.16 Verify all HTMX interactions still work (search, pagination, navigation)

### Phase 3: HTMX to Datastar Conversion
- [ ] 3.1 Create `src/web/datastar.rs` module with `DatastarResponse` builder
- [ ] 3.2 Add `DatastarResponse` methods: `merge_fragment`, `merge_signals`, `remove_fragments`, `execute_script`
- [ ] 3.3 Implement `IntoResponse` for `DatastarResponse` (SSE format)
- [ ] 3.4 Add `MergeMode` enum (Morph, Inner, Outer, Append, Prepend, etc.)
- [ ] 3.5 Register `datastar` module in `src/web/mod.rs`
- [ ] 3.6 Update `main.ts`: replace HTMX import with Datastar SDK import
- [ ] 3.7 Remove HTMX configuration (defaultSwapStyle, historyCacheSize, etc.)
- [ ] 3.8 Convert file list search: `hx-get` → `data-on-keyup__debounce.300ms="@get(...)"`
- [ ] 3.9 Convert show-deleted toggle: `hx-trigger="change"` → `data-on-change="@get(...)"`
- [ ] 3.10 Convert pagination: `hx-get` with page param → `data-on-click="@get(...)"`
- [ ] 3.11 Convert `GET /api/files` handler to return `DatastarResponse` with fragment
- [ ] 3.12 Convert SPA navigation links: `hx-get hx-target="#main" hx-push-url` → `data-on-click="@get(...)"` + `execute_script(pushState)`
- [ ] 3.13 Convert file link navigation to Datastar
- [ ] 3.14 Convert `GET /file/{name}` to return `DatastarResponse` for Datastar requests
- [ ] 3.15 Convert `GET /edit/{hash}` to return `DatastarResponse`
- [ ] 3.16 Convert peers page auto-refresh: `hx-trigger="every 10s"` → `data-on-load__interval.10000ms`
- [ ] 3.17 Convert `GET /peers` handler to return `DatastarResponse`
- [ ] 3.18 Convert settings page to Datastar
- [ ] 3.19 Convert POST endpoints: `/api/new`, `/api/rename`, `/api/copy` to return `DatastarResponse`
- [ ] 3.20 Convert `/api/save` to return `DatastarResponse` (editor save)
- [ ] 3.21 Handle `/api/download` (non-SSE response, regular file download)
- [ ] 3.22 Convert tag REST endpoints to return `DatastarResponse` where applicable
- [ ] 3.23 Replace `htmx.ajax()` calls in main.ts with Datastar actions or custom JS
- [ ] 3.24 Replace `htmx:afterSwap` event handling (editor init on navigation)
- [ ] 3.25 Add `data-signals` to page shell for reactive state (search, page, theme, current_file)
- [ ] 3.26 Update `IdApp` API methods to use Datastar or fetch instead of htmx.ajax
- [ ] 3.27 Remove `HX-Request` header detection from all route handlers
- [ ] 3.28 Test all navigation flows: file list → editor → back, pagination, search
- [ ] 3.29 Test WebSocket collab still works (editor init after Datastar navigation)
- [ ] 3.30 Test WebSocket tags still works (real-time tag updates)

### Phase 4: Cleanup & Polish
- [ ] 4.1 Remove `htmx.org` from `package.json` dependencies
- [ ] 4.2 Delete `build-css.ts` script (replaced by Tailwind CLI)
- [ ] 4.3 Remove any remaining `hx-*` references in templates.rs
- [ ] 4.4 Remove `is_htmx_request()` or equivalent helper functions
- [ ] 4.5 Clean up unused CSS classes from input.css
- [ ] 4.6 Run full visual QA across all 3 themes and all pages
- [ ] 4.7 Test responsive behavior
- [ ] 4.8 Verify bundle sizes are reasonable
- [ ] 4.9 Update any documentation referencing HTMX
- [ ] 4.10 Final build + test: `bun run build` produces working dist/
