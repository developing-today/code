// Rung 4 of the quota watchdog (#6033, design #6031): the escalation
// hand-off. The decision layer (watchdog-state.ts) says "escalate" once
// every repair rung is spent; this module is what that word MEANS in
// effects, kept behind injected effectors so the policy is testable
// without a queue, a network, or a browser:
//
// - exactly one fleet queue task and one GitHub incident issue per
//   (source, failure class) while the incident is open — the dedupe
//   record lives on the incident itself (Incident.escalation), so a
//   second escalation tick for the same pair creates neither;
// - the queue task carries the full context packet (below), so a
//   browser-enabled agent can start from the packet alone;
// - on the next producer success, the recovery plan comments the
//   resolution on the incident issue, closes it, and completes the
//   queue task — the watchdog's own state clears the incident in the
//   same tick (decide's healthy path), nobody closes anything by hand;
// - surfacing follows the quiet-while-repairing rule: a human-blocked
//   incident earns its NEEDS DREW line the same tick it is marked; an
//   ordinary escalation stays quiet until it has been unresolved for
//   ESCALATION_NEEDS_DREW_AFTER_MS (24 h default), then earns its line
//   via the normal sweep path (surfacingLine is what a sweep renders).
//
// The production effectors (fleet queue enqueue via the fleet CLI, gh
// issue create/comment/close) are wired by the watchdog entry point —
// the #6032 remainder — which calls planEscalation / planRecovery with
// the tick's pre-decision health record and persists whatever
// recordEscalation returns.
import type { FailureClass, SourceHealth } from "./watchdog-state.ts";

/** How long an ordinary escalation stays quiet before it surfaces. */
export const ESCALATION_NEEDS_DREW_AFTER_MS = 24 * 60 * 60_000;

/** Per-source wiring the entry point supplies from the adapter. */
export interface EscalationContext {
  /** Repo that owns the adapter, e.g. "developing-today-forks/openchamber". */
  issueRepo: string;
  /** Path of the watchdog state file, for the packet's context. */
  stateFilePath: string;
  /** Adapter file reference, for the packet's context. */
  adapterRef: string;
  /** Producer history reference (history.jsonl tail), for the packet. */
  historyRef: string;
}

/** The full context packet carried by the rung-4 queue task (#6033). */
export interface EscalationPacket {
  schema: "watchdog-escalation/v1";
  kind: "small_fix";
  source: string;
  /** Dedupe pair: (source, failure class). */
  dedupeKey: string;
  outcomeWanted: string;
  failure: {
    class: FailureClass | null;
    consecutiveFailures: number;
    lastDetail: string;
    /** The watchdog's persisted repair log — every rung tried. */
    repairLog: SourceHealth["repairLog"];
    historyRef: string;
    stateFilePath: string;
    adapterRef: string;
  };
  requiredCapability: string;
  acceptance: string;
  stopConditions: string[];
}

export type EscalationAction =
  | { type: "enqueue_task"; packet: EscalationPacket }
  | { type: "open_issue"; repo: string; title: string; body: string; labels: string[] }
  | { type: "comment_issue"; repo: string; issueNumber: number; body: string }
  | { type: "close_issue"; repo: string; issueNumber: number }
  | { type: "complete_task"; queueTaskId: string };

/** Side effects, injected. Production implementations shell to the
 *  fleet CLI / gh; tests count calls. */
export interface EscalationEffectors {
  enqueueTask(packet: EscalationPacket): Promise<string>;
  openIssue(action: Extract<EscalationAction, { type: "open_issue" }>): Promise<number>;
  commentIssue(action: Extract<EscalationAction, { type: "comment_issue" }>): Promise<void>;
  closeIssue(action: Extract<EscalationAction, { type: "close_issue" }>): Promise<void>;
  completeTask(action: Extract<EscalationAction, { type: "complete_task" }>): Promise<void>;
}

export function escalationDedupeKey(source: string, cls: FailureClass | null): string {
  return `watchdog-escalation:${source}:${cls ?? "unknown"}`;
}

/** Build the queue-task packet for an incident that is escalating. */
export function buildEscalationPacket(health: SourceHealth, ctx: EscalationContext): EscalationPacket {
  const cls = health.currentClass;
  return {
    schema: "watchdog-escalation/v1",
    kind: "small_fix",
    source: health.source,
    dedupeKey: escalationDedupeKey(health.source, cls),
    outcomeWanted: `source \`${health.source}\` produces a fresh \`latest.json\` with real quota windows again`,
    failure: {
      class: cls,
      consecutiveFailures: health.consecutiveFailures,
      lastDetail: health.lastDetail,
      repairLog: health.repairLog,
      historyRef: ctx.historyRef,
      stateFilePath: ctx.stateFilePath,
      adapterRef: ctx.adapterRef,
    },
    requiredCapability:
      "browser-enabled agent; laptop Helium via CDP (:19222); adapter origins only",
    acceptance:
      "a fresh producer document lands and the watchdog auto-closes the incident (the fixer does not close anything by hand)",
    stopConditions: [
      "a human-only challenge (2FA / verify-it's-you / passkey) -> stop, mark human-blocked, do not work around it",
      "anything outside the adapter's origin allowlist -> stop and report on the incident issue",
    ],
  };
}

function issueTitle(health: SourceHealth): string {
  return `watchdog incident: ${health.source} quota producer failing (${health.currentClass ?? "unknown"})`;
}

