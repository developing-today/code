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
import { beforeAll, describe, expect, test } from "bun:test";
import { resolveAgentFrontDoor, type AgentCredentialMetadata, type AgentFrontDoorDeps, type AgentAudit } from "../src/agent-front-door.ts";
import {
  resolveFrontDoor,
  type FrontDoorDeps,
  type JsonWebKeySet,
} from "../src/front-door.ts";

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

/**
 * Oracle: component H design §§4.2, 4.5, 4.8 (#9086 / #14262).
 * Signed fixture tokens are not authority without live credential metadata.
 * The state model is active → revoked: the very next request must deny.
 */
const AGENT_NOW = 1_800_000_000;
const AGENT_AUDIENCE = "fleet-service";
let agentPrivateKey!: CryptoKey;
let agentJwks!: AgentFrontDoorDeps["jwks"];

beforeAll(async () => {
  const pair = await crypto.subtle.generateKey(
    { name: "RSASSA-PKCS1-v1_5", modulusLength: 2048, publicExponent: new Uint8Array([1, 0, 1]), hash: "SHA-256" },
    true, ["sign", "verify"],
  );
  agentPrivateKey = pair.privateKey;
  agentJwks = { keys: [{ ...await crypto.subtle.exportKey("jwk", pair.publicKey), kid: "agent-fixture" }] };
});

async function mintAgentToken(overrides: Record<string, unknown> = {}) {
  const encode = (value: unknown) => Buffer.from(JSON.stringify(value)).toString("base64url");
  const head = encode({ alg: "RS256", kid: "agent-fixture" });
  const body = encode({
    iss: "https://provider.example", aud: AGENT_AUDIENCE, sub: "agent:credential-1",
    jti: "credential-1", binding_id: "binding-1", scope: "binding-1", roles: ["agent"],
    iat: AGENT_NOW, exp: AGENT_NOW + 900, ...overrides,
  });
  const bytes = new TextEncoder().encode(`${head}.${body}`);
  const signature = await crypto.subtle.sign("RSASSA-PKCS1-v1_5", agentPrivateKey, bytes);
  return `${head}.${body}.${Buffer.from(signature).toString("base64url")}`;
}

function agentFixture() {
  let metadata: AgentCredentialMetadata | null = {
    jti: "credential-1", accountId: "account-1", bindingId: "binding-1", tenantId: "binding-1",
    audience: AGENT_AUDIENCE, role: "agent", issuedAt: AGENT_NOW - 1, expiresAt: AGENT_NOW + 86400, revokedAt: null,
  };
  const audits: AgentAudit[] = [];
  let reads = 0;
  const deps: AgentFrontDoorDeps = {
    issuer: "https://provider.example", audience: AGENT_AUDIENCE, jwks: agentJwks, now: () => AGENT_NOW,
    resolveAccountId: async () => "account-1",
    loadRoles: async () => ["owner", "agent"],
    loadCredential: async () => { reads++; return metadata; },
    audit: async (actor) => { audits.push(actor); },
  };
  return { deps, audits, reads: () => reads, set: (value: AgentCredentialMetadata | null) => { metadata = value; }, metadata: () => metadata! };
}

