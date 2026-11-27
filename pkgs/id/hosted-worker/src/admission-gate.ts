// Universal signed-and-validated admission gate for both cores
// (slice A9120-signature-gate, #10275; design part A, cross-cutting
// invariants 1, 6 and 7 — statements S1.12–S1.20, S3.4–S3.10,
// S4.19–S4.28).
//
// TypeScript mirror of crates/id-core/src/admission_gate.rs: the
// same pipeline, the same stage order, the same typed rejection
// classes, so an admission decision is identical across the Rust
// core and this hosted worker. Every admission path passes this
// gate before any state transition is accepted; local producers
// sign before entering — there is no unsigned-local exception.
//
// Pipeline: (1) semantic validation of the canonical record,
// (2) signature and payload verification (digest recomputed on
// receipt), (3) signer binding against the directory and the
// authenticated principal, (4) namespace/tenant authorization
// through the injected seam, (5) policy (admitted domains,
// payload budget).
//
// Profile dispatch mirrors the Rust compiled-verifier registry:
// this build compiles the atlas-chunk-sha256-utf8 verifier
// (WebCrypto SHA-256). A payload ID under raw-blake3-256 — or any
// unknown profile — is refused `unsupported`, exactly as a Rust
// node whose dispatch excludes the profile refuses it; the Rust
// core remains the authority for blake3 admissions.

import {
  CborError,
  decodeCanonical,
  encodeCanonical,
  type CborValue,
} from "../../../hosted-primitives/src/canonical-cbor.ts";

/** The stable failure class of a rejection (mirrors the Rust `class()`). */
export type AdmissionRejectionClass =
  | "malformed"
  | "unsupported"
  | "payload"
  | "bad_signature"
  | "signer_binding"
  | "unauthorized"
  | "policy";

/** Typed admission refusal; `rejectionClass` names the gate stage. */
export class AdmissionRejection extends Error {
  readonly rejectionClass: AdmissionRejectionClass;
  constructor(rejectionClass: AdmissionRejectionClass, message: string) {
    super(message);
    this.rejectionClass = rejectionClass;
  }
}

const malformed = (message: string) => new AdmissionRejection("malformed", message);

/** Longest admitted context field, in bytes (mirrors the Rust bound). */
export const MAX_CONTEXT_BYTES = 255;

/**
 * The authenticated tenant/scope context an admission is judged
 * against (the #9083 contract shape, mirroring A9116's mutation
 * context field for field). Authentication happens outside the
 * gate; the gate consumes its result.
 */
export interface AdmissionContext {
  principal: string;
  tenantScope: string;
  authorizationScope: string;
}

function checkContextField(field: string, text: string): void {
  const bytes = new TextEncoder().encode(text);
  const visible = [...text].every((ch) => {
    const code = ch.codePointAt(0) ?? 0;
    return code >= 0x21 && code <= 0x7e;
  });
  if (bytes.length === 0 || bytes.length > MAX_CONTEXT_BYTES || !visible) {
    throw malformed(`context field ${field}: is not 1 to 255 visible ASCII characters`);
  }
}

/** A validated admission context; throws `malformed` on a bad field. */
export function admissionContext(principal: string, tenantScope: string, authorizationScope: string): AdmissionContext {
  checkContextField("principal", principal);
  checkContextField("tenant_scope", tenantScope);
  checkContextField("authorization_scope", authorizationScope);
  return { principal, tenantScope, authorizationScope };
}

/** Gate policy: admitted signing domains and the payload budget. */
export interface GatePolicy {
  /** Admitted domains; an empty or absent set admits every domain. */
  admittedDomains?: ReadonlySet<string>;
  /** Payload budget in bytes; defaults to the Rust default content budget. */
  maxPayloadBytes?: number;
}

const DEFAULT_MAX_PAYLOAD_BYTES = 64 * 2 ** 30;

/**
 * Signer binding source: resolves a signer identity to the key
 * registered for it. Unknown signers and key mismatches refuse.
 */
export interface SignerDirectory {
  keyFor(signerId: string): Uint8Array | undefined;
}

/** A {@link SignerDirectory} over a fixed registration table. */
export class StaticSignerDirectory implements SignerDirectory {
  readonly #keys = new Map<string, Uint8Array>();

