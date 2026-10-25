// The live reader: Kaneo's per-repo boards and Grist's roadmap table read
// directly (the projector does not project them yet — J, 2026-10-25), and
// the projector's /items and /comments for live ref state and the comment
// projection. Anything a reader cannot reach falls back to fixtures, and the
// provenance strings say exactly which facet is live vs fixture-backed.
import type { Config } from "../config.ts";
import { fixtureSource } from "./fixtures.ts";
import {
  type BoardItem,
  type Comment,
  type GhItem,
  type Link,
  type Provenance,
  type QuotaView,
  type RefKey,
  type RoadmapRow,
  type SessionFeedEntry,
  refsInText,
  refUrl,
} from "../model.ts";
import type { Source } from "../source.ts";

interface FetchResult {
  ok: boolean;
  json: unknown;
}

async function getJson(
  url: string,
  headers: Record<string, string> = {},
  init: RequestInit = {},
): Promise<FetchResult> {
  try {
    const res = await fetch(url, { headers, signal: AbortSignal.timeout(8000), ...init });
    if (!res.ok) return { ok: false, json: { status: res.status } };
    return { ok: true, json: await res.json() };
  } catch (err) {
    return { ok: false, json: { error: String(err) } };
  }
}

// --- Kaneo --------------------------------------------------------------

interface KaneoTask {
  id: string;
  number: number;
  title: string;
  status: string;
  priority?: string;
  createdAt?: string;
  externalLinks?: {
    resourceType: string;
    externalId: string;
    url: string;
  }[];
}

interface KaneoColumn {
  id: string;
  name: string;
  tasks: KaneoTask[];
}

async function kaneoProjects(cfg: Config, cookie: string): Promise<{ id: string; name: string; repo: string }[]> {
  const ws = await getJson(`${cfg.kaneo.baseUrl}/api/workspace`, { Cookie: cookie });
  if (!ws.ok) throw new Error("kaneo workspace read failed");
  const workspaces = ws.json as { id: string }[];
  const workspaceId = workspaces[0]?.id;
  if (!workspaceId) throw new Error("kaneo workspace empty");
  const proj = await getJson(`${cfg.kaneo.baseUrl}/api/project?workspaceId=${workspaceId}`, { Cookie: cookie });
  if (!proj.ok) throw new Error("kaneo project read failed");
  return (proj.json as { id: string; slug: string; name: string }[]).map((p) => ({
    id: p.id,
    name: p.name,
    repo: `developing-today/${p.slug}`,
  }));
}

async function kaneoBoard(cfg: Config, cookie: string): Promise<BoardItem[]> {
  const projects = await kaneoProjects(cfg, cookie);
  const out: BoardItem[] = [];
  for (const p of projects) {
    const all: KaneoColumn[] = [];
    for (let page = 1; ; page++) {
      const r = await getJson(`${cfg.kaneo.baseUrl}/api/task/tasks/${p.id}?page=${page}`, { Cookie: cookie });
      if (!r.ok) break;
      const body = r.json as { data?: { columns?: KaneoColumn[] }; pagination?: { totalPages: number } };
      const cols = body.data?.columns ?? [];
      all.push(...cols);
      const totalPages = body.pagination?.totalPages ?? 1;
      if (page >= totalPages || totalPages <= 1) break;
    }
    // The curated in-flight set, not every open issue: in-progress,
    // in-review, and done (recent) columns only. To-do is the backlog.
    const wipStatuses = new Set(["in-progress", "in-review", "done"]);
    for (const col of all) {
      if (!wipStatuses.has(col.id)) continue;
      for (const t of col.tasks) {
        const links: Link[] = (t.externalLinks ?? [])
          .filter((l) => l.resourceType === "issue" || l.resourceType === "pull_request")
          .map((l) => {
            const m = /github\.com\/([\w.-]+\/[\w.-]+)\//.exec(l.url);
            const repo = m ? m[1] : p.repo;
            return { ref: `${repo}#${l.externalId}`, relation: "tracks" as const };
          });
        out.push({
          id: `kaneo:${p.name}/${t.id}`,
          title: t.title,
          repo: p.repo,
          source: "kaneo",
          status: col.id,
          priority: t.priority,
          updatedAt: t.createdAt,
          kaneoUrl: cfg.kaneo.baseUrl,
          links,
        });
      }
    }
  }
  return out;
}

// --- Grist --------------------------------------------------------------

interface GristRecord {
  id: number;
  fields: Record<string, unknown>;
}

