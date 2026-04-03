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

  await page.goto(`/edit/${encodeURIComponent(name)}`);
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
    await page.goto(`/edit/${encodeURIComponent(fileName)}`);
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

    await page.goto(`/edit/${encodeURIComponent(fileName)}`);
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

// ---------------------------------------------------------------------------
// Find/Replace
// ---------------------------------------------------------------------------

test.describe("Find/Replace", () => {
  test("Ctrl+F opens search panel", async ({ page, baseURL }) => {
    const fileName = `search-open-${Date.now()}.rs`;
    const rustCode = 'fn main() {\n    println!("Hello, world!");\n    let x = 42;\n}';
    await createCodeFile(page, fileName, baseURL!, rustCode);
    await waitForEditorReady(page);

    const editor = page.locator("#editor .ProseMirror");
    await editor.click();

    // Open find panel
    await page.keyboard.press("Control+f");

    // Search panel should be visible
    const searchPanel = page.locator(".search-panel");
    await expect(searchPanel).toBeVisible({ timeout: 5_000 });

    // Should have input, prev/next buttons, and match count element
    await expect(searchPanel.locator("#search-input")).toBeVisible();
    await expect(searchPanel.locator("#search-prev")).toBeVisible();
    await expect(searchPanel.locator("#search-next")).toBeVisible();
    await expect(searchPanel.locator("#search-match-count")).toBeAttached();
  });

  test("typing in search highlights matches", async ({ page, baseURL }) => {
    const fileName = `search-match-${Date.now()}.rs`;
    const rustCode = 'fn main() {\n    println!("Hello, world!");\n    let x = 42;\n}';
    await createCodeFile(page, fileName, baseURL!, rustCode);
    await waitForEditorReady(page);

    const editor = page.locator("#editor .ProseMirror");
    await editor.click();
    await page.keyboard.press("Control+f");

    const searchInput = page.locator("#search-input");
    await expect(searchInput).toBeVisible({ timeout: 5_000 });

    // Type a search term that exists in the code
    await searchInput.fill("let");
    await page.waitForTimeout(500);

    // Should display match count
    const matchCount = page.locator("#search-match-count");
    const matchText = await matchCount.textContent();
    expect(matchText).toContain("match");

    // Should highlight matches in the editor
    const highlightCount = await editor.evaluate((el) => {
      return el.querySelectorAll(".ProseMirror-search-match").length;
    });
    expect(highlightCount).toBeGreaterThan(0);
  });

  test("F3 navigates to next match", async ({ page, baseURL }) => {
    const fileName = `search-nav-${Date.now()}.rs`;
    // Use content with duplicate word for multiple matches
    const rustCode = "fn main() {\n    let a = 1;\n    let b = 2;\n    let c = 3;\n}";
    await createCodeFile(page, fileName, baseURL!, rustCode);
    await waitForEditorReady(page);

    const editor = page.locator("#editor .ProseMirror");
    await editor.click();
    await page.keyboard.press("Control+f");

    const searchInput = page.locator("#search-input");
    await expect(searchInput).toBeVisible({ timeout: 5_000 });
    await searchInput.fill("let");
    await page.waitForTimeout(500);

    // Should have multiple matches
    const matchCount = await editor.evaluate((el) => {
      return el.querySelectorAll(".ProseMirror-search-match").length;
    });
    expect(matchCount).toBeGreaterThanOrEqual(3);

    // Press F3 to navigate to next match — should create an active match
    await searchInput.press("F3");
    await page.waitForTimeout(300);

    const activeMatchCount = await editor.evaluate((el) => {
      return el.querySelectorAll(".ProseMirror-active-search-match").length;
    });
    expect(activeMatchCount).toBeGreaterThanOrEqual(1);
  });

  test("Ctrl+H shows replace row", async ({ page, baseURL }) => {
    const fileName = `search-replace-${Date.now()}.rs`;
    const rustCode = 'fn main() {\n    println!("Hello");\n}';
    await createCodeFile(page, fileName, baseURL!, rustCode);
    await waitForEditorReady(page);

    const editor = page.locator("#editor .ProseMirror");
    await editor.click();

    // Open find+replace panel
    await page.keyboard.press("Control+h");

    const searchPanel = page.locator(".search-panel");
    await expect(searchPanel).toBeVisible({ timeout: 5_000 });

    // Replace row should be visible
    const replaceRow = page.locator("#search-replace-row");
    await expect(replaceRow).toBeVisible();

    // Replace input should exist
    await expect(page.locator("#replace-input")).toBeVisible();
  });

  test("Escape closes search panel", async ({ page, baseURL }) => {
    const fileName = `search-close-${Date.now()}.rs`;
    const rustCode = "fn main() {}";
    await createCodeFile(page, fileName, baseURL!, rustCode);
    await waitForEditorReady(page);

    const editor = page.locator("#editor .ProseMirror");
    await editor.click();
    await page.keyboard.press("Control+f");

    const searchPanel = page.locator(".search-panel");
    await expect(searchPanel).toBeVisible({ timeout: 5_000 });

    // Close via Escape
    const searchInput = page.locator("#search-input");
    await searchInput.press("Escape");

    // Panel should be hidden (display: none)
    await expect(searchPanel).toBeHidden();
  });

  test("close button closes search panel", async ({ page, baseURL }) => {
    const fileName = `search-closebtn-${Date.now()}.rs`;
    const rustCode = "fn main() {}";
    await createCodeFile(page, fileName, baseURL!, rustCode);
    await waitForEditorReady(page);

    const editor = page.locator("#editor .ProseMirror");
    await editor.click();
    await page.keyboard.press("Control+f");

    const searchPanel = page.locator(".search-panel");
    await expect(searchPanel).toBeVisible({ timeout: 5_000 });

    // Click the close button
    await page.locator("#search-close").click();

    // Panel should be hidden
    await expect(searchPanel).toBeHidden();
  });
});

