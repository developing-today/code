// What the Worker does with a request, with nothing of the platform in
// it: the object that has the state, the app's files and the host are
// handed in. worker.ts wires the real ones; the tests wire stand-ins.
//
// Every request goes to the state first, which checks who is calling.
// Its answer is either the response, or word of what to do next: serve a
// file of the app, fetch the host's key, or pass the request to the host
// behind the tunnel. If the host cannot be reached, an action that may
// be done late is handed back to the state to hold.
import { SECURITY, sameEntity } from "../../src/http.ts";
import { ACT_MS } from "./timings.ts";

export { ACT_MS } from "./timings.ts";

export interface Io {
  /** The object with the state (edge.ts). */
  state: (request: Request) => Promise<Response>;
  /** A file of the app, by path. */
  asset: (path: string) => Promise<Response>;
  /** The host behind the hostname's route. `ms` is how long it has to
   * start answering. Null when it could not be reached. */
  host: (request: Request, ms: number) => Promise<Response | null>;
  /** The host, asked to do something. As `host`, but what came back in
   * the host's place is looked at: "unreached" when it says the host
   * was never reached (the tunnel had no connection to send it down),
   * so that nothing asked of it was done. Null is no answer at all, or
   * one that does not say: the host may have got it, and done it. */
  act: (request: Request, ms: number) => Promise<Response | "unreached" | null>;
  /** The hostname this Worker is routed on. Only there is the host
   * behind it; anywhere else only the public paths answer. */
  publicHost: string;
  /** What is deployed, for the health check. */
  version: string | null;
  /** Work that may finish after the response has gone. */
  later: (work: Promise<unknown>) => void;
  now: () => number;
}

/** How long the host is given: a read, an action (the longest is a 60
 * second unit operation), a page. */
export const READ_MS = 25_000;
export const PAGE_MS = 60_000;
const POKE_EVERY_MS = 20_000;
/** When the host was last told that someone is looking. */
let poked = 0;

/** For tests: forget when the host was last told. */
export function forgetPoke(): void {
  poked = 0;
}

function unreachablePage(): Response {
  const page = `<!doctype html><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>PM</title>
<body style="font:16px/1.5 system-ui,sans-serif;background:#0b0f14;color:#e6eaf0;max-width:34rem;margin:15vh auto;padding:0 1.25rem">
<h1 style="font-size:1.25rem">The laptop cannot be reached right now</h1>
<p style="color:#8b96a5">This page comes from it. The app itself does not: it still opens, shows the state as of the laptop's last report, and holds what you do until it is back.</p>
<p><a style="color:#6cb6ff" href="/console/">Open the app</a></p>`;
  return new Response(page, {
    status: 503,
    headers: {
      "Content-Type": "text/html; charset=utf-8",
      "Cache-Control": "no-store",
      "Retry-After": "30",
      // The one inline style on this page is its own.
      ...SECURITY,
      "Content-Security-Policy": "default-src 'none'; style-src 'unsafe-inline'; frame-ancestors 'none'",
    },
  });
}

const offline = (message: string) =>
  new Response(JSON.stringify({ error: message }), {
    status: 503,
    headers: { "Content-Type": "application/json; charset=utf-8", "Cache-Control": "no-store", ...SECURITY },
  });

const withHeader = (request: Request, name: string, value: string): Request => {
  const headers = new Headers(request.headers);
  headers.set(name, value);
  return new Request(request, { headers });
};

