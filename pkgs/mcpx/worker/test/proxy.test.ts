import { afterEach, describe, expect, it, vi } from "vitest";
import worker, { type Env } from "../src/index";
import { DEFAULT_FILTER_PROFILE, resolveFilterSelector } from "../src/filter";
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

function mcpRequest(authorization?: string): Request {
  const headers: Record<string, string> = {
    "content-type": "application/json",
    accept: "application/json, text/event-stream",
  };
  if (authorization) headers["authorization"] = authorization;
  return new Request("https://mcpx.security.cab/mcp", {
    method: "POST",
    headers,
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/list" }),
  });
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("mcpx Worker proxy", () => {
  it("answers /healthz without touching the bridge", async () => {
    const bridge = vi.fn();
    vi.stubGlobal("fetch", bridge);
    const res = await worker.fetch(
      new Request("https://mcpx.security.cab/healthz"),
      env,
    );
    expect(res.status).toBe(200);
    expect(await res.json()).toEqual({ ok: true, service: "mcpx" });
    expect(bridge).not.toHaveBeenCalled();
  });

  it("refuses /mcp without a Bearer token and never calls the bridge", async () => {
    const bridge = vi.fn();
    vi.stubGlobal("fetch", bridge);
    const res = await worker.fetch(mcpRequest(), env);
    expect(res.status).toBe(401);
    expect(res.headers.get("www-authenticate")).toBe(
      'Bearer resource_metadata="https://mcpx.security.cab/.well-known/oauth-protected-resource"',
    );
    expect(bridge).not.toHaveBeenCalled();
  });

  it("refuses /mcp with an unknown Bearer token", async () => {
    const bridge = vi.fn();
    vi.stubGlobal("fetch", bridge);
    const res = await worker.fetch(mcpRequest("Bearer not-a-real-token"), env);
    expect(res.status).toBe(401);
    expect(bridge).not.toHaveBeenCalled();
  });

  it("proxies /mcp to the bridge with the egress token, not the caller's", async () => {
    const bridge = vi.fn(async (_url: unknown, _init?: unknown) => {
      return new Response("event: message\ndata: {}\n\n", {
        status: 200,
        headers: {
          "content-type": "text/event-stream",
          "mcp-session-id": "session-123",
        },
      });
    });
    vi.stubGlobal("fetch", bridge);
    const res = await worker.fetch(mcpRequest("Bearer machine-bearer-for-tests"), env);
    expect(res.status).toBe(200);
    expect(res.headers.get("mcp-session-id")).toBe("session-123");

    expect(bridge).toHaveBeenCalledTimes(1);
    const [calledUrl, calledInit] = bridge.mock.calls[0] as unknown as [
      URL,
      RequestInit,
    ];
    expect(String(calledUrl)).toBe("https://mcpx-link.security.cab/mcp");
    const sent = new Headers(calledInit.headers);
    expect(sent.get("cf-access-client-id")).toBe("bridge-id.access");
    expect(sent.get("cf-access-client-secret")).toBe("bridge-secret");
    expect(sent.get("authorization")).toBeNull();
    expect(sent.get("cf-access-jwt-assertion")).toBeNull();
  });

  it("404s unknown paths", async () => {
    vi.stubGlobal("fetch", vi.fn());
    const res = await worker.fetch(
      new Request("https://mcpx.security.cab/nope"),
      env,
    );
    expect(res.status).toBe(404);
  });
});


function bareRequest(path: string, authorization?: string): Request {
  const headers: Record<string, string> = {
    "content-type": "application/json",
    accept: "application/json, text/event-stream",
  };
  if (authorization) headers["authorization"] = authorization;
  return new Request(`https://mcpx.security.cab${path}`, {
    method: "POST",
    headers,
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/list" }),
  });
}

function stubBridge() {
  const bridge = vi.fn(async (_url: unknown, _init?: unknown) => {
    return new Response("event: message\ndata: {}\n\n", {
      status: 200,
      headers: { "content-type": "text/event-stream", "mcp-session-id": "session-123" },
    });
  });
  vi.stubGlobal("fetch", bridge);
  return bridge;
}

describe("bare-path equivalence", () => {
  it("refuses / without a Bearer exactly like /mcp", async () => {
    const bridge = vi.fn();
    vi.stubGlobal("fetch", bridge);
    const res = await worker.fetch(bareRequest("/"), env);
    expect(res.status).toBe(401);
    expect(res.headers.get("www-authenticate")).toBe(
      'Bearer resource_metadata="https://mcpx.security.cab/.well-known/oauth-protected-resource"',
    );
    expect(bridge).not.toHaveBeenCalled();
  });

  it("proxies / to the bridge /mcp with the egress token", async () => {
    const bridge = stubBridge();
    const res = await worker.fetch(bareRequest("/", "Bearer machine-bearer-for-tests"), env);
    expect(res.status).toBe(200);
    expect(bridge).toHaveBeenCalledTimes(1);
    const [calledUrl] = bridge.mock.calls[0] as unknown as [URL, RequestInit];
    expect(String(calledUrl)).toBe("https://mcpx-link.security.cab/mcp");
  });

  it("consumes the ns selector and forwards other query parameters", async () => {
    const bridge = stubBridge();
    const res = await worker.fetch(
      bareRequest("/mcp?ns=readonly&x=1", "Bearer machine-bearer-for-tests"),
      env,
    );
    expect(res.status).toBe(200);
    const [calledUrl] = bridge.mock.calls[0] as unknown as [URL, RequestInit];
    expect(String(calledUrl)).toBe("https://mcpx-link.security.cab/mcp?x=1");
  });

  it("bare / with a selector still lands on upstream /mcp", async () => {
    const bridge = stubBridge();
    const res = await worker.fetch(
      bareRequest("/?ns=readonly", "Bearer machine-bearer-for-tests"),
      env,
    );
    expect(res.status).toBe(200);
    const [calledUrl] = bridge.mock.calls[0] as unknown as [URL, RequestInit];
    expect(String(calledUrl)).toBe("https://mcpx-link.security.cab/mcp");
  });
});

describe("filter selector", () => {
  it("defaults to the named conservative profile without ns", () => {
    const selector = resolveFilterSelector(new URL("https://mcpx.security.cab/mcp"));
    expect(selector.raw).toBeNull();
    expect(selector.profile).toBe(DEFAULT_FILTER_PROFILE);
    expect(DEFAULT_FILTER_PROFILE).toBe("conservative");
  });

  it("uses the ns value as the profile when present", () => {
    const selector = resolveFilterSelector(new URL("https://mcpx.security.cab/mcp?ns=readonly"));
    expect(selector.raw).toBe("readonly");
    expect(selector.profile).toBe("readonly");
  });
});
