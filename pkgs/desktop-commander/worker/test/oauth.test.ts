import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import worker, { type Env } from "../src/index";
import { base64urlEncode, sha256Hex } from "../src/oauth";
import { MemoryKV } from "./memory-kv";

const TEAM_DOMAIN = "team.example.com";
const AUD = "authorize-app-aud";
const OWNER = "drewrypope@gmail.com";
const REDIRECT_URI = "https://client.example.com/callback";

const kv = new MemoryKV();
const env: Env = {
  BRIDGE_BASE: "https://commander-link.security.cab",
  ACCESS_CLIENT_ID: "bridge-id.access",
  ACCESS_CLIENT_SECRET: "bridge-secret",
  MACHINE_BEARER: "machine-bearer-for-tests",
  ACCESS_AUD: AUD,
  ACCESS_TEAM_DOMAIN: TEAM_DOMAIN,
  OAUTH_KV: kv as unknown as KVNamespace,
};

// A real RSA keypair stands in for the Access team's signing key: the
// JWKS endpoint is stubbed to serve its public half, and assertions
// are signed with the private half, so the Worker's RS256 check runs
// for real.
let privateKey: CryptoKey;
let jwks: { keys: Record<string, unknown>[] };

beforeAll(async () => {
  const pair = (await crypto.subtle.generateKey(
    {
      name: "RSASSA-PKCS1-v1_5",
      modulusLength: 2048,
      publicExponent: new Uint8Array([1, 0, 1]),
      hash: "SHA-256",
    },
    true,
    ["sign", "verify"],
  )) as CryptoKeyPair;
  privateKey = pair.privateKey;
  const publicJwk = (await crypto.subtle.exportKey("jwk", pair.publicKey)) as unknown as Record<string, unknown>;
  jwks = { keys: [{ ...publicJwk, kid: "test-key-1", alg: "RS256", use: "sig" }] };
});

async function accessJwt(claims: Record<string, unknown>): Promise<string> {
  const header = { alg: "RS256", typ: "JWT", kid: "test-key-1" };
  const payload = {
    email: OWNER,
    aud: AUD,
    iss: `https://${TEAM_DOMAIN}`,
    exp: Math.floor(Date.now() / 1000) + 600,
    ...claims,
  };
  const encode = (obj: unknown) =>
    base64urlEncode(new TextEncoder().encode(JSON.stringify(obj)));
  const signingInput = `${encode(header)}.${encode(payload)}`;
  const signature = await crypto.subtle.sign(
    "RSASSA-PKCS1-v1_5",
    privateKey,
    new TextEncoder().encode(signingInput),
  );
  return `${signingInput}.${base64urlEncode(new Uint8Array(signature))}`;
}

function stubFetch() {
  const fetchStub = vi.fn(async (input: unknown) => {
    const url = String(input instanceof URL ? input.href : input);
    if (url.startsWith(`https://${TEAM_DOMAIN}/`)) {
      return new Response(JSON.stringify(jwks), {
        headers: { "content-type": "application/json" },
      });
    }
    if (url.startsWith("https://commander-link.security.cab/")) {
      return new Response("event: message\ndata: {}\n\n", {
        status: 200,
        headers: { "content-type": "text/event-stream", "mcp-session-id": "s-1" },
      });
    }
    throw new Error(`unexpected fetch: ${url}`);
  });
  vi.stubGlobal("fetch", fetchStub);
  return fetchStub;
}

afterEach(() => {
  vi.unstubAllGlobals();
});

async function registerTestClient(): Promise<string> {
  const res = await worker.fetch(
    new Request("https://commander.security.cab/register", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        client_name: "Test Client",
        redirect_uris: [REDIRECT_URI],
        grant_types: ["authorization_code", "refresh_token"],
        response_types: ["code"],
        token_endpoint_auth_method: "none",
      }),
    }),
    env,
  );
  expect(res.status).toBe(201);
  const body = (await res.json()) as { client_id: string };
  return body.client_id;
}

async function pkcePair(): Promise<{ verifier: string; challenge: string }> {
  const verifier = base64urlEncode(crypto.getRandomValues(new Uint8Array(32)));
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(verifier));
  return { verifier, challenge: base64urlEncode(new Uint8Array(digest)) };
}

function authorizeUrl(clientId: string, challenge: string, extra = ""): string {
  return (
    `https://commander.security.cab/authorize?response_type=code&client_id=${clientId}` +
    `&redirect_uri=${encodeURIComponent(REDIRECT_URI)}&code_challenge=${challenge}` +
    `&code_challenge_method=S256&state=state-123${extra}`
  );
}

