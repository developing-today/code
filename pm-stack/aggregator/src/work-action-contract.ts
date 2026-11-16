// The MOB-05 preview/apply action envelope (#9472, pm-stack design
// package `2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration`,
// component 9201 §3.2, slice MOB-05).
//
// Why this module exists: the dashboard's action store (src/actions.ts)
// is a REQUEST LEDGER. A record leaving state "new" proves a consumer
// picked the request up; it does not prove the fact was applied, and
// it has no notion of an expected work version, an operation key, or a
// server-side authorization decision made at apply time. MOB-05 asks
// for a contract with exactly those three properties:
//
//   1. operation key — one apply per key, replays return the original
//      receipt and produce no second effect;
//   2. expected work version — apply proceeds only when the expected
//      revision still matches the authoritative fixture; a stale
//      expectation ends in `conflict` (or `uncertain` when the
//      authoritative version cannot be read), never a silent apply;
//   3. server-side authorization — the actor is re-checked at apply
//      time, so a role revoked between preview and apply fails before
//      any downstream effect.
//
// The envelope also carries an explicit state machine, so every
// terminal outcome the design names (draft, previewed, accepted,
// running, pending_sync, partial, conflict, uncertain, rejected,
// verified_complete) is reachable only through a declared edge and a
// named oracle can assert each one.
//
// Boundaries: pure module, no I/O, no clock, no downstream client. The
// caller supplies the authoritative fixture reader, the authorization
// check and the effect sink. Production adoption, the Fleet-side
// operations seam and live acceptance stay with their own slices; this
// file is the executable contract the design asks for.

/** The design's envelope lifecycle. `verified_complete` and `rejected` are terminal. */
export const WORK_ACTION_STATES = [
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
] as const;

export type WorkActionState = (typeof WORK_ACTION_STATES)[number];

/** Declared edges. Anything absent here is a programming error, not a runtime outcome. */
export const WORK_ACTION_EDGES: Readonly<
  Record<WorkActionState, readonly WorkActionState[]>
> = {
  draft: ["previewed", "rejected"],
  previewed: ["accepted", "rejected", "conflict"],
  accepted: ["running", "rejected"],
  running: [
    "pending_sync",
    "partial",
    "conflict",
    "uncertain",
    "rejected",
    "verified_complete",
  ],
  pending_sync: ["verified_complete", "partial", "uncertain"],
  partial: ["verified_complete", "conflict"],
  conflict: [],
  uncertain: [],
  rejected: [],
  verified_complete: [],
};

export function isTerminalState(state: WorkActionState): boolean {
  return WORK_ACTION_EDGES[state].length === 0;
}

export function canTransition(
  from: WorkActionState,
  to: WorkActionState,
): boolean {
  return WORK_ACTION_EDGES[from].includes(to);
}

export interface WorkActionActor {
  kind: string;
  id: string;
  /** Roles the SERVER believes this actor holds right now. Never client-supplied. */
  roles: readonly string[];
}

/** Everything preview needs that the client asserts. The server re-derives the rest. */
export interface WorkActionRequest {
  /** Stable key: one apply per key. A replay returns the first receipt. */
  operationKey: string;
  kind: string;
  objectRef: string;
  action: string;
  payload: Readonly<Record<string, unknown>>;
  /** The work version the client believes it is acting on. */
  expectedWorkVersion: string;
}

export interface WorkActionEnvelope {
  state: WorkActionState;
  request: WorkActionRequest;
  /** Server-computed authorization verdict recorded at preview. */
  authorized: boolean;
  /** The authoritative version observed when the preview was accepted. */
  previewedVersion: string | null;
  /** Populated once the envelope reaches a terminal state, except `partial`/`pending_sync`. */
  receipt: WorkActionReceipt | null;
  /** Ordered audit trail: one entry per accepted edge, plus rejections. */
  transitions: readonly WorkActionTransition[];
  /** Whether an effect has been handed to the downstream. At most once. */
  effectIssued: boolean;
}

export interface WorkActionTransition {
  from: WorkActionState;
  to: WorkActionState;
  /** The oracle that decided the edge, recorded so a review can read the reasoning. */
  oracle: string;
}

