/**
 * SSO relying-party H4 — agent credentials + surface C (#14262).
 *
 * Oracle: component H design §§4.2/4.5/4.8. The suite fails if a
 * credential is minted without an owner/orchestrator session, for
 * '*'/foreign/multiple tenants, or past the 30-day max; if the
 * secret is ever stored (only its digest may persist, and the
 * serialized store must not contain the secret); if the
 * client-credentials exchange mints anything but an audience-bound
 * 15-minute agent token; or if revocation does not deny the very
 * next exchange AND the very next front-door request end to end
 * through the surface C HTTP handler.
 */
import { beforeAll, describe, expect, test } from "bun:test";
import { InMemoryAgentCredentialStore, SqlAgentCredentialStore } from "../src/agent-credential-store.ts";
import {
  exchangeClientCredentials,
  issueAgentCredential,
  revokeAgentCredential,
  type AccessTokenClaims,
} from "../src/agent-credentials.ts";
import { createSurfaceCHandler } from "../src/surface-c-api.ts";
import type { AgentAudit } from "../src/agent-front-door.ts";
import type { JsonWebKeySet, ResolvedPrincipal } from "../src/front-door.ts";
import { openTenantDb } from "../src/tenant-store/sqlite.ts";

const NOW = 1_800_000_000;
const AUDIENCE = "fleet-service";
const ACCOUNT = "account-1";
const BINDING = "binding-1";
const ISSUER = "https://provider.example";

const ownerSession: ResolvedPrincipal = {
  accountId: ACCOUNT, bindingId: BINDING, roles: ["owner"], providerSub: "github|123", tokenId: "owner-session-1",
};
const orchestratorSession: ResolvedPrincipal = { ...ownerSession, roles: ["orchestrator"] };
const agentSession: ResolvedPrincipal = { ...ownerSession, roles: ["agent"] };

let counter = 0;
function deps(store: InMemoryAgentCredentialStore, now: () => number = () => NOW) {
  return {
    store,
    now,
    generateJti: () => `credential-${++counter}`,
    generateSecret: () => `fixture-secret-${counter}-not-a-real-secret`,
  };
}

