// Stamp signatures (#12586; design §4.3 layer 2, §5) — sign and verify
// the canonical provenance object of a Metadata block, so a reader can
// check that a block's provenance was minted by a fleet stamping
// authority and not hand-written or edited afterward. This module is
// the one verification code path for both editions: a native-stamped
// block and a hosted-stamped block verify through the same functions
// here (the collector / query path consumes `verifyStamp`'s result and
// reports it per #9060 §15: verified, diverged, or unsigned).
//
// WHAT IS SIGNED. The block's `provenance` object (the #9060 §12 stamp
// set — queue_id, session_id, lane, host, worktree, picked_up_at,
// work_started_at, created_at, model, lane_start_issues — plus the §5
// additive fields workspace_ref, auth_subject, stamped_by), with
// `stamp_sig` itself removed. Caller-owned block fields (direct_parent,
// links, lineage) live outside `provenance` and are never inputs, so a
// caller editing its own fields cannot invalidate — or forge — a stamp.
// Canonicalization covers every key present in the provenance object
// except `stamp_sig`, so a future additive fleet-owned field is signed
// automatically rather than silently left outside the signature.
//
// CANONICAL FORM. Deterministic JSON, JCS-shaped (RFC 8785) over the
// provenance value domain: object keys sorted by UTF-16 code unit, no
// whitespace, strings with JSON escaping, arrays in order, nulls
// preserved (a stamped null is signed as null; the stamp set is stamped
// in full, so absence and null are not interchangeable after signing).
// Numbers must be safe integers — provenance carries no fractional
// values, and an uncanonicalizable value refuses (fail closed) rather
// than risk two editions serializing it differently. The signed bytes
// are the UTF-8 encoding of the canonical string, nothing else: the
// stamp key is a dedicated key, used for no other protocol, and the
// stamp_sig value's version token separates stamp signatures from any
// other use of Ed25519 in the fleet.
//
// ALGORITHM + KEY CUSTODY — the build-start decision the design's §8.2
// deferral assigns to this implementation. The fleet-CA-certificate
// option is not implementable at this build start: the CA program
// (#9155) is design + synthetic proof only, production CA generation
// and signing are approval-blocked there, and its baseline scheme
// (ML-DSA-44) has no WebCrypto implementation, so the hosted verifier
// could not verify it and verification would stop being edition-blind.
// The design's other named option is therefore the v1 scheme:
// **Ed25519 with a dedicated service key**. Custody: the stamping
// authority holds the private key as configuration, referenced by name
// only (STAMP_SIGNING_KEY_PKCS8 / STAMP_SIGNING_KEY_ID, see config.ts);
// verifiers hold public keys by key id (STAMP_VERIFY_KEYS_JSON), and
// holding several key ids at once is the rotation window — a stamp
// names the key that made it. Unconfigured signing fails closed with
// the skeleton's 503 idiom; unconfigured verification reports
// `unverifiable`, never `verified`.
//
// stamp_sig VALUE FORMAT (v1): `v1.ed25519.<kid>.<sig>` where <kid> is
// the signing key id ([A-Za-z0-9_-]+) and <sig> is the base64url
// (unpadded) 64-byte Ed25519 signature over the canonical bytes. The
// native create path (#9073's signing hook) mints and checks this same
// format over this same canonical form; the golden vectors in
// test/stamp-sig.test.ts are its conformance fixtures (design §6's
// shared-fixture discipline: whichever edition lands second conforms
// to the first's fixtures).
//
// SEAM: this slice is the signature only. The create-path stamping
// endpoint (#12585) calls signStamp with the service's signer; the
// collector/query verification (#12587's surface) calls verifyStamp.
// Nothing here writes a block or a DB row.

import { FailClosed } from "./errors.ts";
import { isValidStampKeyId, type StampSigningMaterial } from "./config.ts";

export { isValidStampKeyId };

/** The provenance object of a Metadata block (fleet-owned fields). */
export type Provenance = Record<string, unknown>;

