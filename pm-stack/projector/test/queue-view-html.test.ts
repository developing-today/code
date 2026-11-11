// Integrated queue/PM view — human face (#7078, #2072 slice 3 second
// piece): the rendering of the SAME read model the machine face emits.
// Oracles: the QueueView fixture itself — the page must show the
// view's counts verbatim, list queued entries in the view's order
// (the broker's claim order, computed by buildQueueView), attribute
// claimed entries to the claiming system/lane on the record, and its
// trend panels must link exactly the refs the machine face emitted
// (defaultTrendLinks) — links, never copies.
import { describe, test, expect } from "bun:test";
import { buildQueueView, defaultTrendLinks, type QueueTaskRecord } from "../src/queue-view";
import { escapeHtml, renderQueueViewHtml } from "../src/queue-view-html";
import type { HotEventRow } from "../src/store";

const SINCE = "2026-11-10T00:00:00Z";
const UNTIL = "2026-11-11T00:00:00Z";

const task = (over: Partial<QueueTaskRecord>): QueueTaskRecord => ({
  id: 1,
  queue: "repo-health-code-pilot",
  title: "task",
  status: "queued",
  priority: 30,
  created_at: "2026-11-09T00:00:00Z",
  claimed_at: null,
  worker_id: null,
  lane_id: null,
  platform: null,
  dedupe_key: null,
  payload: { repo: "developing-today/code", kind: "small_fix", issue: { number: 100 } },
  ...over,
});

const hotRow = (over: Partial<HotEventRow>): HotEventRow => ({
  seq: 1,
  event_id: "d1:issue:100",
  delivery_id: "d1",
  repo: "developing-today/code",
  event: "issues",
  action: "opened",
  ref: "developing-today/code#100",
  actor: "dezren39",
  source_url: "https://github.com/developing-today/code/issues/100",
  source_timestamp: "2026-11-10T01:00:00Z",
  received_at: "2026-11-10T01:00:00Z",
  ...over,
});

const tasks = [
  task({ id: 11, title: "later in claim order", priority: 20, created_at: "2026-11-09T02:00:00Z" }),
  task({
    id: 12,
    title: "first in claim order",
    priority: 10,
    created_at: "2026-11-09T01:00:00Z",
    payload: { repo: "developing-today/code", kind: "triage", issue: { number: 200 } },
  }),
  task({
    id: 13,
    title: "claimed work",
    status: "claimed",
    claimed_at: "2026-11-10T05:00:00Z",
    worker_id: "repo-worker-code",
    lane_id: "code-d",
    platform: "muse",
    payload: { repo: "developing-today/code", kind: "small_fix", issue: { number: 300 } },
  }),
  task({ id: 15, title: "finished work", status: "done", payload: { repo: "developing-today/code", kind: "small_fix", issue: { number: 500 } } }),
];
const events = [hotRow({ seq: 7, ref: "developing-today/code#100" })];

const view = buildQueueView(tasks, events, {
  since: SINCE,
  until: UNTIL,
  now: UNTIL,
  consumer: "queue-view",
  fromSeq: 0,
  highWater: 7,
  links: defaultTrendLinks({}),
});
const html = renderQueueViewHtml(view);

describe("queue-view human face: sections and counts", () => {
  test("page carries every section of the read model", () => {
    for (const marker of [
      "Status counts",
      "Queued — in claim order",
      "Picked up in window",
      "Trend panels",
      "Hot list",
    ]) {
      expect(html).toContain(marker);
    }
    expect(html).toContain(SINCE);
    expect(html).toContain(UNTIL);
  });

  test("counts shown are the view's counts, verbatim", () => {
    expect(view.counts).toEqual({ queued: 2, claimed: 1, done: 1 });
    for (const [status, n] of Object.entries(view.counts)) {
      expect(html).toContain(`<td>${status}</td><td>${n}</td>`);
    }
  });

  test("hot-list summary names the consumer and consumption range", () => {
    expect(html).toContain("queue-view");
    expect(html).toContain("consumed 1 event(s)");
    expect(html).toContain("high-water 7");
  });
});

describe("queue-view human face: order and attribution", () => {
  test("queued entries render in the view's claim order, never re-sorted", () => {
    expect(view.queued.map((q) => q.task_id)).toEqual([12, 11]);
    const first = html.indexOf("first in claim order");
    const later = html.indexOf("later in claim order");
    expect(first).toBeGreaterThan(0);
    expect(later).toBeGreaterThan(first);
  });

  test("claimed entries name the claiming system/lane/platform from the record", () => {
    expect(html).toContain("claimed work");
    expect(html).toContain("repo-worker-code");
    expect(html).toContain("code-d");
    expect(html).toContain("muse");
    expect(html).toContain("2026-11-10T05:00:00Z");
  });

  test("joined hot events render under their entry with a source link", () => {
    expect(html).toContain("issues/opened");
    expect(html).toContain('href="https://github.com/developing-today/code/issues/100"');
  });
});

describe("queue-view human face: trend panels link the machine face's stores", () => {
  test("every trend link the machine face emits appears as a panel href, verbatim", () => {
    expect(view.links.length).toBeGreaterThanOrEqual(4);
    for (const link of view.links) {
      expect(html).toContain(`href="${link.ref}"`);
      expect(html).toContain(escapeHtml(link.label));
      expect(html).toContain(`data-store="${link.store}"`);
    }
  });

  test("panels cover usage, quotas, and scheduled jobs (the spec's trend set)", () => {
    const labels = view.links.map((l) => l.label).join(" ");
    expect(labels).toContain("Usage & billing trends");
    expect(labels).toContain("Plan quota snapshots");
    expect(labels).toContain("Scheduled-job activity stream");
  });
});

describe("queue-view human face: escaping", () => {
  test("escapeHtml escapes markup-significant characters", () => {
    expect(escapeHtml(`<a href="x">&'</a>`)).toBe(
      "&lt;a href=&quot;x&quot;&gt;&amp;&#39;&lt;/a&gt;",
    );
  });

  test("task titles are data, not markup: a script-shaped title renders escaped", () => {
    const evil = buildQueueView(
      [task({ id: 99, title: "<script>alert(1)</script>" })],
      [],
      { since: SINCE, until: UNTIL, now: UNTIL },
    );
    const page = renderQueueViewHtml(evil);
    expect(page).not.toContain("<script>alert(1)</script>");
    expect(page).toContain("&lt;script&gt;alert(1)&lt;/script&gt;");
  });
});
