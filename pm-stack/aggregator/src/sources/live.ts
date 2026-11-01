// The live reader: Kaneo's per-repo boards and Grist's roadmap table read
// directly (the projector does not project them yet — J, 2026-10-25), and
// the projector's /items and /comments for live ref state and the comment
// projection. Anything a reader cannot reach falls back to fixtures, and the
// provenance strings say exactly which facet is live vs fixture-backed.
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { dirname } from "node:path";
import type { Config } from "../config.ts";
import { applyBoardDonePolicy, isRetiredDoneTableTitle } from "../board-policy.ts";
import { isAaSection } from "../aa.ts";
import { isDnsSection } from "../dns.ts";
import {
  diagnoseOrchestrationSection,
  feedAgeText,
  normalizeOrchestrationSection,
  orchestrationSectionProblem,
  loadFeed,
  coverageFromFeed,
  poolAvailableFromFeed,
  poolFromFeed,
  type FleetFeed,
  type SectionDiagnosis,
} from "../feed.ts";
import type { RepoCensus } from "../coverage.ts";
import { isSessionRecord, mergeRecords, recordFromSessionEntry } from "../registry.ts";
import { buildAttentionFlags } from "../derive.ts";
import { summarizeDns } from "../dns.ts";
import {
  applyCloudflareExport,
  applySupabaseExport,
  extractCharges,
  extractPaygLines,
  extractSaasServices,
  extractSubscriptions,
  extractUninstrumentedServices,
} from "../saas-extract.ts";
import { mergeChargeRows } from "../charges-collect.ts";
import type { CloudflareExportDoc, SupabaseExportDoc } from "../saas-extract.ts";
import { poolItemToGhItem } from "../projects.ts";
import { INITIAL_STANDING_AUTO, sanitizeTierItemExtensions, withUserFallbacks } from "../tier-items.ts";
import { overlayQuotaCurrent, overlayTursoUsage, parseQuotaCurrent } from "../quota-current.ts";
import { SnapshotStore } from "../snapshots.ts";
import { gristDocUrl, kaneoTaskUrl } from "../service-links.ts";
import { fixtureSource } from "./fixtures.ts";
import { mapServerSessions, type MappedSessions } from "./opencode.ts";
import {
  type AaFeedSection,
  type AttentionFlag,
  type BoardItem,
  type ChargeRow,
  type Comment,
  type DnsSection,
  type GhItem,
  type ItemResolution,
  type Link,
  type OrchestrationSection,
  type ProjectPoolItem,
  type Provenance,
  type ProvenanceDetail,
  type ProvenanceFacet,
  type QuotaView,
  type RefKey,
  type RoadmapRow,
  type SaasService,
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
  updatedAt?: string;
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
): Promise<{ id: string; slug: string; name: string; repo: string; workspaceId: string }[]> {
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
    workspaceId,
  }));
}

async function kaneoBoard(cfg: Config, cookie: string): Promise<BoardItem[]> {
  const projects = await kaneoProjects(cfg, cookie);
  const out: BoardItem[] = [];
  // A task read that fails on every project is a failed fetch, not a
  // successfully empty board (Codex review, PR #2430): board() must
  // reach its catch path and serve fixtures with that provenance,
  // while a successful read the done policy filters to empty stays
  // live. Partial data (at least one page read) is still served.
  let anyTaskPageRead = false;
  for (const p of projects) {
    const all: KaneoColumn[] = [];
    for (let page = 1; ; page++) {
      const r = await getJson(`${cfg.kaneo.baseUrl}/api/task/tasks/${p.id}?page=${page}`, { Cookie: cookie });
      if (!r.ok) break;
      anyTaskPageRead = true;
      const body = r.json as { data?: { columns?: KaneoColumn[] }; pagination?: { totalPages: number } };
      const cols = body.data?.columns ?? [];
      all.push(...cols);
      const totalPages = body.pagination?.totalPages ?? 1;
      if (page >= totalPages || totalPages <= 1) break;
    }
    // The curated in-flight set, not every open issue: in-progress,
    // in-review, and done (recent) columns only. To-do is the backlog.
    // "Done (recent)" is enforced, not just commented (#1146): done
    // cards older than 7 days drop off below via applyBoardDonePolicy,
    // and the retired ROADMAP Done-table bookkeeping class is filtered
    // here at the aggregator (the board provenance states the filter).
    const wipStatuses = new Set(["in-progress", "in-review", "done"]);
    for (const col of all) {
      if (!wipStatuses.has(col.id)) continue;
      for (const t of col.tasks) {
        if (isRetiredDoneTableTitle(t.title)) continue;
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
          // #1197: never present creation time as update time. When
          // Kaneo omits updatedAt the item stays undated: downstream
          // sorts treat missing as oldest and the done policy keeps
          // undated cards (age unproven), instead of freshness being
          // fabricated from createdAt.
          updatedAt: t.updatedAt,
          kaneoUrl: kaneoTaskUrl(p.workspaceId, p.id, t.id),
          links,
        });
      }
    }
  }
  if (projects.length > 0 && !anyTaskPageRead) throw new Error("kaneo task reads failed for every project");
  return applyBoardDonePolicy(out);
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
      gristUrl: gristDocUrl(cfg.grist.docId),
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

