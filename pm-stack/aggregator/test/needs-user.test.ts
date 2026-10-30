// #1290–#1293: the needs-user action layer contract — per-item
// extension sanitization (fail-soft, the section gate never learns the
// fields), fallbacks + the standing auto toggle defaulting, the
// POST-boundary checks that need the published item, proposal identity
// and expiry, and cross-surface count consistency.
import { describe, expect, test } from "bun:test";
import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { validateNewAction, type DashAction, type NewAction } from "../src/actions.ts";
import { loadConfig } from "../src/config.ts";
import { buildAttention } from "../src/derive.ts";
import { isOrchestrationSection } from "../src/feed.ts";
import type { OrchestrationSection, TierItem } from "../src/model.ts";
import { STANDING_IDENTITIES } from "../src/registry.ts";
import { renderAttention } from "../src/render.ts";
import { fixtureSource } from "../src/sources/fixtures.ts";
import { liveSource } from "../src/sources/live.ts";
import {
  checkActionAgainstItem,
  effectiveStandingAuto,
  proposalExpired,
  proposalIdentity,
  sanitizeTierItemExtensions,
  withUserFallbacks,
} from "../src/tier-items.ts";

const BASE: TierItem = {
  id: "developing-today/code#2001",
  kind: "decision",
  title: "Pick the export format",
  source: "backlog",
  reason: "A scope decision only the user can make.",
  since: "2026-10-26T10:00:00.000Z",
  lastSeenAt: "2026-10-27T03:00:00.000Z",
  ref: "developing-today/code#2001",
};

/** A raw (untrusted-shape) fully extended item, as a publisher sends it. */
function extendedRaw(): Record<string, unknown> {
  return {
    ...BASE,
    whyUser: "decision",
    summary: "The export needs a format before the importer can be built.",
    ifIgnored: "The importer work stays blocked.",
    options: [
      { id: "csv", label: "CSV", description: "Plain export", effect: "Importer unblocks this week" },
      { id: "json", label: "JSON" },
    ],
    questions: [
      {
        id: "audience",
        prompt: "Who consumes the export?",
        type: "single",
        options: [
          { id: "internal", label: "Internal only" },
          { id: "public", label: "Public" },
        ],
        required: true,
        covers: "the privacy review scope",
      },
      { id: "notes", prompt: "Anything else?", type: "text", required: false, covers: "edge cases" },
    ],
    delegation: {
      mode: "first-pass",
      status: "proposal-ready",
      updatedAt: "2026-10-27T02:00:00.000Z",
      runId: "orch-2026-10-27T02:00:00Z",
      proposal: { summary: "Recommend CSV", at: "2026-10-27T02:00:00.000Z" },
    },
  };
}

