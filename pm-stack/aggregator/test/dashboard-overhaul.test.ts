// Epic #1442 (issues #1443/#1444/#1445/#1446/#1449/#1450/#1451/
// #1456): the dashboard reflects its own actions store at render —
// clicks land visibly, suppressed cards move to their strips with a
// way back, cards expand to their detail + history, carry a comment
// box and one-tap resolutions, surveys accumulate partial answers,
// and the Needs-you order is a saved preference.
import { describe, expect, test } from "bun:test";
import { validateNewAction, type DashAction } from "../src/actions.ts";
import { cardState, effectiveActions } from "../src/card-state.ts";
import { buildAttention } from "../src/derive.ts";
import type { OrchestrationSection, TierItem } from "../src/model.ts";
import { STANDING_IDENTITIES } from "../src/registry.ts";
import { attentionSections, renderAttention } from "../src/render.ts";
import { fixtureSource } from "../src/sources/fixtures.ts";
import { effectiveNeedsOrder } from "../src/tier-items.ts";

const NOW = Date.now();
const iso = (ms: number) => new Date(ms).toISOString();
const drew = { kind: "drew" as const, id: "drew", label: "Drew" };
const identities = STANDING_IDENTITIES.map((i) => ({ ...i }));

const BASE: TierItem = {
  id: "goal_plaid",
  kind: "task",
  title: "Link the Citi accounts in Plaid",
  source: "tracked",
  reason: "Only the user can complete the bank sign-in.",
  since: iso(NOW - 86_400_000),
  lastSeenAt: iso(NOW - 3_600_000),
};

const mkAction = (over: Partial<DashAction>): DashAction => ({
  id: "da-x",
  clientToken: "ct-x",
  kind: "triage",
  verb: "acknowledge",
  objectRef: "orch:goal_plaid",
  payload: {},
  author: drew,
  createdAt: iso(NOW - 600_000),
  state: "new",
  ...over,
});

function section(items: TierItem[]): OrchestrationSection {
  return {
    version: 1,
    generatedAt: iso(NOW),
    runId: "orch-test",
    provenance: { sources: ["fixture"] },
    tiers: { needsDrew: items, needsOrchestrator: [] },
    dependencies: [],
    goals: [],
    actions: [],
  };
}

async function page(items: TierItem[], actionsByObject = new Map<string, DashAction[]>(), actExtra = {}) {
  const src = fixtureSource();
  const view = buildAttention([], section(items), NOW);
  return renderAttention(view, await src.coverage(), src.provenance(), undefined, undefined, {
    identities,
    actionsByObject,
    commentOpts: { identities },
    ...actExtra,
  });
}

describe("action boundary additions (#1449/#1450/#1456)", () => {
  const base = {
    clientToken: "ct-overhaul-1",
    kind: "triage" as const,
    objectRef: "orch:goal_plaid",
    author: drew,
    payload: {},
  };
  test("mark-done validates with no required payload", () => {
    expect(validateNewAction({ ...base, verb: "mark-done" }).ok).toBe(true);
  });
  test("restore requires supersedes", () => {
    const bad = validateNewAction({ ...base, verb: "restore" });
    expect(bad.ok).toBe(false);
    if (!bad.ok) expect(bad.error).toContain("restore requires supersedes");
    expect(validateNewAction({ ...base, verb: "restore", supersedes: "da-1" }).ok).toBe(true);
  });
  test("set-needs-order requires a valid needsOrder", () => {
    const bad = validateNewAction({ ...base, kind: "preference", verb: "set-needs-order" });
    expect(bad.ok).toBe(false);
    const bogus = validateNewAction({
      ...base,
      kind: "preference",
      verb: "set-needs-order",
      payload: { needsOrder: "random" },
    });
    expect(bogus.ok).toBe(false);
    expect(
      validateNewAction({ ...base, kind: "preference", verb: "set-needs-order", payload: { needsOrder: "newest" } }).ok,
    ).toBe(true);
  });
});

