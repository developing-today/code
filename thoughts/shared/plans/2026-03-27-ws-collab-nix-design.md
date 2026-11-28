# WebSocket/Collab Fix + E2E Tests + Nix Fixes — Design Document

**Date:** 2026-03-27
**Status:** Approved via brainstorm

## Problem Statement

The `pkgs/id` project has several interconnected issues:
1. Flaky WebSocket/collab UI chain with 7 bugs (2 critical)
2. No E2E tests for WebSocket/collab/SSE features
3. `nix fmt` crashes with a runtime assertion error
4. No E2E tests in nix flake checks (offline/sandboxed mode)

## Findings

### 1. WebSocket Bug Fix Strategy

**Architecture: Connection Object Pattern**

Wrap WebSocket in a `CollabConnection` class that:
- Holds the live `ws` reference, manages lifecycle state (connecting/connected/disconnecting/closed)
- Ensures `send()` always uses the current connection
- When reconnecting, creates a new connection object rather than mutating the old one

**Reconnect policy:** Exponential backoff (1s → 30s max) with jitter and visible UI feedback (toast/banner).

**Bug fix priority:**
1. Fix `scheduleReconnect()` stale reference (connection object pattern)
2. Ensure `send()` queues/drops messages during reconnect
3. Fix `CollabConnection.ws` always pointing to live socket
4. Proper close/cleanup on component unmount
5. Handle server-initiated close codes (don't reconnect on 1000)
6. Error event handling (onerror → trigger reconnect)
7. Surface connection state to UI

**Server-side:** Fix broadcast channel `Lagged` error killing the broadcast task (collab.rs:654-662).

**Error handling:** All WS errors funnel through connection object state machine. Observable state for UI. Queue collab ops during disconnect, drop non-critical with notification.

### 2. E2E Test Plan

**Test priority (user-ranked):**
1. Disconnect/Reconnect — simulate WS drop, verify reconnection
2. WS Connect Ready — verify initial connection + ready state
3. Tag WS Live — live tag updates via WebSocket
4. Error Recovery — server errors, malformed messages
5. Editor Typing — ProseMirror input through WebSocket
6. Multi-User Collab — two browsers, real-time sync

**Structure: Hybrid (helpers + page objects)**
- Test fixtures/helpers for WS/SSE control (mock server, state assertions, message injection)
- Page object models for UI (editor page, collab status, tag panels)
- Utilities: `waitForWsConnection()`, `simulateWsDrop()`, `injectWsMessage()`, `assertReconnected()`

### 3. Nix Fmt/Check Fix

**Approach: Isolate and fix**
1. Run each of 7 formatters individually to find the crasher
2. Likely a formatter hitting binary files (PNG, etc.)
3. Update treefmt.toml excludes for binary files
4. Fix nix flake check failures separately

### 4. E2E in Nix Offline Mode

**Architecture: NixOS VM tests (2-VM split)**
- Server VM: runs app backend as systemd service
- Client VM: Playwright + browser tests against server VM
- Designed for scale-out (2-8 VMs each side for collab scenarios)

**Browser strategy:** Playwright's own browser builds (not nixpkgs), pre-fetched as fixed-output derivations for offline use. Ensures consistency with local dev.

**Test pyramid:** Unit → Integration (mocked) → E2E (real) → NixOS E2E (reproducible)

**Phasing:**
1. Core test helpers/utilities
2. Unit tests for connection logic
3. Integration tests with mock WS/SSE
4. E2E tests with Playwright (local)
5. NixOS VM test infrastructure
6. NixOS E2E flake check wiring

## Implementation Plan

### Phase A: Nix fmt fix (quick win)
- Isolate crashing formatter
- Add binary file excludes to treefmt.toml
- Verify `nix fmt` passes

### Phase B: WebSocket bug fixes (collab.ts + collab.rs)
- Refactor collab.ts: Connection Object pattern
- Fix broadcast Lagged in collab.rs
- Add exponential backoff reconnect
- Error handling improvements
- Tags WS backoff

### Phase C: E2E tests for WebSocket features
- Test helpers and page objects
- Disconnect/reconnect tests
- WS connection ready tests
- Tag WS live update tests
- Editor typing + save tests
- Multi-user collab tests

### Phase D: Nix E2E integration
- Package Playwright browsers as FODs
- NixOS VM test with 2-VM topology
- Wire into flake checks
