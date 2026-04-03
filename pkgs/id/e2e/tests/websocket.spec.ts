import {
  type Browser,
  type BrowserType,
  test as base,
  chromium,
  expect,
  firefox,
  type Page,
  webkit,
} from "@playwright/test";

const browserTypes: Record<string, BrowserType> = { chromium, firefox, webkit };

// ---------------------------------------------------------------------------
// Fresh browser fixture — WHY this file overrides Playwright's default browser
// ---------------------------------------------------------------------------
//
// PROBLEM:
// This test file runs LAST in the Playwright suite (files run alphabetically:
// basic → file-operations → navigation → websocket). By the time Playwright
// reaches this file, Firefox's shared browser process has already executed
// ~54 tests across 3 prior test files — all using the same browser process.
//
// After that many sequential page navigations in a single Firefox process,
// the WebSocket upgrade path SILENTLY DEGRADES: the server receives the HTTP
// upgrade request, but the WS handshake never completes at the protocol level.
// Server logs confirm this — a 40+ second gap between the upgrade request
// arriving and any subsequent activity, with no "Client connected" or
// "Sending Init" log entries. The connection just hangs at the HTTP layer.
//
// Regular HTTP continues to work fine (which is why basic/file-ops/navigation
// tests pass without issue), but the HTTP→WebSocket protocol switch machinery
// in Firefox's networking stack breaks down after extended use.
//
// EVIDENCE:
// - Server logs show the upgrade request arrives but WS handshake never
//   completes (no "Client connected" after the initial connection log)
// - The client's 2s connect timeout fires, triggering reconnect
// - The RETRY always succeeds (same browser process, new socket)
// - But the initial 2s timeout + 30s test timeout = test fails before
//   the retry can complete
// - Chromium is completely unaffected by this issue
// - The problem ONLY occurs after ~40+ prior tests in the same process
// - A fresh browser process connects instantly every time
//
// FIX:
// Override Playwright's `browser` fixture to launch a FRESH browser instance
// for this file. Since `browser` is worker-scoped (one per file when
// workers=1), this gives all WebSocket tests a clean networking stack.
// The built-in `context` and `page` fixtures automatically inherit from our
// fresh browser, so NO test code changes are needed — tests keep using
// `{ page }`, `{ browser, baseURL }`, etc. exactly as before.
//
// COST:
// ~1-2s of browser launch overhead per worker, which eliminates the 30s+
// timeout-and-retry cycle that made these tests flaky in Firefox.
// Chromium is unaffected by the bug but also benefits (no downside).
// ---------------------------------------------------------------------------
// In nix sandbox or VM test, Chromium needs extra flags (matching playwright.config.ts):
//   --no-sandbox, --disable-setuid-sandbox, --no-zygote,
//   --disable-dev-shm-usage, --disable-gpu, --disable-software-rasterizer
const IS_NIX_BUILD = !!process.env.NIX_BUILD_TOP;
const IS_VM_TEST = !!process.env.PLAYWRIGHT_VM_TEST;
const CHROMIUM_NIX_ARGS =
  IS_NIX_BUILD || IS_VM_TEST
    ? [
        "--no-sandbox",
        "--disable-setuid-sandbox",
        "--no-zygote",
        "--disable-dev-shm-usage",
        "--disable-gpu",
        "--disable-software-rasterizer",
      ]
    : [];

const test = base.extend<Record<string, never>, { browser: Browser }>({
  browser: [
    async ({ browserName }, use) => {
      const args = browserName === "chromium" ? CHROMIUM_NIX_ARGS : [];
      const browser = await browserTypes[browserName].launch(args.length > 0 ? { args } : undefined);
      await use(browser);
      await browser.close();
    },
    { scope: "worker" },
  ],
});

// WebSocket tests need more headroom than basic tests because Firefox's WS
// handshake can occasionally hang (~20s browser timeout + reconnect delay).
// 30s gives enough room for multiple reconnect cycles (2s timeout + 0.25–5s
// backoff each) within a single test.
test.setTimeout(30_000);

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
async function waitForEditorReady(page: Page, timeout = 20_000): Promise<void> {
  // Poll JS state directly: collab connected + ProseMirror mounted.
  // Uses explicit interval polling (not rAF) because Firefox throttles
  // requestAnimationFrame during heavy JS initialization, causing missed
  // state transitions. 100ms interval is fast enough for test responsiveness.
  await page.waitForFunction(
    () => {
      const app = (window as unknown as { idApp: { collab: { ws: WebSocket; editor: unknown } | null } }).idApp;
      if (!app?.collab?.ws || app.collab.ws.readyState !== WebSocket.OPEN) return false;
      if (!app.collab.editor) return false;
      // Also verify ProseMirror is in the DOM
      return !!document.querySelector("#editor .ProseMirror");
    },
    { polling: 100, timeout },
  );
}

