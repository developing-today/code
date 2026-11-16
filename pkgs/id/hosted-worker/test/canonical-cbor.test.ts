import { test } from "node:test";
import assert from "node:assert/strict";

import { CborError, decodeCanonical, encodeCanonical, validateRecord, type CborValue } from "../src/canonical-cbor.ts";

// The golden table is shared with the Rust suite
// (crates/id-core/tests/canonical_cbor_vectors.rs): same logical
// values, same hex. Byte equality across Rust and TypeScript is
// asserted by both suites against this one fixed table.

const uint = (value: bigint): CborValue => ({ kind: "uint", value });
const int = (value: bigint): CborValue => ({ kind: "int", value });
const text = (value: string): CborValue => ({ kind: "text", value });
const bytes = (value: number[]): CborValue => ({ kind: "bytes", value: new Uint8Array(value) });

function toHex(data: Uint8Array): string {
  return [...data].map((b) => b.toString(16).padStart(2, "0")).join("");
}

function fromHex(hexText: string): Uint8Array {
  const out = new Uint8Array(hexText.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(hexText.slice(i * 2, i * 2 + 2), 16);
  return out;
}

const GOLDEN: [CborValue, string][] = [
  [uint(0n), "00"],
  [uint(1n), "01"],
  [uint(10n), "0a"],
  [uint(23n), "17"],
  [uint(24n), "1818"],
  [uint(100n), "1864"],
  [uint(1000n), "1903e8"],
  [uint(1_000_000n), "1a000f4240"],
  [uint(0xffffffffffffffffn), "1bffffffffffffffff"],
  [int(-1n), "20"],
  [int(-10n), "29"],
  [int(-100n), "3863"],
  [int(-1000n), "3903e7"],
  [{ kind: "bool", value: false }, "f4"],
  [{ kind: "bool", value: true }, "f5"],
  [{ kind: "null" }, "f6"],
  [bytes([]), "40"],
  [bytes([1, 2]), "420102"],
  [text(""), "60"],
  [text("a"), "6161"],
  [text("hello"), "6568656c6c6f"],
  [{ kind: "array", value: [] }, "80"],
  [{ kind: "array", value: [uint(1n), uint(2n), uint(3n)] }, "83010203"],
  [{ kind: "map", value: [] }, "a0"],
  [{ kind: "map", value: [[text("a"), uint(1n)]] }, "a1616101"],
  [
    {
      kind: "map",
      value: [
        [text("b"), uint(2n)],
        [text("a"), uint(1n)],
      ],
    },
    "a2616101616202",
  ],
  [
    {
      kind: "map",
      value: [
        [text("generation"), uint(7n)],
        [text("parent"), { kind: "null" }],
        [text("payload"), bytes([0xde, 0xad])],
      ],
    },
    "a366706172656e74f6677061796c6f616442dead6a67656e65726174696f6e07",
  ],
];

test("golden vectors encode and decode byte-exactly", () => {
  for (const [value, wantHex] of GOLDEN) {
    assert.equal(toHex(encodeCanonical(value)), wantHex, `encode ${wantHex}`);
    const decoded = decodeCanonical(fromHex(wantHex));
    assert.equal(toHex(encodeCanonical(decoded)), wantHex, `round-trip ${wantHex}`);
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