export interface WorkActionReceipt {
  operationKey: string;
  outcome:
    | "verified_complete"
    | "rejected"
    | "conflict"
    | "uncertain"
    | "partial"
    | "pending_sync";
  workVersion: string | null;
  note: string;
}

/** Server-side collaborators. Injected so the contract has no ambient authority. */
export interface WorkActionAuthority {
  /** Current authoritative work version for the object, or null when unreadable. */
  readWorkVersion(objectRef: string): string | null;
  /** Current roles for the actor, re-read at each gate. */
  readRoles(actorId: string): readonly string[];
  /** The downstream effect. Called at most once per envelope. */
  issueEffect(request: WorkActionRequest): void;
}

const ROLES_BY_STATE: Partial<Record<WorkActionState, string>> = {
  previewed: "work-action/preview",
  accepted: "work-action/accept",
  running: "work-action/apply",
};

/**
 * A rejected edge keeps the audit trail and produces a terminal envelope;
 * the refusal itself is the receipt, so a caller can never mistake a
 * refusal for an unapplied draft.
 */
function refuse(
  envelope: WorkActionEnvelope,
  to: "rejected" | "conflict" | "uncertain",
  oracle: string,
  note: string,
): WorkActionEnvelope {
  const transition: WorkActionTransition = { from: envelope.state, to, oracle };
  return {
    ...envelope,
    state: to,
    authorized: false,
    receipt: {
      operationKey: envelope.request.operationKey,
      outcome: to,
      workVersion: envelope.previewedVersion,
      note,
    },
    transitions: [...envelope.transitions, transition],
  };
}

function invalid(envelope: WorkActionEnvelope, reason: string): never {
  throw new Error(
    `work-action-contract: illegal edge ${envelope.state} — ${reason} (operation ${envelope.request.operationKey})`,
  );
}

/** Create the draft envelope. Validation is structural only; authorization is preview's job. */
export function createWorkAction(
  request: WorkActionRequest,
): WorkActionEnvelope {
  if (!request.operationKey.trim())
    throw new Error("work-action-contract: operation key is required");
  if (!request.expectedWorkVersion.trim()) {
    throw new Error("work-action-contract: expected work version is required");
  }
  return {
    state: "draft",
    request,
    authorized: false,
    previewedVersion: null,
    receipt: null,
    transitions: [],
    effectIssued: false,
  };
}

/**
 * draft → previewed, or → rejected.
 *
 * Oracles, in order: the actor holds the preview role; the expected work
 * version matches the authoritative fixture. An unreadable authoritative
 * version is `uncertain` (never a silent apply), a mismatch is `conflict`.
 */
export function previewWorkAction(
  envelope: WorkActionEnvelope,
  actor: WorkActionActor,
  authority: WorkActionAuthority,
): WorkActionEnvelope {
  if (envelope.state !== "draft")
    invalid(envelope, "preview requires a draft envelope");

  const roles = authority.readRoles(actor.id);
  if (
    !roles.includes("work-action/preview") ||
    !actor.roles.includes("work-action/preview")
  ) {
    return refuse(
      envelope,
      "rejected",
      "work-action/authorized-actor",
      "actor lacks the preview role",
    );
  }

  const version = authority.readWorkVersion(envelope.request.objectRef);
  if (version === null) {
    const transition: WorkActionTransition = {
      from: "draft",
      to: "uncertain",
      oracle: "work-action/expected-version-readable",
    };
    return {
      ...envelope,
      state: "uncertain",
      authorized: false,
      receipt: {
        operationKey: envelope.request.operationKey,
        outcome: "uncertain",
        workVersion: null,
        note: "authoritative work version is unreadable",
      },
      transitions: [...envelope.transitions, transition],
    };
  }
  if (version !== envelope.request.expectedWorkVersion) {
    return refuse(
      envelope,
      "conflict",
      "work-action/expected-version",
      `expected ${envelope.request.expectedWorkVersion}, authoritative ${version}`,
    );
  }

  return {
    ...envelope,
    state: "previewed",
    authorized: true,
    previewedVersion: version,
    transitions: [
      ...envelope.transitions,
      {
        from: "draft",
        to: "previewed",
        oracle: "work-action/expected-version",
      },
    ],
  };
}

