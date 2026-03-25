import { test, expect } from "@playwright/test";

/**
 * Basic E2E tests for the id web UI.
 *
 * These tests verify core functionality across Chromium and Firefox:
 * - Home page loads correctly
 * - File creation flow
 * - Editor page elements (rename, copy, tags)
 * - Navigation
 * - Theme support
 *
 * Prerequisites:
 * - Web variant must be built first (`just build`)
 * - Server starts automatically via playwright.config.ts webServer
 */

// ---------------------------------------------------------------------------
// Home Page
// ---------------------------------------------------------------------------

test.describe("Home Page", () => {
  test("loads with correct title", async ({ page }) => {
    await page.goto("/");
    await expect(page).toHaveTitle("Files - id");
  });

  test("shows file list card", async ({ page }) => {
    await page.goto("/");
    // The file list is inside a card with "Files" header
    await expect(page.locator(".card-header", { hasText: "Files" })).toBeVisible();
  });

  test("has new file form", async ({ page }) => {
    await page.goto("/");
    await expect(page.locator("#new-file-name")).toBeVisible();
    await expect(page.locator("#new-file-form button[type='submit']")).toBeVisible();
  });

  test("has search input", async ({ page }) => {
    await page.goto("/");
    await expect(page.locator("#file-search")).toBeVisible();
    await expect(page.locator("#file-search")).toHaveAttribute("placeholder", /search/i);
  });

  test("has show deleted checkbox", async ({ page }) => {
    await page.goto("/");
    await expect(page.locator("#show-deleted")).toBeVisible();
  });

  test("has theme toggle in footer", async ({ page }) => {
    await page.goto("/");
    await expect(page.locator("footer a", { hasText: "theme" })).toBeVisible();
  });

  test("shows empty state when no files", async ({ page }) => {
    await page.goto("/");
    // Fresh ephemeral server has no files
    await expect(page.locator(".text-muted", { hasText: /no files|empty/i })).toBeVisible();
  });
});

// ---------------------------------------------------------------------------
// File Creation
// ---------------------------------------------------------------------------

