# Collaborative Cursor Enhancements

**Created**: 2026-03-20T05:04:09Z  
**Status**: Planning  
**Package**: `pkgs/id`

## Overview

This document describes enhancements to the collaborative editing cursor system in the `id` web interface. The changes improve cursor visibility feedback, hover interactions, tooltip organization, and debug logging capabilities.

## Feature Matrix

| # | Feature | Files | Priority |
|---|---------|-------|----------|
| 1 | Debug logging (flags + env vars + default) | `main.rs`, `cli.rs` | High |
| 2 | Strobing slows as cursors fade (1s→3s), stops at min | `cursors.ts`, `editor.css` | High |
| 3 | Strobing stops on WebSocket disconnect | `cursors.ts` | Medium |
| 4 | Hover → 100% opacity, no strobing, cursor bolder | `cursors.ts`, `editor.css` | High |
| 5 | Hover → top z-index | `cursors.ts`, `editor.css` | Medium |
| 6 | 1s delay before restoring state after hover | `cursors.ts` | Medium |
| 7 | Horizontal tooltip stacking with merged bars | `cursors.ts`, `editor.css` | High |
| 8 | Local cursor → same-line cursors fully visible | `cursors.ts` | Medium |
| 9 | Reconnect cleanup (1s timer) | `cursors.ts`, `collab.ts` | Medium |

## Key Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Line detection | DOM rendered y-position | Most accurate for actual visual overlap |
| Merged bar hover | Hovered user's cursor + tooltip → 100% + bolder | Highlights specific user, cursor line thickens |
| Strobe engine | CSS hybrid (JS sets `--strobe-duration`, `--strobe-state`) | Browser handles animation, more efficient than RAF loop |
| Strobe transition | Smooth duration interpolation | Duration smoothly changes from 1s→3s as opacity fades |
| Log priority | flags > env vars > default | CLI flags override environment |
| Same-position cursors | Stack NO divider, cursor color = most recent | Visual clarity, recent activity emphasized |
| Overlapping different-position tooltips | Stack WITH divider between position groups | Divider distinguishes separate cursor positions |
| Within position group order | Most recent activity = leftmost | Active users more prominent |
| Between position group order | Cursor position ascending | Predictable, matches document flow |

## Implementation Order

Dependencies require this sequence:

0. **Documentation** (create comprehensive docs file per guidelines)
1. **Debug logging** (Rust, independent)
2. **Reconnect cleanup** (foundation for connection state)
3. **Strobing stops on disconnect** (uses connection state)
4. **CSS hybrid strobe setup** (JS controls CSS custom properties)
5. **Hover behavior** (opacity + no strobe + z-index + 1s delay)
6. **Local cursor same-line detection** (uses DOM line detection)
7. **Horizontal tooltip stacking** (most complex, do last)

---

## Detailed Specifications

### 1. Debug Logging

**Goal**: Enable debug-level logging by default, with multiple override mechanisms.

**Priority Order** (first match wins):
1. `--debug` flag → `debug`
2. `--log-level <LEVEL>` flag → specified level
3. `RUST_LOG` env var → value (existing behavior)
4. `LOG_LEVEL` env var → value
5. `DEBUG` env var (if truthy) → `debug`
6. Default → `debug` (changed from `info`)

**CLI Changes** (`src/cli.rs`):

Add global flags to `Cli` struct:
```rust
#[derive(Parser, Debug)]
pub struct Cli {
    /// Enable debug logging (equivalent to --log-level debug)
    #[arg(long, global = true)]
    pub debug: bool,

    /// Set log level (trace, debug, info, warn, error)
    #[arg(long, global = true, value_name = "LEVEL")]
    pub log_level: Option<String>,

    #[command(subcommand)]
    pub command: Option<Command>,
}
```

**Logging Initialization** (`src/main.rs`):

