// pm-console: the write side of the PM stack. The dashboard
// (pm-aggregator) shows the fleet; this service acts on it.
//
// Everything lives under /console so one public hostname can carry both:
// the tunnel sends /console and the Android asset-links file here and
// every other path to the dashboard.
import { mkdirSync, readFileSync } from "node:fs";
import { appendFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { AuthError, type Identity, identify } from "./auth.ts";
import { type Config, readConfig } from "./config.ts";
import { OpError, Ops } from "./ops.ts";

const BASE = "/console";
const PUBLIC_DIR = join(import.meta.dir, "..", "public");
const STATIC = new Map([
  ["app.js", "text/javascript; charset=utf-8"],
  ["app.css", "text/css; charset=utf-8"],
  ["icon.svg", "image/svg+xml"],
]);
const SECURITY = {
  "Content-Security-Policy":
    "default-src 'self'; img-src 'self' data:; frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
  "X-Content-Type-Options": "nosniff",
  "Referrer-Policy": "same-origin",
};
/** Longest a request may run with nothing written: above the longest
 * operation (a 60 second systemctl call), or the connection would drop
 * while the action carried on. Bun's limit is 255. */
const IDLE_TIMEOUT_SECONDS = 120;

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json; charset=utf-8", "Cache-Control": "no-store", ...SECURITY },
  });
}

// The page's four files, read once: nothing on the request path touches
// the disk synchronously after the first hit.
const fileCache = new Map<string, Uint8Array>();
function file(name: string, type: string): Response {
  let bytes = fileCache.get(name);
  if (!bytes) {
    bytes = readFileSync(join(PUBLIC_DIR, name));
    fileCache.set(name, bytes);
  }
  return new Response(bytes, { headers: { "Content-Type": type, "Cache-Control": "no-cache", ...SECURITY } });
}

/** The Digital Asset Links statement that lets the Android app open
 * this origin full screen. Public by design: Android fetches it. */
export function assetLinks(cfg: Pick<Config, "androidPackage" | "androidFingerprints">): unknown[] {
  if (cfg.androidFingerprints.length === 0) return [];
  return [
    {
      relation: ["delegate_permission/common.handle_all_urls"],
      target: {
        namespace: "android_app",
        package_name: cfg.androidPackage,
        sha256_cert_fingerprints: cfg.androidFingerprints,
      },
    },
  ];
}

export interface Deps {
  cfg: Config;
  ops: Ops;
  /** `peer` is the connection's remote address as the server saw it. */
  identify: (req: Request, peer: string | null) => Promise<Identity>;
  audit: (entry: Record<string, unknown>) => Promise<void>;
}

/** A state-changing request must be one this page made: a custom header
 * a cross-site form cannot set, and an Origin that is this host. */
function sameSiteWrite(req: Request): boolean {
  if (req.headers.get("x-pm-console") !== "1") return false;
  const origin = req.headers.get("origin");
  if (origin === null) return true;
  try {
    return new URL(origin).host === (req.headers.get("host") ?? "");
  } catch {
    return false;
  }
}

