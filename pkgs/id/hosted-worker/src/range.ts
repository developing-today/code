// Range reconciliation over the sync set: a transcription of
// crates/id-events' event_store::range (the /id-range-sync/1
// design) onto the bridge's wire. Two peers exchange compact range
// entries — skip, fingerprint, id lists — and each learns exactly
// the set difference without either side assigning order or
// vouching for content. Keys are (author, sequence, id); a range's
// fingerprint is the XOR of the first 16 bytes of its ids, plus a
// count.
//
// The split fan-out K and the fingerprint threshold T are local
// policy and never travel; two peers with different values still
// converge. Frames are bounded and budgeted: a session that would
// spend past its budget ends instead of growing without limit.
//
// Oracle: range.rs itself — its `simulate` loop and `check_bounds`
// are transcribed below, and test/bridge-range.test.ts drives
// random set pairs to exact symmetric-difference agreement.

import { bytesToHex, hexToBytes } from "./ed25519.ts";

export const KEY_LEN = 72;
export const FRAME_CAP = 16 * 1024 * 1024;
const MIN_FRAME_BUDGET = 256;
const FRAME_HEADER_RESERVE = 6;
/** range.rs ENTRY_CHARGE (size_of::<Entry>() + KEY_LEN + 16): what a
 * decoded entry costs against the session budget on top of its wire
 * bytes, so many tiny entries cannot undercharge the session. */
export const ENTRY_CHARGE = 152;

export interface RangeParams {
  k: number;
  t: number;
  frameBudget: number;
  sessionBudget: number;
}

export const DEFAULT_RANGE_PARAMS: RangeParams = {
  k: 16,
  t: 16,
  frameBudget: 4 * 1024 * 1024,
  sessionBudget: 64 * 1024 * 1024,
};

export function checkParams(params: RangeParams): RangeParams {
  if (params.k < 2) throw new Error(`K must be at least 2, or a split makes no progress (got ${params.k})`);
  if (params.t < 1)
    throw new Error(`T must be at least 1, or one mismatched key never ends the sync (got ${params.t})`);
  if (params.frameBudget < MIN_FRAME_BUDGET || params.frameBudget > FRAME_CAP) {
    throw new Error(
      `the frame budget must be from ${MIN_FRAME_BUDGET} bytes to the ${FRAME_CAP}-byte frame cap (got ${params.frameBudget})`,
    );
  }
  if (params.sessionBudget < MIN_FRAME_BUDGET) {
    throw new Error(`the session budget must be at least ${MIN_FRAME_BUDGET} bytes (got ${params.sessionBudget})`);
  }
  return params;
}

export function idsPerEntry(params: RangeParams): number {
  return Math.max(1, Math.floor(params.frameBudget / 64));
}

export interface SyncKey {
  author: string;
  seq: number;
  id: string;
}

/** Canonical byte order: lexicographic, shorter first on a prefix. */
export function compareBytes(a: Uint8Array, b: Uint8Array): number {
  const n = Math.min(a.length, b.length);
  for (let i = 0; i < n; i++) {
    if (a[i] !== b[i]) return a[i] - b[i];
  }
  return a.length - b.length;
}

export function keyBytes(key: SyncKey): Uint8Array {
  const out = new Uint8Array(KEY_LEN);
  out.set(hexToBytes(key.author), 0);
  let seq = BigInt(key.seq);
  for (let i = 7; i >= 0; i--) {
    out[32 + i] = Number(seq & 0xffn);
    seq >>= 8n;
  }
  out.set(hexToBytes(key.id), 40);
  return out;
}

export function keyFromBytes(bytes: Uint8Array): SyncKey {
  let seq = 0n;
  for (let i = 0; i < 8; i++) seq = (seq << 8n) | BigInt(bytes[32 + i]);
  return {
    author: bytesToHex(bytes.subarray(0, 32)),
    seq: Number(seq),
    id: bytesToHex(bytes.subarray(40, 72)),
  };
}

export type EntryMode =
  | { t: "skip" }
  | { t: "fp"; fp: Uint8Array; count: number }
  | { t: "ids"; ids: string[]; reply: boolean };

export interface RangeEntry {
  /** Exclusive upper bound as a key prefix; null is +infinity. */
  upper: Uint8Array | null;
  mode: EntryMode;
}

export function allSkip(entries: RangeEntry[]): boolean {
  return entries.every((e) => e.mode.t === "skip");
}

/** A whole flight's bounds: each finite bound is 1..=72 bytes and
 * strictly increases; only the last entry may be +infinity. */