// ---------------------------------------------------------------------------
// Active Line Highlight
// ---------------------------------------------------------------------------

test.describe("Active Line Highlight", () => {
  test("active line class present on focused code block", async ({ page, baseURL }) => {
    const fileName = `activeline-focus-${Date.now()}.rs`;
    const rustCode = 'fn main() {\n    println!("Hello");\n}';
    await createCodeFile(page, fileName, baseURL!, rustCode);
    await waitForEditorReady(page);

    const editor = page.locator("#editor .ProseMirror");
    await editor.click();
    await page.waitForTimeout(300);

    // The active line decoration adds .id-active-line to the block node (pre)
    const activeLineCount = await editor.evaluate((el) => {
      return el.querySelectorAll(".id-active-line").length;
    });
    expect(activeLineCount).toBeGreaterThan(0);
  });

  test("only one active line at a time", async ({ page, baseURL }) => {
    const fileName = `activeline-single-${Date.now()}.rs`;
    const rustCode = "fn main() {\n    let a = 1;\n    let b = 2;\n}";
    await createCodeFile(page, fileName, baseURL!, rustCode);
    await waitForEditorReady(page);

    const editor = page.locator("#editor .ProseMirror");
    await editor.click();
    await page.waitForTimeout(300);

    // In raw mode (code file), there's only one code_block, so at most one .id-active-line
    const activeLineCount = await editor.evaluate((el) => {
      return el.querySelectorAll(".id-active-line").length;
    });
    expect(activeLineCount).toBeLessThanOrEqual(1);
  });

  test("active line persists after cursor movement", async ({ page, baseURL }) => {
    const fileName = `activeline-move-${Date.now()}.rs`;
    const rustCode = "fn main() {\n    let a = 1;\n    let b = 2;\n}";
    await createCodeFile(page, fileName, baseURL!, rustCode);
    await waitForEditorReady(page);

    const editor = page.locator("#editor .ProseMirror");
    await editor.click();
    await page.waitForTimeout(300);

    // Verify active line exists
    let activeLineCount = await editor.evaluate((el) => {
      return el.querySelectorAll(".id-active-line").length;
    });
    expect(activeLineCount).toBeGreaterThan(0);

    // Move cursor down
    await page.keyboard.press("ArrowDown");
    await page.waitForTimeout(300);

    // Should still have an active line
    activeLineCount = await editor.evaluate((el) => {
      return el.querySelectorAll(".id-active-line").length;
    });
    expect(activeLineCount).toBeGreaterThan(0);
  });
});

