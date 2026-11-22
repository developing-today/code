// The Worker in front of the public hostname.
//
// It hands every request to entry.ts, which decides, and gives it the
// three things only a Worker on the hostname's own route has: the object
// that holds the state, the app's files, and the host behind the tunnel.
//
// If anything here throws, the request goes to the host exactly as it
// did before this Worker existed. The worst this layer can do is add
// nothing.
import { DurableObject } from "cloudflare:workers";
import { Edge } from "./edge.ts";
import { route } from "./entry.ts";
import { readOpenPullRequests } from "./github.ts";

interface Env {
  PM: DurableObjectNamespace;
  ASSETS: { fetch(request: Request): Promise<Response> };
  PUBLIC_HOST: string;
  /** Optional read-only GitHub token; provisioned as a Worker secret. */
  GITHUB_TOKEN?: string;
  /** Explicit opt-in; reads stay disabled unless set to `on`. */
  GITHUB_READ_ENABLED?: string;
  VERSION?: string;
  /** Local development only: where the host is, when it is not simply
   * behind this hostname's route. Never set where this is deployed. */
  HOST_ORIGIN?: string;
}

/** The object with the state. */
export class PmEdge extends DurableObject<Env> {
  private edge: Edge;

  constructor(ctx: DurableObjectState, env: Env) {
    super(ctx, env);
    this.edge = new Edge({
      kv: {
        get: (key) => ctx.storage.get(key),
        put: (key, value) => ctx.storage.put(key, value),
        delete: async (key) => {
          await ctx.storage.delete(key);
        },
      },
      setAlarm: (at) => ctx.storage.setAlarm(at),
      githubReadEnabled: env.GITHUB_READ_ENABLED === "on",
      githubTokenConfigured: Boolean(env.GITHUB_TOKEN),
      freshRead: env.GITHUB_READ_ENABLED === "on" && env.GITHUB_TOKEN
        ? (section, config) =>
            section === "prs"
              ? readOpenPullRequests(config.repo, env.GITHUB_TOKEN as string)
              : Promise.reject(new Error("unsupported edge read"))
        : undefined,
    });
  }

  fetch(request: Request): Promise<Response> {
    return this.edge.fetch(request);
  }

  alarm(): Promise<void> {
    return this.edge.alarm();
  }
}

/** The host answered, as opposed to something in front of it saying it
 * could not be reached. The host marks every response it writes. */
function fromHost(res: Response): boolean {
  return res.headers.get("x-pm-console") === "1" || res.status < 500;
}

/** Ask the host. `ms` is how long it has to start answering; a stream it
 * then keeps open (the dashboard's live pages) is left to run. */
async function host(request: Request, ms: number, origin?: string): Promise<Response | null> {
  const abort = new AbortController();
  const timer = setTimeout(() => abort.abort(), ms);
  try {
    // On the hostname's own route, fetching the request is asking the host.
    const url = new URL(request.url);
    const target = origin ? new Request(`${origin}${url.pathname}${url.search}`, request) : request;
    const res = await fetch(new Request(target, { redirect: "manual", signal: abort.signal }));
    return fromHost(res) ? res : null;
  } catch {
    return null;
  } finally {
    clearTimeout(timer);
  }
}

export default {
  async fetch(request: Request, env: Env, ctx: ExecutionContext): Promise<Response> {
    ctx.passThroughOnException();
    const stub = env.PM.get(env.PM.idFromName("pm"));
    return route(request, {
      state: (r) => stub.fetch(r),
      asset: (path) => env.ASSETS.fetch(new Request(`https://assets.internal${path}`)),
      host: (r, ms) => host(r, ms, env.HOST_ORIGIN),
      publicHost: env.PUBLIC_HOST,
      version: env.VERSION ?? null,
      later: (work) => ctx.waitUntil(work),
      now: Date.now,
    });
  },
};