async function gristRoadmap(cfg: Config): Promise<RoadmapRow[]> {
  if (!cfg.grist.apiKey) throw new Error("GRIST_API_KEY not set");
  const r = await getJson(`${cfg.grist.baseUrl}/api/docs/${cfg.grist.docId}/tables/Roadmap/records`, {
    Authorization: `Bearer ${cfg.grist.apiKey}`,
  });
  if (!r.ok) throw new Error("grist roadmap read failed");
  const records = (r.json as { records?: GristRecord[] }).records ?? [];
  return records.map((rec) => {
    const f = rec.fields;
    const str = (v: unknown) => (typeof v === "string" ? v : "");
    const source = str(f.Source);
    const notes = str(f.Notes);
    const links = refsInText(`${source} ${notes}`).map((ref) => ({ ref, relation: "tracks" as const }));
    return {
      id: rec.id,
      subject: `grist:${cfg.grist.docId}/Roadmap/${rec.id}`,
      title: str(f.Title),
      area: str(f.Area),
      status: str(f.Status),
      horizon: str(f.Horizon),
      source,
      notes,
      links,
    };
  });
}

// --- Projector (J's service) --------------------------------------------

async function projectorItems(baseUrl: string, refs: RefKey[]): Promise<Map<RefKey, GhItem> | null> {
  const r = await getJson(`${baseUrl}/items?refs=${refs.join(",")}`);
  if (!r.ok) return null;
  const items = (r.json as { items?: GhItem[] }).items ?? [];
  return new Map(items.map((i) => [i.ref, i]));
}

async function projectorComments(baseUrl: string, refs: RefKey[]): Promise<Comment[] | null> {
  const r = await getJson(`${baseUrl}/comments?refs=${refs.join(",")}`);
  if (!r.ok) return null;
  return ((r.json as { comments?: Comment[] }).comments ?? []).filter((c) => !c.deleted);
}

// --- Composition ----------------------------------------------------------

export function liveSource(cfg: Config): Source {
  const fix = fixtureSource();
  const provenance: Provenance = {
    board: "fixtures",
    roadmap: "fixtures",
    items: "fixtures",
    comments: "fixtures",
    quota: "fixtures",
    attention: "fixtures",
  };

  async function kaneoCookieHeader(): Promise<string> {
    if (!cfg.kaneo.email || !cfg.kaneo.password) throw new Error("no kaneo credentials");
    const res = await fetch(`${cfg.kaneo.baseUrl}/api/auth/sign-in/email`, {
      method: "POST",
      headers: { "Content-Type": "application/json", Origin: cfg.kaneo.origin },
      body: JSON.stringify({ email: cfg.kaneo.email, password: cfg.kaneo.password }),
      signal: AbortSignal.timeout(8000),
    });
    const setCookie = res.headers.get("set-cookie") ?? "";
    const cookie = setCookie.split(";")[0];
    if (!res.ok || !cookie) throw new Error(`kaneo sign-in failed (${res.status})`);
    return cookie;
  }

  // Kaneo's session cookie is valid for a week; cache it for the process.
  let cookieCache: { value: string; at: number } | null = null;
  async function kaneoCookie(): Promise<string> {
    if (cookieCache && Date.now() - cookieCache.at < 3600_000) return cookieCache.value;
    const value = await kaneoCookieHeader();
    cookieCache = { value, at: Date.now() };
    return value;
  }

  return {
    async board() {
      try {
        const cookie = await kaneoCookie();
        const board = await kaneoBoard(cfg, cookie);
        if (board.length > 0) {
          provenance.board = "live (Kaneo)";
          return board;
        }
      } catch (err) {
        provenance.board = `fixtures (kaneo unreachable: ${err instanceof Error ? err.message : err})`;
      }
      return fix.board();
    },
    async roadmap() {
      try {
        const rows = await gristRoadmap(cfg);
        if (rows.length > 0) {
          provenance.roadmap = "live (Grist)";
          return rows;
        }
      } catch (err) {
        provenance.roadmap = `fixtures (grist unreachable: ${err instanceof Error ? err.message : err})`;
      }
      return fix.roadmap();
    },
    async items(refs) {
      if (cfg.projector.baseUrl) {
        const live = await projectorItems(cfg.projector.baseUrl, refs);
        if (live && live.size > 0) {
          provenance.items = "live (projector)";
          return live;
        }
      }
      return fix.items(refs);
    },
    async comments(refs) {
      if (cfg.projector.baseUrl) {
        const live = await projectorComments(cfg.projector.baseUrl, refs);
        if (live) {
          provenance.comments = "live (projector)";
          return live;
        }
      }
      return fix.comments(refs);
    },
    async quota() {
      // The quota reader (per the #851 source list) is a projector concern;
      // until it lands there, fixtures stand in.
      return fix.quota();
    },
    async attention() {
      // The session-state feed (per #852) is a projector/watch-cron concern;
      // until it lands there, fixtures stand in.
      return fix.attention();
    },
    provenance: () => provenance,
  };
}

export { refUrl };
