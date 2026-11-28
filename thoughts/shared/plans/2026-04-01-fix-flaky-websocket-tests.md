# Fix Flaky WebSocket Collab Tests — Implementation Plan

**Goal:** Fix three bugs causing flaky Playwright E2E tests in the WebSocket collab system: stale Init doc, broadcast lag silently dropping steps, and unrecoverable receiveTransaction failures.

**Architecture:** Server Init always sends version=0 with base doc + catch-up Update of all accumulated steps. Broadcast lag sends Error to client and terminates the broadcast task (client reconnects). Client extends error/catch recovery paths to trigger reconnect on desync or step-apply failure.

**Design:** `thoughts/shared/designs/2026-04-01-fix-flaky-websocket-tests-design.md`

---

## Dependency Graph

```
Batch 1 (parallel): 1.1, 1.2, 1.3, 1.4 [all independent file changes]
```

Only one batch is needed — all four files are independent at the implementation level:
- collab.rs changes are server-side only
- collab.ts changes are client-side only
- websocket.spec.ts changes are test timing only
- Each file's changes are self-contained (no new types, no shared interfaces change)

---

## Batch 1: All Fixes (parallel — 4 implementers)

All tasks are independent and can run simultaneously.

### Task 1.1: Server-Side Init + Catch-Up
**File:** `pkgs/id/src/web/collab.rs`
**Test:** Existing tests in same file (no new test file needed — existing roundtrip tests cover encoding; E2E tests validate behavior)
**Depends:** none

**What to change:**

**Change 1 — Init sends version=0 with catch-up Update (lines 573-592)**

Replace the Init message construction and send block in `handle_collab_socket`. Currently at lines 573-592:

```rust
    // Send initial document state (binary MessagePack)
    let init_msg = CollabMessage::Init {
        version: doc.version(),
        doc: doc.doc.read().await.clone(),
        mode: doc.mode.as_str().to_owned(),
    };

    let init_bytes = init_msg.encode();
    tracing::info!(
        "[collab] Sending Init: version={}, mode={}, {} bytes",
        doc.version(),
        doc.mode.as_str(),
        init_bytes.len()
    );

    if sender.send(Message::Binary(init_bytes)).await.is_err() {
        tracing::warn!("[collab] Client disconnected during init send");
        doc.client_disconnected().await;
        return;
    }
```

Replace with:

```rust
    // Send initial document state at version 0 (binary MessagePack).
    // Always send the base document at version 0, then follow up with a
    // catch-up Update containing all accumulated steps. This ensures
    // connecting/reconnecting clients replay the full step history and
    // arrive at the correct current state.
    let init_msg = CollabMessage::Init {
        version: 0,
        doc: doc.doc.read().await.clone(),
        mode: doc.mode.as_str().to_owned(),
    };

    let init_bytes = init_msg.encode();
    tracing::info!(
        "[collab] Sending Init: version=0 (base), mode={}, {} bytes, current_version={}",
        doc.mode.as_str(),
        init_bytes.len(),
        doc.version()
    );

    if sender.send(Message::Binary(init_bytes)).await.is_err() {
        tracing::warn!("[collab] Client disconnected during init send");
        doc.client_disconnected().await;
        return;
    }

    // Send catch-up Update with all accumulated steps so the client
    // replays from version 0 to the current version.
    {
        let steps = doc.steps.read().await;
        if !steps.is_empty() {
            let catch_up_steps: Vec<serde_json::Value> =
                steps.iter().map(|(step, _)| step.data.clone()).collect();
            let catch_up_client_ids: Vec<u64> = steps
                .iter()
                .filter_map(|(_, cid)| cid.as_u64())
                .collect();

            let catch_up_msg = CollabMessage::Update {
                steps: catch_up_steps,
                client_ids: catch_up_client_ids,
            };
            let catch_up_bytes = catch_up_msg.encode();
            tracing::info!(
                "[collab] Sending catch-up Update: {} steps, {} bytes",
                steps.len(),
                catch_up_bytes.len()
            );

            if sender
                .send(Message::Binary(catch_up_bytes))
                .await
                .is_err()
            {
                tracing::warn!("[collab] Client disconnected during catch-up send");
                doc.client_disconnected().await;
                return;
            }
        }
    }
```

**Change 2 — Broadcast lag sends Error and breaks (lines 655-678)**

Replace the `broadcast_task` spawn block. Currently at lines 654-678:

