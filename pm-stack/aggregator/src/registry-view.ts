// Sessions registry view (#1209): recency window, search/filter, and
// pagination for /sessions and its JSON twin /api/registry.json.
//
// The registry store grows forever and is never pruned by this layer
// (retention is a separate decision); this module bounds what the
// views render. Both surfaces run the exact same query pipeline —
// parseRegistryQuery + queryRegistry — so the HTML page and the JSON
// twin cannot disagree about which records a given URL selects.
//
// Recency is by last activity: SessionRecord.lastSeenAt, falling back
// to startedAt. A record with no parseable activity timestamp is
// treated as recent (visible): undated records must never be hidden
// silently by a window they cannot be placed in.

import type { SessionRecord } from "./model.ts";

export interface RegistryQuery {
  /** Window length in days, or null for all history. Default 7. */
  windowDays: number | null;
  /** Free-text search across id/label/title/agent/model/dir/refs. */
  q: string | null;
  /** Agent filter: case-insensitive substring of agentId/agentLabel. */
  agent: string | null;
  /** Task-ref filter: case-insensitive substring of any taskRef. */
  task: string | null;
  /** Page size (default 100, clamped 1..500). */
  limit: number;
  /** Page offset (default 0). */
  offset: number;
  /** Deep-link pin: this record id is always in the returned page. */
  id: string | null;
}

export interface RegistryView {
  query: RegistryQuery;
  /** The page slice to render (includes the pinned id record, if any). */
  records: SessionRecord[];
  /** Records in the whole store, before any filtering. */
  totalAll: number;
  /** Records hidden by the recency window alone. */
  hiddenByWindow: number;
  /** Records matching window + filters, before pagination. */
  matched: number;
  /** matched - records shown for reasons other than the pin. */
  hiddenByPagination: number;
  /** True when the pinned id record was pulled into the page. */
  pinnedIncluded: boolean;
}

export const REGISTRY_DEFAULT_WINDOW_DAYS = 7;
export const REGISTRY_DEFAULT_LIMIT = 100;
export const REGISTRY_MAX_LIMIT = 500;

const DAY_MS = 24 * 60 * 60 * 1000;

/** The record's last-activity instant (ms), or null when undated. */
export function registryActivityMs(r: SessionRecord): number | null {
  for (const ts of [r.lastSeenAt, r.startedAt]) {
    if (ts) {
      const ms = Date.parse(ts);
      if (Number.isFinite(ms)) return ms;
    }
  }
  return null;
}

function parsePositiveInt(raw: string | null, fallback: number): number {
  if (raw === null) return fallback;
  const n = Number.parseInt(raw, 10);
  return Number.isFinite(n) && n >= 0 ? n : fallback;
}

/**
 * Parse the shared query params. `window`: a day count ("7", "30"),
 * "all" for all history; anything else falls back to the default.
 * Unknown params are ignored so existing plain links keep working.
 */
export function parseRegistryQuery(params: URLSearchParams, _now = new Date()): RegistryQuery {
  const windowRaw = params.get("window");
  let windowDays: number | null = REGISTRY_DEFAULT_WINDOW_DAYS;
  if (windowRaw === "all") windowDays = null;
  else if (windowRaw !== null && /^\d+$/.test(windowRaw)) windowDays = Number.parseInt(windowRaw, 10);
  const limitRaw = parsePositiveInt(params.get("limit"), REGISTRY_DEFAULT_LIMIT);
  const limit = Math.min(REGISTRY_MAX_LIMIT, Math.max(1, limitRaw || REGISTRY_DEFAULT_LIMIT));
  const offset = parsePositiveInt(params.get("offset"), 0);
  const clean = (v: string | null): string | null => {
    const t = v?.trim();
    return t ? t : null;
  };
  return {
    windowDays,
    q: clean(params.get("q")),
    agent: clean(params.get("agent")),
    task: clean(params.get("task")),
    limit,
    offset,
    id: clean(params.get("id")),
  };
}

function matchesText(r: SessionRecord, q: string): boolean {
  const hay = [
    r.id,
    r.label,
    r.title ?? "",
    r.agentId,
    r.agentLabel,
    r.source,
    r.status,
    r.model ?? "",
    r.directory ?? "",
    ...r.taskRefs,
  ]
    .join("\n")
    .toLowerCase();
  return hay.includes(q.toLowerCase());
}

/** Run the full query pipeline over the store records. Pure. */
export function queryRegistry(records: SessionRecord[], query: RegistryQuery, now = new Date()): RegistryView {
  const totalAll = records.length;
  // Window first so hiddenByWindow is attributable to the window alone.
  let hiddenByWindow = 0;
  let inWindow = records;
  if (query.windowDays !== null) {
    const cutoff = now.getTime() - query.windowDays * DAY_MS;
    inWindow = records.filter((r) => {
      const ms = registryActivityMs(r);
      const visible = ms === null || ms >= cutoff;
      if (!visible) hiddenByWindow++;
      return visible;
    });
  }
  let matchedSet = inWindow;
  const qText = query.q;
  if (qText) matchedSet = matchedSet.filter((r) => matchesText(r, qText));
  if (query.agent) {
    const a = query.agent.toLowerCase();
    matchedSet = matchedSet.filter(
      (r) => r.agentId.toLowerCase().includes(a) || r.agentLabel.toLowerCase().includes(a),
    );
  }
  if (query.task) {
    const t = query.task.toLowerCase();
    matchedSet = matchedSet.filter((r) => r.taskRefs.some((ref) => ref.toLowerCase().includes(t)));
  }
  // Newest activity first so pagination is stable and useful; undated
  // records sort as epoch 0 within the matched set (they passed the
  // window as "recent", but have no recency to rank by).
  const sorted = [...matchedSet].sort((a, b) => (registryActivityMs(b) ?? 0) - (registryActivityMs(a) ?? 0));
  const matched = sorted.length;
  const page = sorted.slice(query.offset, query.offset + query.limit);
  // Deep-link pin: a named record resolves even when the window,
  // filters, or pagination would have excluded it.
  let pinnedIncluded = false;
  if (query.id) {
    const pinned = records.find((r) => r.id === query.id);
    if (pinned && !page.some((r) => r.id === pinned.id)) {
      page.unshift(pinned);
      pinnedIncluded = true;
    }
  }
  return {
    query,
    records: page,
    totalAll,
    hiddenByWindow,
    matched,
    hiddenByPagination: Math.max(0, matched - (page.length - (pinnedIncluded ? 1 : 0))),
    pinnedIncluded,
  };
}

/** Canonical query string for a query (links/forms round-trip). */
export function registryQueryString(query: RegistryQuery, overrides: Partial<RegistryQuery> = {}): string {
  const q = { ...query, ...overrides };
  const p = new URLSearchParams();
  if (q.windowDays === null) p.set("window", "all");
  else if (q.windowDays !== REGISTRY_DEFAULT_WINDOW_DAYS) p.set("window", String(q.windowDays));
  if (q.q) p.set("q", q.q);
  if (q.agent) p.set("agent", q.agent);
  if (q.task) p.set("task", q.task);
  if (q.limit !== REGISTRY_DEFAULT_LIMIT) p.set("limit", String(q.limit));
  if (q.offset > 0) p.set("offset", String(q.offset));
  const s = p.toString();
  return s ? `?${s}` : "";
}
