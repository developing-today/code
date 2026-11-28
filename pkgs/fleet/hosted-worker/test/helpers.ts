// Shared test harness for fleet-hosted (#12583). Mints a real local
// RSA keypair standing in for the #9077 provider's signing key, serves
// its public half as the JWKS through an injected fetcher (no network),
// and builds the config/tenancy fixture the worker runs against.
// Oracles are the design's rules (§4.1, §3.3 rule 3): tests assert the
// refusal codes, not just statuses, so a fail-open change names itself.

import { sha256Hex, type AuthDeps } from "../src/auth.ts";
import { configFromEnv } from "../src/config.ts";
import type { Jwks } from "../src/jwt.ts";

export const ISSUER = "https://sso.devtoday.test";
export const AUDIENCE = "fleet-hosted";
export const KID = "test-key-1";

export const BINDING_A = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
export const BINDING_B = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
export const BINDING_C = "cccccccccccccccccccccccccccccccc";

export const SERVICE_TOKEN = "svc-token-for-tenant-a-0123456789abcdef";

/** The queue-scoped service token (#14942 CF-2): the cutover's
 * recorded auth route — resolved against the tenancy directory
 * alone, scoped to the queue door. Tests minting the full write
 * lifecycle through the HTTP door use this, never the
 * stamping/query-scoped SERVICE_TOKEN. */
export const QUEUE_SERVICE_TOKEN = "svc-token-for-queue-door-a-0123456789abcdef";

/** The webhook receiver's shared secret in the harness config
 * (#12590): deliveries in tests sign with this; the receiver's
 * unconfigured case is exercised with a config built without it. */
export const WEBHOOK_SECRET = "test-webhook-secret-0123456789abcdef";

function b64urlEncode(data: string | Uint8Array): string {
  const bytes = typeof data === "string" ? new TextEncoder().encode(data) : data;
  let bin = "";
  for (const b of bytes) bin += String.fromCharCode(b);
  return btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

export interface Harness {
  deps: AuthDeps;
  /** Sign a payload as an RP token from the fixture provider key. */
  sign(payload: Record<string, unknown>, header?: Record<string, unknown>): Promise<string>;
  /** Sign with an unrelated key (same kid): signature must not verify. */
  signWithWrongKey(payload: Record<string, unknown>): Promise<string>;
  /** A token payload that is valid by default; override per test. */
  rpPayload(overrides?: Record<string, unknown>): Record<string, unknown>;
}

export async function makeHarness(): Promise<Harness> {
  const keyPair = await crypto.subtle.generateKey(
    {
      name: "RSASSA-PKCS1-v1_5",
      modulusLength: 2048,
      publicExponent: new Uint8Array([1, 0, 1]),
      hash: "SHA-256",
    },
    true,
    ["sign", "verify"],
  );
  const wrongKeyPair = await crypto.subtle.generateKey(
    {
      name: "RSASSA-PKCS1-v1_5",
      modulusLength: 2048,
      publicExponent: new Uint8Array([1, 0, 1]),
      hash: "SHA-256",
    },
    true,
    ["sign", "verify"],
  );
  const publicJwk = await crypto.subtle.exportKey("jwk", keyPair.publicKey);
  const jwks: Jwks = { keys: [{ ...publicJwk, kid: KID, alg: "RS256" }] };
  const jwksFetcher = async () => jwks;

  const fixture = JSON.stringify({
    subjects: [
      { issuer: ISSUER, subject: "user-sub-1", binding_id: BINDING_A, active: true },
      { issuer: ISSUER, subject: "agent-sub-1", binding_id: BINDING_A, active: true },
      { issuer: ISSUER, subject: "dormant-sub", binding_id: BINDING_C, active: false },
    ],
    service_tokens: [
      {
        sha256: await sha256Hex(SERVICE_TOKEN),
        binding_id: BINDING_A,
        routes: ["stamping", "query"],
        active: true,
        label: "pm-service-test",
      },
      {
        sha256: await sha256Hex(QUEUE_SERVICE_TOKEN),
        binding_id: BINDING_A,
        routes: ["queue"],
        active: true,
        label: "queue-door-roundtrip-probe",
      },
    ],
  });

  const config = configFromEnv({
    RP_ISSUER: ISSUER,
    RP_AUDIENCE: AUDIENCE,
    TENANCY_FIXTURE_JSON: fixture,
    GITHUB_WEBHOOK_SECRET: WEBHOOK_SECRET,
  });

  async function signWith(
    key: CryptoKey,
    payload: Record<string, unknown>,
    header: Record<string, unknown> = {},
  ): Promise<string> {
    const h = { alg: "RS256", typ: "JWT", kid: KID, ...header };
    const input = `${b64urlEncode(JSON.stringify(h))}.${b64urlEncode(JSON.stringify(payload))}`;
    const sig = new Uint8Array(await crypto.subtle.sign("RSASSA-PKCS1-v1_5", key, new TextEncoder().encode(input)));
    return `${input}.${b64urlEncode(sig)}`;
  }

  return {
    deps: { config, jwksFetcher },
    sign: (payload, header) => signWith(keyPair.privateKey, payload, header),
    signWithWrongKey: (payload) => signWith(wrongKeyPair.privateKey, payload),
    rpPayload: (overrides = {}) => {
      const now = Math.floor(Date.now() / 1000);
      return {
        iss: ISSUER,
        aud: AUDIENCE,
        sub: "user-sub-1",
        iat: now,
        exp: now + 3600,
        roles: ["owner"],
        ...overrides,
      };
    },
  };
}

export function bearerReq(path: string, token: string, init: RequestInit = {}): Request {
  return new Request(`https://fleet-hosted.test${path}`, {
    ...init,
    headers: { authorization: `Bearer ${token}`, ...(init.headers ?? {}) },
  });
}