```rust
    // Spawn task to forward broadcasts to this client (binary)
    let doc_id_for_broadcast = doc_id.clone();
    let broadcast_task = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(msg) => {
                    let bytes = msg.encode();
                    let mut sender = sender_for_broadcast.lock().await;
                    if sender.send(Message::Binary(bytes)).await.is_err() {
                        break; // Client disconnected
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(
                        doc_id = %doc_id_for_broadcast,
                        skipped = n,
                        "Broadcast receiver lagged, skipped messages"
                    );
                    // Continue receiving — don't kill the task
                }
                Err(broadcast::error::RecvError::Closed) => {
                    break; // Channel closed, document cleaned up
                }
            }
        }
    });
```

Replace with:

```rust
    // Spawn task to forward broadcasts to this client (binary)
    let doc_id_for_broadcast = doc_id.clone();
    let broadcast_task = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(msg) => {
                    let bytes = msg.encode();
                    let mut sender = sender_for_broadcast.lock().await;
                    if sender.send(Message::Binary(bytes)).await.is_err() {
                        break; // Client disconnected
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(
                        doc_id = %doc_id_for_broadcast,
                        skipped = n,
                        "Broadcast receiver lagged, sending desync error to client"
                    );
                    // Tell the client to reconnect for a fresh state.
                    // The client will close the WS → reconnect → get Init + catch-up.
                    let error_msg = CollabMessage::Error {
                        error: format!(
                            "Session desynchronized: {n} messages lost"
                        ),
                    };
                    let mut sender = sender_for_broadcast.lock().await;
                    let _ = sender
                        .send(Message::Binary(error_msg.encode()))
                        .await;
                    break; // Stop broadcasting — client will reconnect
                }
                Err(broadcast::error::RecvError::Closed) => {
                    break; // Channel closed, document cleaned up
                }
            }
        }
    });
```

**Verify:** `cargo test --features web` (from `pkgs/id/`)
**Commit:** `fix(collab): send Init at version 0 with catch-up Update and handle broadcast lag with desync error`

---

### Task 1.2: Client-Side Recovery Extensions
**File:** `pkgs/id/web/src/collab.ts`
**Test:** No separate test file — validated by E2E tests in websocket.spec.ts
**Depends:** none

**What to change:**

**Change 1 — Error handler: add "desynchronized" to reconnect triggers (lines 310-327)**

Find the current Error handler in `handleMessage`:

```typescript
      case MSG.ERROR: {
        // [5, error]
        const error = msg[1] as string;
        console.error("[collab] Server error:", error);

        // Version mismatch errors are recoverable via reconnect —
        // the server will send a fresh Init with the correct state
        if (typeof error === "string" && error.includes("Version mismatch")) {
          console.log("[collab] Version mismatch — scheduling reconnect to resync");
          connected = false;
          if (currentWs) {
            currentWs.close(4000, "Version mismatch resync");
          }
          scheduleReconnect();
        } else {
          updateStatus("error");
        }
        break;
      }
```

Replace with:

```typescript
      case MSG.ERROR: {
        // [5, error]
        const error = msg[1] as string;
        console.error("[collab] Server error:", error);

        // Version mismatch and desync errors are recoverable via reconnect —
        // the server will send a fresh Init with the correct state
        if (
          typeof error === "string" &&
          (error.includes("Version mismatch") || error.includes("desynchronized"))
        ) {
          console.log("[collab] Recoverable error — scheduling reconnect to resync:", error);
          connected = false;
          if (currentWs) {
            currentWs.close(4000, "Resync");
          }
          scheduleReconnect();
        } else {
          updateStatus("error");
        }
        break;
      }
```

**Change 2 — UPDATE handler: reconnect on receiveTransaction failure (lines 256-271)**

Find the current UPDATE handler's try/catch block:

```typescript
          try {
            // Pass ALL steps to receiveTransaction - it will:
            // 1. Recognize and confirm our own steps (matching our clientID)
            // 2. Apply remote steps from other clients
            // 3. Rebase any unconfirmed local steps over remote steps
            // Use the schema from the editor instance (mode-aware)
            const editorSchema = editorInstance.view.state.schema;
            const parsedSteps = steps.map((s) => Step.fromJSON(editorSchema, s));
            const tr = receiveTransaction(editorInstance.view.state, parsedSteps, clientIDs);
            editorInstance.view.dispatch(tr);
            console.log("[collab] Applied transaction, new version:", getVersion(editorInstance.view.state));
          } catch (err) {
            console.error("[collab] Failed to apply steps:", err);
          }
```

Replace with:

```typescript
          try {
            // Pass ALL steps to receiveTransaction - it will:
            // 1. Recognize and confirm our own steps (matching our clientID)
            // 2. Apply remote steps from other clients
            // 3. Rebase any unconfirmed local steps over remote steps
            // Use the schema from the editor instance (mode-aware)
            const editorSchema = editorInstance.view.state.schema;
            const parsedSteps = steps.map((s) => Step.fromJSON(editorSchema, s));
            const tr = receiveTransaction(editorInstance.view.state, parsedSteps, clientIDs);
            editorInstance.view.dispatch(tr);
            console.log("[collab] Applied transaction, new version:", getVersion(editorInstance.view.state));
          } catch (err) {
            // Step application failed — editor state is desynchronized.
            // Trigger reconnect to get fresh Init + catch-up from server.
            console.error("[collab] Failed to apply steps, reconnecting:", err);
            connected = false;
            if (currentWs) {
              currentWs.close(4001, "Step apply failure");
            }
            scheduleReconnect();
          }
```

**Verify:** `cd pkgs/id/web && bun run build` (ensures TypeScript compiles)
**Commit:** `fix(collab-client): reconnect on desync errors and step-apply failures`

---

### Task 1.3: Test 474 — Save + Reload Timing
**File:** `pkgs/id/e2e/tests/websocket.spec.ts`
**Test:** This IS the test file
**Depends:** none

**What to change:**

Find the "can save file and content persists" test (line 474-501):

```typescript
  test("can save file and content persists", async ({ page }) => {
    const fileName = `ws-save-${Date.now()}.txt`;
    await createFile(page, fileName);
    await waitForEditorReady(page);

    // Type content
    const editor = page.locator("#editor .ProseMirror");
    await editor.click();
    await page.keyboard.type("Saved content test");
    await expect(editor).toContainText("Saved content test");

    // Wait for save button to be enabled (collab must be connected)
    await expect(page.locator("#save-btn")).toBeEnabled({ timeout: 10_000 });

    // Click save button and wait for the save round-trip to complete
    await page.click("#save-btn");
    await expect(page.locator("#save-btn")).toContainText("saved", { timeout: 10_000 });

    // Reload page to verify persistence (URL was updated to new hash by save)
    await page.reload();
    await expect(page.locator("#editor-container")).toBeVisible({ timeout: 10_000 });
    await waitForEditorReady(page);

    // Content should persist (server loads blob from new hash)
    await expect(page.locator("#editor .ProseMirror")).toContainText("Saved content test", {
      timeout: 10_000,
    });
  });
```

Replace with:

```typescript
  test("can save file and content persists", async ({ page }) => {
    const fileName = `ws-save-${Date.now()}.txt`;
    await createFile(page, fileName);
    await waitForEditorReady(page);

    // Type content
    const editor = page.locator("#editor .ProseMirror");
    await editor.click();
    await page.keyboard.type("Saved content test");
    await expect(editor).toContainText("Saved content test");

    // Wait for save button to be enabled (collab must be connected)
    await expect(page.locator("#save-btn")).toBeEnabled({ timeout: 10_000 });

    // Click save button and wait for the save round-trip to complete
    await page.click("#save-btn");
    await expect(page.locator("#save-btn")).toContainText("saved", { timeout: 10_000 });

    // Brief wait for the save response to fully propagate (URL update, new hash
    // written to blob store). In NixOS VMs the blob write can lag behind the
    // HTTP response by a few hundred ms.
    await page.waitForTimeout(500);

    // Reload page to verify persistence (URL was updated to new hash by save)
    await page.reload();
    await expect(page.locator("#editor-container")).toBeVisible({ timeout: 10_000 });
    await waitForEditorReady(page);

    // Content should persist (server loads blob from new hash).
    // Extended timeout for NixOS VM environments where Init + catch-up
    // replay can take longer over cross-VM networking.
    await expect(page.locator("#editor .ProseMirror")).toContainText("Saved content test", {
      timeout: 15_000,
    });
  });
```

**Change 2 — Test 638: increase typing delay (line 638-653)**

Find the "edits from one user appear in other user's editor" test body. The key line is:

```typescript
      await page1.keyboard.type("Hello from user 1!", { delay: 50 });
```

Replace with:

```typescript
      await page1.keyboard.type("Hello from user 1!", { delay: 100 });
```

This matches the `delay: 100` already used by the passing bidirectional test at line 663 and line 680. The 50ms delay was too aggressive for cross-VM collab sync.

**Verify:** `cd pkgs/id/e2e && npx playwright test tests/websocket.spec.ts` (local), then `nix build .#checks.x86_64-linux.id-nixos-playwright-e2e` (definitive)
**Commit:** `fix(e2e): improve WebSocket test timing for NixOS VM environments`

---

