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
import { probeWorkers, selfCheck, type WorkerCheckTarget } from "./worker-checks.ts";
import { unread } from "../../src/timed.ts";

interface Env {
  PM: DurableObjectNamespace;
  ASSETS: { fetch(request: Request): Promise<Response> };
  PUBLIC_HOST: string;
  VERSION?: string;
  /** What lets the edge do the core itself. The tokens are Worker
   * secrets, put there by scripts/expose.py on a machine that holds the
   * secret store; the flag is the switch, off unless it reads `on`. */
  GITHUB_TOKEN?: string;
  QUEUE_TOKEN?: string;
  EDGE_DIRECT?: string;
  /** The message bridge, by service binding (#14373), and the PM
   * stack's own bridge credential: a Worker secret like the two
   * tokens, put there by scripts/expose.py --edge-direct from the
   * vault. With both, the edge sends the owner's messages itself, as
   * service:pm-stack, and reads the agents list and the message log
   * from the bridge; without either, those stay a machine's. */
  BRIDGE?: { fetch(request: Request): Promise<Response> };
  BRIDGE_TOKEN?: string;
  /** The platform's other Workers, by service binding, for the
   * scheduled checks (#15793): the edge asks each for the commit it
   * was deployed from and keeps the answers in the object. A binding
   * that is not configured simply leaves that Worker out of the
   * round; the bindings are declared in wrangler.jsonc. */
  PM_SERVICE?: { fetch(request: Request): Promise<Response> };
  DECISION_EVENTS?: { fetch(request: Request): Promise<Response> };
  /** The decision-events Worker (#13491): its address and the shared
   * secret its reads take, the secret a Worker secret like the two
   * tokens. With both, the edge reads the Questions discovery section
   * (the sets the Worker holds, and each set's state) itself, with no
   * machine; without either, that section stays a machine's. */
  DECISION_EVENTS_URL?: string;
  DECISION_EVENTS_SECRET?: string;
  /** The fleet-platform ledger the hosted M4 ledger surfaces read
   * (#14968), and their opt-in flag: off unless it reads `on`, and
   * only after the #14865 gate check recorded on #14968. Which store
   * answers is LEDGER_STORE: `d1` (the owner ruling of 2026-11-26 on
   * #14866) reads the LEDGER_DB binding — the D1 database named
   * fleet-platform; anything else reads the Turso ledger at
   * PM_LEDGER_DB_URL, whose token is a Worker secret, put there by
   * scripts/expose.py from the vault (name:
   * turso.fleet-platform-rw-token). The Turso arm is the rollback:
   * the same var, flipped back. */
  PM_LEDGER_DB_URL?: string;
  PM_LEDGER_DB_TOKEN?: string;
  LEDGER_PAGES?: string;
  LEDGER_STORE?: string;
  LEDGER_DB?: import("./ledger-pages.ts").LedgerD1Database;
  /** Hosted /dns (#14968 slice 2) and its opt-in flag: off unless it
   * reads `on`. The surface reads the #902 register snapshot the
   * collector publishes into the same fleet-platform D1 database, so
   * it shares the LEDGER_DB binding; the flag is deliberately not
   * set in wrangler.jsonc / cloudflare.config.ts by the slice that
   * lands the reader — enabling it is the follow-up flip. */
  DNS_PAGES?: string;
  /** Local development only: where GitHub's API and the queue store
   * are, when they are stand-ins. Never set where this is deployed: the
   * Worker's tokens are sent to whatever these name. */
  GITHUB_API_ORIGIN?: string;
  QUEUE_DB_ORIGIN?: string;
  /** Local development only: where the host is, when it is not simply
   * behind this hostname's route. Never set where this is deployed. */
  HOST_ORIGIN?: string;
  /** Local development only: how often the object's own clock wakes,
   * in milliseconds, so a smoke run can watch an alarm fire instead
   * of waiting out the minute. Never set where this is deployed. */
  OWN_CLOCK_MS?: string;
  /** Local development only: a sign-in issuer's key set, named by its
   * real (https) address in KEYS_FROM, is fetched from KEYS_TO
   * instead -- a stand-in on this machine serving the keys of a
   * sign-in the run made itself, so a smoke run can sign the owner's
   * token. The issuer's address in the settings is never rewritten;
   * only this Worker's fetch of it is. Never set where deployed. */
  KEYS_FROM?: string;
  KEYS_TO?: string;
}

