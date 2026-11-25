// The fleet-kv blob upload door (#13318; design
// doc/2026-11-23T12-45-00Z_design_fleet_kv §8, §9, §10, D011, D012).
//
//   PUT /v1/kv/blobs?registry=R&namespace=NS&scope=S
//                   &jurisdiction=J&tier=T&digest=HEX&bytes_len=N
//
// with the value's exact bytes as the body. The pusher authenticates
// with an RP/agent JWT or a service token scoped to the `kv-upload`
// door, and may only push into a namespace scope it could write a row
// in: the namespace's shared scope, or (for a person or agent) one of
// its own private scopes. Then, in order:
//
//   stage    the body streams to a staging key while its length and
//            BLAKE3-256 are recomputed in chunks; nothing is buffered
//            whole
//   verify   a length or digest that differs from what the pusher
//            asserted is refused (422 blob_length_mismatch /
//            blob_digest_mismatch) and the staged bytes are dropped
//   publish  the verified bytes move to the partition's content key;
//            bytes already published there are deduplicated, never
//            crossing a (jurisdiction, tier) partition; then central
//            records the object as published (kv_blobs)
//
// The answer carries the encoded `KvBlobRefV1` the client then writes
// with a publish-CAS (fleet_kv::blob_central::publish_cas): only that
// CAS makes a key's value point at the blob. Unconfigured (no KV_BLOBS
// bucket or no kv primary) → 503, like every missing dependency.

import type { AuthDeps, CallerContext } from "./auth.ts";
import { Blake3Hasher, hex } from "./blake3.ts";
import { FailClosed } from "./errors.ts";
import { checkKvScopes, ensureKvMigrated, KV_CATALOG, privateScopePrefix, runKvTransaction } from "./kv-door.ts";

/** The registry fleet-kv's namespaces live in. */
export const KV_REGISTRY = "fleet-kv-central";

/** The object store under the door: the slice of R2 it uses. */
export interface KvBlobBucket {
  put(key: string, body: ReadableStream<Uint8Array>, length: number): Promise<void>;
  get(key: string): Promise<ReadableStream<Uint8Array> | null>;
  head(key: string): Promise<{ size: number } | null>;
  delete(key: string): Promise<void>;
}

/** The R2 binding's shape, as far as the door touches it. */
export interface R2BucketLike {
  put(key: string, value: ReadableStream<Uint8Array>): Promise<unknown>;
  get(key: string): Promise<{ body: ReadableStream<Uint8Array> } | null>;
  head(key: string): Promise<{ size: number } | null>;
  delete(key: string): Promise<void>;
}

/** An R2 binding as a KvBlobBucket. R2 needs a stream of known
 * length, which the Workers runtime's FixedLengthStream gives it. */
export function r2KvBlobBucket(bucket: R2BucketLike): KvBlobBucket {
  return {
    async put(key, body, length) {
      const Fixed = (
        globalThis as {
          FixedLengthStream?: new (length: number) => TransformStream<Uint8Array>;
        }
      ).FixedLengthStream;
      if (!Fixed) {
        await bucket.put(key, body);
        return;
      }
      const fixed = new Fixed(length);
      await Promise.all([body.pipeTo(fixed.writable), bucket.put(key, fixed.readable)]);
    },
    async get(key) {
      const object = await bucket.get(key);
      return object ? object.body : null;
    },
    head: (key) => bucket.head(key),
    delete: (key) => bucket.delete(key),
  };
}

/** An in-memory bucket for tests and local runs. */
export class MemoryKvBlobBucket implements KvBlobBucket {
  readonly objects = new Map<string, Uint8Array>();

  async put(key: string, body: ReadableStream<Uint8Array>, length: number): Promise<void> {
    const parts: Uint8Array[] = [];
    let total = 0;
    const reader = body.getReader();
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      parts.push(value);
      total += value.length;
    }
    if (total !== length) throw new Error(`stream length ${total} is not ${length}`);
    const bytes = new Uint8Array(total);
    let offset = 0;
    for (const part of parts) {
      bytes.set(part, offset);
      offset += part.length;
    }
    this.objects.set(key, bytes);
  }

  async get(key: string): Promise<ReadableStream<Uint8Array> | null> {
    const bytes = this.objects.get(key);
    if (!bytes) return null;
    return new ReadableStream({
      start(controller) {
        controller.enqueue(bytes.slice());
        controller.close();
      },
    });
  }

  async head(key: string): Promise<{ size: number } | null> {
    const bytes = this.objects.get(key);
    return bytes ? { size: bytes.length } : null;
  }

  async delete(key: string): Promise<void> {
    this.objects.delete(key);
  }
}

