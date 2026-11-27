// The pm-metadata block (#12585; #9060 design §12) — parse, assemble
// and render the Metadata section of an issue/PR body.
//
// The block lives between HTML-comment markers with a fenced JSON
// payload, invisible in rendered views and trivially locatable in the
// raw body:
//
//   <!-- pm-metadata:start -->
//   ```json
//   { "pm_meta": 1, "direct_parent": ..., "links": [...],
//     "lineage": {...}, "provenance": {...} }
//   ```
//   <!-- pm-metadata:end -->
//
// Ownership split (design §4.3 layer 1, §5): `direct_parent`, `links`
// and `lineage` are CALLER-owned — the stamping service preserves them
// exactly and never overwrites a caller-supplied value. `provenance`
// is FLEET-owned — the service is authoritative for every key in it
// and writes the set in full from its own sources (src/routes/
// stamping.ts); a caller-supplied provenance value, forged or honest,
// is replaced, and unknown provenance keys are dropped rather than
// signed sight-unseen. Any other top-level keys a caller carries in
// the block are preserved (the §12 format decision allows arbitrary
// keys; the block is the caller's except for provenance).
//
// This module is pure text/object work: no clock, no registry, no
// signing. The route assembles the provenance; stamp-sig.ts signs it.

import { FailClosed } from "./errors.ts";

export const METADATA_START = "<!-- pm-metadata:start -->";
export const METADATA_END = "<!-- pm-metadata:end -->";

/** The block's caller-owned fields (design §5): preserved verbatim. */
export const CALLER_OWNED_FIELDS = ["direct_parent", "links", "lineage"] as const;

/** Required-field defaults for the caller-owned fields (#9073's
 * "ensures required fields are present — injecting defaults"). */
export function callerOwnedDefaults(): Record<string, unknown> {
  return {
    direct_parent: null,
    links: [],
    lineage: { design: null, roadmap_item: null, priority: null },
  };
}

export interface ParsedBody {
  /** The parsed block, or null when the body carries none. */
  block: Record<string, unknown> | null;
  /** Body text before the block (the whole body when block is null). */
  before: string;
  /** Body text after the block ("" when block is null). */
  after: string;
}

function invalidBlock(message: string): FailClosed {
  return new FailClosed(400, "invalid_metadata_block", message);
}

/**
 * Extract the payload of the fenced block between the markers, or
 * null when the region is not exactly one fenced block (#16075).
 *
 * The closing fence is a delimiter LINE — a line consisting solely
 * of ``` — never a backtick run inside the payload: JSON.stringify
 * leaves backticks inside string values unchanged, so a caller-owned
 * title or link containing ``` (even at a line start in a
 * caller-formatted payload whose strings carry literal newlines)
 * must not end the fence early and truncate the block into an
 * invalid_metadata_block refusal. A payload line of bare backticks
 * cannot be valid JSON string content (a JSON string cannot contain
 * a literal newline), so the first such line is unambiguously the
 * delimiter. Anything other than whitespace before the opening fence
 * or after the closing fence fails closed (returns null), as does a
 * missing opening or closing fence.
 */
function extractFencedPayload(inner: string): string | null {
  const lines = inner.split("\n");
  let i = 0;
  while (i < lines.length && lines[i].trim() === "") i++;
  if (i >= lines.length || !/^```(?:json)?$/.test(lines[i].trim())) {
    return null;
  }
  i++;
  const payloadLines: string[] = [];
  for (; i < lines.length; i++) {
    if (lines[i].trim() === "```") {
      for (let j = i + 1; j < lines.length; j++) {
        if (lines[j].trim() !== "") return null;
      }
      return payloadLines.join("\n");
    }
    payloadLines.push(lines[i]);
  }
  return null;
}

/**
 * Locate and parse the pm-metadata block in a body. A body with no
 * markers parses to block: null. A body whose markers are present but
 * whose payload is not a fenced JSON object with pm_meta: 1 fails
 * closed (400 invalid_metadata_block): silently dropping or ignoring
 * a malformed block would lose caller-owned fields, so the create is
 * refused instead and the caller fixes its block.
 */
export function parseMetadataBlock(body: string): ParsedBody {
  const startIdx = body.indexOf(METADATA_START);
  if (startIdx === -1) {
    if (body.includes(METADATA_END)) {
      throw invalidBlock("pm-metadata end marker without a start marker");
    }
    return { block: null, before: body, after: "" };
  }
  const endIdx = body.indexOf(METADATA_END, startIdx + METADATA_START.length);
  if (endIdx === -1) {
    throw invalidBlock("pm-metadata start marker without an end marker");
  }
  const inner = body.slice(startIdx + METADATA_START.length, endIdx);
  const payload = extractFencedPayload(inner);
  if (payload === null) {
    throw invalidBlock("pm-metadata block does not carry a fenced JSON payload");
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(payload);
  } catch {
    throw invalidBlock("pm-metadata block payload is not valid JSON");
  }
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    throw invalidBlock("pm-metadata block payload must be a JSON object");
  }
  const block = parsed as Record<string, unknown>;
  if (block["pm_meta"] !== 1) {
    throw invalidBlock("pm-metadata block must carry pm_meta: 1");
  }
  return {
    block,
    before: body.slice(0, startIdx),
    after: body.slice(endIdx + METADATA_END.length),
  };
}

/** Render a block in the §12 canonical form. */
export function renderMetadataBlock(block: Record<string, unknown>): string {
  return `${METADATA_START}\n\`\`\`json\n${JSON.stringify(block, null, 2)}\n\`\`\`\n${METADATA_END}`;
}

/**
 * Splice a block into a body: in place of the parsed block's span
 * when one was parsed, else appended at the end (separated from the
 * caller's text by a blank line). Text outside the block is preserved
 * byte-for-byte.
 */
export function renderBodyWithBlock(parsed: ParsedBody, block: Record<string, unknown>): string {
  const rendered = renderMetadataBlock(block);
  if (parsed.block !== null) {
    return parsed.before + rendered + parsed.after;
  }
  const trimmed = parsed.before.replace(/\s+$/, "");
  return (trimmed === "" ? "" : `${trimmed}\n\n`) + `${rendered}\n`;
}

/**
 * Assemble the stamped block: the caller's block (or defaults) with
 * its provenance replaced in full by the service-assembled, already
 * signed provenance object. Caller-owned fields are preserved exactly
 * as parsed; absent ones take the required-field defaults. Every
 * other caller top-level key is preserved; `pm_meta` is pinned to 1.
 */
export function assembleStampedBlock(
  callerBlock: Record<string, unknown> | null,
  signedProvenance: Record<string, unknown>,
): Record<string, unknown> {
  const out: Record<string, unknown> = { ...(callerBlock ?? {}) };
  const defaults = callerOwnedDefaults();
  for (const field of CALLER_OWNED_FIELDS) {
    if (!(field in out) || out[field] === undefined) {
      out[field] = defaults[field];
    }
  }
  out["pm_meta"] = 1;
  out["provenance"] = signedProvenance;
  return out;
}
