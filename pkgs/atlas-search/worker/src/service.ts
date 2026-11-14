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
import { CEILING, validateEmbed, type EmbedResponse } from "./space";

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

/** Embed one query in the pinned space; the response is validated. */
export async function embedQuery(env: ServiceEnv, query: string): Promise<{ embed: EmbedResponse; ms: number }> {
  const t0 = Date.now();
  const raw = await postJson(env, "/v1/embed", { query });
  const embed = validateEmbed(raw);
  return { embed, ms: Date.now() - t0 };
}

/**
 * Run one fan-out search under the personal ceiling. The service
 * clamps the requested ceiling to this client's cap; we still
 * verify the reported effective ceiling before the result may be
 * cached — a result computed above `personal` must never enter a
 * Durable Object.
 */
export async function searchService(
  env: ServiceEnv,
  query: string,
  k: number,
): Promise<{ result: Record<string, unknown>; ms: number }> {
  const t0 = Date.now();
  const raw: unknown = await postJson(env, "/v1/search", {
    query,
    ceiling: CEILING,
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
  if (result.effective_ceiling !== CEILING) {
    throw new ServiceError(
      502,
      `search service reported effective_ceiling ${JSON.stringify(result.effective_ceiling)}, expected ${CEILING}`,
    );
  }
  return { result, ms: Date.now() - t0 };
}
