/**
 * Recommendation snapshot paging state machine (developing-today/code#10158,
 * slice C9139-01, V3 design §#9139 fanout and DO accumulator S6.4–S6.7, S6.15).
 *
 * At job start the parent snapshot inputs, ranking configuration and
 * candidate cutoff are pinned; the total order is materialized once:
 * score DESC, then immutable recommendation ID ASC. Later arrivals belong
 * to the next snapshot and cannot reorder current pages.
 *
 * Identity rules (must stay byte-identical to the fixture oracle in
 * `doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-c/`):
 * - `ParentSnapshotId` is SHA-256 over the deterministic-CBOR encoding of
 *   the complete signed parent envelope. The parent preimage describes
 *   inputs/ranking/cutoff and excludes page EnvelopeIds (no hash cycle).
 * - Each independently retained/transported page has its own signed
 *   envelope; its body binds `ParentSnapshotId`, page position and entries,
 *   and its `EnvelopeId` is SHA-256 over the complete signed page envelope
 *   including the signature bytes.
 * - A cursor binds only the pinned parent ID plus position (and optional
 *   expiry), never a page ID. Restart/replay at the same parent and
 *   position returns the same page envelope bytes.
 * - Wrong-parent and expired cursors reject. `done` is the terminal
 *   signal, computed independently of page emptiness: an empty page is
 *   terminal only when its position has consumed the whole order.
 *
 * Signature fields are deterministic fixture markers: this module performs
 * no cryptographic signature verification and claims no production state,
 * exactly as the oracle states.
 */

export interface RecommendationCandidate {
  id: string;
  score_micros: number;
  arrival_sequence: number;
}

export interface ParentBody {
  type: string;
  tenant: string;
  algorithm: string;
  cutoff_sequence: number;
  page_size: number;
  input_digest: string;
  candidate_set: RecommendationCandidate[];
}

export interface ParentEnvelope {
  body: ParentBody;
  /** Raw signature bytes (fixture markers; not verified). */
  signature: Uint8Array;
}

export interface PageEntry {
  id: string;
  score_micros: number;
  arrival_sequence: number;
}

export interface PageBody {
  parent_snapshot_id: string;
  position: number;
  entries: PageEntry[];
  done: boolean;
}

export interface PageEnvelope {
  body: PageBody;
  /** Raw signature bytes (fixture markers; not verified). */
  signature: Uint8Array;
}

export interface PageCursor {
  parent_snapshot_id: string;
  position: number;
  /** Unix epoch milliseconds; absent cursors never expire. */
  expires_at?: number;
}

/** A pinned snapshot: parent identity plus its materialized total order. */
export interface RecommendationSnapshot {
  parent: ParentEnvelope;
  parent_snapshot_id: string;
  ordered: PageEntry[];
  page_size: number;
}

export class WrongParentSnapshotIdError extends Error {
  readonly code = "WrongParentSnapshotId";
  constructor() {
    super("WrongParentSnapshotId");
    this.name = "WrongParentSnapshotIdError";
  }
}

export class CursorExpiredError extends Error {
  readonly code = "CursorExpired";
  constructor() {
    super("CursorExpired");
    this.name = "CursorExpiredError";
  }
}

export class InvalidPagePositionError extends Error {
  readonly code = "InvalidPagePosition";
  constructor(position: number) {
    super(`InvalidPagePosition: ${position}`);
    this.name = "InvalidPagePositionError";
  }
}

/** Decode a lowercase hex string to bytes. */
export function hexToBytes(hex: string): Uint8Array {
  if (hex.length % 2 !== 0) throw new Error("hex string must have even length");
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) {
    out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  }
  return out;
}

// --- Deterministic CBOR subset (mirrors recommendation-page-oracle.py) ---

type CborValue =
  | boolean
  | number
  | Uint8Array
  | string
  | CborValue[]
  | { [key: string]: CborValue };

function cborHead(major: number, value: number): Uint8Array {
  if (value < 24) return new Uint8Array([(major << 5) | value]);
  if (value < 256) return new Uint8Array([(major << 5) | 24, value]);
  if (value < 65536) {
    return new Uint8Array([(major << 5) | 25, value >> 8, value & 0xff]);
  }
  if (value < 2 ** 32) {
    return new Uint8Array([
      (major << 5) | 26,
      (value >>> 24) & 0xff,
      (value >>> 16) & 0xff,
      (value >>> 8) & 0xff,
      value & 0xff,
    ]);
  }
  const hi = Math.floor(value / 2 ** 32);
  const lo = value >>> 0;
  return new Uint8Array([
    (major << 5) | 27,
    (hi >>> 24) & 0xff,
    (hi >>> 16) & 0xff,
    (hi >>> 8) & 0xff,
    hi & 0xff,
    (lo >>> 24) & 0xff,
    (lo >>> 16) & 0xff,
    (lo >>> 8) & 0xff,
    lo & 0xff,
  ]);
}

function concat(parts: Uint8Array[]): Uint8Array {
  const total = parts.reduce((n, p) => n + p.length, 0);
  const out = new Uint8Array(total);
  let at = 0;
  for (const p of parts) {
    out.set(p, at);
    at += p.length;
  }
  return out;
}

