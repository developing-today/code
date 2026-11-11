// Decision SETS (#8766, extending the #8568 notifications Phase 1
// machinery): Drew answers batches of pending decisions on one page
// instead of one push per question. A set mirrors a group of items from
// the Muse-side decisions ledger (pending-questions.json) — the ledger
// stays the one source of truth; this file-backed store is the
// aggregator's presentation/working copy, exactly like the needs-drew
// question store in needs-drew.ts. Resolutions recorded here are
// published into fleet-feed.json's top-level `decisionResolutions`
// array by scripts/collect-fleet-feed.ts, and the dropped-thread
// catcher applies them to the ledger per question.
//
// Model: a set has items (one per ledger item); an item has questions
// (the ledger item's questions, in order). A question's `qid` is
// `<ledgerId>#<1-based index>` so a resolution maps back to the ledger
// question by index. Each question resolves at most once (first
// resolution wins, mirroring recordAnswer in needs-drew.ts):
//   - answered: Drew picked an option and/or wrote free text.
//   - deferred: Drew sent it back to the orchestrator, with a lean:
//     "none" (plain defer), "skeptical" (leaning no), "approve"
//     (leaning yes).
// The set's per-set token (minted once, at raise) is the authority for
// the decisions page (/decisions/<setId>?t=<token>) and the submit /
// defer endpoints — possession of the set push is the authority, the
// same trust model as the #8568 per-question answer token.
import { randomUUID } from "node:crypto";
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join } from "node:path";

export type DeferLean = "none" | "skeptical" | "approve";
export const DEFER_LEANS: readonly DeferLean[] = ["none", "skeptical", "approve"];

export function isDeferLean(value: unknown): value is DeferLean {
  return typeof value === "string" && (DEFER_LEANS as readonly string[]).includes(value);
}

export interface DecisionOption {
  id: string;
  label: string;
}

export interface DecisionResolution {
  kind: "answered" | "deferred";
  /** Answered only: the chosen option's id and its label at record time. */
  optionId?: string;
  optionLabel?: string;
  /** Answered only: Drew's own words, when he wrote any. */
  freeText?: string;
  /** Deferred only: the lean Drew recorded with the defer. */
  lean?: DeferLean;
  at: string;
  via: string;
}

export interface DecisionQuestion {
  /** `<ledgerId>#<1-based index>` — stable across the set's life. */
  qid: string;
  question: string;
  context?: string;
  options: DecisionOption[];
  resolution?: DecisionResolution;
}

export interface DecisionItem {
  /** The decisions-ledger item id this mirrors. */
  ledgerId: string;
  title: string;
  /** Page grouping (e.g. "Goals census", "Issue rulings"). */
  group: string;
  links: string[];
  questions: DecisionQuestion[];
}

export interface DecisionSet {
  setId: string;
  title: string;
  /** Per-set page authority; minted at raise, never rotated. */
  token: string;
  createdAt: string;
  source: string;
  items: DecisionItem[];
}

export interface DecisionQuestionInput {
  question: string;
  context?: string;
  options: DecisionOption[];
}

export interface DecisionItemInput {
  ledgerId: string;
  title: string;
  group?: string;
  links: string[];
  questions: DecisionQuestionInput[];
}

export interface DecisionSetInput {
  setId: string;
  title: string;
  items: DecisionItemInput[];
}

/** One resolution as published to the fleet feed for the catcher. */
export interface DecisionResolutionRecord {
  setId: string;
  ledgerId: string;
  qid: string;
  /** 1-based index of the question within its ledger item. */
  questionIndex: number;
  kind: "answered" | "deferred";
  optionId?: string;
  optionLabel?: string;
  freeText?: string;
  lean?: DeferLean;
  at: string;
  via: string;
}

export const DECISIONS_OPEN_MAX_AGE_MS = 7 * 24 * 60 * 60_000;

/** Default decision-set store path (mirrors src/config.ts decisionsFile). */
export function defaultDecisionsFile(env: Record<string, string | undefined> = process.env): string {
  const v = env.PM_AGG_DECISIONS_FILE?.trim();
  return v ? v : join(homedir(), ".local", "state", "pm-aggregator", "decisions.json");
}

const SET_ID_RE = /^[A-Za-z0-9][A-Za-z0-9._-]{0,79}$/;

function isHttpUrl(s: string): boolean {
  return /^https?:\/\//.test(s);
}

