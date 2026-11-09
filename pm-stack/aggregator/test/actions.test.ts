import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { spawn, type Subprocess } from "bun";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { ActionsStore, latestInFamily, validateNewAction, type DashAction, type NewAction } from "../src/actions.ts";
import { cardState } from "../src/card-state.ts";
import { buildAttention } from "../src/derive.ts";
import type { OrchestrationSection, ProjectPoolItem, SessionRecord, TierItem } from "../src/model.ts";
import { STANDING_IDENTITIES } from "../src/registry.ts";
import { renderAttention, renderProjects, renderSessions } from "../src/render.ts";
import { fixtureSource } from "../src/sources/fixtures.ts";

// --- Store + validation (#1152/#1154/#1156, coherence design Cluster 3) ------

const drew = { kind: "drew" as const, id: "drew", label: "Drew" };
const NOW = Date.parse("2026-10-27T12:00:00Z");
const iso = (ms: number) => new Date(ms).toISOString();

let tokenSeq = 0;
const newAction = (over: Partial<NewAction> = {}): NewAction => ({
  clientToken: `ct-test-${++tokenSeq}-abcdefgh`,
  kind: "triage",
  verb: "acknowledge",
  objectRef: "orch:item-1",
  payload: {},
  author: drew,
  ...over,
});

