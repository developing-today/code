import { Container, getContainer } from "@cloudflare/containers";

// Official Cloudflare hello-world image (Docker Hub cloudflare/hello-world,
// amd64 digest copied byte-for-byte into our account registry as :v1).
export class HelloContainer extends Container {
  defaultPort = 8000;
  sleepAfter = "2m";
}

// Probe: the Lab Ops Hub crawler image (Nix-built Python app) hosted in a
// FRESH container application, to isolate image vs application faults.
export class CrawlerProbe extends Container {
  defaultPort = 8080;
  sleepAfter = "2m";

  override onError(error: unknown) {
    console.log("CrawlerProbe error:", error);
  }
}

// Probe: official image but with instance_type "lite" (as Lab Ops Hub uses).
export class HelloLite extends Container {
  defaultPort = 8000;
  sleepAfter = "2m";
}

interface Env {
  HELLO_CONTAINER: DurableObjectNamespace<HelloContainer>;
  CRAWLER_PROBE: DurableObjectNamespace<CrawlerProbe>;
  HELLO_LITE: DurableObjectNamespace<HelloLite>;
  // Worker secret (`wrangler secret put API_TOKEN`). Every route that
  // forwards to a container requires it as a Bearer token (#955, mirroring
  // the lab-ops-hub gate, #441). Optional in the type only so a
  // misconfigured deploy compiles — at runtime the gate fails CLOSED
  // (503) when the secret is unset. Before this gate, the Worker forwarded
  // /probe to the crawler container with no auth at all, and the crawler
  // copy it fronted fetched arbitrary URLs (SSRF / file:// fetch).
  API_TOKEN?: string;
}

// --- API auth (#955) ---------------------------------------------------------
// Every container-forwarding route is gated: the crawler container is a
// fetch proxy by design, and the hello containers burn container minutes,
// so none of them may be reachable unauthenticated. Only /worker-marker
// (a plain Worker response that touches no container) stays public, so
// Worker-served vs container-served remains distinguishable without a
// token. Header-only on purpose: no query-string token, so the secret
// never lands in URLs/logs.

async function timingSafeTokenEqual(given: string, expected: string): Promise<boolean> {
  const enc = new TextEncoder();
  const [a, b] = await Promise.all([
    crypto.subtle.digest("SHA-256", enc.encode(given)),
    crypto.subtle.digest("SHA-256", enc.encode(expected)),
  ]);
  const av = new Uint8Array(a);
  const bv = new Uint8Array(b);
  let diff = 0;
  for (let i = 0; i < av.length; i++) diff |= av[i] ^ bv[i];
  return diff === 0;
}

async function isAuthorized(req: Request, env: Env): Promise<boolean> {
  const expected = env.API_TOKEN;
  if (!expected) return false;
  const header = req.headers.get("authorization");
  const bearer = header?.toLowerCase().startsWith("bearer ") ? header.slice(7).trim() : null;
  const given = bearer ?? req.headers.get("x-api-token");
  return given !== null && given.length > 0 && timingSafeTokenEqual(given, expected);
}

const json = (data: unknown, status = 200) =>
  new Response(JSON.stringify(data, null, 2), {
    status,
    headers: { "content-type": "application/json" },
  });

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const pathname = new URL(request.url).pathname;

    if (pathname === "/worker-marker") {
      return new Response("served-by-worker\n");
    }

    // Fail closed: a redeploy that forgets `wrangler secret put API_TOKEN`
    // exposes nothing (503), and an unauthenticated caller gets 401 before
    // any container is started.
    if (!env.API_TOKEN) {
      return json({ error: "API auth is not configured: set the API_TOKEN Worker secret" }, 503);
    }
    if (!(await isAuthorized(request, env))) {
      return json({ error: "unauthorized: send Authorization: Bearer <API_TOKEN>" }, 401);
    }

    if (pathname.startsWith("/probe")) {
      return getContainer(env.CRAWLER_PROBE, "probe-2").fetch(request);
    }

    if (pathname.startsWith("/lite")) {
      return getContainer(env.HELLO_LITE, "lite-1").fetch(request);
    }

    const name = pathname.startsWith("/name/") ? decodeURIComponent(pathname.slice("/name/".length)) : undefined;
    const container = name ? getContainer(env.HELLO_CONTAINER, name) : getContainer(env.HELLO_CONTAINER);
    return container.fetch(request);
  },
};