/** The fetch the object makes, with the local key-set stand-in above
 * applied when the run names one; undefined (the plain fetch) when
 * it does not. Only a request for that one address is rewritten. */
function keysFetch(env: Env): typeof fetch | undefined {
  if (!env.KEYS_FROM || !env.KEYS_TO) return undefined;
  const from = env.KEYS_FROM;
  const to = env.KEYS_TO;
  return ((url: string | URL | Request, init?: RequestInit) =>
    fetch(typeof url === "string" && url === from ? to : (url as string), init)) as typeof fetch;
}

/** The run's clock interval, when it names a usable one (a second or
 * more); anything else leaves the object's own minute in place. */
function ownClockMs(raw: string | undefined): number | undefined {
  const n = Number(raw);
  return raw !== undefined && Number.isFinite(n) && n >= 1_000 ? Math.floor(n) : undefined;
}

/** The object with the state. */
export class PmEdge extends DurableObject<Env> {
  private edge: Edge;

  constructor(ctx: DurableObjectState, env: Env) {
    super(ctx, env);
    // The app says "ping" to keep its connection from being dropped as
    // idle; the platform answers "pong" itself, without waking this.
    ctx.setWebSocketAutoResponse(new WebSocketRequestResponsePair("ping", "pong"));
    const bridge = env.BRIDGE;
    this.edge = new Edge({
      fetchImpl: keysFetch(env),
      kv: {
        get: (key) => ctx.storage.get(key),
        put: (key, value) => ctx.storage.put(key, value),
        delete: async (key) => {
          await ctx.storage.delete(key);
        },
      },
      direct: {
        enabled: env.EDGE_DIRECT === "on",
        githubToken: env.GITHUB_TOKEN,
        queueToken: env.QUEUE_TOKEN,
        bridgeToken: env.BRIDGE_TOKEN,
        decisionEvents:
          env.DECISION_EVENTS_URL && env.DECISION_EVENTS_SECRET
            ? { url: env.DECISION_EVENTS_URL, secret: <redacted> }
            : undefined,
        // The binding's fetch, shaped as the fetch the classes take:
        // not to be called as a method, like the platform's own.
        bridgeFetch: bridge
          ? (url: string | URL | Request, init?: RequestInit) => bridge.fetch(new Request(url as string, init))
          : undefined,
        githubApi: env.GITHUB_API_ORIGIN,
        queueDb: env.QUEUE_DB_ORIGIN,
        fetchImpl: fetch,
      },
      ledgerPages:
        env.LEDGER_PAGES === "on"
          ? env.LEDGER_STORE === "d1"
            ? { store: "d1", db: env.LEDGER_DB }
            : { store: "turso", url: env.PM_LEDGER_DB_URL ?? "", token: env.PM_LEDGER_DB_TOKEN }
          : undefined,
      dnsPages: env.DNS_PAGES === "on" ? { db: env.LEDGER_DB } : undefined,
      // Live connections to the app. The platform holds them, so they
      // stay open while this object is put away between events, and it
      // is woken (by its alarm) only when there is something to read.
      sockets: {
        accept: () => {
          const pair = new WebSocketPair();
          ctx.acceptWebSocket(pair[1]);
          return new Response(null, { status: 101, webSocket: pair[0] });
        },
        send: (text) => {
          let told = 0;
          for (const ws of ctx.getWebSockets()) {
            try {
              ws.send(text);
              told++;
            } catch {
              // Gone between being listed and being told: the platform drops it.
            }
          }
          return told;
        },
        count: () => ctx.getWebSockets().length,
      },
      setAlarm: (at) => ctx.storage.setAlarm(at),
      getAlarm: () => ctx.storage.getAlarm(),
      ownClockMs: ownClockMs(env.OWN_CLOCK_MS),
      // A read started behind an answer is given the time to finish.
      later: (work) => ctx.waitUntil(work),
    });
  }

  fetch(request: Request): Promise<Response> {
    return this.edge.fetch(request);
  }

  alarm(): Promise<void> {
    return this.edge.alarm();
  }

  /** The app says nothing over its connection but "ping" (answered by
   * the platform, above): what it does goes by its requests, which are
   * kept and sent again when there is no connection. Anything else
   * said is not for this. */
  webSocketMessage(): void {}

  /** The app went away, or the connection broke: finish closing it. */
  webSocketClose(ws: WebSocket, code: number): void {
    try {
      // 1005 and 1006 say how it ended and may not be sent back.
      ws.close(code === 1005 || code === 1006 ? 1000 : code);
    } catch {
      // Already closed.
    }
  }

