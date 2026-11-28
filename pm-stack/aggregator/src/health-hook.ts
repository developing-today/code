// C3 health-hook incident records (#12599 decision b, #12602): a
// health hook posts its full incident record to the alert throat as
// the `healthHook` extension (source "health-hook", normally co-riding
// the `stackHealth` extension). The alerts ledger is the record of
// detail: the intake validates the record here and stores it on the
// incident's `opened` / `closed` event detail (see src/alerts.ts);
// the events stream stays the thin audit index (the AuditEvent append
// lives in src/health-hook-audit-event.ts). The record shape is the
// C3 design's "Report format"
// (doc/2026-11-20T00-46-00Z_design_c3-health-hooks/note.md).
import { validateStackHealth, type StackHealthInput } from "./stack-health.ts";

export type HealthHookState = "down" | "up" | "transient";

/** The full C3 incident record, as stored in the alerts ledger. */
export interface HealthHookRecord {
  target: string;
  state: HealthHookState;
  caller: string;
  /** When the hook fired (ISO), as reported. */
  firedAt: string;
  /** When the caller observed the failure (ISO), or null. */
  observedAt: string | null;
  incidentId: string;
  /** The down incident a recovery closes (its incidentId), or null. */
  recoveryOf: string | null;
  /** Structured probe output; members vary by target and evolve with
   * the probes, so only the container is validated (see below). */
  evidence: Record<string, unknown>;
  /** The record's own stackHealth member (same shape the throat's
   * stackHealth extension carries), when the poster includes it. */
  stackHealth?: StackHealthInput;
}

/**
 * Validate the intake `healthHook` extension object (C3 design shape).
 * Scalar fields are validated strictly, in the validateStackHealth
 * house style, and every error names its field. `evidence` is
 * validated as an object only: its members are probe-defined
 * (reachability / serviceState / authHealth / recentChanges /
 * hostHealth / sessionLiveness, plus probe additions such as
 * confirmFailure) and must not 400 a poster whose probes evolved.
 * Envelope duplicates (source / key / severity / title / body) are
 * type-checked when present but not stored — they already live on
 * the incident row the post creates.
 */
export function validateHealthHook(
  value: unknown,
): { ok: true; value: HealthHookRecord } | { ok: false; error: string } {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return { ok: false, error: "healthHook must be an object" };
  }
  const b = value as Record<string, unknown>;
  for (const field of ["source", "key", "severity", "title", "body"] as const) {
    if (b[field] !== undefined && typeof b[field] !== "string") {
      return { ok: false, error: `healthHook.${field} must be a string` };
    }
  }
  const target = typeof b.target === "string" ? b.target.trim() : "";
  if (!target) return { ok: false, error: "healthHook.target required" };
  if (b.state !== "down" && b.state !== "up" && b.state !== "transient") {
    return { ok: false, error: 'healthHook.state must be "down" | "up" | "transient"' };
  }
  const caller = typeof b.caller === "string" ? b.caller.trim() : "";
  if (!caller) return { ok: false, error: "healthHook.caller required" };
  const firedAt = typeof b.firedAt === "string" ? b.firedAt.trim() : "";
  if (!firedAt) return { ok: false, error: "healthHook.firedAt required" };
  if (b.observedAt === undefined) return { ok: false, error: "healthHook.observedAt required" };
  if (b.observedAt !== null && typeof b.observedAt !== "string") {
    return { ok: false, error: "healthHook.observedAt must be a string or null" };
  }
  const incidentId = typeof b.incidentId === "string" ? b.incidentId.trim() : "";
  if (!incidentId) return { ok: false, error: "healthHook.incidentId required" };
  if (b.recoveryOf === undefined) return { ok: false, error: "healthHook.recoveryOf required" };
  if (b.recoveryOf !== null && typeof b.recoveryOf !== "string") {
    return { ok: false, error: "healthHook.recoveryOf must be a string or null" };
  }
  if (!b.evidence || typeof b.evidence !== "object" || Array.isArray(b.evidence)) {
    return { ok: false, error: "healthHook.evidence must be an object" };
  }
  let stackHealth: StackHealthInput | undefined;
  if (b.stackHealth !== undefined) {
    const v = validateStackHealth(b.stackHealth);
    if (!v.ok) return { ok: false, error: v.error.replace(/^stackHealth/, "healthHook.stackHealth") };
    stackHealth = v.value;
  }
  return {
    ok: true,
    value: {
      target,
      state: b.state,
      caller,
      firedAt,
      observedAt: b.observedAt as string | null,
      incidentId,
      recoveryOf: b.recoveryOf as string | null,
      evidence: b.evidence as Record<string, unknown>,
      ...(stackHealth ? { stackHealth } : {}),
    },
  };
}
