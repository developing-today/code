// #2427 (queue task 5324): the live source's attention() consumer must
// drop malformed fleet-feed attention entries before the sessionId
// dedupe insert. isFleetFeed only checks that `attention` is an array:
// a null entry crashes /attention on `.sessionId`, and entries without
// a string sessionId silently collapse into one map slot.
import { describe, expect, test } from "bun:test";
import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadConfig } from "../src/config.ts";
import { isFeedAttentionEntry, liveSource } from "../src/sources/live.ts";

async function sourceWithAttention(attention: unknown[]): Promise<ReturnType<typeof liveSource>> {
  const dir = await mkdtemp(join(tmpdir(), "pm-attention-filter-"));
  const feedFile = join(dir, "fleet-feed.json");
  await writeFile(
    feedFile,
    JSON.stringify({
      version: 1,
      generatedAt: new Date().toISOString(),
      quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
      attention,
    }),
  );
  const cfg = loadConfig({
    PM_AGG_SOURCE: "live",
    PM_AGG_FEED_FILE: feedFile,
    PM_AGG_QUOTA_VIEW: join(dir, "no-quota-view.md"),
    PM_AGG_SUPABASE_EXPORT: join(dir, "no-supabase-export.json"),
    PM_AGG_CLOUDFLARE_EXPORT: join(dir, "no-cloudflare-export.json"),
    PM_AGG_REGISTRY_FILE: join(dir, "registry.json"),
    PM_AGG_COMMENTS_FILE: join(dir, "comments.json"),
    PM_AGG_SNAPSHOTS_DIR: join(dir, "snapshots"),
    PM_AGG_OPENCHAMBER_STATE: join(dir, "no-state.json"),
    PM_PROJECTOR_URL: "off",
    KANEO_ADMIN_EMAIL: "",
    KANEO_ADMIN_PASSWORD: "",
    GRIST_API_KEY: "",
  });
  return liveSource(cfg);
}

const VALID_1 = { sessionId: "ses_valid_1", lastActivityAt: null, title: "first copy" };
const VALID_1_DUP = { sessionId: "ses_valid_1", lastActivityAt: null, title: "second copy wins" };
const VALID_2 = { sessionId: "ses_valid_2", lastActivityAt: null, title: "other session" };

describe("isFeedAttentionEntry (#2427)", () => {
  test("accepts an entry with a non-empty string sessionId", () => {
    expect(isFeedAttentionEntry(VALID_1)).toBe(true);
  });
  test("rejects null, non-objects, and missing/non-string/empty sessionIds", () => {
    expect(isFeedAttentionEntry(null)).toBe(false);
    expect(isFeedAttentionEntry("junk")).toBe(false);
    expect(isFeedAttentionEntry(42)).toBe(false);
    expect(isFeedAttentionEntry({})).toBe(false);
    expect(isFeedAttentionEntry({ sessionId: 123 })).toBe(false);
    expect(isFeedAttentionEntry({ sessionId: "" })).toBe(false);
    expect(isFeedAttentionEntry({ sessionId: null })).toBe(false);
  });
});

describe("live source attention(): malformed feed entries (#2427)", () => {
  test("null and id-less entries are dropped; valid entries survive and dedupe by sessionId", async () => {
    const src = await sourceWithAttention([
      null,
      "junk",
      42,
      { title: "no sessionId at all" },
      { sessionId: 123, title: "numeric sessionId" },
      { sessionId: "", title: "empty sessionId" },
      VALID_1,
      VALID_2,
      VALID_1_DUP,
    ]);
    const entries = await src.attention();
    // Every returned entry is keyed by a real session id (nothing
    // collapsed onto an undefined/empty key).
    for (const e of entries) {
      expect(typeof e.sessionId).toBe("string");
      expect(e.sessionId.length).toBeGreaterThan(0);
    }
    const feed1 = entries.filter((e) => e.sessionId === "ses_valid_1");
    const feed2 = entries.filter((e) => e.sessionId === "ses_valid_2");
    expect(feed1.length).toBe(1);
    expect(feed1[0]?.title).toBe("second copy wins");
    expect(feed2.length).toBe(1);
    expect(feed2[0]?.title).toBe("other session");
  });

  test("a feed whose attention entries are all malformed does not throw", async () => {
    const src = await sourceWithAttention([null, { sessionId: 7 }, "junk"]);
    // Falls back to fixtures/laptop (fail-soft) instead of crashing.
    const entries = await src.attention();
    expect(Array.isArray(entries)).toBe(true);
    for (const e of entries) {
      expect(typeof e.sessionId).toBe("string");
      expect(e.sessionId.length).toBeGreaterThan(0);
    }
  });
});
