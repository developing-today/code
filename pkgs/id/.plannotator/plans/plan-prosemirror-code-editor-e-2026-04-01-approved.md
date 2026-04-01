# Plan: ProseMirror Code Editor Enhancements — Syntax Highlighting, Line Numbers, Word Wrap

**Status**: Draft
**Created**: 2026-04-01
**Branch**: `pretext`

## Summary

Add three code editor features to the ProseMirror-based web editor:

1. **Syntax highlighting** — Shiki via `prosemirror-highlight`, file-extension-based language detection, manual override support
2. **Line numbers** — Gutter with logical line numbers via `prosemirror-highlight`'s `withLineNumbers()`
3. **Word wrap toggle** — Toggle between `pre-wrap` (default, on) and `pre` (off), using `@chenglou/pretext` for text measurement

## Architecture

```
┌───────────────────────────────────────────────────────────────────┐
│                     NEW MODULE STRUCTURE                          │
├───────────────────────────────────────────────────────────────────┤
│                                                                   │
│  web/src/highlight.ts (NEW)                                       │
│    - detectLanguage(filename): string | undefined                 │
│    - createHighlighter(): async Shiki highlighter                 │
│    - createHighlightPlugin(filename): ProseMirror Plugin          │
│    - Line numbers via withLineNumbers() wrapper                   │
│                                                                   │
│  web/src/wrap.ts (NEW)                                            │
│    - createWrapPlugin(container): ProseMirror Plugin              │
│    - WrapState (on/off) managed via plugin state                  │
│    - pretext measurement for layout info                          │
│    - toggleWrap command                                           │
│    - Keyboard shortcut: Alt+Z (VS Code convention)                │
│                                                                   │
│  web/src/editor.ts (MODIFIED)                                     │
│    - rawSchema: add `language` attr to code_block                 │
│    - initEditor: accept filename, integrate new plugins           │
│    - Export new plugin factories for use by main.ts               │
│                                                                   │
│  web/styles/editor.css (MODIFIED)                                 │
│    - Line number gutter styles                                    │
│    - Wrap/no-wrap CSS states                                      │
│    - Syntax highlighting theme integration                        │
│                                                                   │
└───────────────────────────────────────────────────────────────────┘
```

## Key Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Syntax engine | Shiki (JS regex engine, no WASM) | 200+ languages, VS Code-quality, inline styles (no CSS theme files), lazy grammar loading |
| Line numbers | `prosemirror-highlight` `withLineNumbers()` | Already built, logical line numbers, widget decorations |
| Highlight integration | `prosemirror-highlight` | Handles decoration caching, async grammar loading, collab-compatible |
| Word wrap default | ON (`pre-wrap`) | Matches current behavior, most users expect wrapping |
| Pretext usage | Text measurement utility for wrap state | Import directly, use for layout calculations |
| Premirror | Inspiration only (not imported) | Paginated word processor architecture doesn't fit code editor |
| Language detection | Client-side from `data-filename` | Filename already in DOM, no server changes needed |
| Manual override | `language` attr on code_block node | Persisted in document, works with collab |
| Keyboard shortcut | Alt+Z for wrap toggle | VS Code convention, memorable |

## New Dependencies

| Package | Purpose | Size |
|---------|---------|------|
| `prosemirror-highlight` | Decoration management, line numbers, caching | ~55KB unpacked |
| `shiki` | Syntax highlighting engine (200+ languages) | Core ~50KB + per-grammar lazy |
| `@chenglou/pretext` | Text measurement without DOM reflow | ~642KB unpacked |

## Phase 1: Syntax Highlighting + Line Numbers

### 1a. Install dependencies

```bash
cd web && bun add prosemirror-highlight shiki @chenglou/pretext
```

### 1b. Create `web/src/highlight.ts`

**Language detection map** — Maps file extensions to Shiki language identifiers. Covers all extensions from `content_mode.rs` that map to `Raw` mode (~50 extensions).

```typescript
// Extension → Shiki language name
const EXT_TO_LANG: Record<string, string> = {
  rs: 'rust', js: 'javascript', ts: 'typescript', py: 'python',
  // ... full map covering content_mode.rs Raw extensions
}

// Special filenames (Dockerfile, Makefile, etc.)
function detectLanguage(filename: string): string | undefined

// Lazy Shiki highlighter (JS regex engine, no WASM)
async function getHighlighter(): Promise<HighlighterCore>

// Load language grammar on demand
async function ensureLanguage(lang: string): Promise<void>

// Create the prosemirror-highlight plugin
// Combines: Shiki parser + withLineNumbers wrapper
function createHighlightPlugin(filename: string | undefined): Plugin
```

**Shiki configuration:**
- Use `createHighlighterCore` from `shiki/core` (fine-grained, no bundled grammars)
- Use `createJavaScriptRegExpEngine` from `shiki/engine/javascript` (no WASM)
- Start with one dark theme (e.g., `github-dark` or similar that matches our dark UI)
- Lazy-load grammars: only load a language when first needed
- `languageExtractor`: check `node.attrs.language` first (manual override), then fall back to filename detection

### 1c. Modify `web/src/editor.ts`

- Add `language` attribute to rawSchema's `code_block` node: `attrs: { language: { default: null } }`
- Modify `initEditor` signature to accept `filename?: string`
- Add highlight plugin to raw mode's plugin list
- Add highlight plugin to rich mode (for code_block nodes within rich documents)

