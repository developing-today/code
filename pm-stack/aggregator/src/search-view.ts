import type { BoardItem, DnsSection, GhItem, RefKey, RoadmapRow, SaasService, SessionRecord, TierItem } from "./model.ts";

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

function includesQuery(query: string, values: (string | undefined)[]): boolean {
  return values.some((value) => value?.toLowerCase().includes(query));
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

  for (const record of extras.registry ?? []) {
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
        refs: record.taskRefs,
        href: sessionHref(record.id),
      });
    }
  }

  for (const entry of extras.dns?.entries ?? []) {
    if (
      includesQuery(query, [entry.domain, entry.registrar, entry.dnsHost, entry.plan, entry.used, entry.review, entry.status])
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
      includesQuery(query, [item.id, item.title, item.kind, item.source, item.state, item.reason, item.ref, item.sessionId])
    ) {
      results.push({
        kind: "tier",
        id: item.id,
        title: item.title,
        summary: `${item.source} · ${item.state ?? item.kind}`,
        refs: item.ref ? [item.ref] : [],
        href: tierHref(item),
      });
    }
  }

  return results;
}
