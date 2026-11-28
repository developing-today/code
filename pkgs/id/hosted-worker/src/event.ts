// The signed event: p2panda Header<ChainExt> as pkgs/id's chain.rs
// `decode` verifies it, and the bridge's store of record keeps the
// received bytes as the event. A receiver here does exactly what
// chain.rs does and nothing else — a refused event never becomes a
// stored one, and a stored event's bytes are the bytes its author
// signed (no re-encoding, no re-signing, ever).
//
// Layout (p2panda-core 0.7.1, operation/header.rs + operation/any.rs):
// a CBOR array
//   [version=1, verifying_key (32), signature (64), payload_size,
//    payload_hash (32, iff payload_size > 0), seq_num,
//    backlink (32, iff seq_num > 0), extensions (map)]
// The signature is Ed25519 over the canonical encoding of the same
// array with element [2] removed; the operation ID is BLAKE3 over
// the whole received bytes.
//
// The CBOR layer is the package's canonical-cbor.ts (the id-core
// profile), plus the two chain.rs rules that profile does not carry:
// the 512 declared-length limit cbor-core enforces on every array,
// map and string, and the NFC rule over every text string. Both are
// checked over the decoded tree in `checkTree`.
//
// Extension map (chain.rs `Wire`, deny_unknown_fields): keys
// domain, kind, group, after — always present (group may be null) —
// plus the payload key of the event's kind family, and no other key:
//   tag.set / tag.clear        → tag
//   document.put               → document
//   document.remove            → removes
//   note.create / note.revise  → note
//   note.link                  → link
//   session.checkpoint         → checkpoint
//   issuer.rotation            → rotation
//   chat.message.v1            → chat
//   chat.anchor.v1             → anchor
//   anything else              → no payload key
// The deep per-kind payload checks (tag op rules, document checks,
// note/link/checkpoint/rotation rules) are native-store rules that
// this edition does not re-implement: an event failing only those is
// a deliberate self-sabotage by its signer, and the hosted store
// treats the signed header as the event. This boundary is recorded
// in doc/2026-11-23T05-00-00Z_design_bridge_v1_implementation.

import { decodeCanonical, encodeCanonical, type CborValue } from "../../../hosted-primitives/src/canonical-cbor.ts";
import { blake3 } from "../../../hosted-primitives/src/blake3.ts";
import { bytesToHex, verifyEd25519, type IdKey } from "./ed25519.ts";

export const DOMAIN_TAG = "id.events.v1";
export const MAX_AFTER = 32;
export const MAX_KIND_BYTES = 64;
/** cbor-core's declared-length limit, enforced by chain.rs. */
export const MAX_DECLARED_LENGTH = 512;

/** Maximum sequence number the store can hold: SQLite keeps the
 * sequence as an integer and the range keys are 8-byte big-endian,
 * so the bridge refuses a sequence above the safe-integer limit
 * rather than lose precision on it. No real author reaches it. */
export const MAX_SEQ = Number.MAX_SAFE_INTEGER;

export class Malformed extends Error {}

export interface CheckpointInfo {
  /** The canonical encoding of the checkpoint's `header` value —
   * the session binding two checkpoints must share. */
  headerEncoding: Uint8Array;
  segmentSeq: bigint;
  prev: Uint8Array | null;
}

export interface SignedEvent {
  id: string;
  idBytes: Uint8Array;
  bytes: Uint8Array;
  author: Uint8Array;
  authorHex: string;
  /** The envelope's own Ed25519 signature (over the signing bytes),
   * carried verbatim: the §6 export mapping (export.ts) names it as
   * the object's signature — a stored event is never re-signed. */
  signature: Uint8Array;
  signatureHex: string;
  seq: number;
  backlink: Uint8Array | null;
  backlinkHex: string | null;
  payloadSize: bigint;
  payloadHash: Uint8Array | null;
  kind: string;
  group: Uint8Array | null;
  after: Uint8Array[];
  afterHex: string[];
  checkpoint: CheckpointInfo | null;
}

const PAYLOAD_KEYS = ["tag", "document", "removes", "note", "link", "checkpoint", "rotation", "chat", "anchor"];

