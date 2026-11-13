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
import { randomUUID } from "node:crypto";
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join } from "node:path";

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
  /** True when any page of this query came back with GitHub's
   *  `incomplete_results: true` (search timed out / partially indexed).
   *  The count is then a floor even when it equals `total_count`, so the
   *  census must never present it as authoritative (Codex P2 on #3970). */
  incomplete?: boolean;
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

// Cap `t` at `max` UTF-16 code units, but never leave half of a surrogate
// pair at the cut (#7741, mirroring the #7694 capUnits guard in actions.ts):
// an astral character straddling the cap is dropped whole. Local copy, like
// comments.ts (#7731) — a shared import would tangle module dependencies.
function capUnits(t: string, max: number): string {
  if (t.length <= max) return t;
  let end = max;
  const before = t.charCodeAt(end - 1);
  const after = t.charCodeAt(end);
  if (before >= 0xd800 && before <= 0xdbff && after >= 0xdc00 && after <= 0xdfff) end--;
  return t.slice(0, end);
}

/** Best-effort reason: an explicit "Escalated: …" / "Reason: …" line in
 *  the item body. Anything else is null — the fallback is the point. */
export function extractNeedsDrewReason(body: unknown): string | null {
  if (typeof body !== "string") return null;
  const m = /(?:^|\n)\s*(?:escalated(?:\s+to\s+drew)?|reason)\s*[:\-—]\s*(.+)/i.exec(body);
  const reason = m?.[1]?.trim();
  return reason ? capUnits(reason, 240) : null;
}

export interface NeedsDrewSweeperOptions {
  /** How long a completed sweep stays fresh. */
  ttlMs?: number;
  /** Clock, injectable for tests. */
  now?: () => number;
}

/**
 * Whether an attention context should fetch the census at all (#3970).
 *
 * Only the attention page render consumes the census; the overview,
 * Today, /api/attention.json and /api/coverage.json discard it. The
 * sweep is up to eight sequential GitHub searches, so paying it for
 * callers that ignore the result made every view depend on an external
 * API it never needed. Default is "do not fetch" — opting in is
 * explicit, so a new caller cannot inherit the dependency by accident.
 */
export function shouldFetchNeedsDrewCensus(opts?: { census?: boolean }): boolean {
  return opts?.census === true;
}

/**
 * Cache + single-flight wrapper around one census sweep (#1226/#3970).
 *
 * A cold or expired cache used to let every concurrent caller start its
 * own sweep — the three attention fragments rendering together produced
 * 24 GitHub search requests instead of 8, burning the search rate limit
 * and then caching a rate-limited partial result for five minutes.
 * Callers that arrive mid-sweep now share the in-flight promise. Only a
 * resolved sweep is cached, so a failure is never served as real data.
 */
export function createNeedsDrewSweeper(
  fetchPage: NeedsDrewFetchPage,
  buildOpts: { repos?: readonly string[]; labels?: readonly NeedsDrewLabel[] } = {},
  { ttlMs = 300_000, now = Date.now }: NeedsDrewSweeperOptions = {},
): () => Promise<NeedsDrewCensus> {
  let cache: { value: NeedsDrewCensus; at: number } | null = null;
  let inflight: Promise<NeedsDrewCensus> | null = null;
  return () => {
    if (cache && now() - cache.at < ttlMs) return Promise.resolve(cache.value);
    if (inflight) return inflight;
    const run = buildNeedsDrewCensus(fetchPage, buildOpts)
      .then((census) => {
        cache = { value: census, at: now() };
        return census;
      })
      .finally(() => {
        if (inflight === run) inflight = null;
      });
    inflight = run;
    return run;
  };
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
            throw new Error(typeof j?.status === "number" ? `HTTP ${j.status}` : String(j?.error ?? "fetch failed"));
          }
          const payload = res.json as { total_count?: unknown; items?: unknown; incomplete_results?: unknown };
          if (typeof payload?.total_count === "number") total = payload.total_count;
          // GitHub returns 200 + incomplete_results for a search that
          // timed out or is only partially indexed. Ignoring the flag
          // would let a truncated sweep look complete — worst case an
          // authoritative empty queue (Codex P2 on #3970).
          if (payload?.incomplete_results === true) status.incomplete = true;
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
    complete: sources.every((s) => s.ok && !s.truncated && !s.incomplete),
  };
}

