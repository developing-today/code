// Issue #10825: every non-JWKS outbound fetch must carry a timeout
// signal so a stalled upstream cannot park the handler forever.
import { afterEach, describe, expect, test } from "bun:test";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { QueueClient } from "../src/queue.ts";
import { OpenCodeClient } from "../src/opencode.ts";
import { GitHubClient } from "../src/github.ts";
import { OUTBOUND_FETCH_TIMEOUT_MS, outboundTimeout } from "../src/fetchTimeout.ts";

const origFetch = globalThis.fetch;
afterEach(() => {
  globalThis.fetch = origFetch;
});

describe("outbound fetch timeouts (issue #10825)", () => {
  test("timeout bound is a finite positive number", () => {
    expect(OUTBOUND_FETCH_TIMEOUT_MS).toBeGreaterThan(0);
    expect(Number.isFinite(OUTBOUND_FETCH_TIMEOUT_MS)).toBe(true);
  });

  test("outboundTimeout returns an AbortSignal", () => {
    const sig = outboundTimeout();
    expect(sig).toBeInstanceOf(AbortSignal);
  });

  test("queue Supabase fetch carries a timeout signal", async () => {
    let seen: unknown;
    globalThis.fetch = (async (_url: unknown, init?: unknown) => {
      seen = init;
      return Response.json([]);
    }) as typeof fetch;
    const q = new QueueClient("https://example.supabase.co", "key");
    await q.findByDedupeKey("k");
    expect((seen as { signal?: unknown }).signal).toBeInstanceOf(AbortSignal);
  });

  test("opencode fetch carries a timeout signal", async () => {
    let seen: unknown;
    globalThis.fetch = (async (_url: unknown, init?: unknown) => {
      seen = init;
      return new Response("null", { status: 200 });
    }) as typeof fetch;
    const dir = mkdtempSync(join(tmpdir(), "dispatch-10825-"));
    const authFile = join(dir, "auth.json");
    writeFileSync(authFile, JSON.stringify({ password: "secret" }));
    const c = new OpenCodeClient("http://127.0.0.1:9", authFile);
    await c.listSessions();
    expect((seen as { signal?: unknown }).signal).toBeInstanceOf(AbortSignal);
  });

  test("github appApi fetch carries a timeout signal", async () => {
    let seen: unknown;
    globalThis.fetch = (async (_url: unknown, init?: unknown) => {
      seen = init;
      return new Response("{}", { status: 200 });
    }) as typeof fetch;
    const g = new GitHubClient(null, null);
    await (g as unknown as { appApi: (m: string, p: string, t: string) => Promise<unknown> }).appApi(
      "GET",
      "/repos/a/b/issues/1",
      "token",
    );
    expect((seen as { signal?: unknown }).signal).toBeInstanceOf(AbortSignal);
  });
});
