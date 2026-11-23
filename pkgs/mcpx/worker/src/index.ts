// mcpx Worker (#13657): the public MCP front door for the
// self-hosted mcpx endpoint.
//
// Shape:
//   AI client -> https://mcpx.security.cab/mcp
//     Hosted clients (Claude, ChatGPT) connect from their own servers
//     and cannot pass a Cloudflare Access login here, so /mcp is
//     guarded by this Worker's OAuth 2.1 authorization server
//     (src/oauth.ts): a Bearer access token issued through /authorize,
//     or the machine bearer for the owner's own tooling. The one path
//     still behind Access is /authorize — only the owner's Google
//     identity reaches the approval page.
//   this Worker -> https://mcpx-link.security.cab/mcp
//     A second Access app guards the bridge door with a service-token-
//     only policy; the Worker presents that token (ACCESS_CLIENT_ID /
//     ACCESS_CLIENT_SECRET, uploaded as per-version secrets).
//   bridge door -> sandbox-link Cloudflare Tunnel ->
//     http://localhost:8932 bridge proxy on the laptop -> the mcpx
//     daemon's /mcp endpoint, discovered via the daemon state
//     descriptor. The daemon is loopback-only and unauthenticated;
//     this door is its security boundary, and /v1/* is never proxied.
//
// Bare-path equivalence: requests to / are handled exactly as /mcp —
// same auth gate, same proxying (the upstream path is always /mcp).
//
// The /mcp proxy is a transparent streamable-HTTP proxy: method,
// headers and body pass through, and the response (including the
// Mcp-Session-Id header and any SSE stream) passes back untouched.
// The one query parameter the door consumes itself is `ns`: it is
// parsed once, in the router, into a per-request filter selector
// (src/filter.ts), stripped before proxying, and handed to the
// proxy/filter layer; every other query parameter passes through.

import {
  authorizationServerMetadata,
  authorizeGet,
  authorizePost,
  isValidAccessToken,
  protectedResourceMetadata,
  registerClient,
  timingSafeEqual,
  tokenEndpoint,
  type OAuthEnv,
} from "./oauth";
import {
  filterMcpResponse,
  gateMcpRequestBody,
  jsonRpcError,
  mergeBatchRejections,
  resolveFilterSelector,
  type FilterSelector,
  type JsonRpcMessage,
} from "./filter";

export interface Env extends OAuthEnv {
  BRIDGE_BASE: string;
  ACCESS_CLIENT_ID: string;
  ACCESS_CLIENT_SECRET: string;
  // Bearer for the owner's machine clients (mcpx, curl): a Worker
  // secret staged by the publish workflow from the GitHub environment
  // secret of the same name; canonical value in fleet secrets as
  // mcpx.machine-bearer.
  MACHINE_BEARER: string;
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

async function isAuthorizedMcpCaller(request: Request, env: Env): Promise<boolean> {
  const authorization = request.headers.get("authorization");
  if (!authorization?.startsWith("Bearer ")) return false;
  const token = authorization.slice("Bearer ".length).trim();
  if (!token) return false;
  if (await isValidAccessToken(env, token)) return true;
  const machineBearer = (env.MACHINE_BEARER ?? "").trim();
  return machineBearer.length > 0 && timingSafeEqual(token, machineBearer);
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);
    const path = url.pathname;

    if (path === "/healthz") {
      return json({ ok: true, service: "mcpx" });
    }

    // OAuth authorization-server surface (src/oauth.ts).
    if (path === "/.well-known/oauth-protected-resource" || path === "/.well-known/oauth-protected-resource/mcp") {
      return protectedResourceMetadata();
    }
    if (path === "/.well-known/oauth-authorization-server") {
      return authorizationServerMetadata();
    }
    if (path === "/register") {
      if (request.method !== "POST") return json({ error: "method not allowed" }, 405);
      return registerClient(request, env);
    }
    if (path === "/authorize") {
      if (request.method === "GET") return authorizeGet(request, env);
      if (request.method === "POST") return authorizePost(request, env);
      return json({ error: "method not allowed" }, 405);
    }
    if (path === "/token") {
      if (request.method !== "POST") return json({ error: "method not allowed" }, 405);
      return tokenEndpoint(request, env);
    }

