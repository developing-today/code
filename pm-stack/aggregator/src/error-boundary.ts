// #1211: the aggregator's error-boundary responses. One place decides
// what a failed or unmatched request looks like, so page routes and
// API routes cannot drift apart again — the inconsistency this module
// exists to close: /item and /quota/service misses used to render
// "Not found" at HTTP 200, unknown page paths used to get the API's
// JSON 404, and a throwing handler used to reject the fetch outright
// (Bun.serve turns that into an unstyled connection-level failure
// with no render-health record).
//
// Bodies follow the server's existing fragment style: the bare
// <h1> + muted-paragraph shape the #1186 program 404 already renders.
// This module is deliberately free of server state so it is
// unit-testable without booting a server (the path-segment.ts
// precedent).

export function isApiPath(path: string): boolean {
  return path.startsWith("/api/");
}

/** The consistent not-found response: styled HTML 404 for page
 *  routes, JSON 404 for /api routes. */
export function notFoundResponse(path: string): Response {
  if (isApiPath(path)) {
    return new Response(JSON.stringify({ error: "not found", path }, null, 2), {
      status: 404,
      headers: { "content-type": "application/json; charset=utf-8" },
    });
  }
  return new Response(`<h1>Not found</h1><p class="muted">no page at <code>${path}</code></p>`, {
    status: 404,
    headers: { "content-type": "text/html; charset=utf-8" },
  });
}

/** The top-level error boundary's response for a handler throw:
 *  JSON 500 for /api routes, a styled 500 page for page routes. */
export function errorBoundaryResponse(path: string, err: unknown): Response {
  if (isApiPath(path)) {
    const message = err instanceof Error ? err.message : String(err);
    return new Response(JSON.stringify({ error: `internal error: ${message}` }, null, 2), {
      status: 500,
      headers: { "content-type": "application/json; charset=utf-8" },
    });
  }
  return new Response(
    `<h1>Something went wrong</h1><p class="muted">this page failed to render — the failure is recorded, and other pages are unaffected. <a href="/health">health</a></p>`,
    { status: 500, headers: { "content-type": "text/html; charset=utf-8" } },
  );
}
