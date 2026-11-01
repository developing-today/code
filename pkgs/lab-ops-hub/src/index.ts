import { Container, getContainer } from "@cloudflare/containers";
import { DurableObject } from "cloudflare:workers";
import {
  computeBudget,
  crawlFetcher,
  fetchFollowingValidatedRedirects,
  parseJsonObjectBody,
  readResponseTextCapped,
  crawlArchiveKey,
  uploadKey,
  validateCrawlUrl,
  isMachineNameConflict,
  validateMachineBody,
  validateNoteBody,
  validateTaskBody,
} from "./lib";

// ---------------------------------------------------------------------------
// Lab Ops Hub — one Worker exercising the whole Workers Paid surface at
// personal scale: Workers + D1 + R2 + KV + Queues + Durable Objects
// (SQLite storage, WebSockets) + Containers (on-demand crawler) +
// Workers AI (free-neuron summarisation) + Cron Triggers + Workers Logs.
// Everything here is sized to stay inside the $5 plan's included amounts.
// ---------------------------------------------------------------------------

interface Env {
  DB: D1Database;
  FILES: R2Bucket;
  CACHE: KVNamespace;
  JOBS: Queue<Job>;
  LIVE: DurableObjectNamespace<LiveRoom>;
  CRAWLER: DurableObjectNamespace<CrawlerContainerV2>;
  AI: Ai;
  ASSETS: Fetcher;
  // Worker secret (`wrangler secret put API_TOKEN`). Every /api/* route
  // except /api/health requires it as a Bearer token (#441). Optional in
  // the type only so a misconfigured deploy compiles — at runtime the
  // gate fails CLOSED (503) when the secret is unset.
  API_TOKEN?: string;
}

type Job = { type: "index-file"; r2Key: string } | { type: "crawl"; crawlId: number; url: string };

const json = (data: unknown, status = 200) =>
  new Response(JSON.stringify(data, null, 2), {
    status,
    headers: { "content-type": "application/json" },
  });

async function logUsage(env: Env, kind: string, detail = "") {
  // Fire-and-forget ledger write; the budget endpoint reads this back.
  try {
    await env.DB.prepare("INSERT INTO usage_events (kind, detail) VALUES (?1, ?2)").bind(kind, detail).run();
  } catch {
    /* ledger must never break the real request */
  }
}

// --- API auth (#441) ---------------------------------------------------------
// The Worker is public on workers.dev; without a gate anyone could write
// D1/R2 rows, spend AI neurons and container minutes, and use the crawler
// as an open fetch proxy. All /api/* routes except /api/health require
// `Authorization: Bearer <API_TOKEN>` (or `x-api-token`, or `?token=` for
// the WebSocket / file-download cases that cannot set headers).

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

async function isAuthorized(req: Request, env: Env, url: URL): Promise<boolean> {
  const expected = env.API_TOKEN;
  if (!expected) return false;
  const header = req.headers.get("authorization");
  const bearer = header?.toLowerCase().startsWith("bearer ") ? header.slice(7).trim() : null;
  const given = bearer ?? req.headers.get("x-api-token") ?? url.searchParams.get("token");
  return given !== null && given.length > 0 && timingSafeTokenEqual(given, expected);
}

