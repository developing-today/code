// Direct-Turso Metadata read path (#12592; design §3.3, component C's
// prudence rules made concrete). This module is the token half of the
// path: the mint of scoped, expiring, read-only tokens for the
// metadata DB, and the scope inspector a bulk consumer (the collector,
// component A's dashboard) runs before it trusts a token with a query
// load. The live query execution against a deployed metadata DB is
// the next slice; nothing here opens a DB connection.
//
// THE RULES (design §3.3; component G #9085 owns the program-wide
// statement and supersedes if it rules differently):
//   1. Direct access is READ-ONLY, against the METADATA DB ONLY, with
//      a scoped, expiring token minted for one tenancy account.
//   2. Never direct: any write at all, the queue DB (all queue access
//      goes through a broker front door), any other DB, any
//      cross-tenant read, and any post-expiry use.
//   3. A direct-read token is a DATA-PLANE credential. It is not an
//      authentication to the service and confers no stamping or queue
//      rights — src/auth.ts already refuses this exact shape with
//      `data_plane_credential_refused`; the conformance suite asserts
//      a token minted here is refused there.
//
// TOKEN SHAPE: a libsql/Turso auth token is an EdDSA JWT. In the
// libSQL wire format, `a` is the Permission enum (`ro`, `rw`, or
// `roa`), not the Turso API's `read-only` label; sqld deserializes
// these exact values. `id` binds the token to a database and `exp`
// expires it. This mint always sets `a` to `ro` (Permission::Read)
// and `id` to the configured metadata DB — there is no parameter for
// either, so a write-capable or queue-bound token is not mintable
// through this module at all. The
// tenancy account rides as `binding_id` (component E's keying rule:
// the binding_id, never an email or provider subject) so the token's
// scope names its account; sqld ignores claims it does not know.
//
// MINTING SEAM: the design says tokens are "minted by the tenancy
// plane (E's model)". E's DB does not exist yet (see the seam note in
// src/tenancy.ts), so v1 mints locally with key material held in
// configuration — the sqld self-hosted model, parsed and fail-closed
// exactly like the stamp material in src/config.ts. The account is
// taken from the caller's authenticated context (a binding_id the
// tenancy directory resolved); this module checks its shape and
// refuses anything else. When E lands, a platform-API minter can
// replace the local signer behind `mintDirectReadToken`'s signature;
// the inspector and the claim shape do not change.

import type { DirectReadConfig } from "./config.ts";
import { FailClosed } from "./errors.ts";
import { parseJwt } from "./jwt.ts";
import { bytesToB64url } from "./stamp-sig.ts";
import { BINDING_ID_RE } from "./tenancy.ts";

export const DIRECT_READ_ACCESS = "ro";

/** Claims the mint writes, in one place so the shape is auditable. */
export interface DirectReadClaims {
  a: typeof DIRECT_READ_ACCESS;
  id: string;
  binding_id: string;
  iat: number;
  exp: number;
}

export interface MintDirectReadRequest {
  /** Tenancy account the token is scoped to (E's binding_id). */
  bindingId: string;
  /** Lifetime in seconds: required, positive, at most the cap. */
  ttlSeconds: number;
  /** Issuance time, epoch seconds. Injectable for tests. */
  now?: number;
}

function epochSeconds(now?: number): number {
  return now ?? Math.floor(Date.now() / 1000);
}

async function importSigningKey(pkcs8: Uint8Array): Promise<CryptoKey> {
  return crypto.subtle.importKey("pkcs8", pkcs8 as BufferSource, { name: "Ed25519" }, false, ["sign"]);
}

async function importVerifyKey(raw: Uint8Array): Promise<CryptoKey> {
  return crypto.subtle.importKey("raw", raw as BufferSource, { name: "Ed25519" }, false, ["verify"]);
}

/**
 * Mint a scoped, expiring, read-only token for the metadata DB.
 * Fails closed: unconfigured mint, a binding that is not a minted
 * binding_id, or a TTL outside (0, cap] each refuse with a named code
 * and no token is produced.
 */