describe("per-item extension sanitizer (#1290)", () => {
  test("a fully extended item survives intact", () => {
    const out = sanitizeTierItemExtensions(extendedRaw() as unknown as TierItem);
    expect(out.whyUser).toBe("decision");
    expect(out.summary).toContain("export needs a format");
    expect(out.ifIgnored).toBe("The importer work stays blocked.");
    expect(out.options?.map((o) => o.id)).toEqual(["csv", "json"]);
    expect(out.options?.[0]?.effect).toBe("Importer unblocks this week");
    expect(out.questions?.length).toBe(2);
    expect(out.questions?.[0]?.covers).toBe("the privacy review scope");
    expect(out.delegation?.status).toBe("proposal-ready");
    expect(out.delegation?.proposal?.summary).toBe("Recommend CSV");
    // base fields untouched
    expect(out.title).toBe(BASE.title);
    expect(out.reason).toBe(BASE.reason);
  });

  test("duplicate option ids drop the later entry; options cap at 6", () => {
    const raw = extendedRaw();
    raw.options = [
      { id: "a", label: "A" },
      { id: "a", label: "A again" },
      ...Array.from({ length: 8 }, (_, i) => ({ id: `o${i}`, label: `O${i}` })),
    ];
    const out = sanitizeTierItemExtensions(raw as unknown as TierItem);
    expect(out.options?.length).toBe(6);
    expect(out.options?.filter((o) => o.id === "a").length).toBe(1);
    expect(out.options?.[0]?.label).toBe("A");
  });

  test("a one-option survey question drops; a question without covers drops", () => {
    const raw = extendedRaw();
    raw.questions = [
      { id: "thin", prompt: "Pick", type: "single", options: [{ id: "x", label: "X" }], covers: "c" },
      { id: "nocover", prompt: "Why?", type: "text", covers: "" },
      { id: "ok", prompt: "Sure?", type: "yesno", covers: "the go/no-go" },
    ];
    const out = sanitizeTierItemExtensions(raw as unknown as TierItem);
    expect(out.questions?.map((q) => q.id)).toEqual(["ok"]);
    // required defaults to true when absent
    expect(out.questions?.[0]?.required).toBe(true);
  });

  test("an inconsistent delegation drops as a unit — the item stays", () => {
    const cases: Record<string, unknown>[] = [
      // proposal payload without the proposal-ready status
      {
        mode: "first-pass",
        status: "working",
        updatedAt: "2026-10-27T02:00:00.000Z",
        proposal: { summary: "x", at: "2026-10-27T02:00:00.000Z" },
      },
      // proposal-ready without the payload
      { mode: "first-pass", status: "proposal-ready", updatedAt: "2026-10-27T02:00:00.000Z" },
      // unable without whatItTried
      {
        mode: "auto",
        status: "unable",
        updatedAt: "2026-10-27T02:00:00.000Z",
        unable: { reason: "r", whatItNeeds: "n", at: "2026-10-27T02:00:00.000Z" },
      },
      // bad timestamp
      { mode: "user", status: "idle", updatedAt: "not a date" },
      // unknown mode
      { mode: "yolo", status: "idle", updatedAt: "2026-10-27T02:00:00.000Z" },
    ];
    for (const delegation of cases) {
      const raw = { ...extendedRaw(), delegation };
      const out = sanitizeTierItemExtensions(raw as unknown as TierItem);
      expect(out.delegation).toBeUndefined();
      expect(out.id).toBe(BASE.id);
      expect(out.options?.length).toBe(2);
    }
  });

  test("an auto delegation on an access-class item drops; legacy mode 'drew' normalizes to 'user'", () => {
    const access = {
      ...extendedRaw(),
      whyUser: "access",
      delegation: { mode: "auto", status: "working", updatedAt: "2026-10-27T02:00:00.000Z" },
    };
    expect(sanitizeTierItemExtensions(access as unknown as TierItem).delegation).toBeUndefined();
    const legacy = {
      ...extendedRaw(),
      delegation: { mode: "drew", status: "idle", updatedAt: "2026-10-27T02:00:00.000Z" },
    };
    expect(sanitizeTierItemExtensions(legacy as unknown as TierItem).delegation?.mode).toBe("user");
  });

  test("a malformed whyUser drops; the item and its other extensions stay", () => {
    const raw = { ...extendedRaw(), whyUser: "whenever" };
    const out = sanitizeTierItemExtensions(raw as unknown as TierItem);
    expect(out.whyUser).toBeUndefined();
    expect(out.options?.length).toBe(2);
  });
});

describe("needs-user fallbacks + standing toggle defaulting (#1290)", () => {
  const GEN = "2026-10-27T03:00:00.000Z";

  test("whyUser fallback: decision kind → decision; question-bearing item → missing-info; else decision", () => {
    expect(withUserFallbacks(BASE, GEN, true).whyUser).toBe("decision");
    const sessionItem: TierItem = { ...BASE, id: "s1", kind: "session", question: "Which branch?" };
    expect(withUserFallbacks(sessionItem, GEN, true).whyUser).toBe("missing-info");
    const task: TierItem = { ...BASE, id: "t1", kind: "task" };
    expect(withUserFallbacks(task, GEN, true).whyUser).toBe("decision");
  });

  test("ifIgnored fallback is computed from since", () => {
    const out = withUserFallbacks(BASE, GEN, true);
    expect(out.ifIgnored).toContain("in your queue since 2026-10-26");
  });

  test("delegation default follows the standing toggle and the class gate", () => {
    expect(withUserFallbacks(BASE, GEN, true).delegation?.mode).toBe("auto");
    expect(withUserFallbacks(BASE, GEN, false).delegation?.mode).toBe("user");
    const access: TierItem = { ...BASE, whyUser: "access" };
    expect(withUserFallbacks(access, GEN, true).delegation?.mode).toBe("user");
    const policy: TierItem = { ...BASE, whyUser: "policy" };
    expect(withUserFallbacks(policy, GEN, true).delegation?.mode).toBe("user");
    const approval: TierItem = { ...BASE, whyUser: "approval" };
    expect(withUserFallbacks(approval, GEN, true).delegation?.mode).toBe("auto");
  });

  test("a published delegation always wins over the default", () => {
    const item = sanitizeTierItemExtensions(extendedRaw() as unknown as TierItem);
    const out = withUserFallbacks(item, GEN, true);
    expect(out.delegation?.mode).toBe("first-pass");
    expect(out.delegation?.status).toBe("proposal-ready");
  });
});