describe("dashboard actions store", () => {
  const dir = mkdtempSync(join(tmpdir(), "agg-actions-"));
  const file = join(dir, "actions.json");

  test("add/list/byObject/pending counts, dedupe, persistence", async () => {
    const store = new ActionsStore(file);
    const first = await store.add(newAction());
    expect(first.created).toBe(true);
    expect(first.action.id).toStartWith("da-");
    expect(first.action.state).toBe("new");
    // A repost of the same clientToken returns the existing record.
    const repost = await store.add({ ...first.action, payload: {} } as NewAction);
    expect(repost.created).toBe(false);
    expect(repost.action.id).toBe(first.action.id);
    const status = await store.add(
      newAction({
        kind: "status-request",
        verb: "request-status",
        objectRef: "gh:developing-today/code#1154",
        targetRef: "developing-today/code#1154",
        payload: { status: "Done" },
      }),
    );
    expect(status.created).toBe(true);
    expect(await store.pendingCounts()).toEqual({ triage: 1, statusRequests: 1, session: 0, preference: 0 });
    expect((await store.byObject()).get("orch:item-1")?.length).toBe(1);
    expect((await store.forObject("gh:developing-today/code#1154")).length).toBe(1);
    // A fresh store over the same file sees the same state.
    const reloaded = new ActionsStore(file);
    expect((await reloaded.all()).length).toBe(2);
  });

  test("markResult is consume-once: a second result is a no-op", async () => {
    const store = new ActionsStore(file);
    const { action } = await store.add(newAction({ objectRef: "orch:item-2" }));
    const applied = await store.markResult(action.id, {
      outcome: "applied",
      byRun: "orch-test-1",
      note: "ack carried",
    });
    expect(applied?.state).toBe("applied");
    expect(applied?.appliedByRun).toBe("orch-test-1");
    const again = await store.markResult(action.id, { outcome: "rejected", byRun: "orch-test-2", note: "retry" });
    expect(again?.state).toBe("applied"); // unchanged — the first result stands
    expect(again?.appliedByRun).toBe("orch-test-1");
    expect(await store.markResult("da-nope", { outcome: "applied", byRun: "orch-test-1" })).toBeNull();
    expect((await store.pendingCounts()).triage).toBe(1); // only the first test's ack remains new… see below
  });

  test("markResultTransition distinguishes a first result from a retry", async () => {
    const store = new ActionsStore(file);
    const { action } = await store.add(newAction({ objectRef: "orch:item-transition" }));
    const first = await store.markResultTransition(action.id, { outcome: "applied", byRun: "orch-test" });
    const retry = await store.markResultTransition(action.id, { outcome: "rejected", byRun: "retry" });
    expect(first?.transitioned).toBe(true);
    expect(first?.action.state).toBe("applied");
    expect(retry?.transitioned).toBe(false);
    expect(retry?.action.state).toBe("applied");
  });

  test("markResult caps never split a surrogate pair (#7711)", async () => {
    const dir = mkdtempSync(join(tmpdir(), "agg-actions-result-surrogate-"));
    const store = new ActionsStore(join(dir, "actions.json"));
    // byRun cap 200: 199 units + an astral character straddling the cap is
    // dropped whole, never stored as a lone high surrogate.
    const a1 = await store.add(newAction({ objectRef: "orch:item-result-byRun" }));
    const byRun = await store.markResult(a1.action.id, {
      outcome: "applied",
      byRun: "r".repeat(199) + "\u{1F600}" + "tail",
    });
    expect(byRun?.appliedByRun?.length).toBe(199);
    expect(byRun?.appliedByRun?.endsWith("r")).toBe(true);
    // note cap 2000: same rule at the larger cap.
    const a2 = await store.add(newAction({ objectRef: "orch:item-result-note" }));
    const noted = await store.markResult(a2.action.id, {
      outcome: "applied",
      byRun: "orch-test",
      note: "n".repeat(1999) + "\u{1F600}" + "tail",
    });
    expect(noted?.resultNote?.length).toBe(1999);
    expect(noted?.resultNote?.endsWith("n")).toBe(true);
    // A pair that fits exactly inside the cap is kept whole.
    const a3 = await store.add(newAction({ objectRef: "orch:item-result-fit" }));
    const fit = "n".repeat(1998) + "\u{1F600}";
    const fitted = await store.markResult(a3.action.id, { outcome: "applied", byRun: "orch-test", note: fit });
    expect(fitted?.resultNote).toBe(fit);
  });

  test("answer question slots dedupe option and survey submissions with an audit pointer", async () => {
    const dir = mkdtempSync(join(tmpdir(), "agg-answer-dedupe-"));
    const store = new ActionsStore(join(dir, "actions.json"));
    const objectRef = "orch:shared-question";

    const option = await store.add(
      newAction({ verb: "answer", objectRef, payload: { optionId: "yes", questionId: "q1" } }),
    );
    const otherQuestion = await store.add(
      newAction({ verb: "answer", objectRef, payload: { answers: [{ questionId: "q2", value: "notes" }] } }),
    );
    expect(otherQuestion.action.state).toBe("new");

    const surveyDuplicate = await store.add(
      newAction({ verb: "answer", objectRef, payload: { answers: [{ questionId: "q1", value: "no" }] } }),
    );
    expect(surveyDuplicate.action.state).toBe("rejected");
    expect(surveyDuplicate.duplicateOf).toBe(option.action.id);
    expect(surveyDuplicate.action.duplicateOf).toBe(option.action.id);
    expect(surveyDuplicate.action.resultNote).toContain(option.action.id);

    await store.markResult(option.action.id, { outcome: "rejected", byRun: "orch-test", note: "not applied" });
    const retryAfterReject = await store.add(
      newAction({ verb: "answer", objectRef, payload: { answers: [{ questionId: "q1", value: "no" }] } }),
    );
    expect(retryAfterReject.action.state).toBe("new");

    const structuredFirst = await store.add(
      newAction({
        verb: "answer",
        objectRef: "orch:structured-first",
        payload: { answers: [{ questionId: "q3", value: "yes" }] },
      }),
    );
    const optionSecond = await store.add(
      newAction({
        verb: "answer",
        objectRef: "orch:structured-first",
        payload: { optionId: "yes", questionId: "q3" },
      }),
    );
    expect(optionSecond.duplicateOf).toBe(structuredFirst.action.id);

    const applied = await store.add(
      newAction({
        verb: "answer",
        objectRef: "orch:applied",
        payload: { answers: [{ questionId: "q4", value: "yes" }] },
      }),
    );
    await store.markResult(applied.action.id, { outcome: "applied", byRun: "orch-test" });
    const appliedDuplicate = await store.add(
      newAction({ verb: "answer", objectRef: "orch:applied", payload: { optionId: "no", questionId: "q4" } }),
    );
    expect(appliedDuplicate.duplicateOf).toBe(applied.action.id);

    const concurrent = await Promise.all([
      store.add(
        newAction({ verb: "answer", objectRef: "orch:concurrent", payload: { optionId: "yes", questionId: "q5" } }),
      ),
      store.add(
        newAction({
          verb: "answer",
          objectRef: "orch:concurrent",
          payload: { answers: [{ questionId: "q5", value: "yes" }] },
        }),
      ),
    ]);
    expect(concurrent.filter((result) => result.action.state === "new")).toHaveLength(1);
    expect(concurrent.filter((result) => result.duplicateOf)).toHaveLength(1);
  });

  test("text answers dedupe only while pending: the next reply lands after the first is applied", async () => {
    const store = new ActionsStore(join(mkdtempSync(join(tmpdir(), "agg-answer-text-")), "actions.json"));
    const objectRef = "session:reply-turns";
    const first = await store.add(
      newAction({ kind: "session", verb: "answer", objectRef, payload: { text: "first" } }),
    );
    expect(first.action.state).toBe("new");
    const doubleSubmit = await store.add(
      newAction({ kind: "session", verb: "answer", objectRef, payload: { text: "first" } }),
    );
    expect(doubleSubmit.duplicateOf).toBe(first.action.id);
    await store.markResult(first.action.id, { outcome: "applied", byRun: "orch-test" });
    const nextTurn = await store.add(
      newAction({ kind: "session", verb: "answer", objectRef, payload: { text: "second" } }),
    );
    expect(nextTurn.duplicateOf).toBeUndefined();
    expect(nextTurn.action.state).toBe("new");
  });

  test("duplicate detection scans past malformed persisted answer shapes", async () => {
    const dir = mkdtempSync(join(tmpdir(), "agg-answer-malformed-"));
    const file = join(dir, "actions.json");
    const raw = (id: string, payload: unknown) => ({
      id,
      clientToken: `ct-${id}`,
      kind: "triage",
      verb: "answer",
      objectRef: "orch:malformed-scan",
      payload,
      author: drew,
      createdAt: iso(NOW),
      state: "new",
    });
    writeFileSync(
      file,
      JSON.stringify({
        version: 1,
        actions: [
          raw("bad-null-entry", { answers: [null] }),
          raw("bad-answers-object", { answers: { questionId: "q1", value: "x" } }),
          raw("bad-answers-string", { answers: "nope" }),
          raw("good", { answers: [{ questionId: "q1", value: "good" }] }),
        ],
      }),
    );
    const store = new ActionsStore(file);
    const fresh = await store.add(
      newAction({
        verb: "answer",
        objectRef: "orch:malformed-scan",
        payload: { answers: [{ questionId: "q9", value: "new" }] },
      }),
    );
    expect(fresh.action.state).toBe("new");
    const dup = await store.add(
      newAction({
        verb: "answer",
        objectRef: "orch:malformed-scan",
        payload: { answers: [{ questionId: "q1", value: "again" }] },
      }),
    );
    expect(dup.duplicateOf).toBe("good");
  });

  test("supersede transitions a still-new action; applied history stands", async () => {
    const store = new ActionsStore(file);
    const snooze = await store.add(
      newAction({ verb: "snooze", objectRef: "orch:item-3", payload: { until: iso(NOW + 86_400_000) } }),
    );
    const unsnooze = await store.add(
      newAction({
        verb: "snooze",
        objectRef: "orch:item-3",
        payload: { until: iso(NOW) },
        supersedes: snooze.action.id,
      }),
    );
    const all = await store.all();
    expect(all.find((a) => a.id === snooze.action.id)?.state).toBe("superseded");
    expect(unsnooze.action.supersedes).toBe(snooze.action.id);
    // The convergent read: the latest non-superseded snooze wins.
    expect(
      latestInFamily(
        all.filter((a) => a.objectRef === "orch:item-3"),
        "snooze",
      )?.id,
    ).toBe(unsnooze.action.id);
  });

  test("cardState exposes option answers under their canonical question id", () => {
    const action: DashAction = {
      id: "da-option-q1",
      clientToken: "ct-option-q1-abcdefgh",
      kind: "triage",
      verb: "answer",
      objectRef: "orch:item-q1",
      payload: { optionId: "yes", questionId: "q1" },
      author: drew,
      createdAt: iso(NOW),
      state: "new",
    };
    const state = cardState([action], NOW);
    expect(state.answered.get("q1")).toBe("yes");
    expect(state.answerActionsByQuestion.get("q1")).toBe(action);
  });

  test("drops malformed payload records on load and cardState ignores malformed answer shapes", async () => {
    const dir = mkdtempSync(join(tmpdir(), "agg-actions-payload-"));
    const file = join(dir, "actions.json");
    const rawAction = (id: string, payload: unknown) => ({
      id,
      clientToken: `ct-${id}`,
      kind: "triage",
      verb: "answer",
      objectRef: "orch:payload-test",
      payload,
      author: drew,
      createdAt: iso(NOW),
      state: "new",
    });
    const malformedPayload = rawAction("payload-null", null);
    writeFileSync(
      file,
      JSON.stringify({
        version: 1,
        actions: [
          malformedPayload,
          rawAction("payload-missing", undefined),
          rawAction("payload-array", []),
          rawAction("answers-null-entry", { answers: [null], optionId: 42 }),
          rawAction("answers-object", { answers: { questionId: "bad", value: "bad" } }),
          rawAction("answers-string", { answers: "not answers" }),
          rawAction("valid", { answers: [{ questionId: "q1", value: "good" }], optionId: "choice-1" }),
        ],
      }),
    );

    const store = new ActionsStore(file);
    const loaded = await store.all();
    expect(loaded.map((a) => a.id)).toEqual(["answers-null-entry", "answers-object", "answers-string", "valid"]);
    expect(() => cardState(loaded, NOW)).not.toThrow();
    const state = cardState(loaded, NOW);
    expect([...state.answered]).toEqual([["q1", "good"]]);
    expect(state.answeredOptionId).toBe("choice-1");
    const malformedOnly = loaded.filter((a) => a.id !== "valid");
    expect(cardState(malformedOnly, NOW).answered).toEqual(new Map());
    expect(cardState(malformedOnly, NOW).answeredOptionId).toBeUndefined();
    const nullPayloadState = cardState([malformedPayload as unknown as DashAction], NOW);
    expect(nullPayloadState.answered).toEqual(new Map());
    const malformedSnooze = { ...rawAction("snooze-until-number", { until: 123 }), verb: "snooze" };
    expect(cardState([malformedSnooze as unknown as DashAction], NOW).suppressed).toBeNull();

    await store.add(newAction({ objectRef: "orch:payload-test" }));
    const persisted = JSON.parse(readFileSync(file, "utf8")) as { actions: { id: string }[] };
    expect(persisted.actions.map((a) => a.id)).not.toContain("payload-null");
  });
});

