# Collaborative Cursor Enhancements

**Created**: 2026-03-20T05:30:00Z  
**Status**: Implementation Ready  
**Package**: `pkgs/id`

> **Planning Document**: See [../../.opencode/plans/2026-03-20_cursor_enhancements.md](../../../.opencode/plans/2026-03-20_cursor_enhancements.md) for implementation details and code samples.

## Overview

This document describes a comprehensive set of enhancements to the collaborative editing cursor system in the `id` web interface. The changes address cursor visibility feedback based on activity age, hover interactions, intelligent tooltip organization for overlapping cursors, and improved debug logging capabilities.

The primary goals are:

1. **Visual feedback for cursor staleness** - Cursors fade and strobe slower as they age, indicating inactive collaborators
2. **Intuitive hover interactions** - Hovering cursors reveals full details with appropriate visual emphasis
3. **Intelligent tooltip stacking** - Overlapping cursor tooltips merge intelligently based on position relationships
4. **Connection state awareness** - Cursor animations respond to WebSocket connection status
5. **Improved debugging** - Debug logging enabled by default with flexible override mechanisms

## Feature Summary

| Feature | Description | Files |
|---------|-------------|-------|
| Cursor opacity fading | 0-30s full, 30-60s fade, 60s+ at 0.3 opacity | `cursors.ts` (existing) |
| Strobe speed decay | Strobe cycle slows from 1s to 3s as cursor ages | `cursors.ts`, `editor.css` |
| Disconnect strobe stop | All strobing pauses on WebSocket disconnect | `cursors.ts`, `collab.ts` |
| Hover highlighting | 100% opacity, no strobe, bolder cursor on hover | `cursors.ts`, `editor.css` |
| Hover z-index elevation | Hovered cursor/tooltip brought to front | `cursors.ts`, `editor.css` |
| Hover restore delay | 1s delay before returning to previous state | `cursors.ts` |
| Tooltip stacking | Horizontal stacking with position-based grouping | `cursors.ts`, `editor.css` |
| Local cursor visibility | Same-line remote cursors become fully visible | `cursors.ts` |
| Reconnect cleanup | Stale cursors removed 1s after reconnect | `cursors.ts`, `collab.ts` |
| Debug logging | Default debug level with CLI/env overrides | `main.rs`, `cli.rs` |

---

## Architecture

### Cursor Lifecycle

```
┌─────────────┐     ┌─────────────┐     ┌─────────────┐     ┌─────────────┐
│   Active    │────▶│   Fading    │────▶│   Minimum   │────▶│   Hidden    │
│  (0-30s)    │     │  (30-60s)   │     │  (60s-5m)   │     │   (5m+)     │
│ opacity: 1  │     │ opacity:    │     │ opacity:0.3 │     │ opacity: 0  │
│ strobe: 1s  │     │ 1.0 → 0.3   │     │ strobe:none │     │ removed     │
└─────────────┘     │ strobe:1-3s │     └─────────────┘     └─────────────┘
                    └─────────────┘
```

### Component Relationships

```
┌─────────────────────────────────────────────────────────────┐
│                      collab.ts                               │
│  - WebSocket connection management                           │
│  - Calls setConnectionState() on connect/disconnect          │
│  - Calls markCursorFresh() on cursor updates                │
│  - Calls onInitReceived() after Init message                │
└─────────────────────────┬───────────────────────────────────┘
                          │
                          ▼
┌─────────────────────────────────────────────────────────────┐
│                      cursors.ts                              │
│  - Cursor state management (position, color, lastUpdate)    │
│  - Strobe control via CSS custom properties                 │
│  - Hover state tracking and restore timers                  │
│  - Position grouping and tooltip merging logic              │
│  - Reconnect cleanup timer                                  │
└─────────────────────────┬───────────────────────────────────┘
                          │
                          ▼
┌─────────────────────────────────────────────────────────────┐
│                     editor.css                               │
│  - CSS custom properties: --strobe-duration, --strobe-state │
│  - @keyframes cursor-strobe animation                       │
│  - Merged bar and segment styles                            │
│  - Hover state styles (.collab-cursor-hovered)              │
└─────────────────────────────────────────────────────────────┘
```

---

## Detailed Design

### 1. CSS Hybrid Strobe System

**Rationale**: Rather than using a JavaScript `requestAnimationFrame` loop to manually toggle opacity, we delegate animation to the browser via CSS. JavaScript only sets CSS custom properties, making the system more efficient and allowing smooth transitions.