describe("the standing-auto preference records (#1290)", () => {
  function pref(standingAuto: boolean, state: DashAction["state"], createdAt: string): DashAction {
    return {
      id: `da-${createdAt}`,
      clientToken: `ct-${createdAt}`,
      kind: "preference",
      verb: "set-delegation-default",
      objectRef: "needs-user-delegation-default",
      payload: { standingAuto },
      author: { kind: "drew", id: "drew", label: "Drew" },
      createdAt,
      state,
    };
  }

  test("no records → Drew's 2026-10-27 decision: ON, not pending", () => {
    expect(effectiveStandingAuto([])).toEqual({ standingAuto: true, pending: false });
  });

  test("the latest preference wins; a pending one reports pending", () => {
    const records = [pref(false, "applied", "2026-10-27T05:00:00.000Z"), pref(true, "new", "2026-10-27T06:00:00.000Z")];
    expect(effectiveStandingAuto(records)).toEqual({ standingAuto: true, pending: true });
  });

  test("superseded and rejected records do not move the default", () => {
    const records = [
      pref(false, "applied", "2026-10-27T05:00:00.000Z"),
      pref(true, "superseded", "2026-10-27T06:00:00.000Z"),
      pref(true, "rejected", "2026-10-27T07:00:00.000Z"),
    ];
    expect(effectiveStandingAuto(records)).toEqual({ standingAuto: false, pending: false });
  });
});

describe("gate round-trip with extended items (#1290)", () => {
  test("the section gate accepts fully extended items unchanged in shape", () => {
    const section = {
      version: 1,
      generatedAt: "2026-10-27T03:00:00Z",
      runId: "orch-2026-10-27T03:00:00Z",
      provenance: { sources: ["fixture"], runId: "orch-2026-10-27T03:00:00Z" },
      tiers: { needsDrew: [extendedRaw()], needsOrchestrator: [] },
      dependencies: [],
      goals: [],
      actions: [],
    };
    expect(isOrchestrationSection(section)).toBe(true);
  });

  test("the live source serves extensions and drops malformed ones per item", async () => {
    const dir = await mkdtemp(join(tmpdir(), "pm-nu-"));
    const feedFile = join(dir, "fleet-feed.json");
    const bad = extendedRaw();
    bad.delegation = { mode: "auto", status: "working", updatedAt: "2026-10-27T02:00:00.000Z" };
    bad.whyUser = "access"; // auto + access → delegation drops at the seam
    await writeFile(
      feedFile,
      JSON.stringify({
        version: 1,
        generatedAt: new Date().toISOString(),
        quota: { pools: [], burn: [], choices: { hasHeadroom: [], burnBeforeReset: [], doNotStart: [] } },
        attention: [],
        orchestration: {
          version: 1,
          generatedAt: "2026-10-27T03:00:00Z",
          runId: "orch-2026-10-27T03:00:00Z",
          provenance: { sources: ["fixture"], runId: "orch-2026-10-27T03:00:00Z" },
          tiers: { needsDrew: [extendedRaw(), bad], needsOrchestrator: [] },
          dependencies: [],
          goals: [],
          actions: [],
        },
      }),
    );
    const cfg = loadConfig({ PM_AGG_SOURCE: "live", PM_AGG_FEED_FILE: feedFile, PM_PROJECTOR_URL: "off" });
    const src = liveSource(cfg, { standingAuto: async () => true });
    const section = await src.orchestration();
    expect(section).not.toBeNull();
    const [good, dropped] = section!.tiers.needsDrew;
    expect(good?.delegation?.status).toBe("proposal-ready");
    expect(good?.options?.length).toBe(2);
    expect(dropped?.delegation?.mode).toBe("user"); // dropped, then fallback-stamped
    expect(dropped?.whyUser).toBe("access");
    expect(section!.tiers.needsDrew.length).toBe(2); // the section and both items survive
  });
});

