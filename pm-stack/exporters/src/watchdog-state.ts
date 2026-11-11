// Generic browser-dependent quota watchdog core — state machine (#6032,
// design #6031).
//
// This module is the DECISION layer only: pure functions that turn a
// per-source observation into the next persisted health record and name
// the repair rung to attempt. It performs no I/O, spawns no browser, and
// reads no secrets — the oneshot entry point (watchdog.ts) owns disk and
// side effects, and the adapter loader owns provider specifics. Keeping
// the decision layer pure is what makes the ladder testable with
// property- and model-based tests (testing doctrine, AGENTS.md).
//
// The lesson from code#5928 is structural here: every decision-relevant
// value (consecutive failure count, cooldown deadline, per-rung attempt
// counts, incident identity) lives in the record that is written to disk
// each tick. Nothing is cached in process memory, because the watchdog is
// a oneshot and the next tick is a fresh process.

/** Failure classes the watchdog can diagnose (#6031 health model). */
export type FailureClass =
  | "session-expired"
  | "cookie-missing"
  | "endpoint-changed"
  | "browser-down"
  | "cdp-unreachable"
  | "rate-limited"
  | "unknown";

export const FAILURE_CLASSES: readonly FailureClass[] = [
  "session-expired",
  "cookie-missing",
  "endpoint-changed",
  "browser-down",
  "cdp-unreachable",
  "rate-limited",
  "unknown",
];

/** Repair rungs, cheapest first. Rung 4 (escalation) is #6033's contract. */
export type Rung = 0 | 1 | 2 | 3;

export const RUNGS: readonly Rung[] = [0, 1, 2, 3];

/** Max attempts per rung per incident (design + acceptance criteria). */
export const MAX_ATTEMPTS_PER_RUNG = 2;

/** Repair log is bounded so the state file cannot grow without limit. */
export const REPAIR_LOG_LIMIT = 20;

/** One line of the persisted repair log. */
export interface RepairLogEntry {
  at: string;
  rung: Rung;
  outcome: "attempted" | "succeeded" | "failed" | "skipped";
  detail: string;
}

/** One source's health record. This is what is written to disk per tick. */
export interface SourceHealth {
  source: string;
  lastAttemptAt: string | null;
  lastSuccessAt: string | null;
  consecutiveFailures: number;
  currentClass: FailureClass | null;
  /** Sanitized — never a secret value, cookie, or token. */
  lastDetail: string;
  /** ISO deadline before which no browser action is taken (rate-limited). */
  cooldownUntil: string | null;
  /** Attempts spent per rung in the CURRENT incident. Reset on success. */
  rungAttempts: Record<Rung, number>;
  repairLog: RepairLogEntry[];
  /** Incident identity, or null when healthy. */
  incident: Incident | null;
}

export interface Incident {
  /** Stable id: `<source>:<firstFailureAt>` — dedupe key for the GitHub issue. */
  id: string;
  openedAt: string;
  /** Set when a human-only challenge stops the ladder dead. */
  humanBlocked: boolean;
  /** Surfacing flag the stewardship/orchestration sweeps read. */
  surfacing: "none" | "human-blocked" | "escalate";
  /** Rung-4 hand-off record (#6033), set once the escalation effects
   *  have run for this incident. Its (source, class) pair is the dedupe
   *  key: while it is present, no second queue task or incident issue
   *  is created for the same failure. Absent/null = not yet escalated. */
  escalation?: EscalationRecord | null;
}

/** What rung 4 created for one incident (#6033). */
export interface EscalationRecord {
  /** Failure class at escalation time — half of the dedupe pair. */
  class: FailureClass;
  /** When the escalation effects ran. */
  since: string;
  /** Fleet queue task id, or null if the enqueue has not confirmed. */
  queueTaskId: string | null;
  /** Repo owning the adapter (and therefore the incident issue). */
  issueRepo: string;
  /** Incident issue number, or null if creation has not confirmed. */
  issueNumber: number | null;
}

/** What one tick saw for one source. */
export type Observation =
  | { kind: "healthy"; at: string; detail?: string }
  | { kind: "failed"; at: string; class: FailureClass; detail?: string };

