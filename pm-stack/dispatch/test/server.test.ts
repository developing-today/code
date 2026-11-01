// HTTP-level tests for POST /v1/sessions directory scoping (issue #2664).
// Drives the exported handler with a stubbed-JWKS OIDC token: no network,
// queue, or GitHub — valid requests reach the unconfigured queue (502), while
// rejected directories must return 400 first.
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

const base = { repo: "developing-today/code", prompt: "do the thing", issue: false };

describe("POST /v1/sessions directory scoping (issue #2664)", () => {
  test("relative, foreign, and outside paths are rejected with 400 before any side effect", async () => {
    for (const directory of ["src", "/home/user/git/opencode", "/home/user/git/code-other", "/tmp/evil"]) {
      const res = await handler(sessionsRequest(signOidc("developing-today/code"), { ...base, directory }));
      expect(res.status).toBe(400);
      const body = (await res.json()) as { error: string };
      expect(body.error).toMatch(/directory/);
    }
  });

  test("blank directory defaults and in-repo subdirectory passes validation", async () => {
    for (const directory of [undefined, "", " \n ", "/home/user/git/code/src"]) {
      const body = { ...base, ...(directory === undefined ? {} : { directory }) };
      const res = await handler(sessionsRequest(signOidc("developing-today/code"), body));
      // These requests pass validation; without a test queue they fail only
      // when trying to enqueue, after the directory gate.
      expect(res.status).toBe(502);
    }
  });
});