describe("dashboard actions validation", () => {
  const base = {
    clientToken: "ct-valid-token-1",
    kind: "triage",
    verb: "acknowledge",
    objectRef: "orch:item-1",
    author: drew,
    payload: {},
  };
  const v = (over: Record<string, unknown>) => validateNewAction({ ...base, ...over });

  test("accepts the canonical verb set incl. the addendum verbs", () => {
    expect(v({}).ok).toBe(true);
    expect(v({ verb: "snooze", payload: { until: iso(NOW + 3_600_000) } }).ok).toBe(true);
    expect(v({ verb: "escalate", payload: { tier: "needs-drew", reason: "needs a decision" } }).ok).toBe(true);
    expect(v({ verb: "answer", payload: { text: "go with option B" } }).ok).toBe(true);
    expect(v({ verb: "answer", payload: { optionId: "opt-2" } }).ok).toBe(true);
    expect(v({ verb: "answer", payload: { answers: [{ questionId: "q1", value: "yes" }] } }).ok).toBe(true);
    expect(v({ verb: "delegate", payload: { mode: "first-pass" } }).ok).toBe(true);
    expect(v({ verb: "delegate", payload: { mode: "auto" } }).ok).toBe(true);
    expect(v({ verb: "dismiss", payload: { reason: "already handled in #1150" } }).ok).toBe(true);
    expect(v({ verb: "reject-proposal", payload: { reason: "wrong scope" } }).ok).toBe(true);
  });

  test("rejects malformed and under-specified actions at the boundary", () => {
    expect(v({ author: undefined }).ok).toBe(false); // anonymous authorship
    expect(v({ author: "drew" }).ok).toBe(false);
    expect(v({ clientToken: "short" }).ok).toBe(false);
    expect(v({ clientToken: "has whitespace token" }).ok).toBe(false);
    expect(v({ kind: "session", verb: "acknowledge" }).ok).toBe(false); // verb/kind mismatch
    expect(v({ verb: "snooze", payload: {} }).ok).toBe(false); // snooze needs until
    expect(v({ verb: "snooze", payload: { until: "not a date" } }).ok).toBe(false);
    expect(v({ verb: "escalate", payload: {} }).ok).toBe(false); // escalate needs tier
    expect(v({ verb: "answer", payload: {} }).ok).toBe(false); // answer needs content
    expect(v({ verb: "delegate", payload: {} }).ok).toBe(false); // delegate needs mode
    expect(v({ verb: "delegate", payload: { mode: "yolo" } }).ok).toBe(false);
    expect(v({ verb: "dismiss", payload: {} }).ok).toBe(false); // dismiss REQUIRES a reason
    expect(v({ verb: "dismiss", payload: { reason: "  " } }).ok).toBe(false);
    expect(v({ verb: "reject-proposal", payload: {} }).ok).toBe(false);
    expect(v({ payload: { answers: [{ questionId: "q1" }] }, verb: "answer" }).ok).toBe(false);
    expect(v({ objectRef: "has space" }).ok).toBe(false);
  });

  test("caps never split a surrogate pair (#7694)", () => {
    // reason cap 2000: 1999 units + an astral character straddling the cap.
    const straddle = validateNewAction({
      ...base,
      verb: "dismiss",
      payload: { reason: "x".repeat(1999) + "\u{1F600}" + "tail" },
    });
    expect(straddle.ok).toBe(true);
    if (straddle.ok) {
      const reason = (straddle.value.payload as { reason?: string }).reason ?? "";
      expect(reason.length).toBe(1999);
      expect(reason.endsWith("x")).toBe(true);
    }
    // A pair that fits exactly inside the cap is kept whole.
    const fits = validateNewAction({
      ...base,
      verb: "dismiss",
      payload: { reason: "x".repeat(1998) + "\u{1F600}" },
    });
    expect(fits.ok).toBe(true);
    if (fits.ok) {
      const reason = (fits.value.payload as { reason?: string }).reason ?? "";
      expect(reason.length).toBe(2000);
      expect(reason.endsWith("\u{1F600}")).toBe(true);
    }
    // Small cap, same rule: optionId cap 200.
    const opt = validateNewAction({
      ...base,
      verb: "answer",
      payload: { optionId: "o".repeat(199) + "\u{1F600}" + "tail" },
    });
    expect(opt.ok).toBe(true);
    if (opt.ok) {
      const optionId = (opt.value.payload as { optionId?: string }).optionId ?? "";
      expect(optionId.length).toBe(199);
    }
  });

  test("status-request: gh object, status required, targetRef derived", () => {
    const ok = validateNewAction({
      clientToken: "ct-valid-token-2",
      kind: "status-request",
      verb: "request-status",
      objectRef: "gh:developing-today/code#1154",
      author: drew,
      payload: { status: "In Review", reason: "PR is up" },
    });
    expect(ok.ok).toBe(true);
    if (ok.ok) expect(ok.value.targetRef).toBe("developing-today/code#1154");
    expect(
      validateNewAction({ ...base, kind: "status-request", verb: "request-status", payload: { status: "Done" } }).ok,
    ).toBe(false); // objectRef is orch:, not gh:
    expect(
      validateNewAction({
        clientToken: "ct-valid-token-3",
        kind: "status-request",
        verb: "request-status",
        objectRef: "gh:developing-today/code#1154",
        author: drew,
        payload: {},
      }).ok,
    ).toBe(false); // no status
  });

  test("session actions: sessionId + matching objectRef required", () => {
    const ok = validateNewAction({
      clientToken: "ct-valid-token-4",
      kind: "session",
      verb: "answer",
      objectRef: "session:ses_1",
      sessionId: "ses_1",
      sessionSource: "opencode-laptop",
      author: drew,
      payload: { text: "proceed" },
    });
    expect(ok.ok).toBe(true);
    expect(
      validateNewAction({
        clientToken: "ct-valid-token-5",
        kind: "session",
        verb: "poke",
        objectRef: "session:ses_1",
        sessionId: "ses_2",
        author: drew,
        payload: {},
      }).ok,
    ).toBe(false); // objectRef/sessionId mismatch
    expect(
      validateNewAction({
        clientToken: "ct-valid-token-6",
        kind: "session",
        verb: "answer",
        objectRef: "session:ses_1",
        sessionId: "ses_1",
        author: drew,
        payload: {},
      }).ok,
    ).toBe(false); // a session answer needs text
  });
});