/** What the ladder says to do this tick. */
export type Decision =
  | { action: "quiet" }
  | { action: "wait-cooldown"; until: string }
  | { action: "retry" }
  | { action: "repair"; rung: Rung }
  | { action: "human-blocked" }
  | { action: "escalate" };

/** Fresh record for a source the watchdog has not seen before. */
export function initialHealth(source: string): SourceHealth {
  return {
    source,
    lastAttemptAt: null,
    lastSuccessAt: null,
    consecutiveFailures: 0,
    currentClass: null,
    lastDetail: "",
    cooldownUntil: null,
    rungAttempts: { 0: 0, 1: 0, 2: 0, 3: 0 },
    repairLog: [],
    incident: null,
  };
}

/** Strip anything that could carry a secret out of a free-text detail. */
export function sanitizeDetail(detail: string | undefined): string {
  if (!detail) return "";
  // Collapse whitespace and cap length; drop obvious credential shapes.
  const collapsed = detail.replace(/\s+/g, " ").trim();
  const redacted = collapsed
    .replace(/(cookie|authorization|token|bearer|session|password|secret)\s*[:=]\s*\S+/gi, "$1=<redacted>")
    .replace(/[A-Za-z0-9_-]{32,}/g, "<redacted>");
  return redacted.slice(0, 500);
}

function appendLog(log: RepairLogEntry[], entry: RepairLogEntry): RepairLogEntry[] {
  const next = [...log, entry];
  return next.length > REPAIR_LOG_LIMIT ? next.slice(next.length - REPAIR_LOG_LIMIT) : next;
}

function ms(iso: string): number {
  const t = Date.parse(iso);
  return Number.isFinite(t) ? t : NaN;
}

/**
 * The ONE place a failure class maps to its repair rung. Adapters may
 * override via their class map, but the core default is here and it
 * contains no provider names.
 */
export function defaultRungFor(cls: FailureClass): Rung {
  switch (cls) {
    case "rate-limited":
      return 0; // cooldown on disk; no browser action
    case "browser-down":
    case "cdp-unreachable":
      return 2; // re-validate the CDP session / bring the browser back
    case "session-expired":
    case "cookie-missing":
      return 3; // re-login via the standing SSO session
    case "endpoint-changed":
      return 1; // retry next tick; a changed endpoint is often transient
    case "unknown":
      return 1;
  }
}

function rungExhausted(h: SourceHealth, rung: Rung): boolean {
  return (h.rungAttempts[rung] ?? 0) >= MAX_ATTEMPTS_PER_RUNG;
}

/**
 * Decide what this tick should do, given the persisted record and the
 * fresh observation. Pure: same inputs always produce the same decision
 * and the same next record.
 */
