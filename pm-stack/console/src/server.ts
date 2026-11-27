// pm-console: the write side of the PM stack. The dashboard
// (pm-aggregator) shows the fleet; this service acts on it.
//
// One public hostname carries both, and all of it arrives here. /console
// is this service. Every other path is the dashboard's, passed through
// only for a verified owner: the dashboard has no sign-in of its own, so
// the check happens at this origin and does not rest on how the edge
// reads a path. Three files a phone fetches before anyone has signed in
// (the install manifest, its icons, the Android asset links) are public.
// Two more paths carry their own check instead of the sign-in: a linked
// phone's widget presents its own token there to read a short summary.
import { createHash } from "node:crypto";
import { mkdirSync, readFileSync } from "node:fs";
import { appendFile } from "node:fs/promises";
import { hostname } from "node:os";
import { dirname, join } from "node:path";
import { actions, perform } from "./actions.ts";
import { HOLDABLE, type Held, type MachinesAnswer, machineName, STALE_ACT_ERROR, staleAct } from "./edge-proto.ts";
import { AuthError, type Identity, identify } from "./auth.ts";
import { type Config, readConfig } from "./config.ts";
import { Devices, Limiter } from "./devices.ts";
import { DOC_NAMES, type DocName, Docs } from "./docs.ts";
import { fileStore } from "./file-store.ts";
import {
  assetLinks,
  BASE,
  crossSiteWrite,
  deviceLink,
  deviceSummary,
  docResponse,
  json,
  manifest,
  type Phones,
  SECURITY,
  sameEntity,
  sameSiteWrite,
} from "./http.ts";
import { Notifier } from "./notify.ts";
import { OpError, Ops } from "./ops.ts";
import { Plan } from "./plan.ts";
import { ownerSettingsAdopted } from "./settings.ts";
import { platformRead } from "./reads.ts";
import { contact, Push } from "./push.ts";
import { Named } from "./named.ts";
import { EdgeLink } from "./sync.ts";
import { memoryShelf, Trends } from "./trends.ts";
import { Work } from "./work.ts";
import appFiles from "../public/files.json";

const PUBLIC_DIR = join(import.meta.dir, "..", "public");
/** The app's files and what each is served as: public/files.json is the
 * one list of them (the Worker, its build and the service worker read
 * the same one). */
const STATIC = new Map<string, string>(appFiles.files.map((f) => [f.path, f.type]));

const APK_NAME = "pm-console.apk";
const ACT_ID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
/** An action's answer is kept with its name when it is no longer than
 * this, so that the same name offered again is answered the same. */
const RESULT_KEPT_CHARS = 2000;
/** The answer to keep, if it is small enough to. */
const keptSmall = (result: unknown): { result?: unknown } => {
  try {
    return result !== undefined && JSON.stringify(result).length <= RESULT_KEPT_CHARS ? { result } : {};
  } catch {
    return {};
  }
};
/** The dashboard's files that are public, matched against the parsed
 * path in full. They say nothing about the fleet. */
const PUBLIC_DASHBOARD = /^\/icons\/(icon-192|icon-512|icon-maskable-512|apple-touch-icon)\.png$/;
/** Headers that belong to one connection, not to the request or response. */
const HOP_BY_HOP = new Set([
  "connection",
  "keep-alive",
  "proxy-authenticate",
  "proxy-authorization",
  "proxy-connection",
  "te",
  "trailer",
  "transfer-encoding",
  "upgrade",
]);
const UPSTREAM_HEADER_TIMEOUT_MS = 60_000;
const UPSTREAM_BODY_LIMIT = 16 * 1024 * 1024;

/** Longest a request may run with nothing written: above the longest
 * operation (a 60 second systemctl call), or the connection would drop
 * while the action carried on. Bun's limit is 255. */
const IDLE_TIMEOUT_SECONDS = 120;

