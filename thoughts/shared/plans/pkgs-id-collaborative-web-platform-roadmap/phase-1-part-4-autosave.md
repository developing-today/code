# Phase 1 Part 4: Auto-save on Idle — Implementation Plan

**Goal:** Add debounced auto-save (2s after last edit) with visual state indicator and rate-limit retry, fixing the save button disabled-forever bug.

**Architecture:** State-machine-driven `AutoSaveManager` class added to `main.ts` that hooks into the existing `editor:change` custom event and `onNewVersion` collab callback. `saveFile()` is refactored to return a result object. All logic stays in `main.ts` — no new modules needed. E2E tests go in a new `autosave.spec.ts`.

**Design:** [thoughts/shared/designs/2026-04-02-autosave-on-idle-design.md](../../designs/2026-04-02-autosave-on-idle-design.md)

---

## Important Notes for Implementer

- **Single file modification**: ALL TypeScript changes happen in `pkgs/id/web/src/main.ts`
- **Template change**: ONE line in `pkgs/id/src/web/templates.rs` (remove `onclick` from save button)
- **New E2E test file**: `pkgs/id/e2e/tests/autosave.spec.ts`
- **Commit after every task** — each task is a meaningful, independently verifiable step
- **Verification**: Run `just id check` from repo root after each task
- **Never revert, force push, or rebase**

## Current Code Landmarks (main.ts)

- **Line ~25**: `IdApp` interface — `saveFile: () => Promise<void>;`
- **Line ~982**: `openEditor()` method
- **Line ~1016-1047**: `initCollab()` call with editor-ready callback (line ~1024) and onNewVersion callback (line ~1038)
- **Line ~1031-1032**: Save button enable in editor-ready callback
- **Line ~1055**: `closeEditor()` method
- **Line ~1075-1143**: `saveFile()` method (current)
- **Line ~1095**: `saveBtn` lookup inside saveFile
- **Line ~1099**: `saveBtn.disabled = true` — THE BUG (never re-enabled)
- **Line ~1446-1453**: Ctrl+S keydown listener calling `app.saveFile()`
- **Line ~437 in templates.rs**: Save button HTML: `<button ... id="save-btn" ... onclick="window.idApp?.saveFile?.()" disabled>save</button>`

---

## Dependency Graph

```
Task 1 (saveFile refactor + bug fix) — no deps
  ↓
Task 2 (AutoSaveManager class) — depends on Task 1 (uses SaveResult type)
  ↓
Task 3 (wiring: openEditor, closeEditor, Ctrl+S, save button) — depends on Task 2
  ↓
Task 4 (template update: remove onclick from save button) — depends on Task 3
  ↓
Task 5 (E2E tests) — depends on Task 4
```

> Because Tasks 1-4 all modify `main.ts` (or one line in `templates.rs`), they MUST run sequentially.
> Task 5 creates a new file and can conceptually run in parallel with Task 4, but practically needs the feature to be complete.

---

## Task 1: Refactor saveFile() to return SaveResult + fix disabled-forever bug

**File:** `pkgs/id/web/src/main.ts`
**Test:** none (verified by Task 5 E2E tests)
**Depends:** none

### What to change

1. Add a `SaveResult` type alias near the top of the file (after the imports, before the `IdApp` interface):

```typescript
/** Result from saveFile() for AutoSaveManager to process */
type SaveResult = { ok: true } | { ok: false; retryAfterMs?: number };
```

2. Change the `saveFile` return type in the `IdApp` interface (line ~25):

```typescript
// BEFORE:
saveFile: () => Promise<void>;

// AFTER:
saveFile: () => Promise<SaveResult>;
```

3. Rewrite the `saveFile()` method body (line ~1075-1143). The new implementation:
   - Returns `SaveResult` instead of `void`
   - Removes ALL button text management (AutoSaveManager will handle it in Task 3)
   - Adds `finally` block that always re-enables save button (fixes the disabled-forever bug)
   - On 429: parses retry delay from response body, returns `{ ok: false, retryAfterMs }`
   - On success: returns `{ ok: true }`
   - On error: returns `{ ok: false }`

Replace the entire `saveFile()` method with:

```typescript
    async saveFile(): Promise<SaveResult> {
      if (!this.collab?.editor) {
        console.warn("[id] No editor to save");
        return { ok: false };
      }

      const editorContainer = document.getElementById("editor-container");
      if (!editorContainer) return { ok: false };

      const filenameEncoded = editorContainer.dataset.docId;
      const filename = filenameEncoded ? decodeURIComponent(filenameEncoded) : null;
      const hash = editorContainer.dataset.hash;

      if (!filename || !hash) {
        console.error("[id] Missing filename or hash for save");
        return { ok: false };
      }

      // Get current editor state
      const state = getEditorState(this.collab.editor.view);
      const saveBtn = document.getElementById("save-btn") as HTMLButtonElement | null;

      try {
        if (saveBtn) {
          saveBtn.disabled = true;
          saveBtn.textContent = "saving\u2026";
        }

        const response = await fetch("/api/save", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            doc_id: hash,
            name: filename,
            doc: state.doc,
          }),
        });

        if (response.status === 429) {
          const errorText = await response.text();
          console.warn("[id] Save rate limited:", errorText);
          // Parse "Save rate limited. Try again in Xs." → extract seconds
          const match = errorText.match(/(\d+)s/);
          const serverDelaySec = match ? Number.parseInt(match[1], 10) : 5;
          const RATE_LIMIT_BUFFER_MS = 500;
          return { ok: false, retryAfterMs: serverDelaySec * 1000 + RATE_LIMIT_BUFFER_MS };
        }

        if (!response.ok) {
          const errorText = await response.text();
          console.error("[id] Save failed:", errorText);
          return { ok: false };
        }

        const result = (await response.json()) as { hash: string; name: string; archive_name: string | null };
        console.log("[id] File saved:", result);

        // Update the hash in the container (doc_id stays as filename)
        editorContainer.dataset.hash = result.hash;

        return { ok: true };
      } catch (err) {
        console.error("[id] Save error:", err);
        return { ok: false };
      } finally {
        // Always re-enable save button — fixes the disabled-forever bug
        if (saveBtn) saveBtn.disabled = false;
      }
    },
```

4. Temporarily update the Ctrl+S handler (line ~1448-1453) so it still works without AutoSaveManager. Since `saveFile()` now returns a result instead of managing button text, add minimal button feedback:

```typescript
    // BEFORE:
    if ((event.ctrlKey || event.metaKey) && event.key === "s") {
      event.preventDefault();
      if (app.collab?.editor) {
        app.saveFile();
      }
      return;
    }

    // AFTER (temporary — will be replaced in Task 3):
    if ((event.ctrlKey || event.metaKey) && event.key === "s") {
      event.preventDefault();
      if (app.collab?.editor) {
        app.saveFile().then((result) => {
          const btn = document.getElementById("save-btn") as HTMLButtonElement | null;
          if (!btn) return;
          if (result.ok) {
            btn.textContent = "saved \u2713";
            setTimeout(() => { btn.textContent = "save"; }, 2000);
          } else if (!result.retryAfterMs) {
            btn.textContent = "error!";
            setTimeout(() => { btn.textContent = "save"; }, 2000);
          }
        });
      }
      return;
    }
```

**Verify:** `just id check` from repo root
**Commit:** `fix(web): refactor saveFile to return SaveResult and fix disabled-forever bug`

---

## Task 2: Add AutoSaveManager class

**File:** `pkgs/id/web/src/main.ts`
**Test:** none (verified by Task 5 E2E tests)
**Depends:** Task 1

### What to change

Add the `AutoSaveManager` class to `main.ts`, AFTER the `SaveResult` type definition and BEFORE the `IdApp` interface. This class manages the save state machine.

Insert this code block:

```typescript
// =============================================================================
// Auto-save Manager
// =============================================================================

const AUTOSAVE_DEBOUNCE_MS = 2000;

type SaveState = "idle" | "unsaved" | "saving" | "saved" | "rate-limited" | "error";

class AutoSaveManager {
  state: SaveState = "idle";
  private debounceTimer: ReturnType<typeof setTimeout> | null = null;
  private retryTimer: ReturnType<typeof setTimeout> | null = null;
  private savedResetTimer: ReturnType<typeof setTimeout> | null = null;
  private saveFn: () => Promise<SaveResult>;

  constructor(saveFn: () => Promise<SaveResult>) {
    this.saveFn = saveFn;
  }

  /** Called when user makes a local edit (editor:change event) */
  onContentChange(): void {
    // Clear any pending timers
    if (this.debounceTimer !== null) {
      clearTimeout(this.debounceTimer);
      this.debounceTimer = null;
    }
    if (this.savedResetTimer !== null) {
      clearTimeout(this.savedResetTimer);
      this.savedResetTimer = null;
    }

    this.state = "unsaved";
    this.updateIndicator();

    // Start debounce — save after 2s of no edits
    this.debounceTimer = setTimeout(() => {
      this.debounceTimer = null;
      this.triggerSave();
    }, AUTOSAVE_DEBOUNCE_MS);
  }

  /** Called when another client saves (NewVersion received) */
  onNewVersion(): void {
    // Cancel any pending save — their version is newer
    if (this.debounceTimer !== null) {
      clearTimeout(this.debounceTimer);
      this.debounceTimer = null;
    }
    if (this.retryTimer !== null) {
      clearTimeout(this.retryTimer);
      this.retryTimer = null;
    }
    if (this.savedResetTimer !== null) {
      clearTimeout(this.savedResetTimer);
      this.savedResetTimer = null;
    }

    this.state = "saved";
    this.updateIndicator();

    // Reset indicator after 2s
    this.savedResetTimer = setTimeout(() => {
      this.savedResetTimer = null;
      if (this.state === "saved") {
        this.state = "idle";
        this.updateIndicator();
      }
    }, 2000);
  }

  /** Manual save — Ctrl+S or button click. Cancels debounce and saves immediately. */
  saveNow(): void {
    // Cancel debounce timer — we're saving right now
    if (this.debounceTimer !== null) {
      clearTimeout(this.debounceTimer);
      this.debounceTimer = null;
    }
    // Don't save if already saving (button is disabled anyway)
    if (this.state === "saving" || this.state === "rate-limited") {
      return;
    }
    this.triggerSave();
  }

  /** Clean up all timers (called when editor closes) */
  cancel(): void {
    if (this.debounceTimer !== null) {
      clearTimeout(this.debounceTimer);
      this.debounceTimer = null;
    }
    if (this.retryTimer !== null) {
      clearTimeout(this.retryTimer);
      this.retryTimer = null;
    }
    if (this.savedResetTimer !== null) {
      clearTimeout(this.savedResetTimer);
      this.savedResetTimer = null;
    }
    this.state = "idle";
    this.updateIndicator();
  }

  /** Execute the save and handle the result */
  private async triggerSave(): Promise<void> {
    this.state = "saving";
    this.updateIndicator();

    const result = await this.saveFn();
    this.onSaveResult(result);
  }

  /** Transition state based on save outcome */
  private onSaveResult(result: SaveResult): void {
    if (result.ok) {
      this.state = "saved";
      this.updateIndicator();

      // Reset to idle after 2s
      this.savedResetTimer = setTimeout(() => {
        this.savedResetTimer = null;
        if (this.state === "saved") {
          this.state = "idle";
          this.updateIndicator();
        }
      }, 2000);
    } else if (result.retryAfterMs) {
      // Rate limited — schedule retry
      this.state = "rate-limited";
      this.updateIndicator();

      this.retryTimer = setTimeout(() => {
        this.retryTimer = null;
        this.triggerSave();
      }, result.retryAfterMs);
    } else {
      // Generic error — don't auto-retry (prevents infinite loops on network outage)
      this.state = "error";
      this.updateIndicator();

      // Show error for 2s, then revert to "save •" (content is still unsaved)
      this.savedResetTimer = setTimeout(() => {
        this.savedResetTimer = null;
        if (this.state === "error") {
          this.state = "unsaved";
          this.updateIndicator();
        }
      }, 2000);
    }
  }

  /** Update the save button text/state to reflect current state */
  updateIndicator(): void {
    const saveBtn = document.getElementById("save-btn") as HTMLButtonElement | null;
    if (!saveBtn) return;

    switch (this.state) {
      case "idle":
        saveBtn.textContent = "save";
        saveBtn.disabled = false;
        break;
      case "unsaved":
        saveBtn.textContent = "save \u2022";
        saveBtn.disabled = false;
        break;
      case "saving":
        saveBtn.textContent = "saving\u2026";
        saveBtn.disabled = true;
        break;
      case "saved":
        saveBtn.textContent = "saved \u2713";
        saveBtn.disabled = false;
        break;
      case "rate-limited":
        saveBtn.textContent = "retry\u2026";
        saveBtn.disabled = true;
        break;
      case "error":
        saveBtn.textContent = "error!";
        saveBtn.disabled = false;
        break;
    }
  }
}
```