const KIND_FAMILY: Record<string, string> = {
  "tag.set": "tag",
  "tag.clear": "tag",
  "document.put": "document",
  "document.remove": "removes",
  "note.create": "note",
  "note.revise": "note",
  "note.link": "link",
  "session.checkpoint": "checkpoint",
  "issuer.rotation": "rotation",
  "chat.message.v1": "chat",
  "chat.anchor.v1": "anchor",
};

function malformed(message: string): never {
  throw new Malformed(message);
}

function utf8Length(text: string): number {
  return new TextEncoder().encode(text).length;
}

function isNfc(text: string): boolean {
  return text.normalize("NFC") === text;
}

/** The chain.rs rules over the decoded tree that the id-core CBOR
 * profile does not carry: the 512 declared-length limit and NFC. */
function checkTree(value: CborValue): void {
  switch (value.kind) {
    case "bytes":
      if (value.value.length > MAX_DECLARED_LENGTH) {
        malformed("a byte string declares more than 512 bytes");
      }
      return;
    case "text":
      if (utf8Length(value.value) > MAX_DECLARED_LENGTH) {
        malformed("a text string declares more than 512 bytes");
      }
      if (!isNfc(value.value)) malformed("text is not NFC");
      return;
    case "array":
      if (value.value.length > MAX_DECLARED_LENGTH) {
        malformed("an array declares more than 512 items");
      }
      for (const item of value.value) checkTree(item);
      return;
    case "map":
      if (value.value.length > MAX_DECLARED_LENGTH) {
        malformed("a map declares more than 512 entries");
      }
      for (const [k, v] of value.value) {
        checkTree(k);
        checkTree(v);
      }
      return;
    default:
      return;
  }
}

function asBytes(value: CborValue | undefined): Uint8Array | null {
  return value && value.kind === "bytes" ? value.value : null;
}

function asText(value: CborValue | undefined): string | null {
  return value && value.kind === "text" ? value.value : null;
}

function asUint(value: CborValue | undefined): bigint | null {
  return value && value.kind === "uint" ? value.value : null;
}

function mapGet(map: CborValue, key: string): CborValue | undefined {
  if (map.kind !== "map") return undefined;
  for (const [k, v] of map.value) {
    if (k.kind === "text" && k.value === key) return v;
  }
  return undefined;
}

function asHash32(value: CborValue | undefined, field: string): Uint8Array {
  const bytes = asBytes(value);
  if (!bytes || bytes.length !== 32) malformed(`${field} must be a 32-byte hash`);
  return bytes;
}

/** The structural part of a checkpoint payload: its session binding,
 * segment number, and predecessor — the pieces ingest's dependency
 * rule needs. The deep checkpoint checks are native rules. */
function checkpointInfo(value: CborValue): CheckpointInfo {
  if (value.kind !== "map") malformed("checkpoint must be a map");
  const header = mapGet(value, "header");
  if (!header || header.kind !== "map") malformed("checkpoint header must be a map");
  const segment = mapGet(value, "segment");
  if (!segment || segment.kind !== "map") malformed("checkpoint segment must be a map");
  const segmentSeq = asUint(mapGet(segment, "segment_seq"));
  if (segmentSeq === null) malformed("checkpoint segment_seq must be an unsigned integer");
  const prevRaw = mapGet(segment, "prev_checkpoint");
  if (prevRaw === undefined) malformed("checkpoint segment is missing prev_checkpoint");
  let prev: Uint8Array | null = null;
  if (prevRaw.kind === "null") prev = null;
  else prev = asHash32(prevRaw, "checkpoint prev_checkpoint");
  return { headerEncoding: encodeCanonical(header), segmentSeq, prev };
}

interface EventParts {
  event: SignedEvent;
  author: Uint8Array;
  signature: Uint8Array;
  signingBytes: Uint8Array;
}

