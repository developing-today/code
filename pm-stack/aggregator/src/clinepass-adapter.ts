// ClinePass token adapter (#1159, epic #1047 Stage 2, order 5a) — the
// first per-provider adapter on the #1153 framework
// (src/token-adapter.ts), porting the dashboard mapping of
// OpenChamber's quota provider (server/lib/quota/providers/cline-pass.js):
//
//   GET https://api.cline.bot/api/v1/users/me/plan/usage-limits
//   -> { data: { limits: [{ type, percentUsed, resetsAt }] } }
//   type five_hour -> window "5h"      (rolling 5h,  18000s)
//   type weekly    -> window "weekly"  (rolling 7d, 604800s)
//   type monthly   -> window "monthly" (calendar month, no fixed duration)
//
// percentUsed arrives as a number or a numeric string and is clamped
// to 0..100. Unknown limit types and malformed windows are skipped
// (the provider's own discipline); a payload with NO usable window is
// shape drift and fails closed as "unparseable" under the framework —
// the previous entry is retained, never zeroed.
//
// Credentials are CredentialRef-only (#1153 contract): this module
// never holds, logs, or serializes a credential value. The gatherer
// resolves the reference at tick time (Bitwarden by name, or a 0600
// file) and hands the value to the fetch through the injected
// resolver for the duration of one request; the value appears only
// in the Authorization header of that request.
//
// Session expiry is the framework's signal, not a usage reading: a
// 401/403 from the dashboard is "session-expired" with needsRelogin —
// an expired token can never produce a 0% row. Live-account
// verification is gated on #1155 naming the credential provenance;
// the gatherer activates this adapter only when a credential
// reference is configured (CLINEPASS_CREDENTIAL_REF).

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

export const CLINEPASS_PROVIDER_ID = "cline-pass";
export const CLINEPASS_PROVIDER_NAME = "ClinePass";
export const CLINEPASS_USAGE_URL = "https://api.cline.bot/api/v1/users/me/plan/usage-limits";
export const CLINEPASS_SOURCE = "token-adapter/cline-pass";

/** Window kinds, ported from the OpenChamber provider's WINDOW_KINDS. */
const WINDOW_KINDS = new Map<string, { key: string; windowSeconds: number | null }>([
  ["five_hour", { key: "5h", windowSeconds: 5 * 60 * 60 }],
  ["weekly", { key: "weekly", windowSeconds: 7 * 24 * 60 * 60 }],
  ["monthly", { key: "monthly", windowSeconds: null }],
]);

const clampPercent = (value: number): number => Math.min(100, Math.max(0, value));

// Plain decimal spellings only (#3005 field form, closing audit
// task 4350 / issue #3006). Number() also accepts "", whitespace-
// padded, hex ("0x10" -> 16), and scientific ("1e2" -> 100) strings,
// which would turn a drifted provider field into a real measurement;
// reject every non-decimal spelling so drift reads as not-a-reading.
// The sign stays: this gate's domain admits negatives (clamped by
// clampPercent), like aa.ts's PLAIN_DECIMAL.
const PLAIN_DECIMAL = /^[+-]?(?:\d+(?:\.\d*)?|\.\d+)$/;

/** percentUsed as the provider reads it: a finite number, or a
 * plain-decimal string; anything else is not a reading. */
function percentUsedOf(value: unknown): number | null {
  if (typeof value === "number") return Number.isFinite(value) ? value : null;
  if (typeof value === "string" && PLAIN_DECIMAL.test(value)) {
    const n = Number(value);
    return Number.isFinite(n) ? n : null;
  }
  return null;
}

export interface ClinePassWindow {
  name: string;
  usedPct: number;
  windowSeconds: number | null;
  resetsAt: string | null;
}

export interface ClinePassReading {
  windows: ClinePassWindow[];
}

/**
 * Parse one ClinePass dashboard payload (fail-closed): malformed
 * individual windows are skipped like the source provider does, but
 * a payload whose envelope drifts, or that yields no usable window
 * at all, throws AdapterError("unparseable").
 */
export function parseClinePassPayload(payload: unknown): ClinePassReading {
  if (!isRecord(payload)) {
    throw new AdapterError("unparseable", "cline-pass: payload is not an object");
  }
  const data = isRecord(payload.data) ? payload.data : null;
  if (!data || !Array.isArray(data.limits)) {
    throw new AdapterError("unparseable", "cline-pass: payload.data.limits missing or not an array");
  }
  const byKey = new Map<string, ClinePassWindow>();
  for (const item of data.limits) {
    if (!isRecord(item)) continue;
    const kind = typeof item.type === "string" ? WINDOW_KINDS.get(item.type) : undefined;
    if (!kind) continue;
    const rawPct = percentUsedOf(item.percentUsed);
    if (rawPct === null) continue;
    byKey.set(kind.key, {
      name: kind.key,
      usedPct: clampPercent(rawPct),
      windowSeconds: kind.windowSeconds,
      resetsAt: normalizeResetAt(item.resetsAt),
    });
  }
  if (byKey.size === 0) {
    throw new AdapterError("unparseable", "cline-pass: no quota data in response");
  }
  return { windows: [...byKey.values()] };
}

