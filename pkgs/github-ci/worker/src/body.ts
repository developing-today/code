// Bounded webhook body read (#11335). The receiver must not buffer an
// arbitrary unauthenticated body before the HMAC check runs, so the
// body is read with a cap: a declared Content-Length over the cap is
// refused without reading, and an undeclared (chunked) body stops at
// the cap. The HMAC check stays the authority, after this read.

/** GitHub caps webhook payloads at 25 MB; a real delivery never needs more. */
export const MAX_WEBHOOK_BODY_BYTES = 25 * 1024 * 1024;

/** The body as text, or null when it is larger than `limit` bytes. */
export async function readBodyUpTo(
  req: Pick<Request, "headers" | "body">,
  limit: number = MAX_WEBHOOK_BODY_BYTES,
): Promise<string | null> {
  const declared = req.headers.get("content-length");
  if (declared !== null && Number(declared) > limit) return null;
  if (!req.body) return "";
  const reader = req.body.getReader();
  const chunks: Uint8Array[] = [];
  let total = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    total += value.byteLength;
    if (total > limit) {
      await reader.cancel();
      return null;
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
