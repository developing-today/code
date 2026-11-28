---
date: 2026-04-02
topic: "Phase 1 Part 4: Auto-save on idle"
status: validated
---

# Auto-save on Idle

## Problem Statement

Saves are manual-only (save button + Ctrl+S). Users can lose work if they forget to save. Additionally, the save button has a bug where it stays permanently disabled after the first save (`disabled = true` is set but never re-enabled).

## Constraints

- Must respect the server's 5-second rate limit per filename (HTTP 429)
- Must not conflict with real-time collab — NewVersion from another client should cancel pending auto-save
- Client-side only — no server changes needed (rate limiter already exists)
- Must not break manual save (Ctrl+S / button click still works)
- `just id check` must pass after changes

## Approach

State-machine-driven **AutoSaveManager** in `main.ts` that hooks into the existing `editor:change` custom event (already fired on local content changes) and the `onNewVersion` collab callback. All logic stays in `main.ts`, reusing existing save infrastructure.

Rejected alternatives:
- Separate `autosave.ts` module: unnecessary complexity for ~60 lines of state management
- Server-driven auto-save (WebSocket command): adds protocol complexity, client-side debounce is simpler
- ProseMirror plugin: heavier integration, `editor:change` event already exists and is sufficient

## Architecture

### Save State Machine

```
idle ──(content change)──→ unsaved
unsaved ──(2s debounce)──→ saving
saving ──(success)──→ saved
saving ──(429)──→ rate-limited ──(retry after cooldown)──→ saving
saving ──(error)──→ error
saved ──(content change)──→ unsaved
error ──(content change)──→ unsaved
* ──(NewVersion)──→ saved (cancel all pending timers)
* ──(disconnect/close)──→ cancel all timers
```

### Constants

- `AUTOSAVE_DEBOUNCE_MS = 2000` — 2 seconds after last edit
- `RATE_LIMIT_BUFFER_MS = 500` — extra buffer added to server's retry delay

## Components

### 1. AutoSaveManager

New class/object in `main.ts`.

**State:**
- `state: 'idle' | 'unsaved' | 'saving' | 'saved' | 'rate-limited' | 'error'`
- `debounceTimer: number | null` — the 2s idle timer
- `retryTimer: number | null` — rate-limit retry timer
- `saveFn: () => Promise<SaveResult>` — reference to the save function

**Methods:**
- `onContentChange()` — clears and resets debounce timer to 2s, sets state to `unsaved`, updates indicator
- `triggerSave()` — called by debounce timer, calls saveFn, processes result
- `onSaveResult(result: SaveResult)` — transitions state based on outcome (success → saved, 429 → schedule retry, error → error)
- `onNewVersion()` — cancels debounceTimer and retryTimer, sets state to `saved`
- `saveNow()` — for manual save (Ctrl+S / button), cancels debounce and saves immediately
- `cancel()` — cancels all timers, resets state to idle
- `updateIndicator()` — updates save button text and enabled/disabled state

### 2. saveFile() Refactor

Change return type from `void` to `Promise<{ ok: boolean; retryAfterMs?: number }>`.

**Changes:**
- Remove internal button text management (AutoSaveManager handles all UI)
- On success: return `{ ok: true }`
- On 429: parse retry delay from response body ("Save rate limited. Try again in Xs."), return `{ ok: false, retryAfterMs: parsedMs }`
- On other error: return `{ ok: false }`
- Always re-enable save button in finally block (fix the disabled-forever bug)

### 3. Save Button / Indicator

Repurpose existing `#save-btn` text as the state indicator:
- `idle` → text: `"save"`, enabled
- `unsaved` → text: `"save •"` (dot indicates unsaved changes), enabled (allows manual save)
- `saving` → text: `"saving…"`, disabled
- `saved` → text: `"saved ✓"`, enabled — fades back to `"save"` after 2s
- `rate-limited` → text: `"retry…"`, disabled
- `error` → text: `"error!"`, enabled (allows manual retry) — fades back to `"save •"` after 2s (still unsaved)

### 4. Integration Wiring

In `openEditor()`:
- After editor is ready, create AutoSaveManager instance
- Add `editor:change` listener on `#editor-container` → `autoSave.onContentChange()`
- Extend `onNewVersion` callback → also call `autoSave.onNewVersion()`
- Wire save button click and Ctrl+S → `autoSave.saveNow()` (instead of calling saveFile directly)

In `closeEditor()`:
- Call `autoSave.cancel()`
- Remove event listener

## Data Flow

1. User types in ProseMirror editor
2. ProseMirror dispatches transaction with `docChanged: true`
3. `editor.ts` fires `CustomEvent("editor:change")` on the container
4. AutoSaveManager's listener calls `onContentChange()`
5. State → `unsaved`, button shows `"save •"`, debounce timer starts (2s)
6. If user types again within 2s, timer resets
7. After 2s idle, `triggerSave()` fires
8. State → `saving`, button shows `"saving…"` (disabled)
9. `saveFile()` POSTs to `/api/save`
10. Server processes save, returns new hash
11. `onSaveResult({ ok: true })` → state → `saved`, button shows `"saved ✓"`
12. After 2s, button text fades back to `"save"`

### Rate Limit Path

9b. Server returns 429 with "Save rate limited. Try again in 3s."
10b. `saveFile()` returns `{ ok: false, retryAfterMs: 3500 }` (3s + 500ms buffer)
11b. State → `rate-limited`, button shows `"retry…"` (disabled)
12b. After 3.5s, `triggerSave()` fires again automatically

### NewVersion Path

At any point, if `onNewVersion()` fires:
- Cancel debounceTimer and retryTimer
- State → `saved`, button shows `"saved ✓"`
- The existing onNewVersion handler already updates `dataset.hash`
- If user makes more edits, the cycle restarts from step 1

## Error Handling

- **Network error**: State → `error`, show `"error!"`, NO auto-retry (prevents infinite loops on network outage). User can manually retry via Ctrl+S or button click.
- **429 rate limit**: Auto-retry with server-provided delay. This is expected behavior during rapid editing, not an error.
- **Save during in-flight save**: If user hits Ctrl+S while a save is in progress, ignore (button is disabled). The auto-save will catch any changes after the current save completes.
- **Component unmount / editor close**: `cancel()` clears all timers, preventing orphaned callbacks.

## Bug Fix: Save Button Disabled Forever

Current code at `main.ts:1099` sets `saveBtn.disabled = true` and never re-enables. Fix: add `saveBtn.disabled = false` in the completion path of `saveFile()`, regardless of success/failure. The AutoSaveManager also manages disabled state based on save state, providing a second layer of correctness.

## Testing Strategy

### E2E Tests (Playwright)

1. **Auto-save triggers after 2s idle**: Type text, wait 3s, intercept network to verify `/api/save` was called
2. **Debounce works — rapid edits produce single save**: Type rapidly for 3s, verify only 1 save request fires (after 2s idle)
3. **Rate limit retry**: Intercept `/api/save` to return 429 first, then 200 — verify two requests made
4. **NewVersion cancels pending save**: Type text, send mock NewVersion via WebSocket before 2s, verify no save request

### Manual Verification (chrome-devtools)

- Open editor, type, observe button state transitions
- Verify "save •" appears after typing
- Verify "saving…" → "saved ✓" after 2s idle
- Verify Ctrl+S still works for immediate save

## Open Questions

None — all requirements are clear from the roadmap plan.
