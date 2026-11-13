import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { appendFile, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  appendQuestionVersion,
  decisionSetProgress,
  effectiveQuestionMode,
  getDecisionDrafts,
  getDecisionSet,
  listDecisionResolutions,
  raiseDecisionSet,
  recordResolutions,
  upsertDecisionDrafts,
  validateDecisionDraftUpsert,
  validateDecisionSet,
  withGeneratedSetId,
  type DecisionSetInput,
} from "../src/decisions.ts";
import {
  appendDecisionEvent,
  appendDecisionEvents,
  readDecisionEvents,
  validateDecisionEventInput,
} from "../src/decision-events.ts";

// Decision Sets v2 (#9335 Track A): schema v2 (rich detail, single/multi
// modes, custom answer, tags, combination options, recommended defaults,
// ordering, versioning), the append-only event log + outbox, and the
// submit-API preparation. v1 sets must keep working throughout.

let dir: string;
let file: string;
let eventsFile: string;
let outboxFile: string;

beforeEach(async () => {
  dir = await mkdtemp(join(tmpdir(), "decisions-v2-test-"));
  file = join(dir, "decisions.json");
  eventsFile = join(dir, "decision-events.jsonl");
  outboxFile = join(dir, "decision-events-outbox.jsonl");
});

afterEach(async () => {
  await rm(dir, { recursive: true, force: true });
});

function v2Input(setId = "set-v2-1"): DecisionSetInput {
  return {
    setId,
    title: "V2 set",
    items: [
      {
        ledgerId: "item-a",
        title: "Item A",
        group: "G1",
        order: 2,
        links: [],
        questions: [
          {
            question: "Pick the rollout shape?",
            context: "short context",
            description:
              "Background: the rollout touches three repos. Why it matters: a wrong shape doubles the migration. Consequences: A is fastest, A+B is safest.",
            mode: "multi",
            tags: ["rollout"],
            order: 1,
            options: [
              { id: "a", label: "Ship A", tags: ["simpler"] },
              { id: "b", label: "Ship B", tags: ["safer"] },
              { id: "ab", label: "Ship A+B", tags: ["safer", "recommended"], combinationOf: ["a", "b"] },
              { id: "ent", label: "Enterprise rollout", tags: ["portable"] },
            ],
            recommendedOptionIds: ["ab"],
          },
          {
            question: "Single pick?",
            options: [
              { id: "y", label: "Yes" },
              { id: "n", label: "No" },
            ],
            recommendedOptionIds: ["y"],
          },
        ],
      },
    ],
  };
}

function v1Input(setId = "set-v1-1"): DecisionSetInput {
  return {
    setId,
    title: "V1 set",
    items: [
      {
        ledgerId: "legacy",
        title: "Legacy item",
        links: [],
        questions: [{ question: "Old style?", options: [{ id: "o1", label: "One" }] }],
      },
    ],
  };
}

describe("schema v2 validation (#9335)", () => {
  test("accepts a full v2 set: description, multi, tags, combinationOf, recommended, order", () => {
    const raw = JSON.parse(JSON.stringify(v2Input())) as unknown;
    const v = validateDecisionSet(raw);
    expect(v.ok).toBe(true);
    if (!v.ok) return;
    const q = v.value.items[0]!.questions[0]!;
    expect(q.mode).toBe("multi");
    expect(q.description).toContain("Background:");
    expect(q.tags).toEqual(["rollout"]);
    expect(q.recommendedOptionIds).toEqual(["ab"]);
    expect(q.options[2]).toMatchObject({ combinationOf: ["a", "b"], tags: ["safer", "recommended"] });
    expect(v.value.items[0]!.order).toBe(2);
    expect(q.order).toBe(1);
  });

  test("detail is accepted as the description alias", () => {
    const raw = JSON.parse(JSON.stringify(v2Input())) as Record<string, any>;
    delete raw.items[0].questions[0].description;
    raw.items[0].questions[0].detail = "detail text";
    const v = validateDecisionSet(raw);
    expect(v.ok).toBe(true);
    if (v.ok) expect(v.value.items[0]!.questions[0]!.description).toBe("detail text");
  });

  test("rejects combinationOf pointing at an unknown or self option", () => {
    const bad = JSON.parse(JSON.stringify(v2Input())) as Record<string, any>;
    bad.items[0].questions[0].options[2].combinationOf = ["a", "ghost"];
    expect(validateDecisionSet(bad).ok).toBe(false);
    const self = JSON.parse(JSON.stringify(v2Input())) as Record<string, any>;
    self.items[0].questions[0].options[2].combinationOf = ["ab", "a"];
    expect(validateDecisionSet(self).ok).toBe(false);
  });

  test("rejects recommended ids that are not options, and >1 recommended in single mode", () => {
    const bad = JSON.parse(JSON.stringify(v2Input())) as Record<string, any>;
    bad.items[0].questions[0].recommendedOptionIds = ["ghost"];
    expect(validateDecisionSet(bad).ok).toBe(false);
    const single = JSON.parse(JSON.stringify(v2Input())) as Record<string, any>;
    single.items[0].questions[1].recommendedOptionIds = ["y", "n"];
    expect(validateDecisionSet(single).ok).toBe(false);
    const badMode = JSON.parse(JSON.stringify(v2Input())) as Record<string, any>;
    badMode.items[0].questions[0].mode = "ranked";
    expect(validateDecisionSet(badMode).ok).toBe(false);
  });

  test("v1 payloads still validate unchanged", () => {
    expect(validateDecisionSet(JSON.parse(JSON.stringify(v1Input()))).ok).toBe(true);
  });
});