export function checkBounds(entries: RangeEntry[]): void {
  let prev: Uint8Array | null = null;
  let ended = false;
  for (const entry of entries) {
    if (ended) throw new Error("an entry follows +infinity");
    if (entry.upper === null) {
      ended = true;
    } else {
      if (entry.upper.length < 1 || entry.upper.length > KEY_LEN) {
        throw new Error(`a finite bound must contain 1..=${KEY_LEN} bytes (got ${entry.upper.length})`);
      }
      if (prev !== null && compareBytes(entry.upper, prev) <= 0) {
        throw new Error("bounds do not increase");
      }
      prev = entry.upper;
    }
  }
}

// ----- the party -----------------------------------------------------

export class RangeParty {
  private keys: SyncKey[] = [];
  private keyRaw: Uint8Array[] = [];
  private ids: string[] = [];
  private xor: Uint8Array[] = [];

  constructor(entries: SyncKey[]) {
    // Encode each key once: keyBytes parses hex and walks a BigInt, so
    // calling it inside the comparator costs O(n log n) encodings.
    const encoded = entries.map((key) => ({ key, raw: keyBytes(key) }));
    encoded.sort((a, b) => compareBytes(a.raw, b.raw));
    for (let i = 1; i < encoded.length; i++) {
      if (compareBytes(encoded[i - 1].raw, encoded[i].raw) === 0) {
        throw new Error("sync keys are not distinct");
      }
    }
    const sorted = encoded.map((e) => e.key);
    this.keys = sorted;
    this.keyRaw = encoded.map((e) => e.raw);
    this.ids = sorted.map((k) => k.id);
    const acc = new Uint8Array(16);
    this.xor = [new Uint8Array(acc)];
    for (const id of this.ids) {
      const raw = hexToBytes(id);
      for (let i = 0; i < 16; i++) acc[i] ^= raw[i];
      this.xor.push(new Uint8Array(acc));
    }
  }

  get len(): number {
    return this.keys.length;
  }

  get allIds(): string[] {
    return this.ids;
  }

  /** Index of the first key whose prefix is not below `bound`. */
  private idx(bound: Uint8Array): number {
    let lo = 0;
    let hi = this.keyRaw.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      const prefix = this.keyRaw[mid].subarray(0, bound.length);
      if (compareBytes(prefix, bound) < 0) lo = mid + 1;
      else hi = mid;
    }
    return lo;
  }

  private fp(lo: number, hi: number): { fp: Uint8Array; count: number } {
    const out = new Uint8Array(16);
    for (let i = 0; i < 16; i++) out[i] = this.xor[hi][i] ^ this.xor[lo][i];
    return { fp: out, count: hi - lo };
  }

  /** Shortest prefix of keys[p] that sorts above keys[p-1]. */
  private separator(p: number): Uint8Array {
    const a = this.keyRaw[p - 1];
    const b = this.keyRaw[p];
    let d = 0;
    while (d < KEY_LEN && a[d] === b[d]) d++;
    if (d >= KEY_LEN) d = KEY_LEN - 1;
    return b.subarray(0, d + 1);
  }

  /** The whole key space as one fingerprint range. */
  fullRange(): RangeEntry {
    const { fp, count } = this.fp(0, this.len);
    return { upper: null, mode: { t: "fp", fp, count } };
  }

  /** Answers each entry of a received flight from this side's keys,
   * with this side's own params. IDs in the symmetric difference go
   * into `learned`. The flight's bounds must already be checked. */
  respond(msg: RangeEntry[], params: RangeParams, learned: Set<string>): RangeEntry[] {
    const out: RangeEntry[] = [];
    let lower: Uint8Array | null = null;
    const pushSkip = (upper: Uint8Array | null) => {
      const last = out[out.length - 1];
      if (last && last.mode.t === "skip") {
        last.upper = upper;
        return;
      }
      out.push({ upper, mode: { t: "skip" } });
    };
    for (const entry of msg) {
      const lo = lower === null ? 0 : this.idx(lower);
      const hi = entry.upper === null ? this.len : this.idx(entry.upper);
      switch (entry.mode.t) {
        case "skip":
          pushSkip(entry.upper);
          break;
        case "fp": {
          const mine = this.fp(lo, hi);
          const equal =
            mine.count === entry.mode.count && mine.fp.every((v, i) => v === (entry.mode as { fp: Uint8Array }).fp[i]);
          if (equal) {
            pushSkip(entry.upper);
          } else if (mine.count <= params.t) {
            this.pushIds(out, lo, hi, entry.upper, true, params);
          } else {
            const parts = Math.min(params.k, mine.count);
            const pos: number[] = [];
            for (let j = 0; j <= parts; j++) pos.push(lo + Math.floor((j * (hi - lo)) / parts));
            for (let j = 0; j < parts; j++) {
              const childUpper = j + 1 < parts ? this.separator(pos[j + 1]) : entry.upper;
              const child = this.fp(pos[j], pos[j + 1]);
              out.push({
                upper: childUpper,
                mode: { t: "fp", fp: child.fp, count: child.count },
              });
            }
          }
          break;
        }
        case "ids": {
          const mine = new Set(this.ids.slice(lo, hi));
          const theirs = new Set(entry.mode.ids);
          for (const id of mine) if (!theirs.has(id)) learned.add(id);
          for (const id of theirs) if (!mine.has(id)) learned.add(id);
          if (entry.mode.reply) {
            this.pushIds(out, lo, hi, entry.upper, false, params);
          } else {
            pushSkip(entry.upper);
          }
          break;
        }
      }
      lower = entry.upper;
    }
    return out;
  }

  private pushIds(
    out: RangeEntry[],
    lo: number,
    hi: number,
    upper: Uint8Array | null,
    reply: boolean,
    params: RangeParams,
  ): void {
    const per = idsPerEntry(params);
    let start = lo;
    for (;;) {
      const end = Math.min(start + per, hi);
      const pieceUpper = end < hi ? this.separator(end) : upper;
      out.push({
        upper: pieceUpper === null ? null : new Uint8Array(pieceUpper),
        mode: { t: "ids", ids: this.ids.slice(start, end), reply },
      });
      if (end >= hi) break;
      start = end;
    }
  }
}

