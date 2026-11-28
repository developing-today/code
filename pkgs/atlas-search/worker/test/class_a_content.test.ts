import { describe, expect, it, beforeAll, afterEach } from "vitest";
import { fetchMock } from "./fetch-mock";
import { fetchClassAContent, type ClassAContentEnv } from "../src/search";
import type { SkipLog } from "../src/read_path";

const SERVICE_BASE = "https://service.test";
const SERVICE = {
  SERVICE_BASE,
  SERVICE_TOKEN: "test-service-token",
  ACCESS_CLIENT_ID: "test-access-id",
  ACCESS_CLIENT_SECRET: "test-access-secret",
};

const TEXT = "the authorized chunk text";
let HASH = "";

async function sha256Hex(bytes: Uint8Array): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", bytes as unknown as ArrayBuffer);
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

interface SeenRequest {
  method: string;
  url: string;
  authorization: string | null;
  accessId: string | null;
  accessSecret: string | null;
}
const seen: SeenRequest[] = [];

function hydrationBody(text: string): Record<string, unknown> {
  return { chunk_id: 7, text, score: 0.9, source: "shard-test" };
}

function contentReply(statusCode: number, data: string) {
  return () => ({
    statusCode,
    data,
    responseOptions: { headers: { "Content-Type": "application/json" } },
  });
}

function interceptContent(hash: string, statusCode: number, data: string) {
  fetchMock
    .get(SERVICE_BASE)
    .intercept({ path: `/v1/content/${hash}`, method: "GET" })
    .reply(contentReply(statusCode, data));
}

function envWithSkips(skips: SkipLog[]): ClassAContentEnv {
  return { service: SERVICE, tenantId: "tenant-class-a", onSkip: (log) => skips.push(log) };
}

beforeAll(async () => {
  HASH = await sha256Hex(new TextEncoder().encode(TEXT));
  fetchMock.activate();
  fetchMock.disableNetConnect();
  const inner = globalThis.fetch;
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const req = new Request(input as RequestInfo, init);
    seen.push({
      method: req.method,
      url: req.url,
      authorization: req.headers.get("Authorization"),
      accessId: req.headers.get("CF-Access-Client-Id"),
      accessSecret: req.headers.get("CF-Access-Client-Secret"),
    });
    return inner(input, init);
  }) as typeof fetch;
});

afterEach(() => {
  seen.length = 0;
  fetchMock.assertNoPendingInterceptors();
});

describe("Class A caller via the content-by-hash seam (#13105 slice 3)", () => {
  it("a valid Class A hash hydrates the service payload and records skip evidence", async () => {
    const body = hydrationBody(TEXT);
    interceptContent(HASH, 200, JSON.stringify(body));
    const skips: SkipLog[] = [];
    const outcome = await fetchClassAContent(envWithSkips(skips), HASH);
    // The oracle: the served payload is the service hydration
    // payload, byte for byte.
    expect(outcome).toEqual({
      served: true,
      payload: body,
      log: {
        tenant: "tenant-class-a",
        key: HASH,
        dataClass: "A",
        entryWatermark: HASH,
        entryAge: 0,
        reason: "content-hash-match",
        degraded: false,
      },
    });
    expect(skips).toHaveLength(1);
    expect(skips[0].reason).toBe("content-hash-match");
    // Auth reuse: the hash alone is not authorization — the
    // existing service headers ride the GET.
    expect(seen).toHaveLength(1);
    expect(seen[0].method).toBe("GET");
    expect(seen[0].url).toBe(`${SERVICE_BASE}/v1/content/${HASH}`);
    expect(seen[0].authorization).toBe(`Bearer ${SERVICE.SERVICE_TOKEN}`);
    expect(seen[0].accessId).toBe(SERVICE.ACCESS_CLIENT_ID);
    expect(seen[0].accessSecret).toBe(SERVICE.ACCESS_CLIENT_SECRET);
  });

  it("absent content falls back without exposing bytes or a skip", async () => {
    const missing = "0".repeat(64);
    interceptContent(
      missing,
      404,
      JSON.stringify({ error: { code: "chunk_not_found", message: "content not found" } }),
    );
    const skips: SkipLog[] = [];
    const outcome = await fetchClassAContent(envWithSkips(skips), missing);
    expect(outcome).toEqual({ served: false, reason: "content-miss" });
    expect(skips).toEqual([]);
  });

  it("undisclosed content is indistinguishable from absent", async () => {
    const withheld = "f".repeat(64);
    // The server answers withheld content with the same 404 shape
    // as missing content; the caller must not distinguish them.
    interceptContent(
      withheld,
      404,
      JSON.stringify({ error: { code: "chunk_not_found", message: "content not found" } }),
    );
    const skips: SkipLog[] = [];
    const outcome = await fetchClassAContent(envWithSkips(skips), withheld);
    expect(outcome).toEqual({ served: false, reason: "content-miss" });
    expect(skips).toEqual([]);
    expect(JSON.stringify(outcome)).not.toContain("canary");
  });

  it("a malformed hash never reaches the service", async () => {
    const skips: SkipLog[] = [];
    const outcome = await fetchClassAContent(envWithSkips(skips), "not-a-sha256");
    expect(outcome).toEqual({ served: false, reason: "malformed-hash" });
    expect(skips).toEqual([]);
    expect(seen).toEqual([]);
  });

  it("a seam 500 degrades to the fallback with no error-surface change", async () => {
    interceptContent(HASH, 500, JSON.stringify({ error: { code: "internal", message: "boom" } }));
    const skips: SkipLog[] = [];
    const outcome = await fetchClassAContent(envWithSkips(skips), HASH);
    expect(outcome).toEqual({ served: false, reason: "seam-unavailable" });
    expect(skips).toEqual([]);
  });

  it("a service-side auth failure is unavailable, never a miss", async () => {
    interceptContent(HASH, 401, JSON.stringify({ error: { code: "unauthorized", message: "nope" } }));
    const skips: SkipLog[] = [];
    const outcome = await fetchClassAContent(envWithSkips(skips), HASH);
    expect(outcome).toEqual({ served: false, reason: "seam-unavailable" });
    expect(skips).toEqual([]);
  });

  it("non-JSON and non-object answers are unavailable, never served", async () => {
    interceptContent(HASH, 200, "not json{{");
    await expect(fetchClassAContent(envWithSkips([]), HASH)).resolves.toEqual({
      served: false,
      reason: "seam-unavailable",
    });

    interceptContent(HASH, 200, "null");
    await expect(fetchClassAContent(envWithSkips([]), HASH)).resolves.toEqual({
      served: false,
      reason: "seam-unavailable",
    });
  });

  it("bytes that do not hash back to the named hash are never served", async () => {
    const other = await sha256Hex(new TextEncoder().encode("some other text"));
    expect(other).not.toBe(HASH);
    interceptContent(HASH, 200, JSON.stringify(hydrationBody("some other text")));
    const skips: SkipLog[] = [];
    const outcome = await fetchClassAContent(envWithSkips(skips), HASH);
    expect(outcome).toEqual({ served: false, reason: "seam-unavailable" });
    expect(skips).toEqual([]);
    expect(JSON.stringify(outcome)).not.toContain("some other text");
  });

  it("a hydration payload with no text is unavailable", async () => {
    interceptContent(HASH, 200, JSON.stringify({ chunk_id: 7, score: 0.9 }));
    const skips: SkipLog[] = [];
    const outcome = await fetchClassAContent(envWithSkips(skips), HASH);
    expect(outcome).toEqual({ served: false, reason: "seam-unavailable" });
    expect(skips).toEqual([]);
  });
});