// The page's four files, read once: nothing on the request path touches
// the disk synchronously after the first hit.
const fileCache = new Map<string, { bytes: Uint8Array<ArrayBuffer>; etag: string }>();
function file(req: Request, name: string, type: string): Response {
  let kept = fileCache.get(name);
  if (!kept) {
    const bytes = new Uint8Array(readFileSync(join(PUBLIC_DIR, name)));
    kept = { bytes, etag: `"${createHash("sha256").update(bytes).digest("base64url").slice(0, 22)}"` };
    fileCache.set(name, kept);
  }
  // "no-cache" is "ask before using your copy": the tag is what the
  // browser asks with, and a copy that is still the file is not sent again.
  const headers = { "Content-Type": type, "Cache-Control": "no-cache", ETag: kept.etag, ...SECURITY };
  if (sameEntity(req.headers.get("if-none-match"), kept.etag)) return new Response(null, { status: 304, headers });
  return new Response(kept.bytes, { headers });
}

export { assetLinks, type Phones };

/** The history behind the Trends screen, for these dependencies: made
 * once and kept (in memory: a machine reads it again when it starts). */
const trendsKept = new WeakMap<Deps, Trends>();
function trendsOf(deps: Deps): Trends {
  let trends = trendsKept.get(deps);
  if (!trends) {
    trends = new Trends(deps.ops, memoryShelf());
    trendsKept.set(deps, trends);
  }
  return trends;
}

/** The plan's reader for these dependencies: the one given, or one made
 * from the same `ops` and `work` and kept. */
const plans = new WeakMap<Deps, Plan>();
function planOf(deps: Deps): Plan {
  if (deps.plan) return deps.plan;
  let plan = plans.get(deps);
  if (!plan) {
    plan = new Plan(deps.ops, deps.work ?? new Work(deps.ops));
    plans.set(deps, plan);
  }
  return plan;
}

export interface Deps {
  cfg: Config;
  ops: Ops;
  /** Roadmaps, bugs and the intake list. Made from `ops` and `work` when not given. */
  plan?: Plan;
  /** `peer` is the connection's remote address as the server saw it. */
  identify: (req: Request, peer: string | null) => Promise<Identity>;
  audit: (entry: Record<string, unknown>) => Promise<void>;
  /** Reaches the dashboard. Injected so tests can stand in for it. */
  upstream?: (url: string, init: RequestInit) => Promise<Response>;
  /** The interface's state, kept ready, and the actions it can take. */
  docs?: Docs;
  actions?: ReturnType<typeof actions>;
  work?: Work;
  push?: Push;
  /** Phones linked to read the summary, with the limits on the two
   * paths they use: those paths are reachable without sign-in, so they
   * exist only when their limits do. */
  phones?: Phones;
  /** This host's link to the edge, when the app is served from one. */
  link?: Pick<EdgeLink, "publicKey"> & Partial<Pick<EdgeLink, "begin" | "end" | "seen" | "known" | "take" | "release">>;
  /** What became of each action sent under a name, so that one is done
   * once however often it arrives. The link keeps one; a host with no
   * edge in front of it keeps its own. */
  named?: Pick<Named, "begin" | "end" | "seen"> & Partial<Pick<Named, "known" | "take" | "release">>;
  /** What this machine is called and the commit it runs, for where
   * machines are listed. */
  self?: { name: string; version?: string };
}

/** Text that shrinks well. The dashboard's fragments run to a megabyte
 * each, and the link to a phone is the slow part. */
const COMPRESSIBLE = /^(text\/|application\/(json|javascript|manifest\+json|xml)|image\/svg\+xml)/;
const COMPRESS_FROM_BYTES = 1024;

function wantsGzip(req: Request): boolean {
  return /\bgzip\b/.test(req.headers.get("accept-encoding") ?? "");
}

function gzipped(req: Request, bytes: Uint8Array, headers: Headers): Response | null {
  if (!wantsGzip(req) || bytes.byteLength < COMPRESS_FROM_BYTES) return null;
  headers.set("content-encoding", "gzip");
  headers.append("vary", "accept-encoding");
  return new Response(Bun.gzipSync(bytes as Uint8Array<ArrayBuffer>), { headers });
}