const LABEL = /^[a-z0-9-]{1,63}$/;
const DIGEST = /^[0-9a-f]{64}$/;
const DECIMAL = /^(0|[1-9][0-9]{0,15})$/;

/** The largest value one upload takes: R2's single-put ceiling. */
export const MAX_BLOB_BYTES = 5 * 1024 * 1024 * 1024 - 5 * 1024 * 1024;

export interface BlobUpload {
  registry: string;
  namespace: string;
  scope: string;
  jurisdiction: string;
  tier: string;
  digest: string;
  bytesLen: number;
}

function refused(code: string, message: string): never {
  throw new FailClosed(400, code, message);
}

/** Parse and check the upload's query. */
export function parseBlobUpload(url: URL): BlobUpload {
  const q = url.searchParams;
  const need = (name: string): string => {
    const value = q.get(name);
    if (value === null || value === "") refused("kv_blob_malformed", `missing ${name}`);
    return value;
  };
  const registry = q.get("registry") ?? KV_REGISTRY;
  const namespace = need("namespace");
  const scope = need("scope");
  const jurisdiction = need("jurisdiction");
  const tier = need("tier");
  const digest = need("digest");
  const bytesLenText = need("bytes_len");
  if (!LABEL.test(jurisdiction)) refused("kv_blob_malformed", "jurisdiction is not a label");
  if (!LABEL.test(tier)) refused("kv_blob_malformed", "tier is not a label");
  if (!DIGEST.test(digest)) refused("kv_blob_malformed", "digest is not 64 lower-case hex digits");
  if (!DECIMAL.test(bytesLenText)) refused("kv_blob_malformed", "bytes_len is not a canonical decimal");
  const bytesLen = Number(bytesLenText);
  if (bytesLen > MAX_BLOB_BYTES) {
    throw new FailClosed(413, "kv_blob_too_large", `one upload takes at most ${MAX_BLOB_BYTES} bytes`);
  }
  return { registry, namespace, scope, jurisdiction, tier, digest, bytesLen };
}

/** The encoded KvBlobRefV1, byte for byte as fleet_kv::blob encodes it. */
export function encodeKvBlobRef(ref: {
  digest: string;
  bytesLen: number;
  jurisdiction: string;
  tier: string;
  locator: string;
}): string {
  return [
    ["profile", "raw-blake3-256"],
    ["digest", ref.digest],
    ["bytes_len", String(ref.bytesLen)],
    ["jurisdiction", ref.jurisdiction],
    ["tier", ref.tier],
    ["locator", ref.locator],
    ["state", "published"],
  ]
    .map(([field, value]) => `${field}=${value}\n`)
    .join("");
}

/** The partition's content key for `digest`. */
export function publishedKey(jurisdiction: string, tier: string, digest: string): string {
  return `kv/blobs/${jurisdiction}/${tier}/${digest}`;
}

function stagingKey(upload: BlobUpload): string {
  const nonce = crypto.randomUUID();
  return `kv/staging/${upload.jurisdiction}/${upload.tier}/${upload.digest}/${nonce}`;
}

/** Counts and hashes bytes as they pass, refusing more than `limit`. */
function verifying(
  limit: number,
  hasher: Blake3Hasher,
  seen: { overflow: boolean },
): TransformStream<Uint8Array, Uint8Array> {
  return new TransformStream({
    transform(chunk, controller) {
      if (hasher.length + chunk.length > limit) {
        seen.overflow = true;
        controller.error(new FailClosed(422, "blob_length_mismatch", `the body is longer than ${limit} bytes`));
        return;
      }
      hasher.update(chunk);
      controller.enqueue(chunk);
    },
  });
}

function emptyStream(): ReadableStream<Uint8Array> {
  return new ReadableStream({
    start(controller) {
      controller.close();
    },
  });
}