async function sha256Hex(buf: ArrayBuffer): Promise<string> {
  const hash = await crypto.subtle.digest("SHA-256", buf);
  return [...new Uint8Array(hash)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

// --- Durable Object: live status room (WebSocket presence + broadcast) -----
export class LiveRoom extends DurableObject<Env> {
  constructor(ctx: DurableObjectState, env: Env) {
    super(ctx, env);
    // SQLite-backed DO storage: last known cluster status snapshot.
    this.ctx.storage.sql.exec(`CREATE TABLE IF NOT EXISTS status (k TEXT PRIMARY KEY, v TEXT)`);
  }
  async fetch(req: Request): Promise<Response> {
    if (req.headers.get("Upgrade") === "websocket") {
      const pair = new WebSocketPair();
      this.ctx.acceptWebSocket(pair[1]);
      const snap = this.ctx.storage.sql.exec("SELECT v FROM status WHERE k='board'").one() as { v?: string } | null;
      pair[1].send(
        JSON.stringify({ type: "snapshot", board: snap?.v ?? null, clients: this.ctx.getWebSockets().length }),
      );
      return new Response(null, { status: 101, webSocket: pair[0] });
    }
    if (req.method === "POST") {
      const board = await req.text();
      this.ctx.storage.sql.exec(
        "INSERT INTO status (k, v) VALUES ('board', ?1) ON CONFLICT(k) DO UPDATE SET v=?1",
        board,
      );
      for (const ws of this.ctx.getWebSockets()) {
        try {
          ws.send(JSON.stringify({ type: "board", board }));
        } catch {}
      }
      return json({ ok: true, clients: this.ctx.getWebSockets().length });
    }
    return json({ error: "websocket upgrade or POST required" }, 400);
  }
  async webSocketMessage(ws: WebSocket, message: string) {
    for (const other of this.ctx.getWebSockets()) {
      try {
        other.send(message);
      } catch {}
    }
  }
}

// --- Container: on-demand URL crawler (runs only when invoked) -------------
export class CrawlerContainer extends Container<Env> {
  defaultPort = 8080;
  sleepAfter = "2m"; // die quickly; container minutes are the scarce resource
  override onStart() {
    console.log("crawler container started");
  }
  override onStop() {
    console.log("crawler container stopped");
  }
  override onError(err: unknown) {
    console.log("crawler container error:", err);
  }
}

// The original CrawlerContainer's container application (created
// 2026-10-20, during Cloudflare's platform incident window) never starts
// again — every start returns "internal error". CrawlerContainerV2 exists
// so deploys create a FRESH container application; the legacy class stays
// exported because existing Durable Objects depend on it (API error 10064
// if it disappears). Diagnosed 2026-10-25 against Cloudflare's official
// hello-world image; see README "Containers: FIXED (2026-10-25)".
export class CrawlerContainerV2 extends Container<Env> {
  defaultPort = 8080;
  sleepAfter = "2m"; // die quickly; container minutes are the scarce resource
  override onStart() {
    console.log("crawler container started");
  }
  override onStop() {
    console.log("crawler container stopped");
  }
  override onError(err: unknown) {
    console.log("crawler container error:", err);
  }
}

async function crawlWithContainer(env: Env, url: string) {
  const container = getContainer(env.CRAWLER, "crawler-5");
  const res = await container.fetch(
    new Request("http://container/crawl", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ url }),
      // Never let a cold/missing container image hang the request:
      // fall back to the in-Worker crawler after 120s (cold starts of a
      // 225 MB Nix-built image can take most of a minute to provision).
      signal: AbortSignal.timeout(120_000),
    }),
  );
  if (!res.ok) throw new Error(`container returned ${res.status}: ${(await res.text()).slice(0, 300)}`);
  return res.json() as Promise<{
    title?: string;
    excerpt?: string;
    links?: number;
    status_code?: number;
    text?: string;
  }>;
}

async function crawlFallback(url: string) {
  // Same contract, executed in the Worker when the container is unavailable.
  // Redirects are followed manually with every target re-validated (#460):
  // fetch's default "follow" mode would silently chase a 3xx from a public
  // URL to a private/loopback/link-local target the initial check refused.
  // Every hop gets the container crawler's 20s timeout via crawlFetcher
  // (lib.ts, #500): without a signal, a validated public server that accepts
  // the connection and then stalls — or slow-drips a body under the byte
  // cap — hangs the crawl until platform limits kill it. The signal stays
  // attached to the response, so it also bounds the readResponseTextCapped
  // body read below. The factory is unit-tested (#524); it was an inline
  // arrow here, where no test could reach it.
  const res = await fetchFollowingValidatedRedirects(url, crawlFetcher());
  // Cap the body read at the container's 2,000,000 bytes (#481): res.text()
  // would buffer a huge page whole in the Worker's memory.
  const html = await readResponseTextCapped(res);
  const title = html.match(/<title[^>]*>([^<]*)<\/title>/i)?.[1]?.trim() ?? "";
  const links = (html.match(/<a\s/gi) ?? []).length;
  const text = html
    .replace(/<script[\s\S]*?<\/script>/gi, "")
    .replace(/<[^>]+>/g, " ")
    .replace(/\s+/g, " ")
    .trim();
  return { title, excerpt: text.slice(0, 500), links, status_code: res.status, text: text.slice(0, 20000) };
}

