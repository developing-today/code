// The aggregator's HTTP server. Read-mostly: pages are server-rendered
// HTML and /api/*.json serve the same data machine-first. Slice 3 (#906)
// adds the app's first writes — dashboard comments (POST /api/comments
// and its triage/github-result companions); everything underneath
// (GitHub, Kaneo, Grist) stays read-only from here.
import { CommentsStore, validateNewComment } from "./comments.ts";
import { loadConfig } from "./config.ts";
import { allRefs, buildAttention, deriveToday, type RoadmapGrain } from "./derive.ts";
import type { DashComment, RefKey } from "./model.ts";
import { identitiesFromRecords } from "./registry.ts";
import {
  ghObjectRef,
  renderAttention,
  renderBoard,
  renderDiscuss,
  renderDns,
  renderItem,
  renderPages,
  renderQuota,
  renderRoadmap,
  renderSessions,
  renderToday,
  type CommentFormOpts,
} from "./render.ts";
import type { Source } from "./source.ts";
import { fixtureSource } from "./sources/fixtures.ts";
import { liveSource } from "./sources/live.ts";

function pickSource(): Source {
  const cfg = loadConfig();
  return cfg.source === "live" ? liveSource(cfg) : fixtureSource();
}

const source = pickSource();
const cfg = loadConfig();
const commentsStore = new CommentsStore(cfg.commentsFile);

/** Author identities for the comment form: standing + registry sessions. */
async function commentFormOpts(githubRefs: RefKey[] = []): Promise<CommentFormOpts> {
  const records = await source.registry().catch(() => []);
  return { githubRefs, identities: identitiesFromRecords(records) };
}

/** Resolve a discuss-page title for any object ref, best-effort. */
async function discussTitle(objectRef: string): Promise<{ title: string; githubRefs: RefKey[] }> {
  if (objectRef.startsWith("gh:")) {
    const ref = objectRef.slice(3);
    const items = await source.items([ref]).catch(() => new Map());
    return { title: items.get(ref)?.title ?? `Comments — ${ref}`, githubRefs: [ref] };
  }
  if (objectRef.startsWith("session:")) {
    const id = objectRef.slice("session:".length);
    const records = await source.registry().catch(() => []);
    const rec = records.find((r) => r.id === id);
    return { title: `Comments — ${rec?.label ?? id}`, githubRefs: rec?.taskRefs ?? [] };
  }
  if (objectRef.startsWith("orch:")) {
    const id = objectRef.slice("orch:".length);
    const section = await source.orchestration().catch(() => null);
    const item = [...(section?.tiers.needsDrew ?? []), ...(section?.tiers.needsOrchestrator ?? [])].find(
      (i) => i.id === id,
    );
    return {
      title: `Comments — ${item?.title ?? id}`,
      githubRefs: item?.ref ? [item.ref] : [],
    };
  }
  const [board, rows] = await Promise.all([source.board().catch(() => []), source.roadmap().catch(() => [])]);
  const item = board.find((b) => b.id === objectRef);
  if (item) return { title: `Comments — ${item.title}`, githubRefs: item.links.map((l) => l.ref) };
  const row = rows.find((r) => r.subject === objectRef);
  if (row) return { title: `Comments — ${row.title}`, githubRefs: row.links.map((l) => l.ref) };
  return { title: `Comments — ${objectRef}`, githubRefs: [] };
}

async function readJsonBody(req: Request): Promise<unknown> {
  const text = await req.text();
  if (text.length > 100_000) throw new Error("body too large");
  return JSON.parse(text);
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body, null, 2), {
    status,
    headers: { "content-type": "application/json; charset=utf-8" },
  });
}

function html(body: string): Response {
  return new Response(body, { headers: { "content-type": "text/html; charset=utf-8" } });
}

async function boardContext() {
  const board = await source.board();
  const refs = allRefs(board);
  const items = await source.items(refs);
  return { board, items };
}

async function roadmapContext() {
  const rows = await source.roadmap();
  const refs = allRefs(rows);
  const items = await source.items(refs);
  return { rows, items };
}