/** Wait for the Tags WebSocket to be connected */
async function waitForTagsWs(page: Page): Promise<void> {
  await page.waitForFunction(
    () => {
      const app = (window as unknown as { idApp: { tagsWs: WebSocket | null } }).idApp;
      return app?.tagsWs?.readyState === WebSocket.OPEN;
    },
    { polling: 100, timeout: 15_000 },
  );
}

/**
 * Set editor content programmatically via ProseMirror's dispatch API.
 * This generates real collab steps (unlike DOM manipulation) but bypasses
 * keyboard input, which Firefox in NixOS VMs handles unreliably — dropping
 * characters and inserting keycode fragments like "0305".
 */
async function setEditorContent(page: Page, text: string): Promise<void> {
  await page.evaluate((t) => {
    const app = (window as unknown as { idApp: { collab: { editor: { view: any } } } }).idApp;
    const view = app.collab.editor.view;
    const { state } = view;
    // Replace all inline content: pos 1 = inside first block, size-1 = before closing tag
    const tr = state.tr.insertText(t, 1, state.doc.content.size - 1);
    view.dispatch(tr);
  }, text);
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

  // Navigate directly to file — tests that direct URL access works (bookmarks,
  // link sharing, page refresh). The 2s connect timeout in collab.ts handles
  // any WS init race on full page load.
  //
  // NOTE: Do NOT use waitForLoadState("networkidle") here. The WS upgrade
  // request counts as a pending connection; if Firefox's handshake hangs,
  // the 2s connect timeout fires → scheduleReconnect → new pending request,
  // resetting networkidle's 500ms idle counter. This loop eats 30s+ of test
  // budget before networkidle gives up. waitForEditorReady() handles all the
  // real waiting by polling JS/DOM state directly.
  await page.goto(`/edit/${encodeURIComponent(name)}`);
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
  test("can type into ProseMirror editor", async ({ page, baseURL }) => {
    const fileName = `ws-type-${Date.now()}.txt`;
    await createFileWithUniqueContent(page, fileName, baseURL!);
    await waitForEditorReady(page);

    // Set content programmatically — bypasses Firefox keyboard flakiness in NixOS VMs
    const editor = page.locator("#editor .ProseMirror");
    await setEditorContent(page, "Hello from E2E test!");

    // Verify text appears in the editor
    await expect(editor).toContainText("Hello from E2E test!");
  });

  test("can save file and content persists", async ({ page, baseURL }) => {
    const fileName = `ws-save-${Date.now()}.txt`;
    await createFileWithUniqueContent(page, fileName, baseURL!);
    // Wait for save rate limit to expire — createFileWithUniqueContent saves once to get a unique hash
    await page.waitForTimeout(5_500);
    await waitForEditorReady(page);

    // Set content programmatically — bypasses Firefox keyboard flakiness in NixOS VMs
    const editor = page.locator("#editor .ProseMirror");
    await setEditorContent(page, "Saved content test");
    await expect(editor).toContainText("Saved content test");

    // Wait for save button to be enabled (collab must be connected)
    await expect(page.locator("#save-btn")).toBeEnabled({ timeout: 10_000 });

    // Click save button and wait for the save round-trip to complete
    await page.click("#save-btn");
    await expect(page.locator("#save-btn")).toContainText("saved", { timeout: 10_000 });

    // Verify persistence: extract the new hash from the URL (updated by save)
    // and fetch the raw blob content via /blob/:hash API.
    // This avoids the WS/collab race that occurs when reloading or opening a
    // new page immediately after save.
    const url = page.url();
    const hash = url.split("/").pop()!;
    const resp = await page.request.get(`${baseURL}/blob/${hash}`);
    expect(resp.ok()).toBeTruthy();
    const body = await resp.text();
    expect(body).toContain("Saved content test");
  });

  test("Ctrl+S triggers save", async ({ page, baseURL }) => {
    const fileName = `ws-ctrlsave-${Date.now()}.txt`;
    await createFileWithUniqueContent(page, fileName, baseURL!);
    // Wait for save rate limit to expire — createFileWithUniqueContent saves once to get a unique hash
    await page.waitForTimeout(5_500);
    await waitForEditorReady(page);

    // Set content programmatically — bypasses Firefox keyboard flakiness in NixOS VMs
    const _editor = page.locator("#editor .ProseMirror");
    await setEditorContent(page, "Ctrl+S test content");

    // Use Ctrl+S to save
    await page.keyboard.press("Control+s");

    // Wait for save to complete (must match "saved" specifically, not just "save")
    await expect(page.locator("#save-btn")).toContainText("saved", { timeout: 10_000 });

    // Verify persistence: extract hash from URL and fetch raw blob
    const url = page.url();
    const hash = url.split("/").pop()!;
    const resp = await page.request.get(`${baseURL}/blob/${hash}`);
    expect(resp.ok()).toBeTruthy();
    const body = await resp.text();
    expect(body).toContain("Ctrl+S test content");
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

    // Disconnect cleanly via the collab API (sets intentionalClose flag + close(1000))
    // We use disconnect() instead of raw ws.close(1000) because the WebSocket close
    // handshake can fail/timeout, causing the browser to fire onclose with code 1006
    // instead of the requested 1000 — which would spuriously trigger reconnect.
    await page.evaluate(() => {
      const app = (window as unknown as { idApp: { collab: { disconnect: () => void } } }).idApp;
      if (app?.collab) {
        app.collab.disconnect();
      }
    });

    // After disconnect(), currentWs is immediately set to null (no async wait needed)
    await page.waitForTimeout(500);

    // collab.ws getter returns null after disconnect() sets currentWs = null
    const wsIsNull = await page.evaluate(() => {
      const app = (window as unknown as { idApp: { collab: { ws: WebSocket | null } | null } }).idApp;
      return !app?.collab?.ws;
    });
    expect(wsIsNull).toBeTruthy();

    // Wait longer than initial reconnect backoff (1s) to verify no reconnect attempt
    await page.waitForTimeout(3_000);

    // WS should still be null — no reconnect was scheduled
    const wsStillNull = await page.evaluate(() => {
      const app = (window as unknown as { idApp: { collab: { ws: WebSocket | null } | null } }).idApp;
      return !app?.collab?.ws;
    });
    expect(wsStillNull).toBeTruthy();
  });
});

