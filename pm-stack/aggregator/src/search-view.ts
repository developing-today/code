import type {
  BoardItem,
  DnsSection,
  GhItem,
  RefKey,
  RoadmapRow,
  SaasService,
  SessionRecord,
  TierItem,
} from "./model.ts";
import { isValidRef } from "./model.ts";

export interface SearchResult {
  kind: "board" | "roadmap" | "registry" | "dns" | "service" | "tier";
  id: string;
  title: string;
  summary: string;
  refs: RefKey[];
  /** Stable detail view for this result (#1212 slice 2): every kind
   * links to its own home detail route, not /item/<id> for all. */
  href: string;
}

/** The non-board/roadmap subjects slice 2 adds, all optional so
 * first-slice callers and fixtures keep working unchanged. */
export interface SearchExtras {
  registry?: SessionRecord[];
  dns?: DnsSection | null;
  services?: SaasService[];
  tierItems?: TierItem[];
}

interface RegistrySearchIndex {
  /** Postings are appended in record order, so candidate traversal keeps
   * the registry's existing search-result order. */
  postings: Map<string, number[]>;
  /** Refs for the item-title enrichment stage, extracted with the same
   * snapshot build rather than flat-mapped on every search request. */
  refs: RefKey[];
}

// Search snapshots are immutable: the live source replaces the registry
// array when its feed/laptop inputs change. A WeakMap lets each snapshot
// pay the index-build cost once, then query-specific candidates avoid
// visiting every session on each request.
const registrySearchIndexes = new WeakMap<SessionRecord[], RegistrySearchIndex>();
const REGISTRY_SEARCH_GRAM_MAX = 3;

function sessionSearchFields(record: SessionRecord): unknown[] {
  return [
    record.id,
    record.label,
    record.title,
    record.agentId,
    record.agentLabel,
    record.status,
    record.model,
    record.directory,
    ...record.taskRefs,
  ];
}

function grams(value: string, size: number): Set<string> {
  const found = new Set<string>();
  for (let start = 0; start + size <= value.length; start++) found.add(value.slice(start, start + size));
  return found;
}

function addGrams(value: string, size: number, found: Set<string>): void {
  for (let start = 0; start + size <= value.length; start++) found.add(value.slice(start, start + size));
}

function registrySearchIndex(records: SessionRecord[]): RegistrySearchIndex {
  const cached = registrySearchIndexes.get(records);
  if (cached) return cached;

  const postings = new Map<string, number[]>();
  const refs = new Set<RefKey>();
  for (let index = 0; index < records.length; index++) {
    const record = records[index];
    if (!record) continue;
    for (const ref of record.taskRefs) if (isValidRef(ref)) refs.add(ref);
    const recordGrams = new Set<string>();
    for (const field of sessionSearchFields(record)) {
      if (typeof field !== "string") continue;
      const value = field.toLowerCase();
      for (let size = 1; size <= REGISTRY_SEARCH_GRAM_MAX; size++) addGrams(value, size, recordGrams);
    }
    for (const gram of recordGrams) {
      const posting = postings.get(gram);
      if (posting) posting.push(index);
      else postings.set(gram, [index]);
    }
  }
  const built = { postings, refs: [...refs] };
  registrySearchIndexes.set(records, built);
  return built;
}

/** Distinct, validated session refs for item-title enrichment. Cached
 * with the search index so large registries are not flat-mapped per query. */
export function registrySearchRefs(records: SessionRecord[]): RefKey[] {
  return registrySearchIndex(records).refs;
}

function containsSorted(posting: number[], value: number): boolean {
  let low = 0;
  let high = posting.length - 1;
  while (low <= high) {
    const mid = (low + high) >>> 1;
    const current = posting[mid];
    if (current === value) return true;
    if (current === undefined || current > value) high = mid - 1;
    else low = mid + 1;
  }
  return false;
}

