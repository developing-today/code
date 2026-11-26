// MOB-06 contract tests (#9473): safe status, note, comment and priority
// controls through the MOB-05 envelope.
//
// Oracles used here:
// - one policy result per verb: `planMobileAction` returns exactly one
//   allow or deny, and an allow names the owner change the fake owners
//   then apply;
// - the fake owners' state: a denied verb changes nothing, an allowed verb
//   changes exactly the owner it named;
// - the MOB-05 envelope: a stale work version ends in `conflict` with no
//   owner effect;
// - the PRI-01 owner-policy gate order (`check_priority_write`): missing
//   policy fails closed, missing ref, stale ref, then owner-only writes to
//   protected items;
// - the sync-loop guard: the label a priority write projects, read back
//   in, is an echo and plans nothing.
import { describe, expect, test } from "bun:test";
import {
  type ChangeOwner,
  MOBILE_VERBS,
  type MobilePolicyResult,
  type MobilePrincipal,
  type MobileVerbRequest,
  type OwnerPolicyView,
  PRIORITY_MIRROR_LABELS,
  QUEUE_V1_GOLDEN,
  type SourceOfTruthChange,
  type WorkItemFacts,
  inboundPriorityLabelIsEcho,
  planMobileAction,
  priorityLabelFor,
  toWorkActionRequest,
} from "../src/mobile-work-actions.ts";
import {
  type WorkActionAuthority,
  type WorkActionReceipt,
  type WorkActionRequest,
  acceptWorkAction,
  applyWorkAction,
  createWorkAction,
  previewWorkAction,
} from "../src/work-action-contract.ts";

const DREW: MobilePrincipal = { principalRef: "principal:drew" };
const OTHER: MobilePrincipal = { principalRef: "principal:agent-7" };
const POLICY_REF = "fleet.owner-priority-policy/v1#3";

const facts = (over: Partial<WorkItemFacts> = {}): WorkItemFacts => ({
  objectRef: "orch:item-9473",
  workVersion: "wv-1",
  lifecycle: "open",
  manualOnly: false,
  openHardBlockers: [],
  priority: 40,
  doNow: false,
  ...over,
});

const policy = (protectedRefs: readonly string[] = []): OwnerPolicyView => ({
  policyRef: POLICY_REF,
  ownerPrincipalRef: DREW.principalRef,
  protectedMembers: new Set(protectedRefs),
});

/** Fake orchestrator and Fleet owners: one bucket per owner, plus the MOB-05 authority. */
function owners(initial: WorkItemFacts) {
  let version = initial.workVersion;
  let n = 1;
  const applied: SourceOfTruthChange[] = [];
  const remaps: string[] = [];
  const operations = new Map<string, WorkActionReceipt | null>();
  const roles = ["work-action/preview", "work-action/accept", "work-action/apply"];
  const authority: WorkActionAuthority = {
    readWorkVersion: (ref) => (ref === initial.objectRef ? version : null),
    readRoles: () => roles,
    issueEffect: (request: WorkActionRequest) => {
      const payload = request.payload as {
        changes: SourceOfTruthChange[];
        rerunPriorityMapping: boolean;
      };
      applied.push(...payload.changes);
      if (payload.rerunPriorityMapping) remaps.push(request.objectRef);
      version = `wv-${++n}`;
    },
    claimOperation: (key) => {
      if (!operations.has(key)) {
        operations.set(key, null);
        return { claimed: true };
      }
      return { claimed: false, receipt: operations.get(key) ?? null };
    },
    recordReceipt: (r) => operations.set(r.operationKey, r),
  };
  const actor = { kind: "drew", id: "drew", roles };
  /** Run an allowed plan through the full MOB-05 envelope. */
  const run = (f: WorkItemFacts, result: MobilePolicyResult, key: string) => {
    let env = createWorkAction(toWorkActionRequest(f, result, key));
    env = previewWorkAction(env, actor, authority);
    if (env.state !== "previewed") return env;
    env = acceptWorkAction(env, actor, authority);
    if (env.state !== "accepted") return env;
    return applyWorkAction(env, actor, authority, {
      outcome: "verified_complete",
    });
  };
  return {
    authority,
    applied,
    remaps,
    run,
    bump: () => (version = `wv-${++n}`),
  };
}