/** The upload door's handler: authorize, stage, verify, publish. */
export async function handleKvBlobPut(req: Request, ctx: CallerContext, deps: AuthDeps): Promise<Response> {
  const bucket = deps.kvBlobs;
  if (!bucket) {
    throw new FailClosed(503, "kv_blobs_not_configured", "this deployment has no kv blob bucket configured");
  }
  const primary = deps.kvPrimary;
  if (!primary) {
    throw new FailClosed(503, "kv_primary_not_configured", "this deployment has no fleet-kv primary configured");
  }
  const upload = parseBlobUpload(new URL(req.url));
  const declared = req.headers.get("content-length");
  if (declared !== null && declared !== String(upload.bytesLen)) {
    throw new FailClosed(422, "blob_length_mismatch", "content-length differs from bytes_len");
  }
  // The pusher may only push where it could write a row (design §9).
  if (
    upload.scope.startsWith("private:") &&
    ctx.kind !== "service" &&
    !upload.scope.startsWith(privateScopePrefix(ctx.bindingId))
  ) {
    throw new FailClosed(403, "kv_scope_refused", "the private scope is not the caller's");
  }
  await ensureKvMigrated(primary);
  await checkKvScopes(primary, [
    {
      scope: upload.scope,
      registry: upload.registry,
      namespace: upload.namespace,
    },
  ]);

  // Stage while verifying: the staged bytes are invisible until publish.
  const hasher = new Blake3Hasher();
  const seen = { overflow: false };
  const staging = stagingKey(upload);
  const body = (req.body ?? emptyStream()).pipeThrough(verifying(upload.bytesLen, hasher, seen));
  try {
    await bucket.put(staging, body, upload.bytesLen);
  } catch (e) {
    await bucket.delete(staging).catch(() => undefined);
    if (e instanceof FailClosed) throw e;
    if (seen.overflow || hasher.length !== upload.bytesLen) {
      throw new FailClosed(422, "blob_length_mismatch", `received ${hasher.length} of ${upload.bytesLen} bytes`);
    }
    throw new FailClosed(502, "kv_blobs_failed", "the bucket refused the staged bytes");
  }
  if (hasher.length !== upload.bytesLen) {
    await bucket.delete(staging);
    throw new FailClosed(422, "blob_length_mismatch", `received ${hasher.length} of ${upload.bytesLen} bytes`);
  }
  if (hex(hasher.finalize()) !== upload.digest) {
    await bucket.delete(staging);
    throw new FailClosed(422, "blob_digest_mismatch", "the bytes do not hash to the asserted digest");
  }

  // Publish: dedup inside the partition only, then record centrally.
  const key = publishedKey(upload.jurisdiction, upload.tier, upload.digest);
  const existing = await bucket.head(key);
  if (!existing) {
    const staged = await bucket.get(staging);
    if (!staged) throw new FailClosed(502, "kv_blobs_failed", "the staged bytes vanished before publication");
    await bucket.put(key, staged, upload.bytesLen);
  }
  await bucket.delete(staging);
  const locator = `r2:${key}`;
  const publish = KV_CATALOG.statements.find((s) => s.name === "blob.publish");
  if (!publish) throw new FailClosed(500, "kv_catalog_incomplete", "the catalogue has no blob.publish");
  await runKvTransaction(primary, [
    {
      sql: publish.sql,
      args: [
        { type: "text", value: upload.jurisdiction },
        { type: "text", value: upload.tier },
        { type: "text", value: upload.digest },
        { type: "integer", value: String(upload.bytesLen) },
        { type: "text", value: locator },
        { type: "integer", value: String(Date.now()) },
      ],
    },
  ]);
  const reference = encodeKvBlobRef({
    digest: upload.digest,
    bytesLen: upload.bytesLen,
    jurisdiction: upload.jurisdiction,
    tier: upload.tier,
    locator,
  });
  return new Response(
    JSON.stringify({
      reference,
      digest: upload.digest,
      bytes_len: upload.bytesLen,
      jurisdiction: upload.jurisdiction,
      tier: upload.tier,
      locator,
      deduplicated: existing !== null,
    }),
    { status: 201, headers: { "content-type": "application/json" } },
  );
}

// --- reads: capability mint and capability-only read (design §9) ----------
//
//   POST /v1/kv/blobs/capability   {registry?, namespace, scope, key}
//
// A hash grants nothing. The reader first proves it may read the row:
// the same scope rule as a write (the namespace's shared scope, or its
// own private scope), then the key's live head must hold a published
// KvBlobRefV1 whose object is in the bucket. Only then is a five-minute
// capability minted, scoped to that one object and the read operation:
// an HMAC-SHA256 under the Worker's KV_BLOB_CAPABILITY_KEY over every
// field it names. Then
//
//   GET /v1/kv/blobs/read?jurisdiction&tier&digest&bytes_len&expires&mac
//
// serves the bytes under the capability alone, re-hashing them on the
// way out: a stored copy that no longer matches its digest ends the
// body with an error instead of completing, so it is never served as
// the value. No key configured → 503 on both routes.

/** How long a read capability lives. */
export const CAPABILITY_TTL_MS = 5 * 60 * 1000;

export interface BlobCapability {
  jurisdiction: string;
  tier: string;
  digest: string;
  bytesLen: number;
  expiresAtMs: number;
}

