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
//   - request-info: Drew asked for more information before deciding,
//     with an optional note saying what is missing. The question leaves
//     his page like a defer, but it is NOT a ruling: downstream (the
//     dropped-thread catcher) marks the ledger question
//     awaiting-enrichment and never closes the item for it; once the
//     orchestrator gathers the information, the question is re-raised
//     in a later set carrying it as `infoProvided`.
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
  kind: "answered" | "deferred" | "request-info";
  /** Answered: the chosen option's id and label at record time.
   * Deferred: Drew's tentative pick, recorded as guidance for the
   * orchestrator — the defer is still the resolution kind. */
  optionId?: string;
  optionLabel?: string;
  /** Answered only: Drew's own words, when he wrote any. */
  freeText?: string;
  /** Deferred only: the lean Drew recorded with the defer. */
  lean?: DeferLean;
  /** Request-info only: what Drew said is missing (optional). */
  note?: string;
  /** Any kind: a remark Drew attached to the resolution — on a defer it
   * is the free text he wrote on the question; it rides back to the
   * ledger with the resolution, it is not part of the ruling itself. */
  comment?: string;
  at: string;
  via: string;
}

export interface DecisionQuestion {
  /** `<ledgerId>#<1-based index>` — stable across the set's life. */
  qid: string;
  question: string;
  context?: string;
  /** Information the orchestrator gathered after Drew requested more
   * info on this question in an earlier set (the re-raise payload);
   * rendered on the page as a marked "information you requested" block. */
  infoProvided?: string;
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
  infoProvided?: string;
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
  kind: "answered" | "deferred" | "request-info";
  optionId?: string;
  optionLabel?: string;
  freeText?: string;
  lean?: DeferLean;
  /** Request-info only: what Drew said is missing (optional). */
  note?: string;
  /** Any kind: Drew's remark attached to the resolution. */
  comment?: string;
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
      if (q.infoProvided !== undefined && typeof q.infoProvided !== "string") {
        return { ok: false, error: `decisionSet item ${ledgerId} question infoProvided must be a string` };
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
        ...(typeof q.infoProvided === "string" && q.infoProvided.trim() ? { infoProvided: q.infoProvided } : {}),
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
          ...(q.infoProvided !== undefined ? { infoProvided: q.infoProvided } : {}),
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
  kind: "answered" | "deferred" | "request-info";
  optionId?: string;
  freeText?: string;
  lean?: DeferLean;
  /** Request-info only: what Drew says is missing (optional). */
  note?: string;
  /** Any kind: a remark to attach to the resolution (see DecisionResolution.comment). */
  comment?: string;
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
        // Guided defer: a tentative option pick may ride along as
        // guidance. It never changes the resolution kind.
        const guide = sub.optionId !== undefined ? q.options.find((o) => o.id === sub.optionId) : undefined;
        if (sub.optionId !== undefined && !guide) {
          outcomes.push({ qid: sub.qid, applied: false, reason: "bad-option" });
          continue;
        }
        const comment = typeof sub.comment === "string" ? sub.comment.trim() : "";
        q.resolution = {
          kind: "deferred",
          lean: sub.lean,
          ...(guide ? { optionId: guide.id, optionLabel: guide.label } : {}),
          ...(comment ? { comment } : {}),
          at: now,
          via,
        };
        outcomes.push({ qid: sub.qid, applied: true });
        continue;
      }
      if (sub.kind === "request-info") {
        // Not a ruling: the note is optional, an empty request still
        // sends the question back for enrichment.
        const note = typeof sub.note === "string" ? sub.note.trim() : "";
        const comment = typeof sub.comment === "string" ? sub.comment.trim() : "";
        q.resolution = {
          kind: "request-info",
          ...(note ? { note } : {}),
          ...(comment ? { comment } : {}),
          at: now,
          via,
        };
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
      const comment = typeof sub.comment === "string" ? sub.comment.trim() : "";
      q.resolution = {
        kind: "answered",
        ...(option ? { optionId: option.id, optionLabel: option.label } : {}),
        ...(freeText ? { freeText } : {}),
        ...(comment ? { comment } : {}),
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
  comment?: string,
): Promise<{ set: DecisionSet; deferredCount: number } | null> {
  return serializeDecisions(file, async () => {
    const sets = await readDecisionSets(file);
    const set = sets.find((s) => s.setId === setId);
    if (!set) return null;
    const wanted = "allRemaining" in scope ? null : new Set(scope.ledgerIds);
    const now = new Date().toISOString();
    const trimmedComment = typeof comment === "string" ? comment.trim() : "";
    let deferredCount = 0;
    for (const item of set.items) {
      if (wanted && !wanted.has(item.ledgerId)) continue;
      for (const q of item.questions) {
        if (q.resolution) continue;
        q.resolution = { kind: "deferred", lean, ...(trimmedComment ? { comment: trimmedComment } : {}), at: now, via };
        deferredCount++;
      }
    }
    if (deferredCount > 0) await writeDecisionSets(file, sets);
    return { set, deferredCount };
  });
}

/**
 * The bulk send-back with filled answers winning (#8766): record the
 * submissions the page gathered from filled-in fields FIRST — an answer
 * Drew typed but had not submitted is recorded as a ruling, never
 * converted into a deferral or lost — then defer every still-open
 * question in scope with the bulk lean (and the bulk comment, when one
 * was written). A per-question defer inside the submissions keeps its
 * own lean and comment; the bulk lean/comment apply only to the
 * questions the bulk action itself defers.
 */
export async function deferWithSubmissions(
  file: string,
  setId: string,
  submissions: DecisionSubmission[],
  scope: { allRemaining: true } | { ledgerIds: string[] },
  lean: DeferLean,
  comment?: string,
  via = "decisions-page",
): Promise<{ set: DecisionSet; outcomes: SubmissionOutcome[]; deferredCount: number } | null> {
  let outcomes: SubmissionOutcome[] = [];
  if (submissions.length > 0) {
    const applied = await recordResolutions(file, setId, submissions, via);
    if (!applied) return null;
    outcomes = applied.outcomes;
  }
  const deferred = await deferOpenQuestions(file, setId, scope, lean, via, comment);
  if (!deferred) return null;
  return { set: deferred.set, outcomes, deferredCount: deferred.deferredCount };
}

/**
 * Request more information on every still-open question in scope (the
 * page's checked-items request-info action): either the checked items,
 * or all remaining questions in the set. Each open question records a
 * request-info resolution carrying the shared note. Already-resolved
 * questions are untouched. Returns the count requested.
 */
export async function requestInfoOpenQuestions(
  file: string,
  setId: string,
  scope: { allRemaining: true } | { ledgerIds: string[] },
  note?: string,
  via = "decisions-page",
): Promise<{ set: DecisionSet; requestedCount: number } | null> {
  return serializeDecisions(file, async () => {
    const sets = await readDecisionSets(file);
    const set = sets.find((s) => s.setId === setId);
    if (!set) return null;
    const wanted = "allRemaining" in scope ? null : new Set(scope.ledgerIds);
    const now = new Date().toISOString();
    const trimmed = typeof note === "string" ? note.trim() : "";
    let requestedCount = 0;
    for (const item of set.items) {
      if (wanted && !wanted.has(item.ledgerId)) continue;
      for (const q of item.questions) {
        if (q.resolution) continue;
        q.resolution = { kind: "request-info", ...(trimmed ? { note: trimmed } : {}), at: now, via };
        requestedCount++;
      }
    }
    if (requestedCount > 0) await writeDecisionSets(file, sets);
    return { set, requestedCount };
  });
}

export interface DecisionSetProgress {
  items: number;
  questions: number;
  openQuestions: number;
  answered: number;
  deferred: number;
  infoRequested: number;
  /** Items with at least one open question. */
  openItems: number;
}

export function decisionSetProgress(set: DecisionSet): DecisionSetProgress {
  let questions = 0;
  let openQuestions = 0;
  let answered = 0;
  let deferred = 0;
  let infoRequested = 0;
  let openItems = 0;
  for (const item of set.items) {
    let itemOpen = 0;
    for (const q of item.questions) {
      questions++;
      if (!q.resolution) {
        openQuestions++;
        itemOpen++;
      } else if (q.resolution.kind === "answered") answered++;
      else if (q.resolution.kind === "deferred") deferred++;
      else infoRequested++;
    }
    if (itemOpen > 0) openItems++;
  }
  return { items: set.items.length, questions, openQuestions, answered, deferred, infoRequested, openItems };
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
          ...(r.note !== undefined ? { note: r.note } : {}),
          ...(r.comment !== undefined ? { comment: r.comment } : {}),
          at: r.at,
          via: r.via,
        });
      });
    }
  }
  return out;
}

