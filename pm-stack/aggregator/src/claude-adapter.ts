// Claude token adapter (#1163, epic #1047 Stage 2) — the Claude
// per-provider adapter on the #1153 framework (src/token-adapter.ts),
// following the ClinePass adapter (#1159, src/clinepass-adapter.ts):
//
//   GET https://claude.ai/api/organizations            -> [{ uuid, ... }]
//   GET https://claude.ai/api/organizations/<uuid>/usage
//     -> { five_hour: { utilization, resets_at },
//          seven_day: { utilization, resets_at }, ... }
//   five_hour -> window "5h"     (rolling 5h,  18000s)
//   seven_day -> window "weekly" (rolling 7d, 604800s)
//
// This replaces the prose regex extractor (extractClaude in
// src/quota-extract.ts: weekly only, self-described as a dated
// stewardship read) as the primary source for anthropic:claude-pro:
// the adapter produces BOTH the 5h and the weekly windows with their
// resets, on the gatherer tick. extractClaude may remain as the
// dated-read fallback (packet scope: do not remove it).
//
// utilization arrives as a number (percent) and is clamped to
// 0..100. Unknown window keys (seven_day_sonnet, extra_usage, ...)
// are skipped — the provider's own discipline, same as the ClinePass
// port; a payload with NO usable window is shape drift and fails
// closed as "unparseable" under the framework — the previous entry
// is retained, never zeroed.
//
// Credentials are CredentialRef-only (#1153 contract): this module
// never holds, logs, or serializes a credential value. The gatherer
// resolves the reference at tick time (Bitwarden by name, or a 0600
// file) and hands the value to the fetch through the injected
// resolver for the duration of the requests; the value appears only
// in the Cookie header of those requests (claude.ai's session
// credential is the sessionKey cookie, not a bearer token).
//
// Session expiry is the framework's signal, not a usage reading: a
// 401/403 from claude.ai is "session-expired" with needsRelogin —
// an expired session produces an explicit re-login state in the
// tick outputs and retains last-known windows, never zeros. Live-
// account verification is gated on #1155 naming the credential
// provenance; the gatherer activates this adapter only when a
// credential reference is configured (CLAUDE_CREDENTIAL_REF).

import type { QuotaCurrentEntry } from "./model.ts";
import { normalizeResetAt, serviceIdFor } from "./quota-gather-map.ts";
import {
  AdapterError,
  collectAdapter,
  credentialRefLabel,
  validateCredentialRef,
  type AdapterOutcome,
  type CredentialRef,
  type PreviousValue,
} from "./token-adapter.ts";
import { isRecord } from "./validate.ts";

export const CLAUDE_PROVIDER_ID = "claude";
export const CLAUDE_PROVIDER_NAME = "Claude";
export const CLAUDE_API_BASE = "https://claude.ai/api";
export const CLAUDE_ORGANIZATIONS_URL = `${CLAUDE_API_BASE}/organizations`;
export const CLAUDE_SOURCE = "token-adapter/claude";

export const claudeUsageUrl = (organizationUuid: string): string =>
  `${CLAUDE_API_BASE}/organizations/${encodeURIComponent(organizationUuid)}/usage`;

const WINDOW_KINDS = new Map<string, { key: string; windowSeconds: number }>([
  ["five_hour", { key: "5h", windowSeconds: 5 * 60 * 60 }],
  ["seven_day", { key: "weekly", windowSeconds: 7 * 24 * 60 * 60 }],
]);

const clampPercent = (value: number): number => Math.min(100, Math.max(0, value));

// Plain decimal spellings only — same discipline as the ClinePass
// port (#3005 field form): Number() also accepts "", hex and
// scientific spellings, which would turn a drifted provider field
// into a real measurement.
const PLAIN_DECIMAL = /^[+-]?(?:\d+(?:\.\d*)?|\.\d+)$/;

/** utilization as the provider reads it: a finite number, or a
 * plain-decimal string; anything else is not a reading. */
function utilizationOf(value: unknown): number | null {
  if (typeof value === "number") return Number.isFinite(value) ? value : null;
  if (typeof value === "string" && PLAIN_DECIMAL.test(value)) {
    const n = Number(value);
    return Number.isFinite(n) ? n : null;
  }
  return null;
}

export interface ClaudeWindow {
  name: string;
  usedPct: number;
  windowSeconds: number | null;
  resetsAt: string | null;
}

export interface ClaudeReading {
  windows: ClaudeWindow[];
}

/**
 * Parse one claude.ai usage payload (fail-closed): malformed
 * individual windows are skipped like the source provider does, but
 * a payload whose envelope drifts, or that yields no usable window
 * at all, throws AdapterError("unparseable").
 */
