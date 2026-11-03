import type { BoardItem, GhItem, RefKey, RoadmapRow } from "./model.ts";

export interface SearchResult {
  kind: "board" | "roadmap";
  id: string;
  title: string;
  summary: string;
  refs: RefKey[];
}

function includesQuery(query: string, values: (string | undefined)[]): boolean {
  return values.some((value) => value?.toLowerCase().includes(query));
}

/** Search the first-slice canonical subjects and their projected GitHub refs. */
export function searchWork(
  board: BoardItem[],
  rows: RoadmapRow[],
  items: Map<RefKey, GhItem>,
  rawQuery: string,
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
      });
    }
  }

  return results;
}