describe("submit-API preparation (#9335)", () => {
  test("withGeneratedSetId mints a UUID when setId is omitted (bare + envelope)", () => {
    const bare = v2Input();
    delete (bare as { setId?: string }).setId;
    const filled = withGeneratedSetId(JSON.parse(JSON.stringify(bare))) as DecisionSetInput;
    expect(filled.setId).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i);
    expect(validateDecisionSet(filled).ok).toBe(true);
    const env = withGeneratedSetId({ decisionSet: JSON.parse(JSON.stringify(bare)) }) as {
      decisionSet: DecisionSetInput;
    };
    expect(env.decisionSet.setId).toMatch(/^[0-9a-f-]{36}$/i);
    expect(validateDecisionSet(env.decisionSet).ok).toBe(true);
  });

  test("withGeneratedSetId keeps a supplied setId", () => {
    const out = withGeneratedSetId(JSON.parse(JSON.stringify(v2Input("keep-me")))) as DecisionSetInput;
    expect(out.setId).toBe("keep-me");
  });
});

describe("v2 store behavior (#9335)", () => {
  test("raise stores v2 fields and backfills version history at v1", async () => {
    const set = await raiseDecisionSet(file, v2Input());
    const q = set.items[0]!.questions[0]!;
    expect(q.mode).toBe("multi");
    expect(q.description).toContain("Background:");
    expect(q.recommendedOptionIds).toEqual(["ab"]);
    expect(q.versions).toHaveLength(1);
    expect(q.versions![0]).toMatchObject({ version: 1, question: "Pick the rollout shape?" });
    expect(q.currentVersion).toBe(1);
    expect(set.items[0]!.order).toBe(2);
  });

  test("recommended defaults are NOT a resolution until submitted", async () => {
    const set = await raiseDecisionSet(file, v2Input());
    const progress = decisionSetProgress(set);
    expect(progress.openQuestions).toBe(2);
    expect(progress.answered).toBe(0);
    expect(await listDecisionResolutions(file)).toEqual([]);
    // Submitting the default is what records it.
    const res = await recordResolutions(file, "set-v2-1", [
      { ledgerId: "item-a", qid: "item-a#2", kind: "answered", optionId: "y" },
    ]);
    expect(res?.outcomes).toEqual([{ qid: "item-a#2", applied: true }]);
    expect(decisionSetProgress(res!.set).answered).toBe(1);
  });

  test("multi answers record every pick plus the custom answer", async () => {
    await raiseDecisionSet(file, v2Input());
    const res = await recordResolutions(file, "set-v2-1", [
      {
        ledgerId: "item-a",
        qid: "item-a#1",
        kind: "answered",
        optionIds: ["a", "b"],
        customAnswer: "and stage it behind the flag",
      },
    ]);
    expect(res?.outcomes).toEqual([{ qid: "item-a#1", applied: true }]);
    const records = await listDecisionResolutions(file);
    expect(records).toHaveLength(1);
    expect(records[0]).toMatchObject({
      kind: "answered",
      optionId: "a",
      optionIds: ["a", "b"],
      optionLabels: ["Ship A", "Ship B"],
      customAnswer: "and stage it behind the flag",
      freeText: "and stage it behind the flag",
    });
  });

  test("single mode rejects multiple picks; unknown picks are bad-option", async () => {
    await raiseDecisionSet(file, v2Input());
    const res = await recordResolutions(file, "set-v2-1", [
      { ledgerId: "item-a", qid: "item-a#2", kind: "answered", optionIds: ["y", "n"] },
      { ledgerId: "item-a", qid: "item-a#1", kind: "answered", optionIds: ["a", "ghost"] },
    ]);
    expect(res?.outcomes).toEqual([
      { qid: "item-a#2", applied: false, reason: "too-many-options" },
      { qid: "item-a#1", applied: false, reason: "bad-option" },
    ]);
  });

  test("v1 sets: raise + answer exactly as before; mode defaults to single", async () => {
    const set = await raiseDecisionSet(file, v1Input());
    expect(effectiveQuestionMode(set.items[0]!.questions[0]!)).toBe("single");
    const res = await recordResolutions(file, "set-v1-1", [
      { ledgerId: "legacy", qid: "legacy#1", kind: "answered", freeText: "words only" },
    ]);
    expect(res?.outcomes).toEqual([{ qid: "legacy#1", applied: true }]);
  });

  test("a hand-written v1 store file (no v2 fields anywhere) still reads and resolves", async () => {
    await writeFile(
      file,
      JSON.stringify({
        version: 1,
        sets: [
          {
            setId: "hand-v1",
            title: "Hand V1",
            token: "tok-hand",
            createdAt: "2026-11-01T00:00:00.000Z",
            source: "test",
            items: [
              {
                ledgerId: "old",
                title: "Old",
                group: "G",
                links: [],
                questions: [{ qid: "old#1", question: "Q?", options: [{ id: "x", label: "X" }] }],
              },
            ],
          },
        ],
      }),
    );
    const set = await getDecisionSet(file, "hand-v1");
    expect(set?.title).toBe("Hand V1");
    const res = await recordResolutions(file, "hand-v1", [
      { ledgerId: "old", qid: "old#1", kind: "answered", optionId: "x" },
    ]);
    expect(res?.outcomes).toEqual([{ qid: "old#1", applied: true }]);
    expect((await listDecisionResolutions(file))[0]).toMatchObject({ qid: "old#1", kind: "answered" });
  });
});