/** Pass one request to the dashboard and its answer back. `open` is for
 * the public files: nothing the caller sent goes with it. */
async function toDashboard(req: Request, url: URL, deps: Deps, open: boolean): Promise<Response> {
  const headers = new Headers();
  if (!open) {
    for (const [name, value] of req.headers) {
      if (!HOP_BY_HOP.has(name) && name !== "content-length" && name !== "accept-encoding") headers.set(name, value);
    }
  }
  // The dashboard builds links and checks origins against the public
  // name; and an uncompressed answer keeps its headers true as it passes.
  headers.set("host", req.headers.get("host") ?? url.host);
  headers.set("accept-encoding", "identity");
  let body: ArrayBuffer | undefined;
  if (!open && req.method !== "GET" && req.method !== "HEAD") {
    body = await req.arrayBuffer();
    if (body.byteLength > UPSTREAM_BODY_LIMIT) return json({ error: "request body too large" }, 413);
  }
  const abort = new AbortController();
  const stop = () => abort.abort();
  req.signal.addEventListener("abort", stop, { once: true });
  const timer = setTimeout(stop, UPSTREAM_HEADER_TIMEOUT_MS);
  let res: Response;
  try {
    // The parsed path is what was checked, so it is what is sent: the
    // dashboard never sees a spelling of the path the check did not.
    res = await (deps.upstream ?? fetch)(`${deps.cfg.dashboardUrl}${url.pathname}${url.search}`, {
      method: open ? "GET" : req.method,
      headers,
      body,
      redirect: "manual",
      signal: abort.signal,
    });
  } catch (error) {
    console.error(`pm-console: dashboard: ${(error as Error).message}`);
    return json({ error: "the dashboard did not answer" }, 502);
  } finally {
    clearTimeout(timer);
  }
  const out = new Headers();
  for (const [name, value] of res.headers) {
    if (!HOP_BY_HOP.has(name) && name !== "content-length" && name !== "content-encoding") out.append(name, value);
  }
  // A page or a fragment is finite: read it and send it compressed. An
  // event stream that stays open (anything but a fragment) is passed
  // through as it comes, since compressing would hold its events back.
  const type = res.headers.get("content-type") ?? "";
  const finite = type.startsWith("text/event-stream") ? url.pathname.startsWith("/frag/") : COMPRESSIBLE.test(type);
  if (req.method !== "HEAD" && res.status === 200 && finite && wantsGzip(req)) {
    const bytes = new Uint8Array(await res.arrayBuffer());
    return gzipped(req, bytes, out) ?? new Response(bytes, { status: res.status, headers: out });
  }
  return new Response(req.method === "HEAD" ? null : res.body, { status: res.status, headers: out });
}

