import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

import {
  CborError,
  MAX_NESTING_DEPTH,
  decodeCanonical,
  encodeCanonical,
  validateRecord,
  type CborValue,
} from "../../../hosted-primitives/src/canonical-cbor.ts";

// The golden table is shared with the Rust suite
// (crates/id-core/tests/canonical_cbor_vectors.rs): same logical
// values, same hex. Byte equality across Rust and TypeScript is
// asserted by both suites against this one fixed table.

const uint = (value: bigint): CborValue => ({ kind: "uint", value });
const text = (value: string): CborValue => ({ kind: "text", value });

function toHex(data: Uint8Array): string {
  return [...data].map((b) => b.toString(16).padStart(2, "0")).join("");
}

function fromHex(hexText: string): Uint8Array {
  const out = new Uint8Array(hexText.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(hexText.slice(i * 2, i * 2 + 2), 16);
  return out;
}

type FixtureValue =
  | { kind: "null" }
  | { kind: "bool"; value: boolean }
  | { kind: "uint" | "int"; value: string }
  | { kind: "bytes"; value: string }
  | { kind: "text"; value: string }
  | { kind: "array"; value: FixtureValue[] }
  | { kind: "map"; value: [FixtureValue, FixtureValue][] };

interface SharedVectors {
  canonical_cbor: { name: string; value: FixtureValue; encoded_hex: string }[];
}

const SHARED_VECTORS = JSON.parse(
  readFileSync(new URL("../../tests/fixtures/hosted-second-version-g1.json", import.meta.url), "utf8"),
) as SharedVectors;

function fromFixture(value: FixtureValue): CborValue {
  switch (value.kind) {
    case "null":
      return { kind: "null" };
    case "bool":
      return { kind: "bool", value: value.value };
    case "uint":
    case "int":
      return { kind: value.kind, value: BigInt(value.value) };
    case "bytes":
      return { kind: "bytes", value: fromHex(value.value) };
    case "text":
      return { kind: "text", value: value.value };
    case "array":
      return { kind: "array", value: value.value.map(fromFixture) };
    case "map":
      return {
        kind: "map",
        value: value.value.map(([key, item]) => [fromFixture(key), fromFixture(item)] as [CborValue, CborValue]),
      };
  }
}

const GOLDEN = SHARED_VECTORS.canonical_cbor.map(
  ({ name, value, encoded_hex }) => [fromFixture(value), encoded_hex, name] as const,
);

test("golden vectors encode and decode byte-exactly", () => {
  assert.equal(GOLDEN.length, 27, "shared canonical CBOR fixture retains every vector");
  for (const [value, wantHex, name] of GOLDEN) {
    assert.equal(toHex(encodeCanonical(value)), wantHex, `encode ${name}`);
    const decoded = decodeCanonical(fromHex(wantHex));
    assert.equal(toHex(encodeCanonical(decoded)), wantHex, `round-trip ${name}`);
  }
});

test("ambiguous encodings reject with typed errors", () => {
  const cases: [string, string][] = [
    ["1817", "NonShortestForm"],
    ["9fff", "IndefiniteLength"],
    ["bfff", "IndefiniteLength"],
    ["a2616101616102", "DuplicateKey"],
    ["a2616202616101", "KeysOutOfOrder"],
    ["f93c00", "UnsupportedType"],
    ["c000", "UnsupportedType"],
    ["f7", "UnsupportedType"],
    ["0001", "TrailingBytes"],
    ["6261", "UnexpectedEof"],
    ["61ff", "InvalidUtf8"],
  ];
  for (const [input, code] of cases) {
    assert.throws(
      () => decodeCanonical(fromHex(input)),
      (err: unknown) => err instanceof CborError && err.code === code,
      `reject ${input} as ${code}`,
    );
  }
});

test("nesting deeper than the limit rejects", () => {
  const allowed = new Uint8Array(MAX_NESTING_DEPTH + 1);
  allowed.fill(0x81, 0, MAX_NESTING_DEPTH);
  const decoded = decodeCanonical(allowed);
  assert.equal(toHex(encodeCanonical(decoded)), toHex(allowed));

  const deeper = new Uint8Array(MAX_NESTING_DEPTH + 2);
  deeper.fill(0x81, 0, MAX_NESTING_DEPTH + 1);
  assert.throws(
    () => decodeCanonical(deeper),
    (err: unknown) => err instanceof CborError && err.code === "TooDeeplyNested",
  );
});

/** `levels` nested arrays around a `0` leaf, built as a value. */
function nestedArrays(levels: number): CborValue {
  let value: CborValue = uint(0n);
  for (let i = 0; i < levels; i++) value = { kind: "array", value: [value] };
  return value;
}

const tooDeep = (err: unknown) => err instanceof CborError && err.code === "TooDeeplyNested";

test("the encoder applies the decoder's depth limit", () => {
  // #11315: at the limit the encoder succeeds and the bytes decode back.
  const atLimit = nestedArrays(MAX_NESTING_DEPTH);
  assert.deepEqual(decodeCanonical(encodeCanonical(atLimit)), atLimit);

  // One level deeper is refused by the encoder with the decoder's error.
  assert.throws(() => encodeCanonical(nestedArrays(MAX_NESTING_DEPTH + 1)), tooDeep);

  // The same rule through a map value.
  const inMap: CborValue = { kind: "map", value: [[text("k"), nestedArrays(MAX_NESTING_DEPTH)]] };
  assert.throws(() => encodeCanonical(inMap), tooDeep);

  // Far deeper values fail fast instead of overflowing the stack.
  assert.throws(() => encodeCanonical(nestedArrays(100_000)), tooDeep);
});

test("unknown critical fields reject and others tolerate", () => {
  const record: CborValue = {
    kind: "map",
    value: [
      [text("subject"), text("content:abc")],
      [text("future_proof"), uint(1n)],
      [text("future_hint"), text("x")],
    ],
  };
  assert.throws(
    () =>
      validateRecord(record, {
        knownFields: ["subject", "future_hint"],
        criticalFields: ["future_proof"],
      }),
    (err: unknown) => err instanceof CborError && err.code === "UnknownCriticalField",
  );
  validateRecord(record, {
    knownFields: ["subject", "future_proof"],
    criticalFields: ["future_proof"],
  });
});