describe("MOB-06 named tests", () => {
  test("mobile_private_note_is_not_public_by_default", () => {
    const f = facts();
    const o = owners(f);
    const result = planMobileAction(f, { verb: "note.add", body: "check the lease first" }, policy(), DREW);
    expect(result.kind).toBe("allow");
    const env = o.run(f, result, "op-note-1");
    expect(env.state).toBe("verified_complete");
    expect(o.applied).toHaveLength(1);
    expect(o.applied[0]).toMatchObject({
      owner: "fleet.journal",
      fields: { visibility: "private" },
    });
    expect(o.applied.some((c) => c.owner === "github.comment")).toBe(false);
  });

  test("pmo_mobile_protected_expedite_edit_requires_drew", () => {
    const f = facts({ priority: 15 });
    const p = policy([f.objectRef]);
    const req: MobileVerbRequest = {
      verb: "priority.set",
      value: 40,
      reason: "demote",
      ownerPolicyRef: POLICY_REF,
      ownerAction: true,
    };
    // Someone else, even claiming an owner action, is refused.
    const other = planMobileAction(f, req, p, OTHER);
    expect(other).toMatchObject({
      kind: "deny",
      code: "OWNER_POLICY_PROTECTED",
    });
    // Drew on a routine mobile edit (no explicit owner action) is refused too.
    const routine = planMobileAction(f, { ...req, ownerAction: false }, p, DREW);
    expect(routine).toMatchObject({
      kind: "deny",
      code: "OWNER_POLICY_PROTECTED",
    });
    // Drew's explicit owner action passes and keeps the exact ratified value as "before".
    const owner = planMobileAction(f, req, p, DREW);
    expect(owner.kind).toBe("allow");
    if (owner.kind === "allow") expect(owner.changes[0].fields).toMatchObject({ before: 15, after: 40 });
  });
});