describe("agent credential issuance (H4 §4.5)", () => {
  test("issuance fails closed when no lifecycle auditor is configured", async () => {
    const store = new InMemoryAgentCredentialStore();
    const result = await issueAgentCredential(
      ownerSession,
      { tenantId: BINDING, audience: AUDIENCE },
      { store, now: () => NOW, generateJti: () => "unaudited", generateSecret: () => "fixture-secret" },
    );
    expect(result).toEqual({ ok: false, reason: "issuance_failed" });
    expect(await store.getByJti("unaudited")).toBeNull();
  });

  test("owner session mints a single-tenant agent credential; secret shown once, never stored", async () => {
    const store = new InMemoryAgentCredentialStore();
    const result = await issueAgentCredential(ownerSession, { tenantId: BINDING, audience: AUDIENCE, ttlSec: 3600 }, deps(store));
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.credential.role).toBe("agent");
    expect(result.credential.tenantId).toBe(BINDING);
    expect(result.credential.accountId).toBe(ACCOUNT);
    expect(result.credential.expiresAt - result.credential.issuedAt).toBe(3600);
    expect(result.secret.length).toBeGreaterThan(0);

    const stored = await store.getByJti(result.credential.jti);
    expect(stored).not.toBeNull();
    // Metadata + digest only: the secret value appears nowhere persisted.
    expect(JSON.stringify(stored)).not.toContain(result.secret);
    expect(JSON.stringify([...store.records.values()])).not.toContain(result.secret);
    expect(stored?.secretDigest).toMatch(/^[0-9a-f]{64}$/);
    expect(stored?.secretDigest).not.toBe(result.secret);
  });

  test("orchestrator may mint; agent/read-only sessions may not", async () => {
    const store = new InMemoryAgentCredentialStore();
    expect((await issueAgentCredential(orchestratorSession, { tenantId: BINDING, audience: AUDIENCE }, deps(store))).ok).toBe(true);
    const denied = await issueAgentCredential(agentSession, { tenantId: BINDING, audience: AUDIENCE }, deps(store));
    expect(denied).toEqual({ ok: false, reason: "not_minting_role" });
  });

  test("property: '*', foreign, and empty tenants can never be minted", async () => {
    const store = new InMemoryAgentCredentialStore();
    for (const tenantId of ["*", "binding-2", ""]) {
      const result = await issueAgentCredential(ownerSession, { tenantId, audience: AUDIENCE }, deps(store));
      expect(result.ok).toBe(false);
    }
    expect(store.records.size).toBe(0);
  });

  test("TTL boundaries: 30 days exactly mints, one second over and non-positive deny", async () => {
    const store = new InMemoryAgentCredentialStore();
    expect((await issueAgentCredential(ownerSession, { tenantId: BINDING, audience: AUDIENCE, ttlSec: 30 * 86400 }, deps(store))).ok).toBe(true);
    for (const ttlSec of [30 * 86400 + 1, 0, -1, 1.5]) {
      const result = await issueAgentCredential(ownerSession, { tenantId: BINDING, audience: AUDIENCE, ttlSec }, deps(store));
      expect(result).toEqual({ ok: false, reason: "invalid_ttl" });
    }
  });

  test("SQL central store round-trips metadata + digest and revokes live", async () => {
    const { sql } = openTenantDb();
    const store = new SqlAgentCredentialStore(sql);
    const sqlResult = await issueAgentCredential(ownerSession, { tenantId: BINDING, audience: AUDIENCE, ttlSec: 60 }, { store, now: () => NOW, generateJti: () => "sql-credential-1", generateSecret: () => "sql-fixture-secret" });
    expect(sqlResult.ok).toBe(true);
    const rows = sql.all<Record<string, unknown>>("SELECT * FROM agent_credential");
    expect(JSON.stringify(rows)).not.toContain("sql-fixture-secret");
    const loaded = await store.getByJti("sql-credential-1");
    expect(loaded?.tenantId).toBe(BINDING);
    expect(await store.revoke("sql-credential-1", NOW + 1)).toBe(true);
    expect((await store.getByJti("sql-credential-1"))?.revokedAt).toBe(NOW + 1);
    expect(await store.revoke("sql-credential-1", NOW + 2)).toBe(false);
  });
});

describe("client-credentials exchange (H4 §4.2 surface C)", () => {
  async function issued() {
    const store = new InMemoryAgentCredentialStore();
    const d = deps(store);
    const result = await issueAgentCredential(ownerSession, { tenantId: BINDING, audience: AUDIENCE, ttlSec: 86400 }, d);
    if (!result.ok) throw new Error("issuance failed in fixture");
    const minted: AccessTokenClaims[] = [];
    return {
      store, d, result, minted,
      exchangeDeps: { ...d, mintAccessToken: async (claims: AccessTokenClaims) => { minted.push(claims); return "signed-token"; } },
    };
  }

  test("correct secret exchanges for an audience-bound 15-minute agent token", async () => {
    const f = await issued();
    const result = await exchangeClientCredentials(
      { credentialId: f.result.credential.jti, secret: f.result.secret, audience: AUDIENCE }, f.exchangeDeps);
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.expiresIn).toBe(900);
    expect(result.claims.roles).toEqual(["agent"]);
    expect(result.claims.audience).toBe(AUDIENCE);
    expect(result.claims.tenantId).toBe(BINDING);
    expect(result.claims.bindingId).toBe(BINDING);
    expect(result.claims.expiresAt - result.claims.issuedAt).toBe(900);
    expect(result.claims.jti).toBe(f.result.credential.jti);
  });

  test("wrong secret, wrong audience, and unknown credential all deny as invalid_client", async () => {
    const f = await issued();
    for (const input of [
      { credentialId: f.result.credential.jti, secret: "wrong", audience: AUDIENCE },
      { credentialId: f.result.credential.jti, secret: f.result.secret, audience: "pm-service" },
      { credentialId: "nope", secret: f.result.secret, audience: AUDIENCE },
    ]) {
      expect((await exchangeClientCredentials(input, f.exchangeDeps)).ok).toBe(false);
    }
    expect(f.minted).toEqual([]);
  });

  test("the stored digest itself does not authenticate", async () => {
    const f = await issued();
    const stored = await f.store.getByJti(f.result.credential.jti);
    const result = await exchangeClientCredentials(
      { credentialId: f.result.credential.jti, secret: stored?.secretDigest ?? "", audience: AUDIENCE }, f.exchangeDeps);
    expect(result.ok).toBe(false);
  });

  test("model active → revoked denies the next exchange; expired credentials deny", async () => {
    const f = await issued();
    expect((await exchangeClientCredentials({ credentialId: f.result.credential.jti, secret: f.result.secret, audience: AUDIENCE }, f.exchangeDeps)).ok).toBe(true);
    expect((await revokeAgentCredential(ownerSession, f.result.credential.jti, f.d)).ok).toBe(true);
    expect((await exchangeClientCredentials({ credentialId: f.result.credential.jti, secret: f.result.secret, audience: AUDIENCE }, f.exchangeDeps)).ok).toBe(false);

    const g = await issued();
    const later = { ...g.exchangeDeps, now: () => NOW + 86401 };
    expect((await exchangeClientCredentials({ credentialId: g.result.credential.jti, secret: g.result.secret, audience: AUDIENCE }, later)).ok).toBe(false);
  });

  test("revocation is owner-scoped: another account cannot revoke", async () => {
    const f = await issued();
    const stranger: ResolvedPrincipal = { ...ownerSession, accountId: "account-2", bindingId: "binding-2" };
    expect((await revokeAgentCredential(stranger, f.result.credential.jti, f.d)).ok).toBe(false);
    expect((await f.store.getByJti(f.result.credential.jti))?.revokedAt).toBeNull();
  });
});

