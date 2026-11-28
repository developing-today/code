// The Needs-Drew label census (#1226): the GitHub user/* labels are the
// system of record for "waits on Drew", and this module reconciles them
// into the aggregator's attention view as a read-only source. It censuses
// OPEN issues and PRs carrying user/review or user/escalated across the
// four repos, from each repository's own list of open issues by label
// (paginated; that list carries pull requests too), deduplicates items
// carrying both labels, and reports per-source failures and freshness
// honestly — a failed repo/label query is named in `sources`, never
// silently rendered as "nothing waiting".
//
// It asked the search API until #14103. GitHub answers a search of
// issues that names neither `is:issue` nor `is:pull-request` with a
// 422 for some tokens (a GitHub App's, seen 2026-11-24) and not for
// others: with the token the dashboard runs with, the search answered
// and the census was whole. (#14103 was filed as if it failed in
// service; it did not.) The list answers either kind of token, is on
// the core allowance (the search has its own, thirty a minute), does
// not lag behind an index, and is what the PM console's edge reads for
// the same lists (#14112).
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
import { dirname, join } from "node:path";
import { requireHome } from "./home.ts";
import { needsUserStatePath } from "./needs-user-state-path.ts";

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
  /** The hands marker in the item body, when it carries one
   *  (FDR-0013). Absent on items censused before the marker existed;
   *  read it through {@link needsDrewWait}. */
  hands?: NeedsDrewHands | null;
}

/** What only a person's hands can do (FDR-0013). The same four kinds
 *  `fleet queue block --hands` accepts. */
export const HANDS_KINDS = ["physical", "interactive-login", "payment", "signature"] as const;
export type HandsKind = (typeof HANDS_KINDS)[number];

export interface NeedsDrewHands {
  kind: HandsKind;
  /** The one action, when the marker states it. */
  action: string | null;
}

/** How an item waits (FDR-0013, doc/decisions):
 *  - `hands`: something only a person's hands can do; it names one action.
 *  - `review`: a review was asked for (`user/review`).
 *  - `agent-decides`: an agent parked a choice on the owner. That is
 *    fleet work, not the owner's: an agent decides it and continues. */
export type NeedsDrewWait = "hands" | "review" | "agent-decides";

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
  /** True when a page of this query said its results were incomplete.
   *  That was the search API's `incomplete_results` (Codex P2 on #3970);
   *  the repository list this census asks now (#14103) has no such
   *  flag, so nothing sets this today. Kept because a census that
   *  carries it must still never be presented as authoritative. */
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

/** One page of a repository's open issues and pull requests carrying a
 *  label, as the REST API lists them. Not the search: see the note at
 *  the top of this file (#14103). */
export function needsDrewListUrl(repo: string, label: NeedsDrewLabel, page: number): string {
  return `https://api.github.com/repos/${repo}/issues?state=open&labels=${encodeURIComponent(label)}&per_page=${PER_PAGE}&page=${page}`;
}

interface RawListItem {
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

/** The hands marker an item body carries, if any:
 *  `<!-- hands:v1 kind=interactive-login action="one sentence" -->`.
 *  A marker with an unknown kind is not a hands marker: the item then
 *  falls back to its labels rather than being trusted as a hands item. */
export function extractNeedsDrewHands(body: unknown): NeedsDrewHands | null {
  if (typeof body !== "string") return null;
  const m = /<!--\s*hands:v1\s+kind=([a-z-]+)(?:\s+action="([^"]*)")?\s*-->/.exec(body);
  if (!m) return null;
  const kind = (HANDS_KINDS as readonly string[]).includes(m[1]) ? (m[1] as HandsKind) : null;
  if (!kind) return null;
  const action = m[2]?.trim();
  return { kind, action: action ? capUnits(action, 240) : null };
}

/** How one census item waits. A hands marker wins; then a review
 *  request; everything else was parked on the owner by an agent. */