  register(signerId: string, key: Uint8Array): void {
    this.#keys.set(signerId, new Uint8Array(key));
  }

  keyFor(signerId: string): Uint8Array | undefined {
    return this.#keys.get(signerId);
  }
}

/** The body fields the authorization seam judges. */
export interface BodyView {
  domain: string;
  context: string;
  signerId: string;
}

/**
 * Namespace/tenant authorization seam. Implementations judge the
 * authenticated context against the body's requested scope and
 * throw an `unauthorized` {@link AdmissionRejection} to refuse.
 */
export interface AdmissionAuthorizer {
  authorize(context: AdmissionContext, body: BodyView): void;
}

/**
 * The shipped authorizer: the body's context (its requested
 * namespace/tenant scope) must equal the authenticated tenant
 * scope exactly (invariant 6).
 */
export class ScopeBindingAuthorizer implements AdmissionAuthorizer {
  authorize(context: AdmissionContext, body: BodyView): void {
    if (body.context !== context.tenantScope) {
      throw new AdmissionRejection(
        "unauthorized",
        `body context ${JSON.stringify(body.context)} is not the authenticated tenant scope ${JSON.stringify(context.tenantScope)}`,
      );
    }
  }
}

/** Proof of admission, minted only by the gate after every check. */
export interface AdmissionReceipt {
  signerId: string;
  tenantScope: string;
  payloadLength: number;
  payloadProfile: string;
  /** Hex digest of the payload under its profile. */
  payloadDigestHex: string;
  /** Hex SHA-256 of the canonical envelope bytes (this build's receipt digest). */
  envelopeDigestHex: string;
}

const SIGNATURE_DOMAIN_PREFIX = new TextEncoder().encode("id/signed-body/v1\0");
const SUITE_ED25519 = "ed25519";
const BODY_VERSION = 1n;
const MAX_LABEL_BYTES = 256;
const PROFILE_ATLAS_SHA256_UTF8 = "atlas-chunk-sha256-utf8";

const BODY_FIELDS = new Set([
  "v",
  "domain",
  "context",
  "subject",
  "graph",
  "predecessor",
  "signer_id",
  "key",
  "suite",
  "payload_id",
  "payload_length",
]);

type MapEntries = [CborValue, CborValue][];

function mapField(entries: MapEntries, name: string): CborValue | undefined {
  for (const [key, value] of entries) {
    if (key.kind === "text" && key.value === name) return value;
  }
  return undefined;
}

function requireMap(value: CborValue, what: string): MapEntries {
  if (value.kind !== "map") throw malformed(`${what}: is not a map`);
  return value.value;
}

function requireText(value: CborValue | undefined, field: string): string {
  if (value === undefined || value.kind !== "text") {
    throw malformed(`field ${field}: missing or not text`);
  }
  return value.value;
}

function requireBytes(value: CborValue | undefined, field: string): Uint8Array {
  if (value === undefined || value.kind !== "bytes") {
    throw malformed(`field ${field}: missing or not bytes`);
  }
  return value.value;
}

function requireUint(value: CborValue | undefined, field: string): bigint {
  if (value === undefined || value.kind !== "uint") {
    throw malformed(`field ${field}: missing or not an unsigned integer`);
  }
  return value.value;
}

function checkLabel(field: string, label: string): void {
  const bytes = new TextEncoder().encode(label);
  if (bytes.length === 0) throw malformed(`field ${field}: must not be empty`);
  if (bytes.length > MAX_LABEL_BYTES) throw malformed(`field ${field}: is too long`);
}

function bytesEqual(a: Uint8Array, b: Uint8Array): boolean {
  if (a.length !== b.length) return false;
  return a.every((byte, i) => byte === b[i]);
}

function toHex(data: Uint8Array): string {
  return [...data].map((b) => b.toString(16).padStart(2, "0")).join("");
}

interface ParsedEnvelope {
  bodyValue: CborValue;
  domain: string;
  context: string;
  signerId: string;
  key: Uint8Array;
  suite: string;
  payloadProfile: string;
  payloadDigest: Uint8Array;
  payloadLength: bigint;
  payload: Uint8Array;
  signature: Uint8Array;
}

