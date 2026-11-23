// Environment configuration for fleet-hosted (#12583).
//
// Everything the auth layer needs is configuration, because its two
// upstreams are other components' deliverables: the #9077 relying-party
// provider (issuer / JWKS) and component E's tenancy plane (here, the
// fixture seam in tenancy.ts). Missing configuration never fails open:
// it makes authConfigured false, protected routes refuse with 503, and
// health reports the gap as booleans (never values).
//
// The stamp signing material (#12586) follows the same rule: keys are
// configuration referenced by name, parsed and length-checked here,
// held as bytes, and never logged or reported. Malformed material is a
// configuration failure — that piece parses to null and the stamp path
// fails closed (see stamp-sig.ts), exactly like a malformed tenancy
// fixture.

import type { RpValidationConfig } from "./jwt.ts";
import { parseTenancyFixture, type TenancyDirectory } from "./tenancy.ts";

export const HOSTED_VERSION = "0.1.0";
export const SERVICE_NAME = "fleet-hosted";
export const DEFAULT_AUDIENCE = "fleet-hosted";

export interface HostedEnv {
  RP_ISSUER?: string;
  RP_AUDIENCE?: string;
  RP_JWKS_URI?: string;
  TENANCY_FIXTURE_JSON?: string;
  /** Stamp signing key (#12586): base64url PKCS#8 Ed25519 private key.
   * A secret, referenced by name only; never logged. */
  STAMP_SIGNING_KEY_PKCS8?: string;
  /** Key id the signing key mints stamps under ([A-Za-z0-9_-]+). */
  STAMP_SIGNING_KEY_ID?: string;
  /** Stamp verifier keys (#12586): JSON object mapping key id to a
   * base64url raw Ed25519 public key. Several ids = the rotation
   * window; both editions' authorities appear here side by side. */
  STAMP_VERIFY_KEYS_JSON?: string;
}

/** Stamp signing key material: the parsed key, bytes only. */
export interface StampSigningMaterial {
  keyId: string;
  pkcs8: Uint8Array;
}

export interface StampConfig {
  /** Signing material, or null when no (valid) signing key is set. */
  signing: StampSigningMaterial | null;
  /** Verifier public keys by key id (raw Ed25519 bytes), or null when
   * no (valid) verifier set is configured. */
  verifyKeys: ReadonlyMap<string, Uint8Array> | null;
}

export interface HostedConfig {
  version: string;
  /** RP validation config, or null when no issuer is configured. */
  rp: RpValidationConfig | null;
  /** Tenancy directory, or null when none is configured/readable. */
  tenancy: TenancyDirectory | null;
  /** True only when every auth input is present and parsed. */
  authConfigured: boolean;
  /** Stamp signature key material (#12586), independent of auth. */
  stamp: StampConfig;
}

/** Stamp key ids name a key inside the stamp_sig value, so they may
 * not contain the format's `.` separator (see stamp-sig.ts). */
export function isValidStampKeyId(value: string): boolean {
  return /^[A-Za-z0-9_-]+$/.test(value);
}

function b64urlToBytesOrNull(text: string): Uint8Array | null {
  try {
    const b64 = text.replace(/-/g, "+").replace(/_/g, "/");
    const padded = b64 + "=".repeat((4 - (b64.length % 4)) % 4);
    const bin = atob(padded);
    const out = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
    return out;
  } catch {
    return null;
  }
}

const ED25519_PKCS8_BYTES = 48;
const ED25519_PUBLIC_KEY_BYTES = 32;

function parseStampConfig(env: HostedEnv): StampConfig {
  let signing: StampSigningMaterial | null = null;
  if (env.STAMP_SIGNING_KEY_PKCS8 && env.STAMP_SIGNING_KEY_ID) {
    const pkcs8 = b64urlToBytesOrNull(env.STAMP_SIGNING_KEY_PKCS8);
    if (
      pkcs8 !== null &&
      pkcs8.length === ED25519_PKCS8_BYTES &&
      isValidStampKeyId(env.STAMP_SIGNING_KEY_ID)
    ) {
      signing = { keyId: env.STAMP_SIGNING_KEY_ID, pkcs8 };
    }
    // Malformed signing material stays null: signing fails closed.
  }

  let verifyKeys: Map<string, Uint8Array> | null = null;
  if (env.STAMP_VERIFY_KEYS_JSON) {
    try {
      const parsed: unknown = JSON.parse(env.STAMP_VERIFY_KEYS_JSON);
      if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
        throw new Error("verify keys must be a JSON object");
      }
      const map = new Map<string, Uint8Array>();
      for (const [keyId, text] of Object.entries(parsed as Record<string, unknown>)) {
        if (!isValidStampKeyId(keyId) || typeof text !== "string") {
          throw new Error("verify key entry is malformed");
        }
        const raw = b64urlToBytesOrNull(text);
        if (raw === null || raw.length !== ED25519_PUBLIC_KEY_BYTES) {
          throw new Error("verify key is not a raw Ed25519 public key");
        }
        map.set(keyId, raw);
      }
      verifyKeys = map.size > 0 ? map : null;
    } catch {
      // A malformed verifier set is a configuration failure: leave it
      // null so verification reports unverifiable, never verified.
      verifyKeys = null;
    }
  }

  return { signing, verifyKeys };
}

export function configFromEnv(env: HostedEnv): HostedConfig {
  const rp: RpValidationConfig | null = env.RP_ISSUER
    ? {
        issuer: env.RP_ISSUER,
        audience: env.RP_AUDIENCE ?? DEFAULT_AUDIENCE,
        jwksUri: env.RP_JWKS_URI ?? `${env.RP_ISSUER.replace(/\/$/, "")}/.well-known/jwks.json`,
      }
    : null;

  let tenancy: TenancyDirectory | null = null;
  if (env.TENANCY_FIXTURE_JSON) {
    try {
      tenancy = parseTenancyFixture(env.TENANCY_FIXTURE_JSON);
    } catch {
      // A malformed tenancy fixture is a configuration failure, not a
      // per-request one: leave tenancy null so auth fails closed and
      // health shows tenancy unconfigured.
      tenancy = null;
    }
  }

  return {
    version: HOSTED_VERSION,
    rp,
    tenancy,
    authConfigured: rp !== null && tenancy !== null,
    stamp: parseStampConfig(env),
  };
}
