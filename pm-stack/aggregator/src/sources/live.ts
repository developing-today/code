// The live reader: Kaneo's per-repo boards and Grist's roadmap table read
// directly (the projector does not project them yet — J, 2026-10-25), and
// the projector's /items and /comments for live ref state and the comment
// projection. Anything a reader cannot reach falls back to fixtures, and the
// provenance strings say exactly which facet is live vs fixture-backed.
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { dirname } from "node:path";
import type { Config } from "../config.ts";
import { isDnsSection } from "../dns.ts";
import {
  feedAgeText,
  normalizeOrchestrationSection,
  orchestrationSectionProblem,
  loadFeed,
  type FleetFeed,
} from "../feed.ts";
import { isSessionRecord, mergeRecords, recordFromSessionEntry } from "../registry.ts";
import { buildAttentionFlags } from "../derive.ts";
import { summarizeDns } from "../dns.ts";
import { extractCharges, extractPaygLines, extractSaasServices, extractSubscriptions } from "../saas-extract.ts";
import { isProjectPoolItem, poolItemToGhItem } from "../projects.ts";
import { SnapshotStore } from "../snapshots.ts";
import { fixtureSource } from "./fixtures.ts";
import { mapServerSessions, type MappedSessions } from "./opencode.ts";
import {
  type BoardItem,
  type Comment,
  type DnsSection,
  type GhItem,
  type Link,
  type OrchestrationSection,
  type ProjectPoolItem,
  type Provenance,
  type QuotaView,
  type RefKey,
  type RoadmapRow,
  type SessionFeedEntry,
  type SessionRecord,
  type SourceCoverage,
  type UsageView,
  parseRef,
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

// --- Orchestrator in-flight lane -------------------------------------------
// Kaneo's columns are a completion record in practice: the reader above
// keeps only in-progress / in-review / done, to-do is dropped as backlog,
// and in practice the live board holds done and in-review cards only —
// on their own they cannot answer "what is being worked on today". The
// fleet feed's orchestration section (#899) can: its per-goal linedUp
// tasks are the orchestrator's current queue. board() merges the
// in-progress / queued / blocked ones in as synthetic entries with the
// board status "in-flight", so renderBoard groups them into their own
// lane, visibly sourced from the orchestrator. Kaneo is never written
// to — its cards stay the completion record underneath.

/** Repo shorthands the orchestrator uses in linedUp titles. */
const TITLE_REPO_ALIASES: Record<string, string> = {
  code: "developing-today/code",
  opencode: "developing-today/opencode",
  openchamber: "developing-today/openchamber",
  nix: "dezren39/nix",
};

/**
 * GitHub refs a linedUp title carries, as ref keys: "owner/repo#N" and
 * GitHub URLs first, then shorthand "#N" mentions attributed to the
 * repo alias ("code", "nix", …) nearest the mention in the title
 * (a preceding alias wins, else the following one). A "#N" with no
 * alias anywhere in the title is not a ref we can name — the same text
 * also carries queue/task numbers — so it is left unlinked rather
 * than guessed at.
 */
export function titleRefs(title: string): RefKey[] {
  const out: RefKey[] = [];
  const add = (ref: RefKey) => {
    if (!out.includes(ref)) out.push(ref);
  };
  for (const ref of refsInText(title)) add(ref);
  for (const m of title.matchAll(/([\w.-]+\/[\w.-]+)#(\d+)/g)) add(`${m[1]}#${m[2]}`);
  const aliases: { at: number; repo: string }[] = [];
  for (const m of title.matchAll(/\b(code|opencode|openchamber|nix)\b/gi)) {
    aliases.push({ at: m.index ?? 0, repo: TITLE_REPO_ALIASES[m[1].toLowerCase()] });
  }
  if (aliases.length) {
    for (const m of title.matchAll(/#(\d+)/g)) {
      const at = m.index ?? 0;
      const before = aliases.filter((a) => a.at < at).pop();
      const repo = (before ?? aliases.find((a) => a.at > at))?.repo;
      if (repo) add(`${repo}#${m[1]}`);
    }
  }
  return out;
}

const slugify = (s: string): string =>
  s
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 60) || "task";
const normTitle = (s: string): string =>
  s
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, " ")
    .trim();

/**
 * The synthetic in-flight entries for one orchestration section,
 * deduped against the board they will be merged into:
 * - a task whose GitHub ref (its own `ref`, or one its title carries)
 *   already appears in a Kaneo card's links is dropped — the Kaneo
 *   card is the record for that work. This is deliberately coarse:
 *   one shared ref drops the whole entry, even when its title names
 *   further refs no Kaneo card carries.
 * - a task whose title matches a Kaneo card's title is dropped.
 * - repeats inside the section itself (the same task lined up under
 *   two goals, or twice under one) emit once, keyed by first ref,
 *   else by normalized title.
 * Tasks with no ref anywhere still emit (title only, no GitHub link).
 */
export function inFlightBoardItems(section: OrchestrationSection, existing: BoardItem[]): BoardItem[] {
  const existingRefs = new Set(existing.flatMap((b) => b.links.map((l) => l.ref)));
  const existingTitles = new Set(existing.map((b) => normTitle(b.title)));
  const emitted = new Set<string>();
  const out: BoardItem[] = [];
  for (const goal of section.goals) {
    for (const task of goal.linedUp) {
      if (task.status !== "in-progress" && task.status !== "queued" && task.status !== "blocked") continue;
      const refs: RefKey[] = [];
      const add = (ref: RefKey) => {
        if (!refs.includes(ref)) refs.push(ref);
      };
      if (task.ref) add(task.ref);
      for (const ref of titleRefs(task.title)) add(ref);
      if (refs.some((r) => existingRefs.has(r))) continue;
      const titleKey = normTitle(task.title);
      if (existingTitles.has(titleKey)) continue;
      const key = refs[0] ?? titleKey;
      if (emitted.has(key) || emitted.has(titleKey)) continue;
      emitted.add(key);
      emitted.add(titleKey);
      out.push({
        id: `orch:${slugify(goal.goal)}/${slugify(task.title)}`,
        title: task.title,
        repo: refs.length ? parseRef(refs[0]).repo : "",
        source: "orchestrator",
        status: "in-flight",
        orchStatus: task.status,
        goal: goal.goal,
        blockedBy: task.blockedBy?.map((d) => `${d.task} (${d.status})`),
        // The lane header dates the lane by the section that produced
        // it; every synthetic entry carries that stamp.
        updatedAt: section.generatedAt,
        links: refs.map((ref) => ({ ref, relation: "tracks" as const })),
      });
    }
  }
  return out;
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
    usage: "fixtures",
    attention: "fixtures",
    orchestration: "fixtures",
    dns: "fixtures",
    registry: "fixtures",
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

  // Slice-2 coverage state: what the last attention read actually saw,
  // so the coverage note describes reality rather than config.
  let lastFeed: FleetFeed | null = null;
  let laptopSeen: boolean | null = null;
  async function getFeed(): Promise<FleetFeed | null> {
    const feed = await loadFeed(cfg.feedFile);
    if (feed) lastFeed = feed;
    return feed;
  }

  // The laptop's OpenChamber-managed OpenCode server: port + password
  // are read from its state file at request time (the port changes
  // across restarts). Mapped sessions are cached for 45 s (#890).
  let laptopCache: { value: MappedSessions; at: number } | null = null;
  async function laptopMapped(): Promise<MappedSessions | null> {
    if (laptopCache && Date.now() - laptopCache.at < 45_000) return laptopCache.value;
    const state = JSON.parse(await readFile(cfg.openchamberStateFile, "utf8")) as {
      port?: number;
      password?: string;
    };
    if (!state.port || !state.password) return null;
    const mapped = await mapServerSessions(
      {
        baseUrl: `http://127.0.0.1:${state.port}`,
        password: state.password,
      },
      Date.now(),
      { source: "opencode-laptop" },
    );
    if (!mapped) return null;
    laptopCache = { value: mapped, at: Date.now() };
    return mapped;
  }
  async function laptopSessions(): Promise<SessionFeedEntry[] | null> {
    const mapped = await laptopMapped();
    return mapped ? mapped.entries : null;
  }

  // The durable identity registry (#906 §3): records from the feed and
  // the laptop's live server are merged into a store file on every
  // read, so sessions outlive the feed's attention window. The store
  // is the aggregator's own data (like comments.json); a failed write
  // never breaks the read.
  let registryStore: SessionRecord[] | null = null;
  async function loadRegistryStore(): Promise<SessionRecord[]> {
    if (registryStore) return registryStore;
    try {
      const parsed: unknown = JSON.parse(await readFile(cfg.registryFile, "utf8"));
      registryStore = Array.isArray(parsed) ? parsed.filter(isSessionRecord) : [];
    } catch {
      registryStore = [];
    }
    return registryStore;
  }
  async function mergeIntoRegistry(incoming: SessionRecord[]): Promise<SessionRecord[]> {
    const current = await loadRegistryStore();
    const merged = mergeRecords(current, incoming);
    const changed = merged.length !== current.length || merged.some((r, i) => r !== current[i]);
    registryStore = merged;
    if (changed) {
      try {
        await mkdir(dirname(cfg.registryFile), { recursive: true });
        const tmp = `${cfg.registryFile}.tmp-${process.pid}`;
        await writeFile(tmp, JSON.stringify(merged, null, 2));
        await rename(tmp, cfg.registryFile);
      } catch {
        /* durability is best-effort; the in-memory merge still serves */
      }
    }
    return merged;
  }

  return {
    async board() {
      try {
        const cookie = await kaneoCookie();
        const board = await kaneoBoard(cfg, cookie);
        if (board.length > 0) {
          provenance.board = "live (Kaneo)";
          let merged = board;
          if (cfg.projector.baseUrl) {
            merged = await enrichWithRegistryLinks(cfg.projector.baseUrl, board, (b) => b.id);
            provenance.board = "live (Kaneo + projector links)";
          }
          // In-flight lane (see inFlightBoardItems above): synthetic
          // entries from the fleet feed's orchestration section — the
          // same feed read this source already uses for quota and
          // orchestration — merged ahead of the Kaneo cards. The lane
          // fails soft: no feed / no section = the Kaneo board alone.
          try {
            const feed = await getFeed();
            const orchSection = feed?.orchestration ? normalizeOrchestrationSection(feed.orchestration) : null;
            if (orchSection) {
              const inFlight = inFlightBoardItems(orchSection, merged);
              if (inFlight.length) {
                provenance.board += ` + orchestrator in-flight (${inFlight.length} lined-up, section ${orchSection.generatedAt})`;
                merged = [...inFlight, ...merged];
              }
            }
          } catch {
            /* the Kaneo board still renders without the lane */
          }
          return merged;
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
      // #910 stage 5: the fleet feed's GitHub Project pool is the
      // resolution source FIRST — the projector is dead by config and
      // the Projects-scope token cannot read arbitrary issues, only
      // the Project (whose items carry their linked issue's state).
      const out = new Map<RefKey, GhItem>();
      const feed = await getFeed();
      const pool = (feed?.projects ?? []).filter(isProjectPoolItem);
      if (pool.length && refs.length) {
        const byRef = new Map(pool.map((p) => [p.ref, p] as const));
        for (const ref of refs) {
          const p = byRef.get(ref);
          if (p) out.set(ref, poolItemToGhItem(p));
        }
      }
      const remaining = refs.filter((r) => !out.has(r));
      let projectorServed = false;
      if (remaining.length) {
        let rest: Map<RefKey, GhItem> | null = null;
        if (cfg.projector.baseUrl) {
          rest = await projectorItems(cfg.projector.baseUrl, remaining).catch(() => null);
          if (rest && rest.size > 0) projectorServed = true;
        }
        if (!rest || rest.size === 0) rest = await fix.items(remaining);
        for (const [k, v] of rest) out.set(k, v);
      }
      const poolServed = refs.length - remaining.length;
      if (poolServed > 0 && feed) {
        provenance.items =
          `live (project pool · fleet feed · ${feedAgeText(feed.generatedAt)})` +
          (remaining.length
            ? ` + ${projectorServed ? "projector" : "fixtures"} for ${remaining.length} ref(s) outside the pool`
            : "");
      } else if (projectorServed) {
        provenance.items = "live (projector)";
      }
      return out;
    },
    async projects(): Promise<ProjectPoolItem[]> {
      const feed = await getFeed();
      return (feed?.projects ?? []).filter(isProjectPoolItem);
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
      const feed = await getFeed();
      if (feed) {
        provenance.quota = `live (fleet feed · ${feedAgeText(feed.generatedAt)} · collected ${feed.generatedAt})`;
        return feed.quota;
      }
      provenance.quota = "fixtures (no fleet feed file yet)";
      return fix.quota();
    },
    async usage(): Promise<UsageView> {
      // Usage & billing hub (design §3): feed sections first (the
      // collector extracts Part (b) on the sandbox, where the sources
      // of record live); when an older feed has none, extract directly
      // from the quota view + registry files; fixtures only when
      // neither exists. Snapshot history comes from the local store
      // (§4): the feed carries the writers' records (collector-derived
      // plus the stewardship / check-usage outbox), and ingesting them
      // here — append with dedupe — is the one write this reader makes.
      const feed = await getFeed();
      const store = new SnapshotStore(cfg.snapshotsDir);
      if (feed?.snapshots?.length) {
        await store.append(feed.snapshots).catch(() => 0);
      }
      const history = await store.byService().catch(() => ({}));
      const domains =
        feed?.dns && isDnsSection(feed.dns)
          ? {
              totalAnnualUsd: feed.dns.totalAnnualListUsd,
              nextRenewal: summarizeDns(feed.dns).nextRenewal?.renewal ?? null,
              domainCount: feed.dns.entries.length,
            }
          : null;
      if (feed?.saas && feed.saas.length) {
        provenance.usage = `live (fleet feed · ${feedAgeText(feed.generatedAt)} · collected ${feed.generatedAt})`;
        const partial = {
          services: feed.saas,
          payg: feed.payg ?? [],
          charges: feed.charges ?? [],
          subscriptions: feed.subscriptions ?? [],
        };
        return {
          ...partial,
          chargesNote: partial.charges.length
            ? undefined
            : "No charge evidence in the feed's records yet — rows appear as Plaid/Gmail receipt evidence is recorded (§2.4).",
          attention: buildAttentionFlags(feed.quota, partial),
          domains,
          history,
        };
      }
      try {
        const [doc, registryText] = await Promise.all([
          readFile(cfg.quotaViewFile, "utf8"),
          readFile(cfg.quotaRegistryFile, "utf8").catch(() => ""),
        ]);
        const partial = {
          services: extractSaasServices(doc),
          payg: extractPaygLines(doc, registryText),
          charges: extractCharges(doc, registryText),
          subscriptions: extractSubscriptions(doc),
        };
        provenance.usage = "live (quota-view.md + provider registry, direct read — feed carries no hub sections yet)";
        return {
          ...partial,
          chargesNote: partial.charges.length
            ? undefined
            : "No charge evidence in the local records yet — rows appear as Plaid/Gmail receipt evidence is recorded (§2.4).",
          attention: buildAttentionFlags(feed?.quota ?? (await fix.quota()), partial),
          domains,
          history,
        };
      } catch {
        provenance.usage = "fixtures (no fleet feed hub sections and quota view unreadable)";
        const fallback = await fix.usage();
        return { ...fallback, history: Object.keys(history).length ? history : fallback.history };
      }
    },
    async attention() {
      // Slice 1 (#890): laptop sessions live from the OpenChamber-
      // managed OpenCode server (45 s cache); sandbox sessions from
      // the fleet feed (the sandbox server binds its own loopback).
      const parts: string[] = [];
      const byId = new Map<string, SessionFeedEntry>();

      const feed = await getFeed();
      if (feed) {
        for (const e of feed.attention) byId.set(e.sessionId, e);
        parts.push(`sandbox via fleet feed (${feedAgeText(feed.generatedAt)})`);
      }

      try {
        const laptop = await laptopSessions();
        if (laptop) {
          laptopSeen = true;
          for (const e of laptop) byId.set(e.sessionId, e);
          parts.push("laptop live");
        } else {
          laptopSeen = false;
        }
      } catch {
        laptopSeen = false;
        /* laptop part fails soft */
      }

      if (byId.size > 0) {
        provenance.attention = `live (${parts.join(" + ")})`;
        return [...byId.values()];
      }
      provenance.attention = "fixtures (no live attention source reachable)";
      return fix.attention();
    },
    async orchestration(): Promise<OrchestrationSection | null> {
      // #899: the orchestration section rides the fleet feed, embedded
      // by the collector from the orchestrator's own output file. The
      // section carries its own provenance; absent = not measured yet.
      // #1140: three states, told apart — live (accepted, served in
      // the normalized contract shape, #1110), invalid (a section is
      // present but failed validation: name the failed check, never
      // claim the job "has not published"), absent (no section key).
      const feed = await getFeed();
      if (feed?.orchestration) {
        const section = normalizeOrchestrationSection(feed.orchestration);
        if (section) {
          provenance.orchestration = `live (orchestration section ${section.runId} · ${feedAgeText(section.generatedAt)})`;
          return section;
        }
        const problem = orchestrationSectionProblem(feed.orchestration);
        console.error(`aggregator: orchestration section present but invalid (${problem}); the today view derives`);
        provenance.orchestration = `invalid — orchestration section present but failed validation (${problem}); the today view derives`;
        return null;
      }
      provenance.orchestration = feed
        ? "none yet — the orchestration job (#899) has not published a section; the today view derives"
        : "fixtures (no fleet feed file yet)";
      return null;
    },
    async dns(): Promise<DnsSection | null> {
      // Slice 3 (#906 §1): the register rides the fleet feed, parsed
      // from #902's register file by the collector. The page renders
      // the register; it never edits it.
      const feed = await getFeed();
      if (feed?.dns && isDnsSection(feed.dns)) {
        provenance.dns = `live (fleet feed · ${feedAgeText(feed.generatedAt)} · register parsed ${feed.dns.generatedAt})`;
        return feed.dns;
      }
      provenance.dns = feed
        ? "not collected yet — the collector has not embedded a register section"
        : "fixtures (no fleet feed file yet)";
      return null;
    },
    async registry(): Promise<SessionRecord[]> {
      // Slice 3 (#906 §3): feed records (sandbox OpenCode + Jules +
      // Muse launches) merged with the laptop server's live
      // sessions, accumulated into the durable store.
      const incoming: SessionRecord[] = [];
      const feed = await getFeed();
      if (feed?.registry) {
        for (const r of feed.registry) if (isSessionRecord(r)) incoming.push(r);
      }
      try {
        const mapped = await laptopMapped();
        if (mapped) {
          for (const e of mapped.entries) {
            incoming.push(recordFromSessionEntry(e, mapped.tokensBySession.get(e.sessionId)));
          }
        }
      } catch {
        /* laptop part fails soft, like attention */
      }
      if (incoming.length > 0) {
        provenance.registry = `live (${incoming.length} records merged · feed ${feed ? feedAgeText(feed.generatedAt) : "absent"})`;
        return mergeIntoRegistry(incoming);
      }
      provenance.registry = "store only (no live registry source reachable)";
      return loadRegistryStore();
    },
    async coverage(): Promise<SourceCoverage[]> {
      const feed = lastFeed ?? (await getFeed());
      const out: SourceCoverage[] = [];
      out.push({
        source: "opencode-sandbox",
        label: "OpenCode · sandbox server",
        wired: Boolean(feed),
        detail: feed
          ? `via fleet feed (${feedAgeText(feed.generatedAt)}, collected ${feed.generatedAt})`
          : "no fleet feed file — sandbox sessions are not visible",
      });
      out.push({
        source: "opencode-laptop",
        label: "OpenCode · laptop server",
        wired: laptopSeen === true,
        detail:
          laptopSeen === true
            ? "live from the OpenChamber-managed server (45 s cache)"
            : laptopSeen === false
              ? "unreachable on the last read — laptop sessions may be missing"
              : "not read yet this process",
      });
      const j = feed?.julesSummary;
      out.push({
        source: "jules",
        label: "Jules",
        wired: Boolean(j),
        detail: j
          ? `via the keeper's state (jules-review) · ${j.tracked} tracked now · last keeper run ${j.at ?? "unknown"}${
              j.total != null
                ? ` · last run totals: ${j.total} sessions — ${j.completed ?? 0} completed, ${j.failed ?? 0} failed, ${j.inProgress ?? 0} in progress, ${j.awaitingFeedback ?? 0} awaiting feedback (completed runs are pruned by the keeper)`
                : ""
            }`
          : "keeper state not in the feed — Jules sessions are not visible",
      });
      const claudeRecords = (feed?.registry ?? []).filter((r) => r.source === "claude-code");
      out.push({
        source: "claude-code",
        label: "Muse",
        wired: claudeRecords.length > 0,
        detail:
          claudeRecords.length > 0
            ? `via launch records in the fleet feed (${claudeRecords.length} recorded) — a Muse session is visible only when its launch appended a record (scripts/record-claude-launch.ts); the runtime exposes no session list to scrape`
            : "no launch records in the feed yet — Muse sessions register by appending a launch record (scripts/record-claude-launch.ts, #906 §3); the runtime exposes no session list to scrape",
      });
      return out;
    },
    provenance: () => provenance,
  };
}

export { refUrl };