  webSocketError(ws: WebSocket): void {
    try {
      ws.close(1011);
    } catch {
      // Already closed.
    }
  }
}

/** The host answered, as opposed to something in front of it saying it
 * could not be reached. The host marks every response it writes. */
function fromHost(res: Response): boolean {
  return res.headers.get("x-pm-console") === "1" || res.status < 500;
}

/** What answered in the host's place says that the host was never
 * reached. Cloudflare answers 530 (with one of its 1xxx errors in the
 * body: 1033 for a tunnel) when it has no way to the origin at all: for
 * a tunnel, no connection of the machine's to send the request down.
 * Nothing was handed to the machine, so nothing asked of it was done.
 * Any other answer in the host's place does not say as much: a gateway
 * that lost the connection part-way answers for a host that may have
 * had the request. */
function neverReached(res: Response): boolean {
  return res.status === 530 && res.headers.get("x-pm-console") !== "1";
}

/** Ask the host. `ms` is how long it has to start answering; a stream it
 * then keeps open (the dashboard's live pages) is left to run. Null when
 * it did not answer, and "unreached" when what answered in its place
 * says it was never reached (see neverReached). */
async function ask(request: Request, ms: number, origin?: string): Promise<Response | "unreached" | null> {
  const abort = new AbortController();
  const timer = setTimeout(() => abort.abort(), ms);
  try {
    // On the hostname's own route, fetching the request is asking the host.
    const url = new URL(request.url);
    const target = origin ? new Request(`${origin}${url.pathname}${url.search}`, request) : request;
    const res = await fetch(new Request(target, { redirect: "manual", signal: abort.signal }));
    if (fromHost(res)) return res;
    const never = neverReached(res);
    // Not the host's, and not read: let go of, so nothing is left open.
    unread(res);
    return never ? "unreached" : null;
  } catch {
    return null;
  } finally {
    clearTimeout(timer);
  }
}

async function host(request: Request, ms: number, origin?: string): Promise<Response | null> {
  const answer = await ask(request, ms, origin);
  return answer === "unreached" ? null : answer;
}

export default {
  async fetch(request: Request, env: Env, ctx: ExecutionContext): Promise<Response> {
    ctx.passThroughOnException();
    const stub = env.PM.get(env.PM.idFromName("pm"));
    return route(request, {
      state: (r) => stub.fetch(r),
      asset: (path) => env.ASSETS.fetch(new Request(`https://assets.internal${path}`)),
      host: (r, ms) => host(r, ms, env.HOST_ORIGIN),
      act: (r, ms) => ask(r, ms, env.HOST_ORIGIN),
      publicHost: env.PUBLIC_HOST,
      version: env.VERSION ?? null,
      later: (work) => ctx.waitUntil(work),
      now: Date.now,
    });
  },

  /** The scheduled Worker checks (#15793, steps 1–2 of #13494): ask
   * each bound Worker, by its service binding — no public hop, no
   * setting changed — for the commit it was deployed from, and hand
   * the set to the object to keep. The cron trigger is declared in
   * wrangler.jsonc and registered in doc/fleet-schedule-registry.md.
   * A Worker with no binding configured is left out of the round;
   * one that does not answer is recorded as not answering. */
  async scheduled(_controller: ScheduledController, env: Env, ctx: ExecutionContext): Promise<void> {
    const askedAt = Date.now();
    const bound: [string, Env["BRIDGE"]][] = [
      ["message-bridge", env.BRIDGE],
      ["pm-service", env.PM_SERVICE],
      ["decision-events", env.DECISION_EVENTS],
    ];
    const targets: WorkerCheckTarget[] = bound
      .filter((entry): entry is [string, NonNullable<Env["BRIDGE"]>] => entry[1] !== undefined)
      .map(([name, binding]) => ({
        name,
        fetcher: (request: Request) => binding.fetch(request),
      }));
    const set = await probeWorkers(targets, askedAt);
    set.checks.push(selfCheck("pm-console-edge", env.VERSION, askedAt));
    set.checks.sort((a, b) => a.name.localeCompare(b.name));
    const stub = env.PM.get(env.PM.idFromName("pm"));
    ctx.waitUntil(
      stub.fetch(
        new Request("https://pm.internal/internal/worker-checks", {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify(set),
        }),
      ),
    );
  },
};
