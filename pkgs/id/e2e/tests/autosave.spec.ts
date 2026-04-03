import { expect, type Page, test } from "@playwright/test";

/**
 * Auto-save E2E tests for the id web UI.
 *
 * Tests verify:
 * - Auto-save triggers after 2s of idle (debounce)
 * - Rapid edits produce a single save (debounce coalescing)
 * - Save button shows correct state transitions
 * - Manual save (Ctrl+S) still works
 * - Rate limit retry (429 → automatic retry)
 * - Save button disabled-forever bug is fixed
 *
 * Prerequisites:
 * - Web variant must be built first (`just build`)
 * - Server starts automatically via playwright.config.ts webServer
 */

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Create a file with unique content via API and navigate to its editor */
async function createAndOpenFile(page: Page, name: string, baseURL: string): Promise<void> {
  const createResp = await page.request.post(`${baseURL}/api/new`, {
    data: { name },
  });
  expect(createResp.ok()).toBeTruthy();
  const { hash } = (await createResp.json()) as { hash: string; name: string };

  // Save unique content to get a unique blob hash → unique collab document
  const text = `autosave-test-${name}-${Date.now()}`;
  const saveResp = await page.request.post(`${baseURL}/api/save`, {
    data: {
      doc_id: hash,
      name,
      doc: {
        type: "doc",
        content: [{ type: "code_block", content: [{ type: "text", text }] }],
      },
    },
  });
  expect(saveResp.ok()).toBeTruthy();

  await page.goto(`/edit/${encodeURIComponent(name)}`);
  await expect(page.locator("#editor-container")).toBeVisible({ timeout: 10_000 });
  // Wait for editor to be fully initialized (save button enabled)
  await expect(page.locator("#save-btn")).toBeEnabled({ timeout: 10_000 });
}

/** Wait for ProseMirror editor to be interactive */
async function waitForEditorReady(page: Page): Promise<void> {
  await expect(page.locator(".ProseMirror")).toBeVisible({ timeout: 10_000 });
}

