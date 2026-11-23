import { SELF } from "cloudflare:test";
import { fetchMock } from "./fetch-mock";
import { beforeAll, describe, expect, it } from "vitest";
import { EMBED_DIMS, EMBED_MODEL } from "../src/space";

// The MCP endpoint (#8625) rides the same gated search path as
// /v1/search; these tests pin the JSON-RPC surface and, via the
// upstream call counters + the captured service body, that an
// MCP search states the pinned disclosure context and shares
// the REST surface's Durable Object cache.

const GATE = { Authorization: "Bearer test-gate" };
const counts = { embed: 0, search: 0 };
let mcpSearchBody = "";

function embedReply() {
  counts.embed++;
  const vector = new Array(EMBED_DIMS).fill(0);
  vector[0] = 1.0;
  return {
    statusCode: 200,
    data: JSON.stringify({ model: EMBED_MODEL, dims: EMBED_DIMS, vector }),
    responseOptions: { headers: { "Content-Type": "application/json" } },
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
      job_id: "job-mcp-test",
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
  origin.intercept({ path: "/v1/embed", method: "POST" }).reply(embedReply).persist();
  origin
    .intercept({
      path: "/v1/search",
      method: "POST",
      body: (b: string) => {
        if (!b.includes("mcp context probe")) return false;
        mcpSearchBody = b;
        return true;
      },
    })
    .reply(searchReply)
    .persist();
  origin.intercept({
    path: "/v1/search", method: "POST", body: b => b.includes("schema error fixture"),
  }).reply(() => ({ statusCode: 503, data: "fixture unavailable" }));
  origin.intercept({ path: "/v1/search", method: "POST" }).reply(searchReply).persist();
});

async function rpc(message: unknown, headers?: Record<string, string>) {
  return SELF.fetch("https://worker.test/mcp", {
    method: "POST",
    headers: { "Content-Type": "application/json", ...(headers ?? GATE) },
    body: JSON.stringify(message),
  });
}

interface RpcResponse {
  jsonrpc: string;
  id?: string | number | null;
  result?: Record<string, any>;
  error?: { code: number; message: string };
}