// --- End-to-end through the surface C HTTP handler ---

let privateKey: CryptoKey;
let jwks: JsonWebKeySet;
beforeAll(async () => {
  const pair = await crypto.subtle.generateKey(
    { name: "RSASSA-PKCS1-v1_5", modulusLength: 2048, publicExponent: new Uint8Array([1, 0, 1]), hash: "SHA-256" },
    true, ["sign", "verify"],
  );
  privateKey = pair.privateKey;
  jwks = { keys: [{ ...await crypto.subtle.exportKey("jwk", pair.publicKey), kid: "surface-c-fixture" }] };
});

async function signClaims(claims: AccessTokenClaims): Promise<string> {
  const encode = (value: unknown) => Buffer.from(JSON.stringify(value)).toString("base64url");
  const head = encode({ alg: "RS256", kid: "surface-c-fixture" });
  const body = encode({
    iss: ISSUER, aud: claims.audience, sub: `agent:${claims.jti}`, jti: claims.jti,
    binding_id: claims.bindingId, scope: claims.tenantId, roles: claims.roles,
    iat: claims.issuedAt, exp: claims.expiresAt,
  });
  const signature = await crypto.subtle.sign("RSASSA-PKCS1-v1_5", privateKey, new TextEncoder().encode(`${head}.${body}`));
  return `${head}.${body}.${Buffer.from(signature).toString("base64url")}`;
}