/**
 * Refs travel in the query string, and the projector (Bun.serve) rejects
 * request lines past ~16KB with HTTP 431 — at ~30 encoded chars per ref
 * that ceiling is only ~500 refs, which the board prime outgrew on
 * 2026-10-27: the prime's comments/items reads failed over to fixtures
 * while single-item pages (a handful of refs) stayed live. Chunk every
 * multi-ref read so each request stays far below the limit.
 */
const PROJECTOR_REFS_PER_REQUEST = 200;

function refChunks(refs: RefKey[]): RefKey[][] {
  const chunks: RefKey[][] = [];
  for (let i = 0; i < refs.length; i += PROJECTOR_REFS_PER_REQUEST) {
    chunks.push(refs.slice(i, i + PROJECTOR_REFS_PER_REQUEST));
  }
  return chunks;
}

export async function projectorItems(baseUrl: string, refs: RefKey[]): Promise<Map<RefKey, GhItem> | null> {
  if (refs.length === 0) return new Map();
  // Refs contain `#` — they must be percent-encoded or the `#` starts a
  // fragment and the projector never sees the number.
  const out = new Map<RefKey, GhItem>();
  for (const chunk of refChunks(refs)) {
    const r = await getJson(`${baseUrl}/items?refs=${encodeURIComponent(chunk.join(","))}`);
    if (!r.ok) return null;
    const rows = (r.json as { items?: ProjectorItemRow[] }).items ?? [];
    for (const row of rows) out.set(row.ref, mapProjectorItem(row));
  }
  return out;
}

