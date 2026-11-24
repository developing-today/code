import { test } from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";

// Conformance of a TypeScript `id://` V1 codec with the stdlib reference
// codec (slice A9117-uri-reference-pbt, #10280; design A-9117.1,
// S4.12-S4.17).
//
// The reference (doc/.../part-a-addressing/uri_codec_reference.py
// self-test) draws 10,000 array IDs and 10,000 content IDs from
// `random.Random(0x9117_2026)` and pins the SHA-256 of the formatted
// corpus. This suite replays the same draws with a port of CPython's
// Mersenne Twister, formats them with the TypeScript codec below, and
// requires the same digest; the Rust suite
// (crates/id-core/tests/id_uri_property.rs) does the same. Every URI must
// parse back to the value that produced it, and the reference's readable
// vectors, content golden and malformed corpus give the same outcomes.
//
// The hosted worker has no id:// module yet, so the TypeScript edition of
// the codec lives here, as a line-for-line port of the reference.

const CORPUS_SHA256 = "390eaa67a9f94e9034fb8ba77596b14be2765403606b52abfb6c1a4ff9af7fac";
const SEED = 0x9117_2026;

// ---------------------------------------------------------------- codec

class UriError extends Error {}

type Slot = Uint8Array | null;
type ArrayId = { kind: "array"; resolverHint: string; namespace: string; slots: Slot[] };
type ContentId = { kind: "content"; resolverHint: string; namespace: string; profile: string; digest: Uint8Array };

const ALIAS = /^[a-z0-9][a-z0-9._-]{0,62}$/;
const PROFILE = "raw-blake3-256";
const DIGEST = /^[0-9a-f]{64}$/;
const HEX = /^[0-9a-fA-F]$/;

const isUnreserved = (b: number): boolean =>
  (b >= 0x30 && b <= 0x39) ||
  (b >= 0x41 && b <= 0x5a) ||
  (b >= 0x61 && b <= 0x7a) ||
  b === 0x2d ||
  b === 0x2e ||
  b === 0x5f ||
  b === 0x7e;

function checkAlias(alias: string): void {
  if (!ALIAS.test(alias)) throw new UriError("invalid_resolver_alias");
}

function pctEncode(data: Uint8Array): string {
  let out = "";
  for (const b of data) {
    out += isUnreserved(b) ? String.fromCharCode(b) : "%" + b.toString(16).toUpperCase().padStart(2, "0");
  }
  return out;
}

function pctDecode(component: string): Uint8Array {
  const out: number[] = [];
  let i = 0;
  while (i < component.length) {
    const ch = component[i];
    if (ch === "%") {
      if (i + 2 >= component.length || !HEX.test(component[i + 1]) || !HEX.test(component[i + 2])) {
        throw new UriError("malformed_percent_escape");
      }
      out.push(parseInt(component.slice(i + 1, i + 3), 16));
      i += 3;
      continue;
    }
    const code = ch.charCodeAt(0);
    if (code > 0x7f || !isUnreserved(code)) throw new UriError("raw_non_unreserved_component_byte");
    out.push(code);
    i += 1;
  }
  return new Uint8Array(out);
}

function decodeOctets(component: string): Uint8Array {
  const decoded = pctDecode(component);
  if (pctEncode(decoded) !== component) throw new UriError("noncanonical_byte_slot");
  return decoded;
}

function namespaceBytes(namespace: string): Uint8Array {
  if (namespace.length === 0) throw new UriError("empty_namespace");
  if (namespace === "." || namespace === "..") throw new UriError("dot_segment_namespace");
  return new TextEncoder().encode(namespace);
}

function namespaceText(segment: string): string {
  const raw = pctDecode(segment);
  let value: string;
  try {
    value = new TextDecoder("utf-8", { fatal: true }).decode(raw);
  } catch {
    throw new UriError("invalid_namespace_utf8");
  }
  if (raw.length === 0 || value === "." || value === "..") throw new UriError("invalid_namespace_component");
  return value;
}

function head(alias: string, namespace: string): string {
  checkAlias(alias);
  return `id://${alias}/${pctEncode(namespaceBytes(namespace))}/v1`;
}

