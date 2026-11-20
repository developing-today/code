// Remediation executor for the PM-stack SLO watchdog (#9989, design
// note doc/2026-11-14T12-54-00Z_design_pm_stack_slo_watchdog §2).
//
// The watchdog DETECTS (watchdog.ts, #1148) and ESCALATES (#9991). This
// module is the REMEDIATE half: it decides, for a given facet + catalogue
// action + current state, whether an action is allowed to run right now,
// and updates durable state after the attempt + verification.
//
// Safety (design §5 "never-do" list): stateful services (Pad, Grist, any
// database) are NEVER in the catalogue and every request for one is
// refused as `stateful`. Deploy-time refusal, caps, cooldowns and circuit
// breakers are CODE, not guidance — the decision function is the single
// gate every caller must pass through.
//
// This module is PURE: no I/O, no systemd, no network. The host (a
// scheduler / the watchdog loop) owns persistence of RemediationState and
// the actual execution of the allowed action. That keeps the decision
// logic exhaustively testable (see test/remediation.test.ts) and keeps
// the "never-do" guarantees independent of any execution path.

export type FacetId =
  | "feed"
  | "quota"
  | "orchestration"
  | "aggregator"
  | "dispatch"
  | "projector"
  | "tunnel"
  | "pad"
  | "grist"
  | "database";

export type ActionId =
  | "restart-collector"
  | "rerun-feed-push"
  | "restart-aggregator"
  | "restart-dispatch"
  | "restart-projector"
  | "restart-tunnel"
  | "rerun-quota-refresh";

export type ActionKind = "restart" | "rerun";
export type VerificationKind = "health" | "age-decrease";

export interface CatalogueEntry {
  facet: FacetId;
  action: ActionId;
  kind: ActionKind;
  unit: string;
  attemptCap: number; // per incident
  cooldownMs: number;
  verificationMs: number;
  verification: VerificationKind;
}

// §2 catalogue — stateless producers only. Pad/Grist/databases are
// deliberately absent (stateful, never auto-restarted).
export const CATALOGUE: readonly CatalogueEntry[] = [
  { facet: "feed", action: "restart-collector", kind: "restart", unit: "pm-feed-collector", attemptCap: 3, cooldownMs: 15 * 60_000, verificationMs: 5 * 60_000, verification: "health" },
  { facet: "feed", action: "rerun-feed-push", kind: "rerun", unit: "feed-push", attemptCap: 3, cooldownMs: 15 * 60_000, verificationMs: 10 * 60_000, verification: "age-decrease" },
  { facet: "aggregator", action: "restart-aggregator", kind: "restart", unit: "pm-aggregator", attemptCap: 3, cooldownMs: 20 * 60_000, verificationMs: 5 * 60_000, verification: "health" },
  { facet: "dispatch", action: "restart-dispatch", kind: "restart", unit: "pm-dispatch", attemptCap: 3, cooldownMs: 20 * 60_000, verificationMs: 5 * 60_000, verification: "health" },
  { facet: "projector", action: "restart-projector", kind: "restart", unit: "pm-projector", attemptCap: 2, cooldownMs: 20 * 60_000, verificationMs: 5 * 60_000, verification: "health" },
  { facet: "tunnel", action: "restart-tunnel", kind: "restart", unit: "pm-tunnel", attemptCap: 2, cooldownMs: 30 * 60_000, verificationMs: 10 * 60_000, verification: "health" },
  { facet: "quota", action: "rerun-quota-refresh", kind: "rerun", unit: "quota-refresh", attemptCap: 2, cooldownMs: 30 * 60_000, verificationMs: 10 * 60_000, verification: "age-decrease" },
];

// §2 guardrails.
export const FACET_DAILY_CAP = 6;
export const GLOBAL_DAILY_CAP = 15;
export const CIRCUIT_BREAKER_EXHAUSTED_THRESHOLD = 3;
export const CIRCUIT_BREAKER_OPEN_MS = 24 * 60 * 60_000;
export const EXHAUSTED_INCIDENT_WINDOW_MS = 24 * 60 * 60_000;

// Facets that are stateful and must never be auto-remediated (§2, §5).
export const STATEFUL_FACETS: ReadonlySet<FacetId> = new Set<FacetId>(["pad", "grist", "database", "orchestration"]);

// --- State ---

export interface ActionState {
  /** ISO-ish epoch ms of the last attempt (null = never). */
  lastAttemptAt: number | null;
  /** Cooldown gate: no attempt before this time. */
  cooldownUntil: number | null;
  /** The incident this attempt count belongs to (null = none). */
  currentIncidentId: string | null;
  /** Attempts consumed in the current incident. */
  attemptsUsed: number;
  /** True once the incident is exhausted (all attempts used, still bad). */
  exhausted: boolean;
}