describe("MOB-06 failing-test list from the slice", () => {
  test("conflicting revision ends in conflict with no owner effect", () => {
    const f = facts();
    const o = owners(f);
    const result = planMobileAction(f, { verb: "status.request", target: "ready" }, policy(), DREW);
    o.bump(); // the work moved after the facts were read
    const env = o.run(f, result, "op-stale-1");
    expect(env.state).toBe("conflict");
    expect(o.applied).toHaveLength(0);
  });

  test("unsupported Project Status is refused, and Done is never requestable", () => {
    for (const status of ["Shipped", "in review", "Blocked", ""]) {
      expect(planMobileAction(facts(), { verb: "project.request_status", status }, policy(), DREW)).toMatchObject({
        kind: "deny",
        code: "UNSUPPORTED_PROJECT_STATUS",
      });
    }
    expect(planMobileAction(facts(), { verb: "project.request_status", status: "Done" }, policy(), DREW)).toMatchObject(
      {
        kind: "deny",
        code: "DONE_REQUIRES_VERIFIED_RECEIPT",
      },
    );
    const ok = planMobileAction(facts(), { verb: "project.request_status", status: "In Review" }, policy(), DREW);
    expect(ok).toMatchObject({ kind: "allow" });
    if (ok.kind === "allow") expect(ok.changes[0].owner).toBe("orchestrator.project_status");
  });

  test("noncanonical and contradictory priority labels are refused", () => {
    const base = {
      verb: "priority.set" as const,
      reason: "triage",
      ownerPolicyRef: POLICY_REF,
    };
    const cases: [MobileVerbRequest, string][] = [
      [{ ...base, value: 35 }, "NONCANONICAL_PRIORITY"],
      [{ ...base, value: 20, labels: ["priority/very-high"] }, "NONCANONICAL_PRIORITY"],
      [{ ...base, value: 20, labels: ["P1"] }, "NONCANONICAL_PRIORITY"],
      [{ ...base, value: 20, labels: ["priority/low"] }, "CONTRADICTORY_PRIORITY_LABELS"],
      [{ ...base, value: 15, labels: ["expedite", "next-up"] }, "CONTRADICTORY_PRIORITY_LABELS"],
      [{ ...base, value: 30, labels: ["priority/normal"] }, "CONTRADICTORY_PRIORITY_LABELS"],
    ];
    for (const [req, code] of cases) {
      expect(planMobileAction(facts(), req, policy(), DREW)).toMatchObject({
        kind: "deny",
        code,
      });
    }
    // Ratified bands survive exactly with their own mirror label.
    for (const value of [15, 25]) {
      const r = planMobileAction(
        facts(),
        { ...base, value, labels: [priorityLabelFor(value) as string] },
        policy(),
        DREW,
      );
      expect(r.kind).toBe("allow");
      if (r.kind === "allow") expect(r.changes[0].fields.after).toBe(value);
    }
  });

  test("blocked, parked and manual_only items cannot take do-now", () => {
    for (const over of [{ lifecycle: "blocked" as const }, { lifecycle: "parked" as const }, { manualOnly: true }]) {
      expect(
        planMobileAction(facts(over), { verb: "directive.do_now", set: true, confirmed: true }, policy(), DREW),
      ).toMatchObject({ kind: "deny", code: "DO_NOW_HARD_GATE" });
    }
    expect(
      planMobileAction(
        facts({ openHardBlockers: ["code#1"] }),
        { verb: "directive.do_now", set: true, confirmed: true },
        policy(),
        DREW,
      ),
    ).toMatchObject({ kind: "deny", code: "DO_NOW_HARD_GATE" });
    // Clearing a directive on a blocked item is allowed and touches no score.
    const clear = planMobileAction(
      facts({ lifecycle: "blocked" }),
      { verb: "directive.do_now", set: false },
      policy(),
      DREW,
    );
    expect(clear.kind).toBe("allow");
    if (clear.kind === "allow") {
      expect(clear.changes.map((c) => c.owner)).toEqual(["fleet.directive"]);
      expect(clear.rerunPriorityMapping).toBe(false);
    }
  });

  test("a private note is never sent public by accident", () => {
    const unpreviewed = planMobileAction(
      facts(),
      { verb: "note.add", body: "x", visibility: "public" },
      policy(),
      DREW,
    );
    expect(unpreviewed).toMatchObject({
      kind: "deny",
      code: "PUBLIC_MIRROR_NOT_PREVIEWED",
    });
    const mirrored = planMobileAction(
      facts(),
      {
        verb: "note.add",
        body: "x",
        visibility: "public",
        publicMirrorPreviewed: true,
      },
      policy(),
      DREW,
    );
    expect(mirrored.kind).toBe("allow");
    if (mirrored.kind === "allow")
      expect(mirrored.changes.map((c) => c.owner)).toEqual(["fleet.journal", "github.comment"]);
    expect(planMobileAction(facts(), { verb: "comment.add", body: "x" }, policy(), DREW)).toMatchObject({
      kind: "deny",
      code: "CONFIRMATION_REQUIRED",
    });
  });

  test("a status request cannot bypass a hard blocker", () => {
    const blocked = facts({
      lifecycle: "blocked",
      openHardBlockers: ["code#9200"],
    });
    expect(planMobileAction(blocked, { verb: "status.request", target: "ready" }, policy(), DREW)).toMatchObject({
      kind: "deny",
      code: "HARD_BLOCKER_OPEN",
    });
    // Urgency cannot promote it either: a priority write never touches lifecycle.
    const urgent = planMobileAction(
      blocked,
      {
        verb: "priority.set",
        value: 10,
        reason: "hot",
        ownerPolicyRef: POLICY_REF,
      },
      policy(),
      DREW,
    );
    expect(urgent.kind).toBe("allow");
    if (urgent.kind === "allow") expect(urgent.changes.every((c) => c.owner === "fleet.priority")).toBe(true);
  });
});

