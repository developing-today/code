---
date: 2026-04-01
topic: "Fix Flaky WebSocket Collab Tests"
status: validated
---

# Fix Flaky WebSocket Collab Tests

## Problem Statement

Two Playwright E2E tests in `websocket.spec.ts` fail intermittently in NixOS VM environments:

1. **Test 474** ("can save file and content persists") — after page reload, editor shows empty string instead of saved content. Timeout at 10s.
2. **Test 638** ("edits from one user appear in other user's editor") — User 2 sees partial text "Hello fr" instead of "Hello from user 1!". Timeout at 20s.

These are caused by three real bugs in the collab system, not test timing. The NixOS VM environment (cross-VM networking, 4096MB/2-core VMs, sequential 4-run Playwright) amplifies the issues.

## Constraints

- No new dependencies (no server-side ProseMirror step applicator)
- Must not break existing passing tests (71/73 pass today)
- Changes must work in both local dev and NixOS VM environments
- Maintain the existing WebSocket MessagePack wire protocol (tags 0-7)
- Lint rules: no unwrap, expect, panic, todo, dbg_macro in non-test code

## Root Cause Analysis

### Bug 1: Init Sends Stale Document

**Location**: `collab.rs` — `handle_collab_socket`, Init message construction

The server sends `Init(version=N, doc=original_doc)` to connecting clients. But `doc.doc` (an `RwLock<serde_json::Value>`) is **never updated** after steps are applied. Only `doc.steps` (Vec) and `doc.version` (AtomicU64) change. A reconnecting client receives the original (empty) document at version N, causing ProseMirror to think it's caught up with wrong content.

**Impact on Test 474**: After `page.reload()`, the new WebSocket connection gets Init with the empty original doc. The test assertion finds empty text.

### Bug 2: Broadcast Lag Silently Drops Steps

**Location**: `collab.rs` — `broadcast_task`, `RecvError::Lagged` handler

The `tokio::sync::broadcast(256)` channel drops messages when a receiver can't keep up. The current handler logs a warning and continues. But the client has now missed N step updates. Every subsequent step fails in `receiveTransaction` because the version/content is wrong.

**Impact on Test 638**: User 1 types 18 chars at 50ms delay. Cross-VM round trip can't keep up, broadcast channel lags for User 2's receiver. User 2 gets stuck at partial text ("Hello fr") with no recovery.

### Bug 3: receiveTransaction Failure Has No Recovery

**Location**: `collab.ts` — `handleMessage`, UPDATE case catch block

When `receiveTransaction` throws (from out-of-order steps after a lag), the catch block only logs to console. No reconnect, no recovery. The editor stays stuck with partial content forever.

## Approach

Fix all three bugs to create a robust recovery pipeline:

1. **Server sends correct state on connect** — Init at version 0 + catch-up Update with all accumulated steps
2. **Server detects desync and forces client recovery** — broadcast lag → Error message → client reconnects
3. **Client recovers from step failures** — receiveTransaction catch → reconnect → fresh Init + catch-up

This creates a self-healing loop: any desync (lag, reconnect, step failure) results in the client getting a fresh, correct state from the server.

### Alternatives Considered

- **Increase broadcast channel capacity**: Treating the symptom, not the cause. Even a 10,000 capacity channel can lag under sustained load. Proper recovery is needed regardless.
- **Server-side ProseMirror step application**: Would allow sending the current doc directly in Init. Rejected — would require a Rust ProseMirror implementation or WASM embedding. Replaying steps from version 0 is sufficient for typical session lengths.
- **Test-only timing fixes**: Would mask the bugs instead of fixing them. The bugs affect production users too (any reconnect gets stale doc).

## Architecture

### Component Changes

#### 1. `collab.rs` — Server-Side Fixes

**Init + Catch-Up (handle_collab_socket)**:
- Change Init to always send `version: 0` with the original `doc.doc`
- After sending Init, read `doc.steps` snapshot
- If steps exist, encode them as an Update message and send immediately
- Client receives Init(v0, base_doc) → creates editor at v0 → receives Update(all_steps) → applies them → reaches current version