// --- Render wiring ------------------------------------------------------------

const tierItem = (over: Partial<TierItem>): TierItem => ({
  id: "x",
  kind: "task",
  title: "Item",
  source: "github",
  reason: "Because.",
  since: iso(NOW),
  lastSeenAt: iso(NOW),
  ...over,
});

const section = (over: Partial<OrchestrationSection>): OrchestrationSection => ({
  version: 1,
  generatedAt: iso(NOW),
  runId: "orch-test",
  provenance: { sources: ["test run"] },
  tiers: { needsDrew: [], needsOrchestrator: [] },
  dependencies: [],
  goals: [],
  actions: [],
  ...over,
});

const identities = STANDING_IDENTITIES.map((i) => ({ ...i }));

const mkAction = (over: Partial<DashAction>): DashAction => ({
  id: "da-test-1",
  clientToken: "ct-render-1",
  kind: "triage",
  verb: "acknowledge",
  objectRef: "orch:goal_plaid",
  payload: {},
  author: drew,
  createdAt: iso(NOW - 12 * 60_000),
  state: "new",
  ...over,
});

describe("attention action controls (#1152/#1156)", () => {
  const SECTION = section({
    tiers: {
      needsDrew: [
        tierItem({
          id: "goal_plaid",
          kind: "task",
          title: "Link the Citi accounts in Plaid",
          source: "tracked",
          reason: "Only the user can complete the bank sign-in.",
        }),
      ],
      needsOrchestrator: [],
    },
  });

  test("tier cards carry acknowledge / snooze / de-escalate forms posting DashActions", async () => {
    const src = fixtureSource();
    const view = buildAttention(await src.attention(), SECTION, NOW);
    const html = renderAttention(view, await src.coverage(), src.provenance(), undefined, undefined, {
      identities,
      actionsByObject: new Map(),
    });
    expect(html).toContain('data-kind="triage"');
    expect(html).toContain('data-verb="acknowledge"');
    expect(html).toContain('data-verb="snooze"');
    expect(html).toContain('data-verb="de-escalate"');
    expect(html).toContain('data-object="orch:goal_plaid"');
    expect(html).toContain("/api/actions"); // ACTION_SCRIPT is on the page
  });

  test("bulk triage (#1277): queue cards carry picks and the bulk bar posts to /api/actions/batch", async () => {
    const src = fixtureSource();
    const view = buildAttention(await src.attention(), SECTION, NOW);
    const html = renderAttention(view, await src.coverage(), src.provenance(), undefined, undefined, {
      identities,
      actionsByObject: new Map(),
    });
    expect(html).toContain('id="bulkbar"');
    expect(html).toContain('class="bulkpick"');
    expect(html).toContain('data-object="orch:goal_plaid"');
    expect(html).toContain("Review batch");
    expect(html).toContain("/api/actions/batch"); // BULK_SCRIPT is on the page
    expect(html).toContain("encoder.encode(JSON.stringify(a)).byteLength"); // pack by UTF-8 bytes, matching the request cap
    // Without identities there is no action surface \u2014 and no bulk bar.
    const anon = renderAttention(view, await src.coverage(), src.provenance(), undefined, undefined, {
      actionsByObject: new Map(),
    });
    expect(anon).not.toContain('id="bulkbar"');
    expect(anon).not.toContain('class="bulkpick"');
  });

  test("stale tier source: section-item controls disabled with reason; session items stay enabled", async () => {
    const src = fixtureSource();
    // A stale section still serves its last-published items — exactly
    // the state where acting on one is unsafe (it may not exist in the
    // publisher's current view). Absent/invalid sources serve no
    // section items at all, so there is nothing to disable there.
    const view = buildAttention(await src.attention(), SECTION, NOW, {
      state: "stale",
      runId: SECTION.runId,
      generatedAt: SECTION.generatedAt,
    });
    expect(view.tierSource.state).toBe("stale");
    expect(view.drewExtra.length).toBeGreaterThan(0);
    const html = renderAttention(view, await src.coverage(), src.provenance(), undefined, undefined, {
      identities,
      actionsByObject: new Map(),
    });
    expect(html).toContain("Actions unavailable");
    expect(html).toContain("the tier source is stale");
    expect(html).toContain("<fieldset disabled>");
    // Session-derived cards (the live feed) keep enabled controls.
    expect(html).toContain('data-verb="poke"');
  });

  test("pending + applied states render as badges; applied snooze lands in the Snoozed strip", async () => {
    const src = fixtureSource();
    const view = buildAttention(await src.attention(), SECTION, NOW);
    const pending = mkAction({});
    // Render-time suppression compares the snooze's `until` against the
    // wall clock (renderAttention has no injected clock), so the fixture's
    // snooze must still be live when the test runs — NOW is a fixed past
    // instant and a snooze measured from it would have expired by now.
    const snooze = mkAction({
      id: "da-test-2",
      verb: "snooze",
      objectRef: "orch:gone_item",
      state: "applied",
      appliedAt: iso(NOW - 3_600_000),
      appliedByRun: "orch-test",
      payload: { until: iso(Date.now() + 86_400_000) },
    });
    const acked = mkAction({
      id: "da-test-3",
      objectRef: "orch:goal_plaid",
      state: "applied",
      appliedAt: iso(NOW - 3_600_000),
      appliedByRun: "orch-test",
    });
    const html = renderAttention(view, await src.coverage(), src.provenance(), undefined, undefined, {
      identities,
      actionsByObject: new Map<string, DashAction[]>([
        ["orch:goal_plaid", [pending, acked]],
        ["orch:gone_item", [snooze]],
      ]),
    });
    expect(html).toContain("⏳ acknowledge requested");
    expect(html).toContain("✓ acknowledged");
    expect(html).toContain("Snoozed by you (1)");
    expect(html).toContain("orch:gone_item");
    expect(html).toContain('data-verb="snooze"'); // the unsnooze control rides the strip
  });

  test("retired jules source: no keeper-mediated session cards", async () => {
    const src = fixtureSource();
    const view = buildAttention(await src.attention(), SECTION, NOW);
    const html = renderAttention(view, await src.coverage(), src.provenance(), undefined, undefined, {
      identities,
      actionsByObject: new Map(),
    });
    // Jules was retired (code#5098): the fixture feed carries no
    // jules sessions and the keeper-mediated treatment is gone.
    expect(html).not.toContain("keeper-mediated");
  });
});

