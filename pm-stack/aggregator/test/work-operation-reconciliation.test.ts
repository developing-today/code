// MOB-09 reconciliation tests (#9476). Each test names the oracle that
// decides it, following the design record
// doc/2026-11-25T22-32-18Z_design_mob09_reconciliation_freshness_rollback.
//
// Oracles used here:
// - the source readback fixture: a read of the authoritative source taken
//   after the effect was issued, with a revision that moved past the prior
//   one. Nothing else (the writer's own response, a stale or fixture read,
//   an observed_at stamp) can make an effect verified;
// - MOB-05's declared edge table: every transition a reconciliation records
//   is one WORK_ACTION_EDGES declares;
// - the effect sink's call count: with mutations off, no effect is issued.
import { describe, expect, test } from "bun:test";
import {
  type WorkActionActor,
  type WorkActionAuthority,
  type WorkActionEnvelope,
  type WorkActionReceipt,
  type WorkActionRequest,
  acceptWorkAction,
  applyWorkAction,
  canTransition,
  createWorkAction,
  previewWorkAction,
} from "../src/work-action-contract.ts";
import {
  type DesiredEffect,
  type Readback,
  type ReconcileOptions,
  applyIfMutationsEnabled,
  reconcileWorkAction,
} from "../src/work-operation-reconciliation.ts";

const ALL_ROLES = ["work-action/preview", "work-action/accept", "work-action/apply"] as const;
const actor: WorkActionActor = { kind: "drew", id: "drew", roles: ALL_ROLES };

function fixture(objectRef: string, version: string) {
  const effects: WorkActionRequest[] = [];
  const operations = new Map<string, WorkActionReceipt | null>();
  const authority: WorkActionAuthority = {
    readWorkVersion: (ref) => (ref === objectRef ? version : null),
    readRoles: () => ALL_ROLES,
    issueEffect: (request) => effects.push(request),
    claimOperation: (key) => {
      if (!operations.has(key)) {
        operations.set(key, null);
        return { claimed: true };
      }
      return { claimed: false, receipt: operations.get(key) ?? null };
    },
    recordReceipt: (receipt) => operations.set(receipt.operationKey, receipt),
  };
  return { authority, effects };
}

const REF = "orch:item-9476";
const request = (over: Partial<WorkActionRequest> = {}): WorkActionRequest => ({
  operationKey: "op-9476-0001",
  kind: "triage",
  objectRef: REF,
  action: "reprioritize",
  payload: { priority: 30 },
  expectedWorkVersion: "7",
  ...over,
});

function acceptedEnvelope(over: Partial<WorkActionRequest> = {}) {
  const fx = fixture(REF, "7");
  const envelope = acceptWorkAction(
    previewWorkAction(createWorkAction(request(over)), actor, fx.authority),
    actor,
    fx.authority,
  );
  return { fx, envelope };
}

/** The writer said "accepted": MOB-05 records the downstream's pending_sync. */
function pendingEnvelope(over: Partial<WorkActionRequest> = {}): WorkActionEnvelope {
  const { fx, envelope } = acceptedEnvelope(over);
  return applyWorkAction(envelope, actor, fx.authority, { outcome: "pending_sync", note: "writer accepted" });
}

function partialEnvelope(): WorkActionEnvelope {
  const { fx, envelope } = acceptedEnvelope();
  return applyWorkAction(envelope, actor, fx.authority, { outcome: "partial" });
}

/** A running envelope, as MOB-05 holds it between effect and outcome. */
function runningEnvelope(): WorkActionEnvelope {
  const { envelope } = acceptedEnvelope();
  return {
    ...envelope,
    state: "running",
    effectIssued: true,
    transitions: [...envelope.transitions, { from: "accepted", to: "running", oracle: "work-action/effect-issued" }],
  };
}

const ISSUED = "2026-11-25T12:00:00Z";
const effect = (over: Partial<DesiredEffect> = {}): DesiredEffect => ({
  effectId: "e1",
  objectRef: REF,
  source: "github",
  field: "priority",
  desired: 30,
  priorValue: 50,
  priorRevision: { kind: "work_version", value: "7" },
  issuedAt: ISSUED,
  ...over,
});

const read = (over: Partial<Readback> = {}): Readback => ({
  readId: "r1",
  effectId: "e1",
  objectRef: REF,
  matchedObjects: 1,
  source: "github",
  field: "priority",
  value: 30,
  revision: { kind: "work_version", value: "8" },
  observedAt: "2026-11-25T12:00:05Z",
  freshness: "live",
  ...over,
});

const opts = (over: Partial<ReconcileOptions> = {}): ReconcileOptions => ({
  now: "2026-11-25T12:00:10Z",
  syncDeadlineMs: { github: 60_000 },
  ...over,
});

function expectDeclaredEdges(envelope: WorkActionEnvelope) {
  for (const t of envelope.transitions) {
    expect(canTransition(t.from, t.to)).toBe(true);
  }
}