describe("card-state derivation (#1443/#1450)", () => {
  test("a PENDING snooze already suppresses — the click lands before the consumer runs", () => {
    const cs = cardState([mkAction({ verb: "snooze", payload: { until: iso(NOW + 3_600_000) } })], NOW);
    expect(cs.suppressed?.kind).toBe("snoozed");
  });
  test("an expired snooze does not suppress", () => {
    const cs = cardState(
      [mkAction({ verb: "snooze", state: "applied", payload: { until: iso(NOW - 3_600_000) } })],
      NOW,
    );
    expect(cs.suppressed).toBeNull();
  });
  test("dismiss and mark-done suppress; a consumer rejection lifts the suppression", () => {
    expect(cardState([mkAction({ verb: "dismiss", payload: { reason: "Duplicate" } })], NOW).suppressed?.kind).toBe(
      "dismissed",
    );
    expect(cardState([mkAction({ verb: "mark-done" })], NOW).suppressed?.kind).toBe("done");
    expect(
      cardState([mkAction({ verb: "dismiss", state: "rejected", payload: { reason: "Duplicate" } })], NOW).suppressed,
    ).toBeNull();
  });
  test("restore cancels even an APPLIED dismiss; the item is back with its history", () => {
    const dismiss = mkAction({ id: "da-dismiss", verb: "dismiss", state: "applied", payload: { reason: "x" } });
    const restore = mkAction({ id: "da-restore", verb: "restore", supersedes: "da-dismiss", createdAt: iso(NOW) });
    const cs = cardState([dismiss, restore], NOW);
    expect(cs.suppressed).toBeNull();
    expect(cs.pending.map((a) => a.verb)).toContain("restore");
  });
  test("a REJECTED restore does NOT cancel its target — the dismissal stands (#1474)", () => {
    const dismiss = mkAction({ id: "da-dismiss", verb: "dismiss", state: "applied", payload: { reason: "x" } });
    const restore = mkAction({
      id: "da-restore",
      verb: "restore",
      state: "rejected",
      supersedes: "da-dismiss",
      createdAt: iso(NOW),
    });
    const cs = cardState([dismiss, restore], NOW);
    expect(cs.suppressed?.kind).toBe("dismissed");
    expect(effectiveActions([dismiss, restore]).map((a) => a.id)).toContain("da-dismiss");
  });
  test("a REJECTED restore of a snooze leaves the card snoozed (#1474)", () => {
    const snooze = mkAction({
      id: "da-snooze",
      verb: "snooze",
      state: "applied",
      payload: { until: iso(NOW + 3_600_000) },
    });
    const restore = mkAction({
      id: "da-restore",
      verb: "restore",
      state: "rejected",
      supersedes: "da-snooze",
      createdAt: iso(NOW),
    });
    expect(cardState([snooze, restore], NOW).suppressed?.kind).toBe("snoozed");
  });
  test("a REJECTED restore of an ack leaves the card acked (#1474)", () => {
    const ack = mkAction({ id: "da-ack", verb: "acknowledge", state: "applied" });
    const restore = mkAction({
      id: "da-restore",
      verb: "restore",
      state: "rejected",
      supersedes: "da-ack",
      createdAt: iso(NOW),
    });
    expect(cardState([ack, restore], NOW).acked).toBe(true);
  });
  test("an APPLIED restore still cancels its target (#1474)", () => {
    const dismiss = mkAction({ id: "da-dismiss", verb: "dismiss", state: "applied", payload: { reason: "x" } });
    const restore = mkAction({
      id: "da-restore",
      verb: "restore",
      state: "applied",
      supersedes: "da-dismiss",
      createdAt: iso(NOW),
    });
    expect(cardState([dismiss, restore], NOW).suppressed).toBeNull();
  });
  test("survey answers merge across submits, latest per question wins (#1451)", () => {
    const a1 = mkAction({
      verb: "answer",
      payload: { answers: [{ questionId: "q1", value: "a" }] },
    });
    const a2 = mkAction({
      id: "da-y",
      verb: "answer",
      createdAt: iso(NOW),
      payload: {
        answers: [
          { questionId: "q2", value: "b" },
          { questionId: "q1", value: "c" },
        ],
      },
    });
    const cs = cardState([a1, a2], NOW);
    expect(cs.answered.get("q1")).toBe("c");
    expect(cs.answered.get("q2")).toBe("b");
  });
});

