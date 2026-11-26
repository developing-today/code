// #7983 item 5: Durable Object answers and the DO's own inbound body are
// shape-checked at the boundary. The probe matrix from the #3068 contract:
// null, array, scalar, {}, and each relied-on field wrong-typed.
import { env } from "cloudflare:test";
import { describe, expect, it } from "vitest";
import { doJsonObject, executeSearch, MAX_K } from "../src/search";

/** A QUERY_CACHE stand-in whose one stub answers `body` with `status`. */
function cacheAnswering(body: string, status = 200) {
  const stub = { fetch: async () => new Response(body, { status, headers: { "Content-Type": "application/json" } }) };
  return {
    QUERY_CACHE: {
      idFromName: (name: string) => name,
      get: () => stub,
    } as unknown as DurableObjectNamespace,
  };
}

describe("Durable Object answer shapes (#7983 item 5)", () => {
  it("doJsonObject accepts only a JSON object", async () => {
    expect(await doJsonObject(new Response('{"a":1}'))).toEqual({ a: 1 });
    for (const bad of ["null", "[]", "[1]", "3", '"x"', "true", "not json"]) {
      expect(await doJsonObject(new Response(bad))).toBeNull();
    }
  });

  it("a malformed query answer is a 502 envelope, never a thrown TypeError", async () => {
    for (const bad of ["null", "[]", "7", "not json", "{}", '{"outcome":1}', '{"outcome":"maybe"}']) {
      const { status, payload } = await executeSearch(cacheAnswering(bad), "q", 10);
      expect(status).toBe(502);
      expect((payload.error as { code: string }).code).toBe("upstream_malformed");
    }
  });

  it("a well-formed answer still becomes the REST envelope", async () => {
    const { status, payload } = await executeSearch(cacheAnswering('{"outcome":"hit","result":[],"meta":{}}'), "q", 10);
    expect(status).toBe(200);
    expect(payload).toMatchObject({ cache: "hit", query: "q", result: [], meta: {} });
  });

  it("a DO error object passes through with its status", async () => {
    const { status, payload } = await executeSearch(
      cacheAnswering('{"error":{"code":"query_failed","message":"boom"}}', 503),
      "q",
      10,
    );
    expect(status).toBe(503);
    expect(payload).toEqual({ error: { code: "query_failed", message: "boom" } });
  });
});

describe("the DO refuses a malformed query body before storing anything (#7983 item 5)", () => {
  const cache = (env as unknown as { QUERY_CACHE: DurableObjectNamespace }).QUERY_CACHE;
  const stub = (name: string) => cache.get(cache.idFromName(name));
  const post = (name: string, body: string) => stub(name).fetch("https://do/query", { method: "POST", body });

  it("refuses each probe with the 400 envelope and leaves no entry behind", async () => {
    const probes = [
      "not json",
      "null",
      "[]",
      "5",
      "{}",
      '{"query":1,"k":10}',
      '{"query":"","k":10}',
      '{"query":"q"}',
      '{"query":"q","k":"10"}',
      '{"query":"q","k":0}',
      `{"query":"q","k":${MAX_K + 1}}`,
      '{"query":"q","k":2.5}',
    ];
    for (const [i, body] of probes.entries()) {
      const name = `shape-probe-${i}`;
      const resp = await post(name, body);
      expect(resp.status, body).toBe(400);
      const json = (await resp.json()) as { error: { code: string } };
      expect(json.error.code).toBe("bad_request");
      const inspect = await stub(name).fetch("https://do/inspect");
      const meta = (await inspect.json()) as Record<string, unknown>;
      expect(meta.exists, body).toBe(false);
    }
  });
});
