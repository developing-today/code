// The live reader: Kaneo's per-repo boards and Grist's roadmap table read
// directly (the projector does not project them yet — J, 2026-10-25), and
// the projector's /items and /comments for live ref state and the comment
// projection. Anything a reader cannot reach falls back to fixtures, and the
// provenance strings say exactly which facet is live vs fixture-backed.
import { readFile } from "node:fs/promises";
import type { Config } from "../config.ts";
import { feedAgeText, isOrchestrationSection, loadFeed } from "../feed.ts";
import { fixtureSource } from "./fixtures.ts";
import { mapServerSessions } from "./opencode.ts";
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

/** Kaneo project slug -> GitHub repo (pm-stack/kaneo-projects.json, Session H). */
const KANEO_PROJECT_REPOS: Record<string, string> = {
  code: "developing-today/code",
  opencode: "developing-today/opencode",
  openchamber: "developing-today/openchamber",
  nix: "dezren39/nix",
};

async function kaneoProjects(
  cfg: Config,
  cookie: string,
): Promise<{ id: string; slug: string; name: string; repo: string }[]> {
  const ws = await getJson(`${cfg.kaneo.baseUrl}/api/workspace`, { Cookie: cookie });
  if (!ws.ok) throw new Error("kaneo workspace read failed");
  const workspaces = ws.json as { id: string }[];
  const workspaceId = workspaces[0]?.id;
  if (!workspaceId) throw new Error("kaneo workspace empty");
  const proj = await getJson(`${cfg.kaneo.baseUrl}/api/project?workspaceId=${workspaceId}`, { Cookie: cookie });
  if (!proj.ok) throw new Error("kaneo project read failed");
  return (proj.json as { id: string; slug: string; name: string }[]).map((p) => ({
    id: p.id,
    slug: p.slug,
    name: p.name,
    repo: KANEO_PROJECT_REPOS[p.slug] ?? `developing-today/${p.slug}`,
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
          // Registry subject: kaneo:<projectSlug>/<taskId> (Session J).
          id: `kaneo:${p.slug}/${t.id}`,
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

// --- Projector (J's service, pm-stack/projector) -------------------------
// The wire schema is the projector's schema.sql: snake_case fields, `state`
// is only open|closed with `merged` as a separate 0/1 flag, and
// `labels`/`assignees` are JSON array *strings*. Map rows to the read model
// here; nothing else in the app sees the wire shape.

export interface ProjectorItemRow {
  ref: string;
  repo: string;
  number: number;
  kind: "issue" | "pr";
  url: string;
  title: string;
  state: "open" | "closed";
  state_reason?: string | null;
  draft?: number | boolean;
  merged?: number | boolean;
  author?: string | null;
  labels?: string | string[];
  assignees?: string | string[];
  created_at: string;
  updated_at: string;
  closed_at?: string | null;
  merged_at?: string | null;
}

export interface ProjectorCommentRow {
  id: string;
  ref: string;
  kind: Comment["kind"];
  author?: string | null;
  body?: string | null;
  created_at: string;
  url?: string;
  path?: string | null;
  line?: number | null;
  review_state?: string | null;
  commit_id?: string | null;
  deleted?: number | boolean;
}

export interface ProjectorLinkRow {
  subject: string;
  ref: string;
  relation: string;
  note?: string | null;
  created_by?: string | null;
  created_at?: string;
}

const asBool = (v: number | boolean | undefined): boolean => v === 1 || v === true;

export function parseProjectorLabels(labels: string | string[] | undefined): string[] {
  if (Array.isArray(labels)) return labels;
  if (typeof labels !== "string" || labels === "") return [];
  try {
    const parsed: unknown = JSON.parse(labels);
    return Array.isArray(parsed) ? parsed.filter((x): x is string => typeof x === "string") : [];
  } catch {
    return [];
  }
}

export function mapProjectorItem(row: ProjectorItemRow): GhItem {
  return {
    ref: row.ref,
    kind: row.kind,
    url: row.url,
    title: row.title,
    state: asBool(row.merged) ? "merged" : row.state,
    draft: asBool(row.draft),
    labels: parseProjectorLabels(row.labels),
    author: row.author ?? "",
    updatedAt: row.updated_at,
  };
}

export function mapProjectorComment(row: ProjectorCommentRow): Comment {
  return {
    id: String(row.id),
    ref: row.ref,
    kind: row.kind,
    author: row.author ?? "",
    body: row.body ?? "",
    createdAt: row.created_at,
    url: row.url ?? refUrl(row.ref, row.kind === "issue_comment" ? "issue" : "pr"),
    path: row.path ?? undefined,
    line: row.line ?? undefined,
    reviewState: row.review_state ?? undefined,
    deleted: asBool(row.deleted),
  };
}

/** Union two link lists, deduped by ref+relation (first relation wins). */
export function mergeLinks(existing: Link[], extra: Link[]): Link[] {
  const seen = new Set(existing.map((l) => `${l.ref}|${l.relation}`));
  const out = [...existing];
  for (const l of extra) {
    const key = `${l.ref}|${l.relation}`;
    if (!seen.has(key)) {
      seen.add(key);
      out.push(l);
    }
  }
  return out;
}

const RELATIONS = new Set<string>(["tracks", "implements", "relates"]);
const asRelation = (v: string): Link["relation"] => (RELATIONS.has(v) ? (v as Link["relation"]) : "relates");

export async function projectorItems(baseUrl: string, refs: RefKey[]): Promise<Map<RefKey, GhItem> | null> {
  if (refs.length === 0) return new Map();
  // Refs contain `#` — they must be percent-encoded or the `#` starts a
  // fragment and the projector never sees the number.
  const r = await getJson(`${baseUrl}/items?refs=${encodeURIComponent(refs.join(","))}`);
  if (!r.ok) return null;
  const rows = (r.json as { items?: ProjectorItemRow[] }).items ?? [];
  return new Map(rows.map((row) => [row.ref, mapProjectorItem(row)] as const));
}

export async function projectorComments(baseUrl: string, refs: RefKey[]): Promise<Comment[] | null> {
  if (refs.length === 0) return [];
  const r = await getJson(`${baseUrl}/comments?refs=${encodeURIComponent(refs.join(","))}`);
  if (!r.ok) return null;
  return ((r.json as { comments?: ProjectorCommentRow[] }).comments ?? [])
    .map(mapProjectorComment)
    .filter((c) => !c.deleted);
}

/** Registry links for one subject (kaneo:/grist:/gh:), from GET /links. */
export async function projectorLinks(baseUrl: string, subject: string): Promise<Link[]> {
  const r = await getJson(`${baseUrl}/links?subject=${encodeURIComponent(subject)}`);
  if (!r.ok) return [];
  const rows = (r.json as { links?: ProjectorLinkRow[] }).links ?? [];
  return rows.map((row) => ({ ref: row.ref, relation: asRelation(row.relation) }));
}

/**
 * Enrich board items / roadmap rows with their cross-repo registry links.
 * This is the first-class requirement Kaneo cannot provide: the registry is
 * where a `code` task's link to an `opencode` PR lives.
 */
async function enrichWithRegistryLinks<T extends { links: Link[] }>(
  baseUrl: string,
  entries: T[],
  subjectOf: (entry: T) => string,
): Promise<T[]> {
  return Promise.all(
    entries.map(async (entry) => {
      const extra = await projectorLinks(baseUrl, subjectOf(entry));
      return extra.length ? { ...entry, links: mergeLinks(entry.links, extra) } : entry;
    }),
  );
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

  // The laptop's OpenChamber-managed OpenCode server: port + password
  // are read from its state file at request time (the port changes
  // across restarts). Mapped sessions are cached for 45 s (#890).
  let laptopCache: { value: SessionFeedEntry[]; at: number } | null = null;
  async function laptopSessions(): Promise<SessionFeedEntry[] | null> {
    if (laptopCache && Date.now() - laptopCache.at < 45_000) return laptopCache.value;
    const state = JSON.parse(await readFile(cfg.openchamberStateFile, "utf8")) as {
      port?: number;
      password?: string;
    };
    if (!state.port || !state.password) return null;
    const mapped = await mapServerSessions({
      baseUrl: `http://127.0.0.1:${state.port}`,
      password: state.password,
    });
    if (!mapped) return null;
    laptopCache = { value: mapped.entries, at: Date.now() };
    return mapped.entries;
  }

  return {
    async board() {
      try {
        const cookie = await kaneoCookie();
        const board = await kaneoBoard(cfg, cookie);
        if (board.length > 0) {
          provenance.board = "live (Kaneo)";
          if (cfg.projector.baseUrl) {
            const enriched = await enrichWithRegistryLinks(cfg.projector.baseUrl, board, (b) => b.id);
            provenance.board = "live (Kaneo + projector links)";
            return enriched;
          }
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
          if (cfg.projector.baseUrl) {
            const enriched = await enrichWithRegistryLinks(cfg.projector.baseUrl, rows, (r) => r.subject);
            provenance.roadmap = "live (Grist + projector links)";
            return enriched;
          }
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
      // Slice 1 (#890): quota comes from the fleet feed file produced
      // on the sandbox by scripts/collect-fleet-feed.ts. A stale feed
      // keeps rendering, with its age in provenance; fixtures only when
      // no feed exists at all.
      const feed = await loadFeed(cfg.feedFile);
      if (feed) {
        provenance.quota = `live (fleet feed · ${feedAgeText(feed.generatedAt)} · collected ${feed.generatedAt})`;
        return feed.quota;
      }
      provenance.quota = "fixtures (no fleet feed file yet)";
      return fix.quota();
    },
    async attention() {
      // Slice 1 (#890): laptop sessions live from the OpenChamber-
      // managed OpenCode server (45 s cache); sandbox sessions from
      // the fleet feed (the sandbox server binds its own loopback).
      const parts: string[] = [];
      const byId = new Map<string, SessionFeedEntry>();

      const feed = await loadFeed(cfg.feedFile);
      if (feed) {
        for (const e of feed.attention) byId.set(e.sessionId, e);
        parts.push(`sandbox via fleet feed (${feedAgeText(feed.generatedAt)})`);
      }

      try {
        const laptop = await laptopSessions();
        if (laptop) {
          for (const e of laptop) byId.set(e.sessionId, e);
          parts.push("laptop live");
        }
      } catch {
        /* laptop part fails soft */
      }

      if (byId.size > 0) {
        provenance.attention = `live (${parts.join(" + ")})`;
        return [...byId.values()];
      }
      provenance.attention = "fixtures (no live attention source reachable)";
      return fix.attention();
    },
    async orchestration() {
      // #899: the orchestration section rides the fleet feed, embedded
      // by the collector from the orchestrator's own output file. The
      // section carries its own provenance; absent = not measured yet.
      const feed = await loadFeed(cfg.feedFile);
      if (feed?.orchestration && isOrchestrationSection(feed.orchestration)) {
        return feed.orchestration;
      }
      return null;
    },
    provenance: () => provenance,
  };
}

export { refUrl };
