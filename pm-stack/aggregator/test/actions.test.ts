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
      store.add(newAction({ verb: "answer", objectRef: "orch:concurrent", payload: { optionId: "yes", questionId: "q5" } })),
      store.add(
        newAction({ verb: "answer", objectRef: "orch:concurrent", payload: { answers: [{ questionId: "q5", value: "yes" }] } }),
      ),
    ]);
    expect(concurrent.filter((result) => result.action.state === "new")).toHaveLength(1);
    expect(concurrent.filter((result) => result.duplicateOf)).toHaveLength(1);
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
      sessionSource: "jules",
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

  test("jules session card: answer enabled, poke/stop disabled keeper-mediated", async () => {
    const src = fixtureSource();
    const view = buildAttention(await src.attention(), SECTION, NOW);
    const html = renderAttention(view, await src.coverage(), src.provenance(), undefined, undefined, {
      identities,
      actionsByObject: new Map(),
    });
    // The fixture feed carries Jules sessions; their cards must show
    // the keeper-mediated honesty, not enabled poke/stop buttons.
    expect(html).toContain("keeper-mediated — answer only");
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

  test("live jules registry row: answer only; a stale row gets no controls", () => {
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
    expect(html).toContain('data-session-id="jules-live-1"');
    expect(html).toContain("keeper-mediated — answer only");
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

  test("duplicate answer response includes the persisted audit pointer", async () => {
    const body = {
      clientToken: "ct-session-answer-1",
      kind: "session",
      verb: "answer",
      objectRef: "session:duplicate-answer",
      sessionId: "duplicate-answer",
      sessionSource: "jules",
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
    await post("/api/actions/result", { id: original.id, outcome: "applied", byRun: "jules-fixture" });
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
});