function registryCandidateIndices(query: string, records: SessionRecord[]): number[] {
  if (!query) return [];
  const index = registrySearchIndex(records);
  const size = Math.min(REGISTRY_SEARCH_GRAM_MAX, query.length);
  const queryGrams = size === query.length ? [query] : [...grams(query, size)];
  const postings = queryGrams.map((gram) => index.postings.get(gram));
  if (postings.some((posting) => !posting)) return [];
  const available = postings as number[][];
  const seed = available.reduce((smallest, posting) => (posting.length < smallest.length ? posting : smallest));
  const remaining = available.filter((posting) => posting !== seed);
  return remaining.length ? seed.filter((candidate) => remaining.every((posting) => containsSorted(posting, candidate))) : seed;
}

/** #14060 slice 2: search results are paginated and capped so a
 * common-word query cannot ship an unbounded payload (measured:
 * `q=fleet` returned 901 KB of JSON / 941 KB of HTML). The full match
 * count is always reported as `total`; callers render/ship one page. */
export const SEARCH_DEFAULT_LIMIT = 50;
export const SEARCH_MAX_LIMIT = 200;

export interface SearchPagination {
  limit: number;
  offset: number;
}

export interface SearchPage {
  page: SearchResult[];
  total: number;
  limit: number;
  offset: number;
  /** Matches after this page (0 on the final page). */
  remainingAfterPage: number;
}

function parseSearchInt(raw: string | null, fallback: number): number {
  // Strict decimal grammar, same rule as parseRegistryQuery (#3717):
  // prefix-parsing would silently select a different page than spelled.
  if (raw === null || !/^\d+$/.test(raw)) return fallback;
  const n = Number.parseInt(raw, 10);
  return Number.isFinite(n) && n >= 0 ? n : fallback;
}

export function parseSearchPagination(params: URLSearchParams): SearchPagination {
  const rawLimit = parseSearchInt(params.get("limit"), SEARCH_DEFAULT_LIMIT);
  const limit = Math.min(SEARCH_MAX_LIMIT, Math.max(1, rawLimit || SEARCH_DEFAULT_LIMIT));
  return { limit, offset: parseSearchInt(params.get("offset"), 0) };
}

/** Pure page slice over a full result set. Oracle: page is exactly
 * results[offset, offset+limit) and total is the unpaginated count. */
export function paginateSearchResults(results: SearchResult[], pagination: SearchPagination): SearchPage {
  const page = results.slice(pagination.offset, pagination.offset + pagination.limit);
  return {
    page,
    total: results.length,
    limit: pagination.limit,
    offset: pagination.offset,
    remainingAfterPage: Math.max(0, results.length - (pagination.offset + pagination.limit)),
  };
}

function includesQuery(query: string, values: unknown[]): boolean {
  // Codex P2 on PR #7331: accepted records can carry non-string optional
  // fields (an unvalidated registry `model: 7`, a DNS `registrar`, ...);
  // `value?.toLowerCase()` only guards nullish values and threw on those,
  // taking down /search and /api/search.json for every query that reached
  // the field. Only strings are searchable.
  return values.some((value) => typeof value === "string" && value.toLowerCase().includes(query));
}

/** Anchor id for a register row on /dns (dnsTable renders the same id). */
export function dnsAnchor(domain: string): string {
  return `dns-${domain.toLowerCase()}`;
}

/** The stable session view: the registry page filtered to one record. */
function sessionHref(id: string): string {
  return `/sessions?id=${encodeURIComponent(id)}#${encodeURIComponent(id)}`;
}

/** Mirror of render.ts tierItemObjectRef, kept local so the search
 * view stays free of render imports: watchdog items key by their own
 * id, ref-carrying items by their gh: ref, the rest by orch: id. */
function tierObjectRef(item: TierItem): string {
  if (item.source === "watchdog") return `orch:${item.id}`;
  return item.ref ? `gh:${item.ref}` : `orch:${item.id}`;
}

function tierHref(item: TierItem): string {
  if (item.sessionId) return sessionHref(item.sessionId);
  return `/discuss/${encodeURIComponent(tierObjectRef(item))}`;
}