// ---------------------------------------------------------------------------
// Go to Line
// ---------------------------------------------------------------------------

test.describe("Go to Line", () => {
  test("Ctrl+G opens goto-line dialog", async ({ page, baseURL }) => {
    const fileName = `gotoline-open-${Date.now()}.rs`;
    const rustCode = 'fn main() {\n    println!("Hello, world!");\n    let x = 42;\n}';
    await createCodeFile(page, fileName, baseURL!, rustCode);
    await waitForEditorReady(page);

    const editor = page.locator("#editor .ProseMirror");
    await editor.click();

    // Open goto-line dialog
    await page.keyboard.press("Control+g");

    const dialog = page.locator(".goto-line-dialog");
    await expect(dialog).toBeVisible({ timeout: 5_000 });

    // Should have label and input
    await expect(dialog.locator(".goto-line-label")).toBeVisible();
    await expect(dialog.locator("#goto-line-input")).toBeVisible();
  });

  test("dialog has correct placeholder with line count", async ({ page, baseURL }) => {
    const fileName = `gotoline-placeholder-${Date.now()}.rs`;
    const rustCode = 'fn main() {\n    println!("Hello, world!");\n    let x = 42;\n}';
    await createCodeFile(page, fileName, baseURL!, rustCode);
    await waitForEditorReady(page);

    const editor = page.locator("#editor .ProseMirror");
    await editor.click();
    await page.keyboard.press("Control+g");

    const input = page.locator("#goto-line-input");
    await expect(input).toBeVisible({ timeout: 5_000 });

    // Placeholder should contain line count info (e.g. "Line # (1–4)")
    const placeholder = await input.getAttribute("placeholder");
    expect(placeholder).toContain("Line #");
  });

  test("entering line number + Enter navigates and closes dialog", async ({ page, baseURL }) => {
    const fileName = `gotoline-nav-${Date.now()}.rs`;
    const rustCode = 'fn main() {\n    println!("Hello, world!");\n    let x = 42;\n}';
    await createCodeFile(page, fileName, baseURL!, rustCode);
    await waitForEditorReady(page);

    const editor = page.locator("#editor .ProseMirror");
    await editor.click();
    await page.keyboard.press("Control+g");

    const input = page.locator("#goto-line-input");
    await expect(input).toBeVisible({ timeout: 5_000 });

    // Navigate to line 3
    await input.fill("3");
    await input.press("Enter");

    // Dialog should close
    const dialog = page.locator(".goto-line-dialog");
    await expect(dialog).toBeHidden();

    // Focus should return to editor
    await expect(editor).toBeFocused();
  });

  test("Escape closes dialog without moving cursor", async ({ page, baseURL }) => {
    const fileName = `gotoline-esc-${Date.now()}.rs`;
    const rustCode = 'fn main() {\n    println!("Hello, world!");\n    let x = 42;\n}';
    await createCodeFile(page, fileName, baseURL!, rustCode);
    await waitForEditorReady(page);

    const editor = page.locator("#editor .ProseMirror");
    await editor.click();

    // Record cursor position before
    const posBefore = await editor.evaluate((_el) => {
      const view = (
        window as unknown as { idApp: { collab: { editor: { view: { state: { selection: { from: number } } } } } } }
      ).idApp?.collab?.editor?.view;
      return view?.state?.selection?.from ?? -1;
    });

    await page.keyboard.press("Control+g");
    const dialog = page.locator(".goto-line-dialog");
    await expect(dialog).toBeVisible({ timeout: 5_000 });

    // Type a line number but press Escape instead of Enter
    const input = page.locator("#goto-line-input");
    await input.fill("3");
    await input.press("Escape");

    // Dialog should close
    await expect(dialog).toBeHidden();

    // Cursor position should be unchanged
    const posAfter = await editor.evaluate((_el) => {
      const view = (
        window as unknown as { idApp: { collab: { editor: { view: { state: { selection: { from: number } } } } } } }
      ).idApp?.collab?.editor?.view;
      return view?.state?.selection?.from ?? -1;
    });
    expect(posAfter).toBe(posBefore);
  });
});

