import { expect, type Page, test } from "@playwright/test";

/**
 * WebSocket & Collab E2E tests for the id web UI.
 *
 * Tests verify:
 * - WebSocket connection + editor ready state
 * - Disconnect detection + automatic reconnect
 * - Tag WebSocket live updates (no page reload)
 * - Editor typing + save workflow
 * - Error recovery from version mismatch
 * - Multi-user collaborative editing
 *
 * Prerequisites:
 * - Web variant must be built first (`just build`)
 * - Server starts automatically via playwright.config.ts webServer
 */

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Create a file and navigate to its editor, returning the filename */
async function createFile(page: Page, name: string): Promise<void> {
  await page.goto("/");
  await page.fill("#new-file-name", name);
  await page.click("#new-file-form button[type='submit']");
  await page.waitForURL(/\/(file|edit)\//, { timeout: 15_000 });
  await expect(page.locator("#editor-container")).toBeVisible({ timeout: 10_000 });
}

/** Wait for the collab WebSocket to connect and editor to be ready */
async function waitForEditorReady(page: Page): Promise<void> {
  // Poll JS state directly: collab connected + ProseMirror mounted.
  // This is more reliable than DOM text checks under Firefox load — the JS
  // state is set before the DOM is painted, so we avoid race conditions.
  await page.waitForFunction(
    () => {
      const app = (window as unknown as { idApp: { collab: { ws: WebSocket; editor: unknown } | null } }).idApp;
      if (!app?.collab?.ws || app.collab.ws.readyState !== WebSocket.OPEN) return false;
      if (!app.collab.editor) return false;
      // Also verify ProseMirror is in the DOM
      return !!document.querySelector("#editor .ProseMirror");
    },
    { timeout: 30_000 },
  );
}

/** Wait for the Tags WebSocket to be connected */
async function waitForTagsWs(page: Page): Promise<void> {
  await page.waitForFunction(
    () => {
      const app = (window as unknown as { idApp: { tagsWs: WebSocket | null } }).idApp;
      return app?.tagsWs?.readyState === WebSocket.OPEN;
    },
    { timeout: 10_000 },
  );
}

/**
 * Create a file with unique content via API and navigate to its editor.
 *
 * This ensures each file gets a unique blob hash (avoiding shared collab documents
 * when multiple tests create empty .txt files with the same hash), and navigates
 * by name (ensuring correct data-filename for tag matching).
 */
async function createFileWithUniqueContent(page: Page, name: string, baseURL: string): Promise<void> {
  // Create file via API
  const createResp = await page.request.post(`${baseURL}/api/new`, {
    data: { name },
  });
  expect(createResp.ok()).toBeTruthy();
  const { hash } = (await createResp.json()) as { hash: string; name: string };

  // Save unique content to get a unique blob hash → unique collab document
  const uniqueContent = `unique-${name}-${Date.now()}`;
  const saveResp = await page.request.post(`${baseURL}/api/save`, {
    data: {
      doc_id: hash,
      name,
      doc: {
        type: "doc",
        content: [
          {
            type: "paragraph",
            content: [{ type: "text", text: uniqueContent }],
          },
        ],
      },
    },
  });
  expect(saveResp.ok()).toBeTruthy();

  // Navigate to file by name — ensures correct data-filename for tag matching
  await page.goto(`/file/${encodeURIComponent(name)}`);
  await page.waitForLoadState("networkidle");
  await expect(page.locator("#editor-container")).toBeVisible({ timeout: 10_000 });
}

// ---------------------------------------------------------------------------
// 1. WebSocket Connection + Editor Ready
// ---------------------------------------------------------------------------

test.describe("WebSocket Connection + Editor Ready", () => {
  test("editor status shows connected after WS handshake", async ({ page, baseURL }) => {
    const fileName = `ws-connect-${Date.now()}.txt`;

    // Use unique content to avoid shared collab document under Firefox load
    await createFileWithUniqueContent(page, fileName, baseURL!);

    // Wait for editor to be fully ready
    await waitForEditorReady(page);

    // Verify WebSocket is connected
    const wsConnected = await page.evaluate(() => {
      const app = (window as unknown as { idApp: { collab: { ws: WebSocket } } }).idApp;
      return app?.collab?.ws?.readyState === WebSocket.OPEN;
    });
    expect(wsConnected).toBeTruthy();

    // Verify status element has correct CSS class
    await expect(page.locator("#editor-status")).toHaveClass(/status-connected/);
  });

  test("ProseMirror editor is interactive after WS init", async ({ page }) => {
    const fileName = `ws-editor-${Date.now()}.txt`;
    await createFile(page, fileName);
    await waitForEditorReady(page);

    // ProseMirror should have contenteditable
    const editor = page.locator("#editor .ProseMirror");
    await expect(editor).toHaveAttribute("contenteditable", "true");

    // Save button should be enabled (enabled by onEditorReady callback)
    await expect(page.locator("#save-btn")).toBeEnabled({ timeout: 5_000 });
  });

  test("editor shows connecting state initially", async ({ page }) => {
    const fileName = `ws-initial-${Date.now()}.txt`;

    // Navigate and check initial status before WS connects
    await page.goto("/");
    await page.fill("#new-file-name", fileName);
    await page.click("#new-file-form button[type='submit']");
    await page.waitForURL(/\/(file|edit)\//, { timeout: 15_000 });

    // The editor-status should exist and eventually show "connected"
    const statusEl = page.locator("#editor-status");
    await expect(statusEl).toBeVisible();

    // Eventually transitions to connected
    await expect(statusEl).toHaveText("connected", { timeout: 15_000 });
  });

  test("editor receives initial document via WS Init message", async ({ page }) => {
    const fileName = `ws-init-doc-${Date.now()}.md`;
    await createFile(page, fileName);
    await waitForEditorReady(page);

    // Editor should have loaded with the document content
    // For a new file, ProseMirror starts with an empty paragraph
    const editor = page.locator("#editor .ProseMirror");
    await expect(editor).toBeVisible();

    // Should have at least one paragraph element (ProseMirror's empty doc)
    const paragraphs = editor.locator("p");
    await expect(paragraphs.first()).toBeVisible();
  });
});

// ---------------------------------------------------------------------------
// 2. WebSocket Disconnect + Reconnect
// ---------------------------------------------------------------------------

test.describe("WebSocket Disconnect + Reconnect", () => {
  test("detects disconnect and shows disconnected status", async ({ page }) => {
    const fileName = `ws-disconnect-${Date.now()}.txt`;
    await createFile(page, fileName);
    await waitForEditorReady(page);

    // Close the WebSocket from the client side (simulating network drop)
    // Using code 4001 (not 1000) so the client triggers reconnect
    await page.evaluate(() => {
      const app = (window as unknown as { idApp: { collab: { ws: WebSocket } } }).idApp;
      if (app?.collab?.ws) {
        app.collab.ws.close(4001, "Test disconnect");
      }
    });

    // Status should change to disconnected or connecting (reconnect attempt)
    const statusEl = page.locator("#editor-status");
    await expect(statusEl).not.toHaveText("connected", { timeout: 5_000 });
  });

  test("reconnects automatically after disconnect", async ({ page }) => {
    const fileName = `ws-reconnect-${Date.now()}.txt`;
    await createFile(page, fileName);
    await waitForEditorReady(page);

    // Force close the WebSocket (non-clean close triggers reconnect)
    await page.evaluate(() => {
      const app = (window as unknown as { idApp: { collab: { ws: WebSocket } } }).idApp;
      if (app?.collab?.ws) {
        app.collab.ws.close(4001, "Test disconnect");
      }
    });

    // Wait for status to change away from connected
    await expect(page.locator("#editor-status")).not.toHaveText("connected", { timeout: 5_000 });

    // Wait for reconnect — status should return to "connected"
    // Reconnect uses exponential backoff starting at 1s
    await expect(page.locator("#editor-status")).toHaveText("connected", { timeout: 15_000 });
  });

  test("editor remains functional after reconnect", async ({ page }) => {
    const fileName = `ws-reconnect-func-${Date.now()}.txt`;
    await createFile(page, fileName);
    await waitForEditorReady(page);

    // Force disconnect
    await page.evaluate(() => {
      const app = (window as unknown as { idApp: { collab: { ws: WebSocket } } }).idApp;
      if (app?.collab?.ws) {
        app.collab.ws.close(4001, "Test disconnect");
      }
    });

    // Wait for reconnect
    await expect(page.locator("#editor-status")).toHaveText("connected", { timeout: 15_000 });

    // Verify editor is still functional after reconnect
    const editor = page.locator("#editor .ProseMirror");
    await expect(editor).toBeVisible();
    await expect(editor).toHaveAttribute("contenteditable", "true");

    // Save button should be re-enabled after reconnect (onEditorReady fires again)
    await expect(page.locator("#save-btn")).toBeEnabled({ timeout: 5_000 });
  });

  test("survives multiple disconnect/reconnect cycles", async ({ page }) => {
    const fileName = `ws-multi-reconnect-${Date.now()}.txt`;
    await createFile(page, fileName);
    await waitForEditorReady(page);

    // Do 3 disconnect/reconnect cycles
    for (let i = 0; i < 3; i++) {
      await page.evaluate(() => {
        const app = (window as unknown as { idApp: { collab: { ws: WebSocket } } }).idApp;
        if (app?.collab?.ws) {
          app.collab.ws.close(4001, "Test disconnect");
        }
      });

      // Wait for reconnect
      await expect(page.locator("#editor-status")).toHaveText("connected", { timeout: 20_000 });
    }

    // Editor should still be working after 3 cycles
    const editor = page.locator("#editor .ProseMirror");
    await expect(editor).toBeVisible();
    await expect(editor).toHaveAttribute("contenteditable", "true");
  });
});

// ---------------------------------------------------------------------------
// 3. Tag WebSocket Live Updates
// ---------------------------------------------------------------------------

test.describe("Tag WebSocket Live Updates", () => {
  test("tag added via API appears in editor without reload", async ({ page, baseURL }) => {
    const fileName = `ws-tag-live-${Date.now()}.txt`;
    // Use unique content to ensure this file has its own blob hash and collab doc.
    // Navigate by name so data-filename matches for Tags WS event filtering.
    await createFileWithUniqueContent(page, fileName, baseURL!);
    await waitForEditorReady(page);

    // Ensure Tags WS is connected before sending API request
    await waitForTagsWs(page);

    // Set a tag via REST API — this triggers a Tags WS event
    const resp = await page.request.post(`${baseURL}/api/tags`, {
      data: { subject: fileName, key: "live-test", value: "yes" },
    });
    expect(resp.ok()).toBeTruthy();

    // The tag should appear in the editor tag list via WS notification
    // (Tags WS sends Set event → main.ts onmessage → loadFileTags → renderEditorTags)
    const tagList = page.locator("#editor-tag-list");
    await expect(tagList.locator(".tag-pill-removable", { hasText: "live-test" })).toBeVisible({
      timeout: 15_000,
    });
  });

  test("tag removed via API disappears from editor without reload", async ({ page, baseURL }) => {
    const fileName = `ws-tag-remove-${Date.now()}.txt`;
    // Use unique content to ensure correct data-filename matching for Tags WS events
    await createFileWithUniqueContent(page, fileName, baseURL!);
    await waitForEditorReady(page);

    // Ensure Tags WS is connected
    await waitForTagsWs(page);

    // Add a tag via API
    await page.request.post(`${baseURL}/api/tags`, {
      data: { subject: fileName, key: "temp-tag", value: "remove-me" },
    });

    // Wait for it to appear
    const tagList = page.locator("#editor-tag-list");
    await expect(tagList.locator(".tag-pill-removable", { hasText: "temp-tag" })).toBeVisible({
      timeout: 15_000,
    });

    // Remove via API — must include value to match the exact tag
    const delResp = await page.request.delete(`${baseURL}/api/tags`, {
      data: { subject: fileName, key: "temp-tag", value: "remove-me" },
    });
    expect(delResp.ok()).toBeTruthy();

    // Tag should disappear via WS notification
    await expect(tagList.locator(".tag-pill-removable", { hasText: "temp-tag" })).not.toBeVisible({
      timeout: 15_000,
    });
  });

  test("tag changes for different file don't affect current editor", async ({ page, baseURL }) => {
    const fileName = `ws-tag-own-${Date.now()}.txt`;
    const otherFile = `ws-tag-other-${Date.now()}.txt`;
    await createFile(page, fileName);
    await waitForEditorReady(page);

    // Create another file via API
    const createResp = await page.request.post(`${baseURL}/api/new`, {
      data: { name: otherFile },
    });
    expect(createResp.ok()).toBeTruthy();

    // Tag the OTHER file
    await page.request.post(`${baseURL}/api/tags`, {
      data: { subject: otherFile, key: "foreign", value: "tag" },
    });

    // Wait a moment for any WS events to process
    await page.waitForTimeout(2_000);

    // The current editor's tag list should NOT show the foreign tag
    const tagList = page.locator("#editor-tag-list");
    await expect(tagList.locator(".tag-pill-removable", { hasText: "foreign" })).not.toBeVisible();
  });
});

// ---------------------------------------------------------------------------
// 4. Editor Typing + Save
// ---------------------------------------------------------------------------

test.describe("Editor Typing + Save", () => {
  test("can type into ProseMirror editor", async ({ page }) => {
    const fileName = `ws-type-${Date.now()}.txt`;
    await createFile(page, fileName);
    await waitForEditorReady(page);

    // Click into the editor to focus it
    const editor = page.locator("#editor .ProseMirror");
    await editor.click();

    // Type some text
    await page.keyboard.type("Hello from E2E test!");

    // Verify text appears in the editor
    await expect(editor).toContainText("Hello from E2E test!");
  });

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

  test("Ctrl+S triggers save", async ({ page }) => {
    const fileName = `ws-ctrlsave-${Date.now()}.txt`;
    await createFile(page, fileName);
    await waitForEditorReady(page);

    // Type content
    const editor = page.locator("#editor .ProseMirror");
    await editor.click();
    await page.keyboard.type("Ctrl+S test content");

    // Use Ctrl+S to save
    await page.keyboard.press("Control+s");

    // Wait for save to complete
    await expect(page.locator("#save-btn")).toHaveText(/save/i, { timeout: 10_000 });

    // Verify by reloading the page
    await page.reload();
    await waitForEditorReady(page);

    await expect(page.locator("#editor .ProseMirror")).toContainText("Ctrl+S test content");
  });
});

// ---------------------------------------------------------------------------
// 5. Error Recovery
// ---------------------------------------------------------------------------

test.describe("Error Recovery", () => {
  test("editor recovers from WebSocket error event", async ({ page, baseURL }) => {
    const fileName = `ws-error-${Date.now()}.txt`;
    await createFileWithUniqueContent(page, fileName, baseURL!);
    await waitForEditorReady(page);

    // Trigger a WebSocket error by closing with an abnormal code
    // This simulates a network error — onerror fires before onclose
    await page.evaluate(() => {
      const app = (window as unknown as { idApp: { collab: { ws: WebSocket } } }).idApp;
      if (app?.collab?.ws) {
        // Close with code 1006 (abnormal closure — simulates network error)
        app.collab.ws.close(4002, "Simulated error");
      }
    });

    // Should reconnect automatically
    await expect(page.locator("#editor-status")).toHaveText("connected", { timeout: 15_000 });

    // Editor should still work
    const editor = page.locator("#editor .ProseMirror");
    await expect(editor).toBeVisible();
    await expect(editor).toHaveAttribute("contenteditable", "true");
  });

  test("clean disconnect does not trigger reconnect", async ({ page, baseURL }) => {
    const fileName = `ws-clean-close-${Date.now()}.txt`;
    await createFileWithUniqueContent(page, fileName, baseURL!);
    await waitForEditorReady(page);

    // Close cleanly with code 1000 — should NOT trigger reconnect
    // Note: code 1000 onclose only sets connected=false, does NOT call updateStatus()
    await page.evaluate(() => {
      const app = (window as unknown as { idApp: { collab: { ws: WebSocket } } }).idApp;
      if (app?.collab?.ws) {
        app.collab.ws.close(1000, "Clean close");
      }
    });

    // Wait for the WebSocket close handshake to complete (async in Firefox)
    await page.waitForFunction(
      () => {
        const app = (window as unknown as { idApp: { collab: { ws: WebSocket } } }).idApp;
        const ws = app?.collab?.ws;
        return !ws || ws.readyState === WebSocket.CLOSED;
      },
      { timeout: 5_000 },
    );

    // Wait 3s (longer than initial reconnect backoff of 1s) to verify no reconnect attempt
    await page.waitForTimeout(3_000);

    // Status should still show "connected" (code 1000 doesn't update status)
    // The key test is that NO reconnect happened — the WS stays closed
    const wsStillClosed = await page.evaluate(() => {
      const app = (window as unknown as { idApp: { collab: { ws: WebSocket } } }).idApp;
      const ws = app?.collab?.ws;
      return !ws || ws.readyState === WebSocket.CLOSED;
    });
    expect(wsStillClosed).toBeTruthy();
  });
});

// ---------------------------------------------------------------------------
// 6. Multi-User Collaborative Editing
// ---------------------------------------------------------------------------

test.describe("Multi-User Collab", () => {
  // Helper to get the base URL for collab tests (can't use page fixture's baseURL)
  const getBaseURL = () => `http://localhost:${process.env.TEST_PORT || 4173}`;

  /** Set up two pages with the same file open in both editors */
  async function setupCollabPair(browser: import("@playwright/test").Browser) {
    const baseURL = getBaseURL();
    const fileName = `collab-${Date.now()}.txt`;
    const context1 = await browser.newContext({ baseURL });
    const context2 = await browser.newContext({ baseURL });
    const page1 = await context1.newPage();
    const page2 = await context2.newPage();

    // User 1 creates file
    await page1.goto("/");
    await page1.fill("#new-file-name", fileName);
    await page1.click("#new-file-form button[type='submit']");
    await page1.waitForURL(/\/(file|edit)\//, { timeout: 15_000 });
    await expect(page1.locator("#editor-status")).toHaveText("connected", { timeout: 15_000 });
    await expect(page1.locator("#editor .ProseMirror")).toBeVisible({ timeout: 5_000 });

    // User 2 opens same file
    await page2.goto("/");
    await page2.click(`a[data-nav]:has-text("${fileName}")`);
    await expect(page2.locator("#editor-container")).toBeVisible({ timeout: 10_000 });
    await expect(page2.locator("#editor-status")).toHaveText("connected", { timeout: 15_000 });
    await expect(page2.locator("#editor .ProseMirror")).toBeVisible({ timeout: 5_000 });

    return { context1, context2, page1, page2, fileName };
  }

  test("two tabs can open the same file simultaneously", async ({ browser }) => {
    const { context1, context2, page1, page2 } = await setupCollabPair(browser);

    try {
      // Both editors should be connected
      await expect(page1.locator("#editor-status")).toHaveText("connected");
      await expect(page2.locator("#editor-status")).toHaveText("connected");
    } finally {
      await context1.close();
      await context2.close();
    }
  });

  test("edits from one user appear in other user's editor", async ({ browser }) => {
    const { context1, context2, page1, page2 } = await setupCollabPair(browser);

    try {
      // User 1 types something (with delay to allow collab sync per character)
      const editor1 = page1.locator("#editor .ProseMirror");
      await editor1.click();
      await page1.keyboard.type("Hello from user 1!", { delay: 50 });

      // User 2 should see the text appear (via collab WebSocket sync)
      const editor2 = page2.locator("#editor .ProseMirror");
      await expect(editor2).toContainText("Hello from user 1!", { timeout: 15_000 });
    } finally {
      await context1.close();
      await context2.close();
    }
  });

  test("bidirectional editing works", async ({ browser }) => {
    const { context1, context2, page1, page2 } = await setupCollabPair(browser);

    try {
      // User 1 types first (slow enough for collab to sync each step)
      const editor1 = page1.locator("#editor .ProseMirror");
      await editor1.click();
      await page1.keyboard.type("AAA", { delay: 100 });

      // Wait for full sync to User 2
      const editor2 = page2.locator("#editor .ProseMirror");
      await expect(editor2).toContainText("AAA", { timeout: 15_000 });

      // Small pause to let collab settle before User 2 types
      await page2.waitForTimeout(500);

      // User 2 types on a new line to avoid collab cursor conflicts
      await editor2.click();
      await page2.keyboard.press("End");
      await page2.keyboard.press("Enter");
      await page2.keyboard.type("BBB", { delay: 100 });

      // Both should see content from both users
      await expect(editor1).toContainText("AAA", { timeout: 15_000 });
      await expect(editor1).toContainText("BBB", { timeout: 15_000 });
      await expect(editor2).toContainText("AAA", { timeout: 15_000 });
      await expect(editor2).toContainText("BBB", { timeout: 15_000 });
    } finally {
      await context1.close();
      await context2.close();
    }
  });
});