```rust
fn get_log_level(cli: &Cli) -> String {
    // 1. --debug flag
    if cli.debug {
        return "debug".to_string();
    }
    // 2. --log-level flag
    if let Some(ref level) = cli.log_level {
        return level.clone();
    }
    // 3. RUST_LOG env (signal to use EnvFilter default)
    if std::env::var("RUST_LOG").is_ok() {
        return String::new();
    }
    // 4. LOG_LEVEL env
    if let Ok(level) = std::env::var("LOG_LEVEL") {
        return level;
    }
    // 5. DEBUG env (truthy check)
    if let Ok(debug) = std::env::var("DEBUG") {
        if !debug.is_empty() && debug != "0" && debug.to_lowercase() != "false" {
            return "debug".to_string();
        }
    }
    // 6. Default
    "debug".to_string()
}
```

---

### 2. Strobing Slows as Cursors Fade

**Goal**: Cursor strobe cycle slows from 1s to 3s as opacity decreases, stops entirely at minimum opacity (≤0.3).

**Current State**:
- CSS animation: `cursor-blink 1s ease-in-out infinite`
- Animation oscillates opacity 1.0 → 0.7 → 1.0

**New Behavior (CSS Hybrid Approach)**:
- JS sets CSS custom properties on each cursor element
- Browser handles animation via CSS `animation-duration: var(--strobe-duration)`
- Strobe cycle duration varies smoothly with opacity:
  - Opacity 1.0 → 1000ms cycle
  - Opacity 0.3 → 3000ms cycle
  - Opacity ≤0.3 → No strobing (`--strobe-state: paused`)
- Formula: `duration_ms = 1000 + ((1.0 - opacity) / 0.7) * 2000`
- Duration transitions smoothly as opacity fades

**Implementation** (`web/src/cursors.ts`):

```typescript
// Strobe state tracking (for hover pause/resume)
interface StrobeInfo {
  element: HTMLElement;
  baseOpacity: number;
  paused: boolean;
}

const strobeInfos = new Map<string | number, StrobeInfo>();
let connectionState: 'connected' | 'disconnected' = 'connected';

function getStrobeDurationMs(opacity: number): number {
  if (opacity <= 0.3) return 0; // No strobing
  return 1000 + ((1.0 - opacity) / 0.7) * 2000;
}

function updateCursorStrobe(clientID: string | number, element: HTMLElement, opacity: number): void {
  const durationMs = getStrobeDurationMs(opacity);
  const shouldStrobe = durationMs > 0 && connectionState === 'connected';
  
  // Set CSS custom properties
  element.style.setProperty('--strobe-duration', `${durationMs}ms`);
  element.style.setProperty('--strobe-state', shouldStrobe ? 'running' : 'paused');
  element.style.setProperty('--base-opacity', String(opacity));
  
  strobeInfos.set(clientID, { element, baseOpacity: opacity, paused: false });
}

function pauseCursorStrobe(clientID: string | number): void {
  const info = strobeInfos.get(clientID);
  if (info) {
    info.paused = true;
    info.element.style.setProperty('--strobe-state', 'paused');
    info.element.style.opacity = '1'; // Full opacity on hover
  }
}

function resumeCursorStrobe(clientID: string | number): void {
  const info = strobeInfos.get(clientID);
  if (info && connectionState === 'connected') {
    info.paused = false;
    const durationMs = getStrobeDurationMs(info.baseOpacity);
    info.element.style.setProperty('--strobe-state', durationMs > 0 ? 'running' : 'paused');
    info.element.style.opacity = String(info.baseOpacity);
  }
}

function unregisterCursorStrobe(clientID: string | number): void {
  strobeInfos.delete(clientID);
}
```

**CSS Changes** (`web/styles/editor.css`):

```css
.collab-cursor {
  position: relative;
  border-left: 2px solid;
  margin-left: -1px;
  margin-right: -1px;
  pointer-events: auto; /* Changed from none for hover */
  
  /* CSS custom properties for JS control */
  --strobe-duration: 1000ms;
  --strobe-state: running;
  --base-opacity: 1;
  
  /* Strobe animation controlled by custom properties */
  animation: cursor-strobe var(--strobe-duration) ease-in-out infinite;
  animation-play-state: var(--strobe-state);
  
  /* Smooth transition when duration changes */
  transition: --strobe-duration 0.5s ease;
}

@keyframes cursor-strobe {
  0%, 100% { opacity: var(--base-opacity); }
  50% { opacity: calc(var(--base-opacity) * 0.7); }
}

/* When strobing is paused, ensure stable opacity */
.collab-cursor[style*="--strobe-state: paused"] {
  opacity: var(--base-opacity);
}
```