export function decide(
  health: SourceHealth,
  obs: Observation,
  opts: { classMap?: Partial<Record<FailureClass, Rung>>; cooldownMs?: number } = {},
): { decision: Decision; next: SourceHealth } {
  const at = obs.at;
  const classMap = opts.classMap ?? {};
  const cooldownMs = opts.cooldownMs ?? 15 * 60_000;

  // --- healthy tick: quiet, clear the incident, reset per-rung counts ---
  if (obs.kind === "healthy") {
    const next: SourceHealth = {
      ...health,
      lastAttemptAt: at,
      lastSuccessAt: at,
      consecutiveFailures: 0,
      currentClass: null,
      lastDetail: sanitizeDetail(obs.detail),
      cooldownUntil: null,
      rungAttempts: { 0: 0, 1: 0, 2: 0, 3: 0 },
      repairLog:
        health.incident === null
          ? health.repairLog
          : appendLog(health.repairLog, {
              at,
              rung: 1,
              outcome: "succeeded",
              detail: "source recovered; incident closed",
            }),
      incident: null,
    };
    return { decision: { action: "quiet" }, next };
  }

  // --- failure tick ---
  const cls = obs.class;
  const consecutiveFailures = health.consecutiveFailures + 1;
  const incident: Incident =
    health.incident ?? {
      id: `${health.source}:${at}`,
      openedAt: at,
      humanBlocked: false,
      surfacing: "none",
    };

  // A human-blocked incident stays blocked until a human clears it or the
  // source recovers — the ladder must not hammer a login challenge.
  if (incident.humanBlocked) {
    const next: SourceHealth = {
      ...health,
      lastAttemptAt: at,
      consecutiveFailures,
      currentClass: cls,
      lastDetail: sanitizeDetail(obs.detail),
      incident,
    };
    return { decision: { action: "human-blocked" }, next };
  }

  // Rung 0: a persisted cooldown is honored without any browser action.
  if (health.cooldownUntil !== null) {
    const until = ms(health.cooldownUntil);
    const now = ms(at);
    if (Number.isFinite(until) && Number.isFinite(now) && now < until) {
      const next: SourceHealth = {
        ...health,
        lastAttemptAt: at,
        consecutiveFailures,
        currentClass: cls,
        lastDetail: sanitizeDetail(obs.detail),
        incident,
      };
      return { decision: { action: "wait-cooldown", until: health.cooldownUntil }, next };
    }
  }

  const desired = classMap[cls] ?? defaultRungFor(cls);

  // A rate limit whose cooldown rung is spent does not climb to browser
  // repairs — the provider is still refusing us; escalate instead.
  let rung: Rung | null = null;
  if (desired === 0 && rungExhausted(health, 0)) {
    rung = null;
  } else {
    for (let r = desired; r <= 3; r++) {
      if (!rungExhausted(health, r as Rung)) {
        rung = r as Rung;
        break;
      }
    }
  }

  if (rung === null) {
    const next: SourceHealth = {
      ...health,
      lastAttemptAt: at,
      consecutiveFailures,
      currentClass: cls,
      lastDetail: sanitizeDetail(obs.detail),
      incident: { ...incident, surfacing: "escalate" },
      repairLog: appendLog(health.repairLog, {
        at,
        rung: 3,
        outcome: "failed",
        detail: "all rungs exhausted; escalating",
      }),
    };
    return { decision: { action: "escalate" }, next };
  }

  // Rung 0 is the cooldown rung: set the deadline on disk and take no
  // browser action this tick.
  if (rung === 0) {
    const now = ms(at);
    const untilIso = Number.isFinite(now) ? new Date(now + cooldownMs).toISOString() : null;
    const next: SourceHealth = {
      ...health,
      lastAttemptAt: at,
      consecutiveFailures,
      currentClass: cls,
      lastDetail: sanitizeDetail(obs.detail),
      cooldownUntil: untilIso,
      rungAttempts: { ...health.rungAttempts, 0: (health.rungAttempts[0] ?? 0) + 1 },
      incident,
      repairLog: appendLog(health.repairLog, {
        at,
        rung: 0,
        outcome: "attempted",
        detail: `rate-limited; cooldown until ${untilIso ?? "unknown"}`,
      }),
    };
    return { decision: { action: "wait-cooldown", until: untilIso ?? "" }, next };
  }

  const next: SourceHealth = {
    ...health,
    lastAttemptAt: at,
    consecutiveFailures,
    currentClass: cls,
    lastDetail: sanitizeDetail(obs.detail),
    cooldownUntil: null,
    rungAttempts: { ...health.rungAttempts, [rung]: (health.rungAttempts[rung] ?? 0) + 1 },
    incident,
    repairLog: appendLog(health.repairLog, {
      at,
      rung,
      outcome: "attempted",
      detail: `${cls}; attempting rung ${rung}`,
    }),
  };
  return { decision: { action: rung === 1 ? "retry" : "repair", rung }, next };
}

/**
 * Record the outcome of the rung the previous tick chose. Called by the
 * entry point after it actually ran (or refused) the repair, so the log
 * reflects reality rather than intent.
 */
export function recordRungOutcome(
  health: SourceHealth,
  at: string,
  rung: Rung,
  outcome: "succeeded" | "failed" | "skipped",
  detail: string,
): SourceHealth {
  const incident = health.incident;
  // A human-only challenge stops the ladder dead and surfaces immediately.
  const humanBlocked = outcome === "failed" && /human[- ]only|2fa|passkey|verify[- ]it/i.test(detail);
  return {
    ...health,
    incident:
      incident === null
        ? null
        : humanBlocked
          ? { ...incident, humanBlocked: true, surfacing: "human-blocked" }
          : incident,
    repairLog: appendLog(health.repairLog, {
      at,
      rung,
      outcome,
      detail: sanitizeDetail(detail),
    }),
  };
}