/** Type text into the ProseMirror editor */
async function typeInEditor(page: Page, text: string): Promise<void> {
  await page.locator(".ProseMirror").click();
  // Move to end of existing content
  await page.keyboard.press("End");
  await page.keyboard.type(text);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

test.describe("Auto-save", () => {
  test("save button shows unsaved indicator after typing", async ({ page, baseURL }) => {
    const filename = `autosave-indicator-${Date.now()}.txt`;
    await createAndOpenFile(page, filename, baseURL!);
    await waitForEditorReady(page);

    const saveBtn = page.locator("#save-btn");

    // Initially should show "save" (idle state)
    await expect(saveBtn).toHaveText("save");

    // Type something
    await typeInEditor(page, "hello autosave");

    // Should show unsaved indicator "save •"
    await expect(saveBtn).toHaveText("save •", { timeout: 2000 });
  });

  test("auto-save triggers after 2s idle", async ({ page, baseURL }) => {
    const filename = `autosave-trigger-${Date.now()}.txt`;
    await createAndOpenFile(page, filename, baseURL!);
    await waitForEditorReady(page);

    // Set up network interception to track save requests
    const saveRequests: number[] = [];
    await page.route("**/api/save", async (route) => {
      saveRequests.push(Date.now());
      await route.continue();
    });

    const saveBtn = page.locator("#save-btn");

    // Type something and stop
    await typeInEditor(page, " auto-saved-content");

    // Should see "save •" immediately
    await expect(saveBtn).toHaveText("save •", { timeout: 2000 });

    // Wait for auto-save to trigger (2s debounce + network time)
    // Should transition through "saving…" → "saved ✓"
    await expect(saveBtn).toHaveText("saved ✓", { timeout: 8000 });

    // Verify a save request was made
    expect(saveRequests.length).toBeGreaterThanOrEqual(1);

    // After 2s, should go back to "save"
    await expect(saveBtn).toHaveText("save", { timeout: 5000 });
  });

  test("debounce coalesces rapid edits into single save", async ({ page, baseURL }) => {
    const filename = `autosave-debounce-${Date.now()}.txt`;
    await createAndOpenFile(page, filename, baseURL!);
    await waitForEditorReady(page);

    // Track save requests
    let saveCount = 0;
    await page.route("**/api/save", async (route) => {
      saveCount++;
      await route.continue();
    });

    // Type rapidly with pauses shorter than 2s
    await typeInEditor(page, "a");
    await page.waitForTimeout(500);
    await page.keyboard.type("b");
    await page.waitForTimeout(500);
    await page.keyboard.type("c");
    await page.waitForTimeout(500);
    await page.keyboard.type("d");

    // Now stop typing and wait for debounce + save
    await expect(page.locator("#save-btn")).toHaveText("saved ✓", { timeout: 8000 });

    // Only 1 save should have been made (debounce coalesced all edits)
    expect(saveCount).toBe(1);
  });

  test("Ctrl+S triggers immediate manual save", async ({ page, baseURL }) => {
    const filename = `autosave-ctrl-s-${Date.now()}.txt`;
    await createAndOpenFile(page, filename, baseURL!);
    await waitForEditorReady(page);

    // Track save requests
    let saveCount = 0;
    await page.route("**/api/save", async (route) => {
      saveCount++;
      await route.continue();
    });

    // Type something
    await typeInEditor(page, " manual-save");
    await expect(page.locator("#save-btn")).toHaveText("save •", { timeout: 2000 });

    // Hit Ctrl+S immediately (before 2s debounce)
    await page.keyboard.press("Control+s");

    // Should save immediately and show "saved ✓"
    await expect(page.locator("#save-btn")).toHaveText("saved ✓", { timeout: 5000 });
    expect(saveCount).toBe(1);
  });

  test("save button re-enables after save (disabled-forever bug fix)", async ({ page, baseURL }) => {
    const filename = `autosave-reenable-${Date.now()}.txt`;
    await createAndOpenFile(page, filename, baseURL!);
    await waitForEditorReady(page);

    const saveBtn = page.locator("#save-btn");

    // Type and trigger manual save
    await typeInEditor(page, " bug-fix-test");
    await page.keyboard.press("Control+s");

    // Wait for save to complete
    await expect(saveBtn).toHaveText("saved ✓", { timeout: 5000 });

    // Button should be enabled (not permanently disabled)
    await expect(saveBtn).toBeEnabled();

    // Wait for reset and check still enabled
    await expect(saveBtn).toHaveText("save", { timeout: 5000 });
    await expect(saveBtn).toBeEnabled();
  });

  test("rate limit retry shows retry indicator", async ({ page, baseURL }) => {
    const filename = `autosave-ratelimit-${Date.now()}.txt`;
    await createAndOpenFile(page, filename, baseURL!);
    await waitForEditorReady(page);

    let requestCount = 0;
    await page.route("**/api/save", async (route) => {
      requestCount++;
      if (requestCount === 1) {
        // First request: return 429
        await route.fulfill({
          status: 429,
          contentType: "text/plain",
          body: "Save rate limited. Try again in 1s.",
        });
      } else {
        // Second request: allow through
        await route.continue();
      }
    });

    // Type and trigger save
    await typeInEditor(page, " rate-limit-test");
    await page.keyboard.press("Control+s");

    // Should show "retry…" after 429
    await expect(page.locator("#save-btn")).toHaveText("retry…", { timeout: 5000 });

    // Then should auto-retry and succeed
    await expect(page.locator("#save-btn")).toHaveText("saved ✓", { timeout: 10000 });

    // Two save requests total (initial + retry)
    expect(requestCount).toBe(2);
  });

  test("save button click triggers save via triggerSave", async ({ page, baseURL }) => {
    const filename = `autosave-click-${Date.now()}.txt`;
    await createAndOpenFile(page, filename, baseURL!);
    await waitForEditorReady(page);

    let saveCount = 0;
    await page.route("**/api/save", async (route) => {
      saveCount++;
      await route.continue();
    });

    // Type something
    await typeInEditor(page, " click-save");
    await expect(page.locator("#save-btn")).toHaveText("save •", { timeout: 2000 });

    // Click save button (should call triggerSave)
    await page.click("#save-btn");

    // Should save and show "saved ✓"
    await expect(page.locator("#save-btn")).toHaveText("saved ✓", { timeout: 5000 });
    expect(saveCount).toBe(1);
  });
});