// ----- frames --------------------------------------------------------

function putVarint(out: number[], x: number): void {
  let v = x;
  while (v >= 0x80) {
    out.push((v & 0x7f) | 0x80);
    v = Math.floor(v / 128);
  }
  out.push(v);
}

class Reader {
  private pos = 0;
  constructor(private bytes: Uint8Array) {}

  get remaining(): number {
    return this.bytes.length - this.pos;
  }

  byte(): number {
    if (this.pos >= this.bytes.length) throw new Error("message truncated");
    return this.bytes[this.pos++];
  }

  take(n: number): Uint8Array {
    if (this.remaining < n) throw new Error("message truncated");
    const out = this.bytes.subarray(this.pos, this.pos + n);
    this.pos += n;
    return out;
  }

  varint(): number {
    let x = 0;
    for (let i = 0; i < 5; i++) {
      const b = this.byte();
      x += (b & 0x7f) * 2 ** (7 * i);
      if (b < 0x80) {
        if (i > 0 && b === 0) throw new Error("varint is not minimally encoded");
        return x;
      }
    }
    throw new Error("varint longer than 5 bytes");
  }
}

function encodeEntry(entry: RangeEntry): Uint8Array {
  const out: number[] = [];
  if (entry.upper === null) {
    out.push(0);
  } else {
    if (entry.upper.length < 1 || entry.upper.length > KEY_LEN) {
      throw new Error(`a finite bound must contain 1..=${KEY_LEN} bytes`);
    }
    out.push(entry.upper.length);
    out.push(...entry.upper);
  }
  switch (entry.mode.t) {
    case "skip":
      out.push(0);
      break;
    case "fp":
      out.push(1);
      out.push(...entry.mode.fp);
      putVarint(out, entry.mode.count);
      break;
    case "ids":
      out.push(entry.mode.reply ? 3 : 2);
      putVarint(out, entry.mode.ids.length);
      for (const id of entry.mode.ids) out.push(...hexToBytes(id));
      break;
  }
  return new Uint8Array(out);
}

/** Splits a flight into frames of at most `budget` bytes each.
 * Every frame but the last is marked `more`. */
export function frameFlight(entries: RangeEntry[], budget: number): Uint8Array[] {
  const out: Uint8Array[] = [];
  let body: number[] = [];
  let count = 0;
  const finish = (more: boolean) => {
    const frame: number[] = [more ? 1 : 0];
    putVarint(frame, count);
    frame.push(...body);
    out.push(new Uint8Array(frame));
  };
  for (const entry of entries) {
    const encoded = encodeEntry(entry);
    if (encoded.length + FRAME_HEADER_RESERVE > budget) {
      throw new Error(`an entry of ${encoded.length} bytes does not fit a ${budget}-byte frame`);
    }
    if (count > 0 && body.length + encoded.length + FRAME_HEADER_RESERVE > budget) {
      finish(true);
      body = [];
      count = 0;
    }
    body.push(...encoded);
    count += 1;
  }
  finish(false);
  return out;
}