describe("status-request + session renders (#1154/#1156)", () => {
  const poolItem = (): ProjectPoolItem => ({
    id: "PVTI_test",
    url: "https://github.com/orgs/developing-today/projects/4/views/1?pane=item&id=1",
    projectUrl: "https://github.com/orgs/developing-today/projects/4",
    ref: "developing-today/code#1154",
    repo: "developing-today/code",
    number: 1154,
    issueUrl: "https://github.com/developing-today/code/issues/1154",
    title: "Request status change action",
    state: "open",
    contentKind: "issue",
    status: "Todo",
    area: null,
    horizon: null,
    kind: null,
  });

  test("pool card carries the request control; a pending request badges", () => {
    const pending = mkAction({
      kind: "status-request",
      verb: "request-status",
      objectRef: "gh:developing-today/code#1154",
      targetRef: "developing-today/code#1154",
      payload: { status: "Done" },
    });
    const html = renderProjects([poolItem()], fixtureSource().provenance(), {
      identities,
      actionsByObject: new Map([["gh:developing-today/code#1154", [pending]]]),
    });
    expect(html).toContain('data-kind="status-request"');
    expect(html).toContain('data-verb="request-status"');
    expect(html).toContain("Request status change");
    expect(html).toContain("⏳ status → Done requested");
    expect(html).toContain("A request, not a write");
  });

  test("retired jules registry row: renders as history, without controls", () => {
    const live: SessionRecord = {
      id: "jules-live-1",
      source: "jules",
      label: "Jules session live",
      agentId: "jules",
      agentLabel: "Jules",
      parentId: null,
      status: "AWAITING_USER_FEEDBACK",
      lastSeenAt: iso(Date.now() - 5 * 60_000),
      taskRefs: [],
    };
    const stale: SessionRecord = { ...live, id: "jules-old-1", lastSeenAt: iso(Date.now() - 30 * 3_600_000) };
    const html = renderSessions([live, stale], fixtureSource().provenance(), undefined, {
      identities,
      actionsByObject: new Map(),
    });
    expect(html).toContain("jules-live-1");
    expect(html).not.toContain("keeper-mediated");
    // No action box for either row (data-session-id rides the box):
    // the retired source has no verb support, and a stale row never
    // gets controls.
    expect(html).not.toContain('data-session-id="jules-live-1"');
    expect(html).not.toContain('data-session-id="jules-old-1"');
  });
});