describe("atlas-search MCP endpoint", () => {
  it("is gated, and POST-only", async () => {
    const denied = await rpc({ jsonrpc: "2.0", id: 1, method: "ping" }, {});
    expect(denied.status).toBe(401);
    const get = await SELF.fetch("https://worker.test/mcp", { headers: GATE });
    expect(get.status).toBe(405);
    expect(get.headers.get("Allow")).toBe("POST");
  });

  it("initialize negotiates a supported protocol version", async () => {
    const resp = await rpc({
      jsonrpc: "2.0",
      id: 1,
      method: "initialize",
      params: { protocolVersion: "2025-03-26", capabilities: {}, clientInfo: { name: "test", version: "0" } },
    });
    expect(resp.status).toBe(200);
    const body = (await resp.json()) as RpcResponse;
    expect(body.result?.protocolVersion).toBe("2025-03-26");
    expect(body.result?.serverInfo?.name).toBe("atlas-search");
    expect(body.result?.capabilities).toHaveProperty("tools");

    const fallback = await rpc({
      jsonrpc: "2.0",
      id: 2,
      method: "initialize",
      params: { protocolVersion: "1999-01-01", capabilities: {}, clientInfo: { name: "test", version: "0" } },
    });
    const fallbackBody = (await fallback.json()) as RpcResponse;
    expect(fallbackBody.result?.protocolVersion).toBe("2025-03-26");
  });

  it("answers ping, accepts notifications silently, rejects unknown methods", async () => {
    const ping = await rpc({ jsonrpc: "2.0", id: "p1", method: "ping" });
    expect(((await ping.json()) as RpcResponse).result).toEqual({});

    const note = await rpc({ jsonrpc: "2.0", method: "notifications/initialized" });
    expect(note.status).toBe(202);
    expect(await note.text()).toBe("");

    const unknown = await rpc({ jsonrpc: "2.0", id: 9, method: "resources/list" });
    const unknownBody = (await unknown.json()) as RpcResponse;
    expect(unknownBody.error?.code).toBe(-32601);
  });

  it("tools/list exposes exactly the search tool", async () => {
    const resp = await rpc({ jsonrpc: "2.0", id: 3, method: "tools/list" });
    const body = (await resp.json()) as RpcResponse;
    const tools = body.result?.tools as Array<Record<string, any>>;
    expect(tools).toHaveLength(1);
    expect(tools[0].name).toBe("search");
    expect(tools[0].inputSchema.required).toEqual(["query"]);
    expect(tools[0].annotations.readOnlyHint).toBe(true);
  });

  it("advertises a schema for successful structured results on revision 2025-06-18", async () => {
    const list = { jsonrpc: "2.0", id: 1, method: "tools/list" };
    const latest = { ...GATE, "MCP-Protocol-Version": "2025-06-18" };
    for (const headers of [GATE, { ...GATE, "MCP-Protocol-Version": "2025-03-26" }]) {
      const legacy = await (await rpc(list, headers)).json() as RpcResponse;
      expect(legacy.result?.tools[0]).not.toHaveProperty("outputSchema");
    }
    const current = await (await rpc(list, latest)).json() as RpcResponse;
    const schema = current.result?.tools[0].outputSchema;
    // Oracle: both real Worker cache paths obey the discovered envelope
    // contract. The service payload and the metadata may grow independently.
    expect(schema.type).toBe("object");
    expect(schema.required).toEqual(["cache", "key", "query", "result", "meta"]);
    for (const cache of ["miss", "hit"]) {
      const call = await rpc({ jsonrpc: "2.0", id: 2, method: "tools/call",
        params: { name: "search", arguments: { query: "schema success fixture" } } }, latest);
      const body = await call.json() as RpcResponse;
      const result = body.result?.structuredContent;
      expect(body.result?.isError).toBe(false);
      expect(result.cache).toBe(cache);
      expect(schema.properties.cache.enum).toContain(result.cache);
      for (const field of schema.required) {
        expect(result[field]).not.toBeNull();
        expect(Array.isArray(result[field])).toBe(false);
        expect(typeof result[field]).toBe(schema.properties[field].type);
      }
      expect(JSON.parse(body.result?.content[0].text)).toEqual(result);
    }
    const error = await rpc({ jsonrpc: "2.0", id: 3, method: "tools/call",
      params: { name: "search", arguments: { query: "schema error fixture" } } }, latest);
    const failed = await error.json() as RpcResponse;
    expect(failed.result?.isError).toBe(true);
    expect(failed.result).not.toHaveProperty("structuredContent");
  });

  it("tools/call search runs the shared path: context stated, cache shared with REST", async () => {
    const e0 = counts.embed;
    const s0 = counts.search;

    const call = await rpc({
      jsonrpc: "2.0",
      id: 4,
      method: "tools/call",
      params: { name: "search", arguments: { query: "mcp context probe" } },
    });
    expect(call.status).toBe(200);
    const body = (await call.json()) as RpcResponse;
    expect(body.result?.isError).toBe(false);
    const structured = body.result?.structuredContent as Record<string, any>;
    expect(structured.cache).toBe("miss");
    expect(structured.result.results[0].chunk_id).toBe(42);
    const text = (body.result?.content as Array<{ type: string; text: string }>)[0].text;
    expect(JSON.parse(text)).toEqual(structured);
    // The service saw the pinned disclosure context (#12622).
    expect(mcpSearchBody).toContain('"hosted-per-user"');
    expect(counts.embed).toBe(e0 + 1);
    expect(counts.search).toBe(s0 + 1);

    // The same query over REST is a hit on the very same cache
    // entry — the surfaces share one execution path.
    const rest = await SELF.fetch("https://worker.test/v1/search", {
      method: "POST",
      headers: { "Content-Type": "application/json", ...GATE },
      body: JSON.stringify({ query: "mcp context probe" }),
    });
    expect(rest.status).toBe(200);
    expect(((await rest.json()) as { cache: string }).cache).toBe("hit");
    expect(counts.embed).toBe(e0 + 1);
    expect(counts.search).toBe(s0 + 1);
  });

  it("tools/call validates the tool name and arguments", async () => {
    const badTool = await rpc({
      jsonrpc: "2.0",
      id: 5,
      method: "tools/call",
      params: { name: "purge", arguments: { query: "x" } },
    });
    expect(((await badTool.json()) as RpcResponse).error?.code).toBe(-32602);

    const badQuery = await rpc({
      jsonrpc: "2.0",
      id: 6,
      method: "tools/call",
      params: { name: "search", arguments: { query: "   " } },
    });
    expect(((await badQuery.json()) as RpcResponse).error?.code).toBe(-32602);

    const noQuery = await rpc({
      jsonrpc: "2.0",
      id: 7,
      method: "tools/call",
      params: { name: "search", arguments: {} },
    });
    expect(((await noQuery.json()) as RpcResponse).error?.code).toBe(-32602);
  });

  it("answers a batch with one response per request", async () => {
    const resp = await rpc([
      { jsonrpc: "2.0", id: 1, method: "ping" },
      { jsonrpc: "2.0", method: "notifications/initialized" },
      { jsonrpc: "2.0", id: 2, method: "tools/list" },
    ]);
    expect(resp.status).toBe(200);
    const body = (await resp.json()) as RpcResponse[];
    expect(body).toHaveLength(2);
    expect(body.map((r) => r.id)).toEqual([1, 2]);
  });
});
