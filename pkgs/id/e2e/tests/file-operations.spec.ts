import { test, expect } from "@playwright/test";

/**
 * File Operations E2E tests for the id web UI.
 *
 * Tests verify:
 * - Tag CRUD (add via inline inputs, verify display, remove)
 * - File rename via prompt dialog
 * - File copy via prompt dialog
 * - File delete and restore via API
 * - File search/filter
 * - Theme switching
 *
 * Prerequisites:
 * - Web variant must be built first (`just build`)
 * - Server starts automatically via playwright.config.ts webServer
 */

// Helper: create a file and navigate to its editor
async function createFile(page: import("@playwright/test").Page, name: string) {
  await page.goto("/");
  await page.fill("#new-file-name", name);
  await page.click("#new-file-form button[type='submit']");
  await page.waitForURL(/\/(file|edit)\//, { timeout: 15_000 });
  await expect(page.locator("#editor-container")).toBeVisible({ timeout: 10_000 });
}

// ---------------------------------------------------------------------------
// Tag Operations
// ---------------------------------------------------------------------------

test.describe("Tag Operations", () => {
  test("can add a tag via inline inputs", async ({ page }) => {
    const fileName = `tag-add-${Date.now()}.txt`;
    await createFile(page, fileName);

    // Fill in tag key and value
    await page.fill("#tag-add-key", "author");
    await page.fill("#tag-add-value", "testbot");

    // Submit tag by pressing Enter on the value input
    await page.press("#tag-add-value", "Enter");

    // Wait for the tag to appear in the tag list
    const tagList = page.locator("#editor-tag-list");
    await expect(tagList.locator(".tag-pill-removable", { hasText: "author: testbot" })).toBeVisible({
      timeout: 5_000,
    });

    // Inputs should be cleared after adding
    await expect(page.locator("#tag-add-key")).toHaveValue("");
    await expect(page.locator("#tag-add-value")).toHaveValue("");
  });

  test("can add a key-only tag", async ({ page }) => {
    const fileName = `tag-keyonly-${Date.now()}.txt`;
    await createFile(page, fileName);

    // Fill in just the key (no value)
    await page.fill("#tag-add-key", "important");
    await page.press("#tag-add-key", "Enter");

    // Tag should appear with just the key
    const tagList = page.locator("#editor-tag-list");
    await expect(tagList.locator(".tag-pill-removable", { hasText: "important" })).toBeVisible({
      timeout: 5_000,
    });
  });

  test("can remove a tag", async ({ page }) => {
    const fileName = `tag-remove-${Date.now()}.txt`;
    await createFile(page, fileName);

    // Add a tag first
    await page.fill("#tag-add-key", "removeme");
    await page.fill("#tag-add-value", "yes");
    await page.press("#tag-add-value", "Enter");

    // Wait for it to appear
    const tagList = page.locator("#editor-tag-list");
    const tagPill = tagList.locator(".tag-pill-removable", { hasText: "removeme" });
    await expect(tagPill).toBeVisible({ timeout: 5_000 });

    // Click the × button to remove
    await tagPill.locator(".tag-remove-btn").click();

    // Tag should disappear
    await expect(tagPill).not.toBeVisible({ timeout: 5_000 });
  });

  test("can add multiple tags", async ({ page }) => {
    const fileName = `tag-multi-${Date.now()}.txt`;
    await createFile(page, fileName);

    // Add first tag
    await page.fill("#tag-add-key", "color");
    await page.fill("#tag-add-value", "blue");
    await page.press("#tag-add-value", "Enter");

    const tagList = page.locator("#editor-tag-list");
    await expect(tagList.locator(".tag-pill-removable", { hasText: "color: blue" })).toBeVisible({
      timeout: 5_000,
    });

    // Add second tag
    await page.fill("#tag-add-key", "priority");
    await page.fill("#tag-add-value", "high");
    await page.press("#tag-add-value", "Enter");

    await expect(tagList.locator(".tag-pill-removable", { hasText: "priority: high" })).toBeVisible({
      timeout: 5_000,
    });

    // Both tags should be visible
    await expect(tagList.locator(".tag-pill-removable", { hasText: "color: blue" })).toBeVisible({ timeout: 5_000 });
    await expect(tagList.locator(".tag-pill-removable", { hasText: "priority: high" })).toBeVisible({ timeout: 5_000 });
  });

  test("tag persists after navigating away and back", async ({ page, baseURL }) => {
    const fileName = `tag-persist-${Date.now()}.txt`;
    await createFile(page, fileName);

    // Set a tag via REST API (UI tag add is tested separately above)
    const setResp = await page.request.post(`${baseURL}/api/tags`, {
      data: { subject: fileName, key: "status", value: "draft" },
    });
    expect(setResp.ok()).toBeTruthy();

    // Navigate away to files page
    await page.click("a[data-nav][href='/']");
    await expect(page.locator("#new-file-form")).toBeVisible({ timeout: 10_000 });

    // Verify tag persistence via REST API after navigation
    const resp = await page.request.get(`${baseURL}/api/tags?subject=${encodeURIComponent(fileName)}`);
    expect(resp.ok()).toBeTruthy();
    const tags = await resp.json();
    const statusTag = tags.find(
      (t: { key: string; value: string | null }) => t.key === "status" && t.value === "draft",
    );
    expect(statusTag).toBeTruthy();
  });
});

// ---------------------------------------------------------------------------
// File Rename
// ---------------------------------------------------------------------------

test.describe("File Rename", () => {
  test("can rename a file via dialog", async ({ page }) => {
    const fileName = `rename-src-${Date.now()}.txt`;
    const newName = `rename-dst-${Date.now()}.txt`;
    await createFile(page, fileName);

    // Set up dialog handlers for prompt (new name) and confirm (archive)
    page.on("dialog", async (dialog) => {
      if (dialog.type() === "prompt") {
        await dialog.accept(newName);
      } else if (dialog.type() === "confirm") {
        // Archive the original
        await dialog.accept();
      }
    });

    // Click rename button
    await page.click("#rename-btn");

    // Should navigate to the new file URL
    await page.waitForURL(new RegExp(`/file/${newName}`), { timeout: 10_000 });

    // Verify the editor still works with the new name
    await expect(page.locator("#editor-container")).toBeVisible();
    await expect(page.locator("#editor-container")).toHaveAttribute("data-filename", newName);
  });

  test("renamed file appears in file list with new name", async ({ page }) => {
    const fileName = `rename-list-${Date.now()}.txt`;
    const newName = `renamed-list-${Date.now()}.txt`;
    await createFile(page, fileName);

    page.on("dialog", async (dialog) => {
      if (dialog.type() === "prompt") {
        await dialog.accept(newName);
      } else if (dialog.type() === "confirm") {
        await dialog.accept();
      }
    });

    await page.click("#rename-btn");
    await page.waitForURL(new RegExp(newName.replace(/\./g, "\\.")), { timeout: 10_000 });

    // Go to file list
    await page.goto("/");
    await expect(page.locator(`text=${newName}`)).toBeVisible({ timeout: 5_000 });
  });
});

// ---------------------------------------------------------------------------
// File Copy
// ---------------------------------------------------------------------------

test.describe("File Copy", () => {
  test("can copy a file via dialog", async ({ page }) => {
    const fileName = `copy-src-${Date.now()}.txt`;
    const copyName = `copy-dst-${Date.now()}.txt`;
    await createFile(page, fileName);

    page.on("dialog", async (dialog) => {
      if (dialog.type() === "prompt") {
        await dialog.accept(copyName);
      }
    });

    // Click copy button
    await page.click("#copy-btn");

    // Should navigate to the copied file
    await page.waitForURL(new RegExp(copyName.replace(/\./g, "\\.")), { timeout: 10_000 });
    await expect(page.locator("#editor-container")).toBeVisible();
  });

  test("both original and copy exist in file list", async ({ page }) => {
    const fileName = `copy-both-${Date.now()}.txt`;
    const copyName = `copy-both-dup-${Date.now()}.txt`;
    await createFile(page, fileName);

    page.on("dialog", async (dialog) => {
      if (dialog.type() === "prompt") {
        await dialog.accept(copyName);
      }
    });

    await page.click("#copy-btn");
    await page.waitForURL(new RegExp(copyName.replace(/\./g, "\\.")), { timeout: 10_000 });

    // Check file list
    await page.goto("/");
    await expect(page.locator(`text=${fileName}`)).toBeVisible({ timeout: 5_000 });
    await expect(page.locator(`text=${copyName}`)).toBeVisible({ timeout: 5_000 });
  });
});

// ---------------------------------------------------------------------------
// File Delete and Restore
// ---------------------------------------------------------------------------

test.describe("File Delete and Restore", () => {
  test("can delete a file via API and it disappears from list", async ({ page }) => {
    const fileName = `delete-test-${Date.now()}.txt`;
    await createFile(page, fileName);

    // Delete via API
    const response = await page.request.post("/api/delete", {
      data: { name: fileName },
      headers: { "Content-Type": "application/json" },
    });
    expect(response.ok()).toBeTruthy();

    // Go to file list - file should not be visible (without show-deleted)
    await page.goto("/");
    await expect(page.locator(`.file-item a:has-text("${fileName}")`)).not.toBeVisible({ timeout: 5_000 });
  });

  test("deleted file appears when show-deleted is checked", async ({ page }) => {
    const fileName = `delete-show-${Date.now()}.txt`;
    await createFile(page, fileName);

    // Delete via API
    await page.request.post("/api/delete", {
      data: { name: fileName },
      headers: { "Content-Type": "application/json" },
    });

    // Go to file list
    await page.goto("/");

    // Check show-deleted checkbox
    await page.check("#show-deleted");

    // File should appear now (may need a moment for the fetch)
    await expect(page.locator(`text=${fileName}`)).toBeVisible({ timeout: 5_000 });
  });

  test("can restore a deleted file via API", async ({ page }) => {
    const fileName = `restore-test-${Date.now()}.txt`;
    await createFile(page, fileName);

    // Delete
    await page.request.post("/api/delete", {
      data: { name: fileName },
      headers: { "Content-Type": "application/json" },
    });

    // Verify gone
    await page.goto("/");
    await expect(page.locator(`.file-item a:has-text("${fileName}")`)).not.toBeVisible({ timeout: 5_000 });

    // Restore
    const restoreResp = await page.request.post("/api/restore", {
      data: { name: fileName },
      headers: { "Content-Type": "application/json" },
    });
    expect(restoreResp.ok()).toBeTruthy();

    // Reload and verify it's back
    await page.goto("/");
    await expect(page.locator(`text=${fileName}`)).toBeVisible({ timeout: 5_000 });
  });
});

// ---------------------------------------------------------------------------
// File Search
// ---------------------------------------------------------------------------

test.describe("File Search", () => {
  test("search input filters file list", async ({ page }) => {
    // Create two files with different names
    const uniquePrefix = `srch-${Date.now()}`;
    const file1 = `${uniquePrefix}-alpha.txt`;
    const file2 = `${uniquePrefix}-beta.txt`;

    await createFile(page, file1);
    await page.goto("/");
    await createFile(page, file2);
    await page.goto("/");

    // Both files should be visible
    await expect(page.locator(`text=${file1}`)).toBeVisible({ timeout: 5_000 });
    await expect(page.locator(`text=${file2}`)).toBeVisible({ timeout: 5_000 });

    // Search for "alpha" — should filter to just file1
    // Use fill + keyup dispatch since search listens on 'keyup' not 'input'
    await page.fill("#file-search", "alpha");
    await page.locator("#file-search").dispatchEvent("keyup");

    // Wait for debounce (300ms) + fetch
    await expect(page.locator(`text=${file1}`)).toBeVisible({ timeout: 5_000 });
    await expect(page.locator(`text=${file2}`)).not.toBeVisible({ timeout: 5_000 });
  });

  test("clearing search shows all files again", async ({ page }) => {
    const uniquePrefix = `srchclr-${Date.now()}`;
    const file1 = `${uniquePrefix}-one.txt`;
    const file2 = `${uniquePrefix}-two.txt`;

    await createFile(page, file1);
    await page.goto("/");
    await createFile(page, file2);
    await page.goto("/");

    // Search for "one"
    await page.fill("#file-search", "one");
    await page.locator("#file-search").dispatchEvent("keyup");
    await expect(page.locator(`text=${file2}`)).not.toBeVisible({ timeout: 5_000 });

    // Clear search
    await page.fill("#file-search", "");
    await page.locator("#file-search").dispatchEvent("keyup");

    // Both should be visible again
    await expect(page.locator(`text=${file1}`)).toBeVisible({ timeout: 5_000 });
    await expect(page.locator(`text=${file2}`)).toBeVisible({ timeout: 5_000 });
  });

  test("search with no results shows empty state", async ({ page }) => {
    await page.goto("/");

    // Search for something that doesn't exist
    await page.fill("#file-search", `nonexistent-${Date.now()}-xyz`);
    await page.locator("#file-search").dispatchEvent("keyup");

    // Should show empty state
    await expect(page.locator("text=No files match your search.")).toBeVisible({ timeout: 5_000 });
  });
});

// ---------------------------------------------------------------------------
// Theme Switching
// ---------------------------------------------------------------------------

test.describe("Theme Switching", () => {
  test("can switch theme from settings page", async ({ page }) => {
    await page.goto("/settings");

    // Default is sneak
    await expect(page.locator("html")).toHaveAttribute("data-theme", "sneak");

    // Click arch theme button (use text to target settings content buttons, not header buttons)
    await page.click("button[data-theme='arch']:has-text('Arch')");
    await expect(page.locator("html")).toHaveAttribute("data-theme", "arch");

    // Click mech theme button
    await page.click("button[data-theme='mech']:has-text('Mech')");
    await expect(page.locator("html")).toHaveAttribute("data-theme", "mech");

    // Back to sneak
    await page.click("button[data-theme='sneak']:has-text('Sneak')");
    await expect(page.locator("html")).toHaveAttribute("data-theme", "sneak");
  });

  test("theme persists across navigation", async ({ page }) => {
    await page.goto("/settings");

    // Switch to arch theme (use text to target settings content buttons)
    await page.click("button[data-theme='arch']:has-text('Arch')");
    await expect(page.locator("html")).toHaveAttribute("data-theme", "arch");

    // Navigate to files
    await page.click("a[data-nav][href='/']");
    await expect(page.locator("#new-file-form")).toBeVisible({ timeout: 10_000 });

    // Theme should still be arch
    await expect(page.locator("html")).toHaveAttribute("data-theme", "arch");

    // Navigate to peers
    await page.click("a[data-nav][href='/peers']");
    await expect(page.locator("text=Discovered Peers")).toBeVisible({ timeout: 10_000 });

    // Theme should still be arch
    await expect(page.locator("html")).toHaveAttribute("data-theme", "arch");
  });

  test("theme persists across page reload", async ({ page }) => {
    await page.goto("/settings");

    // Switch to mech (use text to target settings content button)
    await page.click("button[data-theme='mech']:has-text('Mech')");
    await expect(page.locator("html")).toHaveAttribute("data-theme", "mech");

    // Reload page
    await page.reload();

    // Theme should still be mech (stored in localStorage)
    await expect(page.locator("html")).toHaveAttribute("data-theme", "mech");
  });
});