function formatArray(value: ArrayId): string {
  const slots = value.slots.map((slot) => (slot === null ? "~" : "b." + pctEncode(slot)));
  return head(value.resolverHint, value.namespace) + "/array" + (slots.length ? "/" + slots.join("/") : "");
}

function formatContent(value: ContentId): string {
  if (value.profile !== PROFILE) throw new UriError("unsupported_hash_profile");
  if (value.digest.length !== 32) throw new UriError("invalid_digest_length");
  const digest = [...value.digest].map((b) => b.toString(16).padStart(2, "0")).join("");
  return `${head(value.resolverHint, value.namespace)}/content/${value.profile}.${digest}`;
}

function split(uri: string): [string, string, string[]] {
  for (let i = 0; i < uri.length; i++) {
    const code = uri.charCodeAt(i);
    if (code < 0x21 || code > 0x7e) throw new UriError("raw_control_space_or_nonascii");
  }
  // Generic split: optional scheme, optional `//` authority ending at the
  // first `/`, `?` or `#`.
  const colon = uri.indexOf(":");
  const hasScheme = colon > 0 && /^[A-Za-z][A-Za-z0-9+.-]*$/.test(uri.slice(0, colon));
  const scheme = hasScheme ? uri.slice(0, colon).toLowerCase() : "";
  const rest = hasScheme ? uri.slice(colon + 1) : uri;
  let authority = "";
  let after = rest;
  if (rest.startsWith("//")) {
    const hier = rest.slice(2);
    const end = hier.search(/[/?#]/);
    authority = end < 0 ? hier : hier.slice(0, end);
    after = end < 0 ? "" : hier.slice(end);
    if (/[[\]]/.test(authority)) throw new UriError("uri_parse_error");
  }
  if (scheme !== "id" || authority === "") throw new UriError("invalid_scheme_or_missing_authority");
  if (uri.includes("?") || uri.includes("#")) throw new UriError("query_or_fragment_not_in_v1");
  checkAlias(authority);
  if (!after.startsWith("/")) throw new UriError("path_must_be_absolute");
  // Split while escapes are still encoded. Never decode and then split.
  const segments = after.slice(1).split("/");
  if (segments.length < 3 || segments.some((s) => s === "")) throw new UriError("empty_or_missing_path_component");
  const namespace = namespaceText(segments[0]);
  if (segments[1] !== "v1") throw new UriError("unsupported_critical_version");
  return [authority, namespace, segments.slice(2)];
}

function parse(uri: string): ArrayId | ContentId {
  const [alias, namespace, tail] = split(uri);
  if (tail[0] === "array") {
    const slots: Slot[] = [];
    for (const token of tail.slice(1)) {
      if (token === "~") slots.push(null);
      else if (token.startsWith("b.")) slots.push(decodeOctets(token.slice(2)));
      else throw new UriError("invalid_array_slot_token");
    }
    return { kind: "array", resolverHint: alias, namespace, slots };
  }
  if (tail[0] === "content" && tail.length === 2) {
    const dot = tail[1].indexOf(".");
    if (dot < 0) throw new UriError("invalid_content_token");
    const profile = tail[1].slice(0, dot);
    const digestText = tail[1].slice(dot + 1);
    if (profile !== PROFILE) throw new UriError("unsupported_hash_profile");
    if (!DIGEST.test(digestText)) throw new UriError("invalid_digest_text");
    const digest = new Uint8Array(32);
    for (let i = 0; i < 32; i++) digest[i] = parseInt(digestText.slice(i * 2, i * 2 + 2), 16);
    return { kind: "content", resolverHint: alias, namespace, profile, digest };
  }
  throw new UriError("unknown_critical_shape");
}

function resolveContext(uriHint: string, selected: string): string {
  checkAlias(uriHint);
  checkAlias(selected);
  if (uriHint !== selected) throw new UriError("resolver_context_mismatch");
  return selected;
}

function errorClass(fn: () => unknown): string {
  try {
    fn();
  } catch (error) {
    if (error instanceof UriError) return error.message;
    throw error;
  }
  return "ok";
}

// ------------------------------------------------- CPython random port

// `random.Random(seed)` for a seed below 2**32: MT19937 seeded by
// init_by_array([seed]), with getrandbits, _randbelow, randrange, choice
// and randbytes as CPython 3.9+ defines them.
class PyRandom {
  private mt = new Uint32Array(624);
  private index = 624;

  constructor(seed: number) {
    const mt = this.mt;
    mt[0] = 19650218;
    for (let i = 1; i < 624; i++) {
      const prev = mt[i - 1] ^ (mt[i - 1] >>> 30);
      mt[i] = (Math.imul(1812433253, prev) + i) >>> 0;
    }
    const key = [seed >>> 0];
    let i = 1;
    let j = 0;
    for (let k = Math.max(624, key.length); k > 0; k--) {
      const prev = mt[i - 1] ^ (mt[i - 1] >>> 30);
      mt[i] = ((mt[i] ^ Math.imul(prev, 1664525)) + key[j] + j) >>> 0;
      i++;
      j++;
      if (i >= 624) {
        mt[0] = mt[623];
        i = 1;
      }
      if (j >= key.length) j = 0;
    }
    for (let k = 623; k > 0; k--) {
      const prev = mt[i - 1] ^ (mt[i - 1] >>> 30);
      mt[i] = ((mt[i] ^ Math.imul(prev, 1566083941)) - i) >>> 0;
      i++;
      if (i >= 624) {
        mt[0] = mt[623];
        i = 1;
      }
    }
    mt[0] = 0x80000000;
  }

  nextU32(): number {
    const mt = this.mt;
    if (this.index >= 624) {
      for (let k = 0; k < 624; k++) {
        const y = (mt[k] & 0x80000000) | (mt[(k + 1) % 624] & 0x7fffffff);
        mt[k] = (mt[(k + 397) % 624] ^ (y >>> 1) ^ (y & 1 ? 0x9908b0df : 0)) >>> 0;
      }
      this.index = 0;
    }
    let y = mt[this.index++];
    y ^= y >>> 11;
    y ^= (y << 7) & 0x9d2c5680;
    y ^= (y << 15) & 0xefc60000;
    return (y ^ (y >>> 18)) >>> 0;
  }

  bits(k: number): number {
    return k === 0 ? 0 : this.nextU32() >>> (32 - k);
  }

  below(n: number): number {
    const k = 32 - Math.clz32(n);
    for (;;) {
      const r = this.bits(k);
      if (r < n) return r;
    }
  }

  randrange(start: number, stop: number): number {
    return start + this.below(stop - start);
  }

  choice<T>(items: ArrayLike<T>): T {
    return items[this.below(items.length)];
  }

  randbytes(n: number): Uint8Array {
    const out = new Uint8Array(n);
    let offset = 0;
    let remaining = n * 8;
    while (remaining > 0) {
      const take = Math.min(remaining, 32);
      const word = this.bits(take);
      for (let b = 0; b < take / 8; b++) out[offset++] = (word >>> (8 * b)) & 0xff;
      remaining -= take;
    }
    return out;
  }
}

// ----------------------------------------------------------------- tests

const utf8 = (text: string): Uint8Array => new TextEncoder().encode(text);

test("CPython Mersenne Twister port matches CPython", () => {
  const rng = new PyRandom(SEED);
  assert.deepEqual([rng.bits(32), rng.bits(32), rng.bits(32)], [1688267078, 1539041827, 1619599464]);
  assert.deepEqual([...rng.randbytes(5)], [101, 213, 5, 69, 198]);
  assert.equal(new PyRandom(5489).bits(32), 3382763572);
});

test("deterministic corpus matches the reference digest and round-trips", () => {
  const rng = new PyRandom(SEED);
  const alphabet: Slot[] = [
    null,
    new Uint8Array(0),
    utf8("/"),
    utf8("%"),
    utf8("~"),
    Uint8Array.from({ length: 256 }, (_, i) => i),
    utf8("雪/é"),
  ];
  const prefixes = ["ns", "雪/é", "café%path"];
  const tail = [..."abcXYZ012-_"];
  const hash = createHash("sha256");
  for (let n = 0; n < 10_000; n++) {
    let namespace = rng.choice(prefixes);
    for (let i = rng.randrange(0, 12); i > 0; i--) namespace += rng.choice(tail);
    const slots: Slot[] = [];
    for (let i = rng.randrange(0, 10); i > 0; i--) {
      slots.push(rng.randrange(0, 3) ? rng.choice(alphabet) : rng.randbytes(rng.randrange(0, 17)));
    }
    const value: ArrayId = { kind: "array", resolverHint: "local", namespace, slots };
    const uri = formatArray(value);
    assert.deepEqual(parse(uri), value);
    hash.update(uri + "\n");
  }
  for (let n = 0; n < 10_000; n++) {
    const value: ContentId = {
      kind: "content",
      resolverHint: "local",
      namespace: "main",
      profile: PROFILE,
      digest: rng.randbytes(32),
    };
    const uri = formatContent(value);
    assert.deepEqual(parse(uri), value);
    assert.equal(formatContent(parse(uri) as ContentId), uri);
    hash.update(uri + "\n");
  }
  assert.equal(hash.digest("hex"), CORPUS_SHA256);
});

test("readable vectors and content golden match the reference", () => {
  const vectors: [string, string, Slot[]][] = [
    ["id://local/main/v1/array", "main", []],
    ["id://local/main/v1/array/~", "main", [null]],
    ["id://local/main/v1/array/b.", "main", [new Uint8Array(0)]],
    ["id://local/main/v1/array/b.%2F", "main", [utf8("/")]],
    ["id://local/wiki/v1/array/b.us%20president.md", "wiki", [utf8("us president.md")]],
    ["id://local/%E9%9B%AA%2F%C3%A9/v1/array", "雪/é", []],
  ];
  for (const [uri, namespace, slots] of vectors) {
    const value: ArrayId = { kind: "array", resolverHint: "local", namespace, slots };
    assert.deepEqual(parse(uri), value);
    assert.equal(formatArray(value), uri);
  }
  const golden =
    "id://local/main/v1/content/raw-blake3-256.af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";
  const parsed = parse(golden) as ContentId;
  assert.equal(parsed.kind, "content");
  assert.equal(formatContent(parsed), golden);
});

test("malformed corpus and negative content cases reject with the reference classes", () => {
  const z63 = "0".repeat(63);
  const bad: [string, string][] = [
    ["id:/local/main/v1/array", "invalid_scheme_or_missing_authority"],
    ["id://Local/main/v1/array", "invalid_resolver_alias"],
    ["id://local/main/v2/array", "unsupported_critical_version"],
    ["id://local/main/v1/array/%2F", "invalid_array_slot_token"],
    ["id://local/main/v1/array/b.%", "malformed_percent_escape"],
    ["id://local/main/v1/array/b.%2F/", "empty_or_missing_path_component"],
    ["id://local/main/v1/array/b.%61", "noncanonical_byte_slot"],
    ["id://local/main/v1/array?", "query_or_fragment_not_in_v1"],
    ["id://local/main/v1/array#", "query_or_fragment_not_in_v1"],
    ["id://local/%FF/v1/array", "invalid_namespace_utf8"],
    ["id://local/main/v1/array?x=1", "query_or_fragment_not_in_v1"],
    ["id://local/main/v1/array#x", "query_or_fragment_not_in_v1"],
    ["id://local/../v1/array", "invalid_namespace_component"],
    [`id://local/main/v1/content/unknown.${"0".repeat(64)}`, "unsupported_hash_profile"],
    [`id://local/main/v1/content/raw-blake3-256.${z63}`, "invalid_digest_text"],
    [`id://local/main/v1/content/raw-blake3-256.${z63}G`, "invalid_digest_text"],
    [`id://local/main/v1/content/raw-blake3-256.${z63}A`, "invalid_digest_text"],
    ["id://[local/main/v1/array", "uri_parse_error"],
  ];
  for (const [uri, expected] of bad)
    assert.equal(
      errorClass(() => parse(uri)),
      expected,
      uri,
    );

  const base = { kind: "content" as const, resolverHint: "local", namespace: "main" };
  assert.equal(
    errorClass(() => formatContent({ ...base, profile: PROFILE, digest: utf8("short") })),
    "invalid_digest_length",
  );
  assert.equal(
    errorClass(() => formatContent({ ...base, profile: "unknown-profile", digest: new Uint8Array(32) })),
    "unsupported_hash_profile",
  );
  assert.equal(
    errorClass(() => resolveContext("local", "other")),
    "resolver_context_mismatch",
  );
  assert.equal(resolveContext("local", "local"), "local");
});
