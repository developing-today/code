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
  /** Hint chips rendered next to the option (simpler, safer,
   * distributed, portable, performance, recommended, ...). */
  tags?: string[];
  /** For a combination option ("A+B"): the ids of the component
   * options it combines. Components stay selectable on their own. */
  combinationOf?: string[];
}

/** Answer mode (#9335 v2): "single" renders radios, "multi" renders
 * checkboxes. Absent means "single" (every v1 set). */
export type DecisionAnswerMode = "single" | "multi";

export function isDecisionAnswerMode(value: unknown): value is DecisionAnswerMode {
  return value === "single" || value === "multi";
}

/** The mode a question renders/submits with; v1 questions are single. */
export function effectiveQuestionMode(q: DecisionQuestion): DecisionAnswerMode {
  return q.mode === "multi" ? "multi" : "single";
}

/** One kept wording version of a question (#9335 v2 versioning). */
export interface DecisionQuestionVersion {
  version: number;
  question: string;
  context?: string;
  description?: string;
  editedAt: string;
  note?: string;
}

export interface DecisionResolution {
  kind: "answered" | "deferred" | "request-info";
  /** Answered: the chosen option's id and label at record time.
   * Deferred: Drew's tentative pick, recorded as guidance for the
   * orchestrator — the defer is still the resolution kind. */
  optionId?: string;
  optionLabel?: string;
  /** Answered (multi mode, or a guided defer with several picks): all
   * chosen option ids/labels. optionId/optionLabel carry the first. */
  optionIds?: string[];
  optionLabels?: string[];
  /** Answered only: Drew's own words, when he wrote any. */
  freeText?: string;
  /** Answered only (#9335 v2): the "my own answer" free-text field.
   * Mirrored into freeText when a submit carries only customAnswer,
   * so v1 feed consumers keep seeing the words. */
  customAnswer?: string;
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
  /** Rich detail (#9335 v2): background, why it matters, and the
   * consequences of each direction — beyond the one-line `context`.
   * The v2 raise contract sends `description` (input alias: `detail`). */
  description?: string;
  /** Question-level hint chips (see DecisionOption.tags). */
  tags?: string[];
  /** Answer mode; absent = "single" (v1). */
  mode?: DecisionAnswerMode;
  /** Raiser-recommended default selection (#9335 v2): the page renders
   * these preselected and offers one-click "submit defaults". They are
   * NOT a resolution until Drew submits. */
  recommendedOptionIds?: string[];
  /** Explicit ordering within the item (lower first); absent keeps
   * raise order. */
  order?: number;
  /** Per-question grouping override; absent = the item's group. */
  group?: string;
  /** ALL wording versions, oldest first (#9335 v2). */
  versions?: DecisionQuestionVersion[];
  /** Pointer into versions: the current version number. */
  currentVersion?: number;
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
  /** Explicit ordering within the set (lower first); absent keeps
   * raise order. */
  order?: number;
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
  description?: string;
  infoProvided?: string;
  options: DecisionOption[];
  mode?: DecisionAnswerMode;
  tags?: string[];
  recommendedOptionIds?: string[];
  order?: number;
  group?: string;
  versions?: DecisionQuestionVersion[];
}

