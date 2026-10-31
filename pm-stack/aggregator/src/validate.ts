// Shared ingest-validation primitives (#2576 Phase 1, #2577).
//
// The ingest contract (README, "Ingest contract"): every document the
// aggregator ingests is validated ONCE, at its boundary — shape,
// element types, value ranges, and real calendar dates — before any
// consumer, store append, or render sees it. These primitives are the
// vocabulary guards are written in; a boundary composes them and
// records which sanctioned outcome a failure produces (reject and
// retain / drop the element and count it / refuse the append). No new
// dependency; every check here is total (never throws on bad input).

/** A plain object record (not null, not an array). */
export function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

/** A string with non-whitespace content (ids, names). */
export function isNonEmptyString(v: unknown): v is string {
  return typeof v === "string" && v.trim().length > 0;
}

/** An array whose every element is a string. */
export function isStringArray(v: unknown): v is string[] {
  return Array.isArray(v) && v.every((e) => typeof e === "string");
}

/** A genuine, finite number — NaN and the infinities are not data. */
export function isFiniteNumber(v: unknown): v is number {
  return typeof v === "number" && Number.isFinite(v);
}

/** A finite number >= 0. A genuine 0 passes as 0 (#1153). */
export function isNonNegative(v: unknown): v is number {
  return isFiniteNumber(v) && v >= 0;
}

/**
 * Guard factory for percents: `isPercentOf(limit)` is a guard for a
 * finite number in [0, limit], where limit is the bound the document
 * itself carries (the #2571 `limit: 100` case). A non-finite limit
 * validates nothing.
 */
export function isPercentOf(limit: unknown): (v: unknown) => v is number {
  return (v: unknown): v is number =>
    isFiniteNumber(v) && isFiniteNumber(limit) && v >= 0 && v <= limit;
}

const ISO_DATE_RE =
  /^(\d{4})-(\d{2})-(\d{2})(?:T(\d{2}):(\d{2})(?::(\d{2})(?:\.(\d{1,9}))?)?(Z|[+-]\d{2}:\d{2})?)?$/;

function isLeapYear(year: number): boolean {
  return (year % 4 === 0 && year % 100 !== 0) || year % 400 === 0;
}

function daysInMonth(year: number, month: number): number {
  return [31, isLeapYear(year) ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][month - 1] ?? 0;
}

/**
 * Parse an ISO-8601 date or datetime string into a Date, or null.
 *
 * Unlike Date.parse, the components are extracted and re-checked, so
 * impossible calendar dates FAIL instead of rolling over: 2026-02-30
 * and 2026-13-01 are null, not March dates. Date-only forms are UTC
 * midnight (the ISO convention); a datetime without an offset is read
 * as UTC, matching the wire shapes the aggregator actually ingests.
 */
export function parseIsoDate(v: unknown): Date | null {
  if (typeof v !== "string") return null;
  const m = ISO_DATE_RE.exec(v);
  if (!m) return null;
  const year = Number(m[1]);
  const month = Number(m[2]);
  const day = Number(m[3]);
  if (month < 1 || month > 12) return null;
  if (day < 1 || day > daysInMonth(year, month)) return null;
  const hour = m[4] === undefined ? 0 : Number(m[4]);
  const minute = m[5] === undefined ? 0 : Number(m[5]);
  const second = m[6] === undefined ? 0 : Number(m[6]);
  if (hour > 23 || minute > 59 || second > 59) return null;
  const fracMs = m[7] === undefined ? 0 : Number((m[7] + "000").slice(0, 3));
  let ms = Date.UTC(year, month - 1, day, hour, minute, second, fracMs);
  const offset = m[8];
  if (offset !== undefined && offset !== "Z") {
    const sign = offset.startsWith("+") ? 1 : -1;
    const offH = Number(offset.slice(1, 3));
    const offM = Number(offset.slice(4, 6));
    if (offH > 23 || offM > 59) return null;
    ms -= sign * (offH * 3_600_000 + offM * 60_000);
  }
  const d = new Date(ms);
  // Re-check: Date.UTC rolls overflow components into neighbouring
  // months/years, so the round trip must reproduce the UTC components
  // exactly for offset-free forms (an explicit offset legitimately
  // shifts the UTC clock reading, so only the instant is checked there
  // — its calendar validity was established above).
  if (offset === undefined || offset === "Z") {
    if (
      d.getUTCFullYear() !== year ||
      d.getUTCMonth() !== month - 1 ||
      d.getUTCDate() !== day ||
      d.getUTCHours() !== hour ||
      d.getUTCMinutes() !== minute ||
      d.getUTCSeconds() !== second
    ) {
      return null;
    }
  }
  return d;
}

/** A string that is a real ISO calendar date (see parseIsoDate). */
export function isIsoDate(v: unknown): v is string {
  return parseIsoDate(v) !== null;
}

/** Default future-skew bound for generatedAt-style stamps: 5 minutes. */
export const MAX_FUTURE_SKEW_MS = 5 * 60_000;

/**
 * A timestamp that is a real date and not implausibly in the future:
 * a generatedAt more than `skewMs` (default 5 minutes) ahead of now is
 * not "maximally fresh", it is invalid (#2242). `nowMs` is injectable
 * for tests.
 */
export function isTimestampNotFuture(
  v: unknown,
  skewMs: number = MAX_FUTURE_SKEW_MS,
  nowMs: number = Date.now(),
): boolean {
  const d = parseIsoDate(v);
  if (d === null) return false;
  return d.getTime() <= nowMs + skewMs;
}

/**
 * Validate a container element-wise: `arrayOf(guard)` returns a
 * function mapping an unknown to { values, dropped } — the elements
 * that pass the guard, plus the COUNT of those dropped (the
 * drop-the-element-and-count-it outcome; the count must be surfaced by
 * the caller, never swallowed). A non-array container yields null:
 * that is a shape failure for the caller's reject path, not an empty
 * success.
 */
export function arrayOf<T>(
  guard: (v: unknown) => v is T,
): (v: unknown) => { values: T[]; dropped: number } | null {
  return (v: unknown) => {
    if (!Array.isArray(v)) return null;
    const values: T[] = [];
    let dropped = 0;
    for (const e of v) {
      if (guard(e)) values.push(e);
      else dropped += 1;
    }
    return { values, dropped };
  };
}