describe("surface C HTTP integration (H4 acceptance)", () => {
  test("issue → exchange → act with audit attribution → revoke → next request denies", async () => {
    const store = new InMemoryAgentCredentialStore();
    const audits: AgentAudit[] = [];
    const lifecycleEvents: unknown[] = [];
    let nowSec = NOW;
    const handler = createSurfaceCHandler({
      store,
      resolveOwnerSession: async (req) => (req.headers.get("x-owner-session") === "yes" ? ownerSession : null),
      frontDoor: {
        issuer: ISSUER, audience: AUDIENCE, jwks, now: () => nowSec,
        resolveAccountId: async () => ACCOUNT,
        loadRoles: async () => ["owner", "agent"],
        audit: async (actor) => { audits.push(actor); },
      },
      mintAccessToken: signClaims,
      auditLifecycle: async (event) => { lifecycleEvents.push(event); },
      now: () => nowSec,
      generateJti: () => "e2e-credential-1",
      generateSecret: () => "e2e-fixture-secret",
    });
    const base = "https://surface-c.example";

    // Unauthenticated issuance denies.
    const anon = await handler(new Request(`${base}/api/agent-credentials`, { method: "POST", body: "{}" }));
    expect(anon.status).toBe(401);

    // Issuance from the owner session: secret in this response only.
    const issuedRes = await handler(new Request(`${base}/api/agent-credentials`, {
      method: "POST",
      headers: { "x-owner-session": "yes", "content-type": "application/json" },
      body: JSON.stringify({ tenantId: BINDING, audience: AUDIENCE, ttlSec: 86400 }),
    }));
    expect(issuedRes.status).toBe(201);
    expect(issuedRes.headers.get("cache-control")).toBe("no-store");
    const issuedBody = (await issuedRes.json()) as { credential: { jti: string }; secret: string };
    expect(issuedBody.credential.jti).toBe("e2e-credential-1");
    expect(JSON.stringify(await store.getByJti("e2e-credential-1"))).not.toContain(issuedBody.secret);

    // Client-credentials exchange.
    const tokenRes = await handler(new Request(`${base}/oauth/token`, {
      method: "POST", headers: { "content-type": "application/json" },
      body: JSON.stringify({ grant_type: "client_credentials", client_id: "e2e-credential-1", client_secret: issuedBody.secret, audience: AUDIENCE }),
    }));
    expect(tokenRes.status).toBe(200);
    expect(tokenRes.headers.get("cache-control")).toBe("no-store");
    const tokenBody = (await tokenRes.json()) as { access_token: string; expires_in: number };
    expect(tokenBody.expires_in).toBe(900);

    // The agent acts on exactly one tenant, audited with jti + owner.
    const whoami = await handler(new Request(`${base}/api/agent/whoami`, {
      headers: { authorization: `Bearer ${tokenBody.access_token}`, "x-tenant-id": BINDING },
    }));
    expect(whoami.status).toBe(200);
    const whoamiBody = (await whoami.json()) as Record<string, unknown>;
    expect(whoamiBody).toEqual({ role: "agent", tenantId: BINDING });
    expect(audits).toEqual([{ jti: "e2e-credential-1", accountId: ACCOUNT, bindingId: BINDING, tenantId: BINDING }]);

    // A foreign tenant denies even with the same token.
    const foreign = await handler(new Request(`${base}/api/agent/whoami`, {
      headers: { authorization: `Bearer ${tokenBody.access_token}`, "x-tenant-id": "binding-2" },
    }));
    expect(foreign.status).toBe(401);

    // Revocation by jti: the very next request and exchange deny.
    const revokeRes = await handler(new Request(`${base}/api/agent-credentials/e2e-credential-1`, {
      method: "DELETE", headers: { "x-owner-session": "yes" },
    }));
    expect(revokeRes.status).toBe(200);
    expect(lifecycleEvents).toEqual([
      {
        kind: "issued", jti: "e2e-credential-1", accountId: ACCOUNT,
        bindingId: BINDING, tenantId: BINDING, actorTokenId: ownerSession.tokenId, at: NOW,
      },
      {
        kind: "revoked", jti: "e2e-credential-1", accountId: ACCOUNT,
        bindingId: BINDING, tenantId: BINDING, actorTokenId: ownerSession.tokenId, at: NOW,
      },
    ]);
    const afterRevoke = await handler(new Request(`${base}/api/agent/whoami`, {
      headers: { authorization: `Bearer ${tokenBody.access_token}`, "x-tenant-id": BINDING },
    }));
    expect(afterRevoke.status).toBe(401);
    const exchangeAfterRevoke = await handler(new Request(`${base}/oauth/token`, {
      method: "POST", headers: { "content-type": "application/json" },
      body: JSON.stringify({ grant_type: "client_credentials", client_id: "e2e-credential-1", client_secret: issuedBody.secret, audience: AUDIENCE }),
    }));
    expect(exchangeAfterRevoke.status).toBe(401);
    nowSec = NOW; // keep the pinned clock explicit for later tests
  });
});