/** previewed → accepted. The version and the actor's role are re-read here. */
export function acceptWorkAction(
  envelope: WorkActionEnvelope,
  actor: WorkActionActor,
  authority: WorkActionAuthority,
): WorkActionEnvelope {
  if (envelope.state !== "previewed")
    invalid(envelope, "accept requires a previewed envelope");

  const roles = authority.readRoles(actor.id);
  if (
    !roles.includes("work-action/accept") ||
    !actor.roles.includes("work-action/accept")
  ) {
    return refuse(
      envelope,
      "rejected",
      "work-action/authorized-actor",
      "actor lacks the accept role",
    );
  }
  const version = authority.readWorkVersion(envelope.request.objectRef);
  if (version === null) {
    const transition: WorkActionTransition = {
      from: "previewed",
      to: "uncertain",
      oracle: "work-action/expected-version-readable",
    };
    return {
      ...envelope,
      state: "uncertain",
      authorized: false,
      receipt: {
        operationKey: envelope.request.operationKey,
        outcome: "uncertain",
        workVersion: null,
        note: "authoritative work version became unreadable after preview",
      },
      transitions: [...envelope.transitions, transition],
    };
  }
  if (version !== envelope.previewedVersion) {
    return refuse(
      envelope,
      "conflict",
      "work-action/expected-version",
      `work moved from ${envelope.previewedVersion} to ${version} after preview`,
    );
  }
  return {
    ...envelope,
    state: "accepted",
    transitions: [
      ...envelope.transitions,
      {
        from: "previewed",
        to: "accepted",
        oracle: "work-action/authorized-actor",
      },
    ],
  };
}

/**
 * accepted → running, issuing the downstream effect exactly once, then to
 * the outcome the downstream reports.
 *
 * The replay guard is structural: an envelope that already issued its effect
 * is returned unchanged, carrying the original receipt. That is the
 * one-effect-per-idempotency-key oracle.
 */
export function applyWorkAction(
  envelope: WorkActionEnvelope,
  actor: WorkActionActor,
  authority: WorkActionAuthority,
  report: {
    outcome:
      | "verified_complete"
      | "partial"
      | "pending_sync"
      | "conflict"
      | "uncertain";
    note?: string;
  },
): WorkActionEnvelope {
  if (envelope.effectIssued) return envelope;
  if (envelope.state !== "accepted")
    invalid(envelope, "apply requires an accepted envelope");

  // Authorization is re-read at the apply gate: a role revoked between
  // accept and apply must fail before any consumer effect.
  const roles = authority.readRoles(actor.id);
  if (!roles.includes("work-action/apply")) {
    return refuse(
      envelope,
      "rejected",
      "work-action/authorized-actor",
      "actor lost the apply role before apply",
    );
  }

  authority.issueEffect(envelope.request);

  const running: WorkActionEnvelope = {
    ...envelope,
    state: "running",
    effectIssued: true,
    transitions: [
      ...envelope.transitions,
      { from: "accepted", to: "running", oracle: "work-action/effect-issued" },
    ],
  };

  const workVersion = authority.readWorkVersion(envelope.request.objectRef);
  const target = report.outcome;
  if (!canTransition("running", target))
    invalid({ ...running, state: "running" }, `downstream reported ${target}`);

  return {
    ...running,
    state: target,
    receipt: {
      operationKey: envelope.request.operationKey,
      outcome: target,
      workVersion,
      note: report.note ?? "",
    },
    transitions: [
      ...running.transitions,
      { from: "running", to: target, oracle: "work-action/downstream-outcome" },
    ],
  };
}

/**
 * The receipt oracle: for a finished envelope, the receipt names the same
 * operation key the request carried and no second effect was issued. Used by
 * the replay test and by callers that dedupe on the operation key.
 */
export function receiptFor(
  envelope: WorkActionEnvelope,
): WorkActionReceipt | null {
  return envelope.receipt;
}
