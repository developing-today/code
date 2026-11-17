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
//
// Ordering (task 20118): the incident issue is created BEFORE the
// queue task is enqueued. Fleet queue validation (repo-task/v1) only
// accepts a small_fix packet whose dedupe key is the fix family —
// `repo-health:v1:fix:<repo>:issue:<n>` — with payload.issue.number
// matching the key, so the task cannot be built until the issue
// exists. applyEscalationActions stamps the confirmed issue number
// into the enqueue packet, and buildQueueTaskWrapper is the pure
// translation from an escalation packet to the validated envelope
// (the pre-fix production wrapper failed validation on 13 fields and
// the live hand-off created neither artifact).
import type { EscalationRecord, FailureClass, SourceHealth } from "./watchdog-state.ts";

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
  /** Repo that owns the adapter: the incident issue's repo, and the
   *  queue task's payload.repo / dedupe-key repo. */
  issueRepo: string;
  /** Incident issue number once the issue exists. Null in a fresh
   *  plan (the issue is created first; applyEscalationActions stamps
   *  the confirmed number in), and carried from the escalation record
   *  on a partial-hand-off resume. */
  issueNumber: number | null;
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

/** Every hand-off record this incident carries, latest first — the
 *  dedupe view planEscalation and buildEscalationPacket share. */
function escalationRecords(health: SourceHealth): EscalationRecord[] {
  const existing = health.incident?.escalation;
  return existing ? [existing, ...(existing.previous ?? [])] : [];
}