export function parseClaudeUsagePayload(payload: unknown): ClaudeReading {
  if (!isRecord(payload)) {
    throw new AdapterError("unparseable", "claude: usage payload is not an object");
  }
  const byKey = new Map<string, ClaudeWindow>();
  for (const [field, kind] of WINDOW_KINDS) {
    const item = payload[field];
    if (!isRecord(item)) continue;
    const rawPct = utilizationOf(item.utilization);
    if (rawPct === null) continue;
    byKey.set(kind.key, {
      name: kind.key,
      usedPct: clampPercent(rawPct),
      windowSeconds: kind.windowSeconds,
      resetsAt: normalizeResetAt(item.resets_at),
    });
  }
  if (byKey.size === 0) {
    throw new AdapterError("unparseable", "claude: no quota data in usage response");
  }
  return { windows: [...byKey.values()] };
}

/**
 * Resolve the organization uuid from the organizations payload:
 * the first entry carrying a non-empty uuid. Anything else is
 * shape drift and fails closed as "unparseable".
 */
export function organizationUuidOf(payload: unknown): string {
  if (!Array.isArray(payload)) {
    throw new AdapterError("unparseable", "claude: organizations payload is not an array");
  }
  for (const item of payload) {
    if (isRecord(item) && typeof item.uuid === "string" && item.uuid.trim() !== "") {
      return item.uuid;
    }
  }
  throw new AdapterError("unparseable", "claude: no organization with a uuid in response");
}

/** Map a parsed reading onto the quota-current contract. The
 * providerId "claude" maps to serviceId anthropic:claude-pro via
 * serviceIdFor — the rows this adapter sources. */
export function entryFromClaudeReading(reading: ClaudeReading, measuredAt: string): QuotaCurrentEntry {
  return {
    providerId: CLAUDE_PROVIDER_ID,
    providerName: CLAUDE_PROVIDER_NAME,
    serviceId: serviceIdFor(CLAUDE_PROVIDER_ID),
    state: "ok",
    measuredAt,
    source: CLAUDE_SOURCE,
    windows: reading.windows.map((w) => ({
      name: w.name,
      value: w.usedPct,
      pctOfLimit: w.usedPct,
      valueLabel: null,
      windowSeconds: w.windowSeconds,
      resetsAt: w.resetsAt,
      measuredAt,
      source: CLAUDE_SOURCE,
      state: "ok" as const,
    })),
  };
}

/** An error entry for a tick that produced no reading — never zeros. */
export function claudeErrorEntry(detail: string, measuredAt: string): QuotaCurrentEntry {
  return {
    providerId: CLAUDE_PROVIDER_ID,
    providerName: CLAUDE_PROVIDER_NAME,
    serviceId: serviceIdFor(CLAUDE_PROVIDER_ID),
    state: "error",
    measuredAt,
    source: CLAUDE_SOURCE,
    error: detail,
    windows: [],
  };
}

export type ClaudeFetch = (
  url: string,
  init: { method: string; headers: Record<string, string>; signal: AbortSignal },
) => Promise<{ status: number; headers?: { get(name: string): string | null }; text(): Promise<string> }>;

/**
 * One usage collection's fetches, shaped for the framework's
 * collectAdapter: resolve the organization, then read its usage.
 * A non-200 at either step is reported with that step's status, so
 * the framework classifies an expired session (401/403) identically
 * whichever step surfaces it. The session credential is used only
 * as the Cookie header value; it is never returned, logged, or
 * embedded in any outcome.
 */
export async function fetchClaudePayload(opts: {
  sessionKey: string;
  fetchImpl?: ClaudeFetch;
  timeoutMs?: number;
}): Promise<{ httpStatus: number; location?: string; payload: unknown }> {
  const fetchImpl = opts.fetchImpl ?? (fetch as unknown as ClaudeFetch);
  const timeoutMs = opts.timeoutMs ?? 15_000;
  const get = async (url: string) => {
    const res = await fetchImpl(url, {
      method: "GET",
      headers: {
        Cookie: `sessionKey=${opts.sessionKey}`,
        Accept: "application/json",
        "Accept-Encoding": "identity",
      },
      signal: AbortSignal.timeout(timeoutMs),
    });
    let payload: unknown = null;
    try {
      const text = await res.text();
      payload = text ? (JSON.parse(text) as unknown) : null;
    } catch {
      payload = null; // an unreadable body on a 200 fails closed in parse
    }
    const location = res.headers?.get("location") ?? undefined;
    return { httpStatus: res.status, ...(location ? { location } : {}), payload };
  };
  const orgs = await get(CLAUDE_ORGANIZATIONS_URL);
  if (orgs.httpStatus !== 200) return orgs;
  let uuid: string;
  try {
    uuid = organizationUuidOf(orgs.payload);
  } catch {
    return { httpStatus: 200, payload: null }; // drift fails closed in parse
  }
  return get(claudeUsageUrl(uuid));
}

