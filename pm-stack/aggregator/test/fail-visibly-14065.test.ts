// #14065 slice 2: in live mode, quota and usage never fall back to
// fixture numbers. With no fleet feed and no quota view, quota serves
// no pools and usage serves empty sections, each naming the outage in
// provenance (facet state "unavailable"); the null-returning feed
// sections name the missing feed instead of claiming fixtures.
import { describe, expect, test } from "bun:test";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadConfig } from "../src/config.ts";
import type { Provenance } from "../src/model.ts";
import { freshnessChips, renderItem, unavailableSinceText } from "../src/render.ts";
import { emptyQuotaView, isUnavailableProvenance, liveSource, trackUnavailable } from "../src/sources/live.ts";

async function withNoSources<T>(run: (src: ReturnType<typeof liveSource>) => Promise<T>): Promise<T> {
  const dir = await mkdtemp(join(tmpdir(), "pm-fail-visibly-14065-"));
  try {
    const cfg = loadConfig({
      PM_AGG_SOURCE: "live",
      PM_AGG_FEED_FILE: join(dir, "no-feed.json"),
      PM_AGG_QUOTA_VIEW: join(dir, "no-quota-view.md"),
      PM_AGG_QUOTA_REGISTRY: join(dir, "no-quota-registry.md"),
      PM_AGG_QUOTA_CURRENT_FILE: join(dir, "no-quota-current.json"),
      PM_AGG_SNAPSHOTS_DIR: join(dir, "snapshots"),
      PM_PROJECTOR_URL: "off",
    });
    return await run(liveSource(cfg));
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
}

describe("live mode fails visibly instead of serving fixtures (#14065 slice 2)", () => {
  test("quota: no feed serves no pools and an unavailable facet", async () => {
    await withNoSources(async (src) => {
      const view = await src.quota();
      expect(view.pools).toEqual([]);
      expect(view.burn).toEqual([]);
      expect(view.choices).toEqual(emptyQuotaView().choices);
      expect(src.provenance().quota).toStartWith("unavailable (no fleet feed file yet");
      expect(src.provenanceDetail!().quota.state).toBe("unavailable");
    });
  });

  test("usage: no feed and no quota view serve empty sections, never fixture services", async () => {
    await withNoSources(async (src) => {
      const usage = await src.usage();
      expect(usage.services).toEqual([]);
      expect(usage.payg).toEqual([]);
      expect(usage.subscriptions).toEqual([]);
      expect(usage.charges).toEqual([]);
      expect(usage.attention).toEqual([]);
      expect(usage.chargesNote).toContain("Usage sources are unreadable");
      expect(src.provenance().usage).toStartWith("unavailable (no fleet feed hub sections and quota view unreadable");
      expect(src.provenanceDetail!().usage.state).toBe("unavailable");
    });
  });

  test("feed sections with no feed name the missing feed, not fixtures", async () => {
    await withNoSources(async (src) => {
      expect(await src.dns()).toBeNull();
      expect(src.provenance().dns).toBe("unavailable (no fleet feed file yet)");
      expect(await src.aa()).toBeNull();
      expect(src.provenance().aa).toBe("unavailable (no fleet feed file yet)");
    });
  });
});

// #14065 slice 3: items and comments. With no project pool and no
// projector, live mode serves no fixture items and no fixture
// comments; the facets say "unavailable" and the item page names why
// its linked refs and comment stream are empty.
describe("live mode fails visibly for items and comments (#14065 slice 3)", () => {
  test("items: refs no live source resolves are left out, never fixture items", async () => {
    await withNoSources(async (src) => {
      // developing-today/code#752 exists in the fixture data.
      const items = await src.items(["developing-today/code#752"]);
      expect(items.size).toBe(0);
      expect(src.provenance().items).toBe(
        "unavailable (no project pool in the fleet feed and no projector configured; fixtures withheld in live mode)",
      );
      expect(src.provenanceDetail!().items.state).toBe("unavailable");
    });
  });

  test("comments and latest comments: no projector serves none, never fixture comments", async () => {
    await withNoSources(async (src) => {
      expect(await src.comments(["developing-today/code#752"])).toEqual([]);
      expect(await src.latestComments!(["developing-today/code#752"])).toEqual([]);
      expect(src.provenance().comments).toBe(
        "unavailable (no projector configured; fixtures withheld in live mode)",
      );
      expect(src.provenanceDetail!().comments.state).toBe("unavailable");
    });
  });

  test("comments: an unreachable projector serves none and says so", async () => {
    const dir = await mkdtemp(join(tmpdir(), "pm-fail-visibly-14065-"));
    try {
      const cfg = loadConfig({
        PM_AGG_SOURCE: "live",
        PM_AGG_FEED_FILE: join(dir, "no-feed.json"),
        // Port 9 (discard) on loopback: refused at once, no wait.
        PM_PROJECTOR_URL: "http://127.0.0.1:9",
      });
      const src = liveSource(cfg);
      expect(await src.comments(["developing-today/code#752"])).toEqual([]);
      expect(src.provenance().comments).toBe("unavailable (projector unreachable; fixtures withheld in live mode)");
    } finally {
      await rm(dir, { recursive: true, force: true });
    }
  });

  test("the item page names the outage for unresolved refs and the empty comment stream", async () => {
    await withNoSources(async (src) => {
      const links = [{ ref: "developing-today/code#752", relation: "tracks" as const }];
      const items = await src.items(links.map((l) => l.ref));
      const comments = await src.comments(links.map((l) => l.ref));
      const html = renderItem("Subject", "subj", links, items, comments, src.provenance() as Provenance);
      expect(html).toContain(
        "1 linked · 0 resolved — the projector is off or unreachable, so refs outside the project pool are unresolved (#14065)",
      );
      expect(html).toContain("The comment stream is unavailable while the projector is off or unreachable (#14065)");
      expect(html).toContain(
        "no comments loaded — the comment stream is unavailable while the projector is off or unreachable (#14065)",
      );
      expect(html).not.toContain("fixture-sourced");
    });
  });
});

// #14065 slice 4: an unavailable facet says since when. The time is
// recorded where the provenance string turns unavailable, kept while it
// stays so, and dropped once the source is back.
describe("an unavailable facet says since when (#14065 slice 4)", () => {
  const base = (): Provenance => ({
    board: "fixtures",
    roadmap: "fixtures",
    items: "fixtures",
    comments: "fixtures",
    quota: "fixtures",
    usage: "fixtures",
    attention: "fixtures",
    orchestration: "fixtures",
    dns: "fixtures",
    registry: "fixtures",
    aa: "fixtures",
  });

  test("the unavailable strings are recognised, partial ones included", () => {
    expect(isUnavailableProvenance("unavailable (projector unreachable; fixtures withheld in live mode)")).toBe(true);
    expect(
      isUnavailableProvenance(
        "live (fleet feed project pool) + unavailable for 2 ref(s) outside the pool (no projector configured; fixtures withheld in live mode)",
      ),
    ).toBe(true);
    expect(isUnavailableProvenance("live (projector)")).toBe(false);
    expect(isUnavailableProvenance("fixtures")).toBe(false);
  });

  test("the first time stands while the facet stays unavailable, and clears when it recovers", () => {
    let clock = 1_000;
    const since: Partial<Record<keyof Provenance, number>> = {};
    const p = trackUnavailable(since, base(), () => clock);
    expect(since.comments).toBeUndefined();
    p.comments = "unavailable (projector unreachable; fixtures withheld in live mode)";
    expect(since.comments).toBe(1_000);
    clock = 5_000;
    p.comments = "unavailable (no projector configured; fixtures withheld in live mode)";
    expect(since.comments).toBe(1_000);
    p.comments = "live (projector)";
    expect(since.comments).toBeUndefined();
    expect(p.comments).toBe("live (projector)");
    clock = 9_000;
    p.comments = "unavailable (projector unreachable; fixtures withheld in live mode)";
    expect(since.comments).toBe(9_000);
    // The record still reads and serialises like a plain one.
    expect(JSON.parse(JSON.stringify(p)).comments).toBe(p.comments);
    expect(Object.keys(p)).toEqual(Object.keys(base()));
  });

  test("a facet that starts unavailable is timed from the start", () => {
    const since: Partial<Record<keyof Provenance, number>> = {};
    trackUnavailable(since, { ...base(), dns: "unavailable (no fleet feed)" }, () => 42);
    expect(since).toEqual({ dns: 42 });
  });

  test("the live source reports since when, and the chip shows it", async () => {
    await withNoSources(async (src) => {
      const before = Date.now();
      await src.comments(["developing-today/code#752"]);
      const after = Date.now();
      const detail = src.provenanceDetail!();
      const at = detail.comments.unavailableSince;
      expect(at).toBeDefined();
      const ms = Date.parse(at!);
      expect(ms).toBeGreaterThanOrEqual(before);
      expect(ms).toBeLessThanOrEqual(after);
      // A facet that is not unavailable carries no time.
      expect(detail.board.unavailableSince).toBeUndefined();
      const chip = freshnessChips(detail, ["comments"]);
      expect(chip).toContain(`since ${unavailableSinceText(at!)}`);
      expect(chip).toContain(`unavailable since ${at}`);
    });
  });

  test("the since text is UTC to the minute", () => {
    expect(unavailableSinceText("2026-11-26T03:41:59.123Z")).toBe("2026-11-26 03:41 UTC");
    expect(unavailableSinceText("not a time")).toBe("not a time");
  });
});