// --- Main Worker -------------------------------------------------------------
export default {
  async fetch(req: Request, env: Env, ctx: ExecutionContext): Promise<Response> {
    const url = new URL(req.url);
    const p = url.pathname;

    // Auth gate (#441): everything under /api/ except /api/health needs
    // the API token. Fail closed when the secret is not configured, so a
    // redeploy that forgets `wrangler secret put API_TOKEN` cannot
    // silently re-open the mutating/AI/crawl endpoints.
    if (p.startsWith("/api/") && p !== "/api/health") {
      if (!env.API_TOKEN) {
        return json({ error: "API auth is not configured: set the API_TOKEN Worker secret" }, 503);
      }
      if (!(await isAuthorized(req, env, url))) {
        return json({ error: "unauthorized: send Authorization: Bearer <API_TOKEN>" }, 401);
      }
    }

    ctx.waitUntil(logUsage(env, "worker_request", `${req.method} ${p}`));

    try {
      // Health: prove every binding is wired.
      if (p === "/api/health") {
        const counts = await env.DB.prepare(
          "SELECT (SELECT COUNT(*) FROM machines) AS machines, (SELECT COUNT(*) FROM notes) AS notes, (SELECT COUNT(*) FROM files) AS files",
        ).first();
        return json({ ok: true, service: "lab-ops-hub", counts, time: new Date().toISOString() });
      }

      // Budget: self-counted usage vs the Workers Paid included amounts.
      // Prices are the plan's published overage rates (see the reference doc).
      if (p === "/api/budget") {
        const rows = await env.DB.prepare(
          "SELECT kind, COUNT(*) AS n FROM usage_events WHERE created_at >= datetime('now','start of month') GROUP BY kind",
        ).all<{ kind: string; n: number }>();
        const by = Object.fromEntries(rows.results.map((r) => [r.kind, r.n]));
        // Overage is computed from the counted usage (#442), and the D1
        // write count covers every write path plus the ledger's own rows
        // (see computeBudget in src/lib.ts).
        const budget = computeBudget(by);
        return json({
          plan: "Workers Paid ($5/mo)",
          ...budget,
          note: "Self-counted by this Worker; Cloudflare dashboard analytics are authoritative.",
        });
      }

      // Machines (D1)
      if (p === "/api/machines" && req.method === "GET") {
        const cached = await env.CACHE.get("machines:all");
        if (cached) return new Response(cached, { headers: { "content-type": "application/json", "x-cache": "KV" } });
        const { results } = await env.DB.prepare("SELECT * FROM machines ORDER BY name").all();
        const body = JSON.stringify({ machines: results }, null, 2);
        ctx.waitUntil(env.CACHE.put("machines:all", body, { expirationTtl: 60 }));
        return new Response(body, { headers: { "content-type": "application/json", "x-cache": "MISS" } });
      }
      if (p === "/api/machines" && req.method === "POST") {
        const parsed = await parseJsonObjectBody(req);
        if (!parsed.ok) return json({ error: parsed.error }, 400);
        const b = parsed.body;
        const valid = validateMachineBody(b);
        if (!valid.ok) return json({ error: valid.error }, 400);
        let r;
        try {
          r = await env.DB.prepare(
            "INSERT INTO machines (name, role, cpu, ram_gb, storage, gpu, status, notes) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
          )
            .bind(
              valid.values.name,
              b.role ?? "compute",
              b.cpu ?? null,
              b.ram_gb ?? null,
              b.storage ?? null,
              b.gpu ?? null,
              b.status ?? "planned",
              b.notes ?? null,
            )
            .run();
        } catch (e) {
          // A duplicate name is a client conflict, not a server fault
          // (#2631); every other DB failure keeps the outer 500.
          if (isMachineNameConflict(e)) {
            return json({ error: "machine name already exists", name: valid.values.name }, 409);
          }
          throw e;
        }
        ctx.waitUntil(Promise.all([logUsage(env, "d1_write", "machine"), env.CACHE.delete("machines:all")]));
        return json({ ok: true, id: r.meta.last_row_id }, 201);
      }

      // Notes (D1) + Workers AI summarisation
      if (p === "/api/notes" && req.method === "GET") {
        const { results } = await env.DB.prepare(
          "SELECT id, title, summary, tags, created_at FROM notes ORDER BY id DESC LIMIT 100",
        ).all();
        return json({ notes: results });
      }
      if (p === "/api/notes" && req.method === "POST") {
        const parsed = await parseJsonObjectBody(req);
        if (!parsed.ok) return json({ error: parsed.error }, 400);
        const b = parsed.body;
        const valid = validateNoteBody(b);
        if (!valid.ok) return json({ error: valid.error }, 400);
        const r = await env.DB.prepare("INSERT INTO notes (title, body, tags) VALUES (?1,?2,?3)")
          .bind(valid.values.title, valid.values.text, b.tags ?? "")
          .run();
        ctx.waitUntil(logUsage(env, "d1_write", "note"));
        return json({ ok: true, id: r.meta.last_row_id }, 201);
      }
      const sumMatch = p.match(/^\/api\/notes\/(\d+)\/summarize$/);
      if (sumMatch && req.method === "POST") {
        const note = await env.DB.prepare("SELECT * FROM notes WHERE id=?1")
          .bind(sumMatch[1])
          .first<{ body: string }>();
        if (!note) return json({ error: "note not found" }, 404);
        ctx.waitUntil(logUsage(env, "ai_request", "summarize"));
        const ai = (await env.AI.run("@cf/meta/llama-3.2-3b-instruct", {
          messages: [
            { role: "system", content: "Summarise in one or two sentences. Plain text only." },
            { role: "user", content: note.body.slice(0, 4000) },
          ],
        })) as { response?: string };
        const summary = (ai.response ?? "").trim();
        await env.DB.prepare("UPDATE notes SET summary=?1 WHERE id=?2").bind(summary, sumMatch[1]).run();
        ctx.waitUntil(logUsage(env, "d1_write", "note-summary"));
        return json({ ok: true, summary });
      }

      // Tasks (D1)
      if (p === "/api/tasks" && req.method === "GET") {
        const { results } = await env.DB.prepare("SELECT * FROM tasks ORDER BY done, id").all();
        return json({ tasks: results });
      }
      if (p === "/api/tasks" && req.method === "POST") {
        const parsed = await parseJsonObjectBody(req);
        if (!parsed.ok) return json({ error: parsed.error }, 400);
        const b = parsed.body;
        const valid = validateTaskBody(b);
        if (!valid.ok) return json({ error: valid.error }, 400);
        const r = await env.DB.prepare("INSERT INTO tasks (title, machine_id, due) VALUES (?1,?2,?3)")
          .bind(valid.values.title, b.machine_id ?? null, b.due ?? null)
          .run();
        ctx.waitUntil(logUsage(env, "d1_write", "task"));
        return json({ ok: true, id: r.meta.last_row_id }, 201);
      }

      // Files (R2 + D1 metadata + Queue indexing)
      if (p === "/api/files" && req.method === "GET") {
        const { results } = await env.DB.prepare("SELECT * FROM files ORDER BY id DESC LIMIT 100").all();
        return json({ files: results });
      }
      if (p === "/api/files" && req.method === "POST") {
        const form = await req.formData();
        const file = form.get("file") as File | null;
        if (!file) return json({ error: "multipart field 'file' required" }, 400);
        const buf = await file.arrayBuffer();
        const key = uploadKey(file.name, Date.now(), crypto.randomUUID());
        await env.FILES.put(key, buf, { httpMetadata: { contentType: file.type || "application/octet-stream" } });
        const r = await env.DB.prepare("INSERT INTO files (r2_key, filename, content_type, size) VALUES (?1,?2,?3,?4)")
          .bind(key, file.name, file.type || null, buf.byteLength)
          .run();
        await env.JOBS.send({ type: "index-file", r2Key: key });
        ctx.waitUntil(
          Promise.all([
            logUsage(env, "d1_write", "file"),
            logUsage(env, "r2_put", key),
            logUsage(env, "queue_msg", "index-file"),
          ]),
        );
        return json({ ok: true, id: r.meta.last_row_id, r2_key: key, queued: "index-file" }, 201);
      }
      const fileMatch = p.match(/^\/api\/files\/(.+)$/);
      if (fileMatch && req.method === "GET") {
        const obj = await env.FILES.get(decodeURIComponent(fileMatch[1]));
        if (!obj) return json({ error: "not found in R2" }, 404);
        return new Response(obj.body, {
          headers: { "content-type": obj.httpMetadata?.contentType ?? "application/octet-stream" },
        });
      }

      // Live board (Durable Object, one global room)
      if (p === "/api/live") {
        const stub = env.LIVE.getByName("global-board");
        ctx.waitUntil(logUsage(env, "do_request", "live"));
        return stub.fetch(req);
      }

      // Crawl (Container first, Worker fallback; full text archived to R2)
      if (p === "/api/crawl" && req.method === "POST") {
        const parsed = await parseJsonObjectBody(req);
        if (!parsed.ok) return json({ error: parsed.error }, 400);
        const b = parsed.body as { url: string; via?: string };
        if (typeof b.url !== "string" || !b.url) return json({ error: "url required" }, 400);
        // SSRF guard (#441): both crawlers only fetch public http(s)
        // targets; the container re-validates independently.
        const target = validateCrawlUrl(b.url);
        if (!target.ok) return json({ error: target.error }, 400);
        b.url = target.url;
        let result;
        let source = "container";
        let containerError: string | undefined;
        try {
          if (b.via === "worker") throw new Error("caller requested worker path");
          result = await crawlWithContainer(env, b.url);
          ctx.waitUntil(logUsage(env, "container_run", b.url));
        } catch (e) {
          source = "worker-fallback";
          containerError = String(e).slice(0, 300);
          result = await crawlFallback(b.url);
        }
        const key = crawlArchiveKey(Date.now(), crypto.randomUUID());
        await env.FILES.put(key, JSON.stringify({ url: b.url, source, ...result }, null, 2), {
          httpMetadata: { contentType: "application/json" },
        });
        const r = await env.DB.prepare(
          "INSERT INTO crawls (url, source, title, excerpt, links, status_code, r2_key) VALUES (?1,?2,?3,?4,?5,?6,?7)",
        )
          .bind(
            b.url,
            source,
            result.title ?? null,
            result.excerpt ?? null,
            result.links ?? null,
            result.status_code ?? null,
            key,
          )
          .run();
        ctx.waitUntil(Promise.all([logUsage(env, "d1_write", "crawl"), logUsage(env, "r2_put", key)]));
        return json({
          ok: true,
          id: r.meta.last_row_id,
          source,
          ...(containerError ? { container_error: containerError } : {}),
          title: result.title,
          links: result.links,
          r2_key: key,
        });
      }
      if (p === "/api/crawls" && req.method === "GET") {
        const { results } = await env.DB.prepare(
          "SELECT id, url, source, title, links, status_code, created_at FROM crawls ORDER BY id DESC LIMIT 50",
        ).all();
        return json({ crawls: results });
      }

      // Everything else: static dashboard from Workers Assets.
      return env.ASSETS.fetch(req);
    } catch (e) {
      return json({ error: String(e) }, 500);
    }
  },

  // Queue consumer: index uploaded files (hash + mark indexed in D1).
  async queue(batch: MessageBatch<Job>, env: Env): Promise<void> {
    for (const msg of batch.messages) {
      try {
        if (msg.body.type === "index-file") {
          const obj = await env.FILES.get(msg.body.r2Key);
          if (obj) {
            const digest = await sha256Hex(await obj.arrayBuffer());
            await env.DB.prepare("UPDATE files SET sha256=?1, indexed=1 WHERE r2_key=?2")
              .bind(digest, msg.body.r2Key)
              .run();
            await logUsage(env, "d1_write", "file-index");
          }
        }
        msg.ack();
      } catch {
        msg.retry();
      }
    }
  },

  // Cron: daily housekeeping — trim the usage ledger, refresh KV snapshot.
  async scheduled(_event: ScheduledController, env: Env, ctx: ExecutionContext): Promise<void> {
    ctx.waitUntil(
      (async () => {
        await env.DB.prepare("DELETE FROM usage_events WHERE created_at < datetime('now','-90 days')").run();
        await logUsage(env, "d1_write", "cron-ledger-trim");
        const { results } = await env.DB.prepare("SELECT * FROM machines ORDER BY name").all();
        await env.CACHE.put("machines:all", JSON.stringify({ machines: results }, null, 2), { expirationTtl: 3600 });
        console.log("lab-ops-hub cron: ledger trimmed, machines snapshot refreshed");
      })(),
    );
  },
} satisfies ExportedHandler<Env, Job>;
