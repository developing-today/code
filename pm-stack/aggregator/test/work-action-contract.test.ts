// MOB-05 contract tests (#9472). Model-based: the state table below IS the
// design's lifecycle, and each test names the oracle that decides its edge.
//
// Oracles used here:
// - the declared edge table (WORK_ACTION_EDGES) — a transition the design does
//   not name must be impossible, and every state the design names must be
//   reachable through the declared chain;
// - the expected-work-version fixture (an authoritative version per object)
//   — stale or unreadable versions never reach an effect;
// - the effect sink's call count — one effect per operation key, proven by
//   replaying apply on the same envelope.
import { describe, expect, test } from "bun:test";
import {
  WORK_ACTION_EDGES,
  WORK_ACTION_STATES,
  acceptWorkAction,
  applyWorkAction,
  canTransition,
  createWorkAction,
  isTerminalState,
  previewWorkAction,
  type WorkActionActor,
  type WorkActionAuthority,
  type WorkActionEnvelope,
  type WorkActionRequest,
  type WorkActionState,
} from "../src/work-action-contract.ts";

const ALL_ROLES = [
  "work-action/preview",
  "work-action/accept",
  "work-action/apply",
] as const;

const actor = (roles: readonly string[] = ALL_ROLES): WorkActionActor => ({
  kind: "drew",
  id: "drew",
  roles,
});

/** The authoritative fixture: one work version per object, mutable so a test
 *  can move the work between preview and apply. */
function fixture(objectRef: string, version: string | null) {
  const versions = new Map<string, string | null>([[objectRef, version]]);
  const effects: WorkActionRequest[] = [];
  const rolesByActor = new Map<string, readonly string[]>([
    ["drew", ALL_ROLES],
  ]);
  const authority: WorkActionAuthority = {
    readWorkVersion: (ref) => versions.get(ref) ?? null,
    readRoles: (id) => rolesByActor.get(id) ?? [],
    issueEffect: (request) => effects.push(request),
  };
  return {
    authority,
    effects,
    setVersion: (v: string | null) => versions.set(objectRef, v),
    setRoles: (id: string, roles: readonly string[]) =>
      rolesByActor.set(id, roles),
  };
}

const request = (over: Partial<WorkActionRequest> = {}): WorkActionRequest => ({
  operationKey: "op-9472-0001",
  kind: "triage",
  objectRef: "orch:item-9472",
  action: "reprioritize",
  payload: { priority: 30 },
  expectedWorkVersion: "wv-7",
  ...over,
});

/** draft → previewed → accepted → running → verified_complete, the happy chain. */
function accepted(over: Partial<WorkActionRequest> = {}) {
  const fx = fixture(
    request(over).objectRef,
    request(over).expectedWorkVersion,
  );
  const envelope = acceptWorkAction(
    previewWorkAction(createWorkAction(request(over)), actor(), fx.authority),
    actor(),
    fx.authority,
  );
  return { fx, envelope };
}

describe("work action contract — state machine (oracle: declared edge table)", () => {
  test("every design state is declared and terminal states have no outgoing edge", () => {
    const expectedStates: WorkActionState[] = [
      "draft",
      "previewed",
      "accepted",
      "running",
      "pending_sync",
      "partial",
      "conflict",
      "uncertain",
      "rejected",
      "verified_complete",
    ];
    expect([...WORK_ACTION_STATES].sort()).toEqual([...expectedStates].sort());
    for (const state of [
      "conflict",
      "uncertain",
      "rejected",
      "verified_complete",
    ] as WorkActionState[]) {
      expect(WORK_ACTION_EDGES[state]).toEqual([]);
      expect(isTerminalState(state)).toBe(true);
    }
  });

  test("the happy chain draft→previewed→accepted→running→verified_complete is the only path to complete", () => {
    const { fx, envelope } = accepted();
    const applied = applyWorkAction(envelope, actor(), fx.authority, {
      outcome: "verified_complete",
    });
    expect(applied.state).toBe("verified_complete");
    expect(applied.transitions.map((t) => `${t.from}->${t.to}`)).toEqual([
      "draft->previewed",
      "previewed->accepted",
      "accepted->running",
      "running->verified_complete",
    ]);
    // No shortcut: a draft cannot jump to complete.
    expect(canTransition("draft", "verified_complete")).toBe(false);
    expect(canTransition("previewed", "running")).toBe(false);
  });

  test("downstream outcomes land on their declared state and keep the receipt", () => {
    for (const outcome of [
      "partial",
      "pending_sync",
      "conflict",
      "uncertain",
    ] as const) {
      const { fx, envelope } = accepted({ operationKey: `op-${outcome}` });
      const applied = applyWorkAction(envelope, actor(), fx.authority, {
        outcome,
        note: outcome,
      });
      expect(applied.state).toBe(outcome);
      expect(applied.receipt?.outcome).toBe(outcome);
      expect(applied.receipt?.operationKey).toBe(`op-${outcome}`);
    }
  });

  test("illegal edges throw instead of silently advancing", () => {
    const { fx } = accepted({ operationKey: "op-illegal" });
    const draft = createWorkAction(request({ operationKey: "op-illegal" }));
    expect(() =>
      applyWorkAction(draft, actor(), fx.authority, {
        outcome: "verified_complete",
      }),
    ).toThrow(/illegal edge draft/);
    const complete = applyWorkAction(
      acceptWorkAction(
        previewWorkAction(
          createWorkAction(request({ operationKey: "op-illegal" })),
          actor(),
          fx.authority,
        ),
        actor(),
        fx.authority,
      ),
      actor(),
      fx.authority,
      { outcome: "verified_complete" },
    );
    expect(() => acceptWorkAction(complete, actor(), fx.authority)).toThrow(
      /illegal edge verified_complete/,
    );
  });
});