/** Search the canonical subjects and their projected GitHub refs.
 * Slice 2 (#1212) adds the registry/session records, the DNS register,
 * the usage-hub services, and the orchestration tier items, each
 * linking to its stable detail view. */
export function searchWork(
  board: BoardItem[],
  rows: RoadmapRow[],
  items: Map<RefKey, GhItem>,
  rawQuery: string,
  extras: SearchExtras = {},
): SearchResult[] {
  const query = rawQuery.trim().toLowerCase();
  if (!query) return [];

  const results: SearchResult[] = [];
  for (const item of board) {
    const refs = item.links.map((link) => link.ref);
    const projectedTitles = refs.map((ref) => items.get(ref)?.title);
    if (includesQuery(query, [item.id, item.title, item.repo, item.status, item.source, ...refs, ...projectedTitles])) {
      results.push({
        kind: "board",
        id: item.id,
        title: item.title,
        summary: `${item.repo} · ${item.status}`,
        refs,
        href: `/item/${encodeURIComponent(item.id)}`,
      });
    }
  }

  for (const row of rows) {
    const refs = row.links.map((link) => link.ref);
    const projectedTitles = refs.map((ref) => items.get(ref)?.title);
    if (
      includesQuery(query, [
        row.subject,
        row.title,
        row.area,
        row.status,
        row.horizon,
        row.source,
        row.notes,
        ...refs,
        ...projectedTitles,
      ])
    ) {
      results.push({
        kind: "roadmap",
        id: row.subject,
        title: row.title,
        summary: `${row.area} · ${row.status} · ${row.horizon}`,
        refs,
        href: `/item/${encodeURIComponent(row.subject)}`,
      });
    }
  }

  const registry = extras.registry ?? [];
  for (const recordIndex of registryCandidateIndices(query, registry)) {
    const record = registry[recordIndex];
    if (!record) continue;
    const title = record.label || record.title || record.id;
    if (
      includesQuery(query, [
        record.id,
        record.label,
        record.title,
        record.agentId,
        record.agentLabel,
        record.status,
        record.model,
        record.directory,
        ...record.taskRefs,
      ])
    ) {
      results.push({
        kind: "registry",
        id: record.id,
        title,
        summary: `${record.agentLabel} · ${record.status}`,
        refs: record.taskRefs.filter(isValidRef),
        href: sessionHref(record.id),
      });
    }
  }

  for (const entry of extras.dns?.entries ?? []) {
    if (
      includesQuery(query, [
        entry.domain,
        entry.registrar,
        entry.dnsHost,
        entry.plan,
        entry.used,
        entry.review,
        entry.status,
      ])
    ) {
      results.push({
        kind: "dns",
        id: entry.domain,
        title: entry.domain,
        summary: `${entry.registrar} · ${entry.status} · renews ${entry.renewal}`,
        refs: [],
        href: `/dns#${dnsAnchor(entry.domain)}`,
      });
    }
  }

  for (const svc of extras.services ?? []) {
    if (includesQuery(query, [svc.id, svc.name, svc.plan, svc.status, svc.group])) {
      results.push({
        kind: "service",
        id: svc.id,
        title: svc.name,
        summary: `${svc.group} · ${svc.plan}`,
        refs: [],
        href: `/quota/service/${encodeURIComponent(svc.id)}`,
      });
    }
  }

  for (const item of extras.tierItems ?? []) {
    if (
      includesQuery(query, [
        item.id,
        item.title,
        item.kind,
        item.source,
        item.state,
        item.reason,
        item.ref,
        item.sessionId,
      ])
    ) {
      results.push({
        kind: "tier",
        id: item.id,
        title: item.title,
        summary: `${item.source} · ${item.state ?? item.kind}`,
        // Codex P1 on PR #7331: a tier ref can be a tracked-goal id or
        // another non-GitHub key; passing it on as a badge ref made
        // renderSearch's refUrl throw and failed the whole /search
        // request. Only owner/repo#N refs become GitHub badges.
        refs: item.ref && isValidRef(item.ref) ? [item.ref] : [],
        href: tierHref(item),
      });
    }
  }

  return results;
}