export async function projectorComments(baseUrl: string, refs: RefKey[]): Promise<Comment[] | null> {
  if (refs.length === 0) return [];
  const out: Comment[] = [];
  for (const chunk of refChunks(refs)) {
    const r = await getJson(`${baseUrl}/comments?refs=${encodeURIComponent(chunk.join(","))}`);
    if (!r.ok) return null;
    out.push(
      ...((r.json as { comments?: ProjectorCommentRow[] }).comments ?? [])
        .map(mapProjectorComment)
        .filter((c) => !c.deleted),
    );
  }
  return out;
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

/**
 * The fleet's own staleness line for the feed itself (#1148's
 * watchdog threshold): a feed-backed facet older than this renders
 * its freshness chip as stale, however live its last read was.
 */
const FEED_STALE_MS = 15 * 60_000;

/**
 * The gatherer's Turso entry (#1049) overlaid onto usage() services,
 * fail-soft like quota(): a missing or malformed quota-current file
 * leaves the base services untouched.
 */
async function tursoOverlay(cfg: Config, services: SaasService[]): Promise<SaasService[]> {
  try {
    const raw = await readFile(cfg.quotaCurrentFile, "utf8");
    const file = parseQuotaCurrent(JSON.parse(raw));
    if (file) return overlayTursoUsage(services, file);
  } catch {
    /* no gatherer file (or unreadable): base services stand */
  }
  return services;
}

export function liveSource(cfg: Config, opts?: { standingAuto?: () => Promise<boolean> }): Source {
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
    aa: "fixtures",
  };

  // #1139/#1140: the last orchestration diagnosis, recomputed on every
  // orchestration() read from the raw feed value — the honesty
  // surfaces (Source.diagnostics) read it from here, never from a
  // null section, so absent/invalid/stale stay distinguishable.
  let lastOrchestrationDiagnosis: SectionDiagnosis = { state: "absent" };
  // #1138: the merged registry total from the last registry() read —
  // the freshness chip counts THIS (the same number /sessions renders
  // in its header), never the per-read incoming batch.
  let lastRegistryMergedCount: number | null = null;

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
  let registryWriteChain: Promise<void> = Promise.resolve();
  let registryWriteSeq = 0;
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
      // Serialized writes + unique temp path per write (#1509): the
      // pid-only temp path raced concurrent merges the same way the
      // comments/actions stores did, and the catch below hid the loss.
      const write = registryWriteChain.then(async () => {
        await mkdir(dirname(cfg.registryFile), { recursive: true });
        const tmp = `${cfg.registryFile}.tmp-${process.pid}-${++registryWriteSeq}`;
        await writeFile(tmp, JSON.stringify(merged, null, 2));
        await rename(tmp, cfg.registryFile);
      });
      registryWriteChain = write.catch(() => {});
      try {
        await write;
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
        // A successfully fetched board is live data even when the
        // done policy (#1146) filters it to empty: falling through to
        // fixtures would replace the real (empty) board with
        // illustrative work and hide the orchestrator in-flight lane
        // merged below. Fixtures are only for a failed fetch (catch).
        provenance.board = "live (Kaneo; done aged out after 7 days, retired ROADMAP Done-table cards filtered)";
        let merged = board;
        if (cfg.projector.baseUrl) {
          merged = await enrichWithRegistryLinks(cfg.projector.baseUrl, board, (b) => b.id);
          provenance.board =
            "live (Kaneo + projector links; done aged out after 7 days, retired ROADMAP Done-table cards filtered)";
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
      // #2399: the pool enters through the single feed gate (fail-soft,
      // edge-aware), shared with projects() below.
      const pool = poolFromFeed(feed);
      if (pool.length && refs.length) {
        const byRef = new Map(pool.map((p) => [p.ref, p] as const));
        for (const ref of refs) {
          const p = byRef.get(ref);
          if (p) out.set(ref, { ...poolItemToGhItem(p), resolution: "pool" });
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
        // #1141: every served item carries its resolution source, so
        // fixture state can render as unverified and health can publish
        // the live-vs-fixture coverage ratio from the same pass.
        let restSource: ItemResolution = "projector";
        if (!rest || rest.size === 0) {
          rest = await fix.items(remaining);
          restSource = "fixture";
        }
        for (const [k, v] of rest) out.set(k, { ...v, resolution: restSource });
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
      } else if (pool.length > 0 && feed) {
        // No refs were resolved this call (e.g. the /api/health
        // provenance prime, #1267): the facet is pool-backed all the
        // same — say so rather than leaving the fixture default
        // standing for a facet nobody happened to exercise yet.
        provenance.items = `live (project pool · fleet feed · ${feedAgeText(feed.generatedAt)})`;
      }
      return out;
    },
    async projects(): Promise<ProjectPoolItem[]> {
      const feed = await getFeed();
      return poolFromFeed(feed);
    },
    async projectsAvailable(): Promise<boolean> {
      const feed = await getFeed();
      return poolAvailableFromFeed(feed);
    },
    async coverageCensus(): Promise<RepoCensus[] | null> {
      const feed = await getFeed();
      return coverageFromFeed(feed);
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
      // Stage 2 (#1151): the quota gatherer's quota-current.json —
      // written into this aggregator's own data dir on a 15-minute
      // tick — overlays the dated-read pools: fresh reads win, error /
      // stale reads surface as the pool's state. Fail-soft: a missing
      // or malformed file leaves the base view untouched.
      const feed = await getFeed();
      let view: QuotaView;
      if (feed) {
        provenance.quota = `live (fleet feed · ${feedAgeText(feed.generatedAt)} · collected ${feed.generatedAt})`;
        view = feed.quota;
        const dropped = feed.quotaDropped;
        if (dropped && dropped.pools + dropped.windows + dropped.burn > 0) {
          provenance.quota += ` · sanitized at load: dropped ${dropped.pools} pool(s), ${dropped.windows} window(s), ${dropped.burn} burn segment(s) (#2429)`;
        }
      } else {
        provenance.quota = "fixtures (no fleet feed file yet)";
        view = await fix.quota();
      }
      try {
        const raw = await readFile(cfg.quotaCurrentFile, "utf8");
        const file = parseQuotaCurrent(JSON.parse(raw));
        if (file) {
          provenance.quota += ` · gatherer ${file.generatedAt}`;
          return overlayQuotaCurrent(view, file);
        }
      } catch {
        /* no gatherer file (or unreadable): base view stands */
      }
      return view;
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
      const uninstrumentedServices =
        feed?.uninstrumentedServices ??
        (await readFile(cfg.freeServicesTrackerFile, "utf8")
          .then(extractUninstrumentedServices)
          .catch(() => []));
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
      // The Supabase export overlay applies to BOTH service sources:
      // the collector's feed.saas cards are built with plain
      // extractSaasServices, so without this the fresh export would be
      // ignored exactly when the fleet collector is operating (#1050).
      const supabaseExport = await readFile(cfg.supabaseExportFile, "utf8")
        .then((text) => JSON.parse(text) as SupabaseExportDoc)
        .catch(() => null);
      // Charges evidence + unmatched-charge flags (#3124): computed
      // once, before the branch. The feed's charge data is not a
      // byproduct of the saas section, so every return path below
      // consumes it — previously both lived only inside the saas
      // gate, and a feed with charges but no saas (or a quota-view
      // fallback) silently dropped every auto-matched charge.
      const feedCharges: ChargeRow[] = feed?.charges ?? [];
      // Charges auto-matching (#1057): the feed carries only
      // first-time unmatched SaaS/AI charges (the collector's
      // surfaced-state guarantees exactly-once), so each becomes
      // an attention flag here and never repeats on later feeds.
      // Charges sanitization loss (#2445): rows loadFeed dropped are
      // counted on the feed; surface that count in the usage
      // provenance on every return path below, never swallow it.
      const chargesDroppedNote = feed?.chargesDropped
        ? ` · sanitized at load: dropped ${feed.chargesDropped} malformed charge row(s) (#2445)`
        : "";
      const trackerRowsDroppedNote = feed?.uninstrumentedServicesDropped
        ? ` · sanitized at load: dropped ${feed.uninstrumentedServicesDropped} malformed tracker row(s) (#1172)`
        : "";
      const usageSanitizationNote = chargesDroppedNote + trackerRowsDroppedNote;
      const feedChargeFlags: AttentionFlag[] = (feed?.chargesAttention ?? []).map((u) => ({
        kind: "unmatched-charge",
        label: `${u.merchant} — unrecognized SaaS/AI charge`,
        detail: `$${u.amountUsd.toFixed(2)} on ${u.at} via ${u.source === "plaid" ? "Plaid" : "Gmail alert"} — no hub service matched; classify it or add the service`,
        href: "/quota",
        severity: u.amountUsd,
      }));
      if (feed?.saas && feed.saas.length) {
        provenance.usage =
          `live (fleet feed · ${feedAgeText(feed.generatedAt)} · collected ${feed.generatedAt})` + usageSanitizationNote;
        // The Cloudflare live export overlays tracker prose on BOTH
        // service sources — a healthy fleet feed must not keep stale
        // prose values when a fresh export exists (Codex, PR #2228).
        const cfExport = await readFile(cfg.cloudflareExportFile, "utf8")
          .then((text) => JSON.parse(text) as CloudflareExportDoc)
          .catch(() => null);
        const partial = {
          services: applySupabaseExport(applyCloudflareExport(feed.saas, cfExport), supabaseExport),
          uninstrumentedServices,
          payg: feed.payg ?? [],
          charges: feedCharges,
          subscriptions: feed.subscriptions ?? [],
        };
        partial.services = await tursoOverlay(cfg, partial.services);
        return {
          ...partial,
          chargesNote: partial.charges.length
            ? undefined
            : "No charge evidence in the feed's records yet — rows appear as Plaid/Gmail receipt evidence is recorded (§2.4).",
          attention: [...buildAttentionFlags(feed.quota, partial), ...feedChargeFlags],
          domains,
          history,
        };
      }
      try {
        const [doc, registryText] = await Promise.all([
          readFile(cfg.quotaViewFile, "utf8"),
          readFile(cfg.quotaRegistryFile, "utf8").catch(() => ""),
        ]);
        const cfExport = await readFile(cfg.cloudflareExportFile, "utf8")
          .then((text) => JSON.parse(text) as CloudflareExportDoc)
          .catch(() => null);
        // #3124: the feed's charges win/merge here too — prose
        // extraction is the fallback for when the feed carries none,
        // never a reason to drop the ones it does carry. The merge is
        // charge-level (mergeChargeRows): the same charge recorded in
        // both places differs in label/date/source strings, so
        // full-row equality would display it twice; automatic
        // evidence (the feed rows) takes precedence.
        const partial = {
          services: applySupabaseExport(applyCloudflareExport(extractSaasServices(doc), cfExport), supabaseExport),
          uninstrumentedServices,
          payg: extractPaygLines(doc, registryText),
          charges: mergeChargeRows(feedCharges, extractCharges(doc, registryText)),
          subscriptions: extractSubscriptions(doc),
        };
        partial.services = await tursoOverlay(cfg, partial.services);
        provenance.usage =
          "live (quota-view.md + provider registry, direct read — feed carries no hub sections yet)" +
          usageSanitizationNote;
        return {
          ...partial,
          chargesNote: partial.charges.length
            ? undefined
            : "No charge evidence in the local records yet — rows appear as Plaid/Gmail receipt evidence is recorded (§2.4).",
          attention: [...buildAttentionFlags(feed?.quota ?? (await fix.quota()), partial), ...feedChargeFlags],
          domains,
          history,
        };
      } catch {
        provenance.usage = "fixtures (no fleet feed hub sections and quota view unreadable)" + usageSanitizationNote;
        const fallback = await fix.usage();
        // #3124: even the fixtures fallback keeps the feed's charge
        // evidence when a feed exists — fixtures stand in for the
        // sections that are missing, not for the ones that arrived.
        return {
          ...fallback,
          uninstrumentedServices,
          charges: feedCharges.length ? feedCharges : fallback.charges,
          chargesNote: feedCharges.length ? undefined : fallback.chargesNote,
          attention: [...fallback.attention, ...feedChargeFlags],
          history: Object.keys(history).length ? history : fallback.history,
        };
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
      // #1139 adds the fourth — stale — tracked as a diagnosis on the
      // source so the attention/today surfaces can banner it.
      const feed = await getFeed();
      lastOrchestrationDiagnosis = diagnoseOrchestrationSection(feed?.orchestration ?? null);
      if (feed?.orchestration) {
        const section = normalizeOrchestrationSection(feed.orchestration);
        if (section) {
          provenance.orchestration = `live (orchestration section ${section.runId} · ${feedAgeText(section.generatedAt)})`;
          // #1290: the per-item extension seam. The section gate above
          // never learns the extension fields; each tier item's
          // extensions are validated here, fail-soft (a malformed
          // extension drops, never the item or the section), and
          // needs-user items gain their contract fallbacks (whyUser,
          // ifIgnored, the standing-toggle delegation default).
          const standingAuto = opts?.standingAuto
            ? await opts.standingAuto().catch(() => INITIAL_STANDING_AUTO)
            : INITIAL_STANDING_AUTO;
          section.tiers = {
            needsDrew: section.tiers.needsDrew.map((item) =>
              withUserFallbacks(sanitizeTierItemExtensions(item), section.generatedAt, standingAuto),
            ),
            needsOrchestrator: section.tiers.needsOrchestrator.map((item) => sanitizeTierItemExtensions(item)),
          };
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
    async aa(): Promise<AaFeedSection | null> {
      // AA cost explorer: the section rides the fleet feed, read from
      // Supabase ainventory by the collector. Its absent form is a real
      // section (the page renders the reason); a missing section means
      // the collector has not embedded one yet.
      const feed = await getFeed();
      if (feed?.aa && isAaSection(feed.aa)) {
        provenance.aa =
          feed.aa.status === "ok"
            ? `live (fleet feed · ${feedAgeText(feed.generatedAt)} · ainventory batch ${feed.aa.batch.slice(0, 8)} · collected ${feed.aa.generatedAt})`
            : `live (fleet feed · ${feedAgeText(feed.generatedAt)} · AA section absent: ${feed.aa.reason})`;
        return feed.aa;
      }
      provenance.aa = feed
        ? "not collected yet — the collector has not embedded an AA section"
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
        const merged = await mergeIntoRegistry(incoming);
        lastRegistryMergedCount = merged.length;
        // #1138: the headline number is the MERGED total — the same
        // count the /sessions header renders — with the latest feed's
        // contribution labelled as what it is. (This string used to
        // headline the incoming batch, so one page carried two counts
        // for "records" with nothing explaining the difference.)
        provenance.registry = `live (${merged.length} session records · ${incoming.length} from the latest feed · feed ${feed ? feedAgeText(feed.generatedAt) : "absent"})`;
        return merged;
      }
      const store = await loadRegistryStore();
      lastRegistryMergedCount = store.length;
      provenance.registry = "store only (no live registry source reachable)";
      return store;
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
    diagnostics: () => ({ orchestration: lastOrchestrationDiagnosis }),
    provenanceDetail: (): ProvenanceDetail => {
      // #1138: facet state derives from the facet's own string (the
      // strings are the contract every facet already maintains),
      // plus two facts strings can't carry: the orchestration
      // diagnosis (stale/invalid precision) and the feed's age (a
      // "live" string over a dead feed is stale, #1148's threshold).
      const feedAgeMs = lastFeed ? Math.max(0, Date.now() - Date.parse(lastFeed.generatedAt)) : undefined;
      const facet = (key: keyof Provenance): ProvenanceFacet => {
        const text = provenance[key];
        let state: ProvenanceFacet["state"];
        if (text.startsWith("live")) state = "live";
        else if (text.startsWith("invalid")) state = "invalid";
        else if (text.startsWith("none yet") || text.startsWith("not collected yet")) state = "absent";
        else if (text.startsWith("store only")) state = "live";
        else state = "fixtures";
        const feedBacked = text.includes("fleet feed");
        if (state === "live" && feedBacked && feedAgeMs !== undefined && feedAgeMs > FEED_STALE_MS) {
          state = "stale";
        }
        const out: ProvenanceFacet = { text, state };
        if (key === "orchestration") {
          if (lastOrchestrationDiagnosis.state === "stale" && state === "live") out.state = "stale";
          if (lastOrchestrationDiagnosis.state === "invalid") out.state = "invalid";
          if (lastOrchestrationDiagnosis.ageMs !== undefined) out.ageMs = lastOrchestrationDiagnosis.ageMs;
        } else if (feedBacked && feedAgeMs !== undefined) {
          out.ageMs = feedAgeMs;
        }
        if (key === "registry") {
          // The registry facet's age is the age of the feed its records
          // merge from, so it derives from the loaded feed alone —
          // gating it on lastRegistryMergedCount made the age (and the
          // #1148 watchdog's registry item, which reads it) depend on
          // some earlier request having called registry() (Codex P2,
          // PR #2431). The COUNT still requires a real merge.
          if (lastRegistryMergedCount !== null) {
            out.count = lastRegistryMergedCount;
            out.countLabel = "session records";
          }
          if (feedAgeMs !== undefined) {
            out.ageMs = feedAgeMs;
            if (out.state === "live" && feedAgeMs > FEED_STALE_MS) out.state = "stale";
          }
        }
        return out;
      };
      return {
        board: facet("board"),
        roadmap: facet("roadmap"),
        items: facet("items"),
        comments: facet("comments"),
        quota: facet("quota"),
        usage: facet("usage"),
        attention: facet("attention"),
        orchestration: facet("orchestration"),
        dns: facet("dns"),
        registry: facet("registry"),
        aa: facet("aa"),
      };
    },
  };
}

export { refUrl };