describe("question versioning (#9335)", () => {
  test("appendQuestionVersion keeps all versions and moves the pointer", async () => {
    await raiseDecisionSet(file, v2Input());
    const updated = await appendQuestionVersion(file, "set-v2-1", "item-a", "item-a#1", {
      question: "Pick the rollout shape — v2 wording?",
      note: "clearer after the incident review",
    });
    expect(updated?.question).toBe("Pick the rollout shape — v2 wording?");
    expect(updated?.currentVersion).toBe(2);
    expect(updated?.versions?.map((v) => v.version)).toEqual([1, 2]);
    expect(updated?.versions![0]!.question).toBe("Pick the rollout shape?");
    expect(updated?.versions![1]).toMatchObject({ note: "clearer after the incident review" });
    const reread = await getDecisionSet(file, "set-v2-1");
    expect(reread?.items[0]!.questions[0]!.currentVersion).toBe(2);
    expect(await appendQuestionVersion(file, "nope", "item-a", "item-a#1", { question: "x" })).toBeNull();
  });

  test("a pre-versioning question is backfilled to v1 before appending v2", async () => {
    await writeFile(
      file,
      JSON.stringify({
        version: 1,
        sets: [
          {
            setId: "hand-v1",
            title: "Hand V1",
            token: "tok-hand",
            createdAt: "2026-11-01T00:00:00.000Z",
            source: "test",
            items: [
              {
                ledgerId: "old",
                title: "Old",
                group: "G",
                links: [],
                questions: [{ qid: "old#1", question: "Original?", options: [] }],
              },
            ],
          },
        ],
      }),
    );
    const updated = await appendQuestionVersion(file, "hand-v1", "old", "old#1", { question: "Reworded?" });
    expect(updated?.versions?.map((v) => v.question)).toEqual(["Original?", "Reworded?"]);
    expect(updated?.currentVersion).toBe(2);
  });
});

