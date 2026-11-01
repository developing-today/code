// Browser-token adapter framework core (#1153, epic #1047 Stage 2).
//
// The framework every per-provider browser-token adapter (#1159
// ClinePass, #1161 OpenCode Go, #1163 Claude, #1165 Command Code,
// #1301 Meta) builds on. It generalizes the as-built pattern of
// pm-stack/meta-exporter/meta-quota-export.mjs (#1301 Phase 1):
//
//   - Credentials are referenced, never held: a CredentialRef names
//     a Bitwarden item or a 0600 file. No token/cookie value ever
//     passes through this module, a repo file, or a log line — the
//     exporter pattern evaluates requests inside the browser's own
//     session context, and adapters that do read a credential read
//     it through the reference at run time on the host that owns it.
//   - Parsing is fail-closed: a dashboard payload whose shape drifts
//     raises AdapterError("unparseable"); the previous value is
//     retained and ages — it never becomes a zero.
//   - Session health is its own signal: an expired session (401/403,
//     or a bounce to an auth page) is "session-expired" and raises a
//     re-login need — it is never reported as zero usage. A genuine
//     zero-usage reading is a successful parse of a valid payload
//     whose values happen to be 0, and only that.

import type { QuotaCurrentEntry } from "./model.ts";
import { entryFromFileExport, type FileExportDoc } from "./quota-gather-map.ts";
import { isRecord } from "./validate.ts";

export type AdapterFailureStatus = "session-expired" | "unparseable" | "transport-unavailable" | "fetch-failed";
export type AdapterStatus = "ok" | AdapterFailureStatus;

/** An adapter failure with the framework's status classification. */
export class AdapterError extends Error {
  status: AdapterFailureStatus;
  constructor(status: AdapterFailureStatus, detail: string) {
    super(detail);
    this.status = status;
  }
}

// ---------- credential contract (references only, never values) ----------

/**
 * A reference to a managed credential. Exactly two kinds exist:
 * a Bitwarden Secrets item named (never valued) here, or a file on
 * the owning host that must be mode 0600. Any other shape — above
 * all, any object carrying a credential VALUE — is a contract
 * violation and is rejected before an adapter runs.
 */
export type CredentialRef = { kind: "bitwarden"; name: string } | { kind: "file"; path: string; mode: 0o600 };

const VALUE_BEARING_KEYS = ["token", "cookie", "value", "secret", "password", "apiKey", "api_key", "authorization"];

/** Validate a credential reference; throws AdapterError on violation. */
export function validateCredentialRef(ref: unknown): CredentialRef {
  if (!ref || typeof ref !== "object") {
    throw new AdapterError("fetch-failed", "credential ref: not an object");
  }
  const record = ref as Record<string, unknown>;
  for (const key of VALUE_BEARING_KEYS) {
    if (key in record) {
      throw new AdapterError(
        "fetch-failed",
        `credential ref: carries a credential value under "${key}" — references only, never values`,
      );
    }
  }
  if (record.kind === "bitwarden") {
    if (typeof record.name !== "string" || record.name.trim() === "") {
      throw new AdapterError("fetch-failed", "credential ref: bitwarden ref needs a non-empty item name");
    }
    return { kind: "bitwarden", name: record.name };
  }
  if (record.kind === "file") {
    if (typeof record.path !== "string" || record.path.trim() === "") {
      throw new AdapterError("fetch-failed", "credential ref: file ref needs a non-empty path");
    }
    if (record.mode !== 0o600) {
      throw new AdapterError("fetch-failed", "credential ref: file ref must declare mode 0600 (owner-only)");
    }
    return { kind: "file", path: record.path, mode: 0o600 };
  }
  throw new AdapterError("fetch-failed", `credential ref: unknown kind ${String(record.kind)}`);
}

/** A log-safe label for a credential reference (never a value). */
export function credentialRefLabel(ref: CredentialRef): string {
  return ref.kind === "bitwarden" ? `bitwarden:${ref.name}` : `file:${ref.path} (0600)`;
}

// ---------- session health ----------

export type SessionHealth = "ok" | "expired" | "unknown";

/**
 * Classify one dashboard-API probe. Expired is raised only on
 * positive evidence (401/403, or a redirect to an auth page); a 200
 * is ok; anything else is unknown — never silently ok, never
 * silently expired.
 */
export function classifyHttpProbe(probe: { httpStatus: number; location?: string }): SessionHealth {
  if (probe.httpStatus === 401 || probe.httpStatus === 403) return "expired";
  if (
    probe.httpStatus >= 300 &&
    probe.httpStatus < 400 &&
    probe.location &&
    /\/(login|auth|signin|sign-in)(\/|\?|$)/i.test(probe.location)
  ) {
    return "expired";
  }
  if (probe.httpStatus === 200) return "ok";
  return "unknown";
}

// ---------- fail-closed collection ----------

/** The last good value an adapter produced, with when it was measured. */
export interface PreviousValue<T> {
  value: T;
  measuredAt: string;
}

export interface AdapterOutcome<T> {
  status: AdapterStatus;
  /** The fresh value on ok; the RETAINED previous value on failure; null when there is neither. Never a fabricated zero. */
  value: T | null;
  /** True when `value` is the retained previous value, aging. */
  stale: boolean;
  /** Age of the retained value in seconds; null on a fresh ok or when no previous exists. */
  ageSeconds: number | null;
  session: SessionHealth;
  /** True only for session-expired: a human re-login is needed (a user/owned item), distinct from any usage reading. */
  needsRelogin: boolean;
  detail?: string;
}

