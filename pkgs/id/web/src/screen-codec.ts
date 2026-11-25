import { Unpackr } from "msgpackr";
import type { ScreenPatch } from "./world-surface";

/**
 * Browser decoder for the negotiated binary screen transport
 * (F12.11 phase 3, #12242; Rust half in #12655): after a join that
 * offers `screen_msgpack`, the host may send a screen patch as one
 * binary WebSocket message — a versioned MessagePack value
 * `{version, frame}` whose frame mirrors the JSON `view` frame's
 * patch fields. The wire format is owned by the Rust codec
 * (`id_core::screen_codec::encode_view_patch`, codec version 1);
 * this decoder is deliberately as strict as the Rust one
 * (deny-unknown-fields, one exact value, bounded size): anything
 * else returns null and the caller falls back to a full view.
 *
 * F12.11 phase 4 (#805) adds codec version 2 for compressed frames:
 * `{version: 2, compression, payload}`, where `compression` 1 means
 * the payload is the raw-DEFLATE (RFC 1951) of a version-1 frame and
 * 0 means the payload is the version-1 frame as-is. The host sends
 * version 2 only to a session that also offered `screen_compress`,
 * and only when the complete compressed frame is smaller. Inflating
 * needs `DecompressionStream`, so version-2 frames decode through
 * the async `decodeViewPatchAsync`; the sync `decodeViewPatch`
 * still handles version 1 only and returns null for version 2.
 */

export interface ViewPatchFrame {
  sequence: number;
  view_revision: number;
  base_view_revision: number | null;
  resume_token: string | null;
  patch: ScreenPatch;
}

const SCREEN_CODEC_VERSION = 1;
/** Version of the compressed screen-patch envelope (F12.11 phase 4). */
const SCREEN_CODEC_VERSION_COMPRESSED = 2;
/** Version-2 compression ids (id-core screen_codec). */
const COMPRESSION_NONE = 0;
const COMPRESSION_DEFLATE = 1;
/** The shared session frame limit, matching the Rust codec. */
const MAX_SCREEN_FRAME_BYTES = 1024 * 1024;
/** StyleToken's serialized names (id-core world_presentation). */
const STYLE_TOKENS = new Set(["emphasis", "muted", "heading", "code", "warning", "success", "link"]);

const unpackr = new Unpackr({ useRecords: false, structuredClone: true });

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function hasExactlyKeys(value: Record<string, unknown>, keys: string[]): boolean {
  const actual = Object.keys(value);
  return actual.length === keys.length && keys.every((key) => key in value);
}

function isUint(value: unknown, max: number): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0 && value <= max;
}

function isUintOrNull(value: unknown, max: number): value is number | null {
  return value === null || isUint(value, max);
}

function decodeCell(value: unknown): ScreenPatch["runs"][number]["cells"][number] | null {
  // The Rust encoder omits `token` when it is None
  // (skip_serializing_if), so a cell carries five or six keys.
  if (!isRecord(value)) return null;
  const keys = Object.keys(value);
  const allowed = ["symbol", "fg", "bg", "bold", "columns", "token"];
  if (!allowed.slice(0, 5).every((key) => key in value)) return null;
  if (!keys.every((key) => allowed.includes(key))) return null;
  if (typeof value.symbol !== "string") return null;
  if (!isUint(value.fg, 0xff) || !isUint(value.bg, 0xff) || !isUint(value.columns, 0xff)) return null;
  if (typeof value.bold !== "boolean") return null;
  const token = "token" in value ? value.token : null;
  if (token !== null && (typeof token !== "string" || !STYLE_TOKENS.has(token))) {
    return null;
  }
  return {
    symbol: value.symbol,
    fg: value.fg,
    bg: value.bg,
    bold: value.bold,
    columns: value.columns,
    token,
  };
}

function decodePatch(value: unknown): ScreenPatch | null {
  if (!isRecord(value) || !hasExactlyKeys(value, ["version", "width", "height", "runs"])) {
    return null;
  }
  if (!isUint(value.version, 0xff) || !isUint(value.width, 0xffff) || !isUint(value.height, 0xffff)) {
    return null;
  }
  if (!Array.isArray(value.runs)) return null;
  const runs: ScreenPatch["runs"] = [];
  for (const runValue of value.runs) {
    if (!isRecord(runValue) || !hasExactlyKeys(runValue, ["start", "cells"])) return null;
    if (!isUint(runValue.start, 0xffffffff) || !Array.isArray(runValue.cells)) return null;
    const cells: ScreenPatch["runs"][number]["cells"] = [];
    for (const cellValue of runValue.cells) {
      const cell = decodeCell(cellValue);
      if (!cell) return null;
      cells.push(cell);
    }
    runs.push({ start: runValue.start, cells });
  }
  return { version: value.version, width: value.width, height: value.height, runs };
}