export async function handle(req: Request, deps: Deps, peer: string | null = null): Promise<Response> {
  const url = new URL(req.url);
  const path = url.pathname;
  const method = req.method;

  // Public, unauthenticated: nothing here says anything about the fleet.
  if (path === "/.well-known/assetlinks.json" && method === "GET") return json(assetLinks(deps.cfg));
  // Which commit this console was started from, where it knows:
  // install.sh waits for the console to answer as the release it has
  // just put into service.
  if (path === `${BASE}/health` && method === "GET") return json({ ok: true, version: deps.self?.version ?? null });
  if (path === "/manifest.webmanifest" && method === "GET") return manifest();
  // Not behind the sign-in, and checked here instead: a linked phone.
  if (deps.phones && deps.docs) {
    if (path === `${BASE}/device/link` && method === "POST") return deviceLink(req, deps.phones, deps.audit);
    if (path === `${BASE}/device/summary` && method === "GET") return deviceSummary(req, deps.docs, deps.phones);
  }
  // The public half of this host's key: the edge asks for it through the
  // tunnel to check that a report really is from this host.
  if (deps.link && path === `${BASE}/edge/key` && method === "GET") return json({ key: await deps.link.publicKey() });
  const ours = path === BASE || path.startsWith(`${BASE}/`);
  if (!ours) {
    if (deps.cfg.dashboardUrl === null) return json({ error: "not found" }, 404);
    if ((method === "GET" || method === "HEAD") && PUBLIC_DASHBOARD.test(path))
      return toDashboard(req, url, deps, true);
  }

  let who: Identity;
  try {
    who = await deps.identify(req, peer);
  } catch (error) {
    if (error instanceof AuthError) return json({ error: error.message }, error.status);
    throw error;
  }
  if (!ours) {
    if (crossSiteWrite(req)) return json({ error: "this request did not come from this site" }, 403);
    // The front door is the app. The dashboard's own home stays one
    // link away, under More, at /?dashboard.
    if (method === "GET" && path === "/" && url.search === "") return Response.redirect(`${BASE}/`, 302);
    return toDashboard(req, url, deps, false);
  }

  const rest = path.slice(BASE.length).replace(/^\/+/, "");
  if (method === "GET" && rest === "") {
    if (path === BASE) return Response.redirect(`${BASE}/`, 308);
    return file(req, "index.html", "text/html; charset=utf-8");
  }
  const type = method === "GET" ? STATIC.get(rest) : undefined;
  if (type !== undefined) return file(req, rest, type);
  // The Android app, for the signed-in owner: the phone gets it from the
  // site it opens, and nothing has to be published anywhere else.
  if (method === "GET" && rest === APK_NAME) {
    const apk = Bun.file(deps.cfg.androidApk);
    if (!(await apk.exists())) return json({ error: "no Android build is installed on this host" }, 404);
    return new Response(apk, {
      headers: {
        "Content-Type": "application/vnd.android.package-archive",
        "Content-Disposition": `attachment; filename="${APK_NAME}"`,
        "Cache-Control": "no-store",
        ...SECURITY,
      },
    });
  }
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
            return json({
              ...who,
              repo: deps.cfg.repo,
              queue: deps.cfg.queue,
              androidApp: await Bun.file(deps.cfg.androidApk).exists(),
              androidPackage: deps.cfg.androidPackage,
            });
          case "overview":
            return json(await ops.overview());
          case "agents":
            return json(await ops.agents());
          case "messages":
            return json(await ops.messages(query.get("limit")));
          case "services":
            return json(await ops.services());
          // When the store tokens run out (#14255). A machine's to
          // read: only a machine can fetch every token by name, so the
          // edge passes this on to one.
          case "tokens":
            return json(await ops.tokenExpiries());
        }
      }
      // What needs only GitHub and the queue store: the same code
      // answers these at the edge without this machine.
      const read = await platformRead(seg, query, ops, planOf(deps), trendsOf(deps));
      if (read !== undefined) return json(read);
      if (seg.length === 2 && seg[0] === "doc" && deps.docs && DOC_NAMES.includes(seg[1] as DocName)) {
        return docResponse(req, await deps.docs.doc(seg[1] as DocName), gzipped);
      }
      if (seg.length === 1 && seg[0] === "push" && deps.push) return json(await deps.push.status());
      if (seg.length === 1 && seg[0] === "devices" && deps.phones) return json(await deps.phones.devices.list());
      if (seg.length === 1 && seg[0] === "machines") {
        // Asked of this machine itself (the edge answers for itself when
        // it is in front): one machine, this one, and nothing held.
        const answer: MachinesAnswer = {
          served: "machine",
          platform: null,
          machines: [
            {
              name: deps.self?.name ?? "this machine",
              seen: Date.now(),
              quiet: false,
              version: deps.self?.version ?? null,
              actions: Object.keys(deps.actions ?? {}).sort(),
              sections: (deps.docs?.names() ?? []).sort(),
            },
          ],
          held: [],
        };
        return json(answer);
      }
      if (seg.length === 1 && seg[0] === "signin") return json(await ops.signin());
      if (seg.length === 1 && seg[0] === "question" && deps.work) {
        return json(await deps.work.question(query.get("set"), query.get("ledger"), query.get("qid")));
      }
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
      /** More to say in the answer when the action worked. */
      extra?: () => Record<string, unknown>,
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
        return json({ ok: true, result, ...extra?.() });
      } catch (error) {
        await done({ ok: false, error: (error as Error).message });
        throw error;
      }
    };

    if (seg.length === 1 && seg[0] === "act" && deps.docs && deps.actions) {
      const { docs, actions: table } = deps;
      const kind = typeof body.kind === "string" ? body.kind : "";
      const args = body.args ?? {};
      const kept =
        Object.hasOwn(table, kind) && args !== null && typeof args === "object"
          ? table[kind].audit(args as Record<string, unknown>)
          : {};
      // Sent under a name, by the edge that relayed it or by the device
      // itself: the name is written down before the action starts and
      // its outcome after. An action is done once under a name, however
      // often it arrives: if the answer was lost and it is offered
      // again, it is answered from what was written down.
      const name = req.headers.get("x-pm-act") ?? (typeof body.id === "string" ? body.id : "");
      const entry = {
        // On the record of what was done under the name it was sent
        // with, so the device that sent it can find how it came out.
        id: ACT_ID.test(name) ? name : crypto.randomUUID(),
        kind,
        at: new Date().toISOString(),
        by: who.email ?? who.certName ?? who.sub ?? who.via,
        note: "",
      };
      // Relayed by the edge: the answer also carries the sections this
      // action read again and the record of what was done, so the edge's
      // copy shows the change without waiting for the next report.
      const forEdge =
        req.headers.get("x-pm-edge") === "1" && Object.hasOwn(table, kind)
          ? () => ({
              sections: docs.snapshot().filter((s) => table[kind].touch.includes(s.name)),
              acts: docs.recentActs(),
            })
          : undefined;
      const record = deps.named ?? deps.link;
      const link = ACT_ID.test(name) && record?.begin && record.end ? record : null;
      // The name is taken for doing here and now, before anything is
      // waited for (what is known of it is read, and the audit written,
      // after): of two arrivals under one name only one can take it,
      // and the other is told it is being done.
      const taken = link?.take && link.known ? link.take(name) : null;
      if (taken === false) return json({ error: "This is still being done.", running: true }, 409);
      try {
        // What was written down for it before. (A record that cannot be
        // taken is asked the older way, which also says "being done".)
        const before = (await (taken ? link?.known?.(name) : link?.seen?.(name))) ?? null;
        if (before?.state === "running") return json({ error: "This is still being done.", running: true }, 409);
        if (before?.state === "lost") {
          // Started, and this host stopped before it ended. Whether it
          // happened is not known, and doing it again could do it twice.
          const fate = {
            ok: false,
            status: 502,
            error: "the host restarted while doing this; check whether it happened",
          };
          await link?.end?.(name, fate);
          return json({ error: fate.error, again: true }, 502);
        }
        if (before?.state === "done") {
          return before.fate.ok
            ? json({ ok: true, result: before.fate.result ?? null, again: true, ...forEdge?.() })
            : json({ error: before.fate.error ?? "it could not be done", again: true }, before.fate.status ?? 502);
        }
        // Something that may have changed is not done unasked once it
        // is more than ten minutes old (#13501), sent straight here or
        // relayed by the edge (which passes the device's body on as it
        // came, and holds the device to the same rule first).
        if (before === null && staleAct(kind, body, Date.now())) {
          return json({ error: STALE_ACT_ERROR, stale: true }, 409);
        }
        return await act(
          kind || "unknown",
          kept,
          async () => {
            await link?.begin?.(name);
            docs.logAct({ ...entry, status: "running" });
            try {
              const result = await perform(table, docs, kind, args);
              docs.logAct({ ...entry, status: "done" });
              await link?.end?.(name, { ok: true, ...keptSmall(result) });
              return result;
            } catch (error) {
              docs.logAct({ ...entry, status: "failed", note: (error as Error).message.slice(0, 200) });
              await link?.end?.(name, {
                ok: false,
                error: (error as Error).message.slice(0, 300),
                status: error instanceof OpError ? error.status : 502,
              });
              throw error;
            }
          },
          forEdge,
        );
      } finally {
        // Given back, whatever became of it: answered from the record,
        // refused before it began, or done (when it is given back already).
        if (taken) link?.release?.(name);
      }
    }
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

