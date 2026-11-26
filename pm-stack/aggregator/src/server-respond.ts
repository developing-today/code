// The server's Response constructors. Split out of server.ts (#13052).
import { openMetadataStore, type EpicSlugMap, type IngestionItem } from "./metadata-ingestion.ts";
import { createMetadataWebhookHandler } from "./webhook-listener.ts";
import { SW_SOURCE } from "./sw-source.ts";

export function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body, null, 2), {
    status,
    headers: { "content-type": "application/json; charset=utf-8" },
  });
}

/** Same response shape as {@link json} without the 2-space pretty-print:
 *  the board JSON legitimately carries seconds of data and the pretty
 *  form adds ~10% bytes for nothing a machine consumer reads (#5082). */
export function jsonCompact(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json; charset=utf-8" },
  });
}

export function html(body: string, status = 200): Response {
  return new Response(body, { status, headers: { "content-type": "text/html; charset=utf-8" } });
}

// The real service worker (#3676 — design 2026-11-01T03-54-00Z §6
// step 2): push + notificationclick handlers, with the kill switch's
// eviction job carried forward per the §6 fallback (cache purge on
// activate in the worker; foreign-registration unregister in the
// page-side probe script). Worker source + history: src/sw-source.ts.
export function serviceWorkerResponse(): Response {
  return new Response(SW_SOURCE, {
    headers: { "content-type": "text/javascript; charset=utf-8", "cache-control": "no-cache" },
  });
}