**Important Unicode characters used:**
- `\u2022` = `•` (bullet, for "save •")
- `\u2713` = `✓` (checkmark, for "saved ✓")
- `\u2026` = `…` (ellipsis, for "saving…" and "retry…")

**Verify:** `just id check` from repo root
**Commit:** `feat(web): add AutoSaveManager state machine class`

---

## Task 3: Wire AutoSaveManager into openEditor, closeEditor, Ctrl+S, and save button

**File:** `pkgs/id/web/src/main.ts`
**Test:** none (verified by Task 5 E2E tests)
**Depends:** Task 2

### What to change

This task integrates the `AutoSaveManager` into the existing app lifecycle. Four areas need changes:

#### 3a. Add autoSave field to the app object

Find the app object literal (it's a large object with methods like `openEditor`, `closeEditor`, `saveFile`, etc., assigned to `window.idApp`). Add a field for the AutoSaveManager instance.

In the `IdApp` interface, add:

```typescript
// Add after the existing fields (after line ~42 `lastFilePath: string | null;`):
autoSave: AutoSaveManager | null;
```

In the app object literal initialization, add:

```typescript
autoSave: null,
```

#### 3b. Create AutoSaveManager in openEditor()

In the `openEditor()` method, inside the editor-ready callback (the callback starting at line ~1024 with `(editor: EditorInstance) => {`), AFTER the save button enable and tag loading lines, add:

```typescript
            // Create AutoSaveManager
            this.autoSave = new AutoSaveManager(() => this.saveFile());

            // Listen for local content changes to trigger auto-save
            const editorContainer = document.getElementById("editor-container");
            if (editorContainer) {
              editorContainer.addEventListener("editor:change", this._onEditorChange);
            }
```

Also add a bound handler method to the app object (to enable removing the listener later). Add this as a method on the app object:

```typescript
    _onEditorChange(): void {
      if (this.autoSave) {
        this.autoSave.onContentChange();
      }
    },
```

**Important:** The `_onEditorChange` method needs to be bound to the app object. Since the app is a plain object literal (not a class), the `this` binding for event listeners won't work automatically. Instead, define `_onEditorChange` as an arrow function stored on the app object, OR bind it during init. The simplest approach: store a bound reference.

Actually, the cleanest approach given the existing code style (plain object literal) is to NOT use a method at all. Instead, create the event handler as a closure in `openEditor()`:

```typescript
            // Create AutoSaveManager and wire editor:change listener
            this.autoSave = new AutoSaveManager(() => this.saveFile());
            const onEditorChange = () => this.autoSave?.onContentChange();
            const editorContainer = document.getElementById("editor-container");
            if (editorContainer) {
              editorContainer.addEventListener("editor:change", onEditorChange);
            }
            // Store reference for cleanup in closeEditor
            this._editorChangeHandler = onEditorChange;
```

Add `_editorChangeHandler` to the `IdApp` interface:

```typescript
_editorChangeHandler: (() => void) | null;
```

And initialize it in the object literal:

```typescript
_editorChangeHandler: null,
```

#### 3c. Extend onNewVersion callback

In the `onNewVersion` callback (line ~1038), add `this.autoSave?.onNewVersion()`:

```typescript
          (hash: string, _name: string) => {
            // NewVersion callback — update the stored hash so the next save
            // sends the correct hash for archiving, without touching the doc_id (filename)
            console.log("[id] NewVersion received: updating hash to", hash);
            const editorContainer = document.getElementById("editor-container");
            if (editorContainer) {
              editorContainer.dataset.hash = hash;
            }
            // Cancel any pending auto-save — their version is newer
            this.autoSave?.onNewVersion();
          },
```

#### 3d. Clean up in closeEditor()

In `closeEditor()` (line ~1055), BEFORE the existing collab disconnect logic, add:

```typescript
      // Cancel auto-save timers and remove editor:change listener
      if (this.autoSave) {
        this.autoSave.cancel();
        this.autoSave = null;
      }
      if (this._editorChangeHandler) {
        const editorContainer = document.getElementById("editor-container");
        if (editorContainer) {
          editorContainer.removeEventListener("editor:change", this._editorChangeHandler);
        }
        this._editorChangeHandler = null;
      }
```

#### 3e. Rewire Ctrl+S to use AutoSaveManager

Replace the Ctrl+S handler (the temporary one from Task 1) with:

```typescript
    if ((event.ctrlKey || event.metaKey) && event.key === "s") {
      event.preventDefault();
      if (app.collab?.editor && app.autoSave) {
        app.autoSave.saveNow();
      }
      return;
    }
```

#### 3f. Rewire save button onclick

The save button's `onclick` attribute in `templates.rs` currently calls `window.idApp?.saveFile?.()`. We need it to call `window.idApp?.autoSave?.saveNow()` instead. BUT — since the button may be rendered before AutoSaveManager is created, and the onclick is in the HTML, we should keep the onclick pointing at a stable app method. Add a `triggerSave()` wrapper method on the app object:

Add to `IdApp` interface:

```typescript
triggerSave: () => void;
```

Add to app object:

```typescript
    triggerSave(): void {
      if (this.autoSave) {
        this.autoSave.saveNow();
      } else if (this.collab?.editor) {
        // Fallback: direct save if AutoSaveManager not yet initialized
        this.saveFile().then((result) => {
          const btn = document.getElementById("save-btn") as HTMLButtonElement | null;
          if (!btn) return;
          if (result.ok) {
            btn.textContent = "saved \u2713";
            setTimeout(() => { btn.textContent = "save"; }, 2000);
          } else if (!result.retryAfterMs) {
            btn.textContent = "error!";
            setTimeout(() => { btn.textContent = "save"; }, 2000);
          }
        });
      }
    },
```

**Verify:** `just id check` from repo root
**Commit:** `feat(web): wire AutoSaveManager into editor lifecycle and keyboard shortcuts`

---

## Task 4: Update save button onclick in templates.rs

**File:** `pkgs/id/src/web/templates.rs`
**Test:** none (verified by Task 5 E2E tests)
**Depends:** Task 3

### What to change

In `pkgs/id/src/web/templates.rs` at line ~437, change the save button's `onclick` attribute:

```rust
// BEFORE:
html.push_str("            <button class=\"btn btn-ghost btn-xs font-mono\" id=\"save-btn\" title=\"Save (Ctrl+S)\" onclick=\"window.idApp?.saveFile?.()\" disabled>save</button>\n");

// AFTER:
html.push_str("            <button class=\"btn btn-ghost btn-xs font-mono\" id=\"save-btn\" title=\"Save (Ctrl+S)\" onclick=\"window.idApp?.triggerSave?.()\" disabled>save</button>\n");
```

**Verify:** `just id check` from repo root (this will run Rust clippy/fmt + web build + tests)
**Commit:** `feat(web): update save button to use triggerSave for autosave integration`

---

## Task 5: Add E2E tests for auto-save

**File:** `pkgs/id/e2e/tests/autosave.spec.ts` (NEW FILE)
**Test:** self (this IS the test)
**Depends:** Tasks 1-4

### What to create

Create a new Playwright test file. Follow the existing patterns from `editor-features.spec.ts` and `websocket.spec.ts`:

```typescript
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
```

**Verify:** Run `just id test-e2e-chromium` from repo root (or run just the new file: `npx playwright test autosave.spec.ts --project=chromium` from `pkgs/id/e2e/`)
**Commit:** `test(web): add E2E tests for auto-save on idle feature`

---

## Summary of All Changes

| Task | File | Action | Lines Changed (approx) |
|------|------|--------|----------------------|
| 1 | `pkgs/id/web/src/main.ts` | Refactor `saveFile()`, add `SaveResult` type | ~80 lines |
| 2 | `pkgs/id/web/src/main.ts` | Add `AutoSaveManager` class | ~160 lines |
| 3 | `pkgs/id/web/src/main.ts` | Wire into `openEditor`, `closeEditor`, Ctrl+S, add `triggerSave` | ~60 lines |
| 4 | `pkgs/id/src/web/templates.rs` | Change onclick from `saveFile` to `triggerSave` | 1 line |
| 5 | `pkgs/id/e2e/tests/autosave.spec.ts` | New E2E test file | ~200 lines |

**Total: ~500 lines across 2 files modified + 1 file created**

## Execution Order

```
Task 1 → commit → verify
  ↓
Task 2 → commit → verify
  ↓
Task 3 → commit → verify
  ↓
Task 4 → commit → verify
  ↓
Task 5 → commit → verify with E2E
```

All tasks are SEQUENTIAL because Tasks 1-4 modify the same file (`main.ts`) with each building on the previous.
