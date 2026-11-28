// desktop-commander Worker (#13404): the public MCP front door for the
// self-hosted Desktop Commander endpoint.
//
// Shape:
//   AI client -> https://commander.security.cab/mcp
//     Hosted clients (Claude, ChatGPT) connect from their own servers
//     and cannot pass a Cloudflare Access login here, so /mcp is
//     guarded by this Worker's OAuth 2.1 authorization server
//     (src/oauth.ts): a Bearer access token issued through /authorize,
//     or the machine bearer for the owner's own tooling. The one path
//     still behind Access is /authorize — only the owner's Google
//     identity reaches the approval page.
//   this Worker -> https://commander-link.security.cab/mcp
//     A second Access app guards the bridge door with a service-token-
//     only policy; the Worker presents that token (ACCESS_CLIENT_ID /
//     ACCESS_CLIENT_SECRET, uploaded as per-version secrets).
//   bridge door -> Cloudflare Tunnel -> supergateway on the laptop
//     -> the packaged Desktop Commander (stdio) on 127.0.0.1:8931.
//
// The /mcp proxy is a transparent streamable-HTTP proxy: method, path,
// query, headers and body pass through, and the response (including
// the Mcp-Session-Id header and any SSE stream) passes back untouched.

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

export interface Env extends OAuthEnv {
  BRIDGE_BASE: string;
  ACCESS_CLIENT_ID: string;
  ACCESS_CLIENT_SECRET: string;
  // Bearer for the owner's machine clients (mcpx, curl): a Worker
  // secret staged by the publish workflow from the GitHub environment
  // secret of the same name; canonical value in fleet secrets as
  // desktop-commander.machine-bearer.
  MACHINE_BEARER: string;
  // The commit this deployment was built from, stamped per deploy as
  // a plain-text var (#15793 criterion 1); absent/blank when the deploy
  // was unstamped, in which case /version answers "unknown".
  VERSION?: string;
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

/** The #15793 criterion-1 answer: the deployed commit from the
 * VERSION var (stamped per deploy), or "unknown" unstamped — the
 * `version`-field shape the edge probe reads (commitOf in
 * pm-stack/console/edge/src/worker-checks.ts). */
export function versionAnswer(env: Env): { ok: true; service: string; version: string } {
  const version = typeof env.VERSION === "string" ? env.VERSION.trim() : "";
  return { ok: true, service: "desktop-commander", version: version || "unknown" };
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

    // #15793 criterion 1: unauthenticated, ahead of every other
    // route — it reads no state and no secret.
    if (path === "/version") {
      if (request.method !== "GET") return json({ error: "method not allowed" }, 405);
      return json(versionAnswer(env));
    }

    if (path === "/healthz") {
      return json({ ok: true, service: "desktop-commander" });
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

    if (path !== "/mcp") {
      return json({ error: "not found" }, 404);
    }

    if (!(await isAuthorizedMcpCaller(request, env))) {
      return new Response(JSON.stringify({ error: "unauthorized" }), {
        status: 401,
        headers: {
          "content-type": "application/json",
          "WWW-Authenticate":
            'Bearer resource_metadata="https://commander.security.cab/.well-known/oauth-protected-resource"',
        },
      });
    }
    if (!env.ACCESS_CLIENT_ID || !env.ACCESS_CLIENT_SECRET) {
      return json({ error: "bridge credentials are not configured" }, 500);
    }

    const target = new URL(url.pathname + url.search, env.BRIDGE_BASE);
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
