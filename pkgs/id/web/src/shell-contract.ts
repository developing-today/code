/**
 * The versioned shell contract (F13.1), checked from the browser side.
 *
 * A TypeScript port of `src/shell_contract.rs`: it reads
 * `contract/shell-contract.v1.json` and checks client and server frames, and
 * whole sessions, against it. Strict about what is sent: a frame type, field,
 * or phase the contract does not name is a violation.
 */

/** A point in a session's lifecycle. */
export type Phase = "opening" | "query" | "install" | "joined" | "closed";

type FieldType = "string" | "hex" | "bool" | "u16" | "u32" | "u64" | "string[]" | "array" | "object";

type RawField =
  | string
  | {
      type: FieldType;
      optional?: boolean;
      nullable?: boolean;
      min?: number;
      max?: number;
      max_bytes?: number;
      enum?: string[];
    };

type RawFrame = {
  class: string;
  phases?: Phase[];
  opens?: Phase;
  forbidden_keys?: string[];
  fields: Record<string, RawField>;
};

/** The parts of the contract document the checks read. */
export type ShellContract = {
  contract: string;
  version: number;
  revision: number;
  max_frame_bytes: number;
  routing_fields: string[];
  client_frames: Record<string, RawFrame>;
  server_frames: Record<string, RawFrame>;
  projection: { redacted_keys: string[] };
};

type Field = {
  kind: FieldType;
  optional: boolean;
  nullable: boolean;
  min?: number;
  max?: number;
  maxBytes?: number;
  allowed?: string[];
};

/** The result of checking one frame. */
export type Check = { ok: true; type: string } | { ok: false; error: string };

const FIELD_TYPES = new Set<string>(["string", "hex", "bool", "u16", "u32", "u64", "string[]", "array", "object"]);
const INTEGER_MAX: Partial<Record<FieldType, number>> = {
  u16: 0xffff,
  u32: 0xffff_ffff,
  // No u64 entry: its ceiling is u64::MAX, which no JS number can
  // represent, so checkValue handles u64 on its own below.
};
const encoder = new TextEncoder();
const byteLength = (text: string): number => encoder.encode(text).length;

function parseField(raw: RawField): Field {
  if (typeof raw === "string") {
    const optional = raw.endsWith("?");
    const kind = optional ? raw.slice(0, -1) : raw;
    if (!FIELD_TYPES.has(kind)) throw new Error(`unknown field type \`${kind}\``);
    return { kind: kind as FieldType, optional, nullable: optional };
  }
  if (!FIELD_TYPES.has(raw.type)) throw new Error(`unknown field type \`${raw.type}\``);
  const optional = raw.optional ?? false;
  return {
    kind: raw.type,
    optional,
    nullable: raw.nullable ?? optional,
    min: raw.min,
    max: raw.max,
    maxBytes: raw.max_bytes,
    allowed: raw.enum,
  };
}

const isObject = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

const isInteger = (value: unknown): value is number => typeof value === "number" && Number.isInteger(value);

function lookup(
  contract: ShellContract,
  frame: unknown,
  table: Record<string, RawFrame>,
  side: string,
): { ok: true; kind: string; spec: RawFrame; object: Record<string, unknown> } | { ok: false; error: string } {
  const encoded = byteLength(JSON.stringify(frame) ?? "");
  if (encoded > contract.max_frame_bytes) {
    return { ok: false, error: `frame of ${encoded} bytes exceeds ${contract.max_frame_bytes}` };
  }
  if (!isObject(frame)) return { ok: false, error: "a frame must be a JSON object" };
  const kind = frame.type;
  if (typeof kind !== "string") return { ok: false, error: "a frame needs a string `type`" };
  const spec = Object.hasOwn(table, kind) ? table[kind] : undefined;
  if (!spec) return { ok: false, error: `\`${kind}\` is not a ${side} frame in the contract` };
  return { ok: true, kind, spec, object: frame };
}

function checkValue(field: Field, value: unknown): string | null {
  if (field.kind === "u64") {
    if (!isInteger(value) || value < 0) return "must be a non-negative integer";
    // u64::MAX itself (2^64 - 1) rounds to 2^64 in a JS number, so 2^64 is
    // the rounded wire form of an in-range value and is accepted -- but
    // anything above it is provably out of range (the next double up is
    // 2^64 + 2048) and the Rust checker rejects it via value.as_u64().
    const min = field.min ?? 0;
    if (value > 2 ** 64) return `must be in ${min}..=18446744073709551615, got ${value}`;
    // The Rust checker caps u64 at u64::MAX. A JS number cannot hold that
    // ceiling, and a wire value above 2^53 arrives already rounded, so a
    // non-negative integer up there cannot be proven out of range and is
    // accepted -- rejecting it would fail frames the host accepts, on a
    // bound that appears nowhere in the contract. The bounds JS can still
    // check exactly are checked: an explicit min, and an explicit max
    // within the safe range (a rounded value past such a max is past it
    // truly, because the rounding of a huge value stays huge).
    if (value < min) return `must be at least ${min}, got ${value}`;
    if (field.max !== undefined && field.max <= Number.MAX_SAFE_INTEGER && value > field.max) {
      return `must be in ${min}..=${field.max}, got ${value}`;
    }
    return null;
  }
  const typeMax = INTEGER_MAX[field.kind];
  if (typeMax !== undefined) {
    if (!isInteger(value) || value < 0) return "must be a non-negative integer";
    const max = Math.min(field.max ?? typeMax, typeMax);
    const min = field.min ?? 0;
    if (value < min || value > max) return `must be in ${min}..=${max}, got ${value}`;
    return null;
  }
  switch (field.kind) {
    case "string":
    case "hex": {
      if (typeof value !== "string") return "must be a string";
      if (field.kind === "hex" && (value.length % 2 !== 0 || !/^[0-9a-fA-F]*$/.test(value))) {
        return "must be an even number of hex digits";
      }
      const bytes = byteLength(value);
      if (field.maxBytes !== undefined && bytes > field.maxBytes) {
        return `is ${bytes} bytes, more than ${field.maxBytes}`;
      }
      if (field.allowed && !field.allowed.includes(value)) return `must be one of ${field.allowed.join(", ")}`;
      return null;
    }
    case "bool":
      return typeof value === "boolean" ? null : "must be a boolean";
    case "string[]":
      return Array.isArray(value) && value.every((item) => typeof item === "string")
        ? null
        : "must be an array of strings";
    case "array":
      return Array.isArray(value) ? null : "must be an array";
    case "object":
      return isObject(value) ? null : "must be an object";
    default:
      return null;
  }
}

