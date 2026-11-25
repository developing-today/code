// #14084 S1 slice 2 — the needs-user list, "Send back", priority and
// blocked-by count over the staged pipeline.
//
// Oracle: the stage store runs its real SQL on SQLite through the same
// dialect bridge the Turso client uses (storeStatement + decodeStoreRow).
import { describe, expect, test } from "bun:test";
import { Database } from "bun:sqlite";
import { decodeStoreRow, storeStatement } from "../src/turso-stores.ts";
import { ItemStageStore, SEND_BACK_OPTION } from "../src/push-registry.ts";
import type { NeedsDrewQuestion } from "../src/needs-drew.ts";
import {
  buildNeedsUserList,
  needsUserItemRef,
  onQuestionAnswered,
  onQuestionRaised,
  parseBlockedByCount,
  parsePriority,
  questionOptions,
  setQuestionBlockedByCount,
  setQuestionPriority,
  stageRowsFor,
} from "../src/needs-user.ts";

function localClient(db: Database) {
  return {
    ref: "local-test",
    async query<T>(input: string): Promise<T[]> {
      const { sql, args } = storeStatement(input);
      return db
        .query(sql)
        .all(...args.map((a) => a.value))
        .map((r) => decodeStoreRow(r as Record<string, unknown>)) as T[];
    },
  };
}

function stepClock(start = Date.parse("2026-11-25T00:00:00.000Z")) {
  let t = start;
  return () => new Date((t += 1000));
}

function question(ledgerId: string, raisedAt: string, extra: Partial<NeedsDrewQuestion> = {}): NeedsDrewQuestion {
  return {
    ledgerId,
    question: `Question ${ledgerId}?`,
    options: [{ id: "yes", label: "Yes" }],
    links: [],
    token: `token-${ledgerId}`,
    raisedAt,
    source: "fleet-queue",
    ...extra,
  };
}

describe("send back is a standard answer", () => {
  test("every question offers Send back once, after its own options", () => {
    const q = question("a", "2026-11-25T00:00:00.000Z");
    expect(questionOptions(q)).toEqual([
      { id: "yes", label: "Yes" },
      { id: SEND_BACK_OPTION.id, label: SEND_BACK_OPTION.label },
    ]);
    const already = question("b", "2026-11-25T00:00:00.000Z", {
      options: [{ id: "send-back", label: "Return it" }],
    });
    expect(questionOptions(already)).toEqual([{ id: "send-back", label: "Return it" }]);
  });

  test("a raise escalates to needs-user; send back returns it to review; another answer resolves", async () => {
    const stages = new ItemStageStore(localClient(new Database(":memory:")), stepClock());
    const a = question("a", "2026-11-25T00:00:00.000Z");
    const b = question("b", "2026-11-25T00:00:01.000Z");
    await onQuestionRaised(stages, a);
    await onQuestionRaised(stages, a); // a duplicate raise changes nothing
    await onQuestionRaised(stages, b);
    expect((await stages.get(needsUserItemRef("a")))?.stage).toBe("needs-user");
    expect((await stages.events(needsUserItemRef("a"))).map((e) => e.to_stage)).toEqual([
      "needs-review",
      "needs-user",
    ]);

    expect(await onQuestionAnswered(stages, "a", "send-back")).toBe("needs-review");
    expect(await onQuestionAnswered(stages, "b", "yes")).toBe("resolved");
    expect((await stages.events(needsUserItemRef("a"))).at(-1)?.reason).toBe("sent back by the user");
  });

  test("an answered question is not raised into the pipeline", async () => {
    const stages = new ItemStageStore(localClient(new Database(":memory:")), stepClock());
    await onQuestionRaised(
      stages,
      question("done", "2026-11-25T00:00:00.000Z", { answer: { optionId: "yes", at: "x", via: "api" } }),
    );
    expect(await stages.get(needsUserItemRef("done"))).toBeNull();
  });

  test("a question raised before the pipeline still lands its answer on a recorded stage", async () => {
    const stages = new ItemStageStore(localClient(new Database(":memory:")), stepClock());
    expect(await onQuestionAnswered(stages, "old", undefined)).toBe("resolved");
    expect((await stages.events(needsUserItemRef("old"))).map((e) => e.to_stage)).toEqual([
      "needs-review",
      "needs-user",
      "resolved",
    ]);
  });
});

describe("the needs-user list", () => {
  test("orders by priority, then issues waiting, then age, and carries no answer token", async () => {
    const stages = new ItemStageStore(localClient(new Database(":memory:")), stepClock());
    const qs = [
      question("oldest", "2026-11-24T00:00:00.000Z"),
      question("blocking", "2026-11-24T01:00:00.000Z"),
      question("urgent", "2026-11-24T02:00:00.000Z"),
      question("legacy", "2026-11-23T00:00:00.000Z"),
    ];
    for (const q of qs.slice(0, 3)) await onQuestionRaised(stages, q);
    await setQuestionPriority(stages, "urgent", 1);
    await setQuestionBlockedByCount(stages, "blocking", 4);

    const list = buildNeedsUserList(qs, await stageRowsFor(stages, qs));
    expect(list.map((i) => i.ledgerId)).toEqual(["urgent", "blocking", "legacy", "oldest"]);
    expect(list[0]).toMatchObject({ priority: 1, blockedByCount: 0, stage: "needs-user" });
    expect(list[1]).toMatchObject({ priority: null, blockedByCount: 4 });
    expect(JSON.stringify(list)).not.toContain("token-");
  });

  test("answered and sent-back questions leave the list", async () => {
    const stages = new ItemStageStore(localClient(new Database(":memory:")), stepClock());
    const open = question("open", "2026-11-24T00:00:00.000Z");
    const back = question("back", "2026-11-24T00:00:00.000Z");
    const answered = question("answered", "2026-11-24T00:00:00.000Z", {
      answer: { optionId: "yes", at: "x", via: "api" },
    });
    for (const q of [open, back]) await onQuestionRaised(stages, q);
    await onQuestionAnswered(stages, "back", "send-back");
    const qs = [open, back, answered];
    expect(buildNeedsUserList(qs, await stageRowsFor(stages, qs)).map((i) => i.ledgerId)).toEqual(["open"]);
  });

  test("a priority set on a pre-pipeline question gives it a row and lands", async () => {
    const stages = new ItemStageStore(localClient(new Database(":memory:")), stepClock());
    const row = await setQuestionPriority(stages, "legacy", 2);
    expect(row).toMatchObject({ stage: "needs-user", priority: 2 });
    expect((await setQuestionPriority(stages, "legacy", null))?.priority).toBeNull();
  });

  test("body parsing", () => {
    expect(parsePriority(3)).toEqual({ ok: true, value: 3 });
    expect(parsePriority(null)).toEqual({ ok: true, value: null });
    for (const bad of [0, 6, 2.5, "1", undefined]) expect(parsePriority(bad).ok).toBe(false);
    expect(parseBlockedByCount(0)).toEqual({ ok: true, value: 0 });
    for (const bad of [-1, 1.5, "2", null]) expect(parseBlockedByCount(bad).ok).toBe(false);
  });
});