**Note**: The `transition` on `--strobe-duration` enables smooth interpolation of strobe speed as the cursor ages. If browser support is inconsistent, we can fall back to discrete duration steps.

---

### 3. Strobing Stops on WebSocket Disconnect

**Goal**: When WebSocket disconnects, all cursor strobing stops immediately.

**Implementation** (`web/src/cursors.ts`):

```typescript
export function setConnectionState(state: 'connected' | 'disconnected'): void {
  connectionState = state;
  
  // Update all cursor strobe states
  strobeInfos.forEach((info) => {
    if (state === 'disconnected') {
      // Pause strobing, set to base opacity
      info.element.style.setProperty('--strobe-state', 'paused');
      info.element.style.opacity = String(info.baseOpacity);
    } else if (!info.paused) {
      // Resume strobing (unless hover-paused)
      const durationMs = getStrobeDurationMs(info.baseOpacity);
      info.element.style.setProperty('--strobe-state', durationMs > 0 ? 'running' : 'paused');
    }
  });
}
```

---

### 4-6. Hover Behavior

**Goals**:
- Hover on cursor line OR label → 100% opacity, no strobing
- Hover on merged bar segment → highlights ONLY that user's cursor and tooltip segment
- Hovered cursor line becomes bolder (thicker border)
- Hover brings cursor group to top z-index
- 1s delay before returning to previous state after hover

**Key Behavior for Merged Bars**:
When hovering a segment in a merged bar, only that specific user's elements are highlighted:
- That segment in the bar → 100% opacity
- That user's cursor line → 100% opacity + bolder (3px instead of 2px)
- Other segments in the bar remain at their current opacity
- The bar itself elevates z-index

**Implementation** (`web/src/cursors.ts`):

```typescript
// Hover state
let hoveredCursorID: string | number | null = null;
let hoverRestoreTimer: ReturnType<typeof setTimeout> | null = null;

function setupHoverHandlers(container: HTMLElement, editorView: EditorView): void {
  // Use event delegation for cursor hover
  container.addEventListener('mouseenter', (event: MouseEvent) => {
    const target = event.target as HTMLElement;
    
    // Check if hovering a segment within a merged bar
    const segment = target.closest('.collab-cursor-bar-segment');
    const cursorEl = target.closest('.collab-cursor');
    const barEl = target.closest('.collab-cursor-bar');
    
    if (!segment && !cursorEl && !barEl) return;

    // Cancel pending restore
    if (hoverRestoreTimer) {
      clearTimeout(hoverRestoreTimer);
      hoverRestoreTimer = null;
    }

    // Determine which client ID is being hovered
    let clientID: string | number | null = null;
    
    if (segment) {
      // Hovering a specific segment in merged bar
      clientID = segment.getAttribute('data-client-id');
    } else if (cursorEl) {
      // Hovering a standalone cursor
      clientID = cursorEl.getAttribute('data-client-id');
    }
    
    if (!clientID) return;
    
    hoveredCursorID = clientID;

    // Highlight ONLY this user's cursor and tooltip
    const state = strobeStates.get(clientID);
    if (state) {
      state.paused = true;
      state.element.style.opacity = '1';
      state.element.classList.add('collab-cursor-hovered'); // Makes cursor bolder
    }
    
    // If in a merged bar, highlight only this segment
    if (segment) {
      segment.classList.add('collab-cursor-segment-hovered');
      // Elevate the entire bar's z-index
      barEl?.classList.add('collab-cursor-bar-elevated');
    }
    
    // For standalone cursor, elevate it
    if (cursorEl) {
      cursorEl.classList.add('collab-cursor-hovered');
    }
  }, true);

  container.addEventListener('mouseleave', (event: MouseEvent) => {
    const target = event.target as HTMLElement;
    const segment = target.closest('.collab-cursor-bar-segment');
    const cursorEl = target.closest('.collab-cursor');
    const barEl = target.closest('.collab-cursor-bar');
    
    if (!segment && !cursorEl && !barEl) return;

    const clientID = hoveredCursorID;
    if (!clientID) return;

    // 1s delay before restoring
    hoverRestoreTimer = setTimeout(() => {
      hoveredCursorID = null;
      
      // Restore this user's cursor
      const state = strobeStates.get(clientID);
      if (state) {
        state.paused = false;
        state.element.style.opacity = String(state.baseOpacity);
        state.element.classList.remove('collab-cursor-hovered');
      }
      
      // Remove segment highlight
      if (segment) {
        segment.classList.remove('collab-cursor-segment-hovered');
        barEl?.classList.remove('collab-cursor-bar-elevated');
      }
      
      // Remove standalone cursor highlight
      if (cursorEl) {
        cursorEl.classList.remove('collab-cursor-hovered');
      }
      
      // Refresh decorations
      editorView.dispatch(editorView.state.tr);
    }, 1000);
  }, true);
}
```

