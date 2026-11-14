import { describe, expect, test } from "bun:test";
import { fixtureSource } from "../src/sources/fixtures.ts";
import {
  FRAG_WATCHDOG_MS,
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
      expect(html).toContain("data-on-load=\"@get('");
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
});
