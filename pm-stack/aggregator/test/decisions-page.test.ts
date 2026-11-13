import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { DecisionSet } from "../src/decisions.ts";

// Decisions page rendering (Drew, 2026-11-13): per-item submit must be
// reachable on multi-question items (v2 category sets carry 7–17
// questions in ONE item — the lone button sat after the last question,
// ~7 screens down, and read as "gone"), and submitted items hide by
// default behind a "Show submitted items" toggle instead of vanishing
// with no way back. Boots the real server against a temp store.

const AGG_DIR = join(import.meta.dir, "..");

function freePort(): number {
  const s = Bun.serve({ port: 0, fetch: () => new Response("x") });
  const p = s.port;
  s.stop(true);
  return p;
}

const NOW = "2026-11-13T08:00:00.000Z";
function q(setId: string, n: number, extra: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    qid: `${setId}-item#${n}`,
    question: `Question ${n}?`,
    options: [
      { id: "opt-1", label: "Yes" },
      { id: "opt-2", label: "No" },
    ],
    ...extra,
  };
}
function item(ledgerId: string, questions: unknown[]): Record<string, unknown> {
  return { ledgerId, title: `Item ${ledgerId}`, group: "G", links: [], questions };
}
function makeSet(setId: string, items: unknown[]): DecisionSet {
  return {
    setId,
    title: `Set ${setId}`,
    token: `tok-${setId}`,
    createdAt: NOW,
    source: "test",
    items,
  } as unknown as DecisionSet;
}

const multiSet = makeSet("page-multi", [
  item("multi-item", [q("page-multi", 1, { recommendedOptionIds: ["opt-1"] }), q("page-multi", 2), q("page-multi", 3)]),
]);
const singleSet = makeSet("page-single", [item("single-a", [q("page-single", 1)]), item("single-b", [q("page-single", 1)] )]);
const doneSet = makeSet("page-done", [
  item("done-item", [
    q("page-done", 1, { resolution: { kind: "answered", optionLabel: "Yes", at: NOW, via: "test" } }),
    q("page-done", 2, { resolution: { kind: "deferred", lean: "none", at: NOW, via: "test" } }),
  ]),
]);

let dir: string;
let proc: ReturnType<typeof Bun.spawn> | null = null;
let base = "";

async function getPage(setId: string): Promise<string> {
  const res = await fetch(`${base}/decisions/${setId}?t=tok-${setId}`);
  expect(res.status).toBe(200);
  return res.text();
}
const buttonCount = (html: string, ledger: string): number => html.split(`data-submit-item="${ledger}"`).length - 1;

beforeAll(async () => {
  dir = await mkdtemp(join(tmpdir(), "decisions-page-test-"));
  const file = join(dir, "decisions.json");
  await writeFile(file, JSON.stringify({ version: 1, sets: [multiSet, singleSet, doneSet] }, null, 2));
  const port = freePort();
  base = `http://127.0.0.1:${port}`;
  proc = Bun.spawn(["bun", "run", "src/server.ts"], {
    cwd: AGG_DIR,
    env: {
      ...process.env,
      PM_AGG_SOURCE: "live",
      PM_AGG_PORT: String(port),
      PM_AGG_FRESHNESS_PORT: String(freePort()),
      PM_AGG_DECISIONS_FILE: file,
      PM_AGG_DECISION_EVENTS_FILE: join(dir, "events.jsonl"),
      PM_AGG_DECISION_OUTBOX_FILE: join(dir, "outbox.jsonl"),
    },
    stdout: "ignore",
    stderr: "ignore",
  });
  const deadline = Date.now() + 30_000;
  for (;;) {
    try {
      const res = await fetch(`${base}/decisions/page-multi?t=tok-page-multi`);
      if (res.status === 200) break;
    } catch { /* not up yet */ }
    if (Date.now() > deadline) throw new Error("aggregator did not come up");
    await Bun.sleep(250);
  }
}, 60_000);

afterAll(async () => {
  proc?.kill();
  if (dir) await rm(dir, { recursive: true, force: true });
});

describe("decisions page — per-item submit placement", () => {
  test("a multi-question item gets a submit button at the TOP of its card as well as the bottom", async () => {
    const html = await getPage("page-multi");
    expect(buttonCount(html, "multi-item")).toBe(2);
    expect(html).toContain('class="submit top"');
    // The top button lands before the first question block.
    expect(html.indexOf('class="submit top"')).toBeLessThan(html.indexOf('<div class="q"'));
  });

  test("single-question (v1-shaped) items keep exactly one submit button each, unchanged", async () => {
    const html = await getPage("page-single");
    expect(buttonCount(html, "single-a")).toBe(1);
    expect(buttonCount(html, "single-b")).toBe(1);
    expect(html).not.toContain('class="submit top"');
  });
});

describe("decisions page — submitted items hide by default", () => {
  test("a fully-resolved item renders as a submitted card behind the toggle, with its recorded lines", async () => {
    const html = await getPage("page-done");
    expect(html).toContain('class="item submitted"');
    expect(html).toContain("data-submitted=\"1\"");
    expect(html).toContain('id="submittedCount">1<');
    expect(html).toContain("submitted-group");
    expect(html).toContain("Show submitted items");
    // Nothing left to submit on a submitted card.
    expect(buttonCount(html, "done-item")).toBe(0);
    // The recorded resolutions stay reviewable in the card.
    expect(html).toContain("answered");
    expect(html).toContain("deferred to the orchestrator");
  });

  test("submitting an item's last open questions turns its card into a submitted one on re-render", async () => {
    const submit = async (n: number) => {
      const res = await fetch(`${base}/api/decisions/page-multi/submit`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          token: "tok-page-multi",
          submissions: [{ ledgerId: "multi-item", qid: `page-multi-item#${n}`, kind: "answered", optionId: "opt-1" }],
        }),
      });
      expect(res.status).toBe(200);
    };
    await submit(1);
    let html = await getPage("page-multi");
    // Partially submitted: the item is still an open card with both buttons.
    expect(html).not.toContain('class="item submitted"');
    expect(buttonCount(html, "multi-item")).toBe(2);
    await submit(2);
    await submit(3);
    html = await getPage("page-multi");
    expect(html).toContain('class="item submitted"');
    expect(html).toContain('id="submittedCount">1<');
    expect(buttonCount(html, "multi-item")).toBe(0);
  }, 30_000);
});
