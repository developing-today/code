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
import { emptyQuotaView, liveSource } from "../src/sources/live.ts";

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
