import { afterEach, describe, expect, test } from "bun:test";
import { generateKeyPairSync, sign } from "node:crypto";
import type { DashAuthor } from "../src/model.ts";
import {
  ACCESS_JWT_HEADER,
  clearAccessJwksCache,
  deriveAuthorFromAccess,
  createAccessAuthorEnforcer,
  enforceAccessAuthor,
  presentedAccessClaim,
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
    // #9606: the passed-through claim is never reported as verified.
    expect(result).toEqual({ ok: true, author: assertedAgent, verified: false });
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
    expect(result).toEqual({
      ok: true,
      author: { kind: "drew", id: "drew", label: "Drew" },
      verified: true,
      identity: { sub: "owner-sub", email: OWNER },
    });
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

  test("a 100-entry batch shares one failed JWT verification when the certs endpoint is down", async () => {
    let fetches = 0;
    const downFetch = (async () => {
      fetches += 1;
      throw new Error("certs endpoint unreachable");
    }) as unknown as typeof fetch;
    const jwt = token();
    const enforce = createAccessAuthorEnforcer(
      new Request("https://aggregator/api/actions/batch", { headers: { [ACCESS_JWT_HEADER]: jwt } }),
      accessEnv(),
      downFetch,
    );
    // Every entry gets the same 401 from one shared request verification;
    // per-item verification would stack one 5 s timeout per action.
    for (let index = 0; index < 100; index += 1) {
      const asserted = index % 2 === 0 ? assertedAgent : assertedDrew;
      const result = await enforce(asserted);
      expect(result).toMatchObject({ ok: false, status: 401 });
    }
    expect(fetches).toBe(1);
  });

  test("a 100-entry batch does not refresh JWKS per entry for an unknown kid", async () => {
    let fetches = 0;
    const countedFetch = (async () => {
      fetches += 1;
      return new Response(JSON.stringify(jwks), { status: 200 });
    }) as unknown as typeof fetch;
    const jwt = token(undefined, { kid: "unknown-key" });
    const enforce = createAccessAuthorEnforcer(
      new Request("https://aggregator/api/actions/batch", { headers: { [ACCESS_JWT_HEADER]: jwt } }),
      accessEnv(),
      countedFetch,
    );
    for (let index = 0; index < 100; index += 1) {
      const result = await enforce(assertedAgent);
      expect(result).toMatchObject({ ok: false, status: 401 });
    }
    // Initial load + one rotation refresh for the single verification;
    // without request-level memoization, each entry forces another refresh.
    expect(fetches).toBe(2);
  });

  test("a 100-entry batch still checks asserted-author contradictions per entry", async () => {
    let fetches = 0;
    const countedFetch = (async () => {
      fetches += 1;
      return new Response(JSON.stringify(jwks), { status: 200 });
    }) as unknown as typeof fetch;
    const enforce = createAccessAuthorEnforcer(
      new Request("https://aggregator/api/actions/batch", { headers: { [ACCESS_JWT_HEADER]: token() } }),
      accessEnv(),
      countedFetch,
    );
    for (let index = 0; index < 100; index += 1) {
      const result = await enforce(index % 2 === 0 ? assertedAgent : assertedDrew);
      if (index % 2 === 0) {
        expect(result).toMatchObject({ ok: true, author: { kind: "agent", id: NON_OWNER_SUB }, verified: true });
      } else {
        expect(result).toMatchObject({ ok: false, status: 403 });
      }
    }
    expect(fetches).toBe(1);
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

describe("Access denials are logged with the presented claim (#9603)", () => {
  const post = (headers: Record<string, string> = {}) =>
    new Request("https://aggregator/api/comments?x=1", { method: "POST", headers });

  function recorder(): { lines: string[]; log: (line: string) => void } {
    const lines: string[] = [];
    return { lines, log: (line) => lines.push(line) };
  }

  test("a missing assertion logs route, status, reason and no claim", async () => {
    const r = recorder();
    const result = await enforceAccessAuthor(post(), assertedAgent, accessEnv(), fakeFetch(), r.log);
    expect(result).toMatchObject({ ok: false, status: 401 });
    expect(r.lines).toHaveLength(1);
    expect(r.lines[0]).toContain("[access] denied POST /api/comments ");
    expect(r.lines[0]).toContain("status=401");
    expect(r.lines[0]).toContain('reason="missing Cf-Access-Jwt-Assertion"');
    expect(r.lines[0]).toContain("claim=none");
    expect(r.lines[0]).toContain('asserted="agent:forged-agent"');
  });

  test("a token that fails verification logs its claims as unverified, never the token", async () => {
    const r = recorder();
    const wrongAud = token({
      iss: `https://${TEAM}`,
      aud: "another-app",
      exp: Math.floor(Date.now() / 1000) + 300,
      sub: NON_OWNER_SUB,
      email: "member@example.com",
    });
    const result = await enforceAccessAuthor(
      post({ [ACCESS_JWT_HEADER]: wrongAud }),
      assertedAgent,
      accessEnv(),
      fakeFetch(),
      r.log,
    );
    expect(result).toMatchObject({ ok: false, status: 401 });
    expect(r.lines).toHaveLength(1);
    expect(r.lines[0]).toContain("JWT audience mismatch");
    expect(r.lines[0]).toContain(`sub="${NON_OWNER_SUB}" email="member@example.com" (unverified)`);
    for (const part of wrongAud.split(".")) expect(r.lines[0]).not.toContain(part);
  });

  test("a verified non-owner claiming drew logs the 403 with the verified claim", async () => {
    const r = recorder();
    const result = await enforceAccessAuthor(
      post({ [ACCESS_JWT_HEADER]: token() }),
      assertedDrew,
      accessEnv(),
      fakeFetch(),
      r.log,
    );
    expect(result).toMatchObject({ ok: false, status: 403 });
    expect(r.lines).toHaveLength(1);
    expect(r.lines[0]).toContain("status=403");
    expect(r.lines[0]).toContain(`sub="${NON_OWNER_SUB}" email="member@example.com" (verified)`);
    expect(r.lines[0]).toContain('asserted="drew:drew"');
  });

  test("allowed writes and unconfigured Access log nothing", async () => {
    const r = recorder();
    expect(
      await enforceAccessAuthor(post({ [ACCESS_JWT_HEADER]: token() }), assertedAgent, accessEnv(), fakeFetch(), r.log),
    ).toMatchObject({ ok: true, verified: true });
    expect(await enforceAccessAuthor(post(), assertedDrew, {}, fakeFetch(), r.log)).toMatchObject({
      ok: true,
      verified: false,
    });
    expect(r.lines).toEqual([]);
  });

  test("a batch logs a shared 401 once and each per-entry 403", async () => {
    const down = recorder();
    const downFetch = (async () => {
      throw new Error("certs endpoint unreachable");
    }) as unknown as typeof fetch;
    const failing = createAccessAuthorEnforcer(
      post({ [ACCESS_JWT_HEADER]: token() }),
      accessEnv(),
      downFetch,
      down.log,
    );
    for (let index = 0; index < 20; index += 1) await failing(assertedAgent);
    expect(down.lines).toHaveLength(1);

    const mixed = recorder();
    const enforce = createAccessAuthorEnforcer(
      post({ [ACCESS_JWT_HEADER]: token() }),
      accessEnv(),
      fakeFetch(),
      mixed.log,
    );
    for (let index = 0; index < 20; index += 1) await enforce(index % 2 === 0 ? assertedAgent : assertedDrew);
    expect(mixed.lines).toHaveLength(10);
    expect(mixed.lines.every((line) => line.includes("status=403"))).toBe(true);
  });

  test("caller-controlled claims cannot split or forge a log line", () => {
    const forged = `${b64url("{}")}.${b64url(JSON.stringify({ sub: "x\n[access] allowed POST /api/comments", email: 7 }))}.sig`;
    const claim = presentedAccessClaim(forged);
    expect(claim).not.toContain("\n");
    expect(claim).toContain('sub="x\\n[access] allowed POST /api/comments"');
    expect(claim).toContain('email="" (unverified)');
    expect(presentedAccessClaim("not-a-jwt")).toBe("malformed token");
    expect(presentedAccessClaim("a.%%%.c")).toBe("malformed token");
    expect(presentedAccessClaim("")).toBe("none");
  });
});
