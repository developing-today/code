import { describe, expect, test } from "bun:test";
import { createSign, generateKeyPairSync } from "node:crypto";
import {
  AuthError,
  assertCallerRepo,
  authenticate,
  parseJwt,
  timingSafeStringEqual,
  verifyOidcToken,
  parseJwks,
  fetchGithubJwks,
  resetJwksCacheForTests,
  GITHUB_JWKS_URL,
  type Jwks,
} from "../src/auth.ts";

// A throwaway RSA keypair stands in for GitHub's OIDC signer; the JWKS
// source is stubbed so no test touches the network.
const { publicKey, privateKey } = generateKeyPairSync("rsa", { modulusLength: 2048 });
const jwk = publicKey.export({ format: "jwk" }) as unknown as Record<string, unknown>;
jwk.kid = "test-kid-1";
jwk.alg = "RS256";
const jwks: Jwks = { keys: [jwk] };
const jwksSource = async () => jwks;

function b64url(data: Buffer | string): string {
  return Buffer.from(data).toString("base64").replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

function signJwt(claims: Record<string, unknown>): string {
  const header = b64url(JSON.stringify({ alg: "RS256", typ: "JWT", kid: "test-kid-1" }));
  const payload = b64url(JSON.stringify(claims));
  const signer = createSign("RSA-SHA256");
  signer.update(`${header}.${payload}`);
  return `${header}.${payload}.${b64url(signer.sign(privateKey))}`;
}

function jwtWithSegments(header: unknown, payload: unknown): string {
  return `${b64url(JSON.stringify(header)!)}.${b64url(JSON.stringify(payload)!)}.signature`;
}

const NOW = 1_800_000_000_000; // fixed "now" for claim checks
const baseClaims = {
  iss: "https://token.actions.githubusercontent.com",
  aud: "pm-dispatch",
  exp: Math.floor(NOW / 1000) + 600,
  iat: Math.floor(NOW / 1000),
  repository: "developing-today/code",
  actor: "dezren39",
  workflow_ref: "developing-today/code/.github/workflows/dispatch.yml@refs/heads/main",
  sub: "repo:developing-today/code:ref:refs/heads/main",
};

const oidcOpts = {
  audience: "pm-dispatch",
  allowlist: ["developing-today/code", "developing-today/opencode"],
  jwksSource,
  now: NOW,
};

describe("timing-safe bearer compare", () => {
  test("equal strings match, different do not", () => {
    expect(timingSafeStringEqual("s3cret", "s3cret")).toBe(true);
    expect(timingSafeStringEqual("s3cret", "s3creT")).toBe(false);
    expect(timingSafeStringEqual("s3cret", "s3cret-longer")).toBe(false);
  });
});

describe("bearer auth via authenticate", () => {
  const deps = { apiToken: "test-bearer-token", oidcAudience: "pm-dispatch", allowlist: ["developing-today/code"] };
  const req = (auth?: string) => new Request("http://x/v1/tasks", { headers: auth ? { authorization: auth } : {} });

  test("accepts the configured token", async () => {
    const id = await authenticate(req("Bearer test-bearer-token"), deps);
    expect(id).toEqual({ type: "bearer" });
  });

  test("rejects a wrong token", async () => {
    await expect(authenticate(req("Bearer nope"), deps)).rejects.toBeInstanceOf(AuthError);
  });

  test("rejects a missing header", async () => {
    await expect(authenticate(req(), deps)).rejects.toBeInstanceOf(AuthError);
  });
});

describe("OIDC verification", () => {
  test("rejects non-object JWT header and payload segments as malformed credentials", async () => {
    const header = { alg: "RS256", typ: "JWT", kid: "test-kid-1" };
    const cases: Array<[string, unknown, unknown]> = [
      ["null header", null, baseClaims],
      ["array header", [], baseClaims],
      ["number header", 42, baseClaims],
      ["string header", "header", baseClaims],
      ["boolean header", true, baseClaims],
      ["null payload", header, null],
      ["array payload", header, [baseClaims]],
      ["number payload", header, 42],
      ["string payload", header, "payload"],
      ["boolean payload", header, true],
    ];

    for (const [name, tokenHeader, payload] of cases) {
      const token = jwtWithSegments(tokenHeader, payload);
      expect(parseJwt(token), name).toBeNull();
      await expect(verifyOidcToken(token, oidcOpts), name).rejects.toBeInstanceOf(AuthError);
    }
  });

  test("accepts a well-formed token and returns the identity", async () => {
    const id = await verifyOidcToken(signJwt(baseClaims), oidcOpts);
    expect(id.type).toBe("oidc");
    if (id.type === "oidc") {
      expect(id.repository).toBe("developing-today/code");
      expect(id.actor).toBe("dezren39");
    }
  });

  test("rejects a tampered payload (bad signature)", async () => {
    const token = signJwt(baseClaims);
    const [h, , s] = token.split(".");
    const forged = `${h}.${b64url(JSON.stringify({ ...baseClaims, repository: "developing-today/opencode" }))}.${s}`;
    await expect(verifyOidcToken(forged, oidcOpts)).rejects.toBeInstanceOf(AuthError);
  });

  test("rejects an expired token", async () => {
    const token = signJwt({ ...baseClaims, exp: Math.floor(NOW / 1000) - 10 });
    await expect(verifyOidcToken(token, oidcOpts)).rejects.toThrow(/expired/);
  });

  test("rejects a wrong audience", async () => {
    const token = signJwt({ ...baseClaims, aud: "something-else" });
    await expect(verifyOidcToken(token, oidcOpts)).rejects.toThrow(/audience/);
  });

  test("rejects a repo outside the allowlist", async () => {
    const token = signJwt({ ...baseClaims, repository: "dezren39/nix" });
    await expect(verifyOidcToken(token, oidcOpts)).rejects.toThrow(/allowlist/);
  });
});

describe("cross-repo grants (assertCallerRepo)", () => {
  const allowlist = [
    "developing-today/code",
    "developing-today/opencode",
    "developing-today/openchamber",
    "dezren39/nix",
    "developing-today/zettel",
    "dezren39/dezren39",
  ];
  const openchamberCaller = {
    type: "oidc" as const,
    repository: "developing-today/openchamber",
    actor: "dezren39",
  };

  test("cross-repo allowed: an openchamber token may target opencode", () => {
    expect(() => assertCallerRepo(openchamberCaller, "developing-today/opencode", allowlist)).not.toThrow();
  });

  test("cross-repo allowed: a zettel token may target dezren39/dezren39 and dezren39/nix", () => {
    const zettelCaller = { type: "oidc" as const, repository: "developing-today/zettel", actor: "dezren39" };
    expect(() => assertCallerRepo(zettelCaller, "dezren39/dezren39", allowlist)).not.toThrow();
    expect(() => assertCallerRepo(zettelCaller, "dezren39/nix", allowlist)).not.toThrow();
  });

  test("same-repo still allowed", () => {
    expect(() => assertCallerRepo(openchamberCaller, "developing-today/openchamber", allowlist)).not.toThrow();
  });

  test("an OIDC caller outside the allowlist is rejected", () => {
    const outsider = { type: "oidc" as const, repository: "someone/else", actor: "mallory" };
    expect(() => assertCallerRepo(outsider, "developing-today/opencode", allowlist)).toThrow(/allowlist/);
  });

  test("a target outside the allowlist is rejected for OIDC callers", () => {
    expect(() => assertCallerRepo(openchamberCaller, "someone/else", allowlist)).toThrow(/allowlist/);
  });

  test("bearer callers are unaffected by caller/target repo pairing: an allowlisted target passes", () => {
    expect(() => assertCallerRepo({ type: "bearer" }, "developing-today/opencode", allowlist)).not.toThrow();
    expect(() => assertCallerRepo({ type: "bearer" }, "dezren39/nix", allowlist)).not.toThrow();
  });

  test("bearer callers are rejected for a non-allowlisted target", () => {
    expect(() => assertCallerRepo({ type: "bearer" }, "someone/else", allowlist)).toThrow(/allowlist/);
  });
});

describe("GitHub JWKS URL (code#4886)", () => {
  // The injectable jwksSource seam meant no test ever exercised the real
  // URL, so a bogus .json suffix shipped and every OIDC validation 401d.
  // Pin the constant to the jwks_uri published in the GitHub OIDC discovery
  // document (/.well-known/openid-configuration on the issuer).
  test("matches the discovery document jwks_uri (no .json suffix)", () => {
    expect(GITHUB_JWKS_URL).toBe("https://token.actions.githubusercontent.com/.well-known/jwks");
  });
});

describe("JWKS body shape (#7565)", () => {
  const malformed: Array<[string, unknown]> = [
    ["null body", null],
    ["empty object", {}],
    ["string body", "x"],
    ["keys is a string", { keys: "x" }],
    ["keys is a number", { keys: 42 }],
    ["keys contains null", { keys: [null] }],
    ["keys contains a number", { keys: [42] }],
  ];

  for (const [name, value] of malformed) {
    test(`verifyOidcToken rejects a malformed JWKS (${name}) as AuthError, not a raw TypeError`, async () => {
      const token = signJwt(baseClaims);
      const badSource = async () => value as Jwks;
      await expect(verifyOidcToken(token, { ...oidcOpts, jwksSource: badSource })).rejects.toBeInstanceOf(AuthError);
    });
    test(`parseJwks rejects ${name}`, () => {
      expect(() => parseJwks(value)).toThrow(AuthError);
    });
  }

  test("parseJwks accepts a well-formed JWKS", () => {
    expect(parseJwks(jwks)).toEqual(jwks);
  });

  test("fetchGithubJwks rejects a non-JSON 200 body as AuthError and does not cache it", async () => {
    resetJwksCacheForTests();
    const originalFetch = globalThis.fetch;
    let calls = 0;
    globalThis.fetch = (async () => {
      calls++;
      return {
        ok: true,
        json: async () => {
          throw new SyntaxError("Unexpected token < in JSON");
        },
      } as unknown as Response;
    }) as unknown as typeof fetch;
    try {
      await expect(fetchGithubJwks()).rejects.toBeInstanceOf(AuthError);
      await expect(fetchGithubJwks()).rejects.toBeInstanceOf(AuthError);
      expect(calls).toBe(2); // a failed fetch is never cached: each call re-fetches
    } finally {
      globalThis.fetch = originalFetch;
      resetJwksCacheForTests();
    }
  });

  test("fetchGithubJwks rejects a wrong-shape 200 body as AuthError", async () => {
    resetJwksCacheForTests();
    const originalFetch = globalThis.fetch;
    globalThis.fetch = (async () => ({ ok: true, json: async () => ({}) }) as unknown as Response) as unknown as typeof fetch;
    try {
      await expect(fetchGithubJwks()).rejects.toBeInstanceOf(AuthError);
    } finally {
      globalThis.fetch = originalFetch;
      resetJwksCacheForTests();
    }
  });
});