describe("render-time suppression + strips (#1443/#1450)", () => {
  test("a pending snooze moves the card out of Needs User into the Snoozed strip, count drops", async () => {
    const snooze = mkAction({ verb: "snooze", payload: { until: iso(NOW + 86_400_000) } });
    const html = await page([BASE], new Map([["orch:goal_plaid", [snooze]]]));
    expect(html).toContain("Needs User (0)");
    expect(html).toContain("Nothing in the user queue right now.");
    expect(html).toContain("Snoozed by you (1)");
    expect(html).toContain("Link the Citi accounts in Plaid");
    expect(html).toContain("⏳ sent · pending");
    expect(html).toContain("Unsnooze now");
  });
  test("a dismissed card lands in the hidden strip with a restore form naming the dismiss", async () => {
    const dismiss = mkAction({ id: "da-d1", verb: "dismiss", payload: { reason: "Already handled" } });
    const html = await page([BASE], new Map([["orch:goal_plaid", [dismiss]]]));
    expect(html).toContain("Dismissed / done by you (1)");
    expect(html).toContain('data-verb="restore"');
    expect(html).toContain('data-supersedes="da-d1"');
    expect(html).toContain("Already handled");
  });
  test("a dismissed ref no longer published at all still shows in the strip (#1450)", async () => {
    const dismiss = mkAction({
      id: "da-d2",
      objectRef: "orch:gone_item",
      verb: "dismiss",
      state: "applied",
      payload: { reason: "Duplicate" },
    });
    const html = await page([BASE], new Map([["orch:gone_item", [dismiss]]]));
    expect(html).toContain("Dismissed / done by you (1)");
    expect(html).toContain("orch:gone_item");
    expect(html).toContain('data-verb="restore"');
    // the visible card is untouched
    expect(html).toContain("Needs User (1)");
  });
  test("an acknowledged card renders quiet with its ack state", async () => {
    const ack = mkAction({ verb: "acknowledge", state: "applied" });
    const html = await page([BASE], new Map([["orch:goal_plaid", [ack]]]));
    expect(html).toContain('class="card acked"');
    expect(html).toContain("✓ Acknowledged");
  });
});

describe("quick actions, detail, comments (#1449/#1445/#1446)", () => {
  test("the quick bar carries one-tap resolutions with preset whys", async () => {
    const html = await page([BASE]);
    expect(html).toContain("Already done");
    expect(html).toContain('data-verb="mark-done"');
    expect(html).toContain("Dismiss: Already handled");
    expect(html).toContain("Orchestrator should have handled this");
    expect(html).toContain("Need more info");
    expect(html).toContain("Need recommendations");
  });
  test("the card expands inline: source, since, last seen, question payload, action history", async () => {
    const item: TierItem = {
      ...BASE,
      questions: [
        {
          id: "more-institutions",
          prompt: "More institutions to link?",
          type: "text",
          required: true,
          covers: "whether Plaid is done",
        },
      ],
    };
    const answer = mkAction({
      verb: "answer",
      state: "applied",
      payload: { answers: [{ questionId: "more-institutions", value: "none" }] },
    });
    const html = await page([item], new Map([["orch:goal_plaid", [answer]]]));
    expect(html).toContain("Details &amp; history");
    expect(html).toContain("Last seen");
    expect(html).toContain("More institutions to link?");
    expect(html).toContain("covers: whether Plaid is done");
    expect(html).toContain("✓ answered: none");
  });
  test("the comment box offers the public-post checkbox only with a gh: ref", async () => {
    const withRef: TierItem = { ...BASE, id: "x1", ref: "developing-today/code#2001" };
    const htmlRef = await page([withRef]);
    expect(htmlRef).toContain("Also post publicly to the GitHub issue (developing-today/code#2001)");
    expect(htmlRef).toContain('data-ghref="developing-today/code#2001"');
    const htmlBare = await page([BASE]);
    expect(htmlBare).toContain("the comment stays internal");
    expect(htmlBare).not.toContain("data-ghref=");
  });
  test("a gh: card title links to the GitHub issue", async () => {
    const withRef: TierItem = { ...BASE, id: "x1", ref: "developing-today/code#2001" };
    const html = await page([withRef]);
    expect(html).toContain(
      '<a href="https://github.com/developing-today/code/issues/2001">Link the Citi accounts in Plaid</a>',
    );
  });
});

describe("multi-question partial state (#1451)", () => {
  test("one answered + one open required question: both states show on the card", async () => {
    const item: TierItem = {
      ...BASE,
      questions: [
        { id: "q1", prompt: "First?", type: "text", required: true, covers: "one" },
        { id: "q2", prompt: "Second?", type: "text", required: true, covers: "two" },
      ],
    };
    const partial = mkAction({ verb: "answer", payload: { answers: [{ questionId: "q1", value: "yes" }] } });
    const html = await page([item], new Map([["orch:goal_plaid", [partial]]]));
    expect(html).toContain("✓ answered: yes");
    expect(html).toContain("still needed");
    expect(html).toContain("still needed: q2");
  });
});

