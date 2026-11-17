// Canonical CBOR profile for signed ID/Atlas records
// (slice A9114-cbor-profile, #10181; design A-9114.3).
//
// TypeScript mirror of crates/id-core/src/canonical_cbor.rs:
// the same deterministic profile, the same closed value model,
// and the same typed rejections, so a body signature or envelope
// ID is byte-stable across Rust and this hosted worker.
// Deterministic means: shortest-form integers/lengths,
// definite lengths only, map keys unique and sorted by encoded
// bytes, and no floats/tags/other ambiguous forms — the decoder
// rejects anything the encoder would not produce.

export type CborValue =
  | { kind: "null" }
  | { kind: "bool"; value: boolean }
  | { kind: "uint"; value: bigint }
  | { kind: "int"; value: bigint }
  | { kind: "bytes"; value: Uint8Array }
  | { kind: "text"; value: string }
  | { kind: "array"; value: CborValue[] }
  | { kind: "map"; value: [CborValue, CborValue][] };

export type CborErrorCode =
  | "UnexpectedEof"
  | "NonShortestForm"
  | "IndefiniteLength"
  | "UnsupportedType"
  | "Overflow"
  | "InvalidUtf8"
  | "InvalidMapKey"
  | "DuplicateKey"
  | "KeysOutOfOrder"
  | "TrailingBytes"
  | "UnknownCriticalField"
  | "TooDeeplyNested";

/** Typed profile error; `code` names the Rust variant it mirrors. */
export class CborError extends Error {
  readonly code: CborErrorCode;
  readonly byte?: number;
  readonly field?: string;
  constructor(code: CborErrorCode, detail?: { byte?: number; field?: string }) {
    super(detail?.field ? `${code}: ${detail.field}` : detail?.byte !== undefined ? `${code}: ${detail.byte}` : code);
    this.code = code;
    this.byte = detail?.byte;
    this.field = detail?.field;
  }
}

function encodeArgument(major: number, value: bigint, out: number[]): void {
  const m = major << 5;
  if (value < 24n) {
    out.push(m | Number(value));
  } else if (value <= 0xffn) {
    out.push(m | 24, Number(value));
  } else if (value <= 0xffffn) {
    out.push(m | 25, Number(value >> 8n), Number(value & 0xffn));
  } else if (value <= 0xffffffffn) {
    out.push(m | 26);
    for (let shift = 24n; shift >= 0n; shift -= 8n) out.push(Number((value >> shift) & 0xffn));
  } else {
    out.push(m | 27);
    for (let shift = 56n; shift >= 0n; shift -= 8n) out.push(Number((value >> shift) & 0xffn));
  }
}

function compareBytes(a: Uint8Array, b: Uint8Array): number {
  const n = Math.min(a.length, b.length);
  for (let i = 0; i < n; i++) {
    if (a[i] !== b[i]) return a[i] - b[i];
  }
  return a.length - b.length;
}

function isKey(value: CborValue): boolean {
  return value.kind === "text" || value.kind === "uint" || value.kind === "int";
}

function encodeInto(value: CborValue, out: number[], depth: number): void {
  // #11315: refuse what decodeCanonical would refuse, and bound recursion.
  if (depth > MAX_NESTING_DEPTH) throw new CborError("TooDeeplyNested");
  switch (value.kind) {
    case "null":
      out.push(0xf6);
      return;
    case "bool":
      out.push(value.value ? 0xf5 : 0xf4);
      return;
    case "uint":
      encodeArgument(0, value.value, out);
      return;
    case "int":
      encodeArgument(1, -1n - value.value, out);
      return;
    case "bytes":
      encodeArgument(2, BigInt(value.value.length), out);
      out.push(...value.value);
      return;
    case "text": {
      const bytes = new TextEncoder().encode(value.value);
      encodeArgument(3, BigInt(bytes.length), out);
      out.push(...bytes);
      return;
    }
    case "array":
      encodeArgument(4, BigInt(value.value.length), out);
      for (const item of value.value) encodeInto(item, out, depth + 1);
      return;
    case "map": {
      const encoded: { keyBytes: Uint8Array; val: CborValue }[] = [];
      for (const [key, val] of value.value) {
        if (!isKey(key)) throw new CborError("InvalidMapKey");
        const keyOut: number[] = [];
        encodeInto(key, keyOut, depth + 1);
        const keyBytes = new Uint8Array(keyOut);
        if (encoded.some((e) => compareBytes(e.keyBytes, keyBytes) === 0)) {
          throw new CborError("DuplicateKey");
        }
        encoded.push({ keyBytes, val });
      }
      encoded.sort((a, b) => compareBytes(a.keyBytes, b.keyBytes));
      encodeArgument(5, BigInt(encoded.length), out);
      for (const { keyBytes, val } of encoded) {
        out.push(...keyBytes);
        encodeInto(val, out, depth + 1);
      }
      return;
    }
  }
}

