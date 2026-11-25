import { test as base, expect, type Page } from "@playwright/test";

/**
 * Offline service worker E2E (#14774 slice 2, design #13918).
 *
 * Exercises the real worker (web/src/sw.ts, served at /sw.js) in a real
 * browser:
 * - a page opened while online is served from the worker's page cache
 *   once the browser context goes offline;
 * - a page never opened shows the worker's 503 offline page;
 * - after reconnecting, pages load from the network again;
 * - a newly installed worker deletes every id-shell-* / id-pages-* cache
 *   that is not the current build's (the activate step a new build runs).
 *
 * The worker registers only in a secure context (https or localhost), so
 * the spec skips where the base URL is a plain-http remote host (the
 * NixOS VM mode, where servers run on other machines).
 *
 * Prerequisites:
 * - Web variant must be built first (`just build`)
 * - Server starts automatically via playwright.config.ts webServer
 */

// Firefox keeps localhost reachable in offline mode unless told not to
// (network.disable-localhost-when-offline defaults to false), and every
// runner serves the app from localhost. Without this pref "offline" would
// still reach the server and the spec would prove nothing. Chromium
// ignores firefoxUserPrefs.
const test = base.extend({
  launchOptions: [
    async ({ launchOptions }, use) => {
      await use({
        ...launchOptions,
        firefoxUserPrefs: {
          ...launchOptions.firefoxUserPrefs,
          "network.disable-localhost-when-offline": true,
        },
      });
    },
    { scope: "worker" },
  ],
});

const SHELL_PREFIX = "id-shell-";
const PAGES_PREFIX = "id-pages-";
const VISITED = "/files";
const UNVISITED = "/settings";
// Cache names a previous build would have left behind. The hash parts
// cannot collide with a real build tag: real tags are 8 base36 / 8 hex
// characters taken from the emitted asset names.
const STALE_SHELL = `${SHELL_PREFIX}v1-zzzzzzzz-00000000`;
const STALE_PAGES = `${PAGES_PREFIX}v1-zzzzzzzz-00000000`;

async function cacheNames(page: Page): Promise<string[]> {
  return page.evaluate(() => caches.keys());
}

/** Wait until the page is controlled by an active id service worker. */
async function waitForController(page: Page): Promise<void> {
  await page.evaluate(() => navigator.serviceWorker.ready.then(() => undefined));
  await page.waitForFunction(() => navigator.serviceWorker.controller !== null, undefined, { timeout: 15_000 });
}

/** True when the pages cache of the current build holds `path`. */
async function pageIsCached(page: Page, path: string): Promise<boolean> {
  return page.evaluate(
    async ({ path, prefix }) => {
      const url = new URL(path, location.origin).toString();
      for (const name of await caches.keys()) {
        if (!name.startsWith(prefix)) continue;
        const cache = await caches.open(name);
        if (await cache.match(url)) return true;
      }
      return false;
    },
    { path, prefix: PAGES_PREFIX },
  );
}

test.describe("Offline service worker", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto(VISITED);
    const supported = await page.evaluate(() => window.isSecureContext && "serviceWorker" in navigator);
    test.skip(!supported, "service workers need a secure context (https or localhost)");
    await waitForController(page);
  });

  test("serves a visited page offline and shows the offline page for an unvisited one", async ({ page, context }) => {
    // The first load ran before the worker controlled the page; this
    // navigation goes through it and keeps the page for offline reading.
    const online = await page.goto(VISITED);
    expect(online?.status()).toBe(200);
    await expect.poll(() => pageIsCached(page, VISITED), { timeout: 10_000 }).toBe(true);
    expect(await pageIsCached(page, UNVISITED)).toBe(false);

    await context.setOffline(true);
    try {
      const cached = await page.goto(VISITED);
      expect(cached?.status()).toBe(200);
      await expect(page.locator("#new-file-form")).toBeVisible();

      const missing = await page.goto(UNVISITED);
      expect(missing?.status()).toBe(503);
      await expect(page.getByRole("heading", { name: "You are offline" })).toBeVisible();
      await expect(page).toHaveTitle("Offline");
    } finally {
      await context.setOffline(false);
    }

    // Reconnected: the unvisited page now comes from the network.
    const back = await page.goto(UNVISITED);
    expect(back?.status()).toBe(200);
    await expect(page.locator("text=Node Identity")).toBeVisible({ timeout: 10_000 });
  });

  test("a newly installed worker replaces caches from other builds", async ({ page }) => {
    await page.goto(VISITED);
    await expect.poll(() => pageIsCached(page, VISITED), { timeout: 10_000 }).toBe(true);

    const current = (await cacheNames(page)).filter((n) => n.startsWith(SHELL_PREFIX) || n.startsWith(PAGES_PREFIX));
    expect(current.some((n) => n.startsWith(SHELL_PREFIX))).toBe(true);

    // Leave the caches an older build would have left, then install the
    // worker afresh: its activate step must delete them and keep this
    // build's caches.
    await page.evaluate(
      async ({ shell, pages }) => {
        await (await caches.open(shell)).put("/stale-shell", new Response("old shell"));
        await (await caches.open(pages)).put("/stale-page", new Response("old page"));
      },
      { shell: STALE_SHELL, pages: STALE_PAGES },
    );
    expect(await cacheNames(page)).toEqual(expect.arrayContaining([STALE_SHELL, STALE_PAGES]));

    await page.evaluate(async () => {
      for (const registration of await navigator.serviceWorker.getRegistrations()) {
        await registration.unregister();
      }
    });
    await page.goto(VISITED);
    await waitForController(page);

    await expect
      .poll(async () => (await cacheNames(page)).filter((n) => n === STALE_SHELL || n === STALE_PAGES), {
        timeout: 15_000,
      })
      .toEqual([]);
    const after = await cacheNames(page);
    expect(after.some((n) => n.startsWith(SHELL_PREFIX))).toBe(true);
    for (const name of current.filter((n) => n.startsWith(SHELL_PREFIX))) {
      expect(after).toContain(name);
    }
  });
});