function parseEventParts(bytes: Uint8Array): EventParts {
  let root: CborValue;
  try {
    root = decodeCanonical(bytes);
  } catch (e) {
    malformed(e instanceof Error ? e.message : "CBOR decoding failed");
  }
  checkTree(root);
  if (root.kind !== "array" || root.value.length < 5 || root.value.length > 8) {
    malformed("the header must be a CBOR array of 5 to 8 items");
  }
  const items = root.value;
  const version = asUint(items[0]);
  if (version === null || version !== 1n) malformed("header version is not 1");
  const author = asBytes(items[1]);
  if (!author || author.length !== 32) malformed("verifying_key must be 32 bytes");
  const signature = asBytes(items[2]);
  if (!signature || signature.length !== 64) malformed("signature must be 64 bytes");
  const payloadSize = asUint(items[3]);
  if (payloadSize === null) malformed("payload_size must be an unsigned integer");
  let at = 4;
  let payloadHash: Uint8Array | null = null;
  if (payloadSize > 0n) {
    payloadHash = asHash32(items[at], "payload_hash");
    at += 1;
  }
  const seqRaw = asUint(items[at]);
  if (seqRaw === null) malformed("seq_num must be an unsigned integer");
  if (seqRaw > BigInt(MAX_SEQ)) malformed("sequence number is out of range");
  const seq = Number(seqRaw);
  at += 1;
  let backlink: Uint8Array | null = null;
  if (seq > 0) {
    backlink = asHash32(items[at], "backlink");
    at += 1;
  }
  const ext = items[at];
  at += 1;
  if (at !== items.length) malformed("excessive fields after the header");
  if (ext === undefined) malformed("the header has no extensions");

  if (ext.kind !== "map") malformed("extensions must be a map");
  const seenKeys = new Set<string>();
  for (const [k] of ext.value) {
    if (k.kind !== "text") malformed("extension keys must be text");
    if (seenKeys.has(k.value)) malformed(`extension key ${k.value} repeats`);
    seenKeys.add(k.value);
  }
  const known = new Set(["domain", "kind", "group", "after", ...PAYLOAD_KEYS]);
  for (const key of seenKeys) {
    if (!known.has(key)) malformed(`unknown extension key ${key}`);
  }
  for (const required of ["domain", "kind", "group", "after"]) {
    if (!seenKeys.has(required)) malformed(`extension is missing ${required}`);
  }
  const domain = asText(mapGet(ext, "domain"));
  if (domain !== DOMAIN_TAG) malformed(`domain tag ${domain} is not ${DOMAIN_TAG}`);
  const kind = asText(mapGet(ext, "kind"));
  if (kind === null) malformed("kind must be text");
  const kindBytes = utf8Length(kind);
  if (kindBytes < 1 || kindBytes > MAX_KIND_BYTES) {
    malformed(`kind is ${kindBytes} bytes, it must be 1 to ${MAX_KIND_BYTES} bytes`);
  }
  const groupRaw = mapGet(ext, "group");
  let group: Uint8Array | null = null;
  if (groupRaw !== undefined) {
    if (groupRaw.kind === "null") group = null;
    else group = asHash32(groupRaw, "group");
  }
  const afterRaw = mapGet(ext, "after");
  if (!afterRaw || afterRaw.kind !== "array") malformed("after must be an array");
  if (afterRaw.value.length > MAX_AFTER) {
    malformed(`after holds ${afterRaw.value.length} hashes, the most allowed is ${MAX_AFTER}`);
  }
  const after: Uint8Array[] = [];
  const afterSeen = new Set<string>();
  for (const item of afterRaw.value) {
    const hash = asHash32(item, "after");
    const hex = bytesToHex(hash);
    if (afterSeen.has(hex)) malformed("after names the same hash twice");
    afterSeen.add(hex);
    after.push(hash);
  }
  // One payload key per kind family, and no payload key for a kind
  // outside every family (chain.rs `ChainExt::validate`).
  const expected = KIND_FAMILY[kind] ?? null;
  for (const key of PAYLOAD_KEYS) {
    const present = seenKeys.has(key);
    if (key === expected && !present) malformed(`kind ${kind} needs its ${key} payload`);
    if (key !== expected && present) malformed(`kind ${kind} carries a ${key} payload`);
  }
  let checkpoint: CheckpointInfo | null = null;
  if (expected === "checkpoint") {
    checkpoint = checkpointInfo(mapGet(ext, "checkpoint") as CborValue);
  }
  if (expected === "removes") {
    asHash32(mapGet(ext, "removes"), "removes");
  }
  if (
    expected === "document" ||
    expected === "note" ||
    expected === "link" ||
    expected === "rotation" ||
    expected === "chat" ||
    expected === "anchor" ||
    expected === "tag"
  ) {
    const payload = mapGet(ext, expected);
    if (!payload || payload.kind !== "map") malformed(`${expected} payload must be a map`);
  }

  const idBytes = blake3(bytes);
  const event: SignedEvent = {
    id: bytesToHex(idBytes),
    idBytes,
    bytes,
    author,
    authorHex: bytesToHex(author),
    signature,
    signatureHex: bytesToHex(signature),
    seq,
    backlink,
    backlinkHex: backlink ? bytesToHex(backlink) : null,
    payloadSize,
    payloadHash,
    kind,
    group,
    after,
    afterHex: after.map(bytesToHex),
    checkpoint,
  };
  // The signature covers the header array without its signature
  // element; the canonical re-encoding of the remaining elements is
  // the signing bytes (p2panda AnyHeader::decode).
  const signingValue: CborValue = {
    kind: "array",
    value: [items[0], items[1], ...items.slice(3)],
  };
  return { event, author, signature, signingBytes: encodeCanonical(signingValue) };
}

