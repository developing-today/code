import { afterEach, describe, expect, it, vi } from "vitest";
import worker, { type Env } from "../src/index";
import { MemoryKV } from "./memory-kv";

const env: Env = {
  BRIDGE_BASE: "https://mcpx-link.security.cab",
  ACCESS_CLIENT_ID: "bridge-id.access",
  ACCESS_CLIENT_SECRET: "bridge-secret",
  MACHINE_BEARER: "machine-bearer-for-tests",
  ACCESS_AUD: "authorize-app-aud",
  ACCESS_TEAM_DOMAIN: "team.example.com",
  OAUTH_KV: new MemoryKV() as unknown as KVNamespace,
};

const AUTH = "Bearer machine-bearer-for-tests";

function post(path: string, body: unknown): Request {
  return new Request(`https://mcpx.security.cab${path}`, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      accept: "application/json, text/event-stream",
      authorization: AUTH,
    },
    body: JSON.stringify(body),
  });
}

function sseResponse(payload: unknown): Response {
  return new Response(`event: message\ndata: ${JSON.stringify(payload)}\n\n`, {
    status: 200,
    headers: { "content-type": "text/event-stream" },
  });
}

function stubBridge(payload: unknown) {
  const bridge = vi.fn(async (_url: unknown, _init?: unknown) => sseResponse(payload));
  vi.stubGlobal("fetch", bridge);
  return bridge;
}

function sentBody(bridge: ReturnType<typeof stubBridge>, index = 0): any {
  const [, init] = bridge.mock.calls[index] as unknown as [URL, RequestInit];
  return JSON.parse(String(init.body));
}

const TOOLS = [
  "mcpx_namespaces",
  "mcpx_catalog",
  "mcpx_types",
  "mcpx_search",
  "mcpx_call",
  "mcpx_batch",
  "mcpx_exec",
  "mcpx_tool_invoke",
  "mcpx_something_else",
];

function toolsListPayload() {
  return {
    jsonrpc: "2.0",
    id: 1,
    result: { tools: TOOLS.map((name) => ({ name })) },
  };
}

