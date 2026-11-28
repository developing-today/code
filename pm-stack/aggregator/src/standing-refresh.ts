// Standing decision-set refresh (#13983 slice 2, remainder of #9354).
//
// Slice 1 (src/standing-seed.ts) seeded the standing open-questions
// set with a bootstrap fixture through the #9360 refresh path. This
// module is the cadence half: it builds the standing set's payload
// from the live pending-questions ledger (the central artifact of
// pm-stack/decisions-ledger-system.md, owned by the
// pending-decisions-hourly-questions-for-drew goal) and applies it
// through the same refresh path, so the hourly job
// (scripts/refresh-standing-decision-sets.ts, driven by
// systemd/standing-refresh.timer) keeps the set mirroring what is
// actually waiting on Drew instead of the bootstrap placeholder.
//
// Ledger contract (as written by the hourly pending-questions job):
// a JSON object with an `items` array; each item carries `id`,
// `title`, `status`, `links`, and `questions` (a list of
// { q, context?, options?, infoProvided? }). An item is OPEN while
// its status is "asked" or "pending"; "answered" and "dropped"
// items leave the mirror on the next refresh. Ledger options are
// strings, { id, label } objects, or { key, label, detail } objects;
// all three shapes map onto the decision-set option contract.
//
// Refresh semantics are #9360's, used as-is (see standing-seed.ts):
// the set keeps its setId, token, createdAt and original source; a
// question whose ledger item and position are unchanged keeps its
// qid (`<ledgerId>#<1-based index>`), so its resolution carries over
// across refreshes. A refresh with no open ledger items is REFUSED:
// the standing set must never be emptied by a bad or empty ledger
// read — the last-known-good mirror stays in the store.
import { homedir } from "node:os";
import { join } from "node:path";
import { validateDecisionSet, type DecisionOption, type DecisionSetInput } from "./decisions.ts";
import {
  seedStandingDecisionSets,
  STANDING_OPEN_QUESTIONS_SET_ID,
  STANDING_SEED_SOURCE,
  type StandingSeedResult,
} from "./standing-seed.ts";

/** Ledger statuses that mean "still waiting on Drew". */
export const OPEN_LEDGER_STATUSES: ReadonlySet<string> = new Set(["asked", "pending"]);

/** Default location of the live pending-questions ledger. */
export function defaultPendingQuestionsFile(): string {
  const fromEnv = process.env.PM_AGG_PENDING_QUESTIONS_FILE;
  if (fromEnv && fromEnv.trim()) return fromEnv;
  return join(
    homedir(),
    "workspace/goals/pending-decisions-hourly-questions-for-drew/hidden_files/pending-questions.json",
  );
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === "object" && !Array.isArray(value);
}

function slugifyOptionId(label: string, fallbackIndex: number): string {
  const slug = label
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 40)
    .replace(/-+$/g, "");
  return slug || `option-${fallbackIndex + 1}`;
}

/** Map one ledger question's options onto the decision-set contract. */
function mapOptions(rawOptions: unknown): DecisionOption[] {
  if (!Array.isArray(rawOptions)) return [];
  const options: DecisionOption[] = [];
  const seen = new Set<string>();
  for (const raw of rawOptions) {
    let id = "";
    let label = "";
    if (typeof raw === "string") {
      label = raw.trim();
      id = slugifyOptionId(label, options.length);
    } else if (isRecord(raw)) {
      label =
        (typeof raw.label === "string" && raw.label.trim()) ||
        (typeof raw.key === "string" && raw.key.trim()) ||
        (typeof raw.id === "string" && raw.id.trim()) ||
        "";
      id =
        (typeof raw.id === "string" && raw.id.trim()) ||
        (typeof raw.key === "string" && raw.key.trim()) ||
        slugifyOptionId(label, options.length);
    }
    if (!label || !id) continue;
    let uniqueId = id;
    for (let n = 2; seen.has(uniqueId); n++) uniqueId = `${id}-${n}`;
    seen.add(uniqueId);
    options.push({ id: uniqueId, label });
    if (options.length >= 12) break; // validateDecisionSet caps options at 12
  }
  return options;
}