// --- Decisions-page draft persistence (#8777) ---------------------------
//
// The 2026-11-11 "front end crashed" incident: pm-aggregator was killed
// mid-session and Drew's typed-but-unsubmitted work existed only in the
// page's form fields — gone the moment the tab died. The page now keeps
// a per-set draft in localStorage: every in-progress answer, comment,
// defer pick, info note and bulk selection is saved as he types and
// restored on load. Drafts for questions that have been recorded (their
// blocks leave the DOM on submit) prune themselves via the
// MutationObserver re-save. Storage is best-effort: any localStorage
// failure (private mode, quota) leaves the page exactly as functional
// as before, just without drafts.

/** localStorage key prefix for one set's decisions-page draft. */
export const DECISIONS_DRAFT_KEY_PREFIX = "decisions-drafts:";

export function decisionsDraftKey(setId: string): string {
  return DECISIONS_DRAFT_KEY_PREFIX + setId;
}

/**
 * The draft-persistence script for the decisions page (JS source, no
 * <script> tags — server.ts interpolates it as the page script's last
 * block, after every handler it complements exists). It works purely
 * off the page's DOM contract: `.q` blocks keyed by data-qid carrying
 * radio inputs named per question, `textarea.ft`, `select.defer`,
 * `textarea.infonote`; item cards `section.item[data-ledger]` with
 * `input.bulk` checkboxes; the bulk bar's #bulkLean and #bulkInfoNote.
 */