/** Strict structural extraction, mirroring the Rust `Fields` discipline. */
function parseEnvelope(decoded: CborValue): ParsedEnvelope {
  const envelope = requireMap(decoded, "envelope");
  for (const [key] of envelope) {
    if (key.kind === "text" && !["body", "payload", "signature"].includes(key.value)) {
      throw malformed(`envelope: unknown field ${key.value}`);
    }
  }
  const bodyValue = mapField(envelope, "body");
  if (bodyValue === undefined) throw malformed("field body: missing");
  const body = requireMap(bodyValue, "body");
  for (const [key] of body) {
    if (key.kind === "text" && !BODY_FIELDS.has(key.value)) {
      throw malformed(`body: unknown field ${key.value}`);
    }
  }
  const version = requireUint(mapField(body, "v"), "v");
  if (version !== BODY_VERSION) {
    throw new AdmissionRejection("unsupported", `unsupported body version ${version}`);
  }
  const payloadId = requireMap(
    (() => {
      const value = mapField(body, "payload_id");
      if (value === undefined) throw malformed("field payload_id: missing");
      return value;
    })(),
    "payload_id",
  );
  return {
    bodyValue,
    domain: requireText(mapField(body, "domain"), "domain"),
    context: requireText(mapField(body, "context"), "context"),
    signerId: requireText(mapField(body, "signer_id"), "signer_id"),
    key: requireBytes(mapField(body, "key"), "key"),
    suite: requireText(mapField(body, "suite"), "suite"),
    payloadProfile: requireText(mapField(payloadId, "profile"), "payload_id.profile"),
    payloadDigest: requireBytes(mapField(payloadId, "digest"), "payload_id.digest"),
    payloadLength: requireUint(mapField(body, "payload_length"), "payload_length"),
    payload: requireBytes(mapField(envelope, "payload"), "payload"),
    signature: requireBytes(mapField(envelope, "signature"), "signature"),
  };
}

/** The exact bytes the body signature covers (mirrors the Rust preimage). */
function signaturePreimage(bodyValue: CborValue, domain: string): Uint8Array {
  const encoded = encodeCanonical(bodyValue);
  const domainBytes = new TextEncoder().encode(domain);
  const out = new Uint8Array(SIGNATURE_DOMAIN_PREFIX.length + 4 + domainBytes.length + encoded.length);
  let pos = 0;
  out.set(SIGNATURE_DOMAIN_PREFIX, pos);
  pos += SIGNATURE_DOMAIN_PREFIX.length;
  new DataView(out.buffer).setUint32(pos, domainBytes.length, false);
  pos += 4;
  out.set(domainBytes, pos);
  pos += domainBytes.length;
  out.set(encoded, pos);
  return out;
}

/** The admission gate. One gate, every path. */
export class AdmissionGate {
  readonly #policy: GatePolicy;
  readonly #directory: SignerDirectory;
  readonly #authorizer: AdmissionAuthorizer;

  constructor(policy: GatePolicy, directory: SignerDirectory, authorizer: AdmissionAuthorizer) {
    this.#policy = policy;
    this.#directory = directory;
    this.#authorizer = authorizer;
  }

