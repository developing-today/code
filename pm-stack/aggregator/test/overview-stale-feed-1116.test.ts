import { describe, expect, test } from "bun:test";
import { FEED_STALE_MS } from "../src/feed.ts";
import { renderOverviewBody, type OverviewSummary } from "../src/render.ts";

// #1116 item (5): killing the collector must make the dashboard visibly
// say so. The drill verifies feed age in /api/health; these tests pin
// the rendered half — the overview carries an actionable stale-feed
// warning (with a data-signal hook the drill greps for) exactly when the
// feed has crossed the configured threshold, and never otherwise.

function summaryWithFeedAge(feedAgeMs: number | null, degraded: boolean): OverviewSummary {
  return {
    needsYou: { count: 0, tierState: "live", top: [] },
    projects: [],
    quota: [],
    health: { degraded, up: 3, down: 0, total: 3, feedAgeMs },
    recentDone: [],
    inProgress: [],
  };
}

describe("overview stale-feed warning (#1116 item 5)", () => {
  test("feed past FEED_STALE_MS renders an actionable warning with the drill hook", () => {
    const html = renderOverviewBody(summaryWithFeedAge(FEED_STALE_MS + 60_000, true));
    expect(html).toContain('data-signal="feed-stale"');
    expect(html).toContain("Feed stale");
    expect(html).toContain('href="/health"');
  });

  test("feed exactly at the threshold counts as stale (matches the health signal)", () => {
    const html = renderOverviewBody(summaryWithFeedAge(FEED_STALE_MS, true));
    expect(html).toContain('data-signal="feed-stale"');
  });

  test("fresh feed renders no stale banner", () => {
    const html = renderOverviewBody(summaryWithFeedAge(60_000, false));
    expect(html).not.toContain('data-signal="feed-stale"');
    expect(html).not.toContain("Feed stale");
    expect(html).toContain("feed ");
  });

  test("unknown feed age renders no stale banner (never invent a warning)", () => {
    const html = renderOverviewBody(summaryWithFeedAge(null, false));
    expect(html).not.toContain('data-signal="feed-stale"');
  });
});
