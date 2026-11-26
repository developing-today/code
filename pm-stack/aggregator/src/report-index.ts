import { createHash } from "node:crypto";
import type { ReportRecord } from "./report-records.ts";

export interface ReportIndexEntry {
  id: string;
  pageId: string;
  recordId: string;
  contentHash: string;
  periodId: string;
  title: string;
  kind: string;
  createdAt: string;
  updatedAt: string;
  status: "current";
  links: string[];
  text: string;
}

export interface ReportIndexQuery {
  q?: string;
  kind?: string;
  from?: string;
  to?: string;
  limit?: number;
  offset?: number;
}

export function reportIndexEntries(records: ReportRecord[]): ReportIndexEntry[] {
  return records.map((record) => {
    const links = [...new Set([...record.sources.map((source) => source.uri), ...record.blocks.flatMap((block) => block.links ?? [])])].sort();
    const title = `${record.period.kind} report · ${record.period.periodStart.slice(0, 10)}`;
    return {
      id: record.id,
      pageId: `report:${record.period.id}`,
      recordId: record.id,
      contentHash: `sha256:${createHash("sha256").update(JSON.stringify(record)).digest("hex")}`,
      periodId: record.period.id,
      title,
      kind: record.period.kind,
      createdAt: record.generatedAt,
      updatedAt: record.generatedAt,
      status: "current",
      links,
      text: record.blocks.map((block) => `${block.id} ${block.text}`).join(" "),
    };
  }).sort((a, b) => b.updatedAt.localeCompare(a.updatedAt) || a.id.localeCompare(b.id));
}

export function searchReportIndex(entries: ReportIndexEntry[], query: ReportIndexQuery = {}) {
  const q = query.q?.trim().toLocaleLowerCase();
  const filtered = entries.filter((entry) => {
    if (query.kind && entry.kind !== query.kind) return false;
    if (query.from && entry.updatedAt < query.from) return false;
    if (query.to && entry.updatedAt > query.to) return false;
    return !q || [entry.title, entry.id, entry.periodId, entry.text, ...entry.links].some((value) => value.toLocaleLowerCase().includes(q));
  });
  const offset = Math.max(0, Math.floor(query.offset ?? 0));
  const limit = Math.min(100, Math.max(1, Math.floor(query.limit ?? 20)));
  return { records: filtered.slice(offset, offset + limit), total: filtered.length, offset, limit };
}
