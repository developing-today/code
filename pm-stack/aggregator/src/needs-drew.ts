// The Needs-Drew label census (#1226): the GitHub user/* labels are the
// system of record for "waits on Drew", and this module reconciles them
// into the aggregator's attention view as a read-only source. It censuses
// OPEN issues and PRs carrying user/review or user/escalated across the
// four repos via the GitHub search API (paginated), deduplicates items
// carrying both labels, and reports per-source failures and freshness
// honestly — a failed repo/label query is named in `sources`, never
// silently rendered as "nothing waiting".
//
// Reconciliation direction (#1226): labels are the INPUT to the page.
// The aggregator never writes labels back; the orchestrator remains the
// only label writer.
//
// Acceptance discrepancy, preserved on purpose: the original #1226
// acceptance said "#1007 appears in the queue", but #1007 is CLOSED and
// this census is open-only by design, so a closed item can never appear
// here. That is recorded on #1226 rather than claimed as met.
import { REPOS } from "./model.ts";

export const NEEDS_DREW_LABELS = ["user/review", "user/escalated"] as const;
export type NeedsDrewLabel = (typeof NEEDS_DREW_LABELS)[number];

export interface NeedsDrewItem {
  /** "owner/repo#123" — the registry's canonical key. */
  ref: string;
  repo: string;
  number: number;
  kind: "issue" | "pr";
  title: string;
  url: string;
  /** Every label on the item, as GitHub reports them. */
  labels: string[];
  /** The subset of NEEDS_DREW_LABELS the item carries (>= 1). */
  needsLabels: NeedsDrewLabel[];
  /** A reason extracted from the item body when it states one
   *  (e.g. an escalation line); null = no reason recorded — renderers
   *  must show the honest fallback, not invent one. */
  reason: string | null;
  updatedAt: string;
}

export interface NeedsDrewSourceStatus {
  repo: string;
  label: NeedsDrewLabel;
  ok: boolean;
  /** Items this query contributed before dedupe (open only). */
  count: number;
  fetchedAt: string;
  error?: string;
  /** True when the page cap stopped the census early — the count is a
   *  floor, and the census says so instead of looking complete. */
  truncated?: boolean;
}

export interface NeedsDrewCensus {
  fetchedAt: string;
  items: NeedsDrewItem[];
  sources: NeedsDrewSourceStatus[];
  /** True only when every repo/label query succeeded untruncated. */
  complete: boolean;
}

/** One page fetch, shaped like the live source's getJson result so the
 *  live source can pass its own fetcher straight in. */
export type NeedsDrewFetchPage = (url: string) => Promise<{ ok: boolean; json: unknown }>;

const PER_PAGE = 100;
const MAX_PAGES = 10;

export function needsDrewSearchUrl(repo: string, label: NeedsDrewLabel, page: number): string {
  const q = `repo:${repo} state:open label:"${label}"`;
  return `https://api.github.com/search/issues?q=${encodeURIComponent(q)}&per_page=${PER_PAGE}&page=${page}`;
}

interface RawSearchItem {
  number?: unknown;
  title?: unknown;
  html_url?: unknown;
  state?: unknown;
  updated_at?: unknown;
  body?: unknown;
  pull_request?: unknown;
  labels?: unknown;
}

function labelNames(raw: unknown): string[] {
  if (!Array.isArray(raw)) return [];
  const out: string[] = [];
  for (const l of raw) {
    if (typeof l === "string") out.push(l);
    else if (l && typeof l === "object" && typeof (l as { name?: unknown }).name === "string")
      out.push((l as { name: string }).name);
  }
  return out;
}

/** Best-effort reason: an explicit "Escalated: …" / "Reason: …" line in
 *  the item body. Anything else is null — the fallback is the point. */
export function extractNeedsDrewReason(body: unknown): string | null {
  if (typeof body !== "string") return null;
  const m = /(?:^|\n)\s*(?:escalated(?:\s+to\s+drew)?|reason)\s*[:\-—]\s*(.+)/i.exec(body);
  const reason = m?.[1]?.trim();
  return reason ? reason.slice(0, 240) : null;
}

export async function buildNeedsDrewCensus(
  fetchPage: NeedsDrewFetchPage,
  opts: { repos?: readonly string[]; labels?: readonly NeedsDrewLabel[]; now?: Date } = {},
): Promise<NeedsDrewCensus> {
  const repos = opts.repos ?? REPOS;
  const labels = opts.labels ?? NEEDS_DREW_LABELS;
  const fetchedAt = (opts.now ?? new Date()).toISOString();
  const byRef = new Map<string, NeedsDrewItem>();
  const sources: NeedsDrewSourceStatus[] = [];

  for (const repo of repos) {
    for (const label of labels) {
      const status: NeedsDrewSourceStatus = { repo, label, ok: true, count: 0, fetchedAt };
      try {
        let page = 1;
        let total = Number.POSITIVE_INFINITY;
        while (page <= MAX_PAGES) {
          const res = await fetchPage(needsDrewSearchUrl(repo, label, page));
          if (!res.ok) {
            const j = res.json as { status?: unknown; error?: unknown } | null;
            throw new Error(
              typeof j?.status === "number" ? `HTTP ${j.status}` : String(j?.error ?? "fetch failed"),
            );
          }
          const payload = res.json as { total_count?: unknown; items?: unknown };
          if (typeof payload?.total_count === "number") total = payload.total_count;
          const rawItems = Array.isArray(payload?.items) ? (payload.items as RawSearchItem[]) : [];
          for (const raw of rawItems) {
            // Defensive: the query asks for state:open, but a stale or
            // hand-built payload must never leak a closed item in.
            if (raw.state !== "open") continue;
            if (typeof raw.number !== "number") continue;
            status.count++;
            const ref = `${repo}#${raw.number}`;
            const existing = byRef.get(ref);
            if (existing) {
              if (!existing.needsLabels.includes(label)) existing.needsLabels.push(label);
              continue;
            }
            const names = labelNames(raw.labels);
            byRef.set(ref, {
              ref,
              repo,
              number: raw.number,
              kind: raw.pull_request ? "pr" : "issue",
              title: typeof raw.title === "string" ? raw.title : ref,
              url: typeof raw.html_url === "string" ? raw.html_url : `https://github.com/${repo}/issues/${raw.number}`,
              labels: names,
              needsLabels: names.filter((n): n is NeedsDrewLabel =>
                (NEEDS_DREW_LABELS as readonly string[]).includes(n),
              ),
              reason: extractNeedsDrewReason(raw.body),
              updatedAt: typeof raw.updated_at === "string" ? raw.updated_at : fetchedAt,
            });
          }
          if (rawItems.length < PER_PAGE || status.count >= total) break;
          page++;
        }
        if (status.count < total) status.truncated = true;
      } catch (err) {
        status.ok = false;
        status.error = err instanceof Error ? err.message : String(err);
      }
      sources.push(status);
    }
  }

  const repoOrder = new Map<string, number>(repos.map((r, i) => [r, i]));
  const items = [...byRef.values()].sort(
    (a, b) => (repoOrder.get(a.repo) ?? 0) - (repoOrder.get(b.repo) ?? 0) || a.number - b.number,
  );
  return {
    fetchedAt,
    items,
    sources,
    complete: sources.every((s) => s.ok && !s.truncated),
  };
}
