import { describe, expect, it } from "bun:test";
import { LatestCommentsCache } from "../src/sources/live.ts";
import type { Comment } from "../src/model.ts";

const comment = (ref: string, body: string): Comment => ({
  id: `c-${ref}-${body}`,
  ref,
  kind: "issue_comment",
  author: "tester",
  body,
  createdAt: "2026-11-13T00:00:00Z",
  url: "https://example.invalid/comment",
});

/** Let fire-and-forget background refreshes land. */
const settle = () => new Promise((resolve) => setTimeout(resolve, 10));

describe("latest-comments SWR cache (#9781)", () => {
  it("serves fresh entries without re-fetching, in ref order", async () => {
    let calls = 0;
    const cache = new LatestCommentsCache(
      async (refs) => {
        calls++;
        return refs.map((ref) => comment(ref, "one"));
      },
      async () => [],
      60_000,
      () => 1_000,
    );
    const first = await cache.get(["o/r#2", "o/r#1"]);
    expect(first.map((c) => c.ref)).toEqual(["o/r#2", "o/r#1"]);
    const second = await cache.get(["o/r#1", "o/r#2"]);
    expect(second.map((c) => c.ref)).toEqual(["o/r#1", "o/r#2"]);
    expect(calls).toBe(1);
  });

  it("serves the stale comment past the TTL and refreshes behind it", async () => {
    let now = 0;
    let generation = 0;
    const cache = new LatestCommentsCache(
      async (refs) => {
        generation++;
        return refs.map((ref) => comment(ref, `gen${generation}`));
      },
      async () => [],
      100,
      () => now,
    );
    expect((await cache.get(["o/r#1"]))[0]?.body).toBe("gen1");
    now = 500; // past the TTL: stale is served, refresh runs behind
    expect((await cache.get(["o/r#1"]))[0]?.body).toBe("gen1");
    await settle();
    expect((await cache.get(["o/r#1"]))[0]?.body).toBe("gen2");
  });

  it("a failed refresh keeps the stale comment; a failed cold read falls back uncached", async () => {
    let now = 0;
    let failing = false;
    const cache = new LatestCommentsCache(
      async (refs) => {
        if (failing) return null;
        return refs.map((ref) => comment(ref, "live"));
      },
      async (refs) => refs.map((ref) => comment(ref, "fallback")),
      100,
      () => now,
    );
    expect((await cache.get(["o/r#1"]))[0]?.body).toBe("live");
    failing = true;
    now = 500;
    expect((await cache.get(["o/r#1"]))[0]?.body).toBe("live"); // stale served
    await settle();
    expect((await cache.get(["o/r#1"]))[0]?.body).toBe("live"); // still the stale value
    // A never-seen ref under failure gets the fallback, and the failure
    // is not cached: once the projector recovers, the live read wins.
    expect((await cache.get(["o/r#9"]))[0]?.body).toBe("fallback");
    failing = false;
    expect((await cache.get(["o/r#9"]))[0]?.body).toBe("live");
  });

  it("refs with no comments are cached as empty and not re-fetched within the TTL", async () => {
    let calls = 0;
    const cache = new LatestCommentsCache(
      async () => {
        calls++;
        return [];
      },
      async () => [],
      60_000,
      () => 0,
    );
    expect(await cache.get(["o/r#1"])).toEqual([]);
    expect(await cache.get(["o/r#1"])).toEqual([]);
    expect(calls).toBe(1);
  });
});
