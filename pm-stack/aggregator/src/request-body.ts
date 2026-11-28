export const MAX_JSON_BODY_BYTES = 100_000;

/** Read JSON while retaining no more than the configured body cap. */
export async function readJsonBody(req: Request): Promise<unknown> {
  const declaredLength = req.headers.get("content-length");
  if (declaredLength !== null && Number(declaredLength) > MAX_JSON_BODY_BYTES) {
    // Avoid pulling an announced-too-large upload at all, while still
    // releasing its source/connection where the runtime supports it.
    await req.body?.cancel().catch(() => {});
    throw new Error("body too large");
  }

  const reader = req.body?.getReader();
  const chunks: Uint8Array[] = [];
  let byteLength = 0;
  if (reader) {
    try {
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        byteLength += value.byteLength;
        if (byteLength > MAX_JSON_BODY_BYTES) {
          await reader.cancel().catch(() => {});
          throw new Error("body too large");
        }
        chunks.push(value);
      }
    } finally {
      reader.releaseLock();
    }
  }

  const bytes = new Uint8Array(byteLength);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return JSON.parse(new TextDecoder().decode(bytes));
}