**CSS**:

```css
/* Standalone cursor hover - bolder line */
.collab-cursor-hovered {
  z-index: 1000 !important;
  border-left-width: 3px !important; /* Bolder cursor line */
}

.collab-cursor-hovered .collab-cursor-label {
  opacity: 1 !important;
}

/* Merged bar elevation */
.collab-cursor-bar-elevated {
  z-index: 1000 !important;
}

/* Individual segment hover within merged bar */
.collab-cursor-segment-hovered {
  opacity: 1 !important;
  /* Subtle highlight to show which segment is active */
  box-shadow: 0 0 0 1px rgba(255, 255, 255, 0.3);
}
```

---

### 7. Horizontal Tooltip Stacking with Merged Bars

**Goal**: 
- Tooltips stack horizontally in whitespace between lines
- Handle cursor position grouping and overlap intelligently
- Each segment matches user's cursor color

**Three Scenarios**:

**Scenario 1: Cursors don't touch → No merging, no dividers**
Each tooltip stands alone with its own color.
```
┌─────────┐     ┌─────────┐     ┌─────────┐
│  Alice  │     │   Bob   │     │  Carol  │
└─────────┘     └─────────┘     └─────────┘
   pink            blue           green
     ↑              ↑               ↑
   pos 10         pos 50          pos 100
```

**Scenario 2: Same cursor position → Stack with NO divider**
Multiple users at exact same position stack together. The cursor line uses the most recently active user's color.
```
┌─────────────────────────────┐
│  Carol  │  Alice  │   Bob   │   ← no dividers between segments
└─────────────────────────────┘
   green     pink      blue
         ↑
   all at pos 50
   cursor line color = Carol's green (most recent)
   
   Sort within group: most recent activity = leftmost
```

**Scenario 3: Different positions but tooltips overlap → Stack WITH divider between position groups**
Groups ordered by cursor position ascending. Within each group, most recent = leftmost.
```
┌───────────────────────┬─────────┐
│  Alice  │   Bob       │  Carol  │
└───────────────────────┴─────────┘
      pos 10            │  pos 12
                        ↑
                 divider here (different positions)
              
   - Left group (pos 10): Alice & Bob at same pos, Alice more recent
   - Right group (pos 12): Carol alone
   - Groups ordered by position ascending (10, then 12)
```

**Key Rules**:
| Condition | Behavior |
|-----------|----------|
| Same position | Stack, NO divider, cursor color = most recent user |
| Different position, tooltips overlap | Stack WITH divider between groups |
| Within a position group | Sort by activity (most recent = leftmost) |
| Between groups | Sort by cursor position ascending |

**Line Detection** (DOM-based):

