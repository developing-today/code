import { describe, expect, test } from "bun:test";
import { fixtureSource } from "../src/sources/fixtures.ts";
import {
  FRAG_WATCHDOG_MS,
  FRAG_WATCHDOG_SCRIPT,
  renderAttentionShell,
  renderBoardShell,
  renderOverviewShell,
  renderQueueShell,
  renderTodayShell,
} from "../src/render.ts";

// Shell fragment watchdog (#1503): a stalled/failed fragment must surface,
// not spin — every shell page carries the watchdog script, which swaps a
// still-loading skeleton for a notice with a ?full=1 link after 10 s.
describe("shell fragment watchdog (#1503)", () => {
  test("watchdog bound is 10 seconds", () => {
    expect(FRAG_WATCHDOG_MS).toBe(10_000);
  });

  test("every shell page carries the watchdog and keeps its fragment load", async () => {
    const src = fixtureSource();
    const shells = [
      renderBoardShell(src.provenance()),
      renderAttentionShell(src.provenance()),
      renderTodayShell(src.provenance(), undefined, null, await src.quota()),
      renderQueueShell(src.provenance()),
      renderOverviewShell(src.provenance()),
    ];
    expect(shells).toHaveLength(5);
    for (const html of shells) {
      expect(html).toContain("frag-failed");
      expect(html).toContain("the live view did not arrive");
      expect(html).toContain("?full=1");
      // #10605: first paint is a Datastar v1 `data-init` (one-shot), not
      // the v0-era `data-on-load`, which the vendored bundle never fires.
      expect(html).toContain("data-init=\"@get('");
      expect(html).not.toContain("data-on-load");
      expect(html).toContain('class="skel"');
    }
  });

  test("attention shell watchdog covers all three fragment containers", () => {
    const src = fixtureSource();
    const html = renderAttentionShell(src.provenance());
    expect(html).toContain('id="attn-header"');
    expect(html).toContain('id="attn-queues"');
    expect(html).toContain('id="attn-sessions"');
    expect(html).toContain("frag-failed");
  });

  test("today shell keeps its health strip ahead of the skeleton", async () => {
    const src = fixtureSource();
    const html = renderTodayShell(src.provenance(), undefined, null, await src.quota());
    expect(html).toContain("frag-failed");
    expect(html.indexOf('class="today-health-strip"')).toBeLessThan(html.indexOf('class="skel"'));
  });

  // #11410: the shells only asserted on strings, so a quoting bug that
  // made the script unparseable shipped unnoticed. Parse and run it.
  test("watchdog script parses and swaps a stalled skeleton for the notice", () => {
    const body = FRAG_WATCHDOG_SCRIPT.replace(/^<script>/, "").replace(/<\/script>$/, "");
    let fired: (() => void) | undefined;
    let delay = 0;
    const created: Array<Record<string, unknown>> = [];
    let replacement: Record<string, unknown> | undefined;
    const skel = { replaceWith: (node: Record<string, unknown>) => (replacement = node) };
    const doc = {
      querySelectorAll: () => [{ querySelector: () => skel }, { querySelector: () => null }],
      createElement: (tag: string) => {
        const node: Record<string, unknown> = { tag, children: [] as unknown[] };
        node.append = (...parts: unknown[]) => (node.children as unknown[]).push(...parts);
        created.push(node);
        return node;
      },
    };
    const run = new Function("document", "location", "setTimeout", body);
    run(doc, { pathname: "/board" }, (fn: () => void, ms: number) => {
      fired = fn;
      delay = ms;
    });
    expect(delay).toBe(FRAG_WATCHDOG_MS);
    fired?.();
    expect(replacement?.className).toBe("frag-failed");
    const link = created.find((node) => node.tag === "a");
    expect(link?.href).toBe("/board?full=1");
    expect(link?.textContent).toBe("load the full page");
    expect(replacement?.children).toEqual(["the live view did not arrive \u2014 ", link]);
  });
});
