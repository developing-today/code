// Oracle: real Edge owner verification + fixed Hrana fixtures. Only SELECTs
// may leave the page; unverified callers must never reach the data plane.
import { beforeEach, expect, test } from "bun:test";
import { Edge } from "../edge/src/edge.ts";
import { clearJwksCache } from "../src/auth.ts";
import { readConfig } from "../src/config.ts";
import { signer } from "./support.ts";

const NOW = Date.parse("2026-11-25T12:00:00Z");
const issuer = { name: "access", issuer: "https://team.cloudflareaccess.com", jwksUri: "https://team.cloudflareaccess.com/certs", audience: "comments", source: "access" as const };
beforeEach(() => clearJwksCache());

async function world(enabled = true, rows: Record<string, unknown>[] = []) {
  const key = await signer("comments");
  const cfg = { ...readConfig({ HOME: "/h", PM_CONSOLE_OWNER_EMAILS: "owner@example.com" }), issuers: [issuer], androidApp: true, androidFingerprints: [] };
  const stored = new Map<string, unknown>([["cfg", cfg]]);
  const calls: { sql: string; headers: Headers; signal: AbortSignal | null | undefined }[] = [];
  const fault = { on: false };
  const options = {
    kv: { get: async (k: string) => stored.get(k), put: async (k: string, v: unknown) => { stored.set(k, v); }, delete: async (k: string) => { stored.delete(k); } },
    now: () => NOW,
    fetchImpl: (async () => new Response(JSON.stringify({ keys: [key.jwk] }))) as typeof fetch,
    comments: {
      enabled, token: "read-only-test-token",
      fetchImpl: (async (_url: unknown, init?: RequestInit) => {
        const body = JSON.parse(String(init?.body));
        calls.push({ sql: body.requests[0].stmt.sql, headers: new Headers(init?.headers), signal: init?.signal });
        if (fault.on) return new Response("upstream sensitive details", { status: 503 });
        const cols = Object.keys(rows[0] ?? { id: "", object_ref: "", kind: "", author: "", body: "", created_at: "", triage_state: "", github_state: "" });
        return Response.json({ results: [{ type: "ok", response: { type: "execute", result: { cols: cols.map(name => ({ name })), rows: rows.map(row => cols.map(name => row[name] === null ? { type: "null" } : { type: "text", value: String(row[name]) })) } } }] });
      }) as typeof fetch,
    },
  };
  const edge = new Edge(options);
  const token = (email = "owner@example.com", exp = NOW / 1000 + 60) => key.sign({ iss: issuer.issuer, aud: issuer.audience, exp, email });
  const request = (headers: HeadersInit = {}, method = "GET") => edge.fetch(new Request("https://pm.example/comments", { method, headers }));
  return { calls, fault, token, request };
}

test("hosted comments: verified owner sees actual rows, read time and honest watermark", async () => {
  const w = await world(true, [{ id: "c1", object_ref: "issue:42", kind: "note", author: JSON.stringify({ label: "Owner" }), body: "<script>bad</script>", created_at: "2026-11-20T00:00:00Z", triage_state: "new", github_state: null }]);
  const res = await w.request({ "cf-access-jwt-assertion": await w.token(), cookie: "private=browser" });
  expect(res.status).toBe(200);
  expect(res.headers.get("x-pm-next")).toBeNull();
  expect(res.headers.get("cache-control")).toContain("no-store");
  const html = await res.text();
  expect(html).toContain("issue:42");
  expect(html).toContain("&lt;script&gt;bad&lt;/script&gt;");
  expect(html).toContain("2026-11-25T12:00:00.000Z");
  expect(html).toContain("2026-11-20T00:00:00Z");
  expect(html).toContain("Producer freshness unknown");
  expect(w.calls).toHaveLength(1);
  expect(w.calls[0].sql).toMatch(/^select .* from shared_comments .*limit 101$/i);
  expect(w.calls[0].headers.get("authorization")).toBe("Bearer read-only-test-token");
  expect(w.calls[0].headers.get("cookie")).toBeNull();
  expect(w.calls[0].headers.get("cf-access-jwt-assertion")).toBeNull();
  expect(w.calls[0].signal).toBeDefined();
});

test("hosted comments: forged transports, nonowner, expired JWT and missing sign-in never query", async () => {
  const w = await world();
  for (const headers of [{}, { "cf-access-authenticated-user-email": "owner@example.com", "x-forwarded-for": "127.0.0.1" }, { "cf-access-jwt-assertion": "forged" }, { "cf-access-jwt-assertion": await w.token("stranger@example.com") }, { "cf-access-jwt-assertion": await w.token("owner@example.com", NOW / 1000 - 3600) }]) {
    const res = await w.request(headers);
    expect(res.status >= 400 || res.headers.get("x-pm-next") === "relay").toBe(true);
  }
  expect(w.calls).toHaveLength(0);
});

test("hosted comments: disabled preserves route; enabled writes return 405 without SQL", async () => {
  const off = await world(false);
  expect((await off.request({ "cf-access-jwt-assertion": await off.token() })).headers.get("x-pm-next")).toBe("relay");
  expect(off.calls).toHaveLength(0);
  // Each world creates a different signing key at the same JWKS URI.
  clearJwksCache();
  const on = await world();
  const write = await on.request({ "cf-access-jwt-assertion": await on.token() }, "POST");
  expect(write.status).toBe(405);
  expect(on.calls).toHaveLength(0);
});

test("hosted comments: empty is explicit; outage fails visibly, never an empty success", async () => {
  const w = await world();
  const headers = { "cf-access-jwt-assertion": await w.token() };
  const empty = await w.request(headers);
  expect(empty.status).toBe(200);
  expect(await empty.text()).toContain("No comments returned");
  w.fault.on = true;
  const failed = await w.request(headers);
  expect(failed.status).toBe(503);
  const html = await failed.text();
  expect(html).toContain("Comments unavailable");
  expect(html).not.toContain("upstream sensitive details");
  expect(html).not.toContain("No comments returned");
});