describe("MOB-09 readback is the only proof (oracle: source readback fixture)", () => {
  test("work_verified_effect_requires_source_readback", () => {
    // The writer accepted; the source still shows the old value at the old revision.
    const result = reconcileWorkAction(
      pendingEnvelope(),
      [effect()],
      [read({ value: 50, revision: { kind: "work_version", value: "7" } })],
      opts(),
    );
    expect(result.envelope.state).toBe("pending_sync");
    expect(result.effects[0]?.result).toBe("not_yet");
    expect(result.displayState).toBe("pending_sync");
  });

  test("work_write_response_is_not_a_readback", () => {
    expect(() =>
      reconcileWorkAction(pendingEnvelope(), [effect()], [read({ fromWriteResponse: true })], opts()),
    ).toThrow(/never a readback/);
  });

  test("work_readback_promotes_to_verified_with_second_read", () => {
    const first = reconcileWorkAction(
      pendingEnvelope(),
      [effect()],
      [read({ readId: "r1", value: 50, revision: { kind: "work_version", value: "7" } })],
      opts(),
    );
    expect(first.envelope.state).toBe("pending_sync");
    const second = reconcileWorkAction(
      first.envelope,
      [effect()],
      [read({ readId: "r2", observedAt: "2026-11-25T12:00:20Z" })],
      opts({ now: "2026-11-25T12:00:30Z" }),
    );
    expect(second.envelope.state).toBe("verified_complete");
    expect(second.reason).toBe("readback_verified");
    expect(second.envelope.receipt).toMatchObject({ outcome: "verified_complete", workVersion: "8" });
    expect(second.effects[0]).toMatchObject({ result: "verified", readId: "r2", observed: 30 });
    expectDeclaredEdges(second.envelope);
  });

  test("a stale, errored or fixture read never verifies", () => {
    for (const freshness of ["stale", "error", "fixtures"] as const) {
      const result = reconcileWorkAction(pendingEnvelope(), [effect()], [read({ freshness })], opts());
      expect(result.envelope.state).toBe("pending_sync");
      expect(result.effects[0]?.result).toBe("unread");
    }
  });

  test("an observed_at stamp is not a revision and never verifies", () => {
    const result = reconcileWorkAction(
      pendingEnvelope(),
      [effect({ priorRevision: null })],
      [read({ revision: { kind: "observed_at", value: "2026-11-25T12:00:05Z" } })],
      opts(),
    );
    expect(result.envelope.state).toBe("pending_sync");
    expect(result.effects[0]?.result).toBe("unread");
  });

  test("a read taken before the effect was issued never verifies", () => {
    const result = reconcileWorkAction(
      pendingEnvelope(),
      [effect()],
      [read({ observedAt: "2026-11-25T11:59:59Z" })],
      opts(),
    );
    expect(result.effects[0]?.result).toBe("unread");
    expect(result.envelope.state).toBe("pending_sync");
  });

  test("a revision that did not advance is not yet, even with the desired value", () => {
    const result = reconcileWorkAction(
      pendingEnvelope(),
      [effect()],
      [read({ revision: { kind: "work_version", value: "7" } })],
      opts(),
    );
    expect(result.effects[0]?.result).toBe("not_yet");
  });

  test("updated_at revisions compare as times", () => {
    const e = effect({ priorRevision: { kind: "updated_at", value: "2026-11-25T11:00:00Z" } });
    const verified = reconcileWorkAction(
      pendingEnvelope(),
      [e],
      [read({ revision: { kind: "updated_at", value: "2026-11-25T12:00:04Z" } })],
      opts(),
    );
    expect(verified.envelope.state).toBe("verified_complete");
    const older = reconcileWorkAction(
      pendingEnvelope(),
      [e],
      [read({ revision: { kind: "updated_at", value: "2026-11-25T10:00:00Z" } })],
      opts(),
    );
    expect(older.effects[0]?.result).toBe("not_yet");
  });
});