export interface StandingInputBuild {
  input: DecisionSetInput;
  /** Ledger items mirrored into the set. */
  openItems: number;
  /** Open ledger items skipped (no id/title, or no usable questions). */
  skippedItems: number;
  /** True when the open-item count exceeded the 64-item set cap. */
  truncated: boolean;
}

/**
 * Build the standing open-questions payload from a parsed
 * pending-questions ledger. Throws on a malformed ledger (not an
 * object, or `items` not an array) and on a ledger with no open
 * items — the caller must not refresh in either case.
 */
export function standingInputFromLedger(ledger: unknown): StandingInputBuild {
  if (!isRecord(ledger) || !Array.isArray(ledger.items)) {
    throw new Error("pending-questions ledger must be an object with an items array");
  }
  const items: DecisionSetInput["items"] = [];
  let openItems = 0;
  let skippedItems = 0;
  let truncated = false;
  for (const rawItem of ledger.items) {
    if (!isRecord(rawItem)) continue;
    if (typeof rawItem.status !== "string" || !OPEN_LEDGER_STATUSES.has(rawItem.status)) continue;
    const ledgerId = typeof rawItem.id === "string" ? rawItem.id.trim() : "";
    const title = typeof rawItem.title === "string" ? rawItem.title.trim() : "";
    const rawQuestions = Array.isArray(rawItem.questions) ? rawItem.questions : [];
    const questions: DecisionSetInput["items"][number]["questions"] = [];
    for (const rawQ of rawQuestions) {
      if (!isRecord(rawQ)) continue;
      const question = typeof rawQ.q === "string" ? rawQ.q.trim() : "";
      if (!question) continue;
      questions.push({
        question,
        ...(typeof rawQ.context === "string" && rawQ.context.trim() ? { context: rawQ.context } : {}),
        ...(typeof rawQ.infoProvided === "string" && rawQ.infoProvided.trim()
          ? { infoProvided: rawQ.infoProvided }
          : {}),
        options: mapOptions(rawQ.options),
      });
      if (questions.length >= 16) break; // validateDecisionSet caps questions at 16 per item
    }
    if (!ledgerId || !title || questions.length === 0) {
      skippedItems++;
      continue;
    }
    if (items.length >= 64) {
      // validateDecisionSet caps a set at 64 items; keep the first
      // 64 in ledger order and report the truncation, never fail.
      truncated = true;
      continue;
    }
    const links = Array.isArray(rawItem.links)
      ? rawItem.links.filter((l): l is string => typeof l === "string" && /^https?:\/\//.test(l))
      : [];
    items.push({ ledgerId, title, group: "Standing", links, questions });
    openItems++;
  }
  if (items.length === 0) {
    throw new Error("pending-questions ledger has no open items to mirror; refusing to empty the standing set");
  }
  const input: DecisionSetInput = {
    setId: STANDING_OPEN_QUESTIONS_SET_ID,
    title: "Standing open questions",
    items,
  };
  const v = validateDecisionSet(input);
  if (!v.ok) throw new Error(`standing refresh payload invalid: ${v.error}`);
  return { input: v.value, openItems, skippedItems, truncated };
}

export interface StandingRefreshResult extends StandingSeedResult {
  openItems: number;
  skippedItems: number;
  truncated: boolean;
}

/**
 * Refresh the standing open-questions set from a parsed
 * pending-questions ledger, through the #9360 refresh path. The
 * store is untouched when the ledger is malformed or has no open
 * items (standingInputFromLedger throws before any write).
 */
export async function refreshStandingDecisionSets(
  file: string,
  ledger: unknown,
  source: string = STANDING_SEED_SOURCE,
): Promise<StandingRefreshResult> {
  const build = standingInputFromLedger(ledger);
  const result = await seedStandingDecisionSets(file, build.input, source);
  return { ...result, openItems: build.openItems, skippedItems: build.skippedItems, truncated: build.truncated };
}