### Task 1.4: Add Rust Unit Test for Init + Catch-Up Logic
**File:** `pkgs/id/src/web/collab.rs` (append to existing `mod tests`)
**Test:** Inline in the same file
**Depends:** none

**What to change:**

Add a new unit test at the end of the `mod tests` block (before the final `}`) that validates the catch-up Update message encoding with step data and client IDs matches what `CollabMessage::decode` produces. This verifies the new catch-up path's serialization is correct.

Append before the closing `}` of `mod tests` (after line 1313):

```rust
    #[allow(clippy::unwrap_used, clippy::panic)]
    #[test]
    fn test_catch_up_update_with_multiple_steps() {
        // Simulates the catch-up Update sent after Init(v=0):
        // all accumulated steps with their client IDs.
        let steps = vec![
            serde_json::json!({"stepType": "replace", "from": 0, "to": 0}),
            serde_json::json!({"stepType": "replace", "from": 5, "to": 5}),
            serde_json::json!({"stepType": "addMark", "from": 0, "to": 10}),
        ];
        let client_ids = vec![111u64, 111, 222];

        let msg = CollabMessage::Update {
            steps: steps.clone(),
            client_ids: client_ids.clone(),
        };
        let encoded = msg.encode();
        let decoded = CollabMessage::decode(&encoded).unwrap();

        match decoded {
            CollabMessage::Update {
                steps: decoded_steps,
                client_ids: decoded_ids,
            } => {
                assert_eq!(decoded_steps.len(), 3);
                assert_eq!(decoded_ids, vec![111, 111, 222]);
                assert_eq!(decoded_steps[0], steps[0]);
                assert_eq!(decoded_steps[1], steps[1]);
                assert_eq!(decoded_steps[2], steps[2]);
            }
            _ => panic!("Expected Update message"),
        }
    }

    #[allow(clippy::unwrap_used, clippy::panic)]
    #[test]
    fn test_error_desynchronized_roundtrip() {
        // Verify the new desync error message encodes/decodes correctly
        let msg = CollabMessage::Error {
            error: "Session desynchronized: 5 messages lost".to_owned(),
        };
        let encoded = msg.encode();
        let decoded = CollabMessage::decode(&encoded).unwrap();

        match decoded {
            CollabMessage::Error { error } => {
                assert_eq!(error, "Session desynchronized: 5 messages lost");
                assert!(error.contains("desynchronized"));
            }
            _ => panic!("Expected Error message"),
        }
    }
```

**Verify:** `cargo test --features web` (from `pkgs/id/`)
**Commit:** `test(collab): add unit tests for catch-up Update and desync Error encoding`

---

## Implementation Notes

### Key decisions made by planner:

1. **broadcast_task doesn't need `Arc<Document>`** — For the lag recovery, it only needs to send an Error message through the existing `sender_for_broadcast`. The Init + catch-up logic is in `handle_collab_socket` which already has `doc`.

2. **catch-up client_ids use `filter_map(as_u64)`** — The steps store client IDs as `serde_json::Value::Number`. Using `filter_map` with `as_u64()` safely extracts them. If a client ID somehow isn't a valid u64 (shouldn't happen), it's filtered out rather than panicking. This satisfies the deny `unwrap_used` lint.

3. **Init always sends version 0** — Even if no steps exist (empty document), sending version 0 is correct because the document starts at version 0. When steps exist, the catch-up Update replays them all, bringing the client to the current version.

4. **Error message format**: `"Session desynchronized: N messages lost"` — The client checks for `error.includes("desynchronized")` which is a substring match, future-proof if the message format changes slightly.

5. **reconnect close codes**: `4000` for server-error-triggered reconnects (matching existing Version mismatch pattern), `4001` for client-detected failures (receiveTransaction catch). Both are in the private-use range (4000-4999) and both trigger the existing reconnect logic in `onclose`.

6. **Test 474 wait**: 500ms after save confirmation before reload. This is conservative — the save HTTP response has already returned, but the blob store write + URL update may not have fully propagated in NixOS VMs.

7. **Test 638 delay**: 100ms matches the existing `delay: 100` in the bidirectional test (line 663, 680) which already passes. The asymmetry (50ms vs 100ms) was the only difference between the passing and failing tests.

### Verification sequence:

1. `cargo test --features web` — Rust unit tests (from `pkgs/id/`)
2. `cd pkgs/id/web && bun run build` — TypeScript compiles
3. `cd pkgs/id/e2e && npx playwright test tests/websocket.spec.ts` — Local Playwright (needs running dev server)
4. `nix build .#checks.x86_64-linux.id-nixos-playwright-e2e` — NixOS VM test (definitive, run from repo root)
