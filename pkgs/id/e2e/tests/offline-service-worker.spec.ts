import { spawn, type ChildProcess } from "node:child_process";
import { existsSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { test, expect, type Page } from "@playwright/test";

/**
 * Offline service worker E2E (#14774 slice 2, design #13918).
 *
 * Exercises the real worker (web/src/sw.ts, served at /sw.js) in a real
 * browser:
 * - a page opened while the server was reachable is served from the
 *   worker's page cache once the network to the server is gone;
 * - a page never opened shows the worker's 503 offline page;
 * - once the server is back, pages load from the network again;
 * - a newly installed worker deletes every id-shell-* / id-pages-* cache
 *   that is not the current build's (the activate step a new build runs).
 *
 * Why the spec stops a server instead of calling context.setOffline():
 * Playwright's offline emulation does not reach requests a service worker
 * makes itself (first remote runs on #14793: with the context offline the
 * worker still fetched /settings from the server and got a 200), so it
 * cannot show what the worker does when its fetch fails. A server of the
 * spec's own that it stops and restarts is a real network failure for
 * the worker in every browser.
 *
 * The worker registers only in a secure context (https or localhost), and
 * the spec starts its server from the local build, so it skips where
 * neither holds (the NixOS VM mode, which serves plain http from remote
 * hosts and has no local binary).
 *
 * Prerequisites:
 * - Web variant must be built first (`just build`)
 * - The shared server starts automatically via playwright.config.ts
 */

const SHELL_PREFIX = "id-shell-";
const PAGES_PREFIX = "id-pages-";
const VISITED = "/files";
const UNVISITED = "/settings";
// Cache names a previous build would have left behind. The hash parts
// cannot collide with a real build tag: real tags are 8 base36 / 8 hex
// characters taken from the emitted asset names.
const STALE_SHELL = `${SHELL_PREFIX}v1-zzzzzzzz-00000000`;
const STALE_PAGES = `${PAGES_PREFIX}v1-zzzzzzzz-00000000`;

// The spec's own server: the same binary the config's web servers run,
// on a port of its own (the config uses TEST_PORT and TEST_PORT + 1).
const ID_BIN = resolve(process.env.ID_E2E_BIN ?? "../target/debug/id");
const OWN_PORT = (Number(process.env.TEST_PORT) || 4173) + 7;
const OWN_ORIGIN = `http://localhost:${OWN_PORT}`;
const SANDBOX_FLAGS = process.env.NIX_BUILD_TOP ? ["--no-mdns", "--no-relay", "--no-gossip"] : [];

/** A disposable `id serve --web --ephemeral` the spec can stop and restart. */
class OwnServer {
  private child: ChildProcess | null = null;
  private readonly dir = mkdtempSync(join(tmpdir(), "id-e2e-offline-"));

  async start(): Promise<void> {
    // Its own working directory keeps its serve lock and keys apart from
    // the config's server, which runs in e2e/.
    const child = spawn(ID_BIN, ["serve", "--web", "--port", String(OWN_PORT), "--ephemeral", ...SANDBOX_FLAGS], {
      cwd: this.dir,
      stdio: ["ignore", "inherit", "inherit"],
    });
    this.child = child;
    const deadline = Date.now() + 60_000;
    while (Date.now() < deadline) {
      if (child.exitCode !== null) throw new Error(`id serve exited early with ${child.exitCode}`);
      if (await reachable(`${OWN_ORIGIN}${UNVISITED}`)) return;
      await new Promise((r) => setTimeout(r, 250));
    }
    throw new Error(`id serve did not answer on ${OWN_ORIGIN} within 60s`);
  }

  async stop(): Promise<void> {
    const child = this.child;
    this.child = null;
    if (!child || child.exitCode !== null) return;
    const exited = new Promise<void>((r) => child.once("exit", () => r()));
    child.kill("SIGTERM");
    const killer = setTimeout(() => child.kill("SIGKILL"), 10_000);
    await exited;
    clearTimeout(killer);
    // The port must refuse connections before the spec relies on it.
    await expect.poll(() => reachable(`${OWN_ORIGIN}/`), { timeout: 10_000 }).toBe(false);
  }

  async dispose(): Promise<void> {
    await this.stop();
    rmSync(this.dir, { recursive: true, force: true });
  }
}

async function reachable(url: string): Promise<boolean> {
  try {
    const response = await fetch(url, { signal: AbortSignal.timeout(2_000) });
    return response.ok;
  } catch {
    return false;
  }
}

async function cacheNames(page: Page): Promise<string[]> {
  return page.evaluate(() => caches.keys());
}

/** Open `url` and wait until an active id service worker controls the page. */
async function openControlled(page: Page, url: string): Promise<boolean> {
  await page.goto(url);
  const supported = await page.evaluate(() => window.isSecureContext && "serviceWorker" in navigator);
  if (!supported) return false;
  await page.evaluate(() => navigator.serviceWorker.ready.then(() => undefined));
  await page.waitForFunction(() => navigator.serviceWorker.controller !== null, undefined, { timeout: 15_000 });
  return true;
}

/** True when a pages cache holds `url`. */
async function pageIsCached(page: Page, url: string): Promise<boolean> {
  return page.evaluate(
    async ({ url, prefix }) => {
      for (const name of await caches.keys()) {
        if (!name.startsWith(prefix)) continue;
        const cache = await caches.open(name);
        if (await cache.match(url)) return true;
      }
      return false;
    },
    { url, prefix: PAGES_PREFIX },
  );
}

test.describe("Offline service worker", () => {
  test("serves a visited page offline and shows the offline page for an unvisited one", async ({ page }) => {
    test.skip(!existsSync(ID_BIN), `no local id binary at ${ID_BIN} (remote-server mode)`);
    const server = new OwnServer();
    try {
      await server.start();
      const visited = `${OWN_ORIGIN}${VISITED}`;
      const unvisited = `${OWN_ORIGIN}${UNVISITED}`;
      test.skip(!(await openControlled(page, visited)), "service workers need a secure context");

      // The first load ran before the worker controlled the page; this
      // navigation goes through it and keeps the page for offline reading.
      const online = await page.goto(visited);
      expect(online?.status()).toBe(200);
      await expect.poll(() => pageIsCached(page, visited), { timeout: 10_000 }).toBe(true);
      expect(await pageIsCached(page, unvisited)).toBe(false);

      // The network to the server is gone.
      await server.stop();

      const cached = await page.goto(visited);
      expect(cached?.status()).toBe(200);
      await expect(page.locator("#new-file-form")).toBeVisible();

      const missing = await page.goto(unvisited);
      expect(missing?.status()).toBe(503);
      await expect(page.getByRole("heading", { name: "You are offline" })).toBeVisible();
      await expect(page).toHaveTitle("Offline");

      // Back online: the unvisited page now comes from the network.
      await server.start();
      const back = await page.goto(unvisited);
      expect(back?.status()).toBe(200);
      await expect(page.locator("text=Node Identity")).toBeVisible({ timeout: 10_000 });
    } finally {
      await server.dispose();
    }
  });

  test("a newly installed worker replaces caches from other builds", async ({ page, baseURL }) => {
    test.skip(!(await openControlled(page, VISITED)), "service workers need a secure context");
    await page.goto(VISITED);
    const visited = new URL(VISITED, baseURL).toString();
    await expect.poll(() => pageIsCached(page, visited), { timeout: 10_000 }).toBe(true);

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
    await openControlled(page, VISITED);

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
