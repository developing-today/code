import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "path";
import { decisionSetSummary, getDecisionSet, listDecisionSets, recordResolutions } from "../src/decisions.ts";
import { questionsSectionHtml } from "../src/render.ts";
import {
  seedStandingDecisionSets,
  standingSeedInput,
  STANDING_OPEN_QUESTIONS_SET_ID,
  STANDING_SEED_SOURCE,
} from "../src/standing-seed.ts";

// #13983 slice 1: the standing open-questions set is seeded through
// the #9360 refresh path, re-seeding is idempotent, and the seeded
// store renders a Questions entry with a standing-set link on /today.
// Oracle for every test below: the decisions store itself (for
// seed/refresh) and the rendered Questions HTML string (for the
// section/link) — no mocks of the code under test.

let dir: string;
let file: string;

beforeEach(async () => {
  dir = await mkdtemp(join(tmpdir(), "standing-seed-13983-test-"));
  file = join(dir, "decisions.json");
});

afterEach(async () => {
  await rm(dir, { recursive: true, force: true });
});

describe("standing seed creates via the refresh path (#13983)", () => {
  test("fresh store: seed creates the standing set with the fixture contents — oracle: the stored set", async () => {
    const result = await seedStandingDecisionSets(file);
    expect(result.created).toBe(true);
    expect(result.refreshed).toBe(false);
    expect(result.set.setId).toBe(STANDING_OPEN_QUESTIONS_SET_ID);
    expect(result.set.token.length).toBeGreaterThan(0);
    expect(result.set.source).toBe(STANDING_SEED_SOURCE);
    const stored = await getDecisionSet(file, STANDING_OPEN_QUESTIONS_SET_ID);
    expect(stored?.title).toBe(standingSeedInput().title);
    expect(stored?.items.length).toBe(standingSeedInput().items.length);
    expect(stored?.items[0]?.questions.map((q) => q.qid)).toEqual(["standing-ledger-seed#1", "standing-ledger-seed#2"]);
  });

  test("re-seed is idempotent: no duplicate set, token + createdAt + source preserved, refreshed:true — oracle: the stored sets", async () => {
    const first = await seedStandingDecisionSets(file);
    const applied = await recordResolutions(file, STANDING_OPEN_QUESTIONS_SET_ID, [
      { ledgerId: "standing-ledger-seed", qid: "standing-ledger-seed#1", kind: "answered", optionId: "yes" },
    ]);
    expect(applied?.outcomes).toEqual([{ qid: "standing-ledger-seed#1", applied: true }]);

    const second = await seedStandingDecisionSets(file);
    expect(second.created).toBe(false);
    expect(second.refreshed).toBe(true);
    expect(second.set.setId).toBe(first.set.setId);
    expect(second.set.token).toBe(first.set.token);
    expect(second.set.createdAt).toBe(first.set.createdAt);
    expect(second.set.source).toBe(first.set.source);
    // Exactly one set with this id in the store.
    const sets = await listDecisionSets(file);
    expect(sets.filter((s) => s.setId === STANDING_OPEN_QUESTIONS_SET_ID).length).toBe(1);
    // The resolution recorded between the seeds carried over by qid.
    const q1 = second.set.items[0]?.questions[0];
    expect(q1?.resolution?.kind).toBe("answered");
    expect(q1?.resolution?.optionId).toBe("yes");
  });
});

describe("seeded store renders the /today Questions entry (#13983)", () => {
  test("Questions section links the standing set by token URL — oracle: the rendered section HTML", async () => {
    const { set } = await seedStandingDecisionSets(file);
    const sets = await listDecisionSets(file);
    const summaries = sets
      .map((s) => decisionSetSummary(s))
      .filter((s) => s.openQuestions > 0)
      .sort((a, b) => b.createdAt.localeCompare(a.createdAt));
    const html = questionsSectionHtml(summaries, (s) => `/decisions/${s.setId}?t=${set.token}`);
    expect(html).toContain("<h2");
    expect(html).toContain("Questions");
    expect(html).toContain(STANDING_OPEN_QUESTIONS_SET_ID);
    expect(html).toContain(`/decisions/${STANDING_OPEN_QUESTIONS_SET_ID}?t=${set.token}`);
    expect(html).toContain("Live from the decision-set store at page load.");
  });
});
