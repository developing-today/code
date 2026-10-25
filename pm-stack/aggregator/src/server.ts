// The aggregator's HTTP server. Read-mostly: every route is a GET. Pages are
// server-rendered HTML; /api/*.json serve the same data machine-first.
import { loadConfig } from "./config.ts";
import { allRefs, buildAttention, deriveToday, type RoadmapGrain } from "./derive.ts";
import { renderAttention, renderBoard, renderItem, renderQuota, renderRoadmap, renderToday } from "./render.ts";
import type { Source } from "./source.ts";
import { fixtureSource } from "./sources/fixtures.ts";
import { liveSource } from "./sources/live.ts";

function pickSource(): Source {
  const cfg = loadConfig();
  return cfg.source === "live" ? liveSource(cfg) : fixtureSource();
}

const source = pickSource();
const cfg = loadConfig();

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

  if (path === "/" || path === "/board") {
    const { board, items } = await boardContext();
    return html(renderBoard(board, items, prov()));
  }

  if (path === "/today") {
    const { today, view, coverage, quota } = await todayContext();
    return html(renderToday(today, view, quota, coverage, prov()));
  }

  if (path === "/roadmap") {
    const { rows, items } = await roadmapContext();
    const g = url.searchParams.get("h") ?? "quarter";
    const grain: RoadmapGrain = GRAINS.has(g) ? (g as RoadmapGrain) : "quarter";
    return html(renderRoadmap(rows, items, prov(), grain));
  }

  if (path === "/quota") {
    return html(renderQuota(await source.quota(), prov()));
  }

  if (path === "/attention") {
    const { view, coverage } = await attentionContext();
    return html(renderAttention(view, coverage, prov()));
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
    const item = board.find((b) => b.id === id);
    if (item) {
      const refs = allRefs([item]);
      const [items, comments] = await Promise.all([source.items(refs), source.comments(refs)]);
      return html(renderItem(item.title, item.id, item.links, item.notes, items, comments, prov()));
    }
    const row = rows.find((r) => r.subject === id);
    if (row) {
      const refs = allRefs([row]);
      const [items, comments] = await Promise.all([source.items(refs), source.comments(refs)]);
      return html(renderItem(row.title, row.subject, row.links, undefined, items, comments, prov()));
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