function checkFields(
  kind: string,
  object: Record<string, unknown>,
  spec: RawFrame,
  routing: readonly string[],
): string | null {
  for (const [key, value] of Object.entries(object)) {
    if (key === "type") continue;
    if (routing.includes(key)) {
      if (!(value === null || typeof value === "string")) return `routing field \`${key}\` must be a string`;
      continue;
    }
    if (!Object.hasOwn(spec.fields, key)) return `\`${kind}\` has no field \`${key}\``;
  }
  for (const [name, raw] of Object.entries(spec.fields)) {
    const field = parseField(raw);
    const present = Object.hasOwn(object, name) && object[name] !== undefined;
    const value = present ? object[name] : undefined;
    if (!present && field.optional) continue;
    if (value === null && field.nullable) continue;
    if (!present || value === null) return `\`${kind}\` needs field \`${name}\``;
    const error = checkValue(field, value);
    if (error) return `\`${kind}.${name}\` ${error}`;
  }
  return null;
}

function namesKey(value: unknown, key: string): boolean {
  if (Array.isArray(value)) return value.some((item) => namesKey(item, key));
  if (isObject(value)) return Object.entries(value).some(([name, inner]) => name === key || namesKey(inner, key));
  return false;
}

/** The first value of `key`, anywhere in `value`, that is not zero or null. */
function nonzeroKey(value: unknown, key: string): { found: unknown } | null {
  if (Array.isArray(value)) {
    for (const item of value) {
      const hit = nonzeroKey(item, key);
      if (hit) return hit;
    }
    return null;
  }
  if (isObject(value)) {
    for (const [name, inner] of Object.entries(value)) {
      if (name === key && inner !== 0 && inner !== null) return { found: inner };
      const hit = nonzeroKey(inner, key);
      if (hit) return hit;
    }
  }
  return null;
}

/** Check one client frame sent in `phase`. */
export function checkClientFrame(contract: ShellContract, frame: unknown, phase: Phase): Check {
  const found = lookup(contract, frame, contract.client_frames, "client");
  if (!found.ok) return found;
  const { kind, spec, object } = found;
  if (!(spec.phases ?? []).includes(phase)) {
    return { ok: false, error: `\`${kind}\` may not be sent in the ${phase} phase` };
  }
  for (const key of spec.forbidden_keys ?? []) {
    if (namesKey(object, key)) return { ok: false, error: `\`${kind}\` may not carry \`${key}\` anywhere` };
  }
  const error = checkFields(kind, object, spec, contract.routing_fields);
  return error ? { ok: false, error } : { ok: true, type: kind };
}

/** Check one server frame, including that no redacted key leaks. */
export function checkServerFrame(contract: ShellContract, frame: unknown): Check {
  const found = lookup(contract, frame, contract.server_frames, "server");
  if (!found.ok) return found;
  const { kind, spec, object } = found;
  const error = checkFields(kind, object, spec, []);
  if (error) return { ok: false, error };
  for (const key of contract.projection.redacted_keys) {
    const leak = nonzeroKey(object, key);
    if (leak) {
      return {
        ok: false,
        error: `\`${kind}\` leaks \`${key}\` (${JSON.stringify(leak.found)}) past the client projection`,
      };
    }
  }
  return { ok: true, type: kind };
}

/** One contract violation in a session. */
export type Violation = { index: number; side: "client" | "server"; message: string };

/** Checks one session's frames in order, following its phase. */
export class Conformance {
  #phase: Phase = "opening";
  #index = 0;
  readonly violations: Violation[] = [];

  constructor(private readonly contract: ShellContract) {}

  /** The session's current phase. */
  get phase(): Phase {
    return this.#phase;
  }

  /** Record a frame the shell sent. */
  client(frame: unknown): void {
    if (this.#phase === "closed") {
      this.#violate("client", "the host already ended this session");
    } else {
      const check = checkClientFrame(this.contract, frame, this.#phase);
      if (check.ok) {
        const opens = this.contract.client_frames[check.type]?.opens;
        if (this.#phase === "opening") this.#phase = opens ?? "closed";
        else if (this.#phase === "query" && check.type === "join") this.#phase = "joined";
        else if (this.#phase === "install" && check.type === "install_end") this.#phase = "closed";
      } else {
        this.#violate("client", check.error);
      }
    }
    this.#index += 1;
  }

  /** Record a frame the host sent. */
  server(frame: unknown): void {
    const check = checkServerFrame(this.contract, frame);
    if (!check.ok) this.#violate("server", check.error);
    // Before a join, an error ends the session.
    if (
      isObject(frame) &&
      frame.type === "error" &&
      (this.#phase === "opening" || this.#phase === "query" || this.#phase === "install")
    ) {
      this.#phase = "closed";
    }
    this.#index += 1;
  }

  #violate(side: Violation["side"], message: string): void {
    this.violations.push({ index: this.#index, side, message });
  }
}