```typescript
function getCursorLineY(element: HTMLElement): number {
  const rect = element.getBoundingClientRect();
  return rect.top;
}

function groupCursorsByRenderedLine(
  cursorElements: Map<string | number, HTMLElement>
): Map<number, (string | number)[]> {
  const LINE_THRESHOLD = 5; // pixels - cursors within this are "same line"
  const groups = new Map<number, (string | number)[]>();
  
  cursorElements.forEach((el, clientID) => {
    const y = getCursorLineY(el);
    
    // Find existing group within threshold
    let foundGroup = false;
    for (const [groupY, ids] of groups) {
      if (Math.abs(y - groupY) < LINE_THRESHOLD) {
        ids.push(clientID);
        foundGroup = true;
        break;
      }
    }
    
    if (!foundGroup) {
      groups.set(y, [clientID]);
    }
  });
  
  return groups;
}
```

**Cursor Data Structure**:

```typescript
interface CursorForMerge {
  clientID: string | number;
  head: number;        // Document position
  name: string;
  color: string;
  lastUpdate: number;  // Timestamp for activity ordering
  opacity: number;
}

// Group cursors by exact document position
interface PositionGroup {
  position: number;
  cursors: CursorForMerge[];  // Sorted by lastUpdate desc (most recent first)
  mostRecentColor: string;    // Cursor line color for this position
}
```

**Grouping and Overlap Detection**:

```typescript
function groupCursorsByPosition(cursors: CursorForMerge[]): PositionGroup[] {
  const positionMap = new Map<number, CursorForMerge[]>();
  
  // Group by exact position
  cursors.forEach(cursor => {
    const existing = positionMap.get(cursor.head) || [];
    existing.push(cursor);
    positionMap.set(cursor.head, existing);
  });
  
  // Convert to PositionGroup[], sort each group by activity
  const groups: PositionGroup[] = [];
  positionMap.forEach((groupCursors, position) => {
    // Sort by lastUpdate descending (most recent = first/leftmost)
    groupCursors.sort((a, b) => b.lastUpdate - a.lastUpdate);
    groups.push({
      position,
      cursors: groupCursors,
      mostRecentColor: groupCursors[0].color,
    });
  });
  
  // Sort groups by position ascending
  groups.sort((a, b) => a.position - b.position);
  
  return groups;
}

function doTooltipsOverlap(groups: PositionGroup[], charWidthPx: number): boolean {
  if (groups.length <= 1) return false;
  
  const LABEL_PADDING = 12;
  const MIN_GAP = 4;
  
  for (let i = 0; i < groups.length - 1; i++) {
    const curr = groups[i];
    const next = groups[i + 1];
    
    // Calculate total width of current group's tooltips
    const currWidth = curr.cursors.reduce((sum, c) => 
      sum + (c.name.length * charWidthPx) + LABEL_PADDING, 0);
    
    // Distance between positions in pixels
    const posDiff = (next.position - curr.position) * charWidthPx;
    
    if (posDiff < currWidth + MIN_GAP) {
      return true;
    }
  }
  return false;
}
```

**Merged Bar DOM Creation**:

```typescript
function createMergedBar(groups: PositionGroup[]): HTMLElement {
  // Groups already sorted by position ascending
  // Within each group, cursors sorted by activity (most recent first)
  
  const bar = document.createElement('span');
  bar.className = 'collab-cursor-bar';
  
  groups.forEach((group, groupIndex) => {
    // Add divider BETWEEN position groups (not within)
    if (groupIndex > 0) {
      const divider = document.createElement('span');
      divider.className = 'collab-cursor-bar-divider';
      bar.appendChild(divider);
    }
    
    // Add all cursors in this position group (no dividers between them)
    group.cursors.forEach((cursor) => {
      const segment = document.createElement('span');
      segment.className = 'collab-cursor-bar-segment';
      segment.style.backgroundColor = cursor.color;
      segment.style.color = isLightColor(cursor.color) ? '#000' : '#fff';
      segment.style.opacity = String(cursor.opacity);
      segment.textContent = cursor.name;
      segment.setAttribute('data-client-id', String(cursor.clientID));
      segment.setAttribute('data-position', String(cursor.head));
      bar.appendChild(segment);
    });
  });
  
  return bar;
}

// Get cursor line color for a position (most recent user's color)
function getCursorLineColor(group: PositionGroup): string {
  return group.mostRecentColor;
}
```

