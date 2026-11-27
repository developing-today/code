import { afterEach, describe, expect, test } from "bun:test";
import { generateKeyPairSync, sign } from "node:crypto";
import {
  clearIdpJwksCache,
  enforceIdpWriteRole,
  verifyIdpAccessToken,
} from "../src/idp-identity.ts";

const ISSUER = "http://idp.test";
const KID = "fixture-kid";
const keys = generateKeyPairSync("rsa", { modulusLength: 2048 });
const jwk = { ...keys.publicKey.export({ format: "jwk" }), kid: KID, use: "sig", alg: "RS256" };

function makeToken(claims: Record<string, unknown> = {}) {
  const header = Buffer.from(JSON.stringify({ alg: "RS256", kid: KID, typ: "JWT" })).toString("base64url");
  const payload = Buffer.from(
    JSON.stringify({
      iss: ISSUER,
      token_use: "access",
      sub: "owner-subject",
      roles: ["owner"],
      exp: Math.floor(Date.now() / 1000) + 300,
      ...claims,
    }),
  ).toString("base64url");
  const content = `${header}.${payload}`;
  return `${content}.${sign("RSA-SHA256", Buffer.from(content), keys.privateKey).toString("base64url")}`;
}

function idpFetch(): typeof fetch {
  return (async (input: RequestInfo | URL) => {
    const url = String(input);
    if (url === `${ISSUER}/.well-known/openid-configuration`) {
      return Response.json({ issuer: ISSUER, jwks_uri: `${ISSUER}/jwks` });
    }
    if (url === `${ISSUER}/jwks`) return Response.json({ keys: [jwk] });
    return new Response("not found", { status: 404 });
  }) as typeof fetch;
}

const env = { IDP_PROVIDER_ISSUER: ISSUER };

afterEach(() => clearIdpJwksCache());

describe("aggregator IdP access-token write gate (#14411)", () => {
  test("no configured issuer preserves the existing rollout behavior", async () => {
    expect(await enforceIdpWriteRole(new Request("https://agg/api/actions"), {}, idpFetch())).toEqual({
      ok: true,
      configured: false,
    });
  });

  test("missing and malformed bearer tokens are 401", async () => {
    const missing = await enforceIdpWriteRole(new Request("https://agg/api/actions"), env, idpFetch());
    expect(missing).toMatchObject({ ok: false, status: 401 });
    const invalid = await enforceIdpWriteRole(
      new Request("https://agg/api/actions", { headers: { authorization: "Bearer not-a-jwt" } }),
      env,
      idpFetch(),
    );
    expect(invalid).toMatchObject({ ok: false, status: 401 });
  });

  test("RS256 owner and orchestrator tokens pass the write gate", async () => {
    const owner = await enforceIdpWriteRole(
      new Request("https://agg/api/actions", { headers: { authorization: `Bearer ${makeToken()}` } }),
      env,
      idpFetch(),
    );
    expect(owner).toMatchObject({ ok: true, configured: true, token: { sub: "owner-subject", roles: ["owner"] } });

    const orchestrator = await enforceIdpWriteRole(
      new Request("https://agg/api/actions", {
        headers: { authorization: `Bearer ${makeToken({ roles: ["orchestrator"] })}` },
      }),
      env,
      idpFetch(),
    );
    expect(orchestrator).toMatchObject({ ok: true, configured: true, token: { roles: ["orchestrator"] } });
  });

  test("valid read-only token is 403 and untrusted token claims cannot grant roles", async () => {
    const denied = await enforceIdpWriteRole(
      new Request("https://agg/api/actions", {
        headers: { authorization: `Bearer ${makeToken({ roles: ["read-only"] })}` },
      }),
      env,
      idpFetch(),
    );
    expect(denied).toMatchObject({ ok: false, status: 403 });
  });

  test("rejects wrong issuer, token use, expiry and signature", async () => {
    for (const claims of [
      { iss: "https://wrong.test" },
      { token_use: "id" },
      { exp: Math.floor(Date.now() / 1000) - 1 },
    ]) {
      await expect(verifyIdpAccessToken(makeToken(claims), ISSUER, idpFetch())).rejects.toThrow();
    }
    const forged = `${makeToken().split(".").slice(0, 2).join(".")}.${Buffer.from("forged").toString("base64url")}`;
    await expect(verifyIdpAccessToken(forged, ISSUER, idpFetch())).rejects.toThrow(/signature/);
  });

  test("IdP discovery outage fails closed as 401", async () => {
    const unavailable = await enforceIdpWriteRole(
      new Request("https://agg/api/actions", { headers: { authorization: `Bearer ${makeToken()}` } }),
      env,
      Object.assign(async () => new Response("unavailable", { status: 503 }), { preconnect: () => {} }),
    );
    expect(unavailable).toMatchObject({ ok: false, status: 401 });
  });
});
