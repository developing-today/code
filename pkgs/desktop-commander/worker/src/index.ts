// desktop-commander Worker (#13404): the public MCP front door for the
// self-hosted Desktop Commander endpoint.
//
// Shape:
//   AI client -> https://commander.security.cab/mcp
//     Cloudflare Access app "desktop-commander" authenticates the caller
//     first (owner Google identity, or the desktop-commander service
//     token for machine clients). Unauthenticated requests never reach
//     this Worker.
//   this Worker -> https://commander-link.security.cab/mcp
//     A second Access app guards the bridge door with a service-token-
//     only policy; the Worker presents that token (ACCESS_CLIENT_ID /
//     ACCESS_CLIENT_SECRET, uploaded as per-version secrets).
//   bridge door -> Cloudflare Tunnel -> supergateway on the laptop
//     -> the packaged Desktop Commander (stdio) on 127.0.0.1:8931.
//
// The Worker is a transparent streamable-HTTP proxy: method, path,
// query, headers and body pass through, and the response (including
// the Mcp-Session-Id header and any SSE stream) passes back untouched.

export interface Env {
  BRIDGE_BASE: string;
  ACCESS_CLIENT_ID: string;
  ACCESS_CLIENT_SECRET: string;
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);

    if (url.pathname === "/healthz") {
      return json({ ok: true, service: "desktop-commander" });
    }
    if (url.pathname !== "/mcp") {
      return json({ error: "not found" }, 404);
    }

    // Defense in depth: Access injects this header only after a caller
    // passes one of the app's policies. Its absence means the request
    // did not come through the gate (a stray route or a future
    // workers.dev re-enable), so refuse before touching the bridge.
    if (!request.headers.get("Cf-Access-Jwt-Assertion")) {
      return json({ error: "missing Cloudflare Access assertion" }, 401);
    }
    if (!env.ACCESS_CLIENT_ID || !env.ACCESS_CLIENT_SECRET) {
      return json({ error: "bridge credentials are not configured" }, 500);
    }

    const target = new URL(url.pathname + url.search, env.BRIDGE_BASE);
    const headers = new Headers(request.headers);
    headers.delete("host");
    // The caller's own Access material is for the public door only;
    // the bridge door gets the Worker's egress token instead.
    headers.delete("cf-access-jwt-assertion");
    headers.delete("cf-access-client-id");
    headers.delete("cf-access-client-secret");
    headers.set("CF-Access-Client-Id", env.ACCESS_CLIENT_ID);
    headers.set("CF-Access-Client-Secret", env.ACCESS_CLIENT_SECRET);

    const init: RequestInit = {
      method: request.method,
      headers,
      redirect: "manual",
    };
    if (request.method !== "GET" && request.method !== "HEAD") {
      init.body = request.body;
    }

    const upstream = await fetch(target, init);
    return new Response(upstream.body, {
      status: upstream.status,
      statusText: upstream.statusText,
      headers: new Headers(upstream.headers),
    });
  },
};