test.describe("File Creation", () => {
  test("can create a new file and navigate to editor", async ({ page }) => {
    const fileName = `test-${Date.now()}.md`;

    await page.goto("/");
    await page.fill("#new-file-name", fileName);
    await page.click("#new-file-form button[type='submit']");

    // createFile() uses htmx.ajax + history.pushState — URL changes but
    // <title> may not update since HTMX swaps #main innerHTML only.
    // Wait for the editor container to appear instead of checking title.
    await page.waitForURL(/\/(file|edit)\//, { timeout: 15_000 });
    await expect(page.locator("#editor-container")).toBeVisible({ timeout: 10_000 });
    await expect(page.locator("#editor-container")).toHaveAttribute("data-filename", /.+/);
  });

  test("editor has rename button", async ({ page }) => {
    const fileName = `rename-test-${Date.now()}.txt`;

    await page.goto("/");
    await page.fill("#new-file-name", fileName);
    await page.click("#new-file-form button[type='submit']");
    await page.waitForURL(/\/(file|edit)\//);

    await expect(page.locator("#rename-btn")).toBeVisible();
    await expect(page.locator("#rename-btn")).toHaveText("rename");
  });

  test("editor has copy button", async ({ page }) => {
    const fileName = `copy-test-${Date.now()}.txt`;

    await page.goto("/");
    await page.fill("#new-file-name", fileName);
    await page.click("#new-file-form button[type='submit']");
    await page.waitForURL(/\/(file|edit)\//);

    await expect(page.locator("#copy-btn")).toBeVisible();
    await expect(page.locator("#copy-btn")).toHaveText("copy");
  });

  test("editor has tag panel", async ({ page }) => {
    const fileName = `tag-test-${Date.now()}.txt`;

    await page.goto("/");
    await page.fill("#new-file-name", fileName);
    await page.click("#new-file-form button[type='submit']");
    await page.waitForURL(/\/(file|edit)\//);

    await expect(page.locator("#editor-tag-panel")).toBeVisible();
    await expect(page.locator("#tag-add-key")).toBeVisible();
    await expect(page.locator("#tag-add-value")).toBeVisible();
  });

  test("editor has ProseMirror editor", async ({ page }) => {
    const fileName = `editor-test-${Date.now()}.txt`;

    await page.goto("/");
    await page.fill("#new-file-name", fileName);
    await page.click("#new-file-form button[type='submit']");
    await page.waitForURL(/\/(file|edit)\//);

    // The editor wrapper and editor div should exist
    await expect(page.locator("#editor-container")).toBeVisible();
    await expect(page.locator("#editor")).toBeVisible();
  });
});

// ---------------------------------------------------------------------------
// Navigation
// ---------------------------------------------------------------------------

test.describe("Navigation", () => {
  test("can navigate from editor back to file list", async ({ page }) => {
    const fileName = `nav-test-${Date.now()}.txt`;

    // Create a file
    await page.goto("/");
    await page.fill("#new-file-name", fileName);
    await page.click("#new-file-form button[type='submit']");
    await page.waitForURL(/\/(file|edit)\//);

    // Click "files" link in editor header
    await page.click("a[href='/']", { timeout: 5_000 });

    // Should be back at file list
    await expect(page).toHaveTitle("Files - id");
  });

  test("created file appears in file list", async ({ page }) => {
    const fileName = `list-test-${Date.now()}.txt`;

    // Create a file
    await page.goto("/");
    await page.fill("#new-file-name", fileName);
    await page.click("#new-file-form button[type='submit']");
    await page.waitForURL(/\/(file|edit)\//);

    // Go back to file list
    await page.goto("/");

    // File should appear in the list
    await expect(page.locator(`text=${fileName}`)).toBeVisible();
  });
});

// ---------------------------------------------------------------------------
// Theme
// ---------------------------------------------------------------------------

test.describe("Theme", () => {
  test("page has default theme attribute", async ({ page }) => {
    await page.goto("/");
    const html = page.locator("html");
    await expect(html).toHaveAttribute("data-theme", "sneak");
  });

  test("theme switcher buttons exist on editor page", async ({ page }) => {
    const fileName = `theme-test-${Date.now()}.txt`;

    await page.goto("/");
    await page.fill("#new-file-name", fileName);
    await page.click("#new-file-form button[type='submit']");
    await page.waitForURL(/\/(file|edit)\//);

    // Editor has theme switcher buttons
    await expect(page.locator(".theme-switcher")).toBeVisible();
    await expect(page.locator("button[data-theme='sneak']")).toBeVisible();
    await expect(page.locator("button[data-theme='arch']")).toBeVisible();
    await expect(page.locator("button[data-theme='mech']")).toBeVisible();
  });
});

// ---------------------------------------------------------------------------
// Editor Features
// ---------------------------------------------------------------------------

test.describe("Editor Features", () => {
  test("save button exists", async ({ page }) => {
    const fileName = `save-test-${Date.now()}.txt`;

    await page.goto("/");
    await page.fill("#new-file-name", fileName);
    await page.click("#new-file-form button[type='submit']");
    await page.waitForURL(/\/(file|edit)\//);

    // Save button is rendered disabled in HTML but gets enabled
    // once the collab WebSocket connects and editor initializes.
    await expect(page.locator("#save-btn")).toBeVisible();
    await expect(page.locator("#save-btn")).toHaveText("save");
  });

  test("download dropdown exists", async ({ page }) => {
    const fileName = `dl-test-${Date.now()}.txt`;

    await page.goto("/");
    await page.fill("#new-file-name", fileName);
    await page.click("#new-file-form button[type='submit']");
    await page.waitForURL(/\/(file|edit)\//);

    await expect(page.locator("#download-btn")).toBeVisible();
  });

  test("editor container has data attributes", async ({ page }) => {
    const fileName = `data-test-${Date.now()}.txt`;

    await page.goto("/");
    await page.fill("#new-file-name", fileName);
    await page.click("#new-file-form button[type='submit']");
    await page.waitForURL(/\/(file|edit)\//);

    const container = page.locator("#editor-container");
    await expect(container).toHaveAttribute("data-doc-id", /.+/);
    await expect(container).toHaveAttribute("data-filename", /.+/);
  });
});
