import { describe, expect, test } from "bun:test";
import { createSign, generateKeyPairSync } from "node:crypto";
import {
  AuthError,
  assertCallerRepo,
  authenticate,
  timingSafeStringEqual,
  verifyOidcToken,
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
  const req = (auth?: string) =>
    new Request("http://x/v1/tasks", { headers: auth ? { authorization: auth } : {} });

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

describe("OIDC repo binding", () => {
  const oidcCaller = {
    type: "oidc" as const,
    repository: "developing-today/code",
    actor: "dezren39",
  };

  test("claim equal to the request repo passes", () => {
    expect(() => assertCallerRepo(oidcCaller, "developing-today/code", ["developing-today/code"])).not.toThrow();
  });

  test("claim different from the request repo is rejected — a workflow only dispatches for its own repo", () => {
    expect(() =>
      assertCallerRepo(oidcCaller, "developing-today/opencode", ["developing-today/code", "developing-today/opencode"]),
    ).toThrow(/does not match/);
  });

  test("bearer callers are bound only by the allowlist", () => {
    expect(() => assertCallerRepo({ type: "bearer" }, "developing-today/opencode", ["developing-today/opencode"])).not.toThrow();
    expect(() => assertCallerRepo({ type: "bearer" }, "dezren39/nix", ["developing-today/code"])).toThrow(/allowlist/);
  });
});