export function decisionsDraftScript(setId: string): string {
  const key = JSON.stringify(decisionsDraftKey(setId));
  return `
// --- Draft persistence (#8777): in-progress work survives a crash ---
var DRAFT_KEY = ${key};
function draftSave() {
  try {
    var d = { q: {}, bulk: [], bulkLean: '', bulkNote: '' };
    document.querySelectorAll('.q').forEach(function (block) {
      var entry = {};
      var picked = block.querySelector('input[type=radio]:checked');
      if (picked) entry.opt = picked.value;
      var ft = block.querySelector('textarea.ft').value;
      if (ft) entry.ft = ft;
      var lean = block.querySelector('select.defer').value;
      if (lean) entry.lean = lean;
      var note = block.querySelector('textarea.infonote').value;
      if (note) entry.note = note;
      if (Object.keys(entry).length) d.q[block.getAttribute('data-qid')] = entry;
    });
    document.querySelectorAll('section.item').forEach(function (card) {
      var box = card.querySelector('input.bulk');
      if (box && box.checked) d.bulk.push(card.getAttribute('data-ledger'));
    });
    var bl = document.getElementById('bulkLean');
    if (bl) d.bulkLean = bl.value;
    var bn = document.getElementById('bulkInfoNote');
    if (bn) d.bulkNote = bn.value;
    if (!Object.keys(d.q).length && !d.bulk.length && !d.bulkNote && !d.bulkLean) {
      localStorage.removeItem(DRAFT_KEY);
    } else {
      localStorage.setItem(DRAFT_KEY, JSON.stringify(d));
    }
  } catch (e) { /* storage unavailable: drafts are best-effort */ }
}
function draftRestore() {
  var d;
  try {
    var raw = localStorage.getItem(DRAFT_KEY);
    if (!raw) return;
    d = JSON.parse(raw);
  } catch (e) { return; }
  if (!d || typeof d !== 'object') return;
  var restored = 0;
  document.querySelectorAll('.q').forEach(function (block) {
    var entry = (d.q || {})[block.getAttribute('data-qid')];
    if (!entry) return;
    if (entry.opt) {
      block.querySelectorAll('input[type=radio]').forEach(function (r) {
        if (r.value === entry.opt) { r.checked = true; restored++; }
      });
    }
    if (entry.ft) { block.querySelector('textarea.ft').value = entry.ft; restored++; }
    if (entry.lean) { block.querySelector('select.defer').value = entry.lean; restored++; }
    if (entry.note) { block.querySelector('textarea.infonote').value = entry.note; restored++; }
  });
  (d.bulk || []).forEach(function (ledger) {
    document.querySelectorAll('section.item').forEach(function (card) {
      var box = card.querySelector('input.bulk');
      if (box && card.getAttribute('data-ledger') === ledger) box.checked = true;
    });
  });
  if (d.bulkLean) { var bl2 = document.getElementById('bulkLean'); if (bl2) bl2.value = d.bulkLean; }
  if (d.bulkNote) { var bn2 = document.getElementById('bulkInfoNote'); if (bn2) { bn2.value = d.bulkNote; restored++; } }
  if (restored) status('Restored your in-progress work from before the interruption — nothing you typed was lost.');
}
document.addEventListener('input', draftSave);
document.addEventListener('change', draftSave);
new MutationObserver(draftSave).observe(document.querySelector('main'), { childList: true, subtree: true });
draftRestore();
draftSave();
`;
}
