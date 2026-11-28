// MOB-06 contract tests (#9473). Model-based: the policy table below IS
// the design's safe-controls contract (component 9201 §3.2–§3.6, slice
// MOB-06), and each test names the oracle that decides its verdict.
//
// Oracles used here:
// - one decision per verb — `decideMobileWorkAction` returns exactly one
//   policy result and at most one expected source-of-truth change, so a
//   verb can never half-apply or fan out to two writers (no sync loop);
// - the fake authority fixture — stale expected versions end in
//   `conflict`, unreadable versions in `uncertain`, never a silent apply;
// - the fake sinks — a private-default note leaves the public comment
//   sink untouched; only an explicitly confirmed public action writes it;
// - the #9200 seam — `urgency.change` records a canonical-model re-run
//   (`reruns9200`) and never a locally computed score, and protected
//   labels (`expedite`, `next-up`) require the owner.
import { describe, expect, test } from "bun:test";
import {
  MOBILE_WORK_VERBS,
  decideMobileWorkAction,
  type MobileWorkArgs,
  type MobileWorkAuthority,
  type MobileWorkDecision,
  type MobileWorkSnapshot,
  type MobileWorkVerb,
} from "../src/mobile-work-actions.ts";

/** The fake authority: one work version per object, mutable so a test can
 *  move the work or make it unreadable between cases. */
function fixture(version: string | null) {
  const versions = new Map([["orch:item-9473", version]]);
  const authority: MobileWorkAuthority = {
    readWorkVersion: (ref) => versions.get(ref) ?? null,
  };
  return { authority, setVersion: (v: string | null) => versions.set("orch:item-9473", v) };
}

const snapshot = (over: Partial<MobileWorkSnapshot> = {}): MobileWorkSnapshot => ({
  objectRef: "orch:item-9473",
  expectedWorkVersion: "wv-3",
  lifecycle: "ready",
  hardBlocker: false,
  projectStatus: "In Progress",
  projectStatusSupported: true,
  priorityLabelsCanonical: true,
  ...over,
});

const args = (over: Partial<MobileWorkArgs> = {}): MobileWorkArgs => ({ ...over });

function decide(
  verb: MobileWorkVerb,
  overArgs: Partial<MobileWorkArgs> = {},
  overSnapshot: Partial<MobileWorkSnapshot> = {},
  version: string | null = "wv-3",
): { decision: MobileWorkDecision; fx: ReturnType<typeof fixture> } {
  const fx = fixture(version);
  return { decision: decideMobileWorkAction(verb, args(overArgs), snapshot(overSnapshot), fx.authority), fx };
}

describe("mobile work actions — private note default (oracle: fake sinks)", () => {
  test("mobile_private_note_is_not_public_by_default", () => {
    const publicComments: string[] = [];
    const privateJournal: string[] = [];
    const { decision } = decide("note.private", { body: "checkpoint for the next agent" });
    expect(decision.policy).toBe("allowed");
    expect(decision.change?.surface).toBe("fleet.private-journal");
    // The harness routes the single expected change to its one owner
    // surface; the public sink is never touched by a private default.
    if (decision.change?.surface === "fleet.private-journal") privateJournal.push("note");
    if (decision.change?.surface === "fleet.public-comment") publicComments.push("note");
    expect(privateJournal).toHaveLength(1);
    expect(publicComments).toHaveLength(0);
  });

  test("a public mirror without explicit select+preview is rejected before any effect", () => {
    const { decision } = decide("note.private", {
      body: "checkpoint",
      publicMirrorSelected: true,
      publicMirrorPreviewed: false,
    });
    expect(decision.policy).toBe("rejected");
    expect(decision.reasonCode).toBe("rejected.public_mirror_not_previewed");
    expect(decision.change).toBeNull();
  });

  test("an explicitly selected and previewed public mirror is allowed and marked mirrored", () => {
    const { decision } = decide("note.private", {
      body: "checkpoint",
      publicMirrorSelected: true,
      publicMirrorPreviewed: true,
    });
    expect(decision.policy).toBe("allowed");
    expect(decision.change?.surface).toBe("fleet.private-journal");
    expect(decision.change?.mirroredPublic).toBe(true);
  });
});

describe("mobile work actions — stale and unreadable authority (oracle: fake fixture)", () => {
  test("a conflicting expected version ends in conflict with no change", () => {
    const { decision } = decide("note.private", { body: "late note" }, {}, "wv-9");
    expect(decision.policy).toBe("conflict");
    expect(decision.reasonCode).toBe("conflict.stale_expected_version");
    expect(decision.change).toBeNull();
  });

  test("an unreadable authoritative version ends in uncertain, never a silent apply", () => {
    const { decision } = decide("status.request", { statusTarget: "parked" }, {}, null);
    expect(decision.policy).toBe("uncertain");
    expect(decision.reasonCode).toBe("uncertain.unreadable_authority");
    expect(decision.change).toBeNull();
  });
});

describe("mobile work actions — public comment (oracle: fake sinks)", () => {
  test("an unconfirmed public comment is rejected before any effect", () => {
    const { decision } = decide("comment.public", { body: "hello" });
    expect(decision.policy).toBe("rejected");
    expect(decision.reasonCode).toBe("rejected.public_confirmation_required");
    expect(decision.change).toBeNull();
  });

  test("a confirmed public comment writes exactly one public effect", () => {
    const publicComments: string[] = [];
    const { decision } = decide("comment.public", { body: "hello", publicConfirmed: true });
    expect(decision.policy).toBe("allowed");
    expect(decision.change?.surface).toBe("fleet.public-comment");
    if (decision.change?.surface === "fleet.public-comment") publicComments.push("hello");
    expect(publicComments).toHaveLength(1);
  });
});

