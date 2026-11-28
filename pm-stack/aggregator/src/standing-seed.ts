// Standing decision-set seed (#13983 slice 1, remainder of #9354).
//
// The #9360 refresh machinery (refreshDecisionSet, POST
// /api/decision-sets with refresh:true, scripts/submit-decision-set.ts
// --refresh) has no standing ledger-mirror sets to maintain until
// something seeds them. This module is that something: the slice-1
// seeding half. Slice 2 (same issue, src/standing-refresh.ts) adds
// the hourly-job cadence that replaces this bootstrap payload from
// the live pending-questions ledger on schedule.
//
// The seed is a bootstrap placeholder, not live ledger content — it
// says so on its own questions — so the Questions section #9360 added
// to Today/Needs-you has a live standing-set data source from day
// one. The hourly job keeps the setId stable and refreshes title/items
// through the same refreshDecisionSet path used here.
//
// Refresh semantics (from #9360, used as-is): an existing set keeps
// its setId, token, createdAt and original source; title/items are
// replaced; a question's resolution carries over by qid (and its
// versions/currentVersion when the wording is identical). Seeding
// twice is therefore idempotent: no duplicate sets, tokens preserved.
import {
  defaultDecisionsFile,
  getDecisionSet,
  refreshDecisionSet,
  validateDecisionSet,
  type DecisionSet,
  type DecisionSetInput,
} from "./decisions.ts";

/** The canonical standing set this seed maintains. */
export const STANDING_OPEN_QUESTIONS_SET_ID = "standing-open-questions";

/** Store source for seeded sets (matches the API submit path default). */
export const STANDING_SEED_SOURCE = "decisions-ledger";

/**
 * The bootstrap seed payload: one ledger-mirror item with open
 * questions, so the seeded set always has open questions until the
 * hourly job (slice 2) replaces it with live ledger content or Drew
 * resolves them. Validated by validateDecisionSet before every seed.
 */
export function standingSeedInput(): DecisionSetInput {
  return {
    setId: STANDING_OPEN_QUESTIONS_SET_ID,
    title: "Standing open questions",
    items: [
      {
        ledgerId: "standing-ledger-seed",
        title: "Standing ledger mirror (seed)",
        group: "Standing",
        links: [],
        questions: [
          {
            question: "Standing seed: is the hourly ledger-mirror refresh running?",
            context:
              "Slice-1 bootstrap placeholder (developing-today/code#13983). " +
              "Slice 2 replaces this payload from the live pending-questions ledger on cadence.",
            options: [
              { id: "yes", label: "Yes" },
              { id: "no", label: "No" },
            ],
          },
          {
            question: "Standing seed: which standing sets should the hourly job maintain?",
            context: "Free-text bootstrap question; answer in your own words.",
            options: [],
          },
        ],
      },
    ],
  };
}

export interface StandingSeedResult {
  set: DecisionSet;
  /** True when the seed created the set (no set with this id existed). */
  created: boolean;
  /** True when the seed refreshed an existing set (token preserved). */
  refreshed: boolean;
}

/**
 * Seed the standing open-questions set through the #9360 refresh
 * path. Idempotent: a second seed with the same input keeps the
 * setId, token, createdAt and original source, carries resolutions
 * over by qid, and reports refreshed:true (mirroring the
 * POST /api/decision-sets refresh:true response shape).
 */
export async function seedStandingDecisionSets(
  file: string = defaultDecisionsFile(),
  input: DecisionSetInput = standingSeedInput(),
  source: string = STANDING_SEED_SOURCE,
): Promise<StandingSeedResult> {
  const v = validateDecisionSet(input);
  if (!v.ok) throw new Error(`standing seed payload invalid: ${v.error}`);
  const before = await getDecisionSet(file, v.value.setId);
  const set = await refreshDecisionSet(file, v.value, source);
  return { set, created: !before, refreshed: !!before };
}