/** Map a parsed reading onto the quota-current contract. */
export function entryFromClinePassReading(reading: ClinePassReading, measuredAt: string): QuotaCurrentEntry {
  return {
    providerId: CLINEPASS_PROVIDER_ID,
    providerName: CLINEPASS_PROVIDER_NAME,
    serviceId: serviceIdFor(CLINEPASS_PROVIDER_ID),
    state: "ok",
    measuredAt,
    source: CLINEPASS_SOURCE,
    windows: reading.windows.map((w) => ({
      name: w.name,
      value: w.usedPct,
      pctOfLimit: w.usedPct,
      valueLabel: null,
      windowSeconds: w.windowSeconds,
      resetsAt: w.resetsAt,
      measuredAt,
      source: CLINEPASS_SOURCE,
      state: "ok" as const,
    })),
  };
}

/** An error entry for a tick that produced no reading — never zeros. */
export function clinePassErrorEntry(detail: string, measuredAt: string): QuotaCurrentEntry {
  return {
    providerId: CLINEPASS_PROVIDER_ID,
    providerName: CLINEPASS_PROVIDER_NAME,
    serviceId: serviceIdFor(CLINEPASS_PROVIDER_ID),
    state: "error",
    measuredAt,
    source: CLINEPASS_SOURCE,
    error: detail,
    windows: [],
  };
}

export type ClinePassFetch = (
  url: string,
  init: { method: string; headers: Record<string, string>; signal: AbortSignal },
) => Promise<{ status: number; headers?: { get(name: string): string | null }; text(): Promise<string> }>;

/**
 * One dashboard fetch, shaped for the framework's collectAdapter.
 * The token is used only as the Authorization header value; it is
 * never returned, logged, or embedded in any outcome.
 */
export async function fetchClinePassPayload(opts: {
  token: string;
  fetchImpl?: ClinePassFetch;
  timeoutMs?: number;
}): Promise<{ httpStatus: number; location?: string; payload: unknown }> {
  const fetchImpl = opts.fetchImpl ?? (fetch as unknown as ClinePassFetch);
  const res = await fetchImpl(CLINEPASS_USAGE_URL, {
    method: "GET",
    headers: { Authorization: `Bearer ${opts.token}`, Accept: "application/json", "Accept-Encoding": "identity" },
    signal: AbortSignal.timeout(opts.timeoutMs ?? 15_000),
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
}

function retainedEntryOutcome(
  status: AdapterOutcome<ClinePassReading>["status"],
  session: AdapterOutcome<ClinePassReading>["session"],
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
 * Collect one ClinePass reading through the framework: session
 * classification first (401/403 -> session-expired + re-login,
 * previous retained), then the fail-closed parse. On success the
 * value is the quota-current entry; on any failure it is the
 * retained previous entry (or null) — never a fabricated zero.
 */
export async function collectClinePassAdapter(opts: {
  now: Date;
  previous: PreviousValue<QuotaCurrentEntry> | null;
  fetchPayload: () => Promise<{ httpStatus: number; location?: string; payload: unknown }>;
}): Promise<AdapterOutcome<QuotaCurrentEntry>> {
  const outcome = await collectAdapter<ClinePassReading>({
    now: opts.now,
    previous: null,
    fetchPayload: opts.fetchPayload,
    parse: parseClinePassPayload,
  });
  if (outcome.status === "ok" && outcome.value) {
    return {
      status: "ok",
      value: entryFromClinePassReading(outcome.value, opts.now.toISOString()),
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
export function credentialRefFromConfig(value: string | undefined): CredentialRef | null {
  if (!value || value.trim() === "") return null;
  const v = value.trim();
  if (v.startsWith("bitwarden:"))
    return validateCredentialRef({ kind: "bitwarden", name: v.slice("bitwarden:".length) });
  if (v.startsWith("file:")) return validateCredentialRef({ kind: "file", path: v.slice("file:".length), mode: 0o600 });
  throw new AdapterError(
    "fetch-failed",
    "cline-pass credential config must be bitwarden:<item name> or file:<path> — a reference, never a credential value",
  );
}

/**
 * The gatherer's ClinePass collection: validate the reference,
 * resolve its value through the injected resolver (the value lives
 * only inside the fetch call), and collect fail-closed. An
 * unresolvable reference is a fetch failure naming the reference —
 * never a zero, never a value leak.
 */
export async function gatherClinePass(opts: {
  now: Date;
  previous: PreviousValue<QuotaCurrentEntry> | null;
  credentialRef: CredentialRef;
  resolveCredentialValue: (ref: CredentialRef) => Promise<string | null>;
  fetchImpl?: ClinePassFetch;
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
  let token: string | null;
  try {
    token = await opts.resolveCredentialValue(ref);
  } catch {
    token = null; // resolver plumbing failed — fail soft, no details (they may name paths)
  }
  if (!token) {
    return retainedEntryOutcome(
      "fetch-failed",
      "unknown",
      `cline-pass credential unavailable (${credentialRefLabel(ref)})`,
      opts.previous,
      opts.now,
    );
  }
  return collectClinePassAdapter({
    now: opts.now,
    previous: opts.previous,
    fetchPayload: () =>
      fetchClinePassPayload({ token: token as string, fetchImpl: opts.fetchImpl, timeoutMs: opts.timeoutMs }),
  });
}