/** Outcome of verifying a block's provenance, per #9060 §15 reporting. */
export type StampVerification =
  | { status: "verified"; keyId: string }
  | { status: "unsigned" }
  | {
      status: "diverged";
      reason: "signature_mismatch" | "malformed_stamp_sig" | "unknown_key" | "unsupported_stamp_version";
    }
  | { status: "unverifiable"; reason: "no_verify_keys_configured" };

const SIG_VERSION = "v1";
const SIG_ALGORITHM = "ed25519";
const ED25519_SIGNATURE_BYTES = 64;
const MAX_DEPTH = 32;

// --- base64url (unpadded), the jwt.ts idiom kept local to this module.

export function bytesToB64url(bytes: Uint8Array): string {
  let bin = "";
  for (const b of bytes) bin += String.fromCharCode(b);
  return btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

export function b64urlToBytes(text: string): Uint8Array {
  const b64 = text.replace(/-/g, "+").replace(/_/g, "/");
  const padded = b64 + "=".repeat((4 - (b64.length % 4)) % 4);
  const bin = atob(padded);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

// --- Canonical form.

function canonicalizeValue(value: unknown, depth: number): string {
  if (depth > MAX_DEPTH) {
    throw new FailClosed(400, "uncanonicalizable_provenance", "provenance nesting too deep");
  }
  if (value === null) return "null";
  if (typeof value === "boolean") return value ? "true" : "false";
  if (typeof value === "number") {
    if (!Number.isSafeInteger(value)) {
      throw new FailClosed(400, "uncanonicalizable_provenance", "provenance numbers must be safe integers");
    }
    return String(value);
  }
  if (typeof value === "string") return JSON.stringify(value);
  if (Array.isArray(value)) {
    return `[${value.map((item) => canonicalizeValue(item, depth + 1)).join(",")}]`;
  }
  if (typeof value === "object") {
    const record = value as Record<string, unknown>;
    const keys = Object.keys(record).sort();
    const parts = keys.map((key) => {
      const v = record[key];
      if (v === undefined) {
        throw new FailClosed(400, "uncanonicalizable_provenance", `provenance field ${key} is undefined`);
      }
      return `${JSON.stringify(key)}:${canonicalizeValue(v, depth + 1)}`;
    });
    return `{${parts.join(",")}}`;
  }
  throw new FailClosed(
    400,
    "uncanonicalizable_provenance",
    `provenance value of type ${typeof value} has no canonical form`,
  );
}

/**
 * The canonical bytes a stamp signature covers: the provenance object
 * with `stamp_sig` removed, serialized deterministically. Identical
 * for a signed and an unsigned copy of the same provenance, and
 * independent of the key order the object was built or parsed with —
 * that is what makes verification edition-blind.
 */
export function canonicalProvenanceBytes(provenance: Provenance): Uint8Array {
  const { stamp_sig: _omitted, ...unsigned } = provenance;
  return new TextEncoder().encode(canonicalizeValue(unsigned, 0));
}

// --- Keys.

export interface StampSigner {
  keyId: string;
  privateKey: CryptoKey;
}

/** Import a signing key from its configured material (PKCS#8 bytes). */
export async function importStampSigner(material: StampSigningMaterial): Promise<StampSigner> {
  const privateKey = await crypto.subtle.importKey(
    "pkcs8",
    material.pkcs8 as BufferSource,
    { name: "Ed25519" },
    false,
    ["sign"],
  );
  return { keyId: material.keyId, privateKey };
}

/** Import verifier public keys (raw 32-byte Ed25519) by key id. */
export async function importStampVerifyKeys(
  material: ReadonlyMap<string, Uint8Array>,
): Promise<Map<string, CryptoKey>> {
  const out = new Map<string, CryptoKey>();
  for (const [keyId, raw] of material) {
    const key = await crypto.subtle.importKey("raw", raw as BufferSource, { name: "Ed25519" }, false, ["verify"]);
    out.set(keyId, key);
  }
  return out;
}

const signerCache = new WeakMap<StampSigningMaterial, Promise<StampSigner>>();

/**
 * The service's signer from configuration, importing once per material
 * object. Unconfigured signing fails closed (503, the skeleton idiom):
 * a stamping authority without a key must refuse, never stamp unsigned
 * and never invent a key.
 */
export function requireStampSigner(material: StampSigningMaterial | null): Promise<StampSigner> {
  if (material === null) {
    throw new FailClosed(503, "stamp_signing_not_configured", "no stamp signing key is configured for this deployment");
  }
  let cached = signerCache.get(material);
  if (!cached) {
    cached = importStampSigner(material);
    signerCache.set(material, cached);
  }
  return cached;
}

// --- Sign.

/**
 * Sign a provenance object and return a NEW object carrying the
 * `stamp_sig` (the input is not mutated). Any stamp_sig already present
 * is replaced: re-stamping is a new mint by the signing authority.
 */
export async function signStamp(provenance: Provenance, signer: StampSigner): Promise<Provenance> {
  const signature = await crypto.subtle.sign(
    { name: "Ed25519" },
    signer.privateKey,
    canonicalProvenanceBytes(provenance) as BufferSource,
  );
  const stampSig = [SIG_VERSION, SIG_ALGORITHM, signer.keyId, bytesToB64url(new Uint8Array(signature))].join(".");
  return { ...provenance, stamp_sig: stampSig };
}

// --- Verify.

interface ParsedStampSig {
  keyId: string;
  signature: Uint8Array;
}

function parseStampSig(value: unknown): ParsedStampSig | StampVerification {
  if (typeof value !== "string") {
    return { status: "diverged", reason: "malformed_stamp_sig" };
  }
  const parts = value.split(".");
  if (parts.length !== 4) {
    return { status: "diverged", reason: "malformed_stamp_sig" };
  }
  const [version, algorithm, keyId, sigText] = parts;
  if (version !== SIG_VERSION || algorithm !== SIG_ALGORITHM) {
    return { status: "diverged", reason: "unsupported_stamp_version" };
  }
  if (!isValidStampKeyId(keyId) || sigText === "") {
    return { status: "diverged", reason: "malformed_stamp_sig" };
  }
  let signature: Uint8Array;
  try {
    signature = b64urlToBytes(sigText);
  } catch {
    return { status: "diverged", reason: "malformed_stamp_sig" };
  }
  if (signature.length !== ED25519_SIGNATURE_BYTES) {
    return { status: "diverged", reason: "malformed_stamp_sig" };
  }
  return { keyId, signature };
}

/**
 * Verify a block's provenance — the one code path both editions'
 * stamps go through. Never throws on a bad stamp: a forged or edited
 * block is data to report (diverged), not an exception. Only a
 * provenance that cannot be canonicalized at all throws, because then
 * no verification statement about it is possible.
 */
export async function verifyStamp(
  provenance: Provenance,
  verifyKeys: ReadonlyMap<string, CryptoKey> | null,
): Promise<StampVerification> {
  const stampSig = provenance["stamp_sig"];
  if (stampSig === undefined || stampSig === null) {
    // Pre-signature legacy blocks are unsigned, not forgeries (#12586
    // acceptance): the collector reports them as unsigned.
    return { status: "unsigned" };
  }
  const parsed = parseStampSig(stampSig);
  if ("status" in parsed) return parsed;
  if (verifyKeys === null || verifyKeys.size === 0) {
    return { status: "unverifiable", reason: "no_verify_keys_configured" };
  }
  const key = verifyKeys.get(parsed.keyId);
  if (!key) {
    return { status: "diverged", reason: "unknown_key" };
  }
  const ok = await crypto.subtle.verify(
    { name: "Ed25519" },
    key,
    parsed.signature as BufferSource,
    canonicalProvenanceBytes(provenance) as BufferSource,
  );
  return ok ? { status: "verified", keyId: parsed.keyId } : { status: "diverged", reason: "signature_mismatch" };
}