// ---------------------------------------------------------------------------
// Needs-Drew QUESTIONS (#8568, notifications Phase 1): a fleet worker
// blocked on Drew raises a question through the alert throat
// (POST /api/alerts with a `needsDrew` extension, source "fleet-queue").
// This file-backed store is the aggregator half of that contract: the
// per-question token (minted once, at first raise) is the authority for
// the answer page (/answer/<ledgerId>?t=<token>) and the answer endpoint
// (POST /api/needs-drew/<ledgerId>/answer). First answer wins. Answered
// items are published into fleet-feed.json's top-level `answers` array
// by scripts/collect-fleet-feed.ts so the fleet lane can resume workers.
// NOTE: this section shares the module with the #1226 label census
// above because the task named this file; the two concerns are
// independent (census = GitHub labels, read-only; questions = local
// store, writable).

export interface NeedsDrewOption {
  id: string;
  label: string;
}

export interface NeedsDrewAnswer {
  optionId?: string;
  freeText?: string;
  at: string;
  via: string;
}

export interface NeedsDrewQuestion {
  ledgerId: string;
  question: string;
  context?: string;
  options: NeedsDrewOption[];
  links: string[];
  /** Per-question answer authority; minted at first raise, never rotated. */
  token: string;
  raisedAt: string;
  source: string;
  answer?: NeedsDrewAnswer;
}

export interface NeedsDrewInput {
  ledgerId: string;
  question: string;
  context?: string;
  options: NeedsDrewOption[];
  links: string[];
}

export const NEEDS_DREW_OPEN_MAX_AGE_MS = 7 * 24 * 60 * 60_000;

/** Default question-store path (mirrors src/config.ts needsDrewFile). */
export function defaultNeedsDrewFile(env: Record<string, string | undefined> = process.env): string {
  const v = env.PM_AGG_NEEDS_DREW_FILE?.trim();
  return v ? v : join(homedir(), ".local", "state", "pm-aggregator", "needs-drew.json");
}

/** Validate the intake `needsDrew` extension object (#8568 wire contract). */
export function validateNeedsDrew(value: unknown): { ok: true; value: NeedsDrewInput } | { ok: false; error: string } {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return { ok: false, error: "needsDrew must be an object" };
  }
  const b = value as Record<string, unknown>;
  const ledgerId = typeof b.ledgerId === "string" ? b.ledgerId.trim() : "";
  if (!ledgerId) return { ok: false, error: "needsDrew.ledgerId required" };
  const question = typeof b.question === "string" ? b.question.trim() : "";
  if (!question) return { ok: false, error: "needsDrew.question required" };
  if (b.context !== undefined && typeof b.context !== "string") {
    return { ok: false, error: "needsDrew.context must be a string" };
  }
  if (!Array.isArray(b.options) || b.options.length < 1 || b.options.length > 8) {
    return { ok: false, error: "needsDrew.options must be an array of 1-8 entries" };
  }
  const options: NeedsDrewOption[] = [];
  for (const raw of b.options) {
    if (!raw || typeof raw !== "object" || Array.isArray(raw)) {
      return { ok: false, error: "needsDrew.options entries must be objects" };
    }
    const o = raw as Record<string, unknown>;
    const id = typeof o.id === "string" ? o.id.trim() : "";
    const label = typeof o.label === "string" ? o.label.trim() : "";
    if (!id || !label) return { ok: false, error: "needsDrew.options entries require id and label" };
    options.push({ id, label });
  }
  let links: string[] = [];
  if (b.links !== undefined) {
    if (!Array.isArray(b.links) || b.links.some((l) => typeof l !== "string")) {
      return { ok: false, error: "needsDrew.links must be an array of strings" };
    }
    links = [...(b.links as string[])];
  }
  return {
    ok: true,
    value: {
      ledgerId,
      question,
      ...(typeof b.context === "string" ? { context: b.context } : {}),
      options,
      links,
    },
  };
}

interface NeedsDrewFileShape {
  version: 1;
  items: NeedsDrewQuestion[];
}

