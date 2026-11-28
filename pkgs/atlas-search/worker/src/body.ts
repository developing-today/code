// Bounded request body read (#11331). /v1/search only needs
// { query, k }, so a body over MAX_SEARCH_BODY_BYTES is refused before
// it is buffered or parsed: a declared Content-Length over the cap is
// refused without reading, and an undeclared body stops at the cap.

export const MAX_SEARCH_BODY_BYTES = 64 * 1024;

/** The body as text, or null when it is larger than `limit` bytes. */
export async function readTextUpTo(
  req: Pick<Request, "headers" | "body">,
  limit: number = MAX_SEARCH_BODY_BYTES,
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