/**
 * Decode one binary view-patch frame. Returns null for anything
 * that is not exactly one well-formed, supported-version frame —
 * the caller treats null as "ask for a full view", never as data.
 * This sync entry handles version-1 frames only; compressed
 * version-2 frames return null here and decode through
 * `decodeViewPatchAsync`.
 */
export function decodeViewPatch(bytes: Uint8Array): ViewPatchFrame | null {
  if (bytes.length === 0 || bytes.length > MAX_SCREEN_FRAME_BYTES) return null;
  let values: unknown[];
  try {
    values = unpackr.unpackMultiple(bytes);
  } catch {
    return null;
  }
  if (values.length !== 1) return null;
  const envelope = values[0];
  if (!isRecord(envelope) || !hasExactlyKeys(envelope, ["version", "frame"])) return null;
  if (envelope.version !== SCREEN_CODEC_VERSION) return null;
  return decodeFrame(envelope.frame);
}

/**
 * Inflate raw-DEFLATE (RFC 1951) bytes, refusing output past the
 * shared frame limit instead of buffering an unbounded stream.
 */
async function inflateRaw(bytes: Uint8Array): Promise<Uint8Array | null> {
  let stream: ReadableStream<Uint8Array>;
  try {
    stream = new Blob([bytes as BlobPart]).stream().pipeThrough(new DecompressionStream("deflate-raw"));
  } catch {
    return null;
  }
  const reader = stream.getReader();
  const chunks: Uint8Array[] = [];
  let total = 0;
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      if (value === undefined) continue;
      total += value.length;
      if (total > MAX_SCREEN_FRAME_BYTES) return null;
      chunks.push(value);
    }
  } catch {
    return null;
  } finally {
    reader.releaseLock();
  }
  const out = new Uint8Array(total);
  let at = 0;
  for (const chunk of chunks) {
    out.set(chunk, at);
    at += chunk.length;
  }
  return out;
}

/**
 * Decode one binary view-patch frame, plain (version 1) or
 * compressed (version 2). Returns null for anything that is not
 * exactly one well-formed frame — the caller treats null as "ask
 * for a full view", never as data.
 */
export async function decodeViewPatchAsync(bytes: Uint8Array): Promise<ViewPatchFrame | null> {
  if (bytes.length === 0 || bytes.length > MAX_SCREEN_FRAME_BYTES) return null;
  let values: unknown[];
  try {
    values = unpackr.unpackMultiple(bytes);
  } catch {
    return null;
  }
  if (values.length !== 1) return null;
  const envelope = values[0];
  if (!isRecord(envelope)) return null;
  if (envelope.version === SCREEN_CODEC_VERSION) return decodeViewPatch(bytes);
  if (envelope.version !== SCREEN_CODEC_VERSION_COMPRESSED) return null;
  if (!hasExactlyKeys(envelope, ["version", "compression", "payload"])) return null;
  if (envelope.compression !== COMPRESSION_NONE && envelope.compression !== COMPRESSION_DEFLATE) {
    return null;
  }
  if (!(envelope.payload instanceof Uint8Array)) return null;
  if (envelope.payload.length > MAX_SCREEN_FRAME_BYTES) return null;
  const inner = envelope.compression === COMPRESSION_NONE ? envelope.payload : await inflateRaw(envelope.payload);
  if (!inner) return null;
  return decodeViewPatch(inner);
}

function decodeFrame(frame: unknown): ViewPatchFrame | null {
  if (
    !isRecord(frame) ||
    !hasExactlyKeys(frame, ["sequence", "view_revision", "base_view_revision", "resume_token", "patch"])
  ) {
    return null;
  }
  if (!isUint(frame.sequence, Number.MAX_SAFE_INTEGER)) return null;
  if (!isUint(frame.view_revision, Number.MAX_SAFE_INTEGER)) return null;
  if (!isUintOrNull(frame.base_view_revision, Number.MAX_SAFE_INTEGER)) return null;
  if (frame.resume_token !== null && typeof frame.resume_token !== "string") return null;
  const patch = decodePatch(frame.patch);
  if (!patch) return null;
  return {
    sequence: frame.sequence,
    view_revision: frame.view_revision,
    base_view_revision: frame.base_view_revision,
    resume_token: frame.resume_token,
    patch,
  };
}