/** Encode a value in the canonical profile. */
export function encodeCanonical(value: CborValue): Uint8Array {
  const out: number[] = [];
  encodeInto(value, out, 0);
  return new Uint8Array(out);
}

/** Maximum nesting depth for decoded values. Mirrors the Rust
 * `MAX_NESTING_DEPTH`: each open array or map consumes at least one
 * input byte, so without this bound hostile nesting recurses until
 * the stack overflows. */
export const MAX_NESTING_DEPTH = 64;

class Reader {
  pos = 0;
  constructor(private readonly bytes: Uint8Array) {}

  private byte(): number {
    const byte = this.bytes[this.pos];
    if (byte === undefined) throw new CborError("UnexpectedEof");
    this.pos += 1;
    return byte;
  }

  private take(len: number): Uint8Array {
    if (this.pos + len > this.bytes.length) throw new CborError("UnexpectedEof");
    const slice = this.bytes.slice(this.pos, this.pos + len);
    this.pos += len;
    return slice;
  }

  private argument(additional: number): bigint {
    if (additional <= 23) return BigInt(additional);
    if (additional === 24) {
      const value = this.byte();
      if (value < 24) throw new CborError("NonShortestForm");
      return BigInt(value);
    }
    if (additional === 25 || additional === 26 || additional === 27) {
      const width = additional === 25 ? 2 : additional === 26 ? 4 : 8;
      let value = 0n;
      for (const byte of this.take(width)) value = (value << 8n) | BigInt(byte);
      const previousMax = width === 2 ? 0xffn : width === 4 ? 0xffffn : 0xffffffffn;
      if (value <= previousMax) throw new CborError("NonShortestForm");
      return value;
    }
    if (additional === 31) throw new CborError("IndefiniteLength");
    throw new CborError("UnsupportedType", { byte: additional });
  }

  value(): CborValue {
    return this.valueInner(0);
  }

  private valueInner(depth: number): CborValue {
    if (depth > MAX_NESTING_DEPTH) throw new CborError("TooDeeplyNested");
    const initial = this.byte();
    const major = initial >> 5;
    const additional = initial & 0x1f;
    switch (major) {
      case 0:
        return { kind: "uint", value: this.argument(additional) };
      case 1: {
        const n = this.argument(additional);
        if (n > 0x7fffffffffffffffn) throw new CborError("Overflow");
        return { kind: "int", value: -1n - n };
      }
      case 2:
        return { kind: "bytes", value: this.take(Number(this.argument(additional))) };
      case 3: {
        const bytes = this.take(Number(this.argument(additional)));
        try {
          return { kind: "text", value: new TextDecoder("utf-8", { fatal: true }).decode(bytes) };
        } catch {
          throw new CborError("InvalidUtf8");
        }
      }
      case 4: {
        const len = Number(this.argument(additional));
        const items: CborValue[] = [];
        for (let i = 0; i < len; i++) items.push(this.valueInner(depth + 1));
        return { kind: "array", value: items };
      }
      case 5: {
        const len = Number(this.argument(additional));
        const entries: [CborValue, CborValue][] = [];
        let previous: Uint8Array | undefined;
        for (let i = 0; i < len; i++) {
          const keyStart = this.pos;
          const key = this.valueInner(depth + 1);
          if (!isKey(key)) throw new CborError("InvalidMapKey");
          const keyBytes = this.bytes.slice(keyStart, this.pos);
          if (previous !== undefined) {
            const order = compareBytes(keyBytes, previous);
            if (order === 0) throw new CborError("DuplicateKey");
            if (order < 0) throw new CborError("KeysOutOfOrder");
          }
          previous = keyBytes;
          entries.push([key, this.valueInner(depth + 1)]);
        }
        return { kind: "map", value: entries };
      }
      case 7:
        if (additional === 20) return { kind: "bool", value: false };
        if (additional === 21) return { kind: "bool", value: true };
        if (additional === 22) return { kind: "null" };
        throw new CborError("UnsupportedType", { byte: initial });
      default:
        throw new CborError("UnsupportedType", { byte: initial });
    }
  }
}

/** Decode exactly one canonical-profile value, strictly. */
export function decodeCanonical(bytes: Uint8Array): CborValue {
  const reader = new Reader(bytes);
  const value = reader.value();
  if (reader.pos !== bytes.length) throw new CborError("TrailingBytes");
  return value;
}

export interface RecordSpec {
  knownFields: readonly string[];
  criticalFields: readonly string[];
}

/**
 * Validate a decoded record map against a field spec: a critical
 * field the reader does not know rejects; unknown non-critical
 * fields are tolerated.
 */
export function validateRecord(record: CborValue, spec: RecordSpec): void {
  if (record.kind !== "map") throw new CborError("InvalidMapKey");
  for (const [key] of record.value) {
    if (key.kind !== "text") continue;
    if (spec.criticalFields.includes(key.value) && !spec.knownFields.includes(key.value)) {
      throw new CborError("UnknownCriticalField", { field: key.value });
    }
  }
}