**Custom Properties**:
- `--strobe-duration`: Animation cycle length (1000ms - 3000ms)
- `--strobe-state`: Animation play state (`running` or `paused`)
- `--base-opacity`: The cursor's current opacity based on age

**Duration Formula**:
```
duration_ms = 1000 + ((1.0 - opacity) / 0.7) * 2000

opacity 1.0  → 1000ms (fast strobe)
opacity 0.65 → 2000ms (medium strobe)  
opacity 0.3  → 3000ms (slow strobe)
opacity ≤0.3 → paused (no strobe)
```

**Smooth Transitions**: The `--strobe-duration` property transitions smoothly as opacity fades, so the strobe gradually slows rather than jumping between discrete speeds.

**Pause Conditions**:
- Cursor at minimum opacity (≤0.3)
- WebSocket disconnected
- Cursor is being hovered

### 2. Hover Interaction System

**Behavior**: When a user hovers over a cursor line or its tooltip:
1. Opacity immediately set to 100%
2. Strobe animation paused
3. Cursor line becomes bolder (3px instead of 2px)
4. Element brought to top z-index
5. After mouse leaves, 1s delay before restoring previous state

**Event Handling**: Uses event delegation on the editor container rather than attaching listeners to each cursor element. This handles dynamic cursor creation/removal gracefully.

**Restore Timer**: The 1s delay prevents flickering when the mouse briefly leaves and re-enters. If the user re-hovers within 1s, the timer is cancelled and the cursor stays highlighted.

### 3. Tooltip Stacking and Merging

This is the most complex feature, handling three distinct scenarios based on cursor position relationships.

#### Scenario A: Non-Overlapping Tooltips

When cursor tooltips don't visually overlap, they remain standalone with no special treatment.

```
┌─────────┐           ┌─────────┐           ┌─────────┐
│  Alice  │           │   Bob   │           │  Carol  │
└─────────┘           └─────────┘           └─────────┘
     │                     │                     │
   pos 10               pos 50                pos 100
```

#### Scenario B: Same Position (Multiple Users)

When multiple users have cursors at the **exact same document position**:
- Tooltips stack horizontally with **no dividers**
- Order: most recently active user = leftmost
- Single cursor line uses the **most recent user's color**
- Colors distinguish users (no dividers needed)

```
┌─────────────────────────────┐
│  Carol  │  Alice  │   Bob   │   ← no dividers
└─────────────────────────────┘
                │
          all at pos 50
          cursor color = Carol's (most recent)
```

#### Scenario C: Different Positions with Overlapping Tooltips

When cursors are at **different positions** but their tooltips would visually overlap:
- Tooltips merge into a single bar
- **Dividers appear between position groups** (not within)
- Groups ordered by cursor position ascending
- Within each group: most recent = leftmost

```
┌───────────────────────┬─────────┐
│  Alice  │   Bob       │  Carol  │
└───────────────────────┴─────────┘
      pos 10            │  pos 12
                        ↑
                 divider (different positions)
```

**Data Flow**:
1. Group cursors by exact document position → `PositionGroup[]`
2. Sort each group by `lastUpdate` descending (most recent first)
3. Sort groups by position ascending
4. Check if any adjacent groups' tooltips would overlap
5. If overlapping: merge into single bar with dividers between groups
6. If not overlapping: render as standalone tooltips

### 4. Local Cursor Same-Line Detection

**Goal**: When the local user's cursor is on the same rendered line as a remote cursor, that remote cursor becomes fully visible regardless of its age.

**Implementation**: Uses DOM `getBoundingClientRect()` to compare rendered Y positions. A threshold of ~5px accounts for sub-pixel rendering differences.

**Why DOM-based**: Document line numbers can differ from rendered lines due to soft wrapping. The DOM approach detects visual overlap accurately.

### 5. Reconnect Cleanup

**Problem**: After a reconnect, the client may have stale cursor data for users who disconnected while the WebSocket was down.

**Solution**:
1. On receiving `Init` message after reconnect, start a 1s timer
2. Mark all cursors received during this window as "fresh"
3. After 1s, remove any cursors not marked fresh
4. Cancel timer if WebSocket disconnects again

### 6. Debug Logging

**Default Behavior**: Debug-level logging is enabled by default for `serve`, `run`, and `repl` commands to aid development and troubleshooting.

**Override Priority** (first match wins):
1. `--debug` CLI flag → `debug`
2. `--log-level <LEVEL>` CLI flag → specified level
3. `RUST_LOG` environment variable → value
4. `LOG_LEVEL` environment variable → value
5. `DEBUG` environment variable (if truthy) → `debug`
6. Default → `debug`