export interface DecisionItemInput {
  ledgerId: string;
  title: string;
  group?: string;
  order?: number;
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
  optionIds?: string[];
  optionLabels?: string[];
  freeText?: string;
  customAnswer?: string;
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

const TAG_MAX = 12;
const TAG_LEN_MAX = 40;

function parseTagList(raw: unknown): string[] | null {
  if (!Array.isArray(raw) || raw.length > TAG_MAX) return null;
  const out: string[] = [];
  for (const t of raw) {
    if (typeof t !== "string" || !t.trim() || t.length > TAG_LEN_MAX) return null;
    const v = t.trim();
    if (!out.includes(v)) out.push(v);
  }
  return out;
}

function sanitizeQuestionVersions(raw: unknown): DecisionQuestionVersion[] | null {
  if (!Array.isArray(raw) || raw.length < 1 || raw.length > 32) return null;
  const out: DecisionQuestionVersion[] = [];
  for (const rv of raw) {
    if (!rv || typeof rv !== "object" || Array.isArray(rv)) return null;
    const v = rv as Record<string, unknown>;
    if (typeof v.version !== "number" || !Number.isFinite(v.version)) return null;
    if (typeof v.question !== "string" || !v.question.trim()) return null;
    if (typeof v.editedAt !== "string" || !v.editedAt) return null;
    out.push({
      version: v.version,
      question: v.question,
      ...(typeof v.context === "string" ? { context: v.context } : {}),
      ...(typeof v.description === "string" ? { description: v.description } : {}),
      editedAt: v.editedAt,
      ...(typeof v.note === "string" ? { note: v.note } : {}),
    });
  }
  return out;
}

/**
 * Submit-API preparation (#9335 v2): a submit may omit setId — a UUID
 * is minted for it. Accepts either the bare set object or the
 * `{ decisionSet: ... }` envelope. Returns the (possibly new) object,
 * ready for validateDecisionSet; validateDecisionSet stays strict.
 */
export function withGeneratedSetId(raw: unknown): unknown {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return raw;
  const b = raw as Record<string, unknown>;
  const inner =
    b.decisionSet && typeof b.decisionSet === "object" && !Array.isArray(b.decisionSet)
      ? (b.decisionSet as Record<string, unknown>)
      : b;
  if (typeof inner.setId === "string" && inner.setId.trim()) return raw;
  const filled = { ...inner, setId: randomUUID() };
  return inner === b ? filled : { ...b, decisionSet: filled };
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
    if (it.order !== undefined && (typeof it.order !== "number" || !Number.isFinite(it.order))) {
      return { ok: false, error: `decisionSet item ${ledgerId} order must be a number` };
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
      if (!Array.isArray(q.options) || q.options.length > 12) {
        return { ok: false, error: `decisionSet item ${ledgerId} question options must be an array of 0-12 entries` };
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
        let optTags: string[] | undefined;
        if (o.tags !== undefined) {
          const parsed = parseTagList(o.tags);
          if (!parsed) {
            return {
              ok: false,
              error: `decisionSet item ${ledgerId} question option ${id} tags must be an array of up to ${TAG_MAX} short strings`,
            };
          }
          optTags = parsed;
        }
        let combinationOf: string[] | undefined;
        if (o.combinationOf !== undefined) {
          if (
            !Array.isArray(o.combinationOf) ||
            o.combinationOf.length < 2 ||
            o.combinationOf.some((c) => typeof c !== "string" || !c.trim())
          ) {
            return {
              ok: false,
              error: `decisionSet item ${ledgerId} question option ${id} combinationOf must be an array of 2+ option ids`,
            };
          }
          combinationOf = [...(o.combinationOf as string[])];
        }
        options.push({ id, label, ...(optTags ? { tags: optTags } : {}), ...(combinationOf ? { combinationOf } : {}) });
      }
      // Combination options must reference real sibling options (#9335).
      for (const o of options) {
        for (const cid of o.combinationOf ?? []) {
          if (cid === o.id || !seenOptionIds.has(cid)) {
            return {
              ok: false,
              error: `decisionSet item ${ledgerId} question option ${o.id} combinationOf references unknown option ${cid}`,
            };
          }
        }
      }
      if (q.description !== undefined && typeof q.description !== "string") {
        return { ok: false, error: `decisionSet item ${ledgerId} question description must be a string` };
      }
      if (q.detail !== undefined && typeof q.detail !== "string") {
        return { ok: false, error: `decisionSet item ${ledgerId} question detail must be a string` };
      }
      const description =
        typeof q.description === "string" && q.description.trim()
          ? q.description
          : typeof q.detail === "string" && q.detail.trim()
            ? q.detail
            : undefined;
      let mode: DecisionAnswerMode | undefined;
      if (q.mode !== undefined) {
        if (!isDecisionAnswerMode(q.mode)) {
          return { ok: false, error: `decisionSet item ${ledgerId} question mode must be single|multi` };
        }
        mode = q.mode;
      }
      let qTags: string[] | undefined;
      if (q.tags !== undefined) {
        const parsed = parseTagList(q.tags);
        if (!parsed) {
          return {
            ok: false,
            error: `decisionSet item ${ledgerId} question tags must be an array of up to ${TAG_MAX} short strings`,
          };
        }
        qTags = parsed;
      }
      let recommendedOptionIds: string[] | undefined;
      if (q.recommendedOptionIds !== undefined) {
        if (
          !Array.isArray(q.recommendedOptionIds) ||
          q.recommendedOptionIds.some((r) => typeof r !== "string" || !seenOptionIds.has(r))
        ) {
          return {
            ok: false,
            error: `decisionSet item ${ledgerId} question recommendedOptionIds must be ids of its options`,
          };
        }
        if (mode !== "multi" && q.recommendedOptionIds.length > 1) {
          return {
            ok: false,
            error: `decisionSet item ${ledgerId} question recommendedOptionIds allows one id unless mode is multi`,
          };
        }
        recommendedOptionIds = [...(q.recommendedOptionIds as string[])];
      }
      if (q.order !== undefined && (typeof q.order !== "number" || !Number.isFinite(q.order))) {
        return { ok: false, error: `decisionSet item ${ledgerId} question order must be a number` };
      }
      if (q.group !== undefined && typeof q.group !== "string") {
        return { ok: false, error: `decisionSet item ${ledgerId} question group must be a string` };
      }
      let versions: DecisionQuestionVersion[] | undefined;
      if (q.versions !== undefined) {
        const parsed = sanitizeQuestionVersions(q.versions);
        if (!parsed) {
          return {
            ok: false,
            error: `decisionSet item ${ledgerId} question versions must be an array of {version, question, editedAt}`,
          };
        }
        versions = parsed;
      }
      questions.push({
        question,
        ...(typeof q.context === "string" && q.context.trim() ? { context: q.context } : {}),
        ...(description ? { description } : {}),
        ...(typeof q.infoProvided === "string" && q.infoProvided.trim() ? { infoProvided: q.infoProvided } : {}),
        options,
        ...(mode ? { mode } : {}),
        ...(qTags ? { tags: qTags } : {}),
        ...(recommendedOptionIds ? { recommendedOptionIds } : {}),
        ...(typeof q.order === "number" ? { order: q.order } : {}),
        ...(typeof q.group === "string" && q.group.trim() ? { group: q.group.trim() } : {}),
        ...(versions ? { versions } : {}),
      });
    }
    items.push({
      ledgerId,
      title: itemTitle,
      ...(typeof it.group === "string" && it.group.trim() ? { group: it.group.trim() } : {}),
      ...(typeof it.order === "number" ? { order: it.order } : {}),
      links,
      questions,
    });
  }
  return { ok: true, value: { setId, title, items } };
}

interface DecisionsFileShape {
  version: 1;
  sets: DecisionSet[];
  /** Server-side drafts, keyed by setId. A separate top-level map so
   * drafts can never be mistaken for resolutions; store files written
   * before the drafts layer simply lack the key. */
  drafts?: Record<string, DecisionSetDrafts>;
}

interface DecisionsData {
  sets: DecisionSet[];
  drafts: Record<string, DecisionSetDrafts>;
}

function sanitizeSetDrafts(raw: unknown): DecisionSetDrafts | null {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return null;
  const r = raw as Record<string, unknown>;
  const questions: Record<string, DecisionQuestionDraft> = {};
  if (r.questions && typeof r.questions === "object" && !Array.isArray(r.questions)) {
    for (const [qid, qRaw] of Object.entries(r.questions as Record<string, unknown>)) {
      const entry = sanitizeDraftEntry(qRaw);
      if (entry) questions[qid] = entry;
    }
  }
  return {
    questions,
    bulk: Array.isArray(r.bulk) ? r.bulk.filter((x): x is string => typeof x === "string") : [],
    bulkLean: typeof r.bulkLean === "string" ? r.bulkLean : "",
    bulkNote: typeof r.bulkNote === "string" ? r.bulkNote : "",
    updatedAt: typeof r.updatedAt === "string" ? r.updatedAt : "",
  };
}