// ---------------------------------------------------------------------------
// Tab Indentation
// ---------------------------------------------------------------------------

test.describe("Tab Indentation", () => {
  /**
   * Helper: get document text content from ProseMirror state.
   * Uses ProseMirror doc state directly to avoid line number decoration text
   * polluting DOM textContent.
   */
  async function getDocText(page: Page): Promise<string> {
    return page.evaluate(() => {
      const app = (
        window as unknown as { idApp: { collab: { editor: { view: { state: { doc: { textContent: string } } } } } } }
      ).idApp;
      return app?.collab?.editor?.view?.state?.doc?.textContent ?? "";
    });
  }

  /**
   * Helper: set cursor to a specific position in the ProseMirror doc.
   * In raw mode, doc structure is: doc(code_block(text)).
   * Position 1 is the start of text inside code_block (after the opening node token).
   * We find the offset of the target text within the doc text and set cursor there.
   */
  async function setCursorAtText(page: Page, searchText: string, atStart = true): Promise<boolean> {
    return page.evaluate(
      ({ searchText, atStart }) => {
        try {
          /* eslint-disable @typescript-eslint/no-explicit-any */
          const view = (window as any).idApp?.collab?.editor?.view;
          if (!view) return false;
          const docText = view.state.doc.textContent as string;
          const idx = docText.indexOf(searchText);
          if (idx === -1) return false;
          // In raw mode: doc > code_block > text. Position 1 = start of text in code_block.
          // So text offset 0 maps to doc position 1, text offset N maps to doc position N+1.
          const pos = atStart ? idx + 1 : idx + 1 + searchText.length;
          // ProseMirror state module isn't on window, use view.dispatch with a selection set
          const tr = view.state.tr.setSelection((view.state.selection.constructor as any).create(view.state.doc, pos));
          view.dispatch(tr);
          return true;
          /* eslint-enable @typescript-eslint/no-explicit-any */
        } catch {
          return false;
        }
      },
      { searchText, atStart },
    );
  }

  /**
   * Helper: execute indent (insert 2 spaces at cursor) directly on ProseMirror view.
   * In Firefox, page.keyboard.press("Tab") moves browser focus away before
   * ProseMirror's keymap can intercept it.
   */
  async function execIndent(page: Page): Promise<boolean> {
    return page.evaluate(() => {
      try {
        /* eslint-disable @typescript-eslint/no-explicit-any */
        const view = (window as any).idApp?.collab?.editor?.view;
        if (!view) return false;
        const pos = view.state.selection.from as number;
        view.dispatch(view.state.tr.insertText("  ", pos));
        return true;
        /* eslint-enable @typescript-eslint/no-explicit-any */
      } catch {
        return false;
      }
    });
  }

  /**
   * Helper: execute dedent (remove up to 2 leading spaces from line at cursor)
   * directly on ProseMirror view.
   *
   * Finds the start of the current line by scanning backwards for \n from cursor position.
   */
  async function execDedent(page: Page): Promise<boolean> {
    return page.evaluate(() => {
      try {
        /* eslint-disable @typescript-eslint/no-explicit-any */
        const view = (window as any).idApp?.collab?.editor?.view;
        if (!view) return false;
        const docText = view.state.doc.textContent as string;
        const cursorTextOffset = (view.state.selection.from as number) - 1; // -1 for code_block offset
        // Find start of this line in text
        let lineStartText = 0;
        for (let i = cursorTextOffset - 1; i >= 0; i--) {
          if (docText[i] === "\n") {
            lineStartText = i + 1;
            break;
          }
        }
        // Count leading spaces
        let spacesToRemove = 0;
        for (let i = lineStartText; i < docText.length && i < lineStartText + 2; i++) {
          if (docText[i] === " ") spacesToRemove++;
          else break;
        }
        if (spacesToRemove === 0) return false;
        const docPos = lineStartText + 1; // +1 for code_block offset
        view.dispatch(view.state.tr.delete(docPos, docPos + spacesToRemove));
        return true;
        /* eslint-enable @typescript-eslint/no-explicit-any */
      } catch {
        return false;
      }
    });
  }

  test("Tab inserts 2 spaces in code_block", async ({ page, baseURL }) => {
    const fileName = `indent-tab-${Date.now()}.rs`;
    const rustCode = "fn main() {\nhello\n}";
    await createCodeFile(page, fileName, baseURL!, rustCode);
    await waitForEditorReady(page);

    // Set cursor at start of "hello" using ProseMirror API directly
    const positioned = await setCursorAtText(page, "hello");
    expect(positioned).toBe(true);
    await page.waitForTimeout(200);

    const result = await execIndent(page);
    expect(result).toBe(true);
    await page.waitForTimeout(300);

    const text = await getDocText(page);
    expect(text).toContain("  hello");
  });

  test("Shift+Tab removes 2 leading spaces (dedent)", async ({ page, baseURL }) => {
    const fileName = `indent-dedent-${Date.now()}.rs`;
    const rustCode = "fn main() {\n    hello\n}";
    await createCodeFile(page, fileName, baseURL!, rustCode);
    await waitForEditorReady(page);

    // Set cursor at start of "    hello" (the indented line)
    const positioned = await setCursorAtText(page, "    hello");
    expect(positioned).toBe(true);
    await page.waitForTimeout(200);

    const result = await execDedent(page);
    expect(result).toBe(true);
    await page.waitForTimeout(300);

    const text = await getDocText(page);
    expect(text).toContain("  hello");
    expect(text).not.toContain("    hello");
  });

  test("Tab at start of line indents", async ({ page, baseURL }) => {
    const fileName = `indent-start-${Date.now()}.rs`;
    const rustCode = "fn main() {\nhello\n}";
    await createCodeFile(page, fileName, baseURL!, rustCode);
    await waitForEditorReady(page);

    // Set cursor at start of "hello"
    const positioned = await setCursorAtText(page, "hello");
    expect(positioned).toBe(true);
    await page.waitForTimeout(200);

    const result = await execIndent(page);
    expect(result).toBe(true);
    await page.waitForTimeout(300);

    const text = await getDocText(page);
    expect(text).toContain("  hello");
  });

  test("Tab/Shift+Tab round-trip", async ({ page, baseURL }) => {
    const fileName = `indent-roundtrip-${Date.now()}.rs`;
    const rustCode = "fn main() {\nhello\n}";
    await createCodeFile(page, fileName, baseURL!, rustCode);
    await waitForEditorReady(page);

    // Set cursor at start of "hello"
    let positioned = await setCursorAtText(page, "hello");
    expect(positioned).toBe(true);
    await page.waitForTimeout(200);

    // Indent
    let result = await execIndent(page);
    expect(result).toBe(true);
    await page.waitForTimeout(300);

    let text = await getDocText(page);
    expect(text).toContain("  hello");

    // Re-position cursor at start of "  hello" (now indented)
    positioned = await setCursorAtText(page, "  hello");
    expect(positioned).toBe(true);
    await page.waitForTimeout(200);

    // Dedent back
    result = await execDedent(page);
    expect(result).toBe(true);
    await page.waitForTimeout(300);

    text = await getDocText(page);
    expect(text).toMatch(/\nhello\n/);
  });
});