function actionInput(partial: Partial<NewAction>): NewAction {
  return {
    clientToken: "ct-test-0001",
    kind: "triage",
    verb: "answer",
    objectRef: "gh:developing-today/code#2001",
    payload: {},
    author: { kind: "drew", id: "drew", label: "Drew" },
    ...partial,
  };
}

describe("POST-boundary checks against the published item (#1291/#1293)", () => {
  const item = sanitizeTierItemExtensions(extendedRaw() as unknown as TierItem);
  const NOW = Date.parse("2026-10-27T04:00:00.000Z");

  test("an answer naming an unpublished option is rejected; a published one passes", () => {
    expect(checkActionAgainstItem(actionInput({ payload: { optionId: "yaml" } }), item, NOW)).toContain(
      "not one of this item's published options",
    );
    expect(checkActionAgainstItem(actionInput({ payload: { optionId: "csv" } }), item, NOW)).toBeNull();
  });

  test("survey answers: unknown ids and bad values are legible 400s; partial submits are accepted (#1451)", () => {
    expect(
      checkActionAgainstItem(actionInput({ payload: { answers: [{ questionId: "nope", value: "x" }] } }), item, NOW),
    ).toContain('unknown question id "nope"');
    expect(
      checkActionAgainstItem(
        actionInput({ payload: { answers: [{ questionId: "audience", value: "bogus" }] } }),
        item,
        NOW,
      ),
    ).toContain("must be one of its options");
    // #1451 (Drew, 2026-10-27): a partial submit — only the optional
    // question answered — is recorded, not rejected; the card keeps
    // showing the required remainder. (Pre-#1451 this was the
    // "missing required answers: audience" 400.)
    expect(
      checkActionAgainstItem(actionInput({ payload: { answers: [{ questionId: "notes", value: "hi" }] } }), item, NOW),
    ).toBeNull();
    expect(
      checkActionAgainstItem(
        actionInput({ payload: { answers: [{ questionId: "audience", value: "internal" }] } }),
        item,
        NOW,
      ),
    ).toBeNull();
  });

  test("a bare text answer cannot bypass required survey questions", () => {
    expect(checkActionAgainstItem(actionInput({ payload: { text: "just do it" } }), item, NOW)).toContain(
      "survey questions",
    );
  });

  test("answers against a vanished item are rejected, not stored", () => {
    expect(checkActionAgainstItem(actionInput({ payload: { optionId: "csv" } }), undefined, NOW)).toContain(
      "no longer on the published",
    );
  });

  test("delegate auto on an access item is refused at the boundary", () => {
    const accessItem: TierItem = { ...BASE, whyUser: "access" };
    expect(
      checkActionAgainstItem(actionInput({ verb: "delegate", payload: { mode: "auto" } }), accessItem, NOW),
    ).toContain("can never be delegated to auto");
    expect(checkActionAgainstItem(actionInput({ verb: "delegate", payload: { mode: "auto" } }), item, NOW)).toBeNull();
  });
});