async function approveAndGetCode(clientId: string, verifier: string): Promise<string> {
  const { challenge } = await pkcePairFor(verifier);
  const jwt = await accessJwt({});
  const page = await worker.fetch(
    new Request(authorizeUrl(clientId, challenge), {
      headers: { "cf-access-jwt-assertion": jwt },
    }),
    env,
  );
  expect(page.status).toBe(200);
  const html = await page.text();
  const nonce = /name="nonce" value="([^"]+)"/.exec(html)?.[1];
  expect(nonce).toBeTruthy();

  const decision = await worker.fetch(
    new Request("https://commander.security.cab/authorize", {
      method: "POST",
      headers: {
        "cf-access-jwt-assertion": jwt,
        "content-type": "application/x-www-form-urlencoded",
      },
      body: new URLSearchParams({ nonce: nonce!, decision: "approve" }).toString(),
    }),
    env,
  );
  expect(decision.status).toBe(302);
  const location = new URL(decision.headers.get("location")!);
  expect(location.searchParams.get("state")).toBe("state-123");
  const code = location.searchParams.get("code");
  expect(code).toBeTruthy();
  return code!;
}

async function pkcePairFor(verifier: string): Promise<{ verifier: string; challenge: string }> {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(verifier));
  return { verifier, challenge: base64urlEncode(new Uint8Array(digest)) };
}

