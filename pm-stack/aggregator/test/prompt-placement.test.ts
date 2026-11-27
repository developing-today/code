// #10032 S1 (design #9539 §2): prompt widgets placed on the pages.
import { describe, expect, test } from "bun:test";
import type { Provenance } from "../src/model.ts";
import { buildQueueView } from "../src/queue-view.ts";
import { renderBoardBody, renderQueue, renderQueueBody, renderRoadmap } from "../src/render.ts";
import { FIXTURE_BOARD, FIXTURE_ROADMAP } from "../src/sources/fixtures.ts";
import { PAGES } from "../src/pages.ts";
import { PRIMARY_PROMPT_PAGES, renderPromptDock, renderSectionPrompt } from "../src/prompt-widget.ts";

const NOW = new Date("2026-11-01T12:00:00Z");
const provenance = Object.fromEntries(
  [
    "board",
    "roadmap",
    "items",
    "comments",
    "quota",
    "usage",
    "attention",
    "orchestration",
    "dns",
    "registry",
    "aa",
  ].map((k) => [k, "fixtures"]),
) as unknown as Provenance;

describe("prompt dock", () => {
  test("every primary prompt page is a page in the manifest", () => {
    const ids = new Set(PAGES.map((p) => p.id));
    for (const id of PRIMARY_PROMPT_PAGES) expect(ids.has(id)).toBe(true);
    expect(PRIMARY_PROMPT_PAGES.length).toBe(7);
  });

  test("a primary page gets page, variant, stack and agent widgets", () => {
    const html = renderPromptDock({ page: "board", pageScope: true });
    expect(html).toContain('data-prompt-scope="page"');
    expect(html).toContain('data-scope="page" data-page="board"');
    expect(html).toContain('class="prompt-variants"');
    expect(html).toContain('data-scope="stack" data-page="board"');
    expect(html).toContain('data-scope="agent" data-page="board"');
  });

  test("a sub-page keeps only the stack and agent widgets", () => {
    const html = renderPromptDock({ page: "program", pageScope: false });
    expect(html).not.toContain('data-prompt-scope="page"');
    expect(html).not.toContain("prompt-variants");
    expect(html.match(/<form /g)?.length).toBe(2);
  });

  test("a section widget carries its ref and is collapsed", () => {
    const html = renderSectionPrompt("queue", "developing-today/code#42");
    expect(html.startsWith('<details class="prompt-section">')).toBe(true);
    expect(html).toContain('<input type="hidden" name="sectionRef" value="developing-today/code#42">');
    expect(html).toContain('data-scope="section" data-page="queue"');
  });
});

describe("pages carry the widgets", () => {
  test("the queue page has its page widget and a section widget per row", () => {
    const view = buildQueueView({ board: [], pool: [], items: new Map(), orchestration: null, now: NOW });
    view.nextUp = [
      {
        area: "PM stack",
        status: "to-do",
        title: "Queued work",
        ref: "developing-today/code#51",
        source: "project",
      } as (typeof view.nextUp)[number],
    ];
    const body = renderQueueBody(view);
    expect(body).toContain('value="developing-today/code#51"');
    const html = renderQueue(view, provenance);
    expect(html).toContain('data-scope="page" data-page="queue"');
    expect(html).toContain('data-scope="stack" data-page="queue"');
    expect(html).toContain('data-scope="agent" data-page="queue"');
  });

  test("board cards carry a board section widget keyed by the card id", () => {
    const item = FIXTURE_BOARD.find((b) => b.status !== "done") ?? FIXTURE_BOARD[0];
    const html = renderBoardBody([item], new Map());
    expect(html).toContain('data-scope="section" data-page="board"');
    expect(html).toContain(`name="sectionRef" value="${item.id}"`);
  });

  test("roadmap cards carry a roadmap section widget keyed by the row subject", () => {
    const html = renderRoadmap(FIXTURE_ROADMAP, new Map(), provenance, "quarter");
    expect(html).toContain('data-scope="section" data-page="roadmap"');
    expect(html).toContain('data-scope="page" data-page="roadmap"');
  });
});
