/**
 * Calls to the fleet search service (`fleet serve search`,
 * #6788). The service is the fan-out engine: it discovers the
 * `atlas-core-*` shard set from the Turso Platform API, reads each
 * shard's `atlas_manifest`, enforces this client's tier ceiling,
 * embeds with the one shared pinned Snowflake session, and runs
 * the parallel scan + merge/rerank. The Worker holds no Turso
 * credentials and keeps no shard list.
 *
 * Reachability: Cloudflare Tunnel hostname + a Cloudflare Access
 * service token (the two `CF-Access-*` headers); the service's own
 * client bearer token rides `Authorization`.
 */
import { CEILING, CONTEXT, validateEmbed, type Ceiling, type DisclosureContext, type EmbedResponse } from "./space";

export interface ServiceEnv {
  SERVICE_BASE: string;
  SERVICE_TOKEN: string;
  ACCESS_CLIENT_ID: string;
  ACCESS_CLIENT_SECRET: string;
}

export class ServiceError extends Error {
  status: number;
  constructor(status: number, message: string) {
    super(message);
    this.status = status;
  }
}

function serviceHeaders(env: ServiceEnv): HeadersInit {
  return {
    "Content-Type": "application/json",
    Authorization: `Bearer ${env.SERVICE_TOKEN}`,
    "CF-Access-Client-Id": env.ACCESS_CLIENT_ID,
    "CF-Access-Client-Secret": env.ACCESS_CLIENT_SECRET,
  };
}

async function postJson(env: ServiceEnv, path: string, body: unknown): Promise<unknown> {
  let resp: Response;
  try {
    resp = await fetch(`${env.SERVICE_BASE}${path}`, {
      method: "POST",
      headers: serviceHeaders(env),
      body: JSON.stringify(body),
    });
  } catch (e) {
    throw new ServiceError(502, `search service unreachable: ${String(e)}`);
  }
  const text = await resp.text();
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    throw new ServiceError(502, `search service returned non-JSON (${resp.status})`);
  }
  if (!resp.ok) {
    const msg = (parsed as { error?: { message?: string } })?.error?.message ?? `search service HTTP ${resp.status}`;
    throw new ServiceError(502, msg);
  }
  return parsed;
}

/** A content-by-hash read the seam refused to serve: malformed (400) or absent/undisclosed (404). Withheld and missing content share the 404 shape by server contract, so the two are deliberately indistinguishable here — the caller falls back without learning which. */
export class ContentMiss extends Error {
  status: 400 | 404;
  constructor(status: 400 | 404, message: string) {
    super(message);
    this.status = status;
  }
}

/** The server's hash-shape rule (server.rs content route): 64 lowercase hex. */
export function isContentHash(value: string): boolean {
  return value.length === 64 && /^[0-9a-f]{64}$/.test(value);
}

async function sha256Hex(text: string): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text));
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

/**
 * Hydrate one content-addressed payload through the merged
 * content-by-hash seam (#13105 slice 3; service route from PR
 * #15901). Auth reuses the existing service headers exactly —
 * the hash alone is not authorization: the service still
 * requires the client bearer (whose ceiling_cap bounds the
 * disclosure check) and answers 401 without it.
 *
 * Failure posture, mirroring searchService above: a 400/404 is
 * a ContentMiss the caller turns into its fallback path (never
 * an exposed byte); anything else — network, non-JSON,
 * non-object, or a payload whose text does not hash back to the
 * requested hash — is a bad upstream (502), never served.
 */
export async function fetchContentByHash(
  env: ServiceEnv,
  hash: string,
): Promise<{ payload: Record<string, unknown>; ms: number }> {
  const t0 = Date.now();
  if (!isContentHash(hash)) {
    throw new ContentMiss(400, "content hash must be 64 lowercase hexadecimal characters");
  }
  let resp: Response;
  try {
    resp = await fetch(`${env.SERVICE_BASE}/v1/content/${hash}`, { headers: serviceHeaders(env) });
  } catch (e) {
    throw new ServiceError(502, `content service unreachable: ${String(e)}`);
  }
  const text = await resp.text();
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    throw new ServiceError(502, `content service returned non-JSON (${resp.status})`);
  }
  if (resp.status === 400 || resp.status === 404) {
    const msg = (parsed as { error?: { message?: string } })?.error?.message ?? `content unavailable (${resp.status})`;
    throw new ContentMiss(resp.status, msg);
  }
  if (!resp.ok) {
    const msg = (parsed as { error?: { message?: string } })?.error?.message ?? `content service HTTP ${resp.status}`;
    throw new ServiceError(502, msg);
  }
  // Same untrusted-response rule as searchService (#6982): only a
  // JSON object can carry a hydration payload.
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    throw new ServiceError(502, "content service returned a non-object response");
  }
  const payload = parsed as Record<string, unknown>;
  // The hash is the version: the served bytes must hash back to
  // the requested hash (the server guarantees this; the check
  // here keeps a mismatched upstream from ever being served as a
  // Class A hit).
  if (typeof payload.text !== "string" || (await sha256Hex(payload.text)) !== hash) {
    throw new ServiceError(502, "content service returned bytes that do not match the requested hash");
  }
  return { payload, ms: Date.now() - t0 };
}

/** Embed one query in the pinned space; the response is validated. */
export async function embedQuery(env: ServiceEnv, query: string): Promise<{ embed: EmbedResponse; ms: number }> {
  const t0 = Date.now();
  const raw = await postJson(env, "/v1/embed", { query });
  const embed = validateEmbed(raw);
  return { embed, ms: Date.now() - t0 };
}

/**
 * Run one fan-out search under the caller's ceiling, stating its
 * disclosure context (#12622) so the service never resolves its
 * own default for the request. The service clamps the requested
 * ceiling to this client's cap; we still verify the reported
 * effective ceiling equals what was requested before the result
 * may be cached — a result computed above the caller's ceiling
 * must never enter a Durable Object, and a result computed below
 * it must never be served as if it satisfied a higher grant.
 */
export async function searchService(
  env: ServiceEnv,
  query: string,
  k: number,
  ceiling: Ceiling = CEILING,
  context: DisclosureContext = CONTEXT,
): Promise<{ result: Record<string, unknown>; ms: number }> {
  const t0 = Date.now();
  const raw: unknown = await postJson(env, "/v1/search", {
    query,
    ceiling,
    context,
    k,
  });
  // The response is untrusted until validated, exactly like the embed
  // door (validateEmbed in space.ts): a JSON null 200 cannot even be
  // dereferenced for the ceiling check, and any other non-object shape
  // has no ceiling to verify - both are a bad upstream (502), never a
  // raw TypeError out of this client (#6982).
  if (typeof raw !== "object" || raw === null || Array.isArray(raw)) {
    const shape = raw === null ? "null" : Array.isArray(raw) ? "array" : typeof raw;
    throw new ServiceError(502, `search service returned a non-object response (${shape})`);
  }
  const result = raw as Record<string, unknown>;
  if (result.effective_ceiling !== ceiling) {
    throw new ServiceError(
      502,
      `search service reported effective_ceiling ${JSON.stringify(result.effective_ceiling)}, expected ${ceiling}`,
    );
  }
  return { result, ms: Date.now() - t0 };
}