export function needsDrewWait(item: Pick<NeedsDrewItem, "needsLabels" | "hands">): NeedsDrewWait {
  if (item.hands) return "hands";
  if (item.needsLabels.includes("user/review")) return "review";
  return "agent-decides";
}

/** The census items that are the owner's: hands items and review
 *  requests. Agent-parked choices are left out. */
export function needsDrewOwnerItems(census: NeedsDrewCensus): NeedsDrewItem[] {
  return census.items.filter((i) => needsDrewWait(i) !== "agent-decides");
}

/** Counts per kind of wait, for headings and scoreboards. */
export function needsDrewWaitCounts(census: NeedsDrewCensus): Record<NeedsDrewWait, number> {
  const counts: Record<NeedsDrewWait, number> = { hands: 0, review: 0, "agent-decides": 0 };
  for (const i of census.items) counts[needsDrewWait(i)]++;
  return counts;
}

/** How long a completed sweep stays fresh. A sweep is one request for
 *  each repository and label (eight), on GitHub's core allowance. With
 *  a token that is nothing against five thousand an hour, and five
 *  minutes is as fresh as the page needs. Without one GitHub allows an
 *  address sixty requests an hour, which a sweep every five minutes
 *  (ninety-six) would use up by itself: it is then made every ten
 *  minutes (forty-eight), which still leaves little for anything else.
 *  (The search this census asked until #14103 was counted apart, by the
 *  minute, so a missing token did not show there.) */
export function needsDrewSweepTtlMs(hasToken: boolean): number {
  return hasToken ? 300_000 : 600_000;
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
        while (page <= MAX_PAGES) {
          const res = await fetchPage(needsDrewListUrl(repo, label, page));
          if (!res.ok) {
            const j = res.json as { status?: unknown; error?: unknown } | null;
            throw new Error(typeof j?.status === "number" ? `HTTP ${j.status}` : String(j?.error ?? "fetch failed"));
          }
          // The list answers an array. Anything else under a 200 (an
          // error object, a payload of another shape) is a failed
          // query: read as "no items" it would be an authoritative
          // empty queue.
          if (!Array.isArray(res.json)) throw new Error("the answer was not a list");
          const rawItems = res.json as RawListItem[];
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
              hands: extractNeedsDrewHands(raw.body),
            });
          }
          // A page that is not full is the last. A full one is followed
          // by another; if the cap on pages is reached with the last
          // page still full, there may be more, and the count is a floor.
          if (rawItems.length < PER_PAGE) break;
          if (page === MAX_PAGES) status.truncated = true;
          page++;
        }
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

/**
 * Generalised question types (#14149 §1). Absent means "radio", the only
 * shape that existed before, so every stored question keeps its meaning.
 * Every type keeps the custom free-text path (`freeText`).
 */
export const NEEDS_DREW_ANSWER_TYPES = ["radio", "checkbox", "text"] as const;
export type NeedsDrewAnswerType = (typeof NEEDS_DREW_ANSWER_TYPES)[number];

/**
 * How a checkbox answer to a broker-sourced question becomes one boolean
 * (#14149 §2). Recorded on the question at raise, never inferred at apply.
 * - unanimous (default): every ticked option maps to the same boolean.
 * - any-deny: one ticked deny-mapped option denies; otherwise approve.
 * - all-approve: approve only when every approve-mapped option is ticked
 *   and no deny-mapped option is; otherwise deny.
 */
export const NEEDS_DREW_RESOLUTION_RULES = ["unanimous", "any-deny", "all-approve"] as const;
export type NeedsDrewResolutionRule = (typeof NEEDS_DREW_RESOLUTION_RULES)[number];

/** A standard option set referenced at raise (e.g. approve-deny v1). The
 * question's `options` stay the frozen copy, so set edits never rewrite it. */
export interface NeedsDrewOptionSetRef {
  setId: string;
  version: number;
}