export async function route(request: Request, io: Io): Promise<Response> {
  const url = new URL(request.url);
  const routed = url.hostname === io.publicHost;
  const tell = (path: string, body: unknown) =>
    io.state(
      new Request(`https://pm.internal${path}`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(body),
      }),
    );

  // A path that is this Worker's own business with its state is never
  // taken from outside: it is the host's, like any path not known here.
  if (url.pathname.startsWith("/internal/")) {
    if (!routed) return new Response("not found", { status: 404, headers: SECURITY });
    return (await io.host(request, PAGE_MS)) ?? unreachablePage();
  }
  if (url.pathname === "/console/health" && request.method === "GET") {
    const body = (await (await io.state(request)).json()) as Record<string, unknown>;
    // The hostname it is built for is said too: the host's setup script
    // will not put any other hostname behind this Worker.
    return new Response(JSON.stringify({ ...body, version: io.version, host: io.publicHost }), {
      headers: { "Content-Type": "application/json; charset=utf-8", "Cache-Control": "no-store", ...SECURITY },
    });
  }

  const forHost = request.clone() as Request;
  let res = await io.state(request);
  let what = res.headers.get("x-pm-next");

  // The host's key is not known yet, or has changed: ask the host for it
  // through the tunnel, which only the host can answer, then let the
  // state look at the same report again.
  if (what === "key") {
    if (!routed) return offline("this address has no host behind it");
    const keyRes = await io.host(new Request(`${url.origin}/console/edge/key`), READ_MS);
    const key = keyRes?.ok ? ((await keyRes.json().catch(() => null)) as { key?: unknown } | null)?.key : null;
    if (!key) return offline("the host did not give its key");
    await tell("/internal/key", { key });
    res = await io.state(withHeader(forHost, "x-pm-key-fetched", "1"));
    what = res.headers.get("x-pm-next");
  }
  if (what === null) {
    // Someone is reading the app's state, which is served from this
    // side's copy. Tell the host in the background, by asking it the
    // same thing: that is what makes it read everything again and report.
    if (routed && request.method === "GET" && url.pathname.startsWith("/console/api/doc/")) {
      const now = io.now();
      if (now - poked > POKE_EVERY_MS) {
        poked = now;
        io.later(io.host(forHost, READ_MS).then((r) => r?.body?.cancel()));
      }
    }
    return res;
  }
  const detail = (await res.json()) as Record<string, unknown>;

  if (what === "asset") {
    const file = await io.asset(String(detail.path));
    if (!file.ok) return new Response("not found", { status: 404, headers: SECURITY });
    // "no-cache" is "ask before using your copy". The file's tag is what
    // the browser asks with; when its copy is still the file, the answer
    // is that and nothing else, so a visit does not download the app again.
    const headers: Record<string, string> = {
      "Content-Type": String(detail.type),
      "Cache-Control": "no-cache",
      ...SECURITY,
    };
    const etag = file.headers.get("etag");
    if (etag) headers.ETag = etag;
    if (sameEntity(request.headers.get("if-none-match"), etag)) {
      await file.body?.cancel();
      return new Response(null, { status: 304, headers });
    }
    return new Response(request.method === "HEAD" ? null : file.body, { headers });
  }

  // Everything below is for the host. Away from the routed hostname
  // there is no host.
  if (!routed) return offline("this address has no host behind it");

  if (detail.mode === "act") {
    // The host is told the action's name with the action. No answer may
    // mean the host never got it, or did it and the answer was lost: it
    // is held under that name, and the host sorts out which. Only when
    // what answered in its place says the host was never reached is the
    // state told so: then, and only then, it may do the action itself.
    const named = withHeader(withHeader(forHost, "x-pm-edge", "1"), "x-pm-act", String(detail.id ?? ""));
    const answer = await io.act(named, ACT_MS);
    if (answer === null || answer === "unreached") {
      return tell("/internal/hold", { ...detail, unreached: answer === "unreached" });
    }
    // The host's answer says what it read again; keep that here, and
    // send the owner the rest.
    if (!(answer.headers.get("content-type") ?? "").startsWith("application/json")) {
      await tell("/internal/relayed", {
        id: detail.id,
        kind: detail.kind,
        args: detail.args,
        relayToken: detail.relayToken,
      });
      return answer;
    }
    const body = (await answer.json().catch(() => null)) as Record<string, unknown> | null;
    if (body === null) {
      // The host answered, so it has the name written down, whatever
      // it said: a hold left under the name is settled as by any other
      // answer of the host's, and is nobody else's to do.
      await tell("/internal/relayed", {
        id: detail.id,
        kind: detail.kind,
        args: detail.args,
        relayToken: detail.relayToken,
      });
      return offline("the host's answer could not be read");
    }
    const { sections, acts, ...rest } = body;
    // The relayed name is settled even when the host read nothing new:
    // a hold left behind by a lost answer for the same name is dropped.
    await tell("/internal/relayed", {
      sections,
      acts,
      id: detail.id,
      kind: detail.kind,
      args: detail.args,
      relayToken: detail.relayToken,
    });
    const headers = new Headers(answer.headers);
    headers.delete("content-length");
    headers.delete("content-encoding");
    return new Response(JSON.stringify(rest), { status: answer.status, headers });
  }
  if (detail.mode === "live") {
    return (
      (await io.host(forHost, READ_MS)) ??
      offline("The laptop cannot be reached, and this is read from it as it is now.")
    );
  }
  return (await io.host(forHost, PAGE_MS)) ?? unreachablePage();
}