describe("needs-you order (#1456)", () => {
  test("effectiveNeedsOrder: default urgency; latest record wins; pending flags", () => {
    expect(effectiveNeedsOrder([])).toEqual({ order: "urgency", pending: false });
    const pref = mkAction({
      kind: "preference",
      verb: "set-needs-order",
      objectRef: "needs-user-order",
      payload: { needsOrder: "newest" },
    });
    expect(effectiveNeedsOrder([pref])).toEqual({ order: "newest", pending: true });
  });
  test("the order control renders and newest-first reorders the list at render", async () => {
    // #1485: newest sorts on the since axis. The lastSeenAt values here
    // point the OTHER way (the older card was seen a minute ago) — under
    // the pre-#1485 seenMs axis this pair ordered oldest-first.
    const older: TierItem = {
      ...BASE,
      id: "older",
      title: "Older card",
      since: iso(NOW - 86_400_000),
      lastSeenAt: iso(NOW - 60_000),
    };
    const newer: TierItem = {
      ...BASE,
      id: "newer",
      title: "Newer card",
      since: iso(NOW - 60_000),
      lastSeenAt: iso(NOW - 86_400_000),
    };
    const src = fixtureSource();
    const view = buildAttention([], section([older, newer]), NOW);
    const html = renderAttention(view, await src.coverage(), src.provenance(), undefined, undefined, {
      identities,
      actionsByObject: new Map(),
      needsOrder: "newest",
    });
    expect(html).toContain('data-verb="set-needs-order"');
    expect(html.indexOf("Newer card")).toBeLessThan(html.indexOf("Older card"));
  });
  test("newest and oldest are exact reverses when since and lastSeenAt disagree (#1485)", async () => {
    // The creator probe pair: a 30-day-old card seen 5 minutes ago led
    // BOTH orders before #1485 (newest read lastSeenAt, oldest read since).
    const newCard: TierItem = {
      ...BASE,
      id: "newcard",
      title: "Fresh card",
      since: iso(NOW - 3_600_000),
      lastSeenAt: iso(NOW - 7_200_000),
    };
    const oldCard: TierItem = {
      ...BASE,
      id: "oldcard",
      title: "Stale card",
      since: iso(NOW - 30 * 86_400_000),
      lastSeenAt: iso(NOW - 300_000),
    };
    const src = fixtureSource();
    const render = async (needsOrder: "newest" | "oldest" | "deadline") => {
      const view = buildAttention([], section([oldCard, newCard]), NOW);
      return renderAttention(view, await src.coverage(), src.provenance(), undefined, undefined, {
        identities,
        actionsByObject: new Map(),
        needsOrder,
      });
    };
    const newest = await render("newest");
    expect(newest.indexOf("Fresh card")).toBeLessThan(newest.indexOf("Stale card"));
    const oldest = await render("oldest");
    expect(oldest.indexOf("Stale card")).toBeLessThan(oldest.indexOf("Fresh card"));
    // Deadline: both cards share rank (no horizon), so the sinceMs
    // tiebreak decides — oldest-waiting first, lastSeenAt irrelevant.
    const deadline = await render("deadline");
    expect(deadline.indexOf("Stale card")).toBeLessThan(deadline.indexOf("Fresh card"));
  });
  test("newest and oldest reverse an agreeing pair too (#1485 control)", async () => {
    const a: TierItem = {
      ...BASE,
      id: "ctl-a",
      title: "Control A",
      since: iso(NOW - 7_200_000),
      lastSeenAt: iso(NOW - 7_200_000),
    };
    const b: TierItem = {
      ...BASE,
      id: "ctl-b",
      title: "Control B",
      since: iso(NOW - 3_600_000),
      lastSeenAt: iso(NOW - 3_600_000),
    };
    const src = fixtureSource();
    const render = async (needsOrder: "newest" | "oldest") => {
      const view = buildAttention([], section([a, b]), NOW);
      return renderAttention(view, await src.coverage(), src.provenance(), undefined, undefined, {
        identities,
        actionsByObject: new Map(),
        needsOrder,
      });
    };
    const newest = await render("newest");
    expect(newest.indexOf("Control B")).toBeLessThan(newest.indexOf("Control A"));
    const oldest = await render("oldest");
    expect(oldest.indexOf("Control A")).toBeLessThan(oldest.indexOf("Control B"));
  });
});

describe("attention sections (#1444)", () => {
  test("the page splits into header / queues / sessions that reassemble", async () => {
    const src = fixtureSource();
    const view = buildAttention([], section([BASE]), NOW);
    const sections = attentionSections(view, await src.coverage(), undefined, {
      identities,
      actionsByObject: new Map(),
    });
    expect(sections.queues).toContain('id="drew"');
    expect(sections.queues).toContain("Link the Citi accounts in Plaid");
    expect(sections.header).toContain("Needs User (1)");
    expect(sections.sessions).toContain('id="sessions"');
  });
});