/** Build the queue-task packet for an incident that is escalating. */
export function buildEscalationPacket(health: SourceHealth, ctx: EscalationContext): EscalationPacket {
  const cls = health.currentClass;
  return {
    schema: "watchdog-escalation/v1",
    kind: "small_fix",
    source: health.source,
    issueRepo: ctx.issueRepo,
    issueNumber: escalationRecords(health).find((r) => r.class === cls)?.issueNumber ?? null,
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
    requiredCapability: "browser-enabled agent; laptop Helium via CDP (:19222); adapter origins only",
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
  const packet = buildEscalationPacket(health, ctx);
  const openIssue: EscalationAction = {
    type: "open_issue",
    repo: ctx.issueRepo,
    title: issueTitle(health),
    body: issueBody(health, packet),
    labels: ["watchdog-incident"],
  };
  // Dedupe consults every hand-off this incident carries, not just
  // the latest (Codex P2 on PR #8357): a failure class that changes
  // back finds its original pair in `previous` instead of creating a
  // third hand-off.
  const records = escalationRecords(health);
  const forClass = records.find((r) => r.class === health.currentClass);
  if (forClass) {
    // Resume a partial hand-off: plan only the creations that never
    // confirmed, so a retry cannot duplicate the confirmed half. The
    // issue still plans before the task (the task packet names it).
    const actions: EscalationAction[] = [];
    if (forClass.issueNumber === null) actions.push(openIssue);
    if (forClass.queueTaskId === null) actions.push({ type: "enqueue_task", packet });
    return { actions, deduped: actions.length === 0 };
  }
  return {
    deduped: false,
    actions: [openIssue, { type: "enqueue_task", packet }],
  };
}

/** The repo-task/v1 envelope the fleet queue validates and enqueues.
 *  Pure translation of an escalation packet: the production
 *  enqueueTask effector writes this to a file and hands it to
 *  `fleet queue enqueue`. The full escalation packet rides inside the
 *  payload, so the claimed task still starts from the packet alone
 *  (#6033 acceptance: the queue task packet matches the context list). */
export interface QueueTaskWrapper {
  title: string;
  body: string;
  dedupe_key: string;
  priority: number;
  payload: Record<string, unknown>;
}

export function buildQueueTaskWrapper(packet: EscalationPacket, at: string): QueueTaskWrapper {
  if (packet.issueNumber === null) {
    throw new Error(
      `escalation packet for ${packet.source} carries no incident issue number — the issue must be created before the queue task can name it`,
    );
  }
  const repo = packet.issueRepo;
  const n = packet.issueNumber;
  const cls = packet.failure.class ?? "unknown";
  const repairLines = packet.failure.repairLog.map((e) => `rung ${e.rung} ${e.outcome} at ${e.at}: ${e.detail}`);
  return {
    title: `watchdog incident: ${packet.source} (${cls}) — repair the quota producer`,
    body: [
      packet.outcomeWanted,
      "",
      `Incident issue: ${repo}#${n} (the watchdog auto-closes it on the next producer success — do not close it by hand).`,
      `Required capability: ${packet.requiredCapability}`,
      `Acceptance: ${packet.acceptance}`,
    ].join("\n"),
    dedupe_key: `repo-health:v1:fix:${repo}:issue:${n}`,
    priority: 30,
    payload: {
      schema: "repo-task/v1",
      kind: "small_fix",
      repo,
      issue: { number: n },
      problem:
        `Source \`${packet.source}\` quota producer is failing (${cls}) and every watchdog ` +
        `repair rung is spent, so the watchdog escalated (watchdog dedupe ${packet.dedupeKey}). ` +
        `Last detail: ${packet.failure.lastDetail || "(none)"}. The full context packet is in ` +
        `payload.escalation_packet; the incident issue ${repo}#${n} is the reporting surface.`,
      priority_reason:
        "Quota watchdog rung-4 escalation (#6033): a dead quota producer blinds the fleet's " +
        "provider-quota reads until a browser-enabled agent repairs it; the watchdog has " +
        "already exhausted its automated repair ladder.",
      evidence: [
        `consecutive failures: ${packet.failure.consecutiveFailures}`,
        ...(repairLines.length > 0 ? repairLines : ["repair log empty — ladder spent without entries"]),
        `watchdog state file: ${packet.failure.stateFilePath}`,
      ],
      acceptance: [
        packet.acceptance,
        `incident issue ${repo}#${n} is auto-commented and auto-closed by the watchdog on producer success (the fixer closes nothing by hand)`,
      ],
      tests: [
        "the producer for this source writes a fresh latest.json with status ok and real quota windows",
        "the watchdog state file shows the incident cleared after the recovery tick",
      ],
      context_refs: [
        packet.failure.stateFilePath,
        packet.failure.historyRef,
        packet.failure.adapterRef,
        `${repo}#${n}`,
        "developing-today/code#6033",
      ],
      scope: {
        include: [
          `restore fresh quota documents for source ${packet.source} via its adapter's browser flow (adapter: ${packet.failure.adapterRef})`,
          "report progress on the incident issue",
        ],
        exclude: [
          ...packet.stopConditions,
          "rebuilding the watchdog core or the escalation module — report on the incident issue instead if the ladder itself is broken",
        ],
      },
      risk: { level: "low", flags: [] },
      timebox_minutes: 45,
      dedupe: {
        checked_at: at,
        basis:
          `watchdog escalation record for (${packet.source}, ${cls}): the incident issue ` +
          `${repo}#${n} was created by this same hand-off, and the fix-family dedupe key ` +
          "names it, so a second live task for the incident cannot be enqueued.",
      },
      escalation_packet: packet,
    },
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
  const cls = health.currentClass ?? "unknown";
  const prev = health.incident.escalation;
  let escalation: EscalationRecord;
  if (prev && prev.class === cls) {
    // Same pair: merge — a confirmed ref is never overwritten by a
    // later partial result, and the original `since` stands.
    escalation = {
      ...prev,
      queueTaskId: prev.queueTaskId ?? refs.queueTaskId,
      issueNumber: prev.issueNumber ?? refs.issueNumber,
    };
  } else if (prev) {
    // Class changed: the displaced hand-off stays on the record
    // (flattened into `previous`) so recovery can close and complete
    // it and dedupe can find it again (Codex P2 on PR #8357).
    const { previous: prevPrevious, ...prevFlat } = prev;
    escalation = {
      class: cls as EscalationRecord["class"],
      since: at,
      queueTaskId: refs.queueTaskId,
      issueRepo: ctx.issueRepo,
      issueNumber: refs.issueNumber,
      previous: [...(prevPrevious ?? []), prevFlat],
    };
  } else {
    escalation = {
      class: cls as EscalationRecord["class"],
      since: at,
      queueTaskId: refs.queueTaskId,
      issueRepo: ctx.issueRepo,
      issueNumber: refs.issueNumber,
    };
  }
  return {
    ...health,
    incident: { ...health.incident, escalation },
  };
}

/**
 * The NEEDS DREW line for this source, or null while it should stay
 * quiet. Human-blocked surfaces the same tick it is marked; an ordinary
 * escalation surfaces only once unresolved for the threshold.
 */
// Mirrors the human-only challenge pattern recordRungOutcome in
// watchdog-state.ts uses to mark an incident human-blocked.
const HUMAN_CHALLENGE_RE = /human[- ]only|2fa|passkey|verify[- ]it/i;

export function surfacingLine(health: SourceHealth, now: number): string | null {
  const incident = health.incident;
  if (!incident) return null;
  if (incident.humanBlocked || incident.surfacing === "human-blocked") {
    // Surface the persisted challenge reason (Codex P2 on PR #8357):
    // the repair-log entry that tripped the human-blocked mark names
    // the intervention actually needed. health.lastDetail is the
    // latest PRODUCER observation and can name a different, earlier
    // symptom — and later blocked ticks can overwrite it again.
    const challenge = [...health.repairLog]
      .reverse()
      .find((e) => e.outcome === "failed" && HUMAN_CHALLENGE_RE.test(e.detail));
    const reason = challenge ? challenge.detail : health.lastDetail || "human-only challenge";
    return `NEEDS DREW: ${health.source} quota producer is human-blocked (${health.currentClass ?? "unknown"}: ${reason}) — incident ${incident.id}`;
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
  // Close and complete EVERY hand-off the incident carries (Codex P2
  // on PR #8357): when the failure class changed mid-incident, the
  // earlier pair in `previous` is just as open as the latest one.
  const records: EscalationRecord[] = [esc, ...(esc.previous ?? [])];
  const actions: EscalationAction[] = [];
  for (const rec of records) {
    if (rec.issueNumber !== null) {
      actions.push({
        type: "comment_issue",
        repo: rec.issueRepo,
        issueNumber: rec.issueNumber,
        body: `Recovered: source \`${health.source}\` produced a fresh document at ${recoveredAt}; the watchdog is auto-closing this incident (#6033).`,
      });
      actions.push({ type: "close_issue", repo: rec.issueRepo, issueNumber: rec.issueNumber });
    }
    if (rec.queueTaskId !== null) {
      actions.push({ type: "complete_task", queueTaskId: rec.queueTaskId });
    }
  }
  return actions;
}

/** A planned action failed after earlier actions in the same plan
 *  had already confirmed effects (Codex P2 on PR #8357). The
 *  confirmed refs ride on the error so the caller can persist them
 *  via recordEscalation before retrying — without them, the retry
 *  plan would re-create the confirmed half and duplicate repair
 *  work, and recovery could never complete the orphaned task. */
export class EscalationPartialError extends Error {
  readonly refs: { queueTaskId: string | null; issueNumber: number | null };
  constructor(cause: unknown, refs: { queueTaskId: string | null; issueNumber: number | null }) {
    super(
      `escalation action failed after partial progress (queueTaskId=${refs.queueTaskId}, issueNumber=${refs.issueNumber}): ${cause instanceof Error ? cause.message : String(cause)}`,
    );
    this.name = "EscalationPartialError";
    this.refs = refs;
    this.cause = cause;
  }
}

/** Run planned actions through the effectors, in order, collecting the
 *  refs recordEscalation needs. On failure the thrown
 *  EscalationPartialError carries the refs confirmed so far. */
export async function applyEscalationActions(
  actions: EscalationAction[],
  effectors: EscalationEffectors,
): Promise<{ queueTaskId: string | null; issueNumber: number | null }> {
  let queueTaskId: string | null = null;
  let issueNumber: number | null = null;
  for (const action of actions) {
    try {
      switch (action.type) {
        case "enqueue_task": {
          // The queue task names the incident issue (fix-family
          // dedupe): stamp the number this same run just confirmed
          // into a packet planned before the issue existed. A resume
          // packet already carries its recorded number.
          const pkt =
            action.packet.issueNumber === null && issueNumber !== null
              ? { ...action.packet, issueNumber }
              : action.packet;
          queueTaskId = await effectors.enqueueTask(pkt);
          break;
        }
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
    } catch (err) {
      throw new EscalationPartialError(err, { queueTaskId, issueNumber });
    }
  }
  return { queueTaskId, issueNumber };
}