function capabilityBinding(cap: BlobCapability): Uint8Array {
  return new TextEncoder().encode(
    `fleet-kv-capability-v1\nread\n${cap.jurisdiction}\n${cap.tier}\n${cap.digest}\n${cap.bytesLen}\n${cap.expiresAtMs}`,
  );
}

async function capabilityKey(secret: string, usage: "sign" | "verify"): Promise<CryptoKey> {
  return crypto.subtle.importKey("raw", new TextEncoder().encode(secret), { name: "HMAC", hash: "SHA-256" }, false, [
    usage,
  ]);
}

/** The capability's MAC, lower-case hex. */
export async function signCapability(secret: string, cap: BlobCapability): Promise<string> {
  const key = await capabilityKey(secret, "sign");
  const mac = await crypto.subtle.sign("HMAC", key, capabilityBinding(cap) as BufferSource);
  return hex(new Uint8Array(mac));
}

/** Whether `mac` is this capability's, compared in constant time. */
export async function verifyCapability(secret: string, cap: BlobCapability, mac: string): Promise<boolean> {
  if (!DIGEST.test(mac)) return false;
  const signature = new Uint8Array(32);
  for (let i = 0; i < 32; i++) signature[i] = parseInt(mac.slice(i * 2, i * 2 + 2), 16);
  const key = await capabilityKey(secret, "verify");
  return crypto.subtle.verify("HMAC", key, signature as BufferSource, capabilityBinding(cap) as BufferSource);
}

/** A KvBlobRefV1 as fleet_kv::blob decodes it: every field, in order. */
export function decodeKvBlobRef(
  text: string,
): { digest: string; bytesLen: number; jurisdiction: string; tier: string; locator: string } | null {
  if (!text.endsWith("\n")) return null;
  const lines = text.slice(0, -1).split("\n");
  const fields = ["profile", "digest", "bytes_len", "jurisdiction", "tier", "locator", "state"];
  if (lines.length !== fields.length) return null;
  const values: string[] = [];
  for (let i = 0; i < fields.length; i++) {
    const prefix = `${fields[i]}=`;
    if (!lines[i].startsWith(prefix)) return null;
    values.push(lines[i].slice(prefix.length));
  }
  const [profile, digest, bytesLen, jurisdiction, tier, locator, state] = values;
  if (profile !== "raw-blake3-256" || state !== "published") return null;
  if (!DIGEST.test(digest) || !DECIMAL.test(bytesLen) || !LABEL.test(jurisdiction) || !LABEL.test(tier)) return null;
  // eslint-disable-next-line no-control-regex
  if (locator === "" || /[\u0000-\u001f\u007f]/.test(locator)) return null;
  return { digest, bytesLen: Number(bytesLen), jurisdiction, tier, locator };
}

function base64Text(b64: string): string {
  const bytes = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
  return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
}

/** The key's live head value as text, or null when absent or masked. */
async function liveHeadText(
  primary: NonNullable<AuthDeps["kvPrimary"]>,
  address: { scope: string; registry: string; namespace: string; key: string },
): Promise<string | null> {
  const read = KV_CATALOG.statements.find((s) => s.name === "head.read");
  if (!read) throw new FailClosed(500, "kv_catalog_incomplete", "the catalogue has no head.read");
  const [rows] = await runKvTransaction(primary, [
    {
      sql: read.sql,
      args: [
        { type: "text", value: address.scope },
        { type: "text", value: address.registry },
        { type: "text", value: address.namespace },
        { type: "text", value: address.key },
      ],
    },
  ]);
  const row = rows?.[0];
  if (!row) return null;
  const state = row[1] as { type?: string; value?: string } | undefined;
  const value = row[2] as { type?: string; base64?: string; value?: string } | undefined;
  if (state?.value !== "live" || !value) return null;
  try {
    if (value.type === "blob" && typeof value.base64 === "string") return base64Text(value.base64);
    if (value.type === "text" && typeof value.value === "string") return value.value;
  } catch {
    return null;
  }
  return null;
}