/** The entry count in one frame's header — read before decoding, so
 * the entries can be charged against a budget first. */
export function frameEntryCount(bytes: Uint8Array): number {
  if (bytes.length > FRAME_CAP) throw new Error("frame is over the frame cap");
  const reader = new Reader(bytes);
  const flag = reader.byte();
  if (flag !== 0 && flag !== 1) throw new Error(`unknown frame flag ${flag}`);
  return reader.varint();
}

/** Parses one frame. Returns its `more` flag and its entries. */
export function decodeFrame(bytes: Uint8Array): { more: boolean; entries: RangeEntry[] } {
  if (bytes.length > FRAME_CAP) throw new Error("frame is over the frame cap");
  const reader = new Reader(bytes);
  const flag = reader.byte();
  if (flag !== 0 && flag !== 1) throw new Error(`unknown frame flag ${flag}`);
  const n = reader.varint();
  const entries: RangeEntry[] = [];
  for (let i = 0; i < n; i++) {
    const len = reader.byte();
    if (len > KEY_LEN) throw new Error(`a bound of ${len} bytes is longer than a key`);
    const upper = len === 0 ? null : new Uint8Array(reader.take(len));
    const modeByte = reader.byte();
    if (modeByte === 0) {
      entries.push({ upper, mode: { t: "skip" } });
    } else if (modeByte === 1) {
      const fp = new Uint8Array(reader.take(16));
      const count = reader.varint();
      if (count > 0xffffffff) throw new Error("fingerprint count does not fit u32");
      entries.push({ upper, mode: { t: "fp", fp, count } });
    } else if (modeByte === 2 || modeByte === 3) {
      const count = reader.varint();
      const ids: string[] = [];
      for (let j = 0; j < count; j++) ids.push(bytesToHex(reader.take(32)));
      entries.push({ upper, mode: { t: "ids", ids, reply: modeByte === 3 } });
    } else {
      throw new Error(`unknown mode byte ${modeByte}`);
    }
  }
  if (reader.remaining !== 0) throw new Error("trailing bytes after frame");
  return { more: flag === 1, entries };
}

/** Decodes a whole flight (frames until `more` clears) and checks
 * its bounds — the receiver's full read of one message. */
export function decodeFlight(frames: Uint8Array[]): RangeEntry[] {
  const entries: RangeEntry[] = [];
  let sawEnd = false;
  for (const frame of frames) {
    if (sawEnd) throw new Error("a frame follows the flight's last frame");
    const decoded = decodeFrame(frame);
    entries.push(...decoded.entries);
    if (!decoded.more) sawEnd = true;
  }
  if (!sawEnd) throw new Error("the flight has no last frame");
  checkBounds(entries);
  return entries;
}

// ----- simulation (the oracle loop) ----------------------------------

export interface SimResult {
  flights: number;
  bytesA: number;
  bytesB: number;
  learnedA: Set<string>;
  learnedB: Set<string>;
  maxFrame: number;
}

/** range.rs `simulate`: runs the protocol between two in-memory
 * sets, through the same frames as the wire, each side with its own
 * params. `a` opens. */
export function simulateExchange(
  a: RangeParty,
  pa: RangeParams,
  b: RangeParty,
  pb: RangeParams,
  maxFlights = 64,
): SimResult {
  const learnedA = new Set<string>();
  const learnedB = new Set<string>();
  let msg: RangeEntry[] = [a.fullRange()];
  let aSends = true;
  let bytesA = 0;
  let bytesB = 0;
  let flights = 0;
  let maxFrame = 0;
  for (;;) {
    flights += 1;
    if (flights > maxFlights) throw new Error("reconciliation did not terminate");
    const senderParams = aSends ? pa : pb;
    const receiverParams = aSends ? pb : pa;
    const wire = frameFlight(msg, senderParams.frameBudget);
    const bytes = wire.reduce((sum, f) => sum + f.length, 0);
    maxFrame = Math.max(maxFrame, ...wire.map((f) => f.length));
    if (aSends) bytesA += bytes;
    else bytesB += bytes;
    const incoming = decodeFlight(wire);
    if (allSkip(incoming)) break;
    msg = aSends ? b.respond(incoming, receiverParams, learnedB) : a.respond(incoming, receiverParams, learnedA);
    aSends = !aSends;
  }
  return { flights, bytesA, bytesB, learnedA, learnedB, maxFrame };
}