function tokenRequest(fields: Record<string, string>): Request {
  return new Request("https://commander.security.cab/token", {
    method: "POST",
    headers: { "content-type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams(fields).toString(),
  });
}

describe("OAuth discovery", () => {
  it("serves the protected-resource metadata, both path shapes", async () => {
    stubFetch();
    for (const path of ["/.well-known/oauth-protected-resource", "/.well-known/oauth-protected-resource/mcp"]) {
      const res = await worker.fetch(new Request(`https://commander.security.cab${path}`), env);
      expect(res.status).toBe(200);
      const body = (await res.json()) as Record<string, unknown>;
      expect(body.resource).toBe("https://commander.security.cab/mcp");
      expect(body.authorization_servers).toEqual(["https://commander.security.cab"]);
      expect(body.bearer_methods_supported).toEqual(["header"]);
      expect(body.scopes_supported).toEqual(["commander"]);
    }
  });

  it("serves the authorization-server metadata", async () => {
    stubFetch();
    const res = await worker.fetch(
      new Request("https://commander.security.cab/.well-known/oauth-authorization-server"),
      env,
    );
    expect(res.status).toBe(200);
    const body = (await res.json()) as Record<string, unknown>;
    expect(body.issuer).toBe("https://commander.security.cab");
    expect(body.authorization_endpoint).toBe("https://commander.security.cab/authorize");
    expect(body.token_endpoint).toBe("https://commander.security.cab/token");
    expect(body.registration_endpoint).toBe("https://commander.security.cab/register");
    expect(body.code_challenge_methods_supported).toEqual(["S256"]);
    expect(body.grant_types_supported).toEqual(["authorization_code", "refresh_token"]);
    expect(body.token_endpoint_auth_methods_supported).toEqual(["none"]);
  });
});

describe("dynamic client registration", () => {
  it("registers a public client and stores it", async () => {
    stubFetch();
    const clientId = await registerTestClient();
    const stored = await kv.get(`client:${clientId}`, "json");
    expect(stored).toMatchObject({ client_name: "Test Client", redirect_uris: [REDIRECT_URI] });
  });

  it("rejects empty and non-loopback-http redirect_uris", async () => {
    stubFetch();
    for (const redirectUris of [[], ["http://evil.example.com/cb"], ["not a uri"]]) {
      const res = await worker.fetch(
        new Request("https://commander.security.cab/register", {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({ redirect_uris: redirectUris }),
        }),
        env,
      );
      expect(res.status).toBe(400);
    }
  });
});

describe("/authorize", () => {
  it("rejects a request without an Access assertion", async () => {
    stubFetch();
    const clientId = await registerTestClient();
    const { challenge } = await pkcePair();
    const res = await worker.fetch(new Request(authorizeUrl(clientId, challenge)), env);
    expect(res.status).toBe(403);
  });

  it("rejects an assertion for a different email", async () => {
    stubFetch();
    const clientId = await registerTestClient();
    const { challenge } = await pkcePair();
    const jwt = await accessJwt({ email: "someone-else@example.com" });
    const res = await worker.fetch(
      new Request(authorizeUrl(clientId, challenge), {
        headers: { "cf-access-jwt-assertion": jwt },
      }),
      env,
    );
    expect(res.status).toBe(403);
  });

  it("rejects a plain PKCE challenge", async () => {
    stubFetch();
    const clientId = await registerTestClient();
    const jwt = await accessJwt({});
    const plain = await worker.fetch(
      new Request(
        `https://commander.security.cab/authorize?response_type=code&client_id=${clientId}` +
          `&redirect_uri=${encodeURIComponent(REDIRECT_URI)}&code_challenge=abc&code_challenge_method=plain`,
        { headers: { "cf-access-jwt-assertion": jwt } },
      ),
      env,
    );
    expect(plain.status).toBe(400);
  });

  it("rejects an unregistered redirect_uri", async () => {
    stubFetch();
    const clientId = await registerTestClient();
    const { challenge } = await pkcePair();
    const jwt = await accessJwt({});
    const res = await worker.fetch(
      new Request(
        `https://commander.security.cab/authorize?response_type=code&client_id=${clientId}` +
          `&redirect_uri=${encodeURIComponent("https://other.example.com/cb")}&code_challenge=${challenge}&code_challenge_method=S256`,
        { headers: { "cf-access-jwt-assertion": jwt } },
      ),
      env,
    );
    expect(res.status).toBe(400);
  });
});

describe("full OAuth flow", () => {
  it("register → approve → exchange → /mcp, with single-use code and refresh rotation", async () => {
    const fetchStub = stubFetch();
    const clientId = await registerTestClient();
    const { verifier } = await pkcePair();
    const code = await approveAndGetCode(clientId, verifier);

    // Exchange the code for tokens.
    const tokenRes = await worker.fetch(
      tokenRequest({
        grant_type: "authorization_code",
        code,
        client_id: clientId,
        redirect_uri: REDIRECT_URI,
        code_verifier: verifier,
      }),
      env,
    );
    expect(tokenRes.status).toBe(200);
    const tokens = (await tokenRes.json()) as {
      access_token: string;
      refresh_token: string;
      token_type: string;
      expires_in: number;
    };
    expect(tokens.token_type).toBe("Bearer");
    expect(tokens.expires_in).toBe(3600);
    // Tokens are stored hashed, never in the clear.
    expect(kv.keys()).toContain(`at:${await sha256Hex(tokens.access_token)}`);
    expect(kv.keys()).not.toContain(`at:${tokens.access_token}`);

    // The access token opens /mcp.
    const mcpRes = await worker.fetch(
      new Request("https://commander.security.cab/mcp", {
        method: "POST",
        headers: {
          authorization: `Bearer ${tokens.access_token}`,
          "content-type": "application/json",
        },
        body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/list" }),
      }),
      env,
    );
    expect(mcpRes.status).toBe(200);
    expect(
      fetchStub.mock.calls.some(([input]) =>
        String(input instanceof URL ? input.href : input).startsWith("https://commander-link.security.cab/"),
      ),
    ).toBe(true);

    // The code cannot be exchanged twice.
    const reuse = await worker.fetch(
      tokenRequest({
        grant_type: "authorization_code",
        code,
        client_id: clientId,
        redirect_uri: REDIRECT_URI,
        code_verifier: verifier,
      }),
      env,
    );
    expect(reuse.status).toBe(400);
    expect(((await reuse.json()) as { error: string }).error).toBe("invalid_grant");

    // Refresh rotates: new pair out, old refresh token dead.
    const refreshed = await worker.fetch(
      tokenRequest({
        grant_type: "refresh_token",
        refresh_token: tokens.refresh_token,
        client_id: clientId,
      }),
      env,
    );
    expect(refreshed.status).toBe(200);
    const rotated = (await refreshed.json()) as { refresh_token: string };
    expect(rotated.refresh_token).not.toBe(tokens.refresh_token);
    const oldRefresh = await worker.fetch(
      tokenRequest({
        grant_type: "refresh_token",
        refresh_token: tokens.refresh_token,
        client_id: clientId,
      }),
      env,
    );
    expect(oldRefresh.status).toBe(400);
  });

  it("rejects a wrong PKCE verifier and burns the code", async () => {
    stubFetch();
    const clientId = await registerTestClient();
    const { verifier } = await pkcePair();
    const code = await approveAndGetCode(clientId, verifier);

    const wrong = await worker.fetch(
      tokenRequest({
        grant_type: "authorization_code",
        code,
        client_id: clientId,
        redirect_uri: REDIRECT_URI,
        code_verifier: "the-wrong-verifier",
      }),
      env,
    );
    expect(wrong.status).toBe(400);
    expect(((await wrong.json()) as { error: string }).error).toBe("invalid_grant");

    const retry = await worker.fetch(
      tokenRequest({
        grant_type: "authorization_code",
        code,
        client_id: clientId,
        redirect_uri: REDIRECT_URI,
        code_verifier: verifier,
      }),
      env,
    );
    expect(retry.status).toBe(400);
  });

  it("deny redirects with access_denied and no code", async () => {
    stubFetch();
    const clientId = await registerTestClient();
    const { challenge } = await pkcePair();
    const jwt = await accessJwt({});
    const page = await worker.fetch(
      new Request(authorizeUrl(clientId, challenge), {
        headers: { "cf-access-jwt-assertion": jwt },
      }),
      env,
    );
    const nonce = /name="nonce" value="([^"]+)"/.exec(await page.text())?.[1];
    const decision = await worker.fetch(
      new Request("https://commander.security.cab/authorize", {
        method: "POST",
        headers: {
          "cf-access-jwt-assertion": jwt,
          "content-type": "application/x-www-form-urlencoded",
        },
        body: new URLSearchParams({ nonce: nonce!, decision: "deny" }).toString(),
      }),
      env,
    );
    expect(decision.status).toBe(302);
    const location = new URL(decision.headers.get("location")!);
    expect(location.searchParams.get("error")).toBe("access_denied");
    expect(location.searchParams.get("code")).toBeNull();
    expect(location.searchParams.get("state")).toBe("state-123");
  });
});
