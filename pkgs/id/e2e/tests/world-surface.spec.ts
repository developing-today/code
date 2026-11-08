import { expect, test, type Page } from "@playwright/test";

/**
 * World-surface E2E (#1322, Phase 9 slice of #1076).
 *
 * The world surface (presenter, input/viewport, chat-adjacent records
 * readback, reflow) had no browser-level coverage: pkgs/id/e2e/tests/ held
 * only editor-focused specs. These tests join the lobby world (hosted by
 * the test servers with the roc-screen fixture program, see
 * playwright.config.ts WORLD_FLAGS) and assert, in a real browser against
 * a real server:
 *
 * - the screen surface renders cell text (not guest HTML),
 * - a keypress and a paste each move the program state and the view updates,
 * - records readback pages with the after cursor and both pages render,
 * - a viewport resize re-renders a reflowed surface (cross-ref #811).
 *
 * The tests in this file share one ephemeral server per project (#229), so
 * the fixture counter accumulates across tests: assertions use before/after
 * deltas, never absolute counts.
 */

// Must match playwright.config.ts WORLD_FLAGS (and the NixOS VM wiring in
// nix/tests/playwright-e2e-test.nix). The VM clients do not forward this env
// var, so the default below is the contract.
const WORLD_ADMIN_TOKEN = process.env.ID_E2E_WORLD_ADMIN_TOKEN ?? "e2e-world-admin-token";

/** Join the lobby world as a fresh guest via the page's own invite + join UI. */
async function joinWorldAsGuest(page: Page, guestName: string): Promise<void> {
  await page.goto("/world");
  await expect(page).toHaveTitle("World - id");
  await expect(page.locator("[data-world-app]")).toBeVisible();
  // Before joining there is no surface, only the placeholder.
  await expect(page.locator("[data-world-view]")).toContainText("Waiting for a world view");

  await page.fill("[data-world-name]", guestName);
  await page.fill("[data-world-admin]", WORLD_ADMIN_TOKEN);
  await page.click("[data-world-invite]");
  // The invite round-trip fills the (persisted) capability input.
  await expect(page.locator("[data-world-capability]")).not.toHaveValue("", { timeout: 15_000 });

  await page.click("[data-world-join]");
  await expect(page.locator("[data-world-status]")).toHaveText("Connected", { timeout: 15_000 });
}

/** The current fixture counter as rendered in the screen surface. */
async function screenCount(page: Page): Promise<number> {
  const text = await page.locator("[data-world-view]").innerText();
  const match = /count=(\d+)/.exec(text);
  if (!match) throw new Error(`no count=N in world view: ${JSON.stringify(text)}`);
  return Number(match[1]);
}