/** The capability mint's handler. */
export async function handleKvBlobCapability(req: Request, ctx: CallerContext, deps: AuthDeps): Promise<Response> {
  const secret = deps.kvBlobCapabilityKey;
  const bucket = deps.kvBlobs;
  const primary = deps.kvPrimary;
  if (!bucket || !secret) {
    throw new FailClosed(503, "kv_blobs_not_configured", "this deployment has no kv blob bucket or capability key");
  }
  if (!primary) {
    throw new FailClosed(503, "kv_primary_not_configured", "this deployment has no fleet-kv primary configured");
  }
  let body: Record<string, unknown>;
  try {
    const parsed: unknown = await req.json();
    if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) throw new Error("not an object");
    body = parsed as Record<string, unknown>;
  } catch {
    refused("kv_blob_malformed", "the body is not a JSON object");
  }
  const field = (name: string): string => {
    const value = body[name];
    if (typeof value !== "string" || value === "") refused("kv_blob_malformed", `missing ${name}`);
    return value;
  };
  const registry = typeof body.registry === "string" && body.registry !== "" ? body.registry : KV_REGISTRY;
  const namespace = field("namespace");
  const scope = field("scope");
  const key = field("key");
  if (scope.startsWith("private:") && ctx.kind !== "service" && !scope.startsWith(privateScopePrefix(ctx.bindingId))) {
    throw new FailClosed(403, "kv_scope_refused", "the private scope is not the caller's");
  }
  await ensureKvMigrated(primary);
  await checkKvScopes(primary, [{ scope, registry, namespace }]);
  const head = await liveHeadText(primary, { scope, registry, namespace, key });
  const reference = head === null ? null : decodeKvBlobRef(head);
  if (!reference) {
    throw new FailClosed(404, "no_live_reference", "the key's live value is not a blob reference");
  }
  if (!(await bucket.head(publishedKey(reference.jurisdiction, reference.tier, reference.digest)))) {
    throw new FailClosed(404, "kv_blob_not_found", "the referenced blob is not published");
  }
  const cap: BlobCapability = {
    jurisdiction: reference.jurisdiction,
    tier: reference.tier,
    digest: reference.digest,
    bytesLen: reference.bytesLen,
    expiresAtMs: Date.now() + CAPABILITY_TTL_MS,
  };
  const mac = await signCapability(secret, cap);
  const q = new URLSearchParams({
    jurisdiction: cap.jurisdiction,
    tier: cap.tier,
    digest: cap.digest,
    bytes_len: String(cap.bytesLen),
    expires: String(cap.expiresAtMs),
    mac,
  });
  return new Response(
    JSON.stringify({
      read_url: `/v1/kv/blobs/read?${q}`,
      expires_at_ms: cap.expiresAtMs,
      digest: cap.digest,
      bytes_len: cap.bytesLen,
      jurisdiction: cap.jurisdiction,
      tier: cap.tier,
    }),
    { status: 200, headers: { "content-type": "application/json" } },
  );
}

/** The capability-only read's handler. */
export async function handleKvBlobRead(req: Request, deps: AuthDeps, now: () => number = Date.now): Promise<Response> {
  const secret = deps.kvBlobCapabilityKey;
  const bucket = deps.kvBlobs;
  if (!bucket || !secret) {
    throw new FailClosed(503, "kv_blobs_not_configured", "this deployment has no kv blob bucket or capability key");
  }
  const q = new URL(req.url).searchParams;
  const get = (name: string): string => q.get(name) ?? "";
  const cap: BlobCapability = {
    jurisdiction: get("jurisdiction"),
    tier: get("tier"),
    digest: get("digest"),
    bytesLen: DECIMAL.test(get("bytes_len")) ? Number(get("bytes_len")) : -1,
    expiresAtMs: DECIMAL.test(get("expires")) ? Number(get("expires")) : -1,
  };
  if (cap.bytesLen < 0 || cap.expiresAtMs < 0 || !(await verifyCapability(secret, cap, get("mac")))) {
    throw new FailClosed(401, "capability_invalid", "the capability does not verify");
  }
  if (now() >= cap.expiresAtMs) {
    throw new FailClosed(401, "capability_expired", "the capability has expired");
  }
  const stored = await bucket.get(publishedKey(cap.jurisdiction, cap.tier, cap.digest));
  if (!stored) throw new FailClosed(404, "kv_blob_not_found", "the blob is not published");
  const hasher = new Blake3Hasher();
  const checked = stored.pipeThrough(
    new TransformStream<Uint8Array, Uint8Array>({
      transform(chunk, controller) {
        hasher.update(chunk);
        if (hasher.length > cap.bytesLen) {
          controller.error(new Error("blob_length_mismatch"));
          return;
        }
        controller.enqueue(chunk);
      },
      flush(controller) {
        if (hasher.length !== cap.bytesLen) controller.error(new Error("blob_length_mismatch"));
        else if (hex(hasher.finalize()) !== cap.digest) controller.error(new Error("blob_digest_mismatch"));
      },
    }),
  );
  return new Response(checked, {
    status: 200,
    headers: {
      "content-type": "application/octet-stream",
      "content-length": String(cap.bytesLen),
      "cache-control": "no-store",
    },
  });
}
