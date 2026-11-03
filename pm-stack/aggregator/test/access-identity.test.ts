import { afterEach, describe, expect, test } from "bun:test";
import { generateKeyPairSync, sign } from "node:crypto";
import type { DashAuthor } from "../src/model.ts";
import {
  ACCESS_JWT_HEADER,
  clearAccessJwksCache,
  deriveAuthorFromAccess,
  enforceAccessAuthor,
  readAccessConfig,
  verifyAccessJwt,
} from "../src/access-identity.ts";

const TEAM = "team.cloudflareaccess.com";
const AUD = "aggregator-aud";
const OWNER = "drew@example.com";
const NON_OWNER_SUB = "access|member-17";

const keys = generateKeyPairSync("rsa", { modulusLength: 2048 });
const publicJwk = keys.publicKey.export({ format: "jwk" });
const jwks = { keys: [{ ...publicJwk, kid: "test-key", alg: "RS256", use: "sig" }] };

function b64url(value: string | Buffer): string {
  return Buffer.from(value).toString("base64url");
}

function token(
  claims: Record<string, unknown> = {
    iss: `https://${TEAM}`,
    aud: AUD,
    exp: Math.floor(Date.now() / 1000) + 300,
    sub: NON_OWNER_SUB,
    email: "member@example.com",
  },
  overrides: { alg?: string; kid?: string } = {},
): string {
  const header = b64url(JSON.stringify({ alg: "RS256", kid: "test-key", ...overrides }));
  const payload = b64url(JSON.stringify(claims));
  const input = `${header}.${payload}`;
  const signature = sign("RSA-SHA256", Buffer.from(input), keys.privateKey).toString("base64url");
  return `${input}.${signature}`;
}

function fakeFetch(body: unknown = jwks, status = 200): typeof fetch {
  return (async () => new Response(JSON.stringify(body), { status })) as unknown as typeof fetch;
}

function accessEnv(ownerIds = OWNER): Record<string, string> {
  return {
    PM_AGG_ACCESS_TEAM_DOMAIN: TEAM,
    PM_AGG_ACCESS_AUD: AUD,
    PM_AGG_OWNER_IDS: ownerIds,
  };
}

const assertedAgent: DashAuthor = { kind: "agent", id: "forged-agent", label: "Forged agent" };
const assertedDrew: DashAuthor = { kind: "drew", id: "drew", label: "Drew" };

afterEach(() => clearAccessJwksCache());

describe("config-gated Access author derivation (#1253)", () => {
  test("unconfigured Access preserves the asserted author exactly", async () => {
    expect(readAccessConfig({ PM_AGG_ACCESS_TEAM_DOMAIN: TEAM })).toBeNull();
    const result = await enforceAccessAuthor(
      new Request("https://aggregator/api/comments"),
      assertedAgent,
      {},
      fakeFetch(),
    );
    expect(result).toEqual({ ok: true, author: assertedAgent });
  });

  test("valid JWT verifies against team JWKS and derives non-owner identity", async () => {
    const verified = await verifyAccessJwt(token(), { teamDomain: TEAM, aud: AUD, ownerIds: [OWNER] }, fakeFetch());
    expect(verified).toEqual({ sub: NON_OWNER_SUB, email: "member@example.com" });
    const result = deriveAuthorFromAccess(verified, assertedAgent, [OWNER]);
    expect(result).toEqual({
      ok: true,
      author: { kind: "agent", id: NON_OWNER_SUB, label: "member@example.com" },
    });
  });

  test("verified allowlisted email derives drew, ignoring caller-provided identity", async () => {
    const jwt = token({
      iss: `https://${TEAM}`,
      aud: AUD,
      exp: Math.floor(Date.now() / 1000) + 300,
      sub: "owner-sub",
      email: OWNER,
    });
    const result = await enforceAccessAuthor(
      new Request("https://aggregator/api/comments", { headers: { [ACCESS_JWT_HEADER]: jwt } }),
      assertedAgent,
      accessEnv(),
      fakeFetch(),
    );
    expect(result).toEqual({ ok: true, author: { kind: "drew", id: OWNER, label: OWNER } });
  });

  test("asserted drew by a non-owner is forbidden", () => {
    expect(deriveAuthorFromAccess({ sub: NON_OWNER_SUB, email: "member@example.com" }, assertedDrew, [OWNER])).toEqual({
      ok: false,
      status: 403,
      error: "asserted drew author contradicts the verified Access identity",
    });
  });

  test("empty owner allowlist fails closed for drew, but derives other identities", async () => {
    const verified = { sub: NON_OWNER_SUB, email: "member@example.com" };
    expect(deriveAuthorFromAccess(verified, assertedDrew, [])).toMatchObject({ ok: false, status: 403 });
    expect(deriveAuthorFromAccess(verified, assertedAgent, [])).toMatchObject({
      ok: true,
      author: { kind: "agent", id: NON_OWNER_SUB, label: "member@example.com" },
    });

    // Exercise the configured request path with OWNER_IDS absent, not just
    // the pure mapping helper: an asserted owner still fails closed.
    const noOwnerConfig = { PM_AGG_ACCESS_TEAM_DOMAIN: TEAM, PM_AGG_ACCESS_AUD: AUD };
    const result = await enforceAccessAuthor(
      new Request("https://aggregator/api/comments", {
        headers: { [ACCESS_JWT_HEADER]: token() },
      }),
      assertedDrew,
      noOwnerConfig,
      fakeFetch(),
    );
    expect(result).toMatchObject({ ok: false, status: 403 });
  });

  test("missing or invalid assertion is 401; audience mismatch is 401", async () => {
    const missing = await enforceAccessAuthor(
      new Request("https://aggregator/api/comments"),
      assertedAgent,
      accessEnv(),
      fakeFetch(),
    );
    expect(missing).toMatchObject({ ok: false, status: 401 });

    const invalid = await enforceAccessAuthor(
      new Request("https://aggregator/api/comments", { headers: { [ACCESS_JWT_HEADER]: "not-a-jwt" } }),
      assertedAgent,
      accessEnv(),
      fakeFetch(),
    );
    expect(invalid).toMatchObject({ ok: false, status: 401 });

    const wrongAud = token({
      iss: `https://${TEAM}`,
      aud: "another-app",
      exp: Math.floor(Date.now() / 1000) + 300,
      sub: NON_OWNER_SUB,
      email: "member@example.com",
    });
    const audience = await enforceAccessAuthor(
      new Request("https://aggregator/api/comments", { headers: { [ACCESS_JWT_HEADER]: wrongAud } }),
      assertedAgent,
      accessEnv(),
      fakeFetch(),
    );
    expect(audience).toMatchObject({ ok: false, status: 401 });
  });

  test("bad signature is rejected even when claims and kid look valid", async () => {
    const good = token();
    const parts = good.split(".");
    const other = generateKeyPairSync("rsa", { modulusLength: 2048 });
    parts[2] = sign("RSA-SHA256", Buffer.from(`${parts[0]}.${parts[1]}`), other.privateKey).toString("base64url");
    await expect(
      verifyAccessJwt(parts.join("."), { teamDomain: TEAM, aud: AUD, ownerIds: [OWNER] }, fakeFetch()),
    ).rejects.toThrow("JWT signature invalid");
  });
});