describe("proposal identity + expiry (#1293)", () => {
  const item = sanitizeTierItemExtensions(extendedRaw() as unknown as TierItem);
  const NOW = Date.parse("2026-10-27T04:00:00.000Z");

  test("identity is (item id, delegation runId, delegation updatedAt)", () => {
    expect(proposalIdentity(item)).toEqual({
      itemId: BASE.id,
      runId: "orch-2026-10-27T02:00:00Z",
      updatedAt: "2026-10-27T02:00:00.000Z",
    });
    expect(proposalIdentity(BASE)).toBeNull();
  });

  test("approve with the live identity passes; a stale triple is rejected and names the supersede", () => {
    const ok = actionInput({
      verb: "approve-proposal",
      payload: { proposalRunId: "orch-2026-10-27T02:00:00Z", proposalUpdatedAt: "2026-10-27T02:00:00.000Z" },
    });
    expect(checkActionAgainstItem(ok, item, NOW)).toBeNull();
    const stale = actionInput({
      verb: "approve-proposal",
      payload: { proposalRunId: "orch-2026-10-27T01:00:00Z", proposalUpdatedAt: "2026-10-27T01:00:00.000Z" },
    });
    expect(checkActionAgainstItem(stale, item, NOW)).toContain("superseded/withdrawn");
    const missing = actionInput({ verb: "reject-proposal", payload: { reason: "no" } });
    expect(checkActionAgainstItem(missing, item, NOW)).toContain("proposal identity");
  });

  test("a proposal past its TTL reads expired, and approving it is refused", () => {
    const later = Date.parse("2026-10-30T04:00:00.000Z");
    expect(proposalExpired(item.delegation, later)).toBe(true);
    expect(proposalExpired(item.delegation, NOW)).toBe(false);
    const approve = actionInput({
      verb: "approve-proposal",
      payload: { proposalRunId: "orch-2026-10-27T02:00:00Z", proposalUpdatedAt: "2026-10-27T02:00:00.000Z" },
    });
    expect(checkActionAgainstItem(approve, item, later)).toContain("expired");
  });

  /** An item whose proposal draft time and delegation updatedAt diverge (#1623). */
  function divergentItem(proposalAt: string, updatedAt: string): TierItem {
    const raw = extendedRaw();
    (raw.delegation as Record<string, unknown>).updatedAt = updatedAt;
    ((raw.delegation as Record<string, unknown>).proposal as Record<string, unknown>).at = proposalAt;
    return sanitizeTierItemExtensions(raw as unknown as TierItem);
  }

  test("expiry is clocked from proposal.at, not delegation updatedAt (#1623)", () => {
    const now = Date.parse("2026-10-30T04:00:00.000Z");

    // Proposal drafted 74h ago; delegation churned 1h ago. The proposal is
    // expired, and approving it (with the live identity triple) is refused.
    const churned = divergentItem("2026-10-27T02:00:00.000Z", "2026-10-30T03:00:00.000Z");
    expect(proposalExpired(churned.delegation, now)).toBe(true);
    const approveStale = actionInput({
      verb: "approve-proposal",
      payload: { proposalRunId: "orch-2026-10-27T02:00:00Z", proposalUpdatedAt: "2026-10-30T03:00:00.000Z" },
    });
    expect(checkActionAgainstItem(approveStale, churned, now)).toContain("expired");

    // Proposal drafted 1h ago on an old delegation record. Not expired, and
    // approving it passes the expiry gate.
    const fresh = divergentItem("2026-10-30T03:00:00.000Z", "2026-10-27T02:00:00.000Z");
    expect(proposalExpired(fresh.delegation, now)).toBe(false);
    const approveFresh = actionInput({
      verb: "approve-proposal",
      payload: { proposalRunId: "orch-2026-10-27T02:00:00Z", proposalUpdatedAt: "2026-10-27T02:00:00.000Z" },
    });
    expect(checkActionAgainstItem(approveFresh, fresh, now)).toBeNull();
  });
});

describe("actions boundary additions (#1290)", () => {
  const author = { kind: "drew", id: "drew", label: "Drew" };

  test("preference actions require a boolean standingAuto", () => {
    const ok = validateNewAction({
      clientToken: "ct-pref-0001",
      kind: "preference",
      verb: "set-delegation-default",
      objectRef: "needs-user-delegation-default",
      payload: { standingAuto: false },
      author,
    });
    expect(ok.ok).toBe(true);
    const bad = validateNewAction({
      clientToken: "ct-pref-0002",
      kind: "preference",
      verb: "set-delegation-default",
      objectRef: "needs-user-delegation-default",
      payload: {},
      author,
    });
    expect(bad.ok).toBe(false);
  });

  test("delegate mode 'drew' normalizes to 'user'; proposal identity fields validate", () => {
    const legacy = validateNewAction({
      clientToken: "ct-del-00001",
      kind: "triage",
      verb: "delegate",
      objectRef: "orch:x",
      payload: { mode: "drew" },
      author,
    });
    expect(legacy.ok).toBe(true);
    if (legacy.ok) expect(legacy.value.payload.mode).toBe("user");
    const approve = validateNewAction({
      clientToken: "ct-app-00001",
      kind: "triage",
      verb: "approve-proposal",
      objectRef: "orch:x",
      payload: { proposalRunId: "orch-r1", proposalUpdatedAt: "2026-10-27T02:00:00.000Z" },
      author,
    });
    expect(approve.ok).toBe(true);
    const badStamp = validateNewAction({
      clientToken: "ct-app-00002",
      kind: "triage",
      verb: "approve-proposal",
      objectRef: "orch:x",
      payload: { proposalRunId: "orch-r1", proposalUpdatedAt: "whenever" },
      author,
    });
    expect(badStamp.ok).toBe(false);
  });

  test("multi-choice answers carry an array value; dismiss/reject still require reasons", () => {
    const multi = validateNewAction({
      clientToken: "ct-ans-00001",
      kind: "triage",
      verb: "answer",
      objectRef: "orch:x",
      payload: { answers: [{ questionId: "tags", value: ["a", "b"] }] },
      author,
    });
    expect(multi.ok).toBe(true);
    const dismiss = validateNewAction({
      clientToken: "ct-dis-00001",
      kind: "triage",
      verb: "dismiss",
      objectRef: "orch:x",
      payload: {},
      author,
    });
    expect(dismiss.ok).toBe(false);
    if (!dismiss.ok) expect(dismiss.error).toContain("requires payload.reason");
  });
});