test.describe("World surface", () => {
  test("join renders cell text on a Screen surface (not guest HTML)", async ({ page }) => {
    await joinWorldAsGuest(page, `surface-${Date.now()}`);

    const view = page.locator("[data-world-view]");
    // The presenter replaced the placeholder with a validated cell grid:
    // a role=img grid with a "columns by rows" label, not guest HTML text.
    const screen = view.locator(".id-world-screen");
    await expect(screen).toBeVisible({ timeout: 15_000 });
    await expect(view.locator('[role="img"]')).toHaveAttribute("aria-label", /World screen, \d+ columns by \d+ rows/);
    await expect(view).not.toContainText("Waiting for a world view");
    // The roc-screen fixture renders its counter through Screen cells.
    await expect(view).toContainText(/count=\d+/);
    // The join is journaled in the event log.
    await expect(page.locator("[data-world-log]")).toContainText(/Connected to lobby at event \d+/);
  });

  test("a keypress sends input and the view updates", async ({ page }) => {
    await joinWorldAsGuest(page, `key-${Date.now()}`);
    const surface = page.locator("[data-world-view] .id-world-screen");
    await expect(surface).toBeVisible({ timeout: 15_000 });

    const before = await screenCount(page);
    // The view is focusable (tabindex=0): a printable key becomes an input frame.
    await page.locator("[data-world-view]").click();
    await page.keyboard.press("+");
    // The roc-screen fixture counts "+" presses: the surface re-renders with +1.
    await expect.poll(() => screenCount(page), { timeout: 15_000 }).toBe(before + 1);
    // The input event frame arrived back over the live channel.
    await expect(page.locator("[data-world-log]")).toContainText(/input #\d+ from player/, {
      timeout: 15_000,
    });
  });

  test("a paste sends a paste frame and the view updates", async ({ page }) => {
    await joinWorldAsGuest(page, `paste-${Date.now()}`);
    const surface = page.locator("[data-world-view] .id-world-screen");
    await expect(surface).toBeVisible({ timeout: 15_000 });

    const before = await screenCount(page);
    // Load the OS clipboard the way a user does: type into the game input,
    // select all, copy. No clipboard permission APIs involved.
    const gameInput = page.locator("[data-world-input]");
    await gameInput.fill("++");
    await gameInput.press("ControlOrMeta+a");
    await gameInput.press("ControlOrMeta+c");

    await page.locator("[data-world-view]").click();
    await page.locator("[data-world-view]").press("ControlOrMeta+v");
    // The pasted "++" is two "+" presses for the fixture: +2.
    await expect.poll(() => screenCount(page), { timeout: 15_000 }).toBe(before + 2);
    await expect(page.locator("[data-world-log]")).toContainText(/input #\d+ from player/, {
      timeout: 15_000,
    });
  });

  test("records readback pages with the after cursor, both pages render", async ({ page }) => {
    await joinWorldAsGuest(page, `records-${Date.now()}`);
    const recordsView = page.locator("[data-world-records]");
    // The snapshot triggers the first-page records fetch, rendered as JSON.
    await expect(recordsView).toContainText(/"count": \d+/, { timeout: 15_000 });
    const rendered = JSON.parse((await recordsView.innerText()) || "{}") as Record<string, unknown>;

    // Page the guest records channel with the after cursor over a second,
    // in-page WebSocket: a real browser socket against the real server, so
    // the guest-channel pagination path is covered beyond the client's
    // automatic first-page fetch above.
    const capability = await page.locator("[data-world-capability]").inputValue();
    const socketUrl = new URL("/ws/world", page.url()).toString().replace(/^http/, "ws");
    const pages = await page.evaluate(
      async ({ socketUrl, capability }: { socketUrl: string; capability: string }) => {
        type RecordsFrame = {
          type?: string;
          records?: Record<string, unknown>;
          next_after?: string | null;
          message?: string;
        };
        const nextFrame = (socket: WebSocket): Promise<string> =>
          new Promise((resolve, reject) => {
            const timer = window.setTimeout(() => reject(new Error("aux world frame timeout")), 10_000);
            socket.addEventListener(
              "message",
              (event) => {
                window.clearTimeout(timer);
                resolve(String(event.data));
              },
              { once: true },
            );
          });
        const socket = new WebSocket(socketUrl);
        await new Promise<void>((resolve, reject) => {
          const timer = window.setTimeout(() => reject(new Error("aux world open timeout")), 10_000);
          socket.addEventListener(
            "open",
            () => {
              window.clearTimeout(timer);
              resolve();
            },
            { once: true },
          );
          socket.addEventListener(
            "error",
            () => {
              window.clearTimeout(timer);
              reject(new Error("aux world socket error"));
            },
            { once: true },
          );
        });
        const send = (frame: Record<string, unknown>): void => {
          socket.send(JSON.stringify(frame));
        };
        const nextRecords = async (): Promise<RecordsFrame> => {
          for (;;) {
            const frame = JSON.parse(await nextFrame(socket)) as RecordsFrame;
            if (frame.type === "records") return frame;
            if (frame.type === "error") throw new Error(`aux records failed: ${frame.message ?? ""}`);
          }
        };
        try {
          send({ type: "join", capability, after: null, screen_patches: true });
          for (;;) {
            const frame = JSON.parse(await nextFrame(socket)) as { type?: string; message?: string };
            if (frame.type === "snapshot") break;
            if (frame.type === "error") throw new Error(`aux join failed: ${frame.message ?? ""}`);
          }
          send({ type: "records", capability });
          const first = await nextRecords();
          const keys = Object.keys(first.records ?? {}).sort();
          // The fixture keeps a single record, so the first page is also the
          // last: page explicitly after its final key to cover the cursor.
          send({ type: "records", capability, after: keys.length > 0 ? keys[keys.length - 1] : "" });
          const second = await nextRecords();
          return { first, second };
        } finally {
          socket.close();
        }
      },
      { socketUrl, capability },
    );

    // The fixture's single record arrives alone: first page holds the count
    // and reports no further page, the explicit after-cursor fetch returns
    // the empty remainder and terminates the chain.
    expect(pages.first.next_after).toBeNull();
    expect(pages.second.records).toEqual({});
    expect(pages.second.next_after).toBeNull();
    // Both pages union to exactly what the UI rendered.
    expect({ ...(pages.first.records ?? {}), ...(pages.second.records ?? {}) }).toEqual(rendered);
  });

  test("viewport resize re-renders a reflowed surface", async ({ page }) => {
    await joinWorldAsGuest(page, `resize-${Date.now()}`);
    const view = page.locator("[data-world-view]");
    const screen = view.locator('[role="img"]');
    await expect(view.locator(".id-world-screen")).toBeVisible({ timeout: 15_000 });
    // Full-width desktop viewport: the fixture lays out at 30 columns.
    await expect(screen).toHaveAttribute("aria-label", "World screen, 30 columns by 3 rows", {
      timeout: 15_000,
    });
    await expect(view).toContainText(/count=\d+/);

    // Narrow the browser window: the client reports a smaller viewport over
    // the live channel (#811: resize stays session-local) and the fixture
    // reflows to its 20-column layout.
    await page.setViewportSize({ width: 320, height: 800 });
    await expect(screen).toHaveAttribute("aria-label", "World screen, 20 columns by 3 rows", {
      timeout: 15_000,
    });
    // The reflowed surface still renders the program state, not an error.
    await expect(view).toContainText(/count=\d+/);

    // Widen again: the surface re-renders back at 30 columns.
    await page.setViewportSize({ width: 1280, height: 720 });
    await expect(screen).toHaveAttribute("aria-label", "World screen, 30 columns by 3 rows", {
      timeout: 15_000,
    });
  });
});