/** The whole state file: one record per source. */
export interface WatchdogState {
  version: 1;
  updatedAt: string;
  sources: Record<string, SourceHealth>;
}

export function emptyState(now: string): WatchdogState {
  return { version: 1, updatedAt: now, sources: {} };
}

/**
 * Structural validation of a state file read back from disk. A corrupt
 * or older file must not crash the tick: unknown shapes yield a fresh
 * record for that source rather than a thrown error (fail open to
 * "re-observe", never to "assume healthy").
 */
export function coerceHealth(source: string, raw: unknown): SourceHealth {
  const base = initialHealth(source);
  if (typeof raw !== "object" || raw === null || Array.isArray(raw)) return base;
  const r = raw as Record<string, unknown>;
  const num = (v: unknown, d: number) => (typeof v === "number" && Number.isFinite(v) && v >= 0 ? v : d);
  const str = (v: unknown) => (typeof v === "string" ? v : null);
  const cls = FAILURE_CLASSES.includes(r.currentClass as FailureClass) ? (r.currentClass as FailureClass) : null;
  const attemptsRaw = (r.rungAttempts ?? {}) as Record<string, unknown>;
  const attempts = { 0: 0, 1: 0, 2: 0, 3: 0 } as Record<Rung, number>;
  for (const rung of RUNGS) attempts[rung] = num(attemptsRaw[String(rung)], 0);
  const logRaw = Array.isArray(r.repairLog) ? r.repairLog : [];
  const repairLog: RepairLogEntry[] = logRaw
    .filter((e): e is Record<string, unknown> => typeof e === "object" && e !== null)
    .map((e) => ({
      at: typeof e.at === "string" ? e.at : "",
      rung: (RUNGS.includes(e.rung as Rung) ? e.rung : 0) as Rung,
      outcome: (["attempted", "succeeded", "failed", "skipped"].includes(e.outcome as string)
        ? e.outcome
        : "skipped") as RepairLogEntry["outcome"],
      detail: sanitizeDetail(typeof e.detail === "string" ? e.detail : ""),
    }))
    .slice(-REPAIR_LOG_LIMIT);
  const incidentRaw = r.incident as Record<string, unknown> | null | undefined;
  const escRaw = (incidentRaw?.escalation ?? null) as Record<string, unknown> | null;
  const escalation: EscalationRecord | null =
    typeof escRaw === "object" &&
    escRaw !== null &&
    FAILURE_CLASSES.includes(escRaw.class as FailureClass) &&
    typeof escRaw.since === "string" &&
    typeof escRaw.issueRepo === "string"
      ? {
          class: escRaw.class as FailureClass,
          since: escRaw.since,
          queueTaskId: typeof escRaw.queueTaskId === "string" ? escRaw.queueTaskId : null,
          issueRepo: escRaw.issueRepo,
          issueNumber:
            typeof escRaw.issueNumber === "number" && Number.isFinite(escRaw.issueNumber)
              ? escRaw.issueNumber
              : null,
        }
      : null;
  const incident: Incident | null =
    typeof incidentRaw === "object" && incidentRaw !== null && typeof incidentRaw.id === "string"
      ? {
          id: incidentRaw.id,
          openedAt: typeof incidentRaw.openedAt === "string" ? incidentRaw.openedAt : "",
          humanBlocked: incidentRaw.humanBlocked === true,
          surfacing: (["none", "human-blocked", "escalate"].includes(incidentRaw.surfacing as string)
            ? incidentRaw.surfacing
            : "none") as Incident["surfacing"],
          escalation,
        }
      : null;
  return {
    source,
    lastAttemptAt: str(r.lastAttemptAt),
    lastSuccessAt: str(r.lastSuccessAt),
    consecutiveFailures: num(r.consecutiveFailures, 0),
    currentClass: cls,
    lastDetail: sanitizeDetail(typeof r.lastDetail === "string" ? r.lastDetail : ""),
    cooldownUntil: str(r.cooldownUntil),
    rungAttempts: attempts,
    repairLog,
    incident,
  };
}