// ---------------------------------------------------------------------------
// 6. Multi-User Collaborative Editing
// ---------------------------------------------------------------------------

test.describe("Multi-User Collab", () => {
  /** Set up two pages with the same file open in both editors */
  async function setupCollabPair(browser: import("@playwright/test").Browser, baseURL: string) {
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

  test("two tabs can open the same file simultaneously", async ({ browser, baseURL }) => {
    const { context1, context2, page1, page2 } = await setupCollabPair(browser, baseURL!);

    try {
      // Both editors should be connected
      await expect(page1.locator("#editor-status")).toHaveText("connected");
      await expect(page2.locator("#editor-status")).toHaveText("connected");
    } finally {
      await context1.close();
      await context2.close();
    }
  });

  test("edits from one user appear in other user's editor", async ({ browser, baseURL }) => {
    const { context1, context2, page1, page2 } = await setupCollabPair(browser, baseURL!);

    try {
      // User 1 types something — use programmatic API to bypass Firefox keyboard flakiness
      const _editor1 = page1.locator("#editor .ProseMirror");
      await setEditorContent(page1, "Hello from user 1!");
      // Brief wait for collab sync propagation over cross-VM networking
      await page1.waitForTimeout(2_000);

      // User 2 should see the text appear (via collab WebSocket sync)
      const editor2 = page2.locator("#editor .ProseMirror");
      await expect(editor2).toContainText("Hello from user 1!", { timeout: 20_000 });
    } finally {
      await context1.close();
      await context2.close();
    }
  });

  test("bidirectional editing works", async ({ browser, baseURL }) => {
    const { context1, context2, page1, page2 } = await setupCollabPair(browser, baseURL!);

    try {
      // User 1 types first (slow enough for collab to sync each step)
      const editor1 = page1.locator("#editor .ProseMirror");
      await editor1.click();
      await page1.keyboard.type("AAA", { delay: 100 });

      // Wait for full sync to User 2
      const editor2 = page2.locator("#editor .ProseMirror");
      await expect(editor2).toContainText("AAA", { timeout: 20_000 });

      // Let collab settle before User 2 types — needs enough time for collab
      // version tracking to stabilize so User 2's edits don't conflict
      await page2.waitForTimeout(1_000);

      // User 2 types after User 1's text — use Ctrl+End for reliable cursor
      // positioning at end of document (End alone only moves to end of line,
      // and click() cursor position depends on element geometry). Avoid Enter
      // (new paragraph node) which can cause collab merge conflicts.
      await editor2.click();
      await page2.keyboard.press("Control+End");
      await page2.waitForTimeout(200);
      await page2.keyboard.type(" BBB", { delay: 100 });

      // Both should see content from both users
      await expect(editor1).toContainText("AAA", { timeout: 20_000 });
      await expect(editor1).toContainText("BBB", { timeout: 20_000 });
      await expect(editor2).toContainText("AAA", { timeout: 20_000 });
      await expect(editor2).toContainText("BBB", { timeout: 20_000 });
    } finally {
      await context1.close();
      await context2.close();
    }
  });

  test("collab persists across save — both clients stay connected", async ({ browser, baseURL }) => {
    const { context1, context2, page1, page2, fileName } = await setupCollabPair(browser, baseURL!);

    try {
      // User 1 types content
      await setEditorContent(page1, "Before save");
      await page1.waitForTimeout(2_000);
      await expect(page2.locator("#editor .ProseMirror")).toContainText("Before save", { timeout: 20_000 });

      // User 1 saves (POST /api/save)
      const hash = await page1.evaluate(() => {
        const el = document.getElementById("editor-container");
        return el?.dataset.hash ?? "";
      });
      const doc = await page1.evaluate(() => {
        const app = (window as unknown as { idApp: { collab: { editor: { view: any } } } }).idApp;
        return app.collab.editor.view.state.doc.toJSON();
      });
      const saveResp = await page1.request.post(`${baseURL}/api/save`, {
        data: { doc_id: hash, name: fileName, doc },
      });
      expect(saveResp.ok()).toBeTruthy();

      // Both clients should still be connected after save
      await page1.waitForTimeout(1_000);
      await expect(page1.locator("#editor-status")).toHaveText("connected", { timeout: 10_000 });
      await expect(page2.locator("#editor-status")).toHaveText("connected", { timeout: 10_000 });

      // User 2 can still type and User 1 sees it
      const editor2 = page2.locator("#editor .ProseMirror");
      await editor2.click();
      await page2.keyboard.press("Control+End");
      await page2.waitForTimeout(200);
      await page2.keyboard.type(" after-save", { delay: 100 });

      await expect(page1.locator("#editor .ProseMirror")).toContainText("after-save", { timeout: 20_000 });
    } finally {
      await context1.close();
      await context2.close();
    }
  });
});

// ---------------------------------------------------------------------------
// 7. URL Scheme — Name-First Routes (Part 3)
// ---------------------------------------------------------------------------

test.describe("Name-First URL Scheme", () => {
  test("/hash/{hash} redirects to /edit/{name}", async ({ page, baseURL }) => {
    // Create a file via API to get its hash
    const fileName = `redirect-test-${Date.now()}.txt`;
    const createResp = await page.request.post(`${baseURL}/api/new`, {
      data: { name: fileName },
    });
    expect(createResp.ok()).toBeTruthy();
    const { hash } = (await createResp.json()) as { hash: string; name: string };

    // Navigate to /hash/{hash} — should redirect to /edit/{name}
    await page.goto(`/hash/${hash}`);
    await page.waitForURL(/\/edit\//, { timeout: 10_000 });
    expect(page.url()).toContain(`/edit/${encodeURIComponent(fileName)}`);

    // Editor should load
    await expect(page.locator("#editor-container")).toBeVisible({ timeout: 10_000 });
  });

  test("/view/{name} redirects to /edit/{name}", async ({ page, baseURL }) => {
    const fileName = `view-test-${Date.now()}.txt`;
    // Create file first
    const createResp = await page.request.post(`${baseURL}/api/new`, {
      data: { name: fileName },
    });
    expect(createResp.ok()).toBeTruthy();

    // Navigate to /view/{name} — should redirect to /edit/{name}
    await page.goto(`/view/${encodeURIComponent(fileName)}`);
    await page.waitForURL(/\/edit\//, { timeout: 10_000 });
    expect(page.url()).toContain(`/edit/${encodeURIComponent(fileName)}`);
  });
});