export interface NeedsDrewAnswer {
  optionId?: string;
  /** Checkbox selection (#14149). `optionId` stays the radio field. */
  optionIds?: string[];
  freeText?: string;
  /** The question's type when this answer was recorded. */
  answerType?: NeedsDrewAnswerType;
  at: string;
  via: string;
}

export interface NeedsDrewBrokerMapping {
  source: "fleet-actions-exec";
  key: string;
  handle: string;
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
  /** Present only when a broker-raised Actions-exec question has a valid key. */
  brokerMapping?: NeedsDrewBrokerMapping;
  /** #14149: absent = "radio". */
  answerType?: NeedsDrewAnswerType;
  optionSet?: NeedsDrewOptionSetRef;
  /** #14149: checkbox rule for broker resolution; absent = "unanimous". */
  resolution?: NeedsDrewResolutionRule;
  /** #14149: option id -> approve boolean, frozen at raise for
   * broker-sourced questions so resolution never reads live set data. */
  resolutionMap?: Record<string, boolean>;
  answer?: NeedsDrewAnswer;
}

export interface NeedsDrewInput {
  ledgerId: string;
  question: string;
  context?: string;
  options: NeedsDrewOption[];
  links: string[];
  answerType?: NeedsDrewAnswerType;
  optionSet?: NeedsDrewOptionSetRef;
  resolution?: NeedsDrewResolutionRule;
  resolutionMap?: Record<string, boolean>;
}

export const NEEDS_DREW_OPEN_MAX_AGE_MS = 7 * 24 * 60 * 60_000;