// --- HTTP round-trip with a fixture consumer ----------------------------------

let proc: Subprocess<"ignore", "pipe", "inherit"> | null = null;
let port = 0;
const tmp = mkdtempSync(join(tmpdir(), "agg-actions-"));

beforeAll(async () => {
  port = 6300 + Math.floor(Math.random() * 700);
  writeFileSync(join(tmp, "comments.json"), JSON.stringify({ version: 1, comments: [] }));
  proc = spawn({
    cmd: ["bun", "run", "src/server.ts"],
    cwd: import.meta.dir + "/..",
    env: {
      ...Bun.env,
      PM_AGG_PORT: String(port),
      PM_AGG_SOURCE: "fixtures",
      PM_AGG_COMMENTS_FILE: join(tmp, "comments.json"),
      PM_AGG_ACTIONS_FILE: join(tmp, "actions.json"),
      PM_AGG_REGISTRY_FILE: join(tmp, "registry.json"),
    },
    stdout: "pipe",
    stderr: "inherit",
  });
  const deadline = Date.now() + 8000;
  let ready = false;
  while (Date.now() < deadline) {
    try {
      const res = await fetch(`http://127.0.0.1:${port}/api/health`);
      if (res.ok) {
        ready = true;
        break;
      }
    } catch {
      /* not up yet */
    }
    await Bun.sleep(120);
  }
  expect(ready).toBe(true);
});