export interface CircuitBreaker {
  openedAt: number;
  closesAt: number;
}

export interface DailyCounter {
  /** UTC date key YYYY-MM-DD. */
  date: string;
  count: number;
}

export interface InFlight {
  facet: FacetId;
  action: ActionId;
  startedAt: number;
}

export interface RemediationState {
  actions: Record<string, ActionState>;
  circuitBreakers: Record<string, CircuitBreaker | null>;
  /** Timestamps (epoch ms) of exhausted incidents per facet (last 24h). */
  exhaustedIncidents: Record<string, number[]>;
  dailyPerFacet: Record<string, DailyCounter>;
  dailyGlobal: DailyCounter;
  inFlight: InFlight | null;
}

function utcDateKey(now: number): string {
  const d = new Date(now);
  const y = d.getUTCFullYear();
  const m = String(d.getUTCMonth() + 1).padStart(2, "0");
  const day = String(d.getUTCDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

export function emptyState(now: number): RemediationState {
  return {
    actions: {},
    circuitBreakers: {},
    exhaustedIncidents: {},
    dailyPerFacet: {},
    dailyGlobal: { date: utcDateKey(now), count: 0 },
    inFlight: null,
  };
}

function actionKey(facet: FacetId, action: ActionId): string {
  return `${facet}:${action}`;
}

function getActionState(state: RemediationState, key: string): ActionState {
  return (
    state.actions[key] ?? {
      lastAttemptAt: null,
      cooldownUntil: null,
      currentIncidentId: null,
      attemptsUsed: 0,
      exhausted: false,
    }
  );
}

// --- Journal ---

export interface RemediationJournalEntry {
  ts: string; // ISO
  facet: FacetId;
  action: ActionId;
  incidentId: string;
  decision: "allowed" | "refused";
  reason: string;
  blockedBy?: string;
  deployLocked: boolean;
  stateSnapshot: {
    attemptsUsed: number;
    cooldownUntil: number | null;
    circuitBreakerOpen: boolean;
    dailyFacetCount: number;
    dailyGlobalCount: number;
    inFlight: boolean;
  };
}

// --- Decision ---

export type BlockedBy =
  | "stateful"
  | "not-in-catalogue"
  | "deploy-lock"
  | "circuit-breaker"
  | "in-flight"
  | "daily-cap-facet"
  | "daily-cap-global"
  | "cooldown"
  | "attempt-cap";

export interface RemediationDecision {
  allowed: boolean;
  reason: string;
  blockedBy?: BlockedBy;
  journalEntry: RemediationJournalEntry;
}

function makeJournalEntry(
  entry: CatalogueEntry,
  incidentId: string,
  decision: "allowed" | "refused",
  reason: string,
  blockedBy: BlockedBy | undefined,
  deployLocked: boolean,
  state: RemediationState,
  now: number,
  actionState: ActionState,
): RemediationJournalEntry {
  const cb = state.circuitBreakers[entry.facet];
  const dailyFacet = state.dailyPerFacet[entry.facet];
  return {
    ts: new Date(now).toISOString(),
    facet: entry.facet,
    action: entry.action,
    incidentId,
    decision,
    reason,
    ...(blockedBy ? { blockedBy } : {}),
    deployLocked,
    stateSnapshot: {
      attemptsUsed: actionState.attemptsUsed,
      cooldownUntil: actionState.cooldownUntil,
      circuitBreakerOpen: !!cb && cb.closesAt > now,
      dailyFacetCount: dailyFacet?.count ?? 0,
      dailyGlobalCount: state.dailyGlobal.count,
      inFlight: state.inFlight !== null,
    },
  };
}

export function findCatalogueEntry(facet: FacetId, action: ActionId): CatalogueEntry | undefined {
  return CATALOGUE.find((e) => e.facet === facet && e.action === action);
}

export function decideRemediation(
  facet: FacetId,
  action: ActionId,
  state: RemediationState,
  now: number,
  deployLocked: boolean,
  incidentId: string,
): RemediationDecision {
  // Stateful services are never auto-remediated (§2, §5).
  if (STATEFUL_FACETS.has(facet)) {
    const syntheticEntry: CatalogueEntry = {
      facet,
      action,
      kind: "restart",
      unit: "",
      attemptCap: 0,
      cooldownMs: 0,
      verificationMs: 0,
      verification: "health",
    };
    const as = getActionState(state, actionKey(facet, action));
    return {
      allowed: false,
      reason: `stateful service (${facet}) is never auto-remediated; escalate with diagnostics`,
      blockedBy: "stateful",
      journalEntry: makeJournalEntry(syntheticEntry, incidentId, "refused", `stateful service (${facet}) is never auto-remediated; escalate with diagnostics`, "stateful", deployLocked, state, now, as),
    };
  }

  const entry = findCatalogueEntry(facet, action);
  if (!entry) {
    const as = getActionState(state, actionKey(facet, action));
    const syntheticEntry: CatalogueEntry = {
      facet,
      action,
      kind: "restart",
      unit: "",
      attemptCap: 0,
      cooldownMs: 0,
      verificationMs: 0,
      verification: "health",
    };
    return {
      allowed: false,
      reason: `action ${action} is not in the remediation catalogue for facet ${facet}`,
      blockedBy: "not-in-catalogue",
      journalEntry: makeJournalEntry(syntheticEntry, incidentId, "refused", `action ${action} is not in the remediation catalogue for facet ${facet}`, "not-in-catalogue", deployLocked, state, now, as),
    };
  }

  const key = actionKey(facet, action);
  const actionState = getActionState(state, key);

  // Deploy lock: refuse and log (§2 precondition, §5).
  if (deployLocked) {
    return {
      allowed: false,
      reason: "deploy in progress: remediation refused (deploy lock present)",
      blockedBy: "deploy-lock",
      journalEntry: makeJournalEntry(entry, incidentId, "refused", "deploy in progress: remediation refused (deploy lock present)", "deploy-lock", deployLocked, state, now, actionState),
    };
  }

  // Circuit breaker open?
  const cb = state.circuitBreakers[facet];
  if (cb && cb.closesAt > now) {
    return {
      allowed: false,
      reason: `circuit breaker open for ${facet} until ${new Date(cb.closesAt).toISOString()} (3 exhausted incidents in 24h)`,
      blockedBy: "circuit-breaker",
      journalEntry: makeJournalEntry(entry, incidentId, "refused", `circuit breaker open for ${facet}`, "circuit-breaker", deployLocked, state, now, actionState),
    };
  }

  // One action in flight at a time (§2).
  if (state.inFlight) {
    return {
      allowed: false,
      reason: `another remediation action in flight (${state.inFlight.facet}/${state.inFlight.action}); one action at a time`,
      blockedBy: "in-flight",
      journalEntry: makeJournalEntry(entry, incidentId, "refused", `another remediation action in flight`, "in-flight", deployLocked, state, now, actionState),
    };
  }

  // Daily cap per facet.
  const facetDaily = state.dailyPerFacet[facet];
  const facetDate = utcDateKey(now);
  const facetCount = facetDaily && facetDaily.date === facetDate ? facetDaily.count : 0;
  if (facetCount >= FACET_DAILY_CAP) {
    return {
      allowed: false,
      reason: `per-facet daily cap reached (${FACET_DAILY_CAP}) for ${facet}`,
      blockedBy: "daily-cap-facet",
      journalEntry: makeJournalEntry(entry, incidentId, "refused", `per-facet daily cap reached`, "daily-cap-facet", deployLocked, state, now, actionState),
    };
  }

  // Daily cap global.
  const globalDate = state.dailyGlobal.date;
  const globalCount = globalDate === facetDate ? state.dailyGlobal.count : 0;
  if (globalCount >= GLOBAL_DAILY_CAP) {
    return {
      allowed: false,
      reason: `global daily cap reached (${GLOBAL_DAILY_CAP})`,
      blockedBy: "daily-cap-global",
      journalEntry: makeJournalEntry(entry, incidentId, "refused", `global daily cap reached`, "daily-cap-global", deployLocked, state, now, actionState),
    };
  }

  // Cooldown.
  if (actionState.cooldownUntil !== null && actionState.cooldownUntil > now) {
    return {
      allowed: false,
      reason: `cooldown active for ${facet}/${action} until ${new Date(actionState.cooldownUntil).toISOString()}`,
      blockedBy: "cooldown",
      journalEntry: makeJournalEntry(entry, incidentId, "refused", `cooldown active`, "cooldown", deployLocked, state, now, actionState),
    };
  }

  // Attempt cap per incident.
  const attemptsForIncident =
    actionState.currentIncidentId === incidentId ? actionState.attemptsUsed : 0;
  if (attemptsForIncident >= entry.attemptCap) {
    return {
      allowed: false,
      reason: `attempt cap reached (${entry.attemptCap}) for ${facet}/${action} in incident ${incidentId}`,
      blockedBy: "attempt-cap",
      journalEntry: makeJournalEntry(entry, incidentId, "refused", `attempt cap reached`, "attempt-cap", deployLocked, state, now, actionState),
    };
  }

  return {
    allowed: true,
    reason: `allowed: ${facet}/${action} (attempt ${attemptsForIncident + 1}/${entry.attemptCap})`,
    journalEntry: makeJournalEntry(entry, incidentId, "allowed", `allowed`, undefined, deployLocked, state, now, actionState),
  };
}

// --- State update after an attempt ---

export interface AttemptOutcome {
  verified: boolean;
}

export function recordAttempt(
  state: RemediationState,
  facet: FacetId,
  action: ActionId,
  incidentId: string,
  now: number,
  outcome: AttemptOutcome,
): RemediationState {
  const entry = findCatalogueEntry(facet, action);
  // Work on a shallow clone.
  const next: RemediationState = {
    actions: { ...state.actions },
    circuitBreakers: { ...state.circuitBreakers },
    exhaustedIncidents: { ...state.exhaustedIncidents },
    dailyPerFacet: { ...state.dailyPerFacet },
    dailyGlobal: { ...state.dailyGlobal },
    inFlight: null, // attempt finished
  };

  // Stateful/unknown actions: do not track (should have been refused).
  if (!entry) return next;

  const key = actionKey(facet, action);
  const prev = getActionState(state, key);

  // Reset attempt counter if this is a new incident.
  const isNewIncident = prev.currentIncidentId !== incidentId;
  const attemptsUsed = isNewIncident ? 0 : prev.attemptsUsed;

  const newAttemptsUsed = attemptsUsed + 1;
  const cooldownUntil = now + entry.cooldownMs;

  const exhausted = !outcome.verified && newAttemptsUsed >= entry.attemptCap;

  next.actions[key] = {
    lastAttemptAt: now,
    cooldownUntil,
    currentIncidentId: incidentId,
    attemptsUsed: newAttemptsUsed,
    exhausted,
  };

  // Daily counters.
  const dateKey = utcDateKey(now);
  const facetCounter = next.dailyPerFacet[facet];
  if (facetCounter && facetCounter.date === dateKey) {
    next.dailyPerFacet[facet] = { date: dateKey, count: facetCounter.count + 1 };
  } else {
    next.dailyPerFacet[facet] = { date: dateKey, count: 1 };
  }
  if (next.dailyGlobal.date === dateKey) {
    next.dailyGlobal = { date: dateKey, count: next.dailyGlobal.count + 1 };
  } else {
    next.dailyGlobal = { date: dateKey, count: 1 };
  }

  // If verified, the incident is closed: reset attempt tracking.
  if (outcome.verified) {
    next.actions[key] = {
      lastAttemptAt: now,
      cooldownUntil: null,
      currentIncidentId: null,
      attemptsUsed: 0,
      exhausted: false,
    };
    return next;
  }

  // If exhausted, record the exhausted incident and check circuit breaker.
  if (exhausted) {
    const list = (next.exhaustedIncidents[facet] ?? []).filter((t) => now - t < EXHAUSTED_INCIDENT_WINDOW_MS);
    list.push(now);
    next.exhaustedIncidents[facet] = list;
    if (list.length >= CIRCUIT_BREAKER_EXHAUSTED_THRESHOLD) {
      next.circuitBreakers[facet] = {
        openedAt: now,
        closesAt: now + CIRCUIT_BREAKER_OPEN_MS,
      };
    }
  }

  return next;
}

// --- Serialization helpers ---

export function serializeState(state: RemediationState): string {
  return JSON.stringify(state);
}

export function deserializeState(raw: string, now: number): RemediationState {
  try {
    const parsed = JSON.parse(raw) as Partial<RemediationState>;
    if (!parsed || typeof parsed !== "object") return emptyState(now);
    return {
      actions: parsed.actions && typeof parsed.actions === "object" ? parsed.actions : {},
      circuitBreakers: parsed.circuitBreakers && typeof parsed.circuitBreakers === "object" ? parsed.circuitBreakers : {},
      exhaustedIncidents: parsed.exhaustedIncidents && typeof parsed.exhaustedIncidents === "object" ? parsed.exhaustedIncidents : {},
      dailyPerFacet: parsed.dailyPerFacet && typeof parsed.dailyPerFacet === "object" ? parsed.dailyPerFacet : {},
      dailyGlobal: parsed.dailyGlobal && typeof parsed.dailyGlobal === "object" && parsed.dailyGlobal.date ? parsed.dailyGlobal : { date: utcDateKey(now), count: 0 },
      inFlight: parsed.inFlight ?? null,
    };
  } catch {
    return emptyState(now);
  }
}

// --- Helpers for callers ---

export function setInFlight(state: RemediationState, facet: FacetId, action: ActionId, now: number): RemediationState {
  return { ...state, inFlight: { facet, action, startedAt: now } };
}

export function clearInFlight(state: RemediationState): RemediationState {
  return { ...state, inFlight: null };
}

export function isCircuitBreakerOpen(state: RemediationState, facet: FacetId, now: number): boolean {
  const cb = state.circuitBreakers[facet];
  return !!cb && cb.closesAt > now;
}