function retainedOutcome<T>(
  status: AdapterFailureStatus,
  previous: PreviousValue<T> | null,
  now: Date,
  session: SessionHealth,
  detail: string,
): AdapterOutcome<T> {
  return {
    status,
    value: previous?.value ?? null,
    stale: previous != null,
    ageSeconds: previous ? Math.max(0, Math.round((now.getTime() - Date.parse(previous.measuredAt)) / 1000)) : null,
    session,
    needsRelogin: status === "session-expired",
    detail,
  };
}

/**
 * Run one adapter collection, fail-closed:
 *   fetch -> session classification -> parse -> outcome.
 * `parse` must throw AdapterError("unparseable", ...) on any shape
 * drift (the meta-exporter parser discipline); a parse that returns
 * is the only path to a fresh value, including a genuine zero.
 */
export async function collectAdapter<T>(opts: {
  now: Date;
  previous: PreviousValue<T> | null;
  fetchPayload: () => Promise<{ httpStatus: number; location?: string; payload: unknown }>;
  parse: (payload: unknown) => T;
}): Promise<AdapterOutcome<T>> {
  let response: { httpStatus: number; location?: string; payload: unknown };
  try {
    response = await opts.fetchPayload();
  } catch (err) {
    return retainedOutcome(
      "fetch-failed",
      opts.previous,
      opts.now,
      "unknown",
      err instanceof Error ? err.message : String(err),
    );
  }
  const session = classifyHttpProbe(response);
  if (session === "expired") {
    return retainedOutcome(
      "session-expired",
      opts.previous,
      opts.now,
      session,
      `dashboard answered HTTP ${response.httpStatus} — session expired, re-login needed`,
    );
  }
  if (response.httpStatus !== 200) {
    return retainedOutcome(
      "fetch-failed",
      opts.previous,
      opts.now,
      session,
      `dashboard answered HTTP ${response.httpStatus}`,
    );
  }
  try {
    const value = opts.parse(response.payload);
    return { status: "ok", value, stale: false, ageSeconds: null, session: "ok", needsRelogin: false };
  } catch (err) {
    if (err instanceof AdapterError) {
      return retainedOutcome(err.status, opts.previous, opts.now, session, err.message);
    }
    return retainedOutcome(
      "unparseable",
      opts.previous,
      opts.now,
      session,
      err instanceof Error ? err.message : String(err),
    );
  }
}

// ---------- the file-export source, as the framework's first adapter ----------

/**
 * The file-export adapter: a producer on the owning host writes
 * latest.json documents (the #1151 convention); this adapter reads
 * one document (injected, so tests need no filesystem) and produces
 * a framework outcome whose value is the quota-current entry from
 * the existing #1151 mapping (entryFromFileExport — the mapping is
 * reused, not rebuilt).
 *
 * Stricter than the bare mapping on one point, by design: the
 * mapping skips malformed windows, but an ADAPTER must treat shape
 * drift as unparseable — so a success document whose windows are
 * missing or malformed fails closed here and the previous entry is
 * retained instead of a partial one being published.
 */
export function collectFileExportAdapter(opts: {
  now: Date;
  previous: PreviousValue<QuotaCurrentEntry> | null;
  doc: FileExportDoc | null;
  fileMtimeIso: string | null;
  stale: boolean;
}): AdapterOutcome<QuotaCurrentEntry> {
  const { doc, now, previous } = opts;
  if (!doc) {
    return retainedOutcome(
      "fetch-failed",
      previous,
      now,
      "unknown",
      "file-export unreadable (missing or unparseable latest.json)",
    );
  }
  if (opts.stale) {
    return retainedOutcome(
      "transport-unavailable",
      previous,
      now,
      "unknown",
      "file-export stale — producer unreachable (not a zero reading)",
    );
  }
  if (doc.status === "session-expired") {
    return retainedOutcome(
      "session-expired",
      previous,
      now,
      "expired",
      "file-export reports session-expired — re-login needed on the producing host",
    );
  }
  if (doc.status === "unparseable") {
    return retainedOutcome("unparseable", previous, now, "unknown", "file-export reports unparseable payload");
  }
  if (doc.status !== "ok") {
    return retainedOutcome("fetch-failed", previous, now, "unknown", `file-export status: ${doc.status ?? "unknown"}`);
  }
  // Fail-closed window validation (drift => unparseable, never partial).
  if (!Array.isArray(doc.windows)) {
    return retainedOutcome("unparseable", previous, now, "ok", "file-export drift: windows missing or not an array");
  }
  for (const w of doc.windows) {
    if (!isRecord(w)) {
      return retainedOutcome(
        "unparseable",
        previous,
        now,
        "ok",
        "file-export drift: window element is not an object",
      );
    }
    if (typeof w.name !== "string" || w.name.length === 0 || typeof w.usedPct !== "number" || !Number.isFinite(w.usedPct)) {
      return retainedOutcome(
        "unparseable",
        previous,
        now,
        "ok",
        `file-export drift: window ${String(w.name)} lacks a name or a finite usedPct`,
      );
    }
  }
  const entry = entryFromFileExport(doc, { fileMtimeIso: opts.fileMtimeIso, stale: false });
  if (entry.state !== "ok") {
    return retainedOutcome(
      "unparseable",
      previous,
      now,
      "ok",
      entry.error ?? "file-export mapping rejected the document",
    );
  }
  return { status: "ok", value: entry, stale: false, ageSeconds: null, session: "ok", needsRelogin: false };
}