afterAll(() => {
  proc?.kill();
});

describe("actions HTTP round-trip (fixture consumer)", () => {
  const post = (path: string, body: unknown) =>
    fetch(`http://127.0.0.1:${port}${path}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    });

  test("post → poll new → result applied → re-render shows the applied state", async () => {
    const body = {
      clientToken: "ct-http-roundtrip-1",
      kind: "triage",
      verb: "acknowledge",
      objectRef: "orch:http-item",
      author: drew,
      payload: {},
    };
    const created = await post("/api/actions", body);
    expect(created.status).toBe(201);
    const { action } = (await created.json()) as { action: DashAction };
    expect(action.state).toBe("new");

    // Dedupe: the same clientToken reposts to the same record.
    const repost = await post("/api/actions", body);
    expect(repost.status).toBe(200);
    const repostBody = (await repost.json()) as { action: DashAction; deduped?: boolean };
    expect(repostBody.deduped).toBe(true);
    expect(repostBody.action.id).toBe(action.id);

    // The consumer's poll: state=new, kind=triage.
    const poll = (await (await fetch(`http://127.0.0.1:${port}/api/actions.json?kind=triage&state=new`)).json()) as {
      actions: DashAction[];
      pending: { triage: number };
    };
    expect(poll.actions.map((a) => a.id)).toContain(action.id);
    expect(poll.pending.triage).toBeGreaterThanOrEqual(1);

    // The consumer reports the result; the record transitions once.
    const result = await post("/api/actions/result", {
      id: action.id,
      outcome: "applied",
      byRun: "orch-fixture-1",
      note: "ack carried in the next published tier entry",
    });
    expect(result.status).toBe(200);
    const { action: applied } = (await result.json()) as { action: DashAction };
    expect(applied.state).toBe("applied");
    expect(applied.appliedByRun).toBe("orch-fixture-1");

    // A retried result report is a no-op (consume-once).
    const retry = await post("/api/actions/result", { id: action.id, outcome: "rejected", byRun: "orch-fixture-2" });
    const { action: afterRetry } = (await retry.json()) as { action: DashAction };
    expect(afterRetry.state).toBe("applied");

    // Unknown id 404s; malformed posts 400.
    expect((await post("/api/actions/result", { id: "da-nope", outcome: "applied", byRun: "x" })).status).toBe(404);
    expect(
      (
        await post("/api/actions", {
          kind: "triage",
          verb: "dismiss",
          objectRef: "orch:x",
          author: drew,
          payload: {},
          clientToken: "ct-http-bad-1",
        })
      ).status,
    ).toBe(400);
    expect((await post("/api/actions", { ...body, clientToken: "ct-http-bad-2", author: undefined })).status).toBe(400);
  });

  test("/api/actions.json?state=all returns applied history; unknown state 400s (#4620)", async () => {
    const body = {
      clientToken: "ct-http-state-all-1",
      kind: "triage",
      verb: "acknowledge",
      objectRef: "orch:state-all-item",
      author: drew,
      payload: {},
    };
    const created = await post("/api/actions", body);
    expect(created.status).toBe(201);
    const { action } = (await created.json()) as { action: DashAction };
    const result = await post("/api/actions/result", {
      id: action.id,
      outcome: "applied",
      byRun: "orch-fixture-state-all",
      note: "applied for the #4620 history-read regression test",
    });
    expect(result.status).toBe(200);

    const listFor = async (qs: string) =>
      ((await (await fetch(`http://127.0.0.1:${port}/api/actions.json?${qs}`)).json()) as { actions: DashAction[] })
        .actions.map((a) => a.id);

    // The regression: state=all must return the applied record, not [].
    expect(await listFor("state=all")).toContain(action.id);
    // Concrete history states return history too.
    expect(await listFor("state=applied")).toContain(action.id);
    // state=new behaviour is unchanged: the applied record is not pending.
    expect(await listFor("state=new")).not.toContain(action.id);
    // An unknown state is a 400, never a silent false zero.
    expect((await fetch(`http://127.0.0.1:${port}/api/actions.json?state=bogus`)).status).toBe(400);
  });

  test("duplicate answer response includes the persisted audit pointer", async () => {
    const body = {
      clientToken: "ct-session-answer-1",
      kind: "session",
      verb: "answer",
      objectRef: "session:duplicate-answer",
      sessionId: "duplicate-answer",
      sessionSource: "opencode-laptop",
      author: drew,
      payload: { text: "continue" },
    };
    const first = await post("/api/actions", body);
    expect(first.status).toBe(201);
    const { action: original } = (await first.json()) as { action: DashAction };

    const duplicate = await post("/api/actions", { ...body, clientToken: "ct-session-answer-2" });
    expect(duplicate.status).toBe(409);
    const recorded = (await duplicate.json()) as { action: DashAction; duplicateOf: string };
    expect(recorded.action.state).toBe("rejected");
    expect(recorded.action.duplicateOf).toBe(original.id);
    expect(recorded.duplicateOf).toBe(original.id);
    // Replaying the persisted duplicate attempt (same client token) keeps
    // the 409 semantics instead of reading as a fresh success.
    const replay = await post("/api/actions", { ...body, clientToken: "ct-session-answer-2" });
    expect(replay.status).toBe(409);
    await post("/api/actions/result", { id: original.id, outcome: "applied", byRun: "jules-fixture" });
    // Once the first reply is applied, the session's next reply is a new
    // turn, not a duplicate (Codex P1 on PR #4024).
    const nextTurn = await post("/api/actions", { ...body, clientToken: "ct-session-answer-3" });
    expect(nextTurn.status).toBe(201);
    const { action: nextAction } = (await nextTurn.json()) as { action: DashAction };
    await post("/api/actions/result", { id: nextAction.id, outcome: "applied", byRun: "jules-fixture" });
  });

  test("/attention serves the action forms; /api/actions.json is inspectable", async () => {
    const html = await (await fetch(`http://127.0.0.1:${port}/attention`)).text();
    expect(html).toContain("aform");
    expect(html).toContain("/api/actions");
    const list = (await (await fetch(`http://127.0.0.1:${port}/api/actions.json`)).json()) as {
      actions: DashAction[];
      pending: { triage: number; statusRequests: number; session: number; preference: number };
    };
    expect(list.actions.length).toBeGreaterThanOrEqual(1);
    expect(list.pending).toEqual({ triage: 0, statusRequests: 0, session: 0, preference: 0 });
  });

  test("POST /api/actions/batch: mixed batch records valid items individually, reports failures by ref (#1277)", async () => {
    const mk = (clientToken: string, over: Record<string, unknown> = {}) => ({
      clientToken,
      kind: "triage",
      verb: "acknowledge",
      objectRef: "orch:bulk-ok",
      author: drew,
      payload: {},
      ...over,
    });
    const res = await post("/api/actions/batch", {
      actions: [
        mk("ct-batch-ok-1"),
        mk("ct-batch-ok-2", { objectRef: "orch:bulk-ok-2" }),
        mk("ct-batch-bad-1", { objectRef: "orch:bulk-bad", verb: "snooze", payload: {} }), // snooze without until
        mk("ct-batch-bad-2", { objectRef: "orch:bulk-bad-2", verb: "explode" }),
      ],
    });
    expect(res.status).toBe(200);
    const body = (await res.json()) as {
      results: { index: number; objectRef: string | null; ok: boolean; action?: DashAction; error?: string }[];
      recorded: number;
      failed: number;
    };
    expect(body.recorded).toBe(2);
    expect(body.failed).toBe(2);
    expect(body.results.map((r) => r.ok)).toEqual([true, true, false, false]);
    expect(body.results[2].objectRef).toBe("orch:bulk-bad");
    expect(body.results[2].error).toContain("until");
    expect(body.results[3].error).toContain("verb");
    // Each accepted item is its OWN store entry (no opaque batch record).
    const ids = body.results.filter((r) => r.ok).map((r) => r.action!.id);
    expect(new Set(ids).size).toBe(2);
    const stored = (await (await fetch(`http://127.0.0.1:${port}/api/actions.json?kind=triage`)).json()) as {
      actions: DashAction[];
    };
    expect(stored.actions.filter((a) => a.objectRef === "orch:bulk-ok").length).toBe(1);
    expect(stored.actions.filter((a) => a.objectRef === "orch:bulk-ok-2").length).toBe(1);
    // Failed items appended nothing \u2014 they remain in Needs-you.
    expect(stored.actions.some((a) => a.objectRef === "orch:bulk-bad")).toBe(false);
    expect(stored.actions.some((a) => a.objectRef === "orch:bulk-bad-2")).toBe(false);
    // A repost of the same clientToken dedupes per item, not per batch.
    const repost = await post("/api/actions/batch", { actions: [mk("ct-batch-ok-1")] });
    const repostBody = (await repost.json()) as { results: { ok: boolean; deduped?: boolean }[]; recorded: number };
    expect(repostBody.recorded).toBe(1);
    expect(repostBody.results[0].deduped).toBe(true);
    const after = (await (await fetch(`http://127.0.0.1:${port}/api/actions.json?kind=triage`)).json()) as {
      actions: DashAction[];
    };
    expect(after.actions.filter((a) => a.objectRef === "orch:bulk-ok").length).toBe(1);
  });

  test("POST /api/actions/batch rejects malformed envelopes with 400", async () => {
    expect((await post("/api/actions/batch", {})).status).toBe(400);
    expect((await post("/api/actions/batch", { actions: [] })).status).toBe(400);
    expect((await post("/api/actions/batch", { actions: "nope" })).status).toBe(400);
  });
});