describe("mobile work actions — urgency and the #9200 seam (oracle: rerun flag)", () => {
  test("noncanonical priority labels reject the change before any re-run", () => {
    const { decision } = decide(
      "urgency.change",
      { urgencyTarget: "high" },
      { priorityLabelsCanonical: false },
    );
    expect(decision.policy).toBe("rejected");
    expect(decision.reasonCode).toBe("rejected.noncanonical_priority_labels");
    expect(decision.change).toBeNull();
  });

  test("pmo_mobile_protected_expedite_edit_requires_drew", () => {
    const denied = decide("urgency.change", { urgencyTarget: "expedite", actorIsDrew: false });
    expect(denied.decision.policy).toBe("rejected");
    expect(denied.decision.reasonCode).toBe("rejected.protected_expedite_edit");
    expect(denied.decision.change).toBeNull();

    const allowed = decide("urgency.change", { urgencyTarget: "expedite", actorIsDrew: true });
    expect(allowed.decision.policy).toBe("allowed");
    expect(allowed.decision.change?.surface).toBe("fleet.urgency-rerun-9200");
  });

  test("an allowed urgency change re-runs #9200 and never carries a local score", () => {
    const { decision } = decide("urgency.change", { urgencyTarget: "high", actorIsDrew: false });
    expect(decision.policy).toBe("allowed");
    expect(decision.change?.reruns9200).toBe(true);
    expect(decision.change && "localScore" in decision.change).toBe(false);
  });
});

describe("mobile work actions — do-now (oracle: single directive, urgency untouched)", () => {
  test("do-now on a blocked item is rejected", () => {
    const { decision } = decide("do-now.set", {}, { lifecycle: "blocked" });
    expect(decision.policy).toBe("rejected");
    expect(decision.reasonCode).toBe("rejected.blocked_do_now");
    expect(decision.change).toBeNull();
  });

  test("do-now set issues one directive and never modifies urgency", () => {
    const { decision } = decide("do-now.set", {}, { lifecycle: "ready" });
    expect(decision.policy).toBe("allowed");
    expect(decision.change?.surface).toBe("fleet.directive");
    expect(decision.change?.urgencyUnchanged).toBe(true);
  });
});

describe("mobile work actions — status requests (oracle: lifecycle and blocker gates)", () => {
  test("a status request that bypasses a hard blocker is rejected", () => {
    const { decision } = decide("status.request", { statusTarget: "ready" }, { hardBlocker: true });
    expect(decision.policy).toBe("rejected");
    expect(decision.reasonCode).toBe("rejected.hard_blocker_present");
    expect(decision.change).toBeNull();
  });

  test("an unsupported Project Status is rejected, not silently mapped", () => {
    const { decision } = decide(
      "status.request",
      { statusTarget: "parked" },
      { projectStatus: "Banana", projectStatusSupported: false },
    );
    expect(decision.policy).toBe("rejected");
    expect(decision.reasonCode).toBe("rejected.unsupported_project_status");
    expect(decision.change).toBeNull();
  });

  test("a permitted status request yields one status-router change", () => {
    const { decision } = decide("status.request", { statusTarget: "parked" });
    expect(decision.policy).toBe("allowed");
    expect(decision.change?.surface).toBe("fleet.status-request");
  });
});

describe("mobile work actions — golden corpus (oracle: one result, one surface, no sync loop)", () => {
  const allowedCases: Array<{ verb: MobileWorkVerb; over: Partial<MobileWorkArgs> }> = [
    { verb: "note.private", over: { body: "n" } },
    { verb: "comment.public", over: { body: "c", publicConfirmed: true } },
    { verb: "urgency.change", over: { urgencyTarget: "high" } },
    { verb: "do-now.set", over: {} },
    { verb: "do-now.clear", over: {} },
    { verb: "status.request", over: { statusTarget: "parked" } },
  ];

  test("every verb is decided and every allowed verb names exactly one owner surface", () => {
    expect(MOBILE_WORK_VERBS).toHaveLength(6);
    const seen = new Set<string>();
    for (const { verb, over } of allowedCases) {
      const { decision } = decide(verb, over);
      expect(decision.policy).toBe("allowed");
      // Exactly one surface: the type carries a single string, and the
      // corpus pins the verb's surface so a second writer cannot appear.
      expect(decision.change?.surface).toBeDefined();
      seen.add(`${verb}:${decision.change?.surface}`);
    }
    expect(seen).toEqual(
      new Set([
        "note.private:fleet.private-journal",
        "comment.public:fleet.public-comment",
        "urgency.change:fleet.urgency-rerun-9200",
        "do-now.set:fleet.directive",
        "do-now.clear:fleet.directive",
        "status.request:fleet.status-request",
      ]),
    );
  });

  test("only urgency.change re-runs #9200; only directives leave urgency untouched", () => {
    for (const { verb, over } of allowedCases) {
      const { decision } = decide(verb, over);
      expect(decision.change?.reruns9200).toBe(verb === "urgency.change");
      if (verb === "do-now.set" || verb === "do-now.clear") {
        expect(decision.change?.urgencyUnchanged).toBe(true);
      }
    }
  });
});