  /**
   * Runs the full admission pipeline over canonical envelope
   * bytes. Resolves with the receipt on acceptance; rejects with
   * an {@link AdmissionRejection} naming the failing stage.
   */
  async admitCanonical(bytes: Uint8Array, context: AdmissionContext): Promise<AdmissionReceipt> {
    // Stage 1 — semantic validation: canonical decode, strict
    // field sets, label bounds, canonical round trip.
    let decoded: CborValue;
    try {
      decoded = decodeCanonical(bytes);
    } catch (error) {
      if (error instanceof CborError) throw malformed(`envelope encoding: ${error.message}`);
      throw error;
    }
    const parsed = parseEnvelope(decoded);
    checkLabel("domain", parsed.domain);
    checkLabel("context", parsed.context);
    if (!bytesEqual(encodeCanonical(decoded), bytes)) {
      throw malformed("envelope: is not in canonical form");
    }
    if (parsed.key.length !== 32) throw malformed("field key: has the wrong length");
    if (parsed.signature.length !== 64) {
      throw malformed("field signature: has the wrong length");
    }
    if (parsed.payloadDigest.length !== 32) {
      throw malformed("field payload_id.digest: has the wrong length");
    }

    // Stage 2 — signature and payload verification.
    if (parsed.suite !== SUITE_ED25519) {
      throw new AdmissionRejection("unsupported", `unsupported signature suite ${JSON.stringify(parsed.suite)}`);
    }
    if (BigInt(parsed.payload.length) !== parsed.payloadLength) {
      throw new AdmissionRejection(
        "payload",
        `payload is ${parsed.payload.length} bytes but the body commits ${parsed.payloadLength}`,
      );
    }
    await this.#verifyPayloadDigest(parsed);
    await this.#verifySignature(parsed);

    // Stage 3 — signer binding.
    const registered = this.#directory.keyFor(parsed.signerId);
    if (registered === undefined) {
      throw new AdmissionRejection(
        "signer_binding",
        `signer ${JSON.stringify(parsed.signerId)}: signer is not registered in the directory`,
      );
    }
    if (!bytesEqual(registered, parsed.key)) {
      throw new AdmissionRejection(
        "signer_binding",
        `signer ${JSON.stringify(parsed.signerId)}: registered key does not match the body key`,
      );
    }
    if (context.principal !== parsed.signerId) {
      throw new AdmissionRejection(
        "signer_binding",
        `signer ${JSON.stringify(parsed.signerId)}: the authenticated principal is not the signer`,
      );
    }

    // Stage 4 — namespace/tenant authorization (injected seam).
    this.#authorizer.authorize(context, {
      domain: parsed.domain,
      context: parsed.context,
      signerId: parsed.signerId,
    });

    // Stage 5 — policy: admitted domain and payload budget.
    const admitted = this.#policy.admittedDomains;
    if (admitted !== undefined && admitted.size > 0 && !admitted.has(parsed.domain)) {
      throw new AdmissionRejection("policy", `domain ${JSON.stringify(parsed.domain)} is not admitted by policy`);
    }
    const maxPayload = this.#policy.maxPayloadBytes ?? DEFAULT_MAX_PAYLOAD_BYTES;
    if (parsed.payloadLength > BigInt(maxPayload)) {
      throw new AdmissionRejection(
        "policy",
        `payload of ${parsed.payloadLength} bytes exceeds the policy maximum of ${maxPayload} bytes`,
      );
    }

    const envelopeDigest = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes as BufferSource));
    return {
      signerId: parsed.signerId,
      tenantScope: parsed.context,
      payloadLength: parsed.payload.length,
      payloadProfile: parsed.payloadProfile,
      payloadDigestHex: toHex(parsed.payloadDigest),
      envelopeDigestHex: toHex(envelopeDigest),
    };
  }

  async #verifyPayloadDigest(parsed: ParsedEnvelope): Promise<void> {
    if (parsed.payloadProfile === PROFILE_ATLAS_SHA256_UTF8) {
      // The atlas profile is defined over UTF-8 text.
      try {
        new TextDecoder("utf-8", { fatal: true }).decode(parsed.payload);
      } catch {
        throw new AdmissionRejection("payload", "atlas-chunk-sha256-utf8 is defined over UTF-8 text");
      }
      const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", parsed.payload as BufferSource));
      if (!bytesEqual(digest, parsed.payloadDigest)) {
        throw new AdmissionRejection("payload", "payload digest does not match the payload ID");
      }
      return;
    }
    // raw-blake3-256 has no compiled verifier in this build; an
    // unknown profile is unsupported on any build.
    throw new AdmissionRejection("unsupported", `unsupported content profile ${JSON.stringify(parsed.payloadProfile)}`);
  }

  async #verifySignature(parsed: ParsedEnvelope): Promise<void> {
    let key: CryptoKey;
    try {
      key = await crypto.subtle.importKey("raw", parsed.key as BufferSource, "Ed25519", false, ["verify"]);
    } catch {
      throw new AdmissionRejection(
        "signer_binding",
        `signer ${JSON.stringify(parsed.signerId)}: the body key is not a valid Ed25519 public key`,
      );
    }
    const ok = await crypto.subtle.verify(
      "Ed25519",
      key,
      parsed.signature as BufferSource,
      signaturePreimage(parsed.bodyValue, parsed.domain) as BufferSource,
    );
    if (!ok) throw new AdmissionRejection("bad_signature", "signature does not verify");
  }
}