describe("count consistency (#1293)", () => {
  test("a proposal-pending item counts once, as a needs-user item", () => {
    const proposalItem = sanitizeTierItemExtensions(extendedRaw() as unknown as TierItem);
    const plain: TierItem = { ...BASE, id: "developing-today/code#2002", ref: "developing-today/code#2002" };
    const section: OrchestrationSection = {
      version: 1,
      generatedAt: "2026-10-27T03:00:00.000Z",
      runId: "orch-2026-10-27T03:00:00Z",
      provenance: { sources: ["fixture"] },
      tiers: { needsDrew: [proposalItem, plain], needsOrchestrator: [] },
      dependencies: [],
      goals: [],
      actions: [],
    };
    const view = buildAttention([], section, Date.parse("2026-10-27T04:00:00.000Z"));
    expect(view.drew.length + view.drewExtra.length).toBe(2);
    expect(view.drewExtra.map((i) => i.id)).toEqual([proposalItem.id, plain.id]);
    expect(view.drewExtra[0]?.delegation?.status).toBe("proposal-ready");
  });
});

describe("needs-user card rendering (#1291/#1293)", () => {
  const identities = STANDING_IDENTITIES.map((i) => ({ ...i }));
  const recent = () => new Date(Date.now() - 3_600_000).toISOString();
  const longAgo = () => new Date(Date.now() - 72 * 3_600_000).toISOString();

  async function page(
    items: TierItem[],
    actionsByObject: Map<string, DashAction[]> = new Map(),
    stale = false,
  ): Promise<string> {
    const src = fixtureSource();
    const section: OrchestrationSection = {
      version: 1,
      generatedAt: new Date().toISOString(),
      runId: "orch-test",
      provenance: { sources: ["fixture"] },
      tiers: { needsDrew: items, needsOrchestrator: [] },
      dependencies: [],
      goals: [],
      actions: [],
    };
    const view = buildAttention(
      [],
      section,
      Date.now(),
      stale ? { state: "stale", runId: section.runId, generatedAt: section.generatedAt } : undefined,
    );
    return renderAttention(view, await src.coverage(), src.provenance(), undefined, undefined, {
      identities,
      actionsByObject,
    });
  }

  /** A proposal-ready item stamped relative to real now, so the TTL
   *  math in the render is exercised the way production runs it. */
  function proposalItem(at: string): TierItem {
    const raw = extendedRaw();
    (raw.delegation as Record<string, unknown>).updatedAt = at;
    ((raw.delegation as Record<string, unknown>).proposal as Record<string, unknown>).at = at;
    return sanitizeTierItemExtensions(raw as unknown as TierItem);
  }

  test("the full card: class badge, summary, if-nothing line, option buttons, survey, delegation controls, dismiss", async () => {
    const item = withUserFallbacks(
      sanitizeTierItemExtensions(extendedRaw() as unknown as TierItem),
      new Date().toISOString(),
      true,
    );
    const html = await page([item]);
    expect(html).toContain("Needs User (1)");
    expect(html).toContain(">Decision</span>");
    expect(html).toContain("The export needs a format before the importer can be built.");
    expect(html).toContain("If nothing happens:");
    expect(html).toContain("The importer work stays blocked.");
    // option button + its predicted effect line
    expect(html).toContain('name="optionId" value="csv"');
    expect(html).toContain("Effect: Importer unblocks this week");
    // survey: prompt, required coverage line, submit
    expect(html).toContain("Who consumes the export?");
    expect(html).toContain("Needed: the privacy review scope");
    expect(html).toContain("Submit answers");
    expect(html).toContain('name="answers:audience"');
    // delegation controls + dismiss are on the card
    expect(html).toContain('data-verb="delegate"');
    expect(html).toContain("Delegate: Auto");
    expect(html).toContain('data-verb="dismiss"');
    // no free-text answer box when the item offers options/survey
    expect(html).not.toContain("Your answer / decision");
  });

  test("an access item's Auto control is disabled with the reason", async () => {
    const item: TierItem = {
      ...BASE,
      whyUser: "access",
      delegation: { mode: "user", status: "idle", updatedAt: recent() },
    };
    const html = await page([item]);
    expect(html).toContain(">Access</span>");
    expect(html).toContain("can never be auto-delegated");
  });

  test("a live proposal renders its box + approve/reject forms carrying the identity", async () => {
    const html = await page([proposalItem(recent())]);
    expect(html).toContain("First-pass proposal");
    expect(html).toContain("Recommend CSV");
    expect(html).toContain('data-verb="approve-proposal"');
    expect(html).toContain('data-verb="reject-proposal"');
    expect(html).toContain('name="proposalRunId" value="orch-2026-10-27T02:00:00Z"');
    expect(html).toContain("nothing is applied unless you approve");
  });

  test("a re-proposal names the previous rejection in its box", async () => {
    const item = proposalItem(recent());
    const rejection: DashAction = {
      id: "da-old",
      clientToken: "ct-old",
      kind: "triage",
      verb: "reject-proposal",
      objectRef: "gh:developing-today/code#2001",
      payload: { reason: "too risky as drafted" },
      author: { kind: "drew", id: "drew", label: "Drew" },
      createdAt: recent(),
      state: "applied",
    };
    const html = await page([item], new Map([["gh:developing-today/code#2001", [rejection]]]));
    expect(html).toContain("second first pass; previous rejected: too risky as drafted");
  });

  test("an expired proposal renders expired and offers no approve form", async () => {
    const html = await page([proposalItem(longAgo())]);
    expect(html).toContain("Proposal expired");
    expect(html).not.toContain('data-verb="approve-proposal"');
    expect(html).toContain("Delegate: First Pass (fresh draft)");
  });

  test("an unable delegation renders the structured inability", async () => {
    const item: TierItem = {
      ...BASE,
      whyUser: "decision",
      delegation: {
        mode: "auto",
        status: "unable",
        updatedAt: recent(),
        runId: "orch-x",
        unable: {
          reason: "No recorded context settles the format.",
          whatItTried: ["searched the repo docs", "checked the decision log"],
          whatItNeeds: "a product call on the public audience",
          at: recent(),
        },
      },
    };
    const html = await page([item]);
    expect(html).toContain("The orchestrator could not resolve this");
    expect(html).toContain("searched the repo docs");
    expect(html).toContain("Needs: a product call on the public audience");
  });

  test("a stale tier source disables the needs-user controls with the triage ones", async () => {
    const html = await page([proposalItem(recent())], new Map(), true);
    expect(html).toContain("Actions unavailable");
    expect(html).toContain("<fieldset disabled>");
    expect(html).toContain('data-verb="delegate"');
  });

  test("the standing-default toggle renders its state and the preference form", async () => {
    const html = await page([{ ...BASE }]);
    expect(html).toContain("Standing delegation default");
    expect(html).toContain("delegate to Auto by default");
    expect(html).toContain('data-kind="preference"');
    expect(html).toContain('data-verb="set-delegation-default"');
    const pendingPref: DashAction = {
      id: "da-pref",
      clientToken: "ct-pref",
      kind: "preference",
      verb: "set-delegation-default",
      objectRef: "needs-user-delegation-default",
      payload: { standingAuto: false },
      author: { kind: "drew", id: "drew", label: "Drew" },
      createdAt: recent(),
      state: "new",
    };
    const html2 = await page([{ ...BASE }], new Map([["needs-user-delegation-default", [pendingPref]]]));
    expect(html2).toContain("items stay with you unless you delegate them per item");
    expect(html2).toContain("change pending");
  });
});