async function attentionContext() {
  const [feed, summary, coverage] = await Promise.all([source.attention(), source.orchestration(), source.coverage()]);
  return { view: buildAttention(feed, summary), coverage };
}

async function todayContext() {
  const [{ board, items }, { rows }, { view, coverage }, quota, summary] = await Promise.all([
    boardContext(),
    roadmapContext(),
    attentionContext(),
    source.quota(),
    source.orchestration(),
  ]);
  return { today: deriveToday(board, rows, items, summary), view, coverage, quota };
}

const GRAINS = new Set<string>(["quarter", "month", "week", "3d", "today"]);

async function handler(req: Request): Promise<Response> {
  const url = new URL(req.url);
  const path = url.pathname;
  const prov = () => source.provenance();

  // --- Dashboard comment writes (#906 §2) --------------------------------
  if (req.method === "POST" && path === "/api/comments") {
    try {
      const parsed = validateNewComment(await readJsonBody(req));
      if (!parsed.ok) return json({ error: parsed.error }, 400);
      const comment = await commentsStore.add(parsed.value);
      return json({ comment }, 201);
    } catch (err) {
      return json({ error: `invalid comment: ${err instanceof Error ? err.message : err}` }, 400);
    }
  }
  if (req.method === "POST" && path === "/api/comments/triage") {
    try {
      const body = (await readJsonBody(req)) as { ids?: unknown };
      const ids = Array.isArray(body.ids) ? body.ids.filter((x): x is string => typeof x === "string") : [];
      return json({ triaged: await commentsStore.markTriaged(ids) });
    } catch (err) {
      return json({ error: `invalid triage: ${err instanceof Error ? err.message : err}` }, 400);
    }
  }
  if (req.method === "POST" && path === "/api/comments/github-result") {
    try {
      const body = (await readJsonBody(req)) as { id?: unknown; url?: unknown; error?: unknown };
      if (typeof body.id !== "string") return json({ error: "id required" }, 400);
      const updated = await commentsStore.markGithubResult(body.id, {
        url: typeof body.url === "string" ? body.url : undefined,
        error: typeof body.error === "string" ? body.error : undefined,
      });
      return updated ? json({ comment: updated }) : json({ error: "no such github comment" }, 404);
    } catch (err) {
      return json({ error: `invalid result: ${err instanceof Error ? err.message : err}` }, 400);
    }
  }
  if (path === "/api/comments.json") {
    const object = url.searchParams.get("object");
    const kind = url.searchParams.get("kind");
    const state = url.searchParams.get("state");
    let list: DashComment[] = await commentsStore.all();
    if (object) list = list.filter((c) => c.objectRef === object);
    if (kind) list = list.filter((c) => c.kind === kind);
    if (state === "new") list = list.filter((c) => c.triageState === "new" || c.githubState === "pending");
    const all = await commentsStore.all();
    return json({
      comments: list,
      pending: {
        messages: all.filter((c) => c.kind === "message" && c.triageState === "new").length,
        github: all.filter((c) => c.kind === "github" && c.githubState === "pending").length,
      },
    });
  }

  if (path === "/" || path === "/board") {
    const { board, items } = await boardContext();
    return html(renderBoard(board, items, prov(), await commentsStore.counts()));
  }

  if (path === "/today") {
    const { today, view, coverage, quota } = await todayContext();
    return html(renderToday(today, view, quota, coverage, prov()));
  }

  if (path === "/roadmap") {
    const { rows, items } = await roadmapContext();
    const g = url.searchParams.get("h") ?? "quarter";
    const grain: RoadmapGrain = GRAINS.has(g) ? (g as RoadmapGrain) : "quarter";
    return html(renderRoadmap(rows, items, prov(), grain, await commentsStore.counts()));
  }

  if (path === "/quota") {
    return html(renderQuota(await source.quota(), prov()));
  }

  if (path === "/attention") {
    const { view, coverage } = await attentionContext();
    return html(renderAttention(view, coverage, prov(), await commentsStore.counts()));
  }

  if (path === "/dns") {
    return html(renderDns(await source.dns(), prov()));
  }
  if (path === "/api/dns.json") {
    const section = await source.dns();
    return json(section ?? { status: "absent", reason: "no DNS register section in the fleet feed yet" });
  }

  if (path === "/sessions") {
    return html(renderSessions(await source.registry(), prov()));
  }
  if (path === "/api/registry.json") {
    const records = await source.registry();
    return json({ records, identities: identitiesFromRecords(records) });
  }

  if (path === "/pages") {
    return html(renderPages(prov()));
  }

  const discussMatch = /^\/discuss\/(.+)$/.exec(path);
  if (discussMatch) {
    const objectRef = decodeURIComponent(discussMatch[1]);
    const { title, githubRefs } = await discussTitle(objectRef);
    const thread = await commentsStore.forObject(objectRef);
    return html(renderDiscuss(objectRef, title, thread, await commentFormOpts(githubRefs), prov()));
  }

  if (path === "/api/board.json") {
    const { board, items } = await boardContext();
    return json({ board, items: [...items.values()] });
  }
  if (path === "/api/roadmap.json") {
    const { rows, items } = await roadmapContext();
    return json({ rows, items: [...items.values()] });
  }
  if (path === "/api/quota.json") {
    return json(await source.quota());
  }
  if (path === "/api/attention.json") {
    // Slice 2: the flat array shape stays; entries are now current-only
    // (expired dropped) and tier-annotated (tier/drewReason/
    // orchestratorNote, source, parentId).
    const { view } = await attentionContext();
    return json(view.all);
  }
  if (path === "/api/coverage.json") {
    const { view, coverage } = await attentionContext();
    return json({ coverage, expired: view.expiredCount });
  }
  if (path === "/api/today.json") {
    const { today } = await todayContext();
    return json(today);
  }
  if (path === "/api/orchestration.json") {
    // #899: the orchestration job's tiers + goal measurements. Absent
    // means "not measured yet" — never an empty queue.
    const section = await source.orchestration();
    return json(section ?? { status: "absent", reason: "no orchestration run has published yet" });
  }
  if (path === "/api/health") {
    return json({ status: "ok", source: prov() });
  }

  const itemMatch = /^\/item\/(.+)$/.exec(path);
  if (itemMatch) {
    const id = decodeURIComponent(itemMatch[1]);
    const [board, rows] = await Promise.all([source.board(), source.roadmap()]);
    const subject = board.find((b) => b.id === id) ?? rows.find((r) => r.subject === id);
    if (subject) {
      const title = subject.title;
      const links = subject.links;
      const notes = "repo" in subject ? subject.notes : undefined;
      const refs = allRefs([subject]);
      const [items, comments, allComments, records] = await Promise.all([
        source.items(refs),
        source.comments(refs),
        commentsStore.all(),
        source.registry().catch(() => []),
      ]);
      // Dashboard comments for this item live on its own subject ref
      // and on each linked GitHub ref (gh:<ref>) — one merged thread.
      const wanted = new Set([id, ...refs.map((r) => ghObjectRef(r))]);
      const dashComments = allComments.filter((c) => wanted.has(c.objectRef));
      const sessions = records.filter((r) => r.taskRefs.some((ref) => refs.includes(ref)));
      const extras = {
        dashComments,
        formRef: id,
        formOpts: await commentFormOpts(refs),
        sessions,
      };
      return html(renderItem(title, id, links, notes, items, comments, prov(), extras));
    }
    return html(`<h1>Not found</h1><p class="muted">no board item or roadmap row with subject <code>${id}</code></p>`);
  }

  return json({ error: "not found", path }, 404);
}

// One server per bind address (PM_AGG_HOSTS). Deployment on the
// laptop binds loopback + the tailnet address, like Pad/Grist — no
// public route (Drew, 2026-10-25: public only behind a password).
const servers = cfg.hosts.map((hostname) =>
  Bun.serve({
    hostname,
    port: cfg.port,
    fetch: handler,
  }),
);

for (const server of servers) {
  console.log(`aggregator listening on http://${server.hostname}:${server.port} (source: ${cfg.source})`);
}