function issueBody(health: SourceHealth, packet: EscalationPacket): string {
  const rungs = health.repairLog.map((e) => `- ${e.at} rung ${e.rung}: ${e.outcome} — ${e.detail}`).join("\n");
  return [
    `The quota watchdog escalated source \`${health.source}\` after every repair rung was spent (#6033, design #6031).`,
    "",
    `- failure class: \`${packet.failure.class ?? "unknown"}\``,
    `- consecutive failures: ${packet.failure.consecutiveFailures}`,
    `- last detail: ${packet.failure.lastDetail || "(none)"}`,
    `- incident id: \`${health.incident?.id ?? "?"}\``,
    "",
    "Repair log (every rung tried):",
    rungs || "- (empty)",
    "",
    `A fleet queue task carries the full context packet (dedupe key \`${packet.dedupeKey}\`). This issue is the durable record and the fixing agent's reporting surface; the watchdog auto-closes it on the next producer success.`,
  ].join("\n");
}

/**
 * Plan the rung-4 effects for a tick whose health record carries an
 * escalating incident. Pure: returns the actions to run, or an empty
 * list when the (source, class) pair is already escalated (dedupe) or
 * the incident is not in an escalating state.
 */
export function planEscalation(
  health: SourceHealth,
  ctx: EscalationContext,
): { actions: EscalationAction[]; deduped: boolean } {
  const incident = health.incident;
  if (!incident || incident.surfacing !== "escalate" || incident.humanBlocked) {
    return { actions: [], deduped: false };
  }
  const existing = incident.escalation;
  if (existing && existing.class === health.currentClass) {
    return { actions: [], deduped: true };
  }
  const packet = buildEscalationPacket(health, ctx);
  return {
    deduped: false,
    actions: [
      { type: "enqueue_task", packet },
      {
        type: "open_issue",
        repo: ctx.issueRepo,
        title: issueTitle(health),
        body: issueBody(health, packet),
        labels: ["watchdog-incident"],
      },
    ],
  };
}

/** Persist the confirmed effect results onto the incident record. */
export function recordEscalation(
  health: SourceHealth,
  ctx: EscalationContext,
  refs: { queueTaskId: string | null; issueNumber: number | null },
  at: string,
): SourceHealth {
  if (!health.incident) return health;
  return {
    ...health,
    incident: {
      ...health.incident,
      escalation: {
        class: health.currentClass ?? "unknown",
        since: at,
        queueTaskId: refs.queueTaskId,
        issueRepo: ctx.issueRepo,
        issueNumber: refs.issueNumber,
      },
    },
  };
}

/**
 * The NEEDS DREW line for this source, or null while it should stay
 * quiet. Human-blocked surfaces the same tick it is marked; an ordinary
 * escalation surfaces only once unresolved for the threshold.
 */
export function surfacingLine(health: SourceHealth, now: number): string | null {
  const incident = health.incident;
  if (!incident) return null;
  if (incident.humanBlocked || incident.surfacing === "human-blocked") {
    return `NEEDS DREW: ${health.source} quota producer is human-blocked (${health.currentClass ?? "unknown"}: ${health.lastDetail || "human-only challenge"}) — incident ${incident.id}`;
  }
  const esc = incident.escalation;
  if (esc) {
    const since = Date.parse(esc.since);
    if (Number.isFinite(since) && now - since >= ESCALATION_NEEDS_DREW_AFTER_MS) {
      return `NEEDS DREW: ${health.source} quota escalation unresolved for 24h (${esc.class}) — incident ${incident.id}, issue ${esc.issueRepo}#${esc.issueNumber ?? "?"}`;
    }
  }
  return null;
}

/**
 * Plan the recovery effects for a source whose incident was escalated
 * and whose latest observation is a producer success. Call with the
 * PRE-tick health record (decide's healthy path clears the incident in
 * the next record; the refs live on the old one). Pure.
 */
export function planRecovery(health: SourceHealth, recoveredAt: string): EscalationAction[] {
  const esc = health.incident?.escalation;
  if (!esc) return [];
  const actions: EscalationAction[] = [];
  if (esc.issueNumber !== null) {
    actions.push({
      type: "comment_issue",
      repo: esc.issueRepo,
      issueNumber: esc.issueNumber,
      body: `Recovered: source \`${health.source}\` produced a fresh document at ${recoveredAt}; the watchdog is auto-closing this incident (#6033).`,
    });
    actions.push({ type: "close_issue", repo: esc.issueRepo, issueNumber: esc.issueNumber });
  }
  if (esc.queueTaskId !== null) {
    actions.push({ type: "complete_task", queueTaskId: esc.queueTaskId });
  }
  return actions;
}

/** Run planned actions through the effectors, in order, collecting the
 *  refs recordEscalation needs. */
export async function applyEscalationActions(
  actions: EscalationAction[],
  effectors: EscalationEffectors,
): Promise<{ queueTaskId: string | null; issueNumber: number | null }> {
  let queueTaskId: string | null = null;
  let issueNumber: number | null = null;
  for (const action of actions) {
    switch (action.type) {
      case "enqueue_task":
        queueTaskId = await effectors.enqueueTask(action.packet);
        break;
      case "open_issue":
        issueNumber = await effectors.openIssue(action);
        break;
      case "comment_issue":
        await effectors.commentIssue(action);
        break;
      case "close_issue":
        await effectors.closeIssue(action);
        break;
      case "complete_task":
        await effectors.completeTask(action);
        break;
    }
  }
  return { queueTaskId, issueNumber };
}
