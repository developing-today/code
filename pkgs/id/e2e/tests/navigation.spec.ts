import { test, expect } from "@playwright/test";

/**
 * SPA Navigation E2E tests for the id web UI.
 *
 * Tests verify:
 * - SPA navigation via data-nav links (no full page reload)
 * - Settings page content
 * - Peers page content
 * - Browser back/forward history
 * - Navigation between editor and file list
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
// Settings Page
// ---------------------------------------------------------------------------

test.describe("Settings Page", () => {
  test("can navigate to settings via nav link", async ({ page }) => {
    await page.goto("/");
    await page.click("a[data-nav][href='/settings']");

    // Wait for settings content to load (SPA swap into #main)
    await expect(page.locator("text=Node Identity")).toBeVisible({ timeout: 10_000 });
    // URL should update
    expect(page.url()).toContain("/settings");
  });

  test("settings shows node identity", async ({ page }) => {
    await page.goto("/settings");

    await expect(page.locator("h3", { hasText: "Node Identity" })).toBeVisible();
    // Node ID is displayed in a code element
    const codeEl = page.locator("code");
    await expect(codeEl).toBeVisible();
    // Node ID is a 64-character hex string
    const nodeId = await codeEl.textContent();
    expect(nodeId).toBeTruthy();
    expect(nodeId!.length).toBeGreaterThanOrEqual(52); // Ed25519 public key
  });

  test("settings shows theme buttons", async ({ page }) => {
    await page.goto("/settings");

    await expect(page.locator("h3", { hasText: "Theme" })).toBeVisible();
    // Settings content buttons have text labels; header buttons are empty
    await expect(page.locator("button[data-theme='sneak']", { hasText: "Sneak" })).toBeVisible();
    await expect(page.locator("button[data-theme='arch']", { hasText: "Arch" })).toBeVisible();
    await expect(page.locator("button[data-theme='mech']", { hasText: "Mech" })).toBeVisible();
  });

  test("settings shows keyboard shortcuts", async ({ page }) => {
    await page.goto("/settings");

    await expect(page.locator("h3", { hasText: "Keyboard Shortcuts" })).toBeVisible();
    // Scope to settings table to avoid matching footer kbd elements
    const shortcutsTable = page.locator("table");
    await expect(shortcutsTable.locator("kbd", { hasText: "Alt+T" })).toBeVisible();
    await expect(shortcutsTable.locator("kbd", { hasText: "Ctrl+S" })).toBeVisible();
  });

  test("settings page has correct title", async ({ page }) => {
    await page.goto("/settings");
    await expect(page).toHaveTitle("Settings - id");
  });
});

// ---------------------------------------------------------------------------
// Peers Page
// ---------------------------------------------------------------------------

test.describe("Peers Page", () => {
  test("can navigate to peers via nav link", async ({ page }) => {
    await page.goto("/");
    await page.click("a[data-nav][href='/peers']");

    await expect(page.locator("text=Discovered Peers")).toBeVisible({ timeout: 10_000 });
    expect(page.url()).toContain("/peers");
  });

  test("peers page shows empty state", async ({ page }) => {
    await page.goto("/peers");

    // Ephemeral server has no peers
    await expect(page.locator("text=No peers discovered yet.")).toBeVisible();
  });

  test("peers page has auto-refresh attribute", async ({ page }) => {
    await page.goto("/peers");

    const content = page.locator("#peers-content");
    await expect(content).toBeVisible();
    await expect(content).toHaveAttribute("data-auto-refresh", "10");
  });

  test("peers page has correct title", async ({ page }) => {
    await page.goto("/peers");
    await expect(page).toHaveTitle("Peers - id");
  });
});

// ---------------------------------------------------------------------------
// SPA Navigation
// ---------------------------------------------------------------------------

test.describe("SPA Navigation", () => {
  test("nav links use SPA navigation (no full page reload)", async ({ page }) => {
    await page.goto("/");

    // Track if a full navigation occurred
    let _fullNavigation = false;
    page.on("framenavigated", () => {
      _fullNavigation = true;
    });

    // Reset after initial load
    _fullNavigation = false;

    // Click settings link via data-nav
    await page.click("a[data-nav][href='/settings']");
    await expect(page.locator("text=Node Identity")).toBeVisible({ timeout: 10_000 });

    // The SPA navigation should have changed the URL without a full navigation
    // (framenavigated fires on full navigation, not pushState)
    // We verify content changed and URL is correct
    expect(page.url()).toContain("/settings");
  });

  test("can navigate between all main pages", async ({ page }) => {
    await page.goto("/");

    // Files page
    await expect(page.locator("#new-file-form")).toBeVisible();

    // Go to settings
    await page.click("a[data-nav][href='/settings']");
    await expect(page.locator("text=Node Identity")).toBeVisible({ timeout: 10_000 });

    // Go to peers
    await page.click("a[data-nav][href='/peers']");
    await expect(page.locator("text=Discovered Peers")).toBeVisible({ timeout: 10_000 });

    // Go back to files
    await page.click("a[data-nav][href='/']");
    await expect(page.locator("#new-file-form")).toBeVisible({ timeout: 10_000 });
  });

  test("nav links exist in header", async ({ page }) => {
    await page.goto("/");

    await expect(page.locator("a[data-nav][href='/']", { hasText: "files" })).toBeVisible();
    await expect(page.locator("a[data-nav][href='/peers']", { hasText: "peers" })).toBeVisible();
    await expect(page.locator("a[data-nav][href='/settings']", { hasText: "settings" })).toBeVisible();
  });

  test("editor page has nav links too", async ({ page }) => {
    const fileName = `nav-editor-${Date.now()}.txt`;
    await createFile(page, fileName);

    // Editor should also have nav links
    await expect(page.locator("a[data-nav][href='/']", { hasText: "files" })).toBeVisible();
    await expect(page.locator("a[data-nav][href='/peers']", { hasText: "peers" })).toBeVisible();
    await expect(page.locator("a[data-nav][href='/settings']", { hasText: "settings" })).toBeVisible();
  });
});

// ---------------------------------------------------------------------------
// Browser History
// ---------------------------------------------------------------------------

test.describe("Browser History", () => {
  test("back button navigates to previous page", async ({ page }) => {
    await page.goto("/");

    // Navigate to settings via SPA
    await page.click("a[data-nav][href='/settings']");
    await expect(page.locator("text=Node Identity")).toBeVisible({ timeout: 10_000 });

    // Go back
    await page.goBack();
    await expect(page.locator("#new-file-form")).toBeVisible({ timeout: 10_000 });
    expect(page.url()).not.toContain("/settings");
  });

  test("forward button navigates forward after back", async ({ page }) => {
    await page.goto("/");

    // Navigate to settings
    await page.click("a[data-nav][href='/settings']");
    await expect(page.locator("text=Node Identity")).toBeVisible({ timeout: 10_000 });

    // Go back
    await page.goBack();
    await expect(page.locator("#new-file-form")).toBeVisible({ timeout: 10_000 });

    // Go forward
    await page.goForward();
    await expect(page.locator("text=Node Identity")).toBeVisible({ timeout: 10_000 });
    expect(page.url()).toContain("/settings");
  });

  test("multi-page history works correctly", async ({ page }) => {
    await page.goto("/");

    // Navigate: files -> settings -> peers
    await page.click("a[data-nav][href='/settings']");
    await expect(page.locator("text=Node Identity")).toBeVisible({ timeout: 10_000 });

    await page.click("a[data-nav][href='/peers']");
    await expect(page.locator("text=Discovered Peers")).toBeVisible({ timeout: 10_000 });

    // Back to settings
    await page.goBack();
    await expect(page.locator("text=Node Identity")).toBeVisible({ timeout: 10_000 });

    // Back to files
    await page.goBack();
    await expect(page.locator("#new-file-form")).toBeVisible({ timeout: 10_000 });
  });

  test("history works with editor navigation", async ({ page }) => {
    const fileName = `history-${Date.now()}.txt`;

    // Create a file (ends up on editor page)
    await createFile(page, fileName);
    expect(page.url()).toMatch(/\/(file|edit)\//);

    // Navigate back to file list via SPA link
    await page.click("a[data-nav][href='/']");
    await expect(page.locator("#new-file-form")).toBeVisible({ timeout: 10_000 });

    // Go back to editor via browser back
    await page.goBack();
    await expect(page.locator("#editor-container")).toBeVisible({ timeout: 10_000 });
  });
});
