// HMAC-SHA256 verification for GitHub webhook deliveries, following the same
// discipline as the fleet's existing edge webhook gateway (bounded body read,
// Web Crypto verify, constant comparison). GitHub signs the raw body with the
// app's webhook secret and sends it as X-Hub-Signature-256: sha256=<hex>.
const encoder = new TextEncoder();

export async function readBoundedBody(stream: ReadableStream<Uint8Array> | null, limit: number): Promise<Uint8Array> {
  if (!stream) return new Uint8Array();
  const reader = stream.getReader();
  const parts: Uint8Array[] = [];
  let size = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    size += value.length;
    if (size > limit) {
      await reader.cancel();
      throw new Error("body_limit");
    }
    parts.push(value);
  }
  const bytes = new Uint8Array(size);
  let offset = 0;
  for (const part of parts) {
    bytes.set(part, offset);
    offset += part.length;
  }
  return bytes;
}

/** Timing-safe hex comparison (length check first, then byte-wise). */
function constantTimeEqual(a: Uint8Array, b: Uint8Array): boolean {
  if (a.length !== b.length) return false;
  let diff = 0;
  for (let i = 0; i < a.length; i++) diff |= a[i] ^ b[i];
  return diff === 0;
}

export async function validSignature(
  body: Uint8Array,
  signature: string | null,
  secret: string | null | undefined,
): Promise<boolean> {
  if (!secret || !signature) return false;
  if (!/^sha256=[0-9a-f]{64}$/i.test(signature)) return false;
  const key = await crypto.subtle.importKey("raw", encoder.encode(secret), { name: "HMAC", hash: "SHA-256" }, false, [
    "sign",
  ]);
  const digest = Uint8Array.from(signature.slice(7).match(/../g) ?? [], (hex) => Number.parseInt(hex, 16));
  const ours = new Uint8Array(
    await crypto.subtle.sign(
      "HMAC",
      key,
      body.buffer.slice(body.byteOffset, body.byteOffset + body.byteLength) as ArrayBuffer,
    ),
  );
  return constantTimeEqual(ours, digest);
}

export function jsonResponse(status: number, body: unknown, extraHeaders: Record<string, string> = {}): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json", ...extraHeaders },
  });
}
