/** #2732 B2a: match raw quota-current evidence without publishing or seating.
 * The receipt must come from a trusted per-window measurement path. A gatherer
 * row can stamp its tick time when the provider supplied no observation time;
 * neither that tick nor the display/snapshot pool series is admission evidence.
 * Call on the selected current document. A rejection leaves that window unknown;
 * do not replace it with an older file or drop already carried stops/walls.
 */
import type { Observation, Scope } from "./quota-admission.ts";
import { isFiniteNumber, isNonEmptyString, isPercentOf, isRecord, parseIsoDate } from "./validate.ts";

export interface QuotaWindowReceipt extends Scope {
  /** Exact producer identity, deliberately separate from billing identity. */
  sourceProviderId: string;
  /** Stable non-secret credential alias recorded by the producer, not a key. */
  sourceCredentialId: string;
  sourceWindowName: string;
  source: string;
  kind: string;
  period: string;
  resetsAt: number;
  measuredAt: number;
  percent: number;
  /** Provider-side stable evidence id and allowance identity, not tick ids. */
  id: string;
  denominator: string;
  windowSeconds: number;
}

const epoch = (v: unknown) =>
  typeof v === "string" && /T.+(?:Z|[+-][0-9]{2}:[0-9]{2})$/.test(v) ? parseIsoDate(v)?.getTime() : undefined;
const percent = isPercentOf(100);

export function normalizeQuotaWindowEvidence(
  document: unknown,
  receipt: QuotaWindowReceipt,
  nowMs: number,
): { observation: Observation; reason: null } | { observation: null; reason: string } {
  const reject = (reason: string) => ({ observation: null, reason });
  if (
    ![
      receipt.provider,
      receipt.account,
      receipt.bucket,
      receipt.sourceProviderId,
      receipt.sourceCredentialId,
      receipt.sourceWindowName,
      receipt.source,
      receipt.kind,
      receipt.period,
      receipt.id,
      receipt.denominator,
    ].every(isNonEmptyString) ||
    ![nowMs, receipt.measuredAt, receipt.resetsAt, receipt.windowSeconds].every(isFiniteNumber) ||
    receipt.windowSeconds <= 0 ||
    receipt.measuredAt > nowMs ||
    receipt.measuredAt >= receipt.resetsAt ||
    !percent(receipt.percent)
  )
    return reject("invalid_receipt");

  if (!isRecord(document) || document.version !== 1 || !Array.isArray(document.providers)) {
    return reject("invalid_document");
  }
  // A sanitized file cannot identify the scopes of removed rows. Do not let
  // silent drops turn conflicting source evidence into apparent consensus.
  if (
    document.sanitized !== undefined &&
    (!isRecord(document.sanitized) ||
      document.sanitized.droppedProviders !== 0 ||
      document.sanitized.droppedWindows !== 0)
  ) {
    return reject("incomplete_document");
  }

  let found = false;
  for (const entry of document.providers) {
    if (!isRecord(entry) || !isNonEmptyString(entry.providerId)) return reject("invalid_provider_row");
    if (entry.providerId !== receipt.sourceProviderId || entry.credentialId !== receipt.sourceCredentialId) continue;
    // Rows with the same exact credential do not get a newer/healthiest-row
    // tie-break. A retained/error/lapsed duplicate blocks positive evidence.
    if (
      entry.state !== "ok" ||
      entry.retainedFromFailedTick !== undefined ||
      entry.source !== receipt.source ||
      !Array.isArray(entry.windows)
    )
      return reject("unavailable_provider_read");
    const matches: Record<string, unknown>[] = [];
    for (const w of entry.windows) {
      if (!isRecord(w) || !isNonEmptyString(w.name)) return reject("invalid_window_row");
      if (w.name === receipt.sourceWindowName) matches.push(w);
    }
    // Display-only split rows can omit a window, but cannot establish it.
    for (const w of matches) {
      if (
        w.state !== "ok" ||
        w.source !== receipt.source ||
        epoch(w.measuredAt) !== receipt.measuredAt ||
        epoch(w.resetsAt) !== receipt.resetsAt ||
        w.windowSeconds !== receipt.windowSeconds ||
        w.pctOfLimit !== receipt.percent
      ) {
        return reject("conflicting_window_evidence");
      }
      found = true;
    }
  }
  if (!found) return reject("missing_window_evidence");
  return {
    reason: null,
    observation: {
      provider: receipt.provider,
      account: receipt.account,
      bucket: receipt.bucket,
      kind: receipt.kind,
      period: receipt.period,
      id: receipt.id,
      source: receipt.source,
      measuredAt: receipt.measuredAt,
      resetsAt: receipt.resetsAt,
      percent: receipt.percent,
      state: "ok",
      denominator: receipt.denominator,
    },
  };
}