function compareBytes(a: Uint8Array, b: Uint8Array): number {
  const n = Math.min(a.length, b.length);
  for (let i = 0; i < n; i++) {
    if (a[i] !== b[i]) return a[i] - b[i];
  }
  return a.length - b.length;
}

/**
 * Deterministic CBOR encoding for the fixture subset. Map keys sort by
 * (encoded-key length, encoded-key bytes), exactly as the oracle does.
 */
export function deterministicCbor(value: CborValue): Uint8Array {
  if (typeof value === "boolean") return new Uint8Array([value ? 0xf5 : 0xf4]);
  if (typeof value === "number") {
    if (!Number.isInteger(value)) throw new Error("CBOR subset supports integers only");
    return value >= 0 ? cborHead(0, value) : cborHead(1, -1 - value);
  }
  if (value instanceof Uint8Array) {
    return concat([cborHead(2, value.length), value]);
  }
  if (typeof value === "string") {
    const data = new TextEncoder().encode(value);
    return concat([cborHead(3, data.length), data]);
  }
  if (Array.isArray(value)) {
    return concat([cborHead(4, value.length), ...value.map(deterministicCbor)]);
  }
  const entries = Object.entries(value).map(([k, v]) => {
    const ek = deterministicCbor(k);
    return { ek, ev: deterministicCbor(v) };
  });
  entries.sort((a, b) => a.ek.length - b.ek.length || compareBytes(a.ek, b.ek));
  return concat([
    cborHead(5, entries.length),
    ...entries.flatMap(({ ek, ev }) => [ek, ev]),
  ]);
}

/** `sha256:` + hex(SHA-256(deterministic-CBOR(value))). */
export async function digestId(value: CborValue): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", deterministicCbor(value));
  return (
    "sha256:" +
    [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("")
  );
}

// --- Snapshot / paging state machine ---

function envelopeValue(envelope: { body: unknown; signature: Uint8Array }): CborValue {
  return { body: envelope.body as CborValue, signature: envelope.signature };
}

/** SHA-256 identity of the complete signed parent envelope. */
export async function parentSnapshotId(parent: ParentEnvelope): Promise<string> {
  return digestId(envelopeValue(parent));
}

/** SHA-256 identity of the complete signed page envelope. */
export async function pageEnvelopeId(page: PageEnvelope): Promise<string> {
  return digestId(envelopeValue(page));
}

/**
 * Pin the parent snapshot: materialize the total order once (score DESC,
 * then immutable recommendation ID ASC) over candidates at or before the
 * cutoff. Later arrivals are excluded; they belong to the next snapshot.
 */
export async function createSnapshot(parent: ParentEnvelope): Promise<RecommendationSnapshot> {
  const body = parent.body;
  const ordered = body.candidate_set
    .filter((c) => c.arrival_sequence <= body.cutoff_sequence)
    .map((c) => ({ id: c.id, score_micros: c.score_micros, arrival_sequence: c.arrival_sequence }))
    .sort((a, b) => b.score_micros - a.score_micros || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
  return {
    parent,
    parent_snapshot_id: await parentSnapshotId(parent),
    ordered,
    page_size: body.page_size,
  };
}

/**
 * Materialize the page at `position` in the pinned order. Positions must
 * be a multiple of the page size within [0, total]; anything else is an
 * `InvalidPagePositionError`. `done` is true exactly when this page
 * consumes the whole order (empty pages at the boundary are terminal).
 */
export async function makePage(
  snapshot: RecommendationSnapshot,
  position: number,
  signatureHex: string,
): Promise<{ envelope: PageEnvelope; envelope_id: string }> {
  const total = snapshot.ordered.length;
  if (position < 0 || position > total || position % snapshot.page_size !== 0) {
    throw new InvalidPagePositionError(position);
  }
  const entries = snapshot.ordered.slice(position, position + snapshot.page_size);
  const envelope: PageEnvelope = {
    body: {
      parent_snapshot_id: snapshot.parent_snapshot_id,
      position,
      entries,
      done: position + entries.length >= total,
    },
    signature: hexToBytes(signatureHex),
  };
  return { envelope, envelope_id: await pageEnvelopeId(envelope) };
}

/**
 * Replay a page from a cursor. The cursor binds only the pinned parent
 * ID plus position. A wrong parent rejects with
 * `WrongParentSnapshotIdError`; an expired cursor rejects with
 * `CursorExpiredError` and is never applied. Replay at the same parent
 * and position returns the same page envelope.
 */
export async function pageFromCursor(
  snapshot: RecommendationSnapshot,
  cursor: PageCursor,
  signatureHex: string,
  nowMs: number = Date.now(),
): Promise<{ envelope: PageEnvelope; envelope_id: string }> {
  if (cursor.parent_snapshot_id !== snapshot.parent_snapshot_id) {
    throw new WrongParentSnapshotIdError();
  }
  if (cursor.expires_at !== undefined && nowMs >= cursor.expires_at) {
    throw new CursorExpiredError();
  }
  return makePage(snapshot, cursor.position, signatureHex);
}

/** The cursor following a page: same parent, position advanced by entries. */
export function nextCursor(page: PageEnvelope): PageCursor {
  return {
    parent_snapshot_id: page.body.parent_snapshot_id,
    position: page.body.position + page.body.entries.length,
  };
}