**CSS for Merged Bar**:

```css
.collab-cursor-bar {
  position: absolute;
  top: -1.4em;
  left: 0;
  display: flex;
  flex-direction: row;
  align-items: stretch;
  font-size: 10px;
  font-family: var(--font-mono);
  border-radius: 3px;
  overflow: hidden;
  white-space: nowrap;
  user-select: none;
  z-index: 10;
  backdrop-filter: blur(2px);
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.3);
  pointer-events: auto;
}

.collab-cursor-bar-segment {
  padding: 2px 6px;
  transition: opacity 0.3s ease;
}

.collab-cursor-bar-divider {
  width: 2px;
  background: rgba(255, 255, 255, 0.6);
  flex-shrink: 0;
}
```

---

### 8. Local Cursor Triggers Same-Line Visibility

**Goal**: Remote cursors on the same rendered line as local cursor get full opacity and resume strobing.

**Implementation**:

```typescript
function isOnSameLineAsLocal(
  remoteCursorEl: HTMLElement, 
  localSelection: Selection,
  editorView: EditorView
): boolean {
  const LINE_THRESHOLD = 5;
  
  // Get local cursor's rendered position
  const localCoords = editorView.coordsAtPos(localSelection.head);
  const localY = localCoords.top;
  
  // Get remote cursor's rendered position
  const remoteY = getCursorLineY(remoteCursorEl);
  
  return Math.abs(localY - remoteY) < LINE_THRESHOLD;
}
```

In decoration creation:
```typescript
// Check if on same line as local cursor
const isOnLocalLine = localSelection && isOnSameLineAsLocal(cursorWidget, localSelection, view);
const displayOpacity = isOnLocalLine || isHovered ? 1.0 : opacity;
```

---

### 9. Reconnect Cleanup

**Goal**: After Init message, wait 1s then remove cursors not refreshed by server.

**Implementation** (`web/src/cursors.ts`):

```typescript
let reconnectCleanupTimer: ReturnType<typeof setTimeout> | null = null;
let freshCursorIDs: Set<string | number> = new Set();
let cleanupEditorView: EditorView | null = null;

export function setCleanupEditorView(view: EditorView): void {
  cleanupEditorView = view;
}

export function onInitReceived(): void {
  if (reconnectCleanupTimer) {
    clearTimeout(reconnectCleanupTimer);
  }
  
  freshCursorIDs = new Set();
  
  reconnectCleanupTimer = setTimeout(() => {
    if (cleanupEditorView && connectionState === 'connected') {
      performReconnectCleanup();
    }
    reconnectCleanupTimer = null;
  }, 1000);
}

export function markCursorFresh(clientID: string | number): void {
  freshCursorIDs.add(clientID);
}

function performReconnectCleanup(): void {
  if (!cleanupEditorView) return;
  
  const pluginState = cursorPluginKey.getState(cleanupEditorView.state);
  if (!pluginState) return;
  
  const staleCursors: (string | number)[] = [];
  pluginState.cursors.forEach((_, clientID) => {
    if (!freshCursorIDs.has(clientID)) {
      staleCursors.push(clientID);
    }
  });
  
  staleCursors.forEach(clientID => {
    removeCursor(cleanupEditorView!, clientID);
    unregisterCursorStrobe(clientID);
  });
  
  freshCursorIDs.clear();
}

// Update setConnectionState to cancel timer on disconnect
// (Note: setConnectionState also handles strobe pause/resume - see Section 3)
export function setConnectionState(state: 'connected' | 'disconnected'): void {
  connectionState = state;
  
  // Pause/resume strobing for all cursors
  strobeInfos.forEach((info) => {
    if (state === 'disconnected') {
      info.element.style.setProperty('--strobe-state', 'paused');
      info.element.style.opacity = String(info.baseOpacity);
    } else if (!info.paused) {
      const durationMs = getStrobeDurationMs(info.baseOpacity);
      info.element.style.setProperty('--strobe-state', durationMs > 0 ? 'running' : 'paused');
    }
  });
  
  // Cancel reconnect cleanup timer on disconnect
  if (state === 'disconnected' && reconnectCleanupTimer) {
    clearTimeout(reconnectCleanupTimer);
    reconnectCleanupTimer = null;
  }
}
```