### 1d. Add line number + gutter CSS to `editor.css`

```css
/* Line number gutter */
.line-number {
  display: inline-block;
  width: 4ch;
  margin-right: 1ch;
  text-align: right;
  color: var(--line-number-color, oklch(0.5 0 0));
  user-select: none;
  pointer-events: none;
  font-variant-numeric: tabular-nums;
}

/* Syntax highlight background integration */
.ProseMirror pre {
  color: var(--prosemirror-highlight, inherit);
  background-color: var(--prosemirror-highlight-bg, inherit);
}
```

### 1e. Wire filename through from `main.ts`

The filename is already available via `editorContainer.dataset.filename`. Pass it through the `initEditor` call to the highlight plugin.

### 1f. Tests — `web/src/highlight.test.ts`

- `detectLanguage` returns correct language for common extensions (`.rs` → `'rust'`, `.py` → `'python'`, etc.)
- `detectLanguage` handles special filenames (`Dockerfile` → `'dockerfile'`, `Makefile` → `'makefile'`)
- `detectLanguage` returns `undefined` for unknown extensions
- `detectLanguage` is case-insensitive for extensions
- Extension map covers all Raw-mode extensions from `content_mode.rs`
- `createHighlightPlugin` returns a valid ProseMirror Plugin (without full Shiki init in unit tests — mock the highlighter)

## Phase 2: Word Wrap Toggle

### 2a. Create `web/src/wrap.ts`

```typescript
import { prepare, layout } from '@chenglou/pretext'

// Plugin state
interface WrapState { enabled: boolean }

// Plugin key for state access
const wrapPluginKey: PluginKey<WrapState>

// Toggle command
function toggleWrap(state: EditorState, dispatch?: (tr: Transaction) => void): boolean

// Create plugin — manages wrap state, applies CSS class to editor
function createWrapPlugin(defaultEnabled?: boolean): Plugin

// Pretext measurement utility
// Measures text width/height for the current editor content
// Used to set proper scroll dimensions when wrap is OFF
function measureContent(text: string, font: string, maxWidth: number): { height: number, lineCount: number }
```

**How the toggle works:**
1. Plugin maintains `enabled` boolean state (default: `true`)
2. On state change, toggles CSS class on the ProseMirror DOM element: `id-editor-wrap` (on) vs `id-editor-nowrap` (off)
3. CSS handles the actual wrapping behavior:
   - `.id-editor-wrap .ProseMirror`: `white-space: pre-wrap; overflow-wrap: break-word`
   - `.id-editor-nowrap .ProseMirror`: `white-space: pre; overflow-x: auto`
4. Pretext `prepare()` + `layout()` called when toggling to calculate content dimensions
5. Alt+Z keymap binding for toggle

### 2b. Add wrap CSS to `editor.css`

```css
/* Word wrap states */
.id-editor-wrap .ProseMirror { white-space: pre-wrap; overflow-wrap: break-word; }
.id-editor-nowrap .ProseMirror { white-space: pre; overflow-x: auto; }
```

### 2c. Integrate into `editor.ts`

- Add `createWrapPlugin()` to both raw and rich mode plugin lists
- Add `keymap({ 'Alt-z': toggleWrap })` binding
- Export `toggleWrap` for UI access (toolbar button in main.ts if desired)

### 2d. Tests — `web/src/wrap.test.ts`

- `toggleWrap` command flips wrap state
- Plugin starts with `enabled: true` by default
- Plugin applies correct CSS class based on state
- `measureContent` returns reasonable dimensions for known text
- Alt+Z keymap is wired correctly

## Phase 3: Integration & Polish

### 3a. Update `main.ts`

- Pass `filename` to `initEditor`
- Add wrap toggle button to editor header or footer (next to existing controls)
- Show current wrap state indicator

### 3b. Theme integration

- Syntax highlighting uses Shiki inline styles (no CSS theme dependency)
- Line number color uses CSS custom property that adapts to sneak/arch/mech themes
- Wrap toggle button styled consistently with existing controls

### 3c. Run quality checks

```bash
just test-web-unit      # New + existing tests pass
just test-web-typecheck  # TypeScript compiles cleanly
just build              # Full build succeeds
just check              # All quality gates pass
```

## File Change Summary

| File | Action | Description |
|------|--------|-------------|
| `web/package.json` | MODIFY | Add prosemirror-highlight, shiki, @chenglou/pretext |
| `web/src/highlight.ts` | CREATE | Language detection, Shiki setup, highlight plugin |
| `web/src/highlight.test.ts` | CREATE | Tests for language detection, plugin creation |
| `web/src/wrap.ts` | CREATE | Word wrap toggle plugin with pretext measurement |
| `web/src/wrap.test.ts` | CREATE | Tests for wrap toggle, measurement |
| `web/src/editor.ts` | MODIFY | Add language attr to rawSchema, integrate plugins, accept filename |
| `web/src/editor.test.ts` | MODIFY | Add tests for new rawSchema language attr |
| `web/src/main.ts` | MODIFY | Pass filename to initEditor, add wrap toggle UI |
| `web/styles/editor.css` | MODIFY | Line number gutter, wrap states, highlight background |

## Verification

After each phase, run:
```bash
just test-web-unit && just test-web-typecheck
```

After all phases:
```bash
just check
```


---

# Plan Feedback

I've reviewed this plan and have 1 piece of feedback:

## 1. Feedback on: "ine num"
> also allow hiding line numbers but show by default

---