/** Validate the intake `decisionSet` extension object (#8766 wire contract). */
export function validateDecisionSet(
  value: unknown,
): { ok: true; value: DecisionSetInput } | { ok: false; error: string } {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return { ok: false, error: "decisionSet must be an object" };
  }
  const b = value as Record<string, unknown>;
  const setId = typeof b.setId === "string" ? b.setId.trim() : "";
  if (!SET_ID_RE.test(setId)) return { ok: false, error: "decisionSet.setId required (slug, <= 80 chars)" };
  const title = typeof b.title === "string" ? b.title.trim() : "";
  if (!title) return { ok: false, error: "decisionSet.title required" };
  if (!Array.isArray(b.items) || b.items.length < 1 || b.items.length > 64) {
    return { ok: false, error: "decisionSet.items must be an array of 1-64 entries" };
  }
  const seenLedgerIds = new Set<string>();
  const items: DecisionItemInput[] = [];
  for (const rawItem of b.items) {
    if (!rawItem || typeof rawItem !== "object" || Array.isArray(rawItem)) {
      return { ok: false, error: "decisionSet.items entries must be objects" };
    }
    const it = rawItem as Record<string, unknown>;
    const ledgerId = typeof it.ledgerId === "string" ? it.ledgerId.trim() : "";
    if (!ledgerId) return { ok: false, error: "decisionSet.items entries require ledgerId" };
    if (seenLedgerIds.has(ledgerId)) {
      return { ok: false, error: `decisionSet.items duplicates ledgerId ${ledgerId}` };
    }
    seenLedgerIds.add(ledgerId);
    const itemTitle = typeof it.title === "string" ? it.title.trim() : "";
    if (!itemTitle) return { ok: false, error: `decisionSet item ${ledgerId} requires title` };
    if (it.group !== undefined && typeof it.group !== "string") {
      return { ok: false, error: `decisionSet item ${ledgerId} group must be a string` };
    }
    let links: string[] = [];
    if (it.links !== undefined) {
      if (!Array.isArray(it.links) || it.links.some((l) => typeof l !== "string" || !isHttpUrl(l))) {
        return { ok: false, error: `decisionSet item ${ledgerId} links must be an array of http(s) URLs` };
      }
      links = [...(it.links as string[])];
    }
    if (!Array.isArray(it.questions) || it.questions.length < 1 || it.questions.length > 16) {
      return { ok: false, error: `decisionSet item ${ledgerId} questions must be an array of 1-16 entries` };
    }
    const questions: DecisionQuestionInput[] = [];
    for (const rawQ of it.questions) {
      if (!rawQ || typeof rawQ !== "object" || Array.isArray(rawQ)) {
        return { ok: false, error: `decisionSet item ${ledgerId} questions entries must be objects` };
      }
      const q = rawQ as Record<string, unknown>;
      const question = typeof q.question === "string" ? q.question.trim() : "";
      if (!question) return { ok: false, error: `decisionSet item ${ledgerId} has a question with no text` };
      if (q.context !== undefined && typeof q.context !== "string") {
        return { ok: false, error: `decisionSet item ${ledgerId} question context must be a string` };
      }
      // Options may be empty: a free-text-only question is legitimate
      // (the ledger carries questions with no recorded options).
      if (!Array.isArray(q.options) || q.options.length > 8) {
        return { ok: false, error: `decisionSet item ${ledgerId} question options must be an array of 0-8 entries` };
      }
      const options: DecisionOption[] = [];
      const seenOptionIds = new Set<string>();
      for (const rawO of q.options) {
        if (!rawO || typeof rawO !== "object" || Array.isArray(rawO)) {
          return { ok: false, error: `decisionSet item ${ledgerId} question options entries must be objects` };
        }
        const o = rawO as Record<string, unknown>;
        const id = typeof o.id === "string" ? o.id.trim() : "";
        const label = typeof o.label === "string" ? o.label.trim() : "";
        if (!id || !label) {
          return { ok: false, error: `decisionSet item ${ledgerId} question options entries require id and label` };
        }
        if (seenOptionIds.has(id)) {
          return { ok: false, error: `decisionSet item ${ledgerId} question duplicates option id ${id}` };
        }
        seenOptionIds.add(id);
        options.push({ id, label });
      }
      questions.push({
        question,
        ...(typeof q.context === "string" && q.context.trim() ? { context: q.context } : {}),
        options,
      });
    }
    items.push({
      ledgerId,
      title: itemTitle,
      ...(typeof it.group === "string" && it.group.trim() ? { group: it.group.trim() } : {}),
      links,
      questions,
    });
  }
  return { ok: true, value: { setId, title, items } };
}

interface DecisionsFileShape {
  version: 1;
  sets: DecisionSet[];
}

async function readDecisionSets(file: string): Promise<DecisionSet[]> {
  try {
    const parsed: unknown = JSON.parse(await readFile(file, "utf8"));
    if (parsed && typeof parsed === "object" && Array.isArray((parsed as DecisionsFileShape).sets)) {
      return (parsed as DecisionsFileShape).sets.filter(
        (s): s is DecisionSet =>
          !!s && typeof s === "object" && typeof s.setId === "string" && typeof s.token === "string" && Array.isArray(s.items),
      );
    }
    return [];
  } catch {
    return []; // absent/corrupt store reads as empty; a raise rewrites it
  }
}