async function readNeedsDrewItems(file: string): Promise<NeedsDrewQuestion[]> {
  try {
    const parsed: unknown = JSON.parse(await readFile(file, "utf8"));
    if (parsed && typeof parsed === "object" && Array.isArray((parsed as NeedsDrewFileShape).items)) {
      return (parsed as NeedsDrewFileShape).items.filter(
        (i): i is NeedsDrewQuestion =>
          !!i && typeof i === "object" && typeof i.ledgerId === "string" && typeof i.token === "string",
      );
    }
    if (Array.isArray(parsed)) return parsed as NeedsDrewQuestion[];
    return [];
  } catch {
    return []; // absent/corrupt store reads as empty; a raise rewrites it
  }
}

let needsDrewWriteSeq = 0;
const needsDrewChains = new Map<string, Promise<unknown>>();

function serializeNeedsDrew<T>(file: string, fn: () => Promise<T>): Promise<T> {
  const prev = needsDrewChains.get(file) ?? Promise.resolve();
  const run = prev.then(fn, fn);
  needsDrewChains.set(
    file,
    run.catch(() => {}),
  );
  return run;
}

async function writeNeedsDrewItems(file: string, items: NeedsDrewQuestion[]): Promise<void> {
  await mkdir(dirname(file), { recursive: true });
  const tmp = `${file}.tmp-${process.pid}-${++needsDrewWriteSeq}`;
  const data: NeedsDrewFileShape = { version: 1, items };
  await writeFile(tmp, JSON.stringify(data, null, 2));
  await rename(tmp, file);
}

/** Raise a question. Idempotent: an existing ledgerId keeps its token
 * and its original fields (a re-post is a duplicate, not an edit). */
export async function raiseNeedsDrew(
  file: string,
  input: NeedsDrewInput,
  source = "fleet-queue",
): Promise<NeedsDrewQuestion> {
  return serializeNeedsDrew(file, async () => {
    const items = await readNeedsDrewItems(file);
    const existing = items.find((i) => i.ledgerId === input.ledgerId);
    if (existing) return existing;
    const item: NeedsDrewQuestion = {
      ledgerId: input.ledgerId,
      question: input.question,
      ...(input.context !== undefined ? { context: input.context } : {}),
      options: input.options,
      links: input.links,
      token: randomUUID(),
      raisedAt: new Date().toISOString(),
      source,
    };
    items.push(item);
    await writeNeedsDrewItems(file, items);
    return item;
  });
}

export async function getNeedsDrew(file: string, ledgerId: string): Promise<NeedsDrewQuestion | null> {
  const items = await readNeedsDrewItems(file);
  return items.find((i) => i.ledgerId === ledgerId) ?? null;
}

/** Record an answer. First answer wins; repeats return current state. */
export async function recordAnswer(
  file: string,
  ledgerId: string,
  answer: { optionId?: string; freeText?: string; via?: string },
): Promise<NeedsDrewQuestion | null> {
  return serializeNeedsDrew(file, async () => {
    const items = await readNeedsDrewItems(file);
    const item = items.find((i) => i.ledgerId === ledgerId);
    if (!item) return null;
    if (item.answer) return item;
    item.answer = {
      ...(answer.optionId !== undefined ? { optionId: answer.optionId } : {}),
      ...(answer.freeText !== undefined ? { freeText: answer.freeText } : {}),
      at: new Date().toISOString(),
      via: answer.via ?? "api",
    };
    await writeNeedsDrewItems(file, items);
    return item;
  });
}

/** Open (unanswered) questions younger than 7 days, oldest first. */
export async function listOpenNeedsDrew(file: string, now = Date.now()): Promise<NeedsDrewQuestion[]> {
  const items = await readNeedsDrewItems(file);
  return items
    .filter((i) => !i.answer && now - Date.parse(i.raisedAt) < NEEDS_DREW_OPEN_MAX_AGE_MS)
    .sort((a, b) => a.raisedAt.localeCompare(b.raisedAt));
}

/** Every answered item, in raise order (the feed's `answers` source). */
export async function listAnswers(file: string): Promise<NeedsDrewQuestion[]> {
  const items = await readNeedsDrewItems(file);
  return items.filter((i) => i.answer);
}