describe("work action contract — expected work version (oracle: authoritative fixture)", () => {
  test("work_action_stale_version_conflicts_without_effect", () => {
    const fx = fixture("orch:item-9472", "wv-9"); // the work moved on
    const envelope = createWorkAction(request({ expectedWorkVersion: "wv-7" }));
    const previewed = previewWorkAction(envelope, actor(), fx.authority);
    expect(previewed.state).toBe("conflict");
    expect(previewed.authorized).toBe(false);
    expect(previewed.receipt).toMatchObject({
      outcome: "conflict",
      operationKey: "op-9472-0001",
    });
    expect(fx.effects).toHaveLength(0); // no effect on a stale expectation
  });

  test("a version moved between preview and apply conflicts before the effect", () => {
    const fx = fixture("orch:item-9472", "wv-7");
    const previewed = previewWorkAction(
      createWorkAction(request()),
      actor(),
      fx.authority,
    );
    expect(previewed.state).toBe("previewed");
    fx.setVersion("wv-8"); // someone else applied first
    const moved = acceptWorkAction(previewed, actor(), fx.authority);
    expect(moved.state).toBe("conflict");
    expect(moved.effectIssued).toBe(false);
    expect(fx.effects).toHaveLength(0);
  });

  test("an unreadable authoritative version is uncertain, never a silent apply", () => {
    const fx = fixture("orch:item-9472", null);
    const previewed = previewWorkAction(
      createWorkAction(request()),
      actor(),
      fx.authority,
    );
    expect(previewed.state).toBe("uncertain");
    expect(previewed.receipt?.outcome).toBe("uncertain");
    expect(fx.effects).toHaveLength(0);
  });
});

describe("work action contract — idempotency (oracle: effect sink call count)", () => {
  test("replaying apply on the same envelope issues one effect and returns the same receipt", () => {
    const { fx, envelope } = accepted();
    const first = applyWorkAction(envelope, actor(), fx.authority, {
      outcome: "verified_complete",
    });
    const replay = applyWorkAction(first, actor(), fx.authority, {
      outcome: "verified_complete",
    });
    expect(fx.effects).toHaveLength(1);
    expect(replay.receipt).toEqual(first.receipt);
    expect(replay.transitions.length).toBe(first.transitions.length);
    expect(replay.state).toBe("verified_complete");
  });

  test("the effect carries the operation key exactly once per applied envelope", () => {
    const { fx, envelope } = accepted();
    const applied = applyWorkAction(envelope, actor(), fx.authority, {
      outcome: "verified_complete",
    });
    expect(applied.effectIssued).toBe(true);
    expect(fx.effects).toHaveLength(1);
    expect(fx.effects[0]?.operationKey).toBe("op-9472-0001");
  });
});

describe("work action contract — server-side authorization (oracle: re-read roles)", () => {
  test("an actor without the preview role is rejected before any read of the work", () => {
    const fx = fixture("orch:item-9472", "wv-7");
    const previewed = previewWorkAction(
      createWorkAction(request()),
      actor([]),
      fx.authority,
    );
    expect(previewed.state).toBe("rejected");
    expect(previewed.receipt?.outcome).toBe("rejected");
    expect(fx.effects).toHaveLength(0);
  });

  test("a role revoked between accept and apply fails before the consumer effect", () => {
    const { fx, envelope } = accepted();
    fx.setRoles("drew", ["work-action/preview", "work-action/accept"]); // apply revoked
    const applied = applyWorkAction(envelope, actor(), fx.authority, {
      outcome: "verified_complete",
    });
    expect(applied.state).toBe("rejected");
    expect(applied.receipt?.outcome).toBe("rejected");
    expect(fx.effects).toHaveLength(0);
  });

  test("a client claiming a role the server does not grant is refused at preview", () => {
    const fx = fixture("orch:item-9472", "wv-7");
    fx.setRoles("drew", []); // server says: no roles
    const previewed = previewWorkAction(
      createWorkAction(request()),
      actor(ALL_ROLES),
      fx.authority,
    );
    expect(previewed.state).toBe("rejected");
    expect(fx.effects).toHaveLength(0);
  });
});

describe("work action contract — structural validation", () => {
  test("an envelope needs an operation key and an expected version", () => {
    expect(() => createWorkAction(request({ operationKey: "  " }))).toThrow(
      /operation key is required/,
    );
    expect(() =>
      createWorkAction(request({ expectedWorkVersion: " " })),
    ).toThrow(/expected work version is required/);
  });

  test("a fresh envelope starts as a draft with no effect and no receipt", () => {
    const envelope: WorkActionEnvelope = createWorkAction(request());
    expect(envelope.state).toBe("draft");
    expect(envelope.effectIssued).toBe(false);
    expect(envelope.receipt).toBeNull();
    expect(envelope.authorized).toBe(false);
  });
});