export async function mintDirectReadToken(
  config: DirectReadConfig | null,
  request: MintDirectReadRequest,
): Promise<string> {
  if (config === null) {
    throw new FailClosed(
      503,
      "direct_read_not_configured",
      "no direct-read mint is configured (metadata/queue DB ids and signing material are required)",
    );
  }
  if (!BINDING_ID_RE.test(request.bindingId)) {
    throw new FailClosed(
      400,
      "invalid_binding_id",
      "a direct-read token is scoped to a tenancy binding_id; the supplied account is not one",
    );
  }
  const { ttlSeconds } = request;
  if (!Number.isSafeInteger(ttlSeconds) || ttlSeconds <= 0 || ttlSeconds > config.maxTtlSeconds) {
    throw new FailClosed(
      400,
      "invalid_ttl",
      `a direct-read token must expire: ttlSeconds must be a positive integer of at most ${config.maxTtlSeconds}`,
    );
  }
  const iat = epochSeconds(request.now);
  const claims: DirectReadClaims = {
    a: DIRECT_READ_ACCESS,
    id: config.metadataDbId,
    binding_id: request.bindingId,
    iat,
    exp: iat + ttlSeconds,
  };
  const enc = new TextEncoder();
  const header = { alg: "EdDSA", typ: "JWT", kid: config.signing.keyId };
  const signingInput = `${bytesToB64url(enc.encode(JSON.stringify(header)))}.${bytesToB64url(
    enc.encode(JSON.stringify(claims)),
  )}`;
  const key = await importSigningKey(config.signing.pkcs8);
  const signature = new Uint8Array(await crypto.subtle.sign({ name: "Ed25519" }, key, enc.encode(signingInput)));
  return `${signingInput}.${bytesToB64url(signature)}`;
}

export type DirectReadRefusal =
  | "malformed_token"
  | "no_verify_keys_configured"
  | "unknown_key"
  | "bad_signature"
  | "not_read_only"
  | "queue_database"
  | "wrong_database"
  | "missing_expiry"
  | "expired"
  | "missing_binding";

export type DirectReadInspection =
  | { status: "ok"; bindingId: string; expiresAt: number }
  | { status: "refused"; reason: DirectReadRefusal };

function refused(reason: DirectReadRefusal): DirectReadInspection {
  return { status: "refused", reason };
}

/**
 * Classify a presented direct-read token against the configured
 * scope. The signature is verified BEFORE any claim is trusted; the
 * claim checks then apply §3.3 in order: read-only, this DB only
 * (the queue DB is named separately because reaching for it is the
 * specific confusion the design forbids), expiry present and in the
 * future, and an account binding present. This is the conformance
 * oracle for the read path: a bulk consumer connects only on "ok".
 */
export async function inspectDirectReadToken(
  token: string,
  config: DirectReadConfig | null,
  now?: number,
): Promise<DirectReadInspection> {
  if (config === null || config.verifyKeys.size === 0) {
    return refused("no_verify_keys_configured");
  }
  let parsed;
  try {
    parsed = parseJwt(token);
  } catch {
    return refused("malformed_token");
  }
  if (parsed.header.alg !== "EdDSA") return refused("malformed_token");
  const kid = parsed.header.kid;
  const rawKey = typeof kid === "string" ? config.verifyKeys.get(kid) : undefined;
  if (rawKey === undefined) return refused("unknown_key");

  const key = await importVerifyKey(rawKey);
  const valid = await crypto.subtle
    .verify({ name: "Ed25519" }, key, parsed.signature as BufferSource, new TextEncoder().encode(parsed.signingInput))
    .catch(() => false);
  if (!valid) return refused("bad_signature");

  const { payload } = parsed;
  if (payload.a !== DIRECT_READ_ACCESS) return refused("not_read_only");
  if (payload.id === config.queueDbId) return refused("queue_database");
  if (payload.id !== config.metadataDbId) return refused("wrong_database");
  if (typeof payload.exp !== "number" || !Number.isSafeInteger(payload.exp)) {
    return refused("missing_expiry");
  }
  if (payload.exp <= epochSeconds(now)) return refused("expired");
  if (typeof payload.binding_id !== "string" || !BINDING_ID_RE.test(payload.binding_id)) {
    return refused("missing_binding");
  }
  return { status: "ok", bindingId: payload.binding_id, expiresAt: payload.exp };
}
