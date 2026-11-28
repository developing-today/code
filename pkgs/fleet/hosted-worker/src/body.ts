// Shared byte-budgeted request-body reader (#16519). Every JSON
// ingress handler in this Worker parses through here instead of a
// bare `req.json()`: the body is streamed under a strict byte cap
// and an oversized body is refused with 413 (FailClosed) BEFORE any
// JSON parsing happens, so a multi-gigabyte payload can never be
// buffered whole into the runtime's parser. A declared
// content-length over the cap is refused without reading at all.
//
// The cap is 1 MiB: control-plane JSON payloads for every door
// (queue verbs, session records, stamp creates, kv pipelines,
// capability mints, bridge sends, sync ops, query filters) are
// orders of magnitude smaller, and the kv blob door's raw-byte
// uploads keep their own separate budget (MAX_BLOB_BYTES).

import { FailClosed } from "./errors.ts";

export const MAX_JSON_BODY_BYTES = 1_048_576;

function tooLarge(maxBytes: number): FailClosed {
  return new FailClosed(413, "body_too_large", `request body exceeds the ${maxBytes}-byte limit`);
}

/** Reads the request body as text under a byte cap. Throws
 * FailClosed 413 the moment the cap is exceeded — on the declared
 * content-length when one is present, otherwise as soon as the
 * streamed bytes pass the cap (the stream is cancelled, never
 * drained). */
export async function readBodyText(req: Request, maxBytes: number = MAX_JSON_BODY_BYTES): Promise<string> {
  const declared = req.headers.get("content-length");
  if (declared !== null) {
    const length = Number(declared);
    if (Number.isFinite(length) && length > maxBytes) throw tooLarge(maxBytes);
  }
  if (!req.body) {
    const text = await req.text();
    if (new TextEncoder().encode(text).byteLength > maxBytes) throw tooLarge(maxBytes);
    return text;
  }
  const reader = req.body.getReader();
  const chunks: Uint8Array[] = [];
  let total = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    total += value.byteLength;
    if (total > maxBytes) {
      await reader.cancel().catch(() => {});
      throw tooLarge(maxBytes);
    }
    chunks.push(value);
  }
  const bytes = new Uint8Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return new TextDecoder().decode(bytes);
}

/** Reads and parses a JSON request body under the byte cap.
 * Oversize → FailClosed 413 body_too_large (before parsing);
 * unparseable → FailClosed 400 invalid_body, the same refusal the
 * doors' local jsonBody helpers have always thrown. */
export async function readJsonBody(req: Request, maxBytes: number = MAX_JSON_BODY_BYTES): Promise<unknown> {
  const text = await readBodyText(req, maxBytes);
  try {
    return JSON.parse(text);
  } catch {
    throw new FailClosed(400, "invalid_body", "request body is not valid JSON");
  }
}