/** How often each source is read again while someone is looking. */
export function loaders(
  ops: Ops,
  work: Work,
  more: unknown,
): Record<string, { ttlMs: number; load: () => Promise<unknown> }> {
  return {
    board: { ttlMs: 10_000, load: () => work.board() },
    sessions: { ttlMs: 20_000, load: () => work.sessions() },
    census: { ttlMs: 60_000, load: () => work.census() },
    asks: { ttlMs: 20_000, load: () => work.asks() },
    questionCount: {
      ttlMs: 15_000,
      load: async () => {
        const q = await work.questions();
        return { total: q.total, sets: q.sets };
      },
    },
    questions: { ttlMs: 15_000, load: () => work.questions() },
    // Decision-set discovery belongs to the edge's Worker reader.
    questionSets: { ttlMs: 45_000, load: async () => null },
    prs: { ttlMs: 45_000, load: () => ops.prs() },
    priorities: { ttlMs: 120_000, load: () => ops.priorities() },
    done: { ttlMs: 60_000, load: () => work.done() },
    agents: { ttlMs: 30_000, load: () => ops.agents() },
    // The pipeline counted down to its unread instructions (#14377):
    // all the phone's summary takes from the feed.
    pipeline: { ttlMs: 30_000, load: () => ops.unreadInstructions() },
    services: { ttlMs: 20_000, load: () => ops.services() },
    more: { ttlMs: 3_600_000, load: async () => more },
  };
}

