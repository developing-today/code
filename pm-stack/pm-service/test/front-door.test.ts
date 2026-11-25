/**
 * SSO relying-party front door, slice H1 (#14259, child of #9086).
 *
 * Oracle: the merged component H design
 * (`doc/2026-11-24T04-50-00Z_design_sso-relying-party-9086/`, §4.1/§5
 * slice H1). The suite fails if a forged, expired, or foreign-audience
 * token is ever accepted, if a fixture provider token does not resolve
 * to (account_id, binding_id, roles), or if resolution is slower than
 * the design's <50ms p99 outline.
 */
import { describe, expect, test } from "bun:test";
import { resolveFrontDoor, type FrontDoorDeps, type JsonWebKeySet } from "../src/front-door.ts";

const ISSUER = "https://id.devtoday.example";
const AUDIENCE = "pm-service";
const KID = "fixture-key-1";
const FIXTURE_SUB = "github|123";
const FIXTURE_ACCOUNT = "acct-1";
const FIXTURE_BINDING = "bind-1";
const NOW_SEC = Math.floor(new Date("2026-11-24T12:00:00Z").getTime() / 1000);

function base64url(bytes: Uint8Array): string {
  return Buffer.from(bytes).toString("base64url");
}

function base64urlJson(value: unknown): string {
  return base64url(new TextEncoder().encode(JSON.stringify(value)));
}

interface Fixture {
  privateKey: CryptoKey;
  jwks: JsonWebKeySet;
}

async function makeFixture(): Promise<Fixture> {
  const pair = await crypto.subtle.generateKey(
    { name: "RSASSA-PKCS1-v1_5", modulusLength: 2048, publicExponent: new Uint8Array([1, 0, 1]), hash: "SHA-256" },
    true,
    ["sign", "verify"],
  );
  const jwk = await crypto.subtle.exportKey("jwk", pair.publicKey);
  return {
    privateKey: pair.privateKey,
    jwks: { keys: [{ ...jwk, kid: KID, alg: "RS256", use: "sig" }] },
  };
}

async function mintToken(
  fixture: Fixture,
  claims: Record<string, unknown>,
  key: CryptoKey = fixture.privateKey,
  header: Record<string, unknown> = { alg: "RS256", typ: "JWT", kid: KID },
): Promise<string> {
  const encodedHeader = base64urlJson(header);
  const encodedPayload = base64urlJson(claims);
  const data = new TextEncoder().encode(`${encodedHeader}.${encodedPayload}`);
  const signature = await crypto.subtle.sign("RSASSA-PKCS1-v1_5", key, data);
  return `${encodedHeader}.${encodedPayload}.${base64url(new Uint8Array(signature))}`;
}

function validClaims(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    iss: ISSUER,
    aud: AUDIENCE,
    sub: FIXTURE_SUB,
    exp: NOW_SEC + 900,
    iat: NOW_SEC,
    jti: "jti-fixture-1",
    binding_id: FIXTURE_BINDING,
    roles: ["owner"],
    ...overrides,
  };
}

function fixtureDeps(fixture: Fixture, overrides: Partial<FrontDoorDeps> = {}): FrontDoorDeps {
  return {
    issuer: ISSUER,
    audience: AUDIENCE,
    jwks: fixture.jwks,
    now: () => NOW_SEC,
    resolveAccountId: async (sub) => (sub === FIXTURE_SUB ? FIXTURE_ACCOUNT : null),
    loadRoles: async (accountId, bindingId) =>
      accountId === FIXTURE_ACCOUNT && bindingId === FIXTURE_BINDING ? ["owner", "agent"] : [],
    ...overrides,
  };
}

