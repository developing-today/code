// HTTP-level tests for POST /v1/sessions directory scoping (issue #2903).
// Drives the exported handler with a stubbed-JWKS OIDC token: no network,
// no queue, no GitHub — any non-JWKS fetch throws, so a test that tried
// to create an issue/task/session would fail loudly instead of 400ing.
import { describe, expect, test } from "bun:test";
import { createSign, generateKeyPairSync } from "node:crypto";

const { publicKey, privateKey } = generateKeyPairSync("rsa", { modulusLength: 2048 });
const jwk = publicKey.export({ format: "jwk" }) as unknown as Record<string, unknown>;
jwk.kid = "test-kid-1";
jwk.alg = "RS256";

const JWKS_URL = "https://token.actions.githubusercontent.com/.well-known/jwks.json";
const realFetch = globalThis.fetch;
globalThis.fetch = (async (input: unknown) => {
  if (String(input) === JWKS_URL) {
    return new Response(JSON.stringify({ keys: [jwk] }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  }
  throw new Error(`unexpected network fetch in test: ${String(input)}`);
}) as typeof fetch;

function b64url(data: Buffer | string): string {
  return Buffer.from(data).toString("base64").replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

function signOidc(repository: string): string {
  const now = Math.floor(Date.now() / 1000);
  const header = b64url(JSON.stringify({ alg: "RS256", typ: "JWT", kid: "test-kid-1" }));
  const payload = b64url(
    JSON.stringify({
      iss: "https://token.actions.githubusercontent.com",
      aud: "pm-dispatch",
      exp: now + 600,
      iat: now,
      nbf: now - 60,
      repository,
      actor: "test-actor",
      sub: `repo:${repository}:ref:refs/heads/main`,
    }),
  );
  const signer = createSign("RSA-SHA256");
  signer.update(`${header}.${payload}`);
  return `${header}.${payload}.${b64url(signer.sign(privateKey))}`;
}

const { handler } = await import("../src/server.ts");

function sessionsRequest(token: string, body: Record<string, unknown>): Request {
  return new Request("http://x/v1/sessions", {
    method: "POST",
    headers: { authorization: `Bearer ${token}`, "content-type": "application/json" },
    body: JSON.stringify(body),
  });
}

const base = { repo: "developing-today/code", prompt: "do the thing" };

describe("POST /v1/sessions directory scoping (issue #2903)", () => {
  test("OIDC + foreign directory is rejected with 400 before any side effect", async () => {
    const token = signOidc("developing-today/code");
    const res = await handler(sessionsRequest(token, { ...base, directory: "/tmp/evil" }));
    expect(res.status).toBe(400);
    const body = (await res.json()) as { error: string };
    expect(body.error).toMatch(/directory/);
  });

  test("OIDC + omitted directory passes validation (fails later at the unconfigured queue)", async () => {
    // No SUPABASE_SERVICE_ROLE_KEY in test env, so a request that clears
    // validation reaches requireQueue() and maps to 502 — proving the
    // directory gate let it through. Any network beyond JWKS throws.
    const token = signOidc("developing-today/code");
    const res = await handler(sessionsRequest(token, base));
    expect(res.status).toBe(502);
  });

  test("OIDC + own clone directory passes validation the same way", async () => {
    const token = signOidc("developing-today/code");
    const res = await handler(sessionsRequest(token, { ...base, directory: "/home/user/git/code" }));
    expect(res.status).toBe(502);
  });
});
