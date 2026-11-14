import type { QuotaCurrentEntry } from "./model.ts";
import { isIsoDate } from "./validate.ts";

export type CredentialLifecycleState =
  | "ok"
  | "expiring"
  | "expired"
  | "lapsed"
  | "rejected"
  | "provider-error"
  | "not-configured"
  | "unknown";

export interface CredentialLifecycleRow {
  providerId: string;
  providerName: string;
  serviceId: string;
  label: string;
  credentialId?: string;
  state: CredentialLifecycleState;
  measuredAt: string;
  expiresAt?: string;
  reason?: string;
  /** A user-owned next step, only when the provider gives a proven route. */
  userAction?: { owner: "user"; title: string };
}

const EXPIRING_WITHIN_MS = 48 * 60 * 60_000;

// Cap `t` at `max` UTF-16 code units, but never leave half of a surrogate
// pair at the cut (#7741, mirroring the #7694 capUnits guard in actions.ts):
// an astral character straddling the cap is dropped whole. Local copy, like
// comments.ts (#7731) — a shared import would tangle module dependencies.
function capUnits(t: string, max: number): string {
  if (t.length <= max) return t;
  let end = max;
  const before = t.charCodeAt(end - 1);
  const after = t.charCodeAt(end);
  if (before >= 0xd800 && before <= 0xdbff && after >= 0xdc00 && after <= 0xdfff) end--;
  return t.slice(0, end);
}

/** Provider errors are useful evidence, but must never turn a credential value into feed data. */
export function safeProviderReason(reason: string): string {
  return capUnits(
    reason
      .replace(/\bBearer\s+[^\s,;]+/gi, "Bearer [redacted]")
      .replace(/\beyJ[a-zA-Z0-9_-]{8,}\.[a-zA-Z0-9_-]{8,}(?:\.[a-zA-Z0-9_-]+)?\b/g, "[redacted]")
      .replace(
        /\b((?:access|refresh|api|auth)?[_-]?(?:token|secret|password|cookie|api[_-]?key))\s*[:=]\s*["']?[^\s,;"'}]+/gi,
        "$1=[redacted]",
      )
      .replace(/\bsk-[A-Za-z0-9_-]{8,}\b/g, "[redacted]"),
    240,
  );
}

export function lifecycleForEntry(entry: QuotaCurrentEntry, now = new Date()): CredentialLifecycleRow {
  const expiresAt = entry.credentialExpiresAt;
  const expiresMs = expiresAt ? Date.parse(expiresAt) : Number.NaN;
  const hasExpiry = Number.isFinite(expiresMs);
  const error = entry.error ? safeProviderReason(entry.error) : undefined;
  let state: CredentialLifecycleState;

  if (entry.state === "lapsed") state = "lapsed";
  else if (entry.state === "unconfigured") state = "not-configured";
  else if (entry.state === "error" && /\bClaude session expired\b/i.test(error ?? "")) state = "expired";
  else if (
    entry.state === "error" &&
    /\b(?:credential rejected|invalid (?:api )?key|invalid token)\b/i.test(error ?? "")
  ) {
    state = "rejected";
  } else if (entry.state === "error") state = "provider-error";
  else if (hasExpiry && expiresMs <= now.getTime()) state = "expired";
  else if (entry.retainedFromFailedTick) state = "unknown";
  else if (hasExpiry && expiresMs - now.getTime() <= EXPIRING_WITHIN_MS) state = "expiring";
  else if (hasExpiry) state = "ok";
  else state = "unknown";

  const row: CredentialLifecycleRow = {
    providerId: entry.providerId,
    providerName: entry.providerName,
    serviceId: entry.serviceId,
    label: entry.credentialLabel ?? entry.providerName,
    ...(entry.credentialId ? { credentialId: entry.credentialId } : {}),
    state,
    measuredAt: entry.measuredAt,
    ...(hasExpiry ? { expiresAt } : {}),
    ...(error ? { reason: error } : {}),
  };
  if (
    (entry.providerId === "claude" || entry.providerId === "_claude") &&
    (state === "expired" || state === "expiring")
  ) {
    const title =
      entry.credentialSource === "opencode"
        ? "Refresh Anthropic authentication in OpenCode"
        : entry.credentialSource === "environment"
          ? "Renew the configured Claude OAuth environment credential"
          : entry.credentialSource === "claude-code" || /\bClaude session expired\b/i.test(error ?? "")
            ? "Open Claude Code to sign in again"
            : undefined;
    if (title) row.userAction = { owner: "user", title };
  }
  if ((entry.providerId === "claude" || entry.providerId === "_claude") && state === "expired" && !row.reason) {
    row.reason = "Claude credential expiry is confirmed by its configured credential source.";
  }
  return row;
}

export function isCredentialLifecycleRow(value: unknown): value is CredentialLifecycleRow {
  if (!value || typeof value !== "object") return false;
  const row = value as Record<string, unknown>;
  const states: CredentialLifecycleState[] = [
    "ok",
    "expiring",
    "expired",
    "lapsed",
    "rejected",
    "provider-error",
    "not-configured",
    "unknown",
  ];
  return (
    typeof row.providerId === "string" &&
    typeof row.providerName === "string" &&
    typeof row.serviceId === "string" &&
    typeof row.label === "string" &&
    typeof row.state === "string" &&
    states.includes(row.state as CredentialLifecycleState) &&
    isIsoDate(row.measuredAt) &&
    (row.credentialId === undefined || typeof row.credentialId === "string") &&
    (row.expiresAt === undefined || isIsoDate(row.expiresAt)) &&
    (row.reason === undefined || typeof row.reason === "string") &&
    (row.userAction === undefined ||
      (typeof row.userAction === "object" &&
        row.userAction !== null &&
        (row.userAction as Record<string, unknown>).owner === "user" &&
        typeof (row.userAction as Record<string, unknown>).title === "string"))
  );
}