async function readSseJson(res: Response): Promise<any> {
  const text = await res.text();
  const line = text.split("\n").find((entry) => entry.startsWith("data:"));
  if (!line) throw new Error(`no SSE data line in ${JSON.stringify(text)}`);
  return JSON.parse(line.slice("data:".length).trim());
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("tools/list filtering", () => {
  it("default profile lists exactly the four discovery tools", async () => {
    stubBridge(toolsListPayload());
    const res = await worker.fetch(post("/mcp", { jsonrpc: "2.0", id: 1, method: "tools/list" }), env);
    expect(res.status).toBe(200);
    const message = await readSseJson(res);
    expect(message.result.tools.map((tool: any) => tool.name)).toEqual([
      "mcpx_namespaces",
      "mcpx_catalog",
      "mcpx_types",
      "mcpx_search",
    ]);
  });

  it("selected profile adds mcpx_call and mcpx_batch but never exec/extras", async () => {
    stubBridge(toolsListPayload());
    const res = await worker.fetch(post("/mcp?ns=codedb,fff", { jsonrpc: "2.0", id: 1, method: "tools/list" }), env);
    const message = await readSseJson(res);
    expect(message.result.tools.map((tool: any) => tool.name)).toEqual([
      "mcpx_namespaces",
      "mcpx_catalog",
      "mcpx_types",
      "mcpx_search",
      "mcpx_call",
      "mcpx_batch",
    ]);
  });

  it("filters SSE-framed tools/list responses by rewriting the data line", async () => {
    stubBridge(toolsListPayload());
    const res = await worker.fetch(post("/mcp", { jsonrpc: "2.0", id: 1, method: "tools/list" }), env);
    expect(res.headers.get("content-type")).toContain("text/event-stream");
    const text = await res.text();
    expect(text.startsWith("event: message\ndata: ")).toBe(true);
    expect(text).not.toContain("mcpx_exec");
  });
});

describe("tools/call gating", () => {
  it("allows an in-set mcpx_call and forwards it upstream", async () => {
    const bridge = stubBridge({ jsonrpc: "2.0", id: 2, result: { content: [] } });
    const res = await worker.fetch(
      post("/mcp?ns=codedb", {
        jsonrpc: "2.0",
        id: 2,
        method: "tools/call",
        params: { name: "mcpx_call", arguments: { namespace: "codedb", tool: "search", arguments: {} } },
      }),
      env,
    );
    expect(res.status).toBe(200);
    expect(bridge).toHaveBeenCalledTimes(1);
    expect(sentBody(bridge).params.name).toBe("mcpx_call");
  });

  it("rejects an out-of-set mcpx_call without calling upstream", async () => {
    const bridge = vi.fn();
    vi.stubGlobal("fetch", bridge);
    const res = await worker.fetch(
      post("/mcp?ns=codedb", {
        jsonrpc: "2.0",
        id: 3,
        method: "tools/call",
        params: { name: "mcpx_call", arguments: { namespace: "fff", tool: "grep" } },
      }),
      env,
    );
    expect(res.status).toBe(200);
    expect(bridge).not.toHaveBeenCalled();
    const message = (await res.json()) as any;
    expect(message.id).toBe(3);
    expect(message.error).toEqual({ code: -32602, message: "not exposed by this door's selector" });
  });

  it("handles the dotted-namespace form for mcpx_call", async () => {
    const bridge = stubBridge({ jsonrpc: "2.0", id: 4, result: { content: [] } });
    const res = await worker.fetch(
      post("/mcp?ns=codedb", {
        jsonrpc: "2.0",
        id: 4,
        method: "tools/call",
        params: { name: "mcpx_call", arguments: { namespace: "codedb.search", tool: "" } },
      }),
      env,
    );
    expect(res.status).toBe(200);
    expect(bridge).toHaveBeenCalledTimes(1);
  });

  it("rejects a batch containing one out-of-set item as a whole", async () => {
    const bridge = vi.fn();
    vi.stubGlobal("fetch", bridge);
    const res = await worker.fetch(
      post("/mcp?ns=codedb", {
        jsonrpc: "2.0",
        id: 5,
        method: "tools/call",
        params: {
          name: "mcpx_batch",
          arguments: {
            calls: [
              { namespace: "codedb", tool: "search" },
              { namespace: "fff", tool: "grep" },
            ],
          },
        },
      }),
      env,
    );
    expect(bridge).not.toHaveBeenCalled();
    const message = (await res.json()) as any;
    expect(message.error.code).toBe(-32602);
  });

  it("rejects a batch item that targets a gateway tool", async () => {
    const bridge = vi.fn();
    vi.stubGlobal("fetch", bridge);
    await worker.fetch(
      post("/mcp?ns=codedb", {
        jsonrpc: "2.0",
        id: 6,
        method: "tools/call",
        params: {
          name: "mcpx_batch",
          arguments: { calls: [{ namespace: "codedb", tool: "exec" }, { tool: "mcpx_exec" }] },
        },
      }),
      env,
    );
    expect(bridge).not.toHaveBeenCalled();
  });

  it("rejects mcpx_types when a requested namespace is out of set", async () => {
    const bridge = vi.fn();
    vi.stubGlobal("fetch", bridge);
    const res = await worker.fetch(
      post("/mcp?ns=codedb", {
        jsonrpc: "2.0",
        id: 7,
        method: "tools/call",
        params: { name: "mcpx_types", arguments: { namespaces: ["codedb", "fff"] } },
      }),
      env,
    );
    expect(bridge).not.toHaveBeenCalled();
    expect(((await res.json()) as any).error.code).toBe(-32602);
  });

  it("rejects mcpx_exec even when ns names a namespace", async () => {
    const bridge = vi.fn();
    vi.stubGlobal("fetch", bridge);
    const res = await worker.fetch(
      post("/mcp?ns=codedb", {
        jsonrpc: "2.0",
        id: 8,
        method: "tools/call",
        params: { name: "mcpx_exec", arguments: { namespace: "codedb" } },
      }),
      env,
    );
    expect(bridge).not.toHaveBeenCalled();
    expect(((await res.json()) as any).error.code).toBe(-32602);
  });

  it("rejects mcpx_exec in the default profile too", async () => {
    const bridge = vi.fn();
    vi.stubGlobal("fetch", bridge);
    const res = await worker.fetch(
      post("/mcp", {
        jsonrpc: "2.0",
        id: 9,
        method: "tools/call",
        params: { name: "mcpx_exec", arguments: {} },
      }),
      env,
    );
    expect(bridge).not.toHaveBeenCalled();
    expect(((await res.json()) as any).error.code).toBe(-32602);
  });
});

describe("ns=* full profile", () => {
  it("passes tools/list through unmodified", async () => {
    stubBridge(toolsListPayload());
    const res = await worker.fetch(post("/mcp?ns=*", { jsonrpc: "2.0", id: 1, method: "tools/list" }), env);
    const message = await readSseJson(res);
    expect(message.result.tools.map((tool: any) => tool.name)).toEqual(TOOLS);
  });

  it("passes an mcpx_exec call through unmodified", async () => {
    const bridge = stubBridge({ jsonrpc: "2.0", id: 2, result: { ok: true } });
    const res = await worker.fetch(
      post("/mcp?ns=*", {
        jsonrpc: "2.0",
        id: 2,
        method: "tools/call",
        params: { name: "mcpx_exec", arguments: { code: "1+1" } },
      }),
      env,
    );
    expect(bridge).toHaveBeenCalledTimes(1);
    expect(sentBody(bridge).params.name).toBe("mcpx_exec");
    expect(res.status).toBe(200);
  });
});

describe("response filtering", () => {
  it("filters an mcpx_namespaces response to the selected set", async () => {
    stubBridge({
      jsonrpc: "2.0",
      id: 2,
      result: { content: [{ type: "text", text: JSON.stringify(["codedb", "fff", "other"]) }] },
    });
    const res = await worker.fetch(
      post("/mcp?ns=codedb", {
        jsonrpc: "2.0",
        id: 2,
        method: "tools/call",
        params: { name: "mcpx_namespaces", arguments: {} },
      }),
      env,
    );
    const message = await readSseJson(res);
    expect(JSON.parse(message.result.content[0].text)).toEqual(["codedb"]);
  });

  it("returns an empty namespace list in the conservative profile", async () => {
    stubBridge({
      jsonrpc: "2.0",
      id: 2,
      result: { structuredContent: { namespaces: ["codedb", "fff"] } },
    });
    const res = await worker.fetch(
      post("/mcp", {
        jsonrpc: "2.0",
        id: 2,
        method: "tools/call",
        params: { name: "mcpx_namespaces", arguments: {} },
      }),
      env,
    );
    const message = await readSseJson(res);
    expect(message.result.structuredContent.namespaces).toEqual([]);
  });

  it("rejects a resources/read for an out-of-set mcpx:// URI", async () => {
    const bridge = vi.fn();
    vi.stubGlobal("fetch", bridge);
    const res = await worker.fetch(
      post("/mcp?ns=codedb", {
        jsonrpc: "2.0",
        id: 10,
        method: "resources/read",
        params: { uri: "mcpx://fff/doc" },
      }),
      env,
    );
    expect(bridge).not.toHaveBeenCalled();
    expect(((await res.json()) as any).error.code).toBe(-32602);
  });
});

describe("bare-path equivalence for filtering", () => {
  it("behaves identically to /mcp for a filtered call", async () => {
    const bridge = vi.fn();
    vi.stubGlobal("fetch", bridge);
    const res = await worker.fetch(
      post("/?ns=codedb", {
        jsonrpc: "2.0",
        id: 11,
        method: "tools/call",
        params: { name: "mcpx_call", arguments: { namespace: "fff", tool: "grep" } },
      }),
      env,
    );
    expect(bridge).not.toHaveBeenCalled();
    expect(((await res.json()) as any).error.code).toBe(-32602);
  });
});