describe("decision event log + outbox (#9335)", () => {
  test("append writes the log and a pending outbox twin for every event", async () => {
    const e1 = await appendDecisionEvent(eventsFile, outboxFile, {
      setId: "set-v2-1",
      type: "set.created",
      payload: { via: "test" },
    });
    expect(e1.eventId).toMatch(/^[0-9a-f-]{36}$/i);
    await appendDecisionEvents(eventsFile, outboxFile, [
      {
        setId: "set-v2-1",
        type: "draft",
        questionRef: { ledgerId: "item-a", qid: "item-a#1", questionIndex: 1 },
        payload: { customAnswer: "typing…" },
      },
      { setId: "other-set", type: "submit", payload: {} },
    ]);
    const page = await readDecisionEvents(eventsFile, "set-v2-1");
    expect(page.count).toBe(2);
    expect(page.events.map((e) => e.type)).toEqual(["set.created", "draft"]);
    expect(page.events[1]!.questionRef).toMatchObject({ qid: "item-a#1", questionIndex: 1 });
    const outboxLines = (await readFile(outboxFile, "utf8")).trim().split("\n");
    expect(outboxLines).toHaveLength(3);
    const first = JSON.parse(outboxLines[0]!) as { eventId: string; outbox: { status: string; sink: string } };
    expect(first.eventId).toBe(e1.eventId);
    expect(first.outbox).toMatchObject({ status: "pending", sink: "turso-cloudflare" });
  });

  test("since cursor pages forward: eventId cursor and ISO cursor", async () => {
    const a = await appendDecisionEvent(eventsFile, outboxFile, { setId: "s1", type: "set.created", payload: {} });
    await appendDecisionEvent(eventsFile, outboxFile, { setId: "s1", type: "draft", payload: { n: 1 } });
    await appendDecisionEvent(eventsFile, outboxFile, { setId: "s1", type: "submit", payload: { n: 2 } });
    const afterFirst = await readDecisionEvents(eventsFile, "s1", { since: a.eventId });
    expect(afterFirst.events.map((e) => e.type)).toEqual(["draft", "submit"]);
    expect(afterFirst.nextCursor).toBe(afterFirst.events[1]!.eventId);
    const none = await readDecisionEvents(eventsFile, "s1", { since: afterFirst.nextCursor! });
    expect(none.count).toBe(0);
    const byTs = await readDecisionEvents(eventsFile, "s1", { since: "2999-01-01T00:00:00.000Z" });
    expect(byTs.count).toBe(0);
    const allByOldTs = await readDecisionEvents(eventsFile, "s1", { since: "2000-01-01T00:00:00.000Z" });
    expect(allByOldTs.count).toBe(3);
  });

  test("a torn final line never poisons the read", async () => {
    await appendDecisionEvent(eventsFile, outboxFile, { setId: "s1", type: "set.created", payload: {} });
    await appendFile(eventsFile, '{"eventId":"torn"', "utf8");
    const page = await readDecisionEvents(eventsFile, "s1");
    expect(page.count).toBe(1);
  });

  test("validateDecisionEventInput accepts draft/submit shapes, rejects bad types", () => {
    expect(
      validateDecisionEventInput({ type: "draft", questionRef: { ledgerId: "a", qid: "a#1" }, payload: { x: 1 } }).ok,
    ).toBe(true);
    expect(validateDecisionEventInput({ type: "keystroke", payload: {} }).ok).toBe(false);
    expect(validateDecisionEventInput({ type: "draft", questionRef: { ledgerId: "a" } }).ok).toBe(false);
    expect(validateDecisionEventInput(null).ok).toBe(false);
  });
});

describe("multi drafts (#9335)", () => {
  test("draft opts round-trip and validate", async () => {
    await raiseDecisionSet(file, v2Input());
    const up = await upsertDecisionDrafts(file, "set-v2-1", {
      questions: { "item-a#1": { opts: ["a", "b"], ft: "draft", updatedAt: "2026-11-12T00:00:00.000Z" } },
    });
    expect(up?.applied).toEqual(["item-a#1"]);
    expect((await getDecisionDrafts(file, "set-v2-1"))?.questions["item-a#1"]?.opts).toEqual(["a", "b"]);
    expect(
      validateDecisionDraftUpsert({
        questions: { "item-a#1": { opts: ["a"], updatedAt: "2026-11-12T00:00:00.000Z" } },
      }).ok,
    ).toBe(true);
  });
});