describe("owner-policy gate order (PRI-01 check_priority_write)", () => {
  const req = { verb: "priority.set" as const, value: 20, reason: "triage" };
  test("missing policy fails closed, then missing ref, then stale ref", () => {
    expect(planMobileAction(facts(), { ...req, ownerPolicyRef: POLICY_REF }, null, DREW)).toMatchObject({
      code: "owner_policy_unavailable",
    });
    expect(planMobileAction(facts(), req, policy(), DREW)).toMatchObject({
      code: "owner_policy_ref_missing",
    });
    expect(
      planMobileAction(facts(), { ...req, ownerPolicyRef: "fleet.owner-priority-policy/v1#2" }, policy(), DREW),
    ).toMatchObject({
      code: "OWNER_POLICY_VERSION_CONFLICT",
    });
  });
});

describe("golden corpus: every verb, one result, one owner change, no sync loop", () => {
  const corpus: {
    request: MobileVerbRequest;
    owners: ChangeOwner[];
    remap: boolean;
  }[] = [
    {
      request: { verb: "note.add", body: "n" },
      owners: ["fleet.journal"],
      remap: false,
    },
    {
      request: { verb: "comment.add", body: "c", confirmed: true },
      owners: ["github.comment"],
      remap: false,
    },
    {
      request: {
        verb: "priority.set",
        value: 25,
        reason: "r",
        ownerPolicyRef: POLICY_REF,
        labels: ["next-up"],
      },
      owners: ["fleet.priority"],
      remap: true,
    },
    {
      request: { verb: "directive.do_now", set: true, confirmed: true },
      owners: ["fleet.directive"],
      remap: false,
    },
    {
      request: {
        verb: "status.request",
        target: "parked",
        reason: "waiting on review",
        checkpoint: "branch pushed",
      },
      owners: ["fleet.lifecycle"],
      remap: false,
    },
    {
      request: { verb: "project.request_status", status: "Todo" },
      owners: ["orchestrator.project_status"],
      remap: false,
    },
  ];

  test("the corpus covers every verb", () => {
    expect(new Set(corpus.map((c) => c.request.verb))).toEqual(new Set(MOBILE_VERBS));
  });

  for (const [i, entry] of corpus.entries()) {
    test(`${entry.request.verb} applies once to ${entry.owners.join("+")}`, () => {
      const f = facts();
      const o = owners(f);
      const result = planMobileAction(f, entry.request, policy(), DREW);
      expect(result.kind).toBe("allow");
      const key = `op-golden-${i}`;
      expect(o.run(f, result, key).state).toBe("verified_complete");
      expect(o.applied.map((c) => c.owner)).toEqual(entry.owners);
      expect(o.remaps.length > 0).toBe(entry.remap);
      // Replaying the same operation key issues no second effect.
      o.run(f, result, key);
      expect(o.applied.map((c) => c.owner)).toEqual(entry.owners);
    });
  }

  test("every priority write re-runs the #9200 mapping", () => {
    for (const value of QUEUE_V1_GOLDEN.keys()) {
      const r = planMobileAction(
        facts(),
        {
          verb: "priority.set",
          value,
          reason: "r",
          ownerPolicyRef: POLICY_REF,
        },
        policy(),
        DREW,
      );
      expect(r).toMatchObject({ kind: "allow", rerunPriorityMapping: true });
    }
  });

  test("the projected mirror label read back in is an echo, not a new write", () => {
    for (const [value, label] of PRIORITY_MIRROR_LABELS) {
      expect(inboundPriorityLabelIsEcho(facts({ priority: value }), [label, "area/pm"])).toBe(true);
      expect(inboundPriorityLabelIsEcho(facts({ priority: value }), [label, "priority/very-high"])).toBe(false);
    }
    expect(inboundPriorityLabelIsEcho(facts({ priority: 30 }), [])).toBe(true);
    expect(inboundPriorityLabelIsEcho(facts({ priority: 40 }), [])).toBe(false);
  });

  test("a denied plan never becomes a work action", () => {
    const denied = planMobileAction(facts(), { verb: "comment.add", body: "c" }, policy(), DREW);
    expect(() => toWorkActionRequest(facts(), denied, "op-x")).toThrow(/never becomes a work action/);
  });
});