describe("MOB-09 outcomes (oracle: MOB-05 declared edges)", () => {
  test("drift from pending_sync is uncertain", () => {
    const result = reconcileWorkAction(pendingEnvelope(), [effect()], [read({ value: 99 })], opts());
    expect(result.effects[0]?.result).toBe("drift");
    expect(result.envelope.state).toBe("uncertain");
    expect(result.reason).toBe("drift");
    expectDeclaredEdges(result.envelope);
  });

  test("drift from partial is conflict", () => {
    const result = reconcileWorkAction(partialEnvelope(), [effect()], [read({ value: 99 })], opts());
    expect(result.envelope.state).toBe("conflict");
    expectDeclaredEdges(result.envelope);
  });

  test("one of two effects verified is partial, then complete when the second lands", () => {
    const effects = [effect(), effect({ effectId: "e2", field: "status", desired: "ready", priorValue: "triage" })];
    const half = reconcileWorkAction(
      pendingEnvelope(),
      effects,
      [
        read(),
        read({
          readId: "r2",
          effectId: "e2",
          field: "status",
          value: "triage",
          revision: { kind: "work_version", value: "7" },
        }),
      ],
      opts(),
    );
    expect(half.envelope.state).toBe("partial");
    expect(half.effects.map((e) => e.result)).toEqual(["verified", "not_yet"]);
    const full = reconcileWorkAction(
      half.envelope,
      effects,
      [
        read(),
        read({
          readId: "r3",
          effectId: "e2",
          field: "status",
          value: "ready",
          revision: { kind: "work_version", value: "9" },
        }),
      ],
      opts(),
    );
    expect(full.envelope.state).toBe("verified_complete");
    expectDeclaredEdges(full.envelope);
  });

  test("a passed sync deadline with nothing verified is uncertain", () => {
    const result = reconcileWorkAction(
      pendingEnvelope(),
      [effect()],
      [read({ value: 50, revision: { kind: "work_version", value: "7" } })],
      opts({ now: "2026-11-25T12:05:00Z" }),
    );
    expect(result.envelope.state).toBe("uncertain");
    expect(result.reason).toBe("sync_deadline_passed");
  });

  test("a source with no deadline never times out", () => {
    const result = reconcileWorkAction(
      pendingEnvelope(),
      [effect()],
      [],
      opts({ now: "2026-12-25T12:00:00Z", syncDeadlineMs: {} }),
    );
    expect(result.envelope.state).toBe("pending_sync");
  });

  test("a read matching more than one object is ambiguous and uncertain", () => {
    const result = reconcileWorkAction(pendingEnvelope(), [effect()], [read({ matchedObjects: 2 })], opts());
    expect(result.effects[0]?.result).toBe("ambiguous");
    expect(result.envelope.state).toBe("uncertain");
    expect(result.reason).toBe("ambiguous_resource");
  });

  test("a read of another object is ambiguous, never verifying", () => {
    const result = reconcileWorkAction(pendingEnvelope(), [effect()], [read({ objectRef: "orch:other" })], opts());
    expect(result.effects[0]?.result).toBe("ambiguous");
    expect(result.envelope.state).toBe("uncertain");
  });

  test("an unknown revision kind is uncertain", () => {
    const result = reconcileWorkAction(
      pendingEnvelope(),
      [effect()],
      [read({ revision: { kind: "lamport", value: "3" } })],
      opts(),
    );
    expect(result.effects[0]?.result).toBe("unknown_version");
    expect(result.envelope.state).toBe("uncertain");
    expect(result.reason).toBe("unknown_version");
  });

  test("running moves only along declared edges", () => {
    const waiting = reconcileWorkAction(runningEnvelope(), [effect()], [], opts());
    expect(waiting.envelope.state).toBe("pending_sync");
    expect(waiting.reason).toBe("awaiting_readback");
    expectDeclaredEdges(waiting.envelope);
    const done = reconcileWorkAction(runningEnvelope(), [effect()], [read()], opts());
    expect(done.envelope.state).toBe("verified_complete");
    expectDeclaredEdges(done.envelope);
  });

  test("a verified_complete with no readback behind it is flagged and shown as pending sync", () => {
    const { fx, envelope } = acceptedEnvelope();
    const selfReported = applyWorkAction(envelope, actor, fx.authority, { outcome: "verified_complete" });
    const result = reconcileWorkAction(selfReported, [effect()], [], opts());
    expect(result.envelope).toBe(selfReported); // terminal: never moved
    expect(result.unbackedVerified).toBe(true);
    expect(result.displayState).toBe("pending_sync");
    expect(result.reason).toBe("unbacked_verified");
    const backed = reconcileWorkAction(selfReported, [effect()], [read()], opts());
    expect(backed.unbackedVerified).toBe(false);
    expect(backed.displayState).toBe("verified_complete");
  });

  test("terminal states other than verified_complete are not reconciled", () => {
    const { envelope } = acceptedEnvelope();
    expect(() => reconcileWorkAction(envelope, [effect()], [], opts())).toThrow(/cannot reconcile/);
    expect(() => reconcileWorkAction(pendingEnvelope(), [], [], opts())).toThrow(/at least one/);
  });
});

describe("MOB-09 rollback switch (oracle: effect sink call count)", () => {
  test("mutations off refuses apply before any effect", () => {
    const { fx, envelope } = acceptedEnvelope();
    const gated = applyIfMutationsEnabled(false, envelope, actor, fx.authority, { outcome: "pending_sync" });
    expect(gated.applied).toBe(false);
    if (!gated.applied) expect(gated.refused).toBe("mutations_disabled");
    expect(fx.effects).toHaveLength(0);
  });

  test("mutations off keeps the envelope accepted and replayable once back on", () => {
    const { fx, envelope } = acceptedEnvelope();
    const off = applyIfMutationsEnabled(false, envelope, actor, fx.authority, { outcome: "pending_sync" });
    expect(off.envelope).toBe(envelope);
    expect(off.envelope.state).toBe("accepted");
    expect(off.envelope.effectIssued).toBe(false);
    const on = applyIfMutationsEnabled(true, off.envelope, actor, fx.authority, { outcome: "pending_sync" });
    expect(on.applied).toBe(true);
    expect(on.envelope.state).toBe("pending_sync");
    expect(fx.effects).toHaveLength(1);
  });
});