    // Bare-path equivalence: / is the same door as /mcp.
    if (path !== "/mcp" && path !== "/") {
      return json({ error: "not found" }, 404);
    }

    if (!(await isAuthorizedMcpCaller(request, env))) {
      return new Response(JSON.stringify({ error: "unauthorized" }), {
        status: 401,
        headers: {
          "content-type": "application/json",
          "WWW-Authenticate":
            'Bearer resource_metadata="https://mcpx.security.cab/.well-known/oauth-protected-resource"',
        },
      });
    }
    if (!env.ACCESS_CLIENT_ID || !env.ACCESS_CLIENT_SECRET) {
      return json({ error: "bridge credentials are not configured" }, 500);
    }

    // The filter selector is computed in exactly one place — here,
    // from the `ns` query parameter — and handed to the proxy/filter
    // layer (src/filter.ts).
    const selector = resolveFilterSelector(url);
    return proxyMcp(request, env, selector);
  },
};

// Transparent streamable-HTTP proxy to the bridge door. The upstream
// path is always /mcp (bare-path equivalence), the caller's own
// credentials are swapped for the Worker's bridge egress token, and
// the `ns` selector parameter is consumed here, never forwarded.
export async function proxyMcp(
  request: Request,
  env: Env,
  selector: FilterSelector,
): Promise<Response> {
  const url = new URL(request.url);
  const upstreamParams = new URLSearchParams(url.search);
  upstreamParams.delete("ns");
  // The code-mode surface is the only surface this door offers; drop
  // any caller request for the flat surface.
  upstreamParams.delete("codemode");
  const query = upstreamParams.toString();
  const target = new URL("/mcp" + (query ? `?${query}` : ""), env.BRIDGE_BASE);
  const headers = new Headers(request.headers);
  headers.delete("host");
  // The caller's own credentials are for the public door only; the
  // bridge door gets the Worker's egress token instead.
  headers.delete("authorization");
  headers.delete("cf-access-jwt-assertion");
  headers.delete("cf-access-client-id");
  headers.delete("cf-access-client-secret");
  headers.set("CF-Access-Client-Id", env.ACCESS_CLIENT_ID);
  headers.set("CF-Access-Client-Secret", env.ACCESS_CLIENT_SECRET);

  // The body is inspected before proxying so disallowed tools/call
  // requests are rejected without ever reaching the daemon.
  let bodyText: string | undefined;
  let allowed: JsonRpcMessage[] = [];
  let rejected: JsonRpcMessage[] = [];
  if (request.method !== "GET" && request.method !== "HEAD") {
    const raw = await request.text();
    const gate = gateMcpRequestBody(raw, selector);
    if (gate) {
      bodyText = gate.bodyText;
      allowed = gate.allowed;
      rejected = gate.rejected;
      // Every element was rejected: answer here, call nothing upstream.
      if (allowed.length === 0) {
        const body = gate.batch
          ? JSON.stringify(rejected)
          : JSON.stringify(rejected[0] ?? jsonRpcError(null));
        headers.delete("content-length");
        headers.delete("content-type");
        return new Response(body, {
          status: 200,
          headers: { "content-type": "application/json" },
        });
      }
    } else {
      bodyText = raw;
    }
  }

  const init: RequestInit = {
    method: request.method,
    headers,
    redirect: "manual",
  };
  if (bodyText !== undefined) {
    init.body = bodyText;
  }

  const upstream = await fetch(target, init);
  const response = new Response(upstream.body, {
    status: upstream.status,
    statusText: upstream.statusText,
    headers: new Headers(upstream.headers),
  });
  if (rejected.length > 0) {
    return mergeBatchRejections(response, selector, allowed, rejected);
  }
  return filterMcpResponse(response, selector, allowed);
}
