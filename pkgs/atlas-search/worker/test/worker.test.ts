import { fetchMock, SELF } from "cloudflare:test";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { EMBED_DIMS, EMBED_MODEL } from "../src/space";

const GATE = { Authorization: "Bearer test-gate" };
const counts = { embed: 0, search: 0 };

function embedReply(space: "good" | "bad") {
  return () => {
    counts.embed++;
    if (space === "bad") {
      return {
        statusCode: 200,
        data: JSON.stringify({
          model: "@cf/baai/bge-small-en-v1.5",
          dims: 384,
          vector: new Array(384).fill(0),
        }),
        responseOptions: { headers: { "Content-Type": "application/json" } },
      };
    }
    const vector = new Array(EMBED_DIMS).fill(0);
    vector[0] = 1.0;
    return {
      statusCode: 200,
      data: JSON.stringify({ model: EMBED_MODEL, dims: EMBED_DIMS, vector }),
      responseOptions: { headers: { "Content-Type": "application/json" } },
    };
  };
}

function searchReply() {
  counts.search++;
  return {
    statusCode: 200,
    data: JSON.stringify({
      status: "complete",
      effective_ceiling: "personal",
      total_results: 1,
      job_id: "job-test",
      next_cursor: null,
      envelope: {},
      results: [{ chunk_id: 42, text: "the claim protocol", score: 0.5 }],
    }),
    responseOptions: { headers: { "Content-Type": "application/json" } },
  };
}

beforeAll(() => {
  fetchMock.activate();
  fetchMock.disableNetConnect();
  const origin = fetchMock.get("https://service.test");
  origin
    .intercept({
      path: "/v1/embed",
      method: "POST",
      body: (b: string) => b.includes("badspace query"),
    })
    .reply(embedReply("bad"))
    .persist();
  origin
    .intercept({ path: "/v1/embed", method: "POST" })
    .reply(embedReply("good"))
    .persist();
  origin
    .intercept({
      path: "/v1/search",
      method: "POST",
      body: (b: string) => b.includes("nullsearch query"),
    })
    .reply(() => {
      counts.search++;
      return {
        statusCode: 200,
        data: "null",
        responseOptions: { headers: { "Content-Type": "application/json" } },
      };
    })
    .persist();
  origin
    .intercept({ path: "/v1/search", method: "POST" })
    .reply(searchReply)
    .persist();
});

afterEach(() => {
  // Every interceptor is persistent; unexpected outbound calls
  // fail via disableNetConnect instead of leaking.
  expect(fetchMock).toBeDefined();
});

async function search(body: unknown, headers?: Record<string, string>) {
  return SELF.fetch("https://worker.test/v1/search", {
    method: "POST",
    headers: { "Content-Type": "application/json", ...(headers ?? GATE) },
    body: JSON.stringify(body),
  });
}

describe("atlas-search worker", () => {
  it("health is open, search is gated", async () => {
    const health = await SELF.fetch("https://worker.test/health");
    expect(health.status).toBe(200);
    const embedBefore = counts.embed;
    const denied = await search({ query: "alpha beta" }, {});
    expect(denied.status).toBe(401);
    expect(counts.embed).toBe(embedBefore);
  });

  it("miss embeds + fans out once; the repeat is a DO hit, byte-identical", async () => {
    // Cache-hit identity oracle: the upstream call counters. The
    // second request (same normalized query, messier whitespace)
    // must not move them, and its result must deep-equal the
    // first response's result.
    const e0 = counts.embed;
    const s0 = counts.search;

    const first = await search({ query: "  alpha   beta " });
    expect(first.status).toBe(200);
    const firstBody = (await first.json()) as Record<string, unknown>;
    expect(firstBody.cache).toBe("miss");
    expect(counts.embed).toBe(e0 + 1);
    expect(counts.search).toBe(s0 + 1);

    const second = await search({ query: "alpha beta" });
    expect(second.status).toBe(200);
    const secondBody = (await second.json()) as Record<string, unknown>;
    expect(secondBody.cache).toBe("hit");
    expect(secondBody.result).toEqual(firstBody.result);
    expect(counts.embed).toBe(e0 + 1);
    expect(counts.search).toBe(s0 + 1);

    const inspect = await SELF.fetch(
      "https://worker.test/v1/cache/inspect?query=alpha%20beta",
      { headers: GATE },
    );
    expect(inspect.status).toBe(200);
    const meta = (await inspect.json()) as Record<string, unknown>;
    expect(meta.exists).toBe(true);
    expect(meta.has_embedding).toBe(true);
    expect(meta.embedding_dims).toBe(768);
    expect(meta.embedding_valid).toBe(true);
    expect(meta.has_result).toBe(true);
    expect(meta.hit_count).toBe(1);
  });

  it("rejects a whitespace-only query on both routes (#6937)", async () => {
    const e0 = counts.embed;
    const s0 = counts.search;
    const deniedSearch = await search({ query: "   " });
    expect(deniedSearch.status).toBe(400);
    const inspect = await SELF.fetch(
      "https://worker.test/v1/cache/inspect?query=%20%20%20",
      { headers: GATE },
    );
    expect(inspect.status).toBe(400);
    const body = (await inspect.json()) as { error?: { message?: string } };
    expect(body.error?.message).toBe("query must not be empty");
    // Neither route reached the service or a Durable Object fill.
    expect(counts.embed).toBe(e0);
    expect(counts.search).toBe(s0);
  });

  it("refuses to cache a wrong-space embedding", async () => {
    const resp = await search({ query: "badspace query" });
    expect(resp.status).toBe(502);
    const inspect = await SELF.fetch(
      "https://worker.test/v1/cache/inspect?query=badspace%20query",
      { headers: GATE },
    );
    const meta = (await inspect.json()) as Record<string, unknown>;
    // Nothing stored at all: validation happens before the DO
    // write, so a bad-space vector cannot enter the cache.
    expect(meta.has_result ?? false).toBe(false);
    expect(meta.has_embedding ?? false).toBe(false);
  });

  it("returns the 400 envelope for a JSON null body, not a thrown TypeError (#6879)", async () => {
    const e0 = counts.embed;
    const s0 = counts.search;
    // `null` is valid JSON, so req.json() resolves to null and the
    // query check must not dereference it. Oracle: the route's own
    // error envelope plus untouched upstream counters.
    const resp = await search(null);
    expect(resp.status).toBe(400);
    const body = (await resp.json()) as {
      error?: { code?: string; message?: string };
    };
    expect(body.error?.code).toBe("bad_request");
    expect(body.error?.message).toBe("body must be a JSON object");
    expect(counts.embed).toBe(e0);
    expect(counts.search).toBe(s0);
  });

  it("reports a null search-service response as a 502, not a raw TypeError (#6982)", async () => {
    const resp = await search({ query: "nullsearch query" });
    expect(resp.status).toBe(502);
    const body = (await resp.json()) as { error?: { message?: string } };
    expect(body.error?.message).toContain("non-object response (null)");
    const inspect = await SELF.fetch(
      "https://worker.test/v1/cache/inspect?query=nullsearch%20query",
      { headers: GATE },
    );
    const meta = (await inspect.json()) as Record<string, unknown>;
    // The embedding was stored before the search ran (the entry stays
    // retryable in the embedded state); no result was cached.
    expect(meta.has_embedding).toBe(true);
    expect(meta.has_result).toBe(false);
  });
});