/** Default question-store path (mirrors src/config.ts needsDrewFile). */
export function defaultNeedsDrewFile(env: Record<string, string | undefined> = process.env): string {
  return needsUserStatePath(env, () => join(requireHome("PM_AGG_NEEDS_USER_FILE"), ".local", "state", "pm-aggregator"));
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
  const ids = new Set(options.map((o) => o.id));
  let answerType: NeedsDrewAnswerType | undefined;
  if (b.answerType !== undefined) {
    if (!NEEDS_DREW_ANSWER_TYPES.includes(b.answerType as NeedsDrewAnswerType)) {
      return { ok: false, error: `needsDrew.answerType must be one of ${NEEDS_DREW_ANSWER_TYPES.join(", ")}` };
    }
    answerType = b.answerType as NeedsDrewAnswerType;
  }
  let optionSet: NeedsDrewOptionSetRef | undefined;
  if (b.optionSet !== undefined) {
    const os = b.optionSet as Record<string, unknown> | null;
    const setId = os && typeof os === "object" && typeof os.setId === "string" ? os.setId.trim() : "";
    const version = os && typeof os === "object" ? os.version : undefined;
    if (!setId || typeof version !== "number" || !Number.isInteger(version) || version < 1) {
      return { ok: false, error: "needsDrew.optionSet requires setId and an integer version >= 1" };
    }
    optionSet = { setId, version };
  }
  let resolution: NeedsDrewResolutionRule | undefined;
  if (b.resolution !== undefined) {
    if (!NEEDS_DREW_RESOLUTION_RULES.includes(b.resolution as NeedsDrewResolutionRule)) {
      return { ok: false, error: `needsDrew.resolution must be one of ${NEEDS_DREW_RESOLUTION_RULES.join(", ")}` };
    }
    resolution = b.resolution as NeedsDrewResolutionRule;
  }
  let resolutionMap: Record<string, boolean> | undefined;
  if (b.resolutionMap !== undefined) {
    const m = b.resolutionMap;
    if (!m || typeof m !== "object" || Array.isArray(m)) {
      return { ok: false, error: "needsDrew.resolutionMap must be an object" };
    }
    resolutionMap = {};
    for (const [id, v] of Object.entries(m as Record<string, unknown>)) {
      if (!ids.has(id)) return { ok: false, error: `needsDrew.resolutionMap names unknown option "${id}"` };
      if (typeof v !== "boolean") return { ok: false, error: "needsDrew.resolutionMap values must be booleans" };
      resolutionMap[id] = v;
    }
  }
  return {
    ok: true,
    value: {
      ledgerId,
      question,
      ...(typeof b.context === "string" ? { context: b.context } : {}),
      options,
      links,
      ...(answerType ? { answerType } : {}),
      ...(optionSet ? { optionSet } : {}),
      ...(resolution ? { resolution } : {}),
      ...(resolutionMap ? { resolutionMap } : {}),
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
  key?: string,
): Promise<NeedsDrewQuestion> {
  return serializeNeedsDrew(file, async () => {
    const items = await readNeedsDrewItems(file);
    const existing = items.find((i) => i.ledgerId === input.ledgerId);
    if (existing) return existing;
    const brokerMapping = actionsExecMapping(source, key);
    const item: NeedsDrewQuestion = {
      ledgerId: input.ledgerId,
      question: input.question,
      ...(input.context !== undefined ? { context: input.context } : {}),
      options: input.options,
      links: input.links,
      token: randomUUID(),
      raisedAt: new Date().toISOString(),
      source,
      ...(brokerMapping ? { brokerMapping } : {}),
      ...(input.answerType ? { answerType: input.answerType } : {}),
      ...(input.optionSet ? { optionSet: input.optionSet } : {}),
      ...(input.resolution ? { resolution: input.resolution } : {}),
      // #14149 §2: a broker-sourced question freezes its resolution map
      // with its options at raise; later set edits never change it.
      ...(input.resolutionMap
        ? { resolutionMap: { ...input.resolutionMap } }
        : brokerMapping
          ? { resolutionMap: defaultResolutionMap(input.options) }
          : {}),
    };
    items.push(item);
    await writeNeedsDrewItems(file, items);
    return item;
  });
}

function actionsExecMapping(source: string, key: string | undefined): NeedsDrewBrokerMapping | null {
  const normalizedKey = key?.trim() ?? "";
  const prefix = "actions-exec-ext:";
  if (source !== "fleet-actions-exec" || !normalizedKey.startsWith(prefix)) return null;
  // The broker's alert key is actions-exec-ext:<handle>:<ordinal>; the
  // session handle is the middle part, so the trailing ordinal goes.
  const handle = normalizedKey.slice(prefix.length).replace(/:\d+$/, "").trim();
  if (!handle) return null;
  return { source: "fleet-actions-exec", key: normalizedKey, handle };
}

/** Resolve only answered questions with a verified broker mapping. */
export function needsDrewAnswerHandle(question: NeedsDrewQuestion): string | null {
  const mapping = question.brokerMapping;
  if (!question.answer || question.source !== "fleet-actions-exec" || !mapping) return null;
  const parsed = actionsExecMapping(mapping.source, mapping.key);
  if (!parsed || parsed.handle !== mapping.handle) return null;
  return mapping.handle;
}

/** The standard approve-deny mapping: option `approve` -> true, `deny` -> false. */
export function defaultResolutionMap(options: NeedsDrewOption[]): Record<string, boolean> {
  const map: Record<string, boolean> = {};
  for (const o of options) {
    if (o.id === "approve") map[o.id] = true;
    else if (o.id === "deny") map[o.id] = false;
  }
  return map;
}

export type NeedsDrewBrokerResolution =
  | { kind: "resolved"; handle: string; approve: boolean }
  | { kind: "unresolvable"; handle: string; reason: string };

/**
 * #14149 §2: turn a broker-sourced answer into the broker's unchanged
 * `{handle, approve}` envelope, or say why it cannot be. Null when the
 * question is not an answered broker question (the applier ignores it).
 * An unresolvable answer is never dropped: the caller raises the #14043
 * §4 alert naming the handle, the answer and this reason.
 */
export function resolveBrokerAnswer(question: NeedsDrewQuestion): NeedsDrewBrokerResolution | null {
  const handle = needsDrewAnswerHandle(question);
  if (handle === null || !question.answer) return null;
  const answer = question.answer;
  // Questions raised before #14149 carry no frozen map; their implicit
  // convention was the approve/deny option ids.
  const map = question.resolutionMap ?? defaultResolutionMap(question.options);
  const type = question.answerType ?? "radio";
  const unresolvable = (reason: string): NeedsDrewBrokerResolution => ({ kind: "unresolvable", handle, reason });
  if (type === "text") return unresolvable("free-text answer has no approve/deny mapping");
  if (type === "radio") {
    const id = answer.optionId;
    if (id === undefined) return unresolvable("free-text answer has no approve/deny mapping");
    if (!(id in map)) return unresolvable(`option "${id}" has no approve/deny mapping`);
    return { kind: "resolved", handle, approve: map[id]! };
  }
  const ids = answer.optionIds ?? (answer.optionId !== undefined ? [answer.optionId] : []);
  if (ids.length === 0) return unresolvable("empty selection");
  const unmapped = ids.filter((id) => !(id in map));
  if (unmapped.length) return unresolvable(`option "${unmapped[0]}" has no approve/deny mapping`);
  const values = ids.map((id) => map[id]!);
  const rule = question.resolution ?? "unanimous";
  if (rule === "any-deny") return { kind: "resolved", handle, approve: !values.includes(false) };
  if (rule === "all-approve") {
    const picked = new Set(ids);
    const approvals = Object.keys(map).filter((id) => map[id]);
    const approve = !values.includes(false) && approvals.length > 0 && approvals.every((id) => picked.has(id));
    return { kind: "resolved", handle, approve };
  }
  if (values.every((v) => v === values[0])) return { kind: "resolved", handle, approve: values[0]! };
  return unresolvable("mixed selection (approve and deny both ticked)");
}

/**
 * #14149 §3: lossless radio -> checklist conversion. Adds options freely,
 * never renames or removes an option id, and never touches the recorded
 * answer, the broker mapping or the frozen resolution map, so an answer
 * recorded before the conversion still resolves to the same handle.
 */
export function convertToChecklist(question: NeedsDrewQuestion, added: NeedsDrewOption[] = []): NeedsDrewQuestion {
  const ids = new Set(question.options.map((o) => o.id));
  for (const o of added) {
    if (ids.has(o.id)) throw new Error(`option id "${o.id}" already exists; conversion never renames ids`);
    ids.add(o.id);
  }
  return { ...question, options: [...question.options, ...added], answerType: "checkbox" };
}

export async function getNeedsDrew(file: string, ledgerId: string): Promise<NeedsDrewQuestion | null> {
  const items = await readNeedsDrewItems(file);
  return items.find((i) => i.ledgerId === ledgerId) ?? null;
}

/**
 * The alert-incident key a needs-drew question's incident is posted under:
 * the raiser (fleet-queue's needs_drew poster) posts the incident with key
 * `needs-drew:<ledgerId>`, and the answer path closes the incident by this
 * key (#8605). Pinned here so the convention has exactly one spelling.
 */
export function needsDrewIncidentKey(ledgerId: string): string {
  return `needs-drew:${ledgerId}`;
}

/** Record an answer. First answer wins; repeats return current state. */
export async function recordAnswer(
  file: string,
  ledgerId: string,
  answer: { optionId?: string; optionIds?: string[]; freeText?: string; via?: string },
): Promise<NeedsDrewQuestion | null> {
  return serializeNeedsDrew(file, async () => {
    const items = await readNeedsDrewItems(file);
    const item = items.find((i) => i.ledgerId === ledgerId);
    if (!item) return null;
    if (item.answer) return item;
    const optionIds = answer.optionIds ? [...new Set(answer.optionIds)] : undefined;
    item.answer = {
      ...(answer.optionId !== undefined ? { optionId: answer.optionId } : {}),
      ...(optionIds !== undefined ? { optionIds } : {}),
      ...(answer.freeText !== undefined ? { freeText: answer.freeText } : {}),
      ...(item.answerType !== undefined ? { answerType: item.answerType } : {}),
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
