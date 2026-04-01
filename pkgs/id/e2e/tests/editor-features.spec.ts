import { expect, type Page, test } from "@playwright/test";

/**
 * Editor Pretext Features E2E tests for the id web UI.
 *
 * Tests verify:
 * - Syntax highlighting (Shiki via prosemirror-highlight)
 * - Line numbers (prosemirror-highlight withLineNumbers())
 * - Word wrap toggle (Alt+Z)
 * - Line number toggle (Alt+L)
 * - Keyboard shortcut hints in editor footer
 *
 * Prerequisites:
 * - Web variant must be built first (`just build`)
 * - Server starts automatically via playwright.config.ts webServer
 */

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Create a file and navigate to its editor */
async function createFile(page: Page, name: string): Promise<void> {
  await page.goto("/");
  await page.fill("#new-file-name", name);
  await page.click("#new-file-form button[type='submit']");
  await page.waitForURL(/\/(file|edit)\//, { timeout: 15_000 });
  await expect(page.locator("#editor-container")).toBeVisible({ timeout: 10_000 });
}

/**
 * Create a file with unique content via API and navigate to its editor.
 *
 * This ensures each file gets a unique blob hash (avoiding shared collab documents
 * when multiple tests create empty files with the same hash). Uses raw doc format
 * (code_block) for code files.
 */
async function createCodeFile(page: Page, name: string, baseURL: string, content?: string): Promise<void> {
  const createResp = await page.request.post(`${baseURL}/api/new`, {
    data: { name },
  });
  expect(createResp.ok()).toBeTruthy();
  const { hash } = (await createResp.json()) as { hash: string; name: string };

  // Save unique content to get a unique blob hash → unique collab document
  const text = content ?? `unique-${name}-${Date.now()}`;
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

  await page.goto(`/file/${encodeURIComponent(name)}`);
  await expect(page.locator("#editor-container")).toBeVisible({ timeout: 10_000 });
}

/** Wait for the collab WebSocket to connect and editor to be ready */
async function waitForEditorReady(page: Page, timeout = 20_000): Promise<void> {
  await page.waitForFunction(
    () => {
      const app = (window as unknown as { idApp: { collab: { ws: WebSocket; editor: unknown } | null } }).idApp;
      if (!app?.collab?.ws || app.collab.ws.readyState !== WebSocket.OPEN) return false;
      if (!app.collab.editor) return false;
      return !!document.querySelector("#editor .ProseMirror");
    },
    { polling: 100, timeout },
  );
}

// ---------------------------------------------------------------------------
// Syntax Highlighting
// ---------------------------------------------------------------------------

test.describe("Syntax Highlighting", () => {
  test("code file has colored syntax spans", async ({ page, baseURL }) => {
    const fileName = `highlight-rs-${Date.now()}.rs`;
    const rustCode = 'fn main() { println!("hello"); }';

    // Create file with content via API to avoid collab version mismatch from rapid typing
    const createResp = await page.request.post(`${baseURL}/api/new`, {
      data: { name: fileName },
    });
    expect(createResp.ok()).toBeTruthy();
    const { hash } = (await createResp.json()) as { hash: string; name: string };

    // Save Rust code content so Shiki has something to highlight
    const saveResp = await page.request.post(`${baseURL}/api/save`, {
      data: {
        doc_id: hash,
        name: fileName,
        doc: {
          type: "doc",
          content: [{ type: "code_block", content: [{ type: "text", text: rustCode }] }],
        },
      },
    });
    expect(saveResp.ok()).toBeTruthy();

    // Navigate to the file editor
    await page.goto(`/file/${encodeURIComponent(fileName)}`);
    await expect(page.locator("#editor-container")).toBeVisible({ timeout: 10_000 });
    await waitForEditorReady(page);

    // Wait for Shiki to load Rust grammar and apply highlighting (async)
    // prosemirror-highlight creates inline decorations with style="color: ..."
    await page.waitForFunction(
      () => {
        const pm = document.querySelector("#editor .ProseMirror");
        if (!pm) return false;
        const spans = pm.querySelectorAll("span[style]");
        for (const span of spans) {
          if ((span as HTMLElement).style.color) return true;
        }
        return false;
      },
      { timeout: 20_000, polling: 250 },
    );

    // Verify colored spans exist
    const coloredSpanCount = await page.locator("#editor .ProseMirror").evaluate((el) => {
      const spans = el.querySelectorAll("span[style]");
      let count = 0;
      for (const span of spans) {
        if ((span as HTMLElement).style.color) count++;
      }
      return count;
    });
    expect(coloredSpanCount).toBeGreaterThan(0);
  });

  test("plain text file has no syntax highlighting", async ({ page }) => {
    const fileName = `highlight-txt-${Date.now()}.txt`;
    await createFile(page, fileName);
    await waitForEditorReady(page);

    // Focus and type plain text
    const editor = page.locator("#editor .ProseMirror");
    await editor.click();
    await page.keyboard.type("This is plain text with no highlighting.");

    // Wait a moment for any async processing to settle
    await page.waitForTimeout(2_000);

    // .txt files use rich schema (not raw/code_block), so no Shiki spans
    // Check that no syntax-colored spans exist
    const coloredSpanCount = await editor.evaluate((el) => {
      const spans = el.querySelectorAll("span[style]");
      let count = 0;
      for (const span of spans) {
        if ((span as HTMLElement).style.color) count++;
      }
      return count;
    });
    expect(coloredSpanCount).toBe(0);
  });

  test("highlighting persists after wrap toggle", async ({ page, baseURL }) => {
    const fileName = `highlight-wrap-${Date.now()}.js`;
    const jsCode = 'const x = "hello"; console.log(x);';

    // Create file with JS content via API
    const createResp = await page.request.post(`${baseURL}/api/new`, {
      data: { name: fileName },
    });
    expect(createResp.ok()).toBeTruthy();
    const { hash } = (await createResp.json()) as { hash: string; name: string };

    const saveResp = await page.request.post(`${baseURL}/api/save`, {
      data: {
        doc_id: hash,
        name: fileName,
        doc: {
          type: "doc",
          content: [{ type: "code_block", content: [{ type: "text", text: jsCode }] }],
        },
      },
    });
    expect(saveResp.ok()).toBeTruthy();

    await page.goto(`/file/${encodeURIComponent(fileName)}`);
    await expect(page.locator("#editor-container")).toBeVisible({ timeout: 10_000 });
    await waitForEditorReady(page);

    // Wait for highlighting
    await page.waitForFunction(
      () => {
        const pm = document.querySelector("#editor .ProseMirror");
        if (!pm) return false;
        const spans = pm.querySelectorAll("span[style]");
        for (const span of spans) {
          if ((span as HTMLElement).style.color) return true;
        }
        return false;
      },
      { timeout: 20_000, polling: 250 },
    );

    // Count colored spans before toggle
    const editor = page.locator("#editor .ProseMirror");
    const beforeCount = await editor.evaluate((el) => {
      const spans = el.querySelectorAll("span[style]");
      let count = 0;
      for (const span of spans) {
        if ((span as HTMLElement).style.color) count++;
      }
      return count;
    });
    expect(beforeCount).toBeGreaterThan(0);

    // Toggle wrap off and back on
    await editor.click();
    await page.keyboard.press("Alt+z");
    await page.waitForTimeout(500);
    await page.keyboard.press("Alt+z");
    await page.waitForTimeout(500);

    // Colored spans should still be present
    const afterCount = await editor.evaluate((el) => {
      const spans = el.querySelectorAll("span[style]");
      let count = 0;
      for (const span of spans) {
        if ((span as HTMLElement).style.color) count++;
      }
      return count;
    });
    expect(afterCount).toBeGreaterThan(0);
  });
});

// ---------------------------------------------------------------------------
// Line Numbers
// ---------------------------------------------------------------------------

test.describe("Line Numbers", () => {
  test("line numbers appear for code files", async ({ page, baseURL }) => {
    const fileName = `linenums-${Date.now()}.py`;
    await createCodeFile(page, fileName, baseURL!, "x = 1\ny = 2\nz = 3");
    await waitForEditorReady(page);

    // Wait for line numbers to appear (prosemirror-highlight withLineNumbers)
    await page.waitForFunction(
      () => {
        const pm = document.querySelector("#editor .ProseMirror");
        if (!pm) return false;
        return pm.querySelectorAll(".line-number").length > 0;
      },
      { timeout: 15_000, polling: 250 },
    );

    const editor = page.locator("#editor .ProseMirror");
    const lineNumbers = editor.locator(".line-number");
    await expect(lineNumbers.first()).toBeVisible();
  });

  test("line numbers show correct count", async ({ page, baseURL }) => {
    const fileName = `linecount-${Date.now()}.rs`;
    await createCodeFile(page, fileName, baseURL!, "fn a() {}\nfn b() {}\nfn c() {}");
    await waitForEditorReady(page);

    // Wait for line numbers
    await page.waitForFunction(
      () => {
        const pm = document.querySelector("#editor .ProseMirror");
        if (!pm) return false;
        return pm.querySelectorAll(".line-number").length >= 3;
      },
      { timeout: 15_000, polling: 250 },
    );

    // Should have at least 3 line number elements
    const editor = page.locator("#editor .ProseMirror");
    const count = await editor.evaluate((el) => el.querySelectorAll(".line-number").length);
    expect(count).toBeGreaterThanOrEqual(3);
  });

  test("Alt+L toggles line number visibility", async ({ page, baseURL }) => {
    const fileName = `linetoggle-${Date.now()}.rs`;
    await createCodeFile(page, fileName, baseURL!, "let x = 1;\nlet y = 2;");
    await waitForEditorReady(page);

    const editor = page.locator("#editor .ProseMirror");

    // Wait for line numbers to appear
    await page.waitForFunction(
      () => {
        const pm = document.querySelector("#editor .ProseMirror");
        if (!pm) return false;
        return pm.querySelectorAll(".line-number").length > 0;
      },
      { timeout: 15_000, polling: 250 },
    );

    // Verify line numbers are visible initially
    await expect(editor.locator(".line-number").first()).toBeVisible();

    // Focus editor then press Alt+L to hide line numbers
    await editor.click();
    await page.keyboard.press("Alt+l");

    // ProseMirror element should now have the hide class
    await expect(editor).toHaveClass(/id-editor-no-line-numbers/);

    // Line numbers should be hidden (display: none via CSS)
    await expect(editor.locator(".line-number").first()).toBeHidden();
  });

  test("Alt+L round-trip restores line numbers", async ({ page, baseURL }) => {
    const fileName = `lineround-${Date.now()}.rs`;
    await createCodeFile(page, fileName, baseURL!, "let a = 1;");
    await waitForEditorReady(page);

    const editor = page.locator("#editor .ProseMirror");

    // Wait for line numbers
    await page.waitForFunction(
      () => {
        const pm = document.querySelector("#editor .ProseMirror");
        if (!pm) return false;
        return pm.querySelectorAll(".line-number").length > 0;
      },
      { timeout: 15_000, polling: 250 },
    );

    // Focus editor then toggle off
    await editor.click();
    await page.keyboard.press("Alt+l");
    await expect(editor).toHaveClass(/id-editor-no-line-numbers/);

    // Toggle back on
    await page.keyboard.press("Alt+l");

    // Class should be removed
    const hasNoLineClass = await editor.evaluate((el) => el.classList.contains("id-editor-no-line-numbers"));
    expect(hasNoLineClass).toBe(false);

    // Line numbers should be visible again
    await expect(editor.locator(".line-number").first()).toBeVisible();
  });
});

// ---------------------------------------------------------------------------
// Word Wrap
// ---------------------------------------------------------------------------

test.describe("Word Wrap", () => {
  test("editor starts in wrap mode", async ({ page }) => {
    const fileName = `wrapdefault-${Date.now()}.rs`;
    await createFile(page, fileName);
    await waitForEditorReady(page);

    const editor = page.locator("#editor .ProseMirror");
    await expect(editor).toHaveClass(/id-editor-wrap/);
  });

  test("Alt+Z toggles to nowrap mode", async ({ page }) => {
    const fileName = `wraptoggle-${Date.now()}.rs`;
    await createFile(page, fileName);
    await waitForEditorReady(page);

    const editor = page.locator("#editor .ProseMirror");
    await editor.click();

    // Should start with wrap class
    await expect(editor).toHaveClass(/id-editor-wrap/);

    // Toggle to nowrap
    await page.keyboard.press("Alt+z");

    // Should now have nowrap class
    await expect(editor).toHaveClass(/id-editor-nowrap/);

    // Should not have wrap class
    const hasWrapClass = await editor.evaluate((el) => el.classList.contains("id-editor-wrap"));
    expect(hasWrapClass).toBe(false);
  });

  test("Alt+Z round-trip restores wrap mode", async ({ page }) => {
    const fileName = `wrapround-${Date.now()}.rs`;
    await createFile(page, fileName);
    await waitForEditorReady(page);

    const editor = page.locator("#editor .ProseMirror");
    await editor.click();

    // Start: wrap
    await expect(editor).toHaveClass(/id-editor-wrap/);

    // Toggle to nowrap
    await page.keyboard.press("Alt+z");
    await expect(editor).toHaveClass(/id-editor-nowrap/);

    // Toggle back to wrap
    await page.keyboard.press("Alt+z");
    await expect(editor).toHaveClass(/id-editor-wrap/);

    // Verify nowrap class is removed
    const hasNowrapClass = await editor.evaluate((el) => el.classList.contains("id-editor-nowrap"));
    expect(hasNowrapClass).toBe(false);
  });

  test("nowrap mode has correct CSS properties", async ({ page }) => {
    const fileName = `nowrapcss-${Date.now()}.rs`;
    await createFile(page, fileName);
    await waitForEditorReady(page);

    const editor = page.locator("#editor .ProseMirror");
    await editor.click();

    // Toggle to nowrap
    await page.keyboard.press("Alt+z");
    await expect(editor).toHaveClass(/id-editor-nowrap/);

    // Check computed CSS properties
    const styles = await editor.evaluate((el) => {
      const computed = window.getComputedStyle(el);
      return {
        whiteSpace: computed.whiteSpace,
        overflowX: computed.overflowX,
      };
    });

    expect(styles.whiteSpace).toBe("pre");
    expect(styles.overflowX).toBe("auto");
  });

  test("wrap mode has correct CSS properties", async ({ page }) => {
    const fileName = `wrapcss-${Date.now()}.rs`;
    await createFile(page, fileName);
    await waitForEditorReady(page);

    const editor = page.locator("#editor .ProseMirror");

    // Default is wrap mode
    await expect(editor).toHaveClass(/id-editor-wrap/);

    // Check computed CSS properties
    const styles = await editor.evaluate((el) => {
      const computed = window.getComputedStyle(el);
      return {
        whiteSpace: computed.whiteSpace,
      };
    });

    expect(styles.whiteSpace).toBe("pre-wrap");
  });
});

// ---------------------------------------------------------------------------
// Keyboard Shortcut Hints
// ---------------------------------------------------------------------------

test.describe("Keyboard Shortcut Hints", () => {
  test("footer shows wrap shortcut hint", async ({ page }) => {
    const fileName = `footerwrap-${Date.now()}.rs`;
    await createFile(page, fileName);

    // The editor inline footer contains keyboard shortcut hints
    const footer = page.locator(".editor-inline-footer");
    await expect(footer).toBeVisible({ timeout: 10_000 });
    await expect(footer.locator("text=Alt+Z")).toBeVisible();
  });

  test("footer shows line number shortcut hint", async ({ page }) => {
    const fileName = `footerline-${Date.now()}.rs`;
    await createFile(page, fileName);

    const footer = page.locator(".editor-inline-footer");
    await expect(footer).toBeVisible({ timeout: 10_000 });
    await expect(footer.locator("text=Alt+L")).toBeVisible();
  });
});