**Update** (`web/src/collab.ts`):

```typescript
// In MSG.CURSOR handler:
markCursorFresh(clientID);
updateCursor(...);

// After editor init:
setCleanupEditorView(editorInstance.view);
```

---

## Files Modified Summary

| File | Changes |
|------|---------|
| `pkgs/id/src/cli.rs` | Add `debug: bool`, `log_level: Option<String>` to `Cli` |
| `pkgs/id/src/main.rs` | Add `get_log_level()`, update tracing init, parse CLI before init |
| `pkgs/id/web/src/cursors.ts` | CSS hybrid strobe control, hover system, DOM line detection, merged bars with activity reordering, reconnect cleanup |
| `pkgs/id/web/src/collab.ts` | Call `markCursorFresh()`, `setCleanupEditorView()`, `setConnectionState()` |
| `pkgs/id/web/styles/editor.css` | CSS custom properties for strobe (`--strobe-duration`, `--strobe-state`), merged bar styles with dividers, hover states |

---

## Testing Plan

### Manual Testing

1. **Debug logging**:
   - `id serve` → debug output appears
   - `RUST_LOG=warn id serve` → warn level only
   - `id serve --debug` → debug level
   - `id serve --log-level trace` → trace level
   - `LOG_LEVEL=error id serve` → error level
   - `DEBUG=1 id serve` → debug level

2. **Strobing**:
   - Two browsers, move cursor → strobe at ~1s
   - Wait 30s → strobe slows
   - Wait 60s → strobe stops (static at 0.3)
   - Disconnect WebSocket → all strobing stops
   - Reconnect → strobing resumes

3. **Hover**:
   - Hover cursor line → 100% opacity, no strobe
   - Hover label → same behavior
   - Mouse away → 1s delay before restore
   - Re-hover within 1s → stays at 100%

4. **Z-index**:
   - Create overlapping cursors
   - Hover one → appears on top

5. **Tooltip stacking**:
   - Two cursors same position → stacked labels, NO divider, cursor color = most recent
   - Two cursors different positions but tooltips overlap → merged bar WITH divider between groups
   - Within same position group: most recent = leftmost
   - Between groups: ordered by position ascending
   - Each segment matches user color

6. **Local cursor visibility**:
   - Remote cursor idles on line
   - Move local cursor to same line → remote becomes fully visible
   - Move away → remote fades normally

7. **Reconnect cleanup**:
   - Two clients connected
   - Disconnect one, wait, reconnect
   - Stale cursor removed after 1s

---

## Risks and Mitigations

| Risk | Mitigation |
|------|------------|
| CSS custom property transitions not supported | Fall back to discrete duration steps (1s, 2s, 3s) |
| Animation duration change causes reset | Use `animation-delay` manipulation or accept brief glitch |
| DOM line detection during scroll | Use `getBoundingClientRect()` which accounts for scroll |
| Hover events lost on decoration rebuild | Track hover state separately, re-apply after refresh |
| Race between reconnect cleanup and new cursors | `markCursorFresh()` called before cleanup runs |
| Label width estimation inaccurate | Use `getBoundingClientRect()` for actual measurement |
| Merged bar reordering causes visual jump | Add CSS transition for segment position changes |

---

## Future Enhancements

- Animated transition when bars merge/split
- Touch support (long-press for hover behavior)
- Configurable timing constants via settings
- Cursor movement trails
- Keyboard navigation to jump between cursors