---

## Key Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Strobe engine | CSS hybrid with custom properties | Browser handles animation efficiently; JS only sets properties |
| Duration transition | Smooth interpolation | Gradual slowdown feels natural vs discrete jumps |
| Same-position dividers | None | Colors distinguish users; dividers add visual noise |
| Different-position dividers | Yes | Visually separates distinct cursor locations |
| Group ordering | Position ascending | Matches document reading order; predictable |
| Within-group ordering | Activity descending | Most relevant user (most recent) is prominent |
| Cursor line color (same pos) | Most recent user | Indicates who was last active at that position |
| Line detection | DOM Y-coordinate | Accounts for soft wrapping; accurate visual overlap |

---

## File Changes Summary

| File | Changes |
|------|---------|
| `src/cli.rs` | Add `--debug` and `--log-level` global CLI flags |
| `src/main.rs` | Add `get_log_level()` function; update tracing initialization |
| `web/src/cursors.ts` | Strobe control, hover system, position grouping, merged bars, reconnect cleanup |
| `web/src/collab.ts` | Call cursor state functions on WebSocket events |
| `web/styles/editor.css` | CSS custom properties, strobe animation, merged bar styles, hover states |

---

## Testing Approach

### Manual Test Cases

1. **Strobe Decay**: Open two browsers, observe strobe at 1s cycle. Wait 30s, verify strobe slows. Wait 60s, verify strobe stops.

2. **Disconnect Behavior**: Disconnect WebSocket (e.g., disable network), verify all cursors stop strobing immediately. Reconnect, verify strobing resumes.

3. **Hover Interaction**: Hover cursor line, verify 100% opacity and no strobe. Move mouse away, verify 1s delay before fading resumes.

4. **Same-Position Stacking**: Two users place cursor at same position. Verify stacked tooltips with no dividers, cursor color = most recent.

5. **Different-Position Merging**: Two users with cursors at close but different positions. Verify merged bar with divider between groups.

6. **Local Cursor Visibility**: Remote cursor idles and fades. Move local cursor to same line. Verify remote cursor becomes fully visible.

7. **Reconnect Cleanup**: Two clients connected. Disconnect Client A. Client B reconnects. Verify Client A's cursor removed after 1s.

---

## Risks and Mitigations

| Risk | Mitigation |
|------|------------|
| CSS custom property transitions not fully supported | Graceful fallback to discrete duration steps |
| Animation duration change causes reset | Accept brief visual glitch; browser behavior varies |
| Hover events lost during decoration rebuild | Track hover state separately; re-apply after refresh |
| Position grouping with floating-point positions | Use exact equality; document positions are integers |
| Merged bar reordering causes visual jump | Add CSS transitions for segment movements |

---

## References

- **Planning Document**: [../../.opencode/plans/2026-03-20_cursor_enhancements.md](../../../.opencode/plans/2026-03-20_cursor_enhancements.md) - Contains detailed implementation code samples
- **Existing Implementation**: `web/src/cursors.ts` - Current cursor system with opacity fading
- **WebSocket Handler**: `web/src/collab.ts` - Connection state management
- **Cursor Styles**: `web/styles/editor.css` - Lines 210-260

---

## Implementation Synopsis (2026-03-20)

**Status**: ✅ Complete - All features implemented and tested

### What Was Implemented

All features described above have been fully implemented. The implementation followed the plan closely with some notable refinements discovered during development.

#### Rust CLI Changes

**Files modified:**
- `src/cli.rs` - Added global `--debug` and `--log-level` flags to the `Cli` struct
- `src/main.rs` - Added `get_log_level()` function implementing the priority chain; configured tracing to write to stderr

**Key discovery**: `tracing_subscriber::fmt()` writes to stdout by default, which interferes with command output (e.g., `id id` should print only the node ID). Solution: Use `.with_writer(std::io::stderr)` to redirect logs to stderr.

**Integration test fix**: The existing `run_cmd_success()` helper combined stdout and stderr. Created `run_cmd_success_stdout()` to capture only stdout for tests that verify exact command output.

#### TypeScript Web Changes

**Files created:**
- `web/src/cursor-utils.ts` - Pure utility functions extracted for testability
- `web/src/cursor-utils.test.ts` - 57 unit tests covering all utility functions
- `web/vitest.config.ts` - Vitest configuration with happy-dom environment