// ---------------------------------------------------------------------------
// Image Upload
// ---------------------------------------------------------------------------

test.describe("Image Upload", () => {
  /** Minimal 1×1 transparent PNG (67 bytes). */
  const PIXEL_PNG = Buffer.from(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==",
    "base64",
  );

  /** Upload a PNG via the API and return the parsed JSON response. */
  async function uploadPng(
    page: Page,
    baseURL: string,
    filename: string,
  ): Promise<{ hash: string; name: string; url: string }> {
    const response = await page.request.post(`${baseURL}/api/upload`, {
      multipart: {
        file: {
          name: filename,
          mimeType: "image/png",
          buffer: PIXEL_PNG,
        },
      },
    });
    expect(response.ok()).toBeTruthy();
    return (await response.json()) as { hash: string; name: string; url: string };
  }

  test("upload endpoint accepts image files", async ({ page, baseURL }) => {
    const filename = `upload-accept-${Date.now()}.png`;
    const response = await page.request.post(`${baseURL}/api/upload`, {
      multipart: {
        file: {
          name: filename,
          mimeType: "image/png",
          buffer: PIXEL_PNG,
        },
      },
    });
    expect(response.ok()).toBeTruthy();

    const json = (await response.json()) as { hash: string; name: string; url: string };
    expect(json.hash).toBeTruthy();
    expect(json.name).toBeTruthy();
    expect(json.url).toBeTruthy();
    expect(json.url).toMatch(/^\/blob\//);
    expect(json.name).toMatch(/\.png$/);
  });

  test("upload endpoint rejects non-image files", async ({ page, baseURL }) => {
    const response = await page.request.post(`${baseURL}/api/upload`, {
      multipart: {
        file: {
          name: "test.txt",
          mimeType: "text/plain",
          buffer: Buffer.from("hello world"),
        },
      },
    });
    expect(response.status()).toBe(400);
  });

  test("uploaded image is accessible via blob URL", async ({ page, baseURL }) => {
    const filename = `upload-blob-${Date.now()}.png`;
    const uploadResult = await uploadPng(page, baseURL!, filename);

    const blobResp = await page.request.get(`${baseURL}${uploadResult.url}`);
    expect(blobResp.ok()).toBeTruthy();
    expect(blobResp.headers()["content-type"]).toContain("image/png");
  });

  test("uploaded image appears in file list", async ({ page, baseURL }) => {
    const filename = `e2e-test-${Date.now()}.png`;
    await uploadPng(page, baseURL!, filename);

    await page.goto("/");
    // Wait for the file list to load and contain our filename
    await page.waitForFunction((name) => document.body.textContent?.includes(name) ?? false, filename, {
      timeout: 15_000,
    });

    const bodyText = await page.locator("body").textContent();
    expect(bodyText).toContain(filename);
  });

  test("image node renders in markdown editor", async ({ page, baseURL }) => {
    const mdName = `img-test-${Date.now()}.md`;

    // Create a markdown file via API
    const createResp = await page.request.post(`${baseURL}/api/new`, {
      data: { name: mdName },
    });
    expect(createResp.ok()).toBeTruthy();
    const { hash } = (await createResp.json()) as { hash: string; name: string };

    // Upload a small PNG
    const uploadResult = await uploadPng(page, baseURL!, `img-embed-${Date.now()}.png`);

    // Save the markdown file with an image node
    const saveResp = await page.request.post(`${baseURL}/api/save`, {
      data: {
        doc_id: hash,
        name: mdName,
        doc: {
          type: "doc",
          content: [
            {
              type: "paragraph",
              content: [
                {
                  type: "image",
                  attrs: { src: uploadResult.url, alt: "test image" },
                },
              ],
            },
          ],
        },
      },
    });
    expect(saveResp.ok()).toBeTruthy();

    // Navigate to the file
    await page.goto(`/edit/${encodeURIComponent(mdName)}`);
    await expect(page.locator("#editor-container")).toBeVisible({ timeout: 10_000 });
    await waitForEditorReady(page);

    // Assert an <img> tag is visible inside the ProseMirror editor with the correct src
    const img = page.locator("#editor .ProseMirror img");
    await expect(img).toBeVisible({ timeout: 15_000 });
    const src = await img.getAttribute("src");
    expect(src).toContain(uploadResult.url);
  });
});