export async function handle(req: Request, deps: Deps, peer: string | null = null): Promise<Response> {
  const url = new URL(req.url);
  const path = url.pathname;
  const method = req.method;

  // Public, unauthenticated: nothing here says anything about the fleet.
  if (path === "/.well-known/assetlinks.json" && method === "GET") return json(assetLinks(deps.cfg));
  if (path === `${BASE}/health` && method === "GET") return json({ ok: true });
  if (path !== BASE && !path.startsWith(`${BASE}/`)) return json({ error: "not found" }, 404);

  let who: Identity;
  try {
    who = await deps.identify(req, peer);
  } catch (error) {
    if (error instanceof AuthError) return json({ error: error.message }, error.status);
    throw error;
  }

  const rest = path.slice(BASE.length).replace(/^\/+/, "");
  if (method === "GET" && rest === "") {
    if (path === BASE) return Response.redirect(`${BASE}/`, 308);
    return file("index.html", "text/html; charset=utf-8");
  }
  const type = method === "GET" ? STATIC.get(rest) : undefined;
  if (type !== undefined) return file(rest, type);
  if (!rest.startsWith("api/")) return json({ error: "not found" }, 404);

  let seg: string[];
  try {
    seg = rest.slice(4).split("/").map(decodeURIComponent);
  } catch {
    return json({ error: "bad path" }, 400);
  }
  const ops = deps.ops;
  const query = url.searchParams;
  try {
    if (method === "GET") {
      if (seg.length === 1) {
        switch (seg[0]) {
          case "me":
            return json({ ...who, repo: deps.cfg.repo, queue: deps.cfg.queue });
          case "overview":
            return json(await ops.overview());
          case "queue":
            return json({
              counts: await ops.queueCounts(),
              tasks: await ops.queueList(query.get("status") ?? "queued", query.get("q"), query.get("limit")),
            });
          case "priorities":
            return json(await ops.priorities());
          case "agents":
            return json(await ops.agents());
          case "messages":
            return json(await ops.messages(query.get("limit")));
          case "services":
            return json(await ops.services());
          case "prs":
            return json(await ops.prs());
        }
      }
      if (seg.length === 2 && seg[0] === "prs") return json(await ops.pr(seg[1]));
      if (seg.length === 3 && seg[0] === "services" && seg[2] === "logs") {
        return json(await ops.serviceLogs(seg[1], query.get("lines")));
      }
      return json({ error: "not found" }, 404);
    }

    if (method !== "POST") return json({ error: "method not allowed" }, 405);
    if (!sameSiteWrite(req)) return json({ error: "this request did not come from the console" }, 403);
    let body: Record<string, unknown> = {};
    try {
      const parsed = await req.json();
      if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) throw new Error("not an object");
      body = parsed as Record<string, unknown>;
    } catch {
      return json({ error: "body: expected a JSON object" }, 400);
    }

    // Two audit records per action. The first is written before anything
    // runs and must succeed: an action that cannot be recorded is not
    // taken, and one that kills this process (restarting the console) is
    // still on record. The second records the outcome; if it cannot be
    // written the action's own result still goes back to the caller, so a
    // finished merge is never reported as failed and tried again.
    const act = async (
      name: string,
      args: Record<string, unknown>,
      work: () => Promise<unknown>,
    ): Promise<Response> => {
      const base = {
        at: new Date().toISOString(),
        who: who.email ?? who.certName ?? who.sub ?? who.via,
        via: who.via,
        action: name,
        args,
      };
      try {
        await deps.audit({ ...base, phase: "start" });
      } catch (error) {
        console.error(`pm-console: audit: ${(error as Error).message}`);
        return json({ error: "the action could not be recorded, so it was not taken" }, 502);
      }
      const done = (entry: Record<string, unknown>) =>
        deps
          .audit({ ...base, phase: "done", ...entry })
          .catch((error) => console.error(`pm-console: audit: ${(error as Error).message}`));
      try {
        const result = await work();
        await done({ ok: true });
        return json({ ok: true, result });
      } catch (error) {
        await done({ ok: false, error: (error as Error).message });
        throw error;
      }
    };

    if (seg.length === 3 && seg[0] === "queue") {
      return await act(`queue.${seg[2]}`, { task: seg[1], reason: body.reason }, () =>
        ops.queueAct(seg[2], seg[1], body.reason),
      );
    }
    if (seg.length === 3 && seg[0] === "priorities" && seg[2] === "rank") {
      return await act("priority.rank", { issue: seg[1], rank: body.rank }, () => ops.setRank(seg[1], body.rank));
    }
    if (seg.length === 3 && seg[0] === "issues" && seg[2] === "comment") {
      return await act("issue.comment", { issue: seg[1] }, () => ops.comment(seg[1], body.body));
    }
    if (seg.length === 1 && seg[0] === "ideas") {
      return await act("idea.file", { title: body.title, rank: body.rank }, () =>
        ops.fileIdea(body.title, body.body, body.rank ?? "none"),
      );
    }
    if (seg.length === 1 && seg[0] === "messages") {
      return await act("message.send", { to: body.to, command: body.command === true }, () =>
        ops.send(body.to, body.text, body.command),
      );
    }
    if (seg.length === 3 && seg[0] === "services") {
      return await act(`service.${seg[2]}`, { unit: seg[1] }, () => ops.serviceAct(seg[1], seg[2]));
    }
    if (seg.length === 3 && seg[0] === "prs" && seg[2] === "merge") {
      return await act("pr.merge", { pr: seg[1] }, () => ops.mergePr(seg[1]));
    }
    return json({ error: "not found" }, 404);
  } catch (error) {
    if (error instanceof OpError) return json({ error: error.message }, error.status);
    console.error(`pm-console: ${method} ${path}: ${(error as Error).stack ?? error}`);
    return json({ error: "the console could not complete that" }, 502);
  }
}

if (import.meta.main) {
  const cfg = readConfig();
  const ops = new Ops(cfg);
  mkdirSync(dirname(cfg.auditFile), { recursive: true, mode: 0o700 });
  const deps: Deps = {
    cfg,
    ops,
    identify: (req, peer) => identify(req, cfg, { peer }),
    audit: (entry) => appendFile(cfg.auditFile, `${JSON.stringify(entry)}\n`, { mode: 0o600 }),
  };
  const server = Bun.serve({
    hostname: cfg.bind,
    port: cfg.port,
    idleTimeout: IDLE_TIMEOUT_SECONDS,
    fetch: (req, srv) => handle(req, deps, srv.requestIP(req)?.address ?? null),
  });
  console.log(
    `pm-console listening on http://${server.hostname}:${server.port}${BASE}/ — owners: ${cfg.owners.length}, issuers: ${cfg.issuers.map((i) => i.name).join(", ") || "none (edge requests are refused)"}`,
  );
}