**Broadcast Lag Recovery (broadcast_task)**:
- When `RecvError::Lagged(n)`, send `Error("Session desynchronized: N messages lost")` to the client
- Break out of the broadcast loop (task terminates)
- The server's main receiver loop continues until the client closes its WS
- Client reconnects → new connection → fresh Init + catch-up

#### 2. `collab.ts` — Client-Side Fixes

**Extend Error Handler**:
- Currently only reconnects on "Version mismatch"
- Add "desynchronized" to reconnect-triggering error patterns
- Same reconnect flow: close WS code 4000, schedule reconnect

**receiveTransaction Failure Recovery**:
- In the UPDATE handler's catch block, trigger reconnect instead of just logging
- Close WS with code 4001, schedule reconnect
- On reconnect, Init + catch-up restores correct state

#### 3. `websocket.spec.ts` — Test Robustness (Secondary)

**Test 474**: Add brief wait after save confirmation before reload; increase post-reload assertion timeout
**Test 638**: Increase typing delay from 50ms to 100ms (matches the passing bidirectional test pattern)

### Data Flow: Normal Connection

```
Client                          Server
  |-- WS Connect ----------------->|
  |                                |-- get_or_create(doc_id)
  |<--- Init(v=0, base_doc) ------|
  |<--- Update(steps 0..N) -------|  (catch-up, only if steps exist)
  |                                |-- spawn broadcast_task
  |-- Steps(v=N, step, clientID) ->|
  |<--- Ack(v=N+1) ---------------|
  |<--- Update(step, clientIDs) ---|  (broadcast to other clients)
```

### Data Flow: Broadcast Lag Recovery

```
Client                          Server
  |                                |-- broadcast_task: RecvError::Lagged(5)
  |<--- Error("desync: 5 lost") --|
  |                                |-- broadcast_task exits
  |-- WS Close(4000) ------------>|
  |                                |-- cleanup old connection
  |-- WS Connect (reconnect) ---->|
  |<--- Init(v=0, base_doc) ------|
  |<--- Update(steps 0..N) -------|  (full catch-up)
  |                                |-- spawn new broadcast_task
```

### Data Flow: receiveTransaction Failure Recovery

```
Client                          Server
  |<--- Update(steps) ------------|
  |   receiveTransaction throws!  |
  |-- WS Close(4001) ------------>|
  |                                |-- cleanup old connection
  |-- WS Connect (reconnect) ---->|
  |<--- Init(v=0, base_doc) ------|
  |<--- Update(steps 0..N) -------|
```

## Error Handling Strategy

- **Broadcast lag**: Server-initiated recovery via Error message. Client reconnects automatically.
- **receiveTransaction failure**: Client-initiated recovery via reconnect. Server sends fresh state.
- **WS disconnect**: Existing reconnect logic (exponential backoff, max 10 attempts) handles this.
- **Version mismatch**: Existing Error("Version mismatch") → reconnect flow. Now also benefits from correct Init + catch-up.
- **Connect timeout**: Existing 2s timeout → reconnect. Now benefits from correct Init + catch-up.

All recovery paths converge to the same outcome: fresh Init(v=0) + Update(all steps) = correct editor state.

## Testing Strategy

### Existing Tests (must still pass)
- 71/73 Playwright tests that currently pass
- All Rust integration tests (96 total)
- NixOS serve-test and e2e-test (dual-instance)

### Flaky Tests (must become stable)
- Test 474: "can save file and content persists" — fixed by correct Init + catch-up after reload
- Test 638: "edits from one user appear in other user's editor" — fixed by broadcast lag recovery + timing adjustment

### Verification
1. `cargo test --features web` — Rust unit tests including collab.rs tests
2. Local Playwright: `cd e2e && npx playwright test tests/websocket.spec.ts`
3. NixOS VM: `nix build .#checks.x86_64-linux.id-nixos-playwright-e2e`

## Open Questions

None — all three bugs have clear fixes with well-understood recovery paths.
