import type { QuotaProviderResult } from "./quota-gather-map.ts";

export interface OpenCodeServerAuth {
  username: string;
  password: string;
}

export interface OpenCodeProviderQuotaOptions {
  baseUrl: string;
  auth: OpenCodeServerAuth;
  providerId: string;
  fetchImpl?: typeof fetch;
}

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

/**
 * Ask OpenCode to answer one provider quota request with its own credential.
 * A null result means the server/route/credential is not available, allowing
 * the caller to retain its existing source during a staged rollout. A valid
 * provider response, including an upstream error result, is authoritative.
 */
export async function fetchOpenCodeProviderQuota(
  options: OpenCodeProviderQuotaOptions,
): Promise<QuotaProviderResult | null> {
  const fetchImpl = options.fetchImpl ?? fetch;
  const token = Buffer.from(`${options.auth.username}:${options.auth.password}`).toString("base64");
  let response: Response;
  try {
    response = await fetchImpl(
      `${options.baseUrl.replace(/\/+$/, "")}/api/experimental/credential/provider/${encodeURIComponent(options.providerId)}/quota`,
      {
        headers: { Accept: "application/json", Authorization: `Basic ${token}` },
        signal: AbortSignal.timeout(20_000),
      },
    );
  } catch {
    return null;
  }
  if (!response.ok) return null;

  let body: unknown;
  try {
    body = await response.json();
  } catch {
    return null;
  }
  const data = isRecord(body) && isRecord(body.data) ? body.data : null;
  if (
    !data ||
    data.providerId !== options.providerId ||
    typeof data.providerName !== "string" ||
    typeof data.ok !== "boolean" ||
    typeof data.configured !== "boolean" ||
    typeof data.fetchedAt !== "number" ||
    !(data.usage === null || isRecord(data.usage)) ||
    (data.error !== undefined && typeof data.error !== "string")
  ) {
    return null;
  }
  return data as unknown as QuotaProviderResult;
}
