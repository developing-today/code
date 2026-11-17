// MOB-00 (#9467): the browser acceptance contract for the mobile work
// views, named for the slices whose acceptance it carries — MOB-03
// (responsive mobile queue, item detail, saved views, #9470) and
// MOB-04 (accessible compact responsive interaction system, #9471).
//
// The test runs against the aggregator's deterministic fixture mode
// only: every request that is not aimed at the local fixture server
// is aborted, and the queue item under test is read from the fixture
// server's own /api/queue.json, so the run is reproducible offline
// with no GitHub/Project/session credentials.
//
// Assertions (the MOB-00 acceptance contract):
//   1. Queue flow at 360px: the queue shell loads its fragment and
//      renders the fixture item's card; the page has no horizontal
//      overflow at 360px or at 768px.
//   2. Keyboard reachability: repeated Tab presses move focus onto
//      the queue card's title link (no mouse involved).
//   3. Detail flow: the same item's /item/<id> detail page renders
//      its title as the page heading at 360px, again with no
//      horizontal overflow.
//   4. Action + status announcement: on the needs-you work view, a
//      triage action submitted from the keyboard (focus + Enter) is
//      accepted by the actions API (201) and the app then presents
//      the result back in the page — the acted item joins the
//      "Dismissed / done by you" section, whose count increments.
//      (Comment posting is deliberately not used here: its author
//      authorization resolves through the host secret store, which
//      would make the harness non-deterministic across machines.)
import { expect, test, type Page } from "@playwright/test";

const ORIGIN = "http://127.0.0.1:4797";

test.beforeEach(async ({ page }) => {
  // Network isolation: local fixture server only.
  await page.route("**/*", (route) => {
    const url = route.request().url();
    if (url.startsWith(ORIGIN) || url.startsWith("data:") || url.startsWith("blob:")) {
      return route.continue();
    }
    return route.abort();
  });
});

async function expectNoHorizontalOverflow(page: Page) {
  const overflow = await page.evaluate(
    () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
  );
  expect(overflow).toBeLessThanOrEqual(1);
}

test("pmo_mob_03/04_mobile_queue_keyboard_and_viewport_contract", async ({ page, request }) => {
  // The deterministic fixture item: first in-progress row of the
  // queue's own API, rendered by the queue page below.
  const apiRes = await request.get(`${ORIGIN}/api/queue.json`);
  expect(apiRes.ok()).toBe(true);
  const queue = (await apiRes.json()) as {
    inProgress: { id: string; title: string }[];
  };
  const item = queue.inProgress[0];
  expect(item, "fixture queue must contain an in-progress item").toBeTruthy();

  // 1. Queue flow at 360px (fragment shell, as a phone browser gets it).
  await page.setViewportSize({ width: 360, height: 740 });
  await page.goto("/queue");
  const card = page.locator("#queue-main article.card", { hasText: item.title }).first();
  await expect(card).toBeVisible();
  await expectNoHorizontalOverflow(page);

  // The same queue page must also fit the 768px tablet width.
  await page.setViewportSize({ width: 768, height: 900 });
  await expectNoHorizontalOverflow(page);
  await page.setViewportSize({ width: 360, height: 740 });

  // 2. Keyboard reachability: Tab from the top of the page until the
  // card's title link holds focus.
  const titleLink = card.locator("a").first();
  await page.evaluate(() => window.scrollTo(0, 0));
  let focused = false;
  for (let i = 0; i < 80 && !focused; i += 1) {
    await page.keyboard.press("Tab");
    focused = await titleLink.evaluate((el) => document.activeElement === el);
  }
  expect(focused, "queue card title link must be reachable by Tab").toBe(true);

  // 3. Detail flow for the same queue item, at phone width.
  await page.goto(`/item/${encodeURIComponent(item.id)}`);
  await expect(page.locator("h1")).toContainText(item.title);
  await expectNoHorizontalOverflow(page);

  // 4. Action + status announcement, driven from the keyboard:
  // dismiss the first needs-you card and verify the result is
  // presented back in the page (the Dismissed / done count grows).
  await page.goto("/attention?full=1");
  const dismissedCount = async (): Promise<number> => {
    try {
      return await page.evaluate(() => {
        const m = document.body.innerText.match(/Dismissed \/ done by you \((\d+)\)/);
        return m ? Number(m[1]) : 0;
      });
    } catch {
      return -1; // navigation in flight; the poll below retries
    }
  };
  const before = await dismissedCount();
  expect(before).toBeGreaterThanOrEqual(0);
  const dismissButton = page.locator('form.aform[data-verb="dismiss"] button[type="submit"]').first();
  await expect(dismissButton).toBeVisible();
  const actionResponse = page.waitForResponse(
    (r) => r.url().includes("/api/actions") && r.request().method() === "POST",
  );
  await dismissButton.focus();
  await page.keyboard.press("Enter");
  expect((await actionResponse).status()).toBe(201);
  await expect.poll(dismissedCount).toBe(before + 1);
});