**Files modified:**
- `web/src/cursors.ts` - Full implementation of strobe, hover, same-line detection, and tooltip stacking
- `web/src/collab.ts` - Added `markCursorFresh()` call in CURSOR message handler
- `web/styles/editor.css` - CSS custom properties and animations for strobe, hover states, merged bars
- `web/package.json` - Added vitest and happy-dom dev dependencies, test scripts

#### Architectural Decision: Pure Function Extraction

To enable unit testing without a full CodeMirror environment, pure functions were extracted to `cursor-utils.ts`:

| Function | Purpose |
|----------|---------|
| `getOpacityForAge(ageSeconds)` | Calculate opacity based on cursor age |
| `getStrobeDurationMs(opacity)` | Calculate strobe speed based on opacity |
| `isLightColor(hexColor)` | Determine if color needs dark text |
| `getColorForClient(clientId)` | Generate consistent color from client ID |
| `estimateTooltipWidth(name)` | Estimate tooltip pixel width from name length |
| `groupCursorsByPosition(cursors)` | Group cursors by document position |
| `doGroupsOverlap(groups, ...)` | Check if tooltip groups would visually overlap |

### Decisions and Deviations

| Aspect | Original Plan | Implementation | Rationale |
|--------|---------------|----------------|-----------|
| Logs destination | Not specified | stderr only | Prevents interference with command stdout |
| Strobe duration property | Inline calculation | CSS custom property `--strobe-duration` | Browser handles transitions smoothly |
| Same-line detection | `coordsAtPos()` comparison | Implemented as planned | Works correctly for wrapped lines |
| Test framework | Not specified | Vitest + happy-dom | Fast, modern, native TypeScript support |
| Tooltip width estimation | Fixed assumption | `name.length * 8 + 16` formula | Accounts for padding and character width |

### Test Coverage

#### Rust Tests (296 total)
- **242 unit tests** in `src/` modules
- **54 integration tests** in `tests/cli_integration.rs`
- All tests pass with `just test`

#### TypeScript Tests (57 total)
All in `web/src/cursor-utils.test.ts`:

| Category | Count | Description |
|----------|-------|-------------|
| Constants | 3 | Verify exported constants match expected values |
| `getOpacityForAge` | 9 | Age thresholds, boundary conditions, extreme values |
| `getStrobeDurationMs` | 8 | Duration formula, edge cases, opacity thresholds |
| `isLightColor` | 8 | Light/dark detection, edge cases, invalid input |
| `getColorForClient` | 7 | Color generation, consistency, format validation |
| `estimateTooltipWidth` | 6 | Width calculation, empty/long names |
| `groupCursorsByPosition` | 10 | Position grouping, sorting, empty inputs |
| `doGroupsOverlap` | 6 | Overlap detection with configurable thresholds |

Run with: `cd web && bun test` or `just test-web-unit`

### Build & CI Commands

```bash
# Primary quality check (fix + ci)
just check

# CI-safe read-only checks
just ci

# Run all tests
just test                 # Rust tests
just test-web             # Rust tests + TypeScript type checking
just test-web-unit        # TypeScript unit tests only (57 tests)
just test-web-typecheck   # TypeScript type checking only

# Nix equivalents
nix run .#check
nix run .#test-web-unit
```

### Files Changed Summary

| File | Lines | Change Type |
|------|-------|-------------|
| `src/cli.rs` | +15 | Added debug/log-level flags, tests |
| `src/main.rs` | +45 | Added `get_log_level()`, stderr logging |
| `tests/cli_integration.rs` | +20 | Added `run_cmd_success_stdout()` helper |
| `web/src/cursor-utils.ts` | ~180 | New file - pure utility functions |
| `web/src/cursor-utils.test.ts` | ~450 | New file - 57 unit tests |
| `web/src/cursors.ts` | ~200 | Strobe, hover, stacking implementation |
| `web/src/collab.ts` | +5 | `markCursorFresh()` call |
| `web/styles/editor.css` | ~100 | CSS custom properties, animations |
| `web/package.json` | +8 | vitest, happy-dom, test scripts |
| `web/vitest.config.ts` | ~15 | New file - test configuration |
| `justfile` | +15 | New web test commands |
| `flake.nix` | +10 | New nix app entries |

### Known Limitations

1. **`nix flake check -L` in sandboxed builds**: Fails due to network isolation (can't reach crates.io/GitHub). This is expected Nix sandbox behavior, not a code issue. Evaluation succeeds; only network-dependent builds fail.

2. **CSS animation duration transitions**: Some browsers may briefly reset animation on duration change. Accepted as minor visual artifact.

3. **Tooltip width estimation**: Uses heuristic formula; may be slightly off for non-Latin characters or unusual fonts.
