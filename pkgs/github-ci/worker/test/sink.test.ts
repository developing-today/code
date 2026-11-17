import { afterEach, describe, expect, it, vi } from "vitest";
import { appendAgentEvent, type SinkEnv } from "../src/sink";

const brokerEnv: SinkEnv = {
  TURSO_QUEUE_URL: "https://fleet-queue.example.invalid",
  TURSO_QUEUE_TOKEN: "queue-token",
  EVENTS_BROKER_URL: "https://queue-broker.security.cab/",
  EVENTS_BROKER_TOKEN: "broker-token",
  EVENTS_BROKER_ACCESS_CLIENT_ID: "access-id",
  EVENTS_BROKER_ACCESS_CLIENT_SECRET: "access-secret",
};

afterEach(() => vi.unstubAllGlobals());

describe("appendAgentEvent broker route (#6925)", () => {
  it("POSTs /events-append with X-Agent-Token and CF-Access headers", async () => {
    const calls: { url: string; init: RequestInit }[] = [];
    vi.stubGlobal("fetch", async (url: string, init: RequestInit) => {
      calls.push({ url, init });
      return new Response(JSON.stringify({ ok: true, row: { id: 7, kind: "push" } }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      });
    });
    await appendAgentEvent(brokerEnv, "push", { repo: "developing-today/code" });
    expect(calls).toHaveLength(1);
    expect(calls[0].url).toBe("https://queue-broker.security.cab/events-append");
    const headers = calls[0].init.headers as Record<string, string>;
    expect(headers["X-Agent-Token"]).toBe("broker-token");
    expect(headers["CF-Access-Client-Id"]).toBe("access-id");
    expect(headers["CF-Access-Client-Secret"]).toBe("access-secret");
    expect(headers["Authorization"]).toBeUndefined();
    expect(JSON.parse(String(calls[0].init.body))).toEqual({
      worker_id: "github-webhook",
      kind: "push",
      detail: { repo: "developing-today/code" },
    });
  });

  it("omits auth headers when the broker secrets are unset", async () => {
    const calls: { init: RequestInit }[] = [];
    vi.stubGlobal("fetch", async (_url: string, init: RequestInit) => {
      calls.push({ init });
      return new Response(JSON.stringify({ ok: true }), { status: 200 });
    });
    const env: SinkEnv = { ...brokerEnv, EVENTS_BROKER_TOKEN: undefined, EVENTS_BROKER_ACCESS_CLIENT_ID: undefined, EVENTS_BROKER_ACCESS_CLIENT_SECRET: undefined };
    await appendAgentEvent(env, "issues", {});
    const headers = calls[0].init.headers as Record<string, string>;
    expect(headers["X-Agent-Token"]).toBeUndefined();
    expect(headers["CF-Access-Client-Id"]).toBeUndefined();
    expect(headers["CF-Access-Client-Secret"]).toBeUndefined();
  });

  it("throws when the broker answers not-ok", async () => {
    vi.stubGlobal("fetch", async () => new Response(JSON.stringify({ ok: false, error: "nope" }), { status: 200 }));
    await expect(appendAgentEvent(brokerEnv, "push", {})).rejects.toThrow("broker returned not-ok");
  });

  it("throws on a non-2xx broker response", async () => {
    vi.stubGlobal("fetch", async () => new Response("unauthorized", { status: 401 }));
    await expect(appendAgentEvent(brokerEnv, "push", {})).rejects.toThrow("HTTP 401");
  });
});