function retainedEntryOutcome(
  status: AdapterOutcome<ClaudeReading>["status"],
  session: AdapterOutcome<ClaudeReading>["session"],
  detail: string | undefined,
  previous: PreviousValue<QuotaCurrentEntry> | null,
  now: Date,
): AdapterOutcome<QuotaCurrentEntry> {
  return {
    status,
    value: previous?.value ?? null,
    stale: previous != null,
    ageSeconds: previous ? Math.max(0, Math.round((now.getTime() - Date.parse(previous.measuredAt)) / 1000)) : null,
    session,
    needsRelogin: status === "session-expired",
    ...(detail ? { detail } : {}),
  };
}

/**
 * Collect one Claude reading through the framework: session
 * classification first (401/403 -> session-expired + re-login,
 * previous retained), then the fail-closed parse. On success the
 * value is the quota-current entry; on any failure it is the
 * retained previous entry (or null) — never a fabricated zero.
 */
export async function collectClaudeAdapter(opts: {
  now: Date;
  previous: PreviousValue<QuotaCurrentEntry> | null;
  fetchPayload: () => Promise<{ httpStatus: number; location?: string; payload: unknown }>;
}): Promise<AdapterOutcome<QuotaCurrentEntry>> {
  const outcome = await collectAdapter<ClaudeReading>({
    now: opts.now,
    previous: null,
    fetchPayload: opts.fetchPayload,
    parse: parseClaudeUsagePayload,
  });
  if (outcome.status === "ok" && outcome.value) {
    return {
      status: "ok",
      value: entryFromClaudeReading(outcome.value, opts.now.toISOString()),
      stale: false,
      ageSeconds: null,
      session: "ok",
      needsRelogin: false,
    };
  }
  return retainedEntryOutcome(outcome.status, outcome.session, outcome.detail, opts.previous, opts.now);
}

/**
 * Parse a credential reference from configuration text
 * ("bitwarden:<item name>" or "file:<path>") — references only.
 * Null/empty means the adapter is not configured for this host
 * (provenance unnamed, #1155) and the gatherer skips it.
 */
export function claudeCredentialRefFromConfig(value: string | undefined): CredentialRef | null {
  if (!value || value.trim() === "") return null;
  const v = value.trim();
  if (v.startsWith("bitwarden:")) return validateCredentialRef({ kind: "bitwarden", name: v.slice("bitwarden:".length) });
  if (v.startsWith("file:")) return validateCredentialRef({ kind: "file", path: v.slice("file:".length), mode: 0o600 });
  throw new AdapterError(
    "fetch-failed",
    "claude credential config must be bitwarden:<item name> or file:<path> — a reference, never a credential value",
  );
}

/**
 * The gatherer's Claude collection: validate the reference, resolve
 * its value through the injected resolver (the value lives only
 * inside the fetch calls), and collect fail-closed. An unresolvable
 * reference is a fetch failure naming the reference — never a zero,
 * never a value leak.
 */
export async function gatherClaude(opts: {
  now: Date;
  previous: PreviousValue<QuotaCurrentEntry> | null;
  credentialRef: CredentialRef;
  resolveCredentialValue: (ref: CredentialRef) => Promise<string | null>;
  fetchImpl?: ClaudeFetch;
  timeoutMs?: number;
}): Promise<AdapterOutcome<QuotaCurrentEntry>> {
  let ref: CredentialRef;
  try {
    ref = validateCredentialRef(opts.credentialRef);
  } catch (err) {
    return retainedEntryOutcome(
      "fetch-failed",
      "unknown",
      err instanceof Error ? err.message : String(err),
      opts.previous,
      opts.now,
    );
  }
  let sessionKey: string | null;
  try {
    sessionKey = await opts.resolveCredentialValue(ref);
  } catch {
    sessionKey = null;
  }
  if (!sessionKey) {
    return retainedEntryOutcome(
      "fetch-failed",
      "unknown",
      `claude credential unavailable (${credentialRefLabel(ref)})`,
      opts.previous,
      opts.now,
    );
  }
  return collectClaudeAdapter({
    now: opts.now,
    previous: opts.previous,
    fetchPayload: () => fetchClaudePayload({ sessionKey: sessionKey as string, fetchImpl: opts.fetchImpl, timeoutMs: opts.timeoutMs }),
  });
}