let decisionsWriteSeq = 0;
const decisionsChains = new Map<string, Promise<unknown>>();

function serializeDecisions<T>(file: string, fn: () => Promise<T>): Promise<T> {
  const prev = decisionsChains.get(file) ?? Promise.resolve();
  const run = prev.then(fn, fn);
  decisionsChains.set(
    file,
    run.catch(() => {}),
  );
  return run;
}

async function writeDecisionSets(file: string, sets: DecisionSet[]): Promise<void> {
  await mkdir(dirname(file), { recursive: true });
  const tmp = `${file}.tmp-${process.pid}-${++decisionsWriteSeq}`;
  const data: DecisionsFileShape = { version: 1, sets };
  await writeFile(tmp, JSON.stringify(data, null, 2));
  await rename(tmp, file);
}

/** Raise a decision set. Idempotent: an existing setId keeps its token
 * and its original contents (a re-post is a duplicate, not an edit). */
export async function raiseDecisionSet(
  file: string,
  input: DecisionSetInput,
  source = "decisions-ledger",
): Promise<DecisionSet> {
  return serializeDecisions(file, async () => {
    const sets = await readDecisionSets(file);
    const existing = sets.find((s) => s.setId === input.setId);
    if (existing) return existing;
    const set: DecisionSet = {
      setId: input.setId,
      title: input.title,
      token: randomUUID(),
      createdAt: new Date().toISOString(),
      source,
      items: input.items.map((item) => ({
        ledgerId: item.ledgerId,
        title: item.title,
        group: item.group ?? "Decisions",
        links: item.links,
        questions: item.questions.map((q, i) => ({
          qid: `${item.ledgerId}#${i + 1}`,
          question: q.question,
          ...(q.context !== undefined ? { context: q.context } : {}),
          options: q.options,
        })),
      })),
    };
    sets.push(set);
    await writeDecisionSets(file, sets);
    return set;
  });
}

export async function getDecisionSet(file: string, setId: string): Promise<DecisionSet | null> {
  const sets = await readDecisionSets(file);
  return sets.find((s) => s.setId === setId) ?? null;
}

export interface DecisionSubmission {
  ledgerId: string;
  qid: string;
  kind: "answered" | "deferred";
  optionId?: string;
  freeText?: string;
  lean?: DeferLean;
}

export interface SubmissionOutcome {
  qid: string;
  applied: boolean;
  reason?: string;
}

function findQuestion(set: DecisionSet, ledgerId: string, qid: string): DecisionQuestion | null {
  const item = set.items.find((i) => i.ledgerId === ledgerId);
  if (!item) return null;
  return item.questions.find((q) => q.qid === qid) ?? null;
}

/**
 * Record a batch of per-question resolutions (one item's submit, or the
 * page's submit-all). First resolution wins per question; each
 * submission is judged independently and its outcome reported, so one
 * stale or malformed entry never fails the batch.
 */
export async function recordResolutions(
  file: string,
  setId: string,
  submissions: DecisionSubmission[],
  via = "decisions-page",
): Promise<{ set: DecisionSet; outcomes: SubmissionOutcome[] } | null> {
  return serializeDecisions(file, async () => {
    const sets = await readDecisionSets(file);
    const set = sets.find((s) => s.setId === setId);
    if (!set) return null;
    const outcomes: SubmissionOutcome[] = [];
    const now = new Date().toISOString();
    for (const sub of submissions) {
      const q = findQuestion(set, sub.ledgerId, sub.qid);
      if (!q) {
        outcomes.push({ qid: sub.qid, applied: false, reason: "unknown-question" });
        continue;
      }
      if (q.resolution) {
        outcomes.push({ qid: sub.qid, applied: false, reason: "already-resolved" });
        continue;
      }
      if (sub.kind === "deferred") {
        if (!isDeferLean(sub.lean)) {
          outcomes.push({ qid: sub.qid, applied: false, reason: "bad-lean" });
          continue;
        }
        q.resolution = { kind: "deferred", lean: sub.lean, at: now, via };
        outcomes.push({ qid: sub.qid, applied: true });
        continue;
      }
      // kind === "answered"
      const option = sub.optionId !== undefined ? q.options.find((o) => o.id === sub.optionId) : undefined;
      if (sub.optionId !== undefined && !option) {
        outcomes.push({ qid: sub.qid, applied: false, reason: "bad-option" });
        continue;
      }
      const freeText = typeof sub.freeText === "string" ? sub.freeText.trim() : "";
      if (!option && !freeText) {
        outcomes.push({ qid: sub.qid, applied: false, reason: "empty-answer" });
        continue;
      }
      q.resolution = {
        kind: "answered",
        ...(option ? { optionId: option.id, optionLabel: option.label } : {}),
        ...(freeText ? { freeText } : {}),
        at: now,
        via,
      };
      outcomes.push({ qid: sub.qid, applied: true });
    }
    if (outcomes.some((o) => o.applied)) await writeDecisionSets(file, sets);
    return { set, outcomes };
  });
}