/** The dashboard's own pages, grouped as it groups them, for the list
 * under More. Read from the dashboard's manifest beside this package;
 * if that is not there, the list is simply empty. */
export async function morePages(): Promise<
  { group: string; pages: { title: string; route: string; blurb: string }[] }[]
> {
  try {
    const manifest = (await import("../../aggregator/src/pages.ts")) as {
      PAGE_GROUPS: { id: string; title: string }[];
      PAGES: { title: string; route: string; blurb: string; group: string }[];
    };
    return manifest.PAGE_GROUPS.map((g) => ({
      group: g.title,
      pages: manifest.PAGES.filter((pg) => pg.group === g.id).map((pg) => ({
        title: pg.title,
        // The dashboard's home is at /, which is now the app's front door.
        route: pg.route === "/" ? "/?dashboard" : pg.route,
        blurb: pg.blurb,
      })),
    })).filter((g) => g.pages.length > 0);
  } catch {
    return [];
  }
}

/** Runs an action the edge held while this host could not be reached,
 * recorded like any other. Only the actions that may be done late. */
export function heldRunner(deps: Pick<Deps, "docs" | "actions" | "audit">): (held: Held) => Promise<void> {
  return async (held) => {
    const { docs, actions: table } = deps;
    if (!docs || !table) throw new Error("this host takes no actions");
    if (!HOLDABLE.has(held.kind) || !Object.hasOwn(table, held.kind)) {
      throw new Error("this is not an action that may be done late");
    }
    const base = {
      at: new Date().toISOString(),
      who: held.who,
      via: "edge-held",
      action: held.kind,
      args: table[held.kind].audit(held.args),
      held: { id: held.id, at: held.at },
    };
    // As with any action: one that cannot be recorded is not taken.
    await deps.audit({ ...base, phase: "start" });
    const entry = { id: held.id, kind: held.kind, at: held.at, by: held.who };
    const done = (more: Record<string, unknown>) =>
      deps
        .audit({ ...base, phase: "done", ...more })
        .catch((error) => console.error(`pm-console: audit: ${(error as Error).message}`));
    try {
      const result = (await perform(table, docs, held.kind, held.args)) as {
        applied?: number;
        refused?: { why?: string }[];
      } | null;
      // An answer can come back "not recorded" without anything being
      // thrown (the question was settled in the meantime). Sent at once,
      // the app says so; sent late, this has to.
      const refused = Array.isArray(result?.refused) ? result.refused : [];
      if (refused.length > 0) {
        const why = String(refused[0]?.why ?? "no reason given");
        throw new Error(
          (result?.applied ?? 0) > 0
            ? `${result?.applied} recorded, ${refused.length} not recorded: ${why}`
            : `not recorded: ${why}`,
        );
      }
      docs.logAct({ ...entry, status: "done", note: "Sent late: held while the laptop could not be reached." });
      await done({ ok: true });
    } catch (error) {
      const message = (error as Error).message;
      docs.logAct({ ...entry, status: "failed", note: `Sent late, and it failed: ${message.slice(0, 160)}` });
      await done({ ok: false, error: message });
      throw error;
    }
  };
}

