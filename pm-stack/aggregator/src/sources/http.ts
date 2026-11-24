// Shared JSON-over-HTTP helper for the live sources (split from live.ts
// for the production line limit, #12671).

export interface FetchResult {
  ok: boolean;
  json: unknown;
}

export async function getJson(
  url: string,
  headers: Record<string, string> = {},
  init: RequestInit = {},
): Promise<FetchResult> {
  try {
    const res = await fetch(url, { headers, signal: AbortSignal.timeout(8000), ...init });
    if (!res.ok) return { ok: false, json: { status: res.status } };
    return { ok: true, json: await res.json() };
  } catch (err) {
    return { ok: false, json: { error: String(err) } };
  }
}