/**
 * Defer every still-open question in scope (the page's bulk send-back):
 * either the checked items, or all remaining questions in the set.
 * Already-resolved questions are untouched. Returns the count deferred.
 */
export async function deferOpenQuestions(
  file: string,
  setId: string,
  scope: { allRemaining: true } | { ledgerIds: string[] },
  lean: DeferLean,
  via = "decisions-page",
): Promise<{ set: DecisionSet; deferredCount: number } | null> {
  return serializeDecisions(file, async () => {
    const sets = await readDecisionSets(file);
    const set = sets.find((s) => s.setId === setId);
    if (!set) return null;
    const wanted = "allRemaining" in scope ? null : new Set(scope.ledgerIds);
    const now = new Date().toISOString();
    let deferredCount = 0;
    for (const item of set.items) {
      if (wanted && !wanted.has(item.ledgerId)) continue;
      for (const q of item.questions) {
        if (q.resolution) continue;
        q.resolution = { kind: "deferred", lean, at: now, via };
        deferredCount++;
      }
    }
    if (deferredCount > 0) await writeDecisionSets(file, sets);
    return { set, deferredCount };
  });
}

export interface DecisionSetProgress {
  items: number;
  questions: number;
  openQuestions: number;
  answered: number;
  deferred: number;
  /** Items with at least one open question. */
  openItems: number;
}

export function decisionSetProgress(set: DecisionSet): DecisionSetProgress {
  let questions = 0;
  let openQuestions = 0;
  let answered = 0;
  let deferred = 0;
  let openItems = 0;
  for (const item of set.items) {
    let itemOpen = 0;
    for (const q of item.questions) {
      questions++;
      if (!q.resolution) {
        openQuestions++;
        itemOpen++;
      } else if (q.resolution.kind === "answered") answered++;
      else deferred++;
    }
    if (itemOpen > 0) openItems++;
  }
  return { items: set.items.length, questions, openQuestions, answered, deferred, openItems };
}

export function isDecisionSetFullyResolved(set: DecisionSet): boolean {
  return decisionSetProgress(set).openQuestions === 0;
}

/** The page/submit-response view of what is still open, per item. */
export function decisionSetOpenState(set: DecisionSet): {
  setId: string;
  remainingOpenQuestions: number;
  items: Array<{ ledgerId: string; openQids: string[]; fullyResolved: boolean }>;
} {
  const progress = decisionSetProgress(set);
  return {
    setId: set.setId,
    remainingOpenQuestions: progress.openQuestions,
    items: set.items.map((item) => {
      const openQids = item.questions.filter((q) => !q.resolution).map((q) => q.qid);
      return { ledgerId: item.ledgerId, openQids, fullyResolved: openQids.length === 0 };
    }),
  };
}

/** Open sets (at least one unresolved question, younger than 7 days), oldest first. */
export async function listOpenDecisionSets(file: string, now = Date.now()): Promise<DecisionSet[]> {
  const sets = await readDecisionSets(file);
  return sets
    .filter((s) => !isDecisionSetFullyResolved(s) && now - Date.parse(s.createdAt) < DECISIONS_OPEN_MAX_AGE_MS)
    .sort((a, b) => a.createdAt.localeCompare(b.createdAt));
}

/** Every recorded resolution across all sets, in raise order (the feed's
 * `decisionResolutions` source for the dropped-thread catcher). */
export async function listDecisionResolutions(file: string): Promise<DecisionResolutionRecord[]> {
  const sets = await readDecisionSets(file);
  const out: DecisionResolutionRecord[] = [];
  for (const set of sets) {
    for (const item of set.items) {
      item.questions.forEach((q, i) => {
        if (!q.resolution) return;
        const r = q.resolution;
        out.push({
          setId: set.setId,
          ledgerId: item.ledgerId,
          qid: q.qid,
          questionIndex: i + 1,
          kind: r.kind,
          ...(r.optionId !== undefined ? { optionId: r.optionId } : {}),
          ...(r.optionLabel !== undefined ? { optionLabel: r.optionLabel } : {}),
          ...(r.freeText !== undefined ? { freeText: r.freeText } : {}),
          ...(r.lean !== undefined ? { lean: r.lean } : {}),
          at: r.at,
          via: r.via,
        });
      });
    }
  }
  return out;
}