/** Every response this service writes is marked, so the edge in front
 * can tell "the host answered, with an error" from "the host could not
 * be reached". */
export function marked(res: Response): Response {
  const headers = new Headers(res.headers);
  headers.set("X-PM-Console", "1");
  return new Response(res.body, { status: res.status, statusText: res.statusText, headers });
}

/** The signed Android build at `path`, described for the edge. Its
 * fingerprint is worked out again only when the file has changed. */
export function androidBuild(path: string) {
  let known: { mtime: number; size: number; sha256: string } | null = null;
  return async () => {
    const file = Bun.file(path);
    if (!(await file.exists())) return null;
    const { lastModified: mtime, size } = file;
    if (!known || known.mtime !== mtime || known.size !== size) {
      const sha256 = new Bun.CryptoHasher("sha256").update(await file.arrayBuffer()).digest("hex");
      known = { mtime, size, sha256 };
    }
    return {
      sha256: known.sha256,
      size,
      at: mtime,
      read: async () => new Uint8Array(await Bun.file(path).arrayBuffer()),
    };
  };
}

/** What a release laid out by redeploy.sh says of itself, in the file
 * beside its code: the commit of main it is, and its console's tree.
 * Null for anything that is not such a file. */
export function releaseSaid(text: string): { commit: string; tree: string | null } | null {
  const commit = /^commit=([0-9a-f]{40})$/m.exec(text)?.[1];
  if (!commit) return null;
  return { commit, tree: /^tree=([0-9a-f]{40})$/m.exec(text)?.[1] ?? null };
}

/** The commit this console was started from, or nothing if it cannot
 * be told. In service it runs from a release, which says; started from
 * a checkout (by hand, or in a test), the checkout is asked. The
 * service is restarted when a release is put into service, so this is
 * the code that is running. */
async function runningCommit(): Promise<string | undefined> {
  try {
    const said = releaseSaid(readFileSync(join(import.meta.dir, "..", ".release"), "utf8"));
    if (said) return said.commit.slice(0, 12);
  } catch {
    // Not a release: a checkout.
  }
  try {
    const git = Bun.spawn(["git", "-C", import.meta.dir, "rev-parse", "--short=12", "HEAD"], {
      stdout: "pipe",
      stderr: "ignore",
    });
    const said = (await new Response(git.stdout).text()).trim();
    return (await git.exited) === 0 && /^[0-9a-f]{12}$/.test(said) ? said : undefined;
  } catch {
    return undefined;
  }
}