describe("front door (H1)", () => {
  test("fixture provider token resolves to (account_id, binding_id, roles)", async () => {
    const fixture = await makeFixture();
    const token = await mintToken(fixture, validClaims());
    const result = await resolveFrontDoor(token, fixtureDeps(fixture));
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.principal.accountId).toBe(FIXTURE_ACCOUNT);
    expect(result.principal.bindingId).toBe(FIXTURE_BINDING);
    expect(result.principal.roles).toEqual(["owner", "agent"]);
    expect(result.principal.providerSub).toBe(FIXTURE_SUB);
  });

  test("forged token (signed with a different key) is rejected", async () => {
    const fixture = await makeFixture();
    const attacker = await makeFixture();
    const token = await mintToken(attacker, validClaims(), attacker.privateKey);
    const result = await resolveFrontDoor(token, fixtureDeps(fixture));
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.reason).toBe("bad_signature");
  });

  test("tampered payload is rejected", async () => {
    const fixture = await makeFixture();
    const token = await mintToken(fixture, validClaims());
    const parts = token.split(".");
    const tamperedPayload = base64urlJson(validClaims({ roles: ["owner", "orchestrator"] }));
    const result = await resolveFrontDoor(`${parts[0]}.${tamperedPayload}.${parts[2]}`, fixtureDeps(fixture));
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.reason).toBe("bad_signature");
  });

  test("expired token is rejected", async () => {
    const fixture = await makeFixture();
    const token = await mintToken(fixture, validClaims({ exp: NOW_SEC - 60 }));
    const result = await resolveFrontDoor(token, fixtureDeps(fixture));
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.reason).toBe("expired");
  });

  test("foreign-audience token is rejected", async () => {
    const fixture = await makeFixture();
    const token = await mintToken(fixture, validClaims({ aud: "some-other-service" }));
    const result = await resolveFrontDoor(token, fixtureDeps(fixture));
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.reason).toBe("bad_audience");
  });

  test("wrong-issuer token is rejected", async () => {
    const fixture = await makeFixture();
    const token = await mintToken(fixture, validClaims({ iss: "https://evil.example" }));
    const result = await resolveFrontDoor(token, fixtureDeps(fixture));
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.reason).toBe("bad_issuer");
  });

  test("token without binding_id is rejected", async () => {
    const fixture = await makeFixture();
    const claims = validClaims();
    delete claims.binding_id;
    const token = await mintToken(fixture, claims);
    const result = await resolveFrontDoor(token, fixtureDeps(fixture));
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.reason).toBe("bad_claim_shape");
  });

  test("token with a role outside #1253 vocabulary is rejected", async () => {
    const fixture = await makeFixture();
    const token = await mintToken(fixture, validClaims({ roles: ["superuser"] }));
    const result = await resolveFrontDoor(token, fixtureDeps(fixture));
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.reason).toBe("bad_claim_shape");
  });

  test("unlinked provider subject is rejected", async () => {
    const fixture = await makeFixture();
    const token = await mintToken(fixture, validClaims({ sub: "github|nobody" }));
    const result = await resolveFrontDoor(token, fixtureDeps(fixture));
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.reason).toBe("no_identity_link");
  });

  test("account with no live roles is rejected (fail-closed)", async () => {
    const fixture = await makeFixture();
    const token = await mintToken(fixture, validClaims());
    const result = await resolveFrontDoor(token, fixtureDeps(fixture, { loadRoles: async () => [] }));
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.reason).toBe("no_live_roles");
  });

  test("resolution stays inside the design's <50ms p99 outline", async () => {
    const fixture = await makeFixture();
    const token = await mintToken(fixture, validClaims());
    const deps = fixtureDeps(fixture);
    const samples: number[] = [];
    for (let i = 0; i < 200; i++) {
      const start = performance.now();
      const result = await resolveFrontDoor(token, deps);
      samples.push(performance.now() - start);
      expect(result.ok).toBe(true);
    }
    samples.sort((a, b) => a - b);
    const p99 = samples[Math.floor(0.99 * (samples.length - 1))] as number;
    console.log(`front-door resolve p99 over 200 samples: ${p99.toFixed(3)}ms`);
    expect(p99).toBeLessThan(50);
  });
});
