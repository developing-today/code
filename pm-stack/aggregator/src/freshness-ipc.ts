import type { FreshnessInput, FreshnessStore } from "./freshness.ts";

const MAX_BODY_BYTES = 16 * 1024;

function json(status: number, value: unknown): Response {
  return new Response(JSON.stringify(value), {
    status,
    headers: { "content-type": "application/json; charset=utf-8", "cache-control": "no-store" },
  });
}

async function readBoundedBody(request: Request): Promise<Uint8Array | null> {
  const declared = request.headers.get("content-length");
  if (declared !== null && /^\d+$/.test(declared) && Number(declared) > MAX_BODY_BYTES) return null;
  if (!request.body) return new Uint8Array();

  const reader = request.body.getReader();
  const chunks: Uint8Array[] = [];
  let size = 0;
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      size += value.byteLength;
      if (size > MAX_BODY_BYTES) {
        await reader.cancel();
        return null;
      }
      chunks.push(value);
    }
  } finally {
    reader.releaseLock();
  }

  const body = new Uint8Array(size);
  let offset = 0;
  for (const chunk of chunks) {
    body.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return body;
}

/** The only write seam for cross-process freshness publishers.
 *  The caller must bind this handler to loopback, never to the public
 *  aggregator hosts. No source payload is accepted or returned. */
export function createFreshnessIpcHandler(store: FreshnessStore): (request: Request) => Promise<Response> {
  return async (request: Request): Promise<Response> => {
    const url = new URL(request.url);
    if (url.pathname !== "/publish") return json(404, { error: "not_found" });
    if (request.method !== "POST") return json(405, { error: "method_not_allowed" });

    const bytes = await readBoundedBody(request);
    if (bytes === null) return json(413, { error: "body_too_large" });
    let input: unknown;
    try {
      input = JSON.parse(new TextDecoder().decode(bytes));
    } catch {
      return json(400, { error: "invalid_json" });
    }

    try {
      const accepted = await store.publish(input as FreshnessInput);
      return json(202, accepted);
    } catch {
      // Do not echo request material or filesystem errors across the
      // internal publisher seam; a source publisher may safely retry.
      return json(503, { error: "freshness_unavailable" });
    }
  };
}
