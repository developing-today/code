import { afterEach, describe, expect, it, vi } from "vitest";
import worker, { type Env } from "../src/index";

const env: Env = {
  BRIDGE_BASE: "https://commander-link.security.cab",
  ACCESS_CLIENT_ID: "bridge-id.access",
  ACCESS_CLIENT_SECRET: "bridge-secret",
};

function mcpRequest(withAssertion: boolean): Request {
  const headers: Record<string, string> = {
    "content-type": "application/json",
    accept: "application/json, text/event-stream",
  };
  if (withAssertion) headers["cf-access-jwt-assertion"] = "test-jwt";
  return new Request("https://commander.security.cab/mcp", {
    method: "POST",
    headers,
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/list" }),
  });
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("desktop-commander Worker proxy", () => {
  it("answers /healthz without touching the bridge", async () => {
    const bridge = vi.fn();
    vi.stubGlobal("fetch", bridge);
    const res = await worker.fetch(
      new Request("https://commander.security.cab/healthz"),
      env,
    );
    expect(res.status).toBe(200);
    expect(await res.json()).toEqual({ ok: true, service: "desktop-commander" });
    expect(bridge).not.toHaveBeenCalled();
  });

  it("refuses /mcp without an Access assertion and never calls the bridge", async () => {
    const bridge = vi.fn();
    vi.stubGlobal("fetch", bridge);
    const res = await worker.fetch(mcpRequest(false), env);
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
    const res = await worker.fetch(mcpRequest(true), env);
    expect(res.status).toBe(200);
    expect(res.headers.get("mcp-session-id")).toBe("session-123");

    expect(bridge).toHaveBeenCalledTimes(1);
    const [calledUrl, calledInit] = bridge.mock.calls[0] as unknown as [
      URL,
      RequestInit,
    ];
    expect(String(calledUrl)).toBe("https://commander-link.security.cab/mcp");
    const sent = new Headers(calledInit.headers);
    expect(sent.get("cf-access-client-id")).toBe("bridge-id.access");
    expect(sent.get("cf-access-client-secret")).toBe("bridge-secret");
    expect(sent.get("cf-access-jwt-assertion")).toBeNull();
  });

  it("404s unknown paths", async () => {
    vi.stubGlobal("fetch", vi.fn());
    const res = await worker.fetch(
      new Request("https://commander.security.cab/nope"),
      env,
    );
    expect(res.status).toBe(404);
  });
});