async function readDecisionsData(file: string): Promise<DecisionsData> {
  try {
    const parsed: unknown = JSON.parse(await readFile(file, "utf8"));
    if (parsed && typeof parsed === "object" && Array.isArray((parsed as DecisionsFileShape).sets)) {
      const shape = parsed as DecisionsFileShape;
      const drafts: Record<string, DecisionSetDrafts> = {};
      if (shape.drafts && typeof shape.drafts === "object") {
        for (const [setId, raw] of Object.entries(shape.drafts)) {
          const d = sanitizeSetDrafts(raw);
          if (d) drafts[setId] = d;
        }
      }
      return {
        sets: shape.sets.filter(
          (s): s is DecisionSet =>
            !!s &&
            typeof s === "object" &&
            typeof s.setId === "string" &&
            typeof s.token === "string" &&
            Array.isArray(s.items),
        ),
        drafts,
      };
    }
    return { sets: [], drafts: {} };
  } catch {
    return { sets: [], drafts: {} }; // absent/corrupt store reads as empty; a raise rewrites it
  }
}

async function readDecisionSets(file: string): Promise<DecisionSet[]> {
  return (await readDecisionsData(file)).sets;
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

async function writeDecisionsData(file: string, data: DecisionsData): Promise<void> {
  await mkdir(dirname(file), { recursive: true });
  const tmp = `${file}.tmp-${process.pid}-${++decisionsWriteSeq}`;
  const shape: DecisionsFileShape = { version: 1, sets: data.sets, drafts: data.drafts };
  await writeFile(tmp, JSON.stringify(shape, null, 2));
  await rename(tmp, file);
}

/** Delete stored drafts for questions that now carry a resolution.
 * Returns how many were removed. */
function pruneResolvedDrafts(set: DecisionSet, drafts: DecisionSetDrafts | undefined): number {
  if (!drafts) return 0;
  let removed = 0;
  for (const item of set.items) {
    for (const q of item.questions) {
      if (q.resolution && drafts.questions[q.qid]) {
        delete drafts.questions[q.qid];
        removed++;
      }
    }
  }
  return removed;
}

/** The sets write shared by every existing writer (raise / record /
 * defer / request-info): it preserves all sets' server drafts and
 * prunes the drafts of any question the write just resolved, so a
 * draft can never outlive its question on the server layer. Callers
 * are inside serializeDecisions, so the re-read here is consistent. */
async function writeDecisionSets(file: string, sets: DecisionSet[]): Promise<void> {
  const { drafts } = await readDecisionsData(file);
  for (const set of sets) pruneResolvedDrafts(set, drafts[set.setId]);
  await writeDecisionsData(file, { sets, drafts });
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
    const createdAt = new Date().toISOString();
    const set: DecisionSet = {
      setId: input.setId,
      title: input.title,
      token: randomUUID(),
      createdAt,
      source,
      items: input.items.map((item) => ({
        ledgerId: item.ledgerId,
        title: item.title,
        group: item.group ?? "Decisions",
        ...(item.order !== undefined ? { order: item.order } : {}),
        links: item.links,
        questions: item.questions.map((q, i) => {
          // Versioning backfill (#9335): every raised question starts
          // its version history at v1 (or keeps the history the raiser
          // supplied); the current pointer is the newest version.
          const versions: DecisionQuestionVersion[] =
            q.versions && q.versions.length
              ? q.versions
              : [
                  {
                    version: 1,
                    question: q.question,
                    ...(q.context !== undefined ? { context: q.context } : {}),
                    ...(q.description !== undefined ? { description: q.description } : {}),
                    editedAt: createdAt,
                    note: "initial version",
                  },
                ];
          return {
            qid: `${item.ledgerId}#${i + 1}`,
            question: q.question,
            ...(q.context !== undefined ? { context: q.context } : {}),
            ...(q.description !== undefined ? { description: q.description } : {}),
            ...(q.infoProvided !== undefined ? { infoProvided: q.infoProvided } : {}),
            ...(q.mode !== undefined ? { mode: q.mode } : {}),
            ...(q.tags !== undefined ? { tags: q.tags } : {}),
            ...(q.recommendedOptionIds !== undefined ? { recommendedOptionIds: q.recommendedOptionIds } : {}),
            ...(q.order !== undefined ? { order: q.order } : {}),
            ...(q.group !== undefined ? { group: q.group } : {}),
            options: q.options,
            versions,
            currentVersion: Math.max(...versions.map((v) => v.version)),
          };
        }),
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

/**
 * Append a new wording version to a question (#9335 v2 versioning):
 * the new text becomes current, every prior version stays on the
 * record. A question raised before versioning existed is first
 * backfilled with its current wording as v1. Returns the updated
 * question, or null for an unknown set/question.
 */
export async function appendQuestionVersion(
  file: string,
  setId: string,
  ledgerId: string,
  qid: string,
  update: { question: string; context?: string; description?: string; note?: string },
): Promise<DecisionQuestion | null> {
  if (!update.question?.trim()) return null;
  return serializeDecisions(file, async () => {
    const sets = await readDecisionSets(file);
    const set = sets.find((x) => x.setId === setId);
    if (!set) return null;
    const q = findQuestion(set, ledgerId, qid);
    if (!q) return null;
    const prev: DecisionQuestionVersion[] =
      q.versions && q.versions.length
        ? q.versions
        : [
            {
              version: 1,
              question: q.question,
              ...(q.context !== undefined ? { context: q.context } : {}),
              ...(q.description !== undefined ? { description: q.description } : {}),
              editedAt: set.createdAt,
              note: "initial version",
            },
          ];
    const nextVersion = Math.max(...prev.map((v) => v.version), q.currentVersion ?? 0) + 1;
    const entry: DecisionQuestionVersion = {
      version: nextVersion,
      question: update.question,
      ...(update.context !== undefined
        ? { context: update.context }
        : q.context !== undefined
          ? { context: q.context }
          : {}),
      ...(update.description !== undefined
        ? { description: update.description }
        : q.description !== undefined
          ? { description: q.description }
          : {}),
      editedAt: new Date().toISOString(),
      ...(update.note ? { note: update.note } : {}),
    };
    q.versions = [...prev, entry];
    q.currentVersion = nextVersion;
    q.question = update.question;
    if (update.context !== undefined) q.context = update.context;
    if (update.description !== undefined) q.description = update.description;
    await writeDecisionSets(file, sets);
    return q;
  });
}

// --- Decision drafts (server layer, #8766 both-layers upgrade) --------
//
// Drafts are Drew's in-progress form state, mirrored server-side so a
// draft survives the device or browser it was typed on. They live in
// the same store file as the sets, under a separate top-level `drafts`
// map, and are NOT resolutions: nothing here ever reaches
// listDecisionResolutions or the fleet feed. A question's draft is
// deleted the moment the question resolves (any kind), and an upsert
// for an already-resolved or unknown question is a no-op, so a stale
// client can never resurrect a settled question. Per question, the
// newest updatedAt wins — a slow tab's late push cannot clobber a
// newer draft of the same question.

export interface DecisionQuestionDraft {
  /** Selected option id (the page radio's value). */
  opt?: string;
  /** Selected option ids for a multi-mode question's checkboxes. */
  opts?: string[];
  /** The question's text box: the answer text, or the comment that
   * rides with a defer — the page uses one field for both. */
  ft?: string;
  /** Defer pick: a DeferLean value when the defer select is set. */
  lean?: string;
  /** Request-info note text. */
  note?: string;
  /** Client-stamped last-edit time (ISO); newest wins per question. */
  updatedAt: string;
}

export interface DecisionSetDrafts {
  questions: Record<string, DecisionQuestionDraft>;
  /** Ledger ids checked for a bulk action. */
  bulk: string[];
  bulkLean: string;
  bulkNote: string;
  /** Last write to any part of this set's drafts (ISO). */
  updatedAt: string;
}

/** Wire shape for a draft upsert: per-question entries; `null` deletes. */
export interface DecisionDraftUpsert {
  questions: Record<string, DecisionQuestionDraft | null>;
  bulk?: string[];
  bulkLean?: string;
  bulkNote?: string;
}

const DRAFT_TEXT_MAX = 16384;

function sanitizeDraftEntry(raw: unknown): DecisionQuestionDraft | null {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return null;
  const r = raw as Record<string, unknown>;
  const entry: DecisionQuestionDraft = { updatedAt: "" };
  if (typeof r.opt === "string" && r.opt) entry.opt = r.opt.slice(0, 200);
  if (Array.isArray(r.opts)) {
    const ids = r.opts
      .filter((x): x is string => typeof x === "string" && !!x)
      .slice(0, 12)
      .map((x) => x.slice(0, 200));
    if (ids.length) entry.opts = [...new Set(ids)];
  }
  if (typeof r.ft === "string" && r.ft) entry.ft = r.ft.slice(0, DRAFT_TEXT_MAX);
  if (isDeferLean(r.lean)) entry.lean = r.lean;
  if (typeof r.note === "string" && r.note) entry.note = r.note.slice(0, DRAFT_TEXT_MAX);
  const at = typeof r.updatedAt === "string" ? Date.parse(r.updatedAt) : NaN;
  entry.updatedAt = Number.isFinite(at) ? new Date(at).toISOString() : "";
  if (!entry.opt && !entry.opts && !entry.ft && !entry.lean && !entry.note) return null;
  return entry;
}

/** Validate the POST /api/decisions/<setId>/drafts payload body. */
export function validateDecisionDraftUpsert(
  value: unknown,
): { ok: true; value: DecisionDraftUpsert } | { ok: false; error: string } {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return { ok: false, error: "drafts must be an object" };
  }
  const b = value as Record<string, unknown>;
  if (!b.questions || typeof b.questions !== "object" || Array.isArray(b.questions)) {
    return { ok: false, error: "drafts.questions must be an object keyed by qid" };
  }
  const qEntries = Object.entries(b.questions as Record<string, unknown>);
  if (qEntries.length > 64) return { ok: false, error: "drafts.questions has too many entries (max 64)" };
  const questions: Record<string, DecisionQuestionDraft | null> = {};
  for (const [qid, raw] of qEntries) {
    if (raw === null) {
      questions[qid] = null;
      continue;
    }
    const entry = sanitizeDraftEntry(raw);
    if (!entry) return { ok: false, error: `draft for ${qid} has no usable fields` };
    questions[qid] = entry;
  }
  const out: DecisionDraftUpsert = { questions };
  if (b.bulk !== undefined) {
    if (!Array.isArray(b.bulk) || b.bulk.length > 64 || (b.bulk as unknown[]).some((x) => typeof x !== "string")) {
      return { ok: false, error: "drafts.bulk must be an array of up to 64 strings" };
    }
    out.bulk = [...(b.bulk as string[])];
  }
  if (b.bulkLean !== undefined) {
    if (typeof b.bulkLean !== "string" || (b.bulkLean !== "" && !isDeferLean(b.bulkLean))) {
      return { ok: false, error: "drafts.bulkLean must be '' or a defer lean" };
    }
    out.bulkLean = b.bulkLean;
  }
  if (b.bulkNote !== undefined) {
    if (typeof b.bulkNote !== "string") return { ok: false, error: "drafts.bulkNote must be a string" };
    out.bulkNote = b.bulkNote.slice(0, DRAFT_TEXT_MAX);
  }
  return { ok: true, value: out };
}

export interface DecisionSubmission {
  ledgerId: string;
  qid: string;
  kind: "answered" | "deferred" | "request-info";
  optionId?: string;
  /** Multi mode (#9335 v2): every chosen option id. */
  optionIds?: string[];
  freeText?: string;
  /** The "my own answer" field (#9335 v2); an alias source for the
   * answer text, recorded on the resolution as customAnswer. */
  customAnswer?: string;
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
        // Guided defer: tentative option pick(s) may ride along as
        // guidance. They never change the resolution kind.
        const guideIds = [
          ...(sub.optionId ? [sub.optionId] : []),
          ...(sub.optionIds ?? []).filter((id) => id !== sub.optionId),
        ];
        const guides = guideIds.map((id) => q.options.find((o) => o.id === id));
        if (guides.some((g) => !g)) {
          outcomes.push({ qid: sub.qid, applied: false, reason: "bad-option" });
          continue;
        }
        const guide = guides[0];
        const comment = typeof sub.comment === "string" ? sub.comment.trim() : "";
        q.resolution = {
          kind: "deferred",
          lean: sub.lean,
          ...(guide ? { optionId: guide.id, optionLabel: guide.label } : {}),
          ...(guides.length > 1 ? { optionIds: guideIds, optionLabels: guides.map((g) => g!.label) } : {}),
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
      // kind === "answered" (#9335 v2: multi-select + custom answer)
      const pickedIds = [
        ...(sub.optionId ? [sub.optionId] : []),
        ...(sub.optionIds ?? []).filter((id) => id !== sub.optionId),
      ];
      const picked = pickedIds.map((id) => q.options.find((o) => o.id === id));
      if (picked.some((o) => !o)) {
        outcomes.push({ qid: sub.qid, applied: false, reason: "bad-option" });
        continue;
      }
      if (pickedIds.length > 1 && effectiveQuestionMode(q) !== "multi") {
        outcomes.push({ qid: sub.qid, applied: false, reason: "too-many-options" });
        continue;
      }
      const customAnswer = typeof sub.customAnswer === "string" ? sub.customAnswer.trim() : "";
      const freeText = (typeof sub.freeText === "string" ? sub.freeText.trim() : "") || customAnswer;
      if (picked.length === 0 && !freeText) {
        outcomes.push({ qid: sub.qid, applied: false, reason: "empty-answer" });
        continue;
      }
      const first = picked[0];
      const comment = typeof sub.comment === "string" ? sub.comment.trim() : "";
      q.resolution = {
        kind: "answered",
        ...(first ? { optionId: first.id, optionLabel: first.label } : {}),
        ...(picked.length ? { optionIds: pickedIds, optionLabels: picked.map((o) => o!.label) } : {}),
        ...(freeText ? { freeText } : {}),
        ...(customAnswer ? { customAnswer } : {}),
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

/** Every set in the store, in raise order (the #8867 index page's
 * source — unlike listOpenDecisionSets, fully-resolved and older sets
 * are included: the index is the browse-everything view). */
export async function listDecisionSets(file: string): Promise<DecisionSet[]> {
  return readDecisionSets(file);
}

/** One set's row on the decision-sets index (#8867): identity plus the
 * same counts the set page itself reports (decisionSetProgress), so the
 * index can never disagree with the page it links to. Open means "no
 * resolution" — answered, deferred and request-info questions are all
 * resolved for counting, exactly as on the set page. */
export interface DecisionSetSummary {
  setId: string;
  title: string;
  createdAt: string;
  items: number;
  questions: number;
  openQuestions: number;
  answered: number;
  deferred: number;
  infoRequested: number;
  fullyResolved: boolean;
}

export function decisionSetSummary(set: DecisionSet): DecisionSetSummary {
  const p = decisionSetProgress(set);
  return {
    setId: set.setId,
    title: set.title,
    createdAt: set.createdAt,
    items: p.items,
    questions: p.questions,
    openQuestions: p.openQuestions,
    answered: p.answered,
    deferred: p.deferred,
    infoRequested: p.infoRequested,
    fullyResolved: p.openQuestions === 0,
  };
}

/** Index views (#8867): "today" = raised on the current America/Chicago
 * calendar day; "week" = raised in the current America/Chicago calendar
 * week (Monday–Sunday); "all" = everything. */
export type DecisionSetView = "today" | "week" | "all";
export const DECISION_SET_VIEWS: readonly DecisionSetView[] = ["today", "week", "all"];

export function isDecisionSetView(value: unknown): value is DecisionSetView {
  return typeof value === "string" && (DECISION_SET_VIEWS as readonly string[]).includes(value);
}

const CHICAGO_DAY = new Intl.DateTimeFormat("en-US", {
  timeZone: "America/Chicago",
  year: "numeric",
  month: "2-digit",
  day: "2-digit",
});

/** The America/Chicago calendar date of an instant, as a day number
 * (the calendar date read as UTC, in days since the epoch). Comparing
 * day numbers keeps today/week arithmetic free of DST edges. Returns
 * null for an unparseable instant. */
function chicagoDayNumber(ms: number): number | null {
  if (!Number.isFinite(ms)) return null;
  let y = 0;
  let m = 0;
  let d = 0;
  for (const part of CHICAGO_DAY.formatToParts(new Date(ms))) {
    if (part.type === "year") y = Number(part.value);
    else if (part.type === "month") m = Number(part.value);
    else if (part.type === "day") d = Number(part.value);
  }
  if (!y || !m || !d) return null;
  return Math.floor(Date.UTC(y, m - 1, d) / 86_400_000);
}

/** Monday (as a Chicago day number) of the week containing a Chicago
 * day number. Day number 0 (1970-01-01) was a Thursday, so the weekday
 * of day n is ((n + 4) mod 7) with 0 = Sunday. */
function chicagoWeekMonday(dayNum: number): number {
  const weekday = (((dayNum + 4) % 7) + 7) % 7; // 0 = Sunday
  return dayNum - ((weekday + 6) % 7);
}

/** The sets visible in an index view, newest first (#8867). A set with
 * an unparseable createdAt only ever appears in "all". */
export function decisionSetsForView(sets: DecisionSet[], view: DecisionSetView, now = Date.now()): DecisionSet[] {
  const sorted = [...sets].sort((a, b) => b.createdAt.localeCompare(a.createdAt));
  if (view === "all") return sorted;
  const today = chicagoDayNumber(now);
  if (today === null) return sorted;
  if (view === "today") {
    return sorted.filter((s) => chicagoDayNumber(Date.parse(s.createdAt)) === today);
  }
  const monday = chicagoWeekMonday(today);
  return sorted.filter((s) => {
    const d = chicagoDayNumber(Date.parse(s.createdAt));
    return d !== null && d >= monday && d <= monday + 6;
  });
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
          ...(r.optionIds !== undefined ? { optionIds: r.optionIds } : {}),
          ...(r.optionLabels !== undefined ? { optionLabels: r.optionLabels } : {}),
          ...(r.freeText !== undefined ? { freeText: r.freeText } : {}),
          ...(r.customAnswer !== undefined ? { customAnswer: r.customAnswer } : {}),
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

function emptySetDrafts(): DecisionSetDrafts {
  return { questions: {}, bulk: [], bulkLean: "", bulkNote: "", updatedAt: "" };
}

/** The stored server drafts for a set (null: unknown set). Drafts for
 * questions that have since resolved are filtered out — belt and
 * braces on top of the prune-on-resolve in the writers. */
export async function getDecisionDrafts(file: string, setId: string): Promise<DecisionSetDrafts | null> {
  const data = await readDecisionsData(file);
  const set = data.sets.find((s) => s.setId === setId);
  if (!set) return null;
  const stored = data.drafts[setId];
  if (!stored) return emptySetDrafts();
  const out = emptySetDrafts();
  out.bulk = stored.bulk;
  out.bulkLean = stored.bulkLean;
  out.bulkNote = stored.bulkNote;
  out.updatedAt = stored.updatedAt;
  for (const item of set.items) {
    for (const q of item.questions) {
      if (q.resolution) continue;
      const d = stored.questions[q.qid];
      if (d) out.questions[q.qid] = d;
    }
  }
  return out;
}

/**
 * Upsert a set's server drafts from the page's debounced push. Each
 * entry is judged independently: unknown qids are skipped, drafts for
 * already-resolved questions are dropped (never resurrected), a null
 * entry deletes, and a stale entry (older updatedAt than the stored
 * draft) loses. The file is rewritten only when something changed,
 * so a no-op push leaves the store byte-identical.
 */
export async function upsertDecisionDrafts(
  file: string,
  setId: string,
  upsert: DecisionDraftUpsert,
): Promise<{ drafts: DecisionSetDrafts; applied: string[]; skipped: Array<{ qid: string; reason: string }> } | null> {
  return serializeDecisions(file, async () => {
    const data = await readDecisionsData(file);
    const set = data.sets.find((s) => s.setId === setId);
    if (!set) return null;
    const stored: DecisionSetDrafts = data.drafts[setId] ?? emptySetDrafts();
    const applied: string[] = [];
    const skipped: Array<{ qid: string; reason: string }> = [];
    let changed = false;
    for (const [qid, entry] of Object.entries(upsert.questions)) {
      const cut = qid.lastIndexOf("#");
      const ledgerId = cut > 0 ? qid.slice(0, cut) : "";
      const q = ledgerId ? findQuestion(set, ledgerId, qid) : null;
      if (!q) {
        skipped.push({ qid, reason: "unknown-question" });
        continue;
      }
      if (q.resolution) {
        if (stored.questions[qid]) {
          delete stored.questions[qid];
          changed = true;
        }
        skipped.push({ qid, reason: "already-resolved" });
        continue;
      }
      if (entry === null) {
        if (stored.questions[qid]) {
          delete stored.questions[qid];
          changed = true;
          applied.push(qid);
        }
        continue;
      }
      const existing = stored.questions[qid];
      if (existing && existing.updatedAt && entry.updatedAt && entry.updatedAt < existing.updatedAt) {
        skipped.push({ qid, reason: "stale" });
        continue;
      }
      const stamped: DecisionQuestionDraft = {
        ...entry,
        updatedAt: entry.updatedAt || new Date().toISOString(),
      };
      if (JSON.stringify(existing) !== JSON.stringify(stamped)) {
        stored.questions[qid] = stamped;
        changed = true;
      }
      applied.push(qid);
    }
    if (upsert.bulk !== undefined && JSON.stringify(upsert.bulk) !== JSON.stringify(stored.bulk)) {
      stored.bulk = upsert.bulk;
      changed = true;
    }
    if (upsert.bulkLean !== undefined && upsert.bulkLean !== stored.bulkLean) {
      stored.bulkLean = upsert.bulkLean;
      changed = true;
    }
    if (upsert.bulkNote !== undefined && upsert.bulkNote !== stored.bulkNote) {
      stored.bulkNote = upsert.bulkNote;
      changed = true;
    }
    if (changed) {
      stored.updatedAt = new Date().toISOString();
      data.drafts[setId] = stored;
      await writeDecisionsData(file, data);
    }
    return { drafts: stored, applied, skipped };
  });
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
//
// Both-layers upgrade (#8766, later the same day, Drew: "probably
// both"): the local layer moved from localStorage to IndexedDB — a
// localStorage draft written by the build above is migrated into
// IndexedDB on first load and the localStorage copy retired
// (localStorage remains the fallback when IndexedDB is unavailable) —
// and a server layer was added through the drafts endpoints above, so
// a draft also survives the device it was typed on. The page saves
// locally on every change, pushes to the server debounced (flushed
// when the tab hides or closes), and on load merges the two layers
// per question: newest updatedAt wins.

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
  const sid = JSON.stringify(setId);
  return `

// --- Draft persistence: IndexedDB local layer + server layer ---
// Two layers (#8766, extended after #8777): IndexedDB on this device
// (localStorage before it — drafts saved by the previous build are
// migrated on first load, never orphaned, and localStorage remains the
// fallback when IndexedDB is unavailable), plus a server copy pushed
// to the aggregator so a draft survives the device or browser it was
// typed on. Local saves are immediate; the server push is debounced
// and flushed when the tab hides or closes. On load the layers merge
// per question — newest edit wins — and drafts apply only to question
// blocks still on the page, so a settled question never resurrects
// from a draft on either layer. Best-effort throughout: any storage or
// network failure leaves the page exactly as functional as before.
var DRAFT_KEY = ${key};
var DRAFT_SET_ID = ${sid};
var draftState = { q: {}, bulk: [], bulkLean: '', bulkNote: '', updatedAt: 0 };
var draftGone = {};
var draftFromServer = 0;
var draftServerKnown = false;
var draftDb = null;
var draftDbReady = false;

function draftNormalize(d) {
  var out = { q: {}, bulk: [], bulkLean: '', bulkNote: '', updatedAt: 0 };
  if (!d || typeof d !== 'object') return out;
  if (d.q && typeof d.q === 'object') out.q = d.q;
  if (Array.isArray(d.bulk)) out.bulk = d.bulk;
  if (typeof d.bulkLean === 'string') out.bulkLean = d.bulkLean;
  if (typeof d.bulkNote === 'string') out.bulkNote = d.bulkNote;
  if (typeof d.updatedAt === 'number') out.updatedAt = d.updatedAt;
  return out;
}
function draftIsEmpty() {
  return !Object.keys(draftState.q).length && !draftState.bulk.length && !draftState.bulkLean && !draftState.bulkNote;
}
function draftDbOpen() {
  return new Promise(function (resolve) {
    try {
      if (!window.indexedDB) { resolve(false); return; }
      var rq = indexedDB.open('decisions-drafts-db', 1);
      rq.onupgradeneeded = function () { rq.result.createObjectStore('drafts'); };
      rq.onsuccess = function () { draftDb = rq.result; draftDbReady = true; resolve(true); };
      rq.onerror = function () { resolve(false); };
      rq.onblocked = function () { resolve(false); };
    } catch (e) { resolve(false); }
  });
}
function draftLocalPersist() {
  if (draftIsEmpty()) { draftLocalClear(); return; }
  var payload = JSON.stringify(draftState);
  if (draftDbReady && draftDb) {
    try {
      draftDb.transaction('drafts', 'readwrite').objectStore('drafts').put(payload, DRAFT_SET_ID);
      return;
    } catch (e) { /* fall through to the localStorage fallback */ }
  }
  try { localStorage.setItem(DRAFT_KEY, payload); } catch (e) { /* best-effort */ }
}
function draftLocalClear() {
  if (draftDbReady && draftDb) {
    try { draftDb.transaction('drafts', 'readwrite').objectStore('drafts').delete(DRAFT_SET_ID); } catch (e) {}
  }
  try { localStorage.removeItem(DRAFT_KEY); } catch (e) {}
}
function draftLocalRead() {
  // IndexedDB is the local layer now. A localStorage copy can also
  // exist — written by the previous build, or by this one while
  // IndexedDB was unavailable. When both exist the newer blob wins;
  // a localStorage-only draft is migrated into IndexedDB on first
  // sight and the localStorage copy is retired, never orphaned.
  return new Promise(function (resolve) {
    var lsData = null;
    try {
      var raw = localStorage.getItem(DRAFT_KEY);
      if (raw) lsData = draftNormalize(JSON.parse(raw));
    } catch (e) {}
    if (!draftDbReady || !draftDb) { resolve(lsData); return; }
    try {
      var rq = draftDb.transaction('drafts', 'readonly').objectStore('drafts').get(DRAFT_SET_ID);
      rq.onsuccess = function () {
        var idbData = null;
        try { if (rq.result) idbData = draftNormalize(JSON.parse(rq.result)); } catch (e) {}
        if (idbData && !lsData) { resolve(idbData); return; }
        if (!idbData && !lsData) { resolve(null); return; }
        var winner = idbData;
        if (lsData && (!idbData || (lsData.updatedAt || 0) > (idbData.updatedAt || 0))) {
          winner = lsData;
          var now = Date.now();
          Object.keys(winner.q).forEach(function (qid) {
            if (!winner.q[qid].updatedAt) winner.q[qid].updatedAt = now;
          });
          if (!winner.updatedAt) winner.updatedAt = now;
        }
        draftState = winner;
        draftLocalPersist();
        try { localStorage.removeItem(DRAFT_KEY); } catch (e) {}
        resolve(winner);
      };
      rq.onerror = function () { resolve(lsData); };
    } catch (e) { resolve(lsData); }
  });
}
function draftCollect() {
  var seen = {};
  document.querySelectorAll('.q').forEach(function (block) {
    var entry = {};
    var picked = block.querySelector('input[type=radio]:checked');
    if (picked) entry.opt = picked.value;
    var boxes = block.querySelectorAll('input[type=checkbox].optin:checked');
    if (boxes.length) entry.opts = Array.prototype.map.call(boxes, function (c) { return c.value; });
    var ft = block.querySelector('textarea.ft').value;
    if (ft) entry.ft = ft;
    var lean = block.querySelector('select.defer').value;
    if (lean) entry.lean = lean;
    var note = block.querySelector('textarea.infonote').value;
    if (note) entry.note = note;
    if (Object.keys(entry).length) {
      var qid = block.getAttribute('data-qid');
      seen[qid] = true;
      var prev = draftState.q[qid];
      if (!prev || prev.opt !== entry.opt || JSON.stringify(prev.opts || []) !== JSON.stringify(entry.opts || []) || prev.ft !== entry.ft || prev.lean !== entry.lean || prev.note !== entry.note) {
        entry.updatedAt = Date.now();
      } else {
        entry.updatedAt = prev.updatedAt || Date.now();
      }
      draftState.q[qid] = entry;
    }
  });
  // A question whose block left the DOM was recorded; a question whose
  // fields were cleared is intentionally blank. Either way its draft
  // is gone locally and tombstoned for the next server push.
  Object.keys(draftState.q).forEach(function (qid) {
    if (!seen[qid]) { delete draftState.q[qid]; draftGone[qid] = true; }
  });
  draftState.bulk = [];
  document.querySelectorAll('section.item').forEach(function (card) {
    var box = card.querySelector('input.bulk');
    if (box && box.checked) draftState.bulk.push(card.getAttribute('data-ledger'));
  });
  var bl = document.getElementById('bulkLean');
  draftState.bulkLean = bl ? bl.value : '';
  var bn = document.getElementById('bulkInfoNote');
  draftState.bulkNote = bn ? bn.value : '';
  draftState.updatedAt = Date.now();
}
var draftPushTimer = null;
function draftPush() {
  if (draftPushTimer) return;
  draftPushTimer = setTimeout(function () { draftPushTimer = null; draftPushNow(false); }, 900);
}
function draftPushNow(keepalive) {
  if (typeof TOKEN === 'undefined') return;
  var questions = {};
  Object.keys(draftState.q).forEach(function (qid) {
    var e = draftState.q[qid];
    var out = {};
    if (e.opt) out.opt = e.opt;
    if (e.opts && e.opts.length) out.opts = e.opts;
    if (e.ft) out.ft = e.ft;
    if (e.lean) out.lean = e.lean;
    if (e.note) out.note = e.note;
    out.updatedAt = new Date(e.updatedAt || Date.now()).toISOString();
    questions[qid] = out;
  });
  Object.keys(draftGone).forEach(function (qid) { questions[qid] = null; });
  if (!Object.keys(questions).length && !draftState.bulk.length && !draftState.bulkLean && !draftState.bulkNote && !draftServerKnown) return;
  var payload = { token: TOKEN, drafts: { questions: questions, bulk: draftState.bulk, bulkLean: draftState.bulkLean, bulkNote: draftState.bulkNote } };
  try {
    fetch('/api/decisions/' + encodeURIComponent(DRAFT_SET_ID) + '/drafts', {
      method: 'POST', headers: { 'content-type': 'application/json' },
      body: JSON.stringify(payload), keepalive: !!keepalive
    }).then(function (r) { if (r && r.ok) { draftGone = {}; draftServerKnown = true; } }).catch(function () {});
  } catch (e) { /* drafts are best-effort */ }
}
function draftSave() {
  draftCollect();
  draftLocalPersist();
  if (!draftIsEmpty() || Object.keys(draftGone).length || draftServerKnown) draftPush();
}
function draftFetchServer() {
  if (typeof TOKEN === 'undefined') return Promise.resolve(null);
  return fetch('/api/decisions/' + encodeURIComponent(DRAFT_SET_ID) + '/drafts?t=' + encodeURIComponent(TOKEN))
    .then(function (r) { return r.ok ? r.json() : null; })
    .then(function (j) {
      if (j && j.drafts) { draftServerKnown = true; return j.drafts; }
      return null;
    })
    .catch(function () { return null; });
}
function draftMergeServer(sv) {
  if (!sv || typeof sv !== 'object') return;
  var sq = sv.questions || {};
  Object.keys(sq).forEach(function (qid) {
    var s = sq[qid];
    if (!s) return;
    var sAt = Date.parse(s.updatedAt) || 0;
    var l = draftState.q[qid];
    var lAt = l ? (l.updatedAt || 0) : -1;
    if (sAt > lAt) {
      var e = {};
      if (s.opt) e.opt = s.opt;
      if (Array.isArray(s.opts) && s.opts.length) e.opts = s.opts;
      if (s.ft) e.ft = s.ft;
      if (s.lean) e.lean = s.lean;
      if (s.note) e.note = s.note;
      e.updatedAt = sAt || Date.now();
      draftState.q[qid] = e;
      draftFromServer++;
    }
  });
  var svAt = Date.parse(sv.updatedAt) || 0;
  if (svAt > (draftState.updatedAt || 0)) {
    draftState.bulk = Array.isArray(sv.bulk) ? sv.bulk : [];
    draftState.bulkLean = typeof sv.bulkLean === 'string' ? sv.bulkLean : '';
    draftState.bulkNote = typeof sv.bulkNote === 'string' ? sv.bulkNote : '';
  }
}
function draftRestore() {
  var d = draftState;
  var restored = 0;
  document.querySelectorAll('.q').forEach(function (block) {
    var entry = (d.q || {})[block.getAttribute('data-qid')];
    if (!entry) return;
    if (entry.opt) {
      block.querySelectorAll('input[type=radio]').forEach(function (r) {
        if (r.value === entry.opt) { r.checked = true; restored++; }
      });
    }
    if (entry.opts) {
      block.querySelectorAll('input[type=checkbox].optin').forEach(function (c) {
        if (entry.opts.indexOf(c.value) !== -1) { c.checked = true; restored++; }
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
  if (restored) {
    status(draftFromServer
      ? 'Restored your in-progress work — including drafts synced from the server (started on another device or browser). Nothing you typed was lost.'
      : 'Restored your in-progress work from before the interruption — nothing you typed was lost.');
  }
}
document.addEventListener('input', draftSave);
document.addEventListener('change', draftSave);
new MutationObserver(draftSave).observe(document.querySelector('main'), { childList: true, subtree: true });
document.addEventListener('visibilitychange', function () {
  if (document.visibilityState === 'hidden') draftPushNow(true);
});
window.addEventListener('beforeunload', function () { draftPushNow(true); });
draftDbOpen().then(function () { return draftLocalRead(); }).then(function (local) {
  if (local) draftState = local;
  return draftFetchServer();
}).then(function (serverDrafts) {
  draftMergeServer(serverDrafts);
  draftRestore();
  draftSave();
}).catch(function () { draftRestore(); draftSave(); });
`;
}
