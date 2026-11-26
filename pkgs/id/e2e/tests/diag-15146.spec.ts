import { test, expect } from "@playwright/test";

// TEMPORARY diagnostic for #15146 — not part of the suite; deleted before PR.
// Reproduces "created file appears in file list" and dumps why the anchor
// is reported hidden, by throwing the dump as the test failure message.
test("diag: created file anchor visibility", async ({ page }) => {
  const fileName = `list-test-${Date.now()}.txt`;

  await page.goto("/files");
  await page.fill("#new-file-name", fileName);
  await page.click("#new-file-form button[type='submit']");
  await page.waitForURL(/\/(file|edit)\//);
  await page.goto("/files");

  // Give any post-load swaps a moment, mirroring the failing test's waits.
  await page.waitForTimeout(1500);

  const dump = await page.evaluate((name) => {
    const anchors = Array.from(document.querySelectorAll("a")).filter(
      (a) => a.textContent === name,
    );
    const textMatches: string[] = [];
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_ELEMENT);
    let node: Node | null;
    while ((node = walker.nextNode())) {
      const el = node as HTMLElement;
      if (el.children.length === 0 && el.textContent === name) {
        textMatches.push(el.tagName + "." + el.className);
      }
    }
    const describe = (el: HTMLElement | null): Record<string, unknown> | null => {
      if (!el) return null;
      const cs = getComputedStyle(el);
      const r = el.getBoundingClientRect();
      return {
        tag: el.tagName,
        cls: el.className,
        display: cs.display,
        visibility: cs.visibility,
        opacity: cs.opacity,
        width: r.width,
        height: r.height,
        x: r.x,
        y: r.y,
      };
    };
    const a = anchors[0] as HTMLElement | undefined;
    const chain: (Record<string, unknown> | null)[] = [];
    let cur: HTMLElement | null = a ?? null;
    while (cur && chain.length < 8) {
      chain.push(describe(cur));
      cur = cur.parentElement;
    }
    let atPoint: string | null = null;
    if (a) {
      const r = a.getBoundingClientRect();
      const el = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2);
      atPoint = el ? el.tagName + "." + el.className + " text=" + (el.textContent || "").slice(0, 40) : null;
    }
    return {
      anchorCount: anchors.length,
      textMatches,
      chain,
      atPoint,
      liHTML: a?.closest("li")?.outerHTML?.slice(0, 600) ?? null,
      viewport: { w: window.innerWidth, h: window.innerHeight },
      bodyChildren: Array.from(document.body.children).map((c) => c.tagName + "#" + c.id),
    };
  }, fileName);

  throw new Error("DIAG15146 " + JSON.stringify(dump));
  // eslint-disable-next-line no-unreachable
  expect(true).toBe(true);
});