describe("surface C agent front door (H4 slice)", () => {
  test("agent role never inherits its owner's roles, and audit includes jti plus owner", async () => {
    const f = agentFixture();
    const result = await resolveAgentFrontDoor(await mintAgentToken(), "binding-1", f.deps);
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.principal.roles).toEqual(["agent"]);
    expect(f.audits).toEqual([{ jti: "credential-1", accountId: "account-1", bindingId: "binding-1", tenantId: "binding-1" }]);
  });

  test("model active → revoked denies the next request without caching metadata", async () => {
    const f = agentFixture();
    const token = await mintAgentToken();
    expect((await resolveAgentFrontDoor(token, "binding-1", f.deps)).ok).toBe(true);
    f.set({ ...f.metadata(), revokedAt: AGENT_NOW });
    expect((await resolveAgentFrontDoor(token, "binding-1", f.deps)).ok).toBe(false);
    expect(f.reads()).toBe(2);
    expect(f.audits.length).toBe(1);
  });

  test("property: signed scope/binding/role/audience/lifetime mutations cannot widen authority", async () => {
    const mutations = [
      { scope: "*" }, { scope: ["binding-1", "binding-2"] }, { scope: "binding-2" },
      { binding_id: "binding-2" }, { roles: ["owner"] }, { roles: ["agent", "owner"] },
      { aud: "pm-service" }, { aud: [AGENT_AUDIENCE, "pm-service"] },
      { exp: AGENT_NOW + 901 }, { iat: AGENT_NOW + 1 }, { iat: null }, { iat: AGENT_NOW - 901 },
    ];
    for (const mutation of mutations) {
      const f = agentFixture();
      expect((await resolveAgentFrontDoor(await mintAgentToken(mutation), "binding-1", f.deps)).ok).toBe(false);
      expect(f.audits).toEqual([]);
    }
  });

  test("lifetime boundaries are inclusive at 15 minutes and 30 days, never one second over", async () => {
    const exactToken = await mintAgentToken({ exp: AGENT_NOW + 900 });
    const exact = agentFixture();
    exact.set({ ...exact.metadata(), issuedAt: AGENT_NOW - (30 * 86400 - 900), expiresAt: AGENT_NOW + 900 });
    expect((await resolveAgentFrontDoor(exactToken, "binding-1", exact.deps)).ok).toBe(true);

    const tooLongToken = await mintAgentToken({ exp: AGENT_NOW + 901 });
    expect((await resolveAgentFrontDoor(tooLongToken, "binding-1", agentFixture().deps)).ok).toBe(false);

    const tooLongMetadata = agentFixture();
    tooLongMetadata.set({ ...tooLongMetadata.metadata(), issuedAt: AGENT_NOW - 30 * 86400, expiresAt: AGENT_NOW + 1 });
    expect((await resolveAgentFrontDoor(await mintAgentToken({ exp: AGENT_NOW + 1 }), "binding-1", tooLongMetadata.deps)).ok).toBe(false);
  });

  test("forged tokens and a non-agent live role do not read credential metadata or audit", async () => {
    const fixture = agentFixture();
    const signed = await mintAgentToken();
    const parts = signed.split(".");
    const forged = `${parts[0]}.${parts[1]}.${Buffer.from("not-a-signature").toString("base64url")}`;
    expect((await resolveAgentFrontDoor(forged, "binding-1", fixture.deps)).ok).toBe(false);
    expect(fixture.reads()).toBe(0);
    expect(fixture.audits).toEqual([]);

    const ownerOnly = agentFixture();
    expect((await resolveAgentFrontDoor(signed, "binding-1", { ...ownerOnly.deps, loadRoles: async () => ["owner"] })).ok).toBe(false);
    expect(ownerOnly.reads()).toBe(0);
    expect(ownerOnly.audits).toEqual([]);
  });

  test("live credential metadata must match every owner/scope/lifetime constraint", async () => {
    const token = await mintAgentToken();
    const mutations: Partial<AgentCredentialMetadata>[] = [
      { jti: "other" }, { bindingId: "other" }, { tenantId: "*" }, { audience: "other" },
      { accountId: "other" }, { expiresAt: AGENT_NOW }, { issuedAt: AGENT_NOW + 1 },
      { issuedAt: AGENT_NOW - 30 * 86400, expiresAt: AGENT_NOW + 1 }, { expiresAt: NaN },
    ];
    for (const mutation of mutations) {
      const f = agentFixture();
      f.set({ ...f.metadata(), ...mutation });
      expect((await resolveAgentFrontDoor(token, "binding-1", f.deps)).ok).toBe(false);
      expect(f.audits).toEqual([]);
    }
  });

  test("missing metadata, foreign request tenant, lost live roles and audit failure deny", async () => {
    const token = await mintAgentToken();
    const f = agentFixture();
    f.set(null);
    expect((await resolveAgentFrontDoor(token, "binding-1", f.deps)).ok).toBe(false);
    expect((await resolveAgentFrontDoor(token, "binding-2", agentFixture().deps)).ok).toBe(false);
    expect((await resolveAgentFrontDoor(token, "*", agentFixture().deps)).ok).toBe(false);
    expect((await resolveAgentFrontDoor(token, "binding-1", { ...agentFixture().deps, loadRoles: async () => [] })).ok).toBe(false);
    expect((await resolveAgentFrontDoor(token, "binding-1", { ...agentFixture().deps, audit: async () => { throw new Error("offline"); } })).ok).toBe(false);
  });
});