/** Parses received event bytes and enforces every structural rule
 * of chain.rs `decode` except the signature check. The store
 * re-parses bytes it already holds with this — they were
 * signature-verified when they first arrived, and their bytes have
 * not changed since. Fresh arrivals go through `decodeEvent`. */
export function parseEvent(bytes: Uint8Array): SignedEvent {
  return parseEventParts(bytes).event;
}

/** Decodes and verifies received event bytes exactly as chain.rs
 * `decode` does: `parseEvent`'s structural rules plus the Ed25519
 * signature over the signing bytes. Throws `Malformed` with a named
 * reason; never returns a partial event. */
export async function decodeEvent(bytes: Uint8Array): Promise<SignedEvent> {
  const parts = parseEventParts(bytes);
  if (!(await verifyEd25519(parts.author, parts.signature, parts.signingBytes))) {
    malformed("signature does not verify");
  }
  return parts.event;
}

const cu = (v: bigint): CborValue => ({ kind: "uint", value: v });
const cb = (v: Uint8Array): CborValue => ({ kind: "bytes", value: v });
const ct = (v: string): CborValue => ({ kind: "text", value: v });

/** Signs a new event with the caller's ID key (the native
 * chain.rs `sign` shape): sequence, backlink, optional body, and the
 * extension. The caller composes `extra` payload pairs for the
 * kind's payload key. The result is re-decoded before return, so a
 * signer can never produce bytes the bridge itself refuses. */
export async function signEvent(
  key: IdKey,
  draft: {
    seq: number;
    backlink?: Uint8Array | null;
    body?: Uint8Array | null;
    kind: string;
    group?: Uint8Array | null;
    after?: Uint8Array[];
    extra?: [string, CborValue][];
  },
): Promise<SignedEvent> {
  if (!Number.isSafeInteger(draft.seq) || draft.seq < 0) {
    throw new Error("sequence number must be a non-negative safe integer");
  }
  const payloadSize = BigInt(draft.body?.length ?? 0);
  const payloadHash = draft.body ? blake3(draft.body) : null;
  const extPairs: [CborValue, CborValue][] = [
    [ct("domain"), ct(DOMAIN_TAG)],
    [ct("kind"), ct(draft.kind)],
    [ct("group"), draft.group ? cb(draft.group) : { kind: "null" }],
    [ct("after"), { kind: "array", value: (draft.after ?? []).map((h) => cb(h)) }],
  ];
  for (const [name, value] of draft.extra ?? []) extPairs.push([ct(name), value]);
  const ext: CborValue = { kind: "map", value: extPairs };
  const base: CborValue[] = [cu(1n), cb(key.publicRaw), cu(payloadSize)];
  if (payloadHash) base.push(cb(payloadHash));
  base.push(cu(BigInt(draft.seq)));
  if (draft.seq > 0) {
    if (!draft.backlink) throw new Error("sequence above 0 needs a backlink");
    base.push(cb(draft.backlink));
  }
  base.push(ext);
  const signingBytes = encodeCanonical({ kind: "array", value: base });
  const signature = await key.sign(signingBytes);
  const full = [base[0], base[1], cb(signature), ...base.slice(2)];
  const bytes = encodeCanonical({ kind: "array", value: full });
  return decodeEvent(bytes);
}