if (import.meta.main) {
  // `--check` builds everything this service needs and stops before
  // listening: what a redeploy runs to learn that new code starts.
  const check = process.argv.includes("--check");
  const cfg = readConfig();
  const ops = new Ops(cfg);
  mkdirSync(dirname(cfg.auditFile), { recursive: true, mode: 0o700 });
  const work = new Work(ops);
  const docs = new Docs(loaders(ops, work, await morePages()));
  const state = dirname(cfg.auditFile);
  const push = new Push(fileStore(join(state, "push.json")), contact(cfg.repo));
  const notifier = new Notifier(
    docs,
    push,
    fileStore(join(state, "notified.json")),
    Date.now,
    async () => (await ops.tokenExpiries()).tokens,
  );
  const devices = new Devices(fileStore(cfg.devicesFile));
  const deps: Deps = {
    cfg,
    ops,
    docs,
    work,
    push,
    phones: { devices, link: new Limiter(30, 60_000), refused: new Limiter(60, 60_000) },
    actions: actions(ops, work, push, devices),
    identify: (req, peer) => identify(req, cfg, { peer }),
    audit: (entry) => appendFile(cfg.auditFile, `${JSON.stringify(entry)}\n`, { mode: 0o600 }),
  };
  // What became of each action sent under a name. One record, whether
  // the action came straight here, was relayed by the edge, or was held
  // by it and offered late: a name is done once.
  const named = new Named(fileStore(cfg.edgeDoneFile));
  deps.named = named;
  // Behind an edge: report to it, and run what it held for this host.
  const version = await runningCommit();
  // What this machine is called: what it was told to be, or its own name.
  const self = { name: cfg.machineName || machineName(hostname()) || "machine", version };
  deps.self = self;
  // What this machine runs with of the settings the owner may change,
  // before the owner's changes: its environment's, or main's.
  const ownSettings = { promptLabel: cfg.promptLabel, needsRepos: [...cfg.needsRepos] };
  const link = cfg.edgeUrl
    ? new EdgeLink({
        url: cfg.edgeUrl,
        version,
        machine: () => ({ name: self.name, actions: Object.keys(deps.actions ?? {}).sort() }),
        apk: androidBuild(cfg.androidApk),
        keys: fileStore(cfg.edgeKeyFile),
        done: fileStore(cfg.edgeDoneFile),
        named,
        docs,
        phones: devices,
        // The owner's changes to the settings the owner may change, as
        // the edge says them, run with in place of this machine's own;
        // its own again once they are put back (#14957).
        adopt: (said) => {
          const adopted = ownerSettingsAdopted(ownSettings, said);
          if (adopted === null) return null;
          cfg.promptLabel = adopted.values.promptLabel;
          cfg.needsRepos = adopted.values.needsRepos;
          return adopted.rev;
        },
        config: async () => ({
          owners: cfg.owners,
          certNames: cfg.certNames,
          issuers: cfg.issuers,
          repo: cfg.repo,
          queue: cfg.queue,
          androidPackage: cfg.androidPackage,
          androidFingerprints: cfg.androidFingerprints,
          androidApp: await Bun.file(cfg.androidApk).exists(),
          queueDbUrl: cfg.queueDbUrl,
          queueTokenName: cfg.queueTokenName,
          promptLabel: cfg.promptLabel,
          needsRepos: cfg.needsRepos,
        }),
        run: heldRunner(deps),
      })
    : null;
  if (link) deps.link = link;
  if (check) {
    const health = await handle(new Request(`http://127.0.0.1:${cfg.port}${BASE}/health`), deps, "127.0.0.1");
    if (health.status !== 200) throw new Error(`self-check: health answered ${health.status}`);
    console.log("pm-console: check passed");
    process.exit(0);
  }
  docs.start();
  notifier.start();
  link?.start();
  const server = Bun.serve({
    hostname: cfg.bind,
    port: cfg.port,
    idleTimeout: IDLE_TIMEOUT_SECONDS,
    fetch: async (req, srv) => marked(await handle(req, deps, srv.requestIP(req)?.address ?? null)),
  });
  console.log(
    `pm-console listening on http://${server.hostname}:${server.port}${BASE}/ — owners: ${cfg.owners.length}, issuers: ${cfg.issuers.map((i) => i.name).join(", ") || "none (edge requests are refused)"}, dashboard: ${cfg.dashboardUrl ?? "not fronted"}, edge: ${cfg.edgeUrl ?? "none"}`,
  );
}
