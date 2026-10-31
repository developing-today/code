// validate.ts — the shared ingest validation primitives (#2576 Phase 1,
// design: code-aggregator-ingest-contract.md).
//
// Every document the aggregator ingests is validated once, at its
// ingest boundary, before any consumer, store append, or render sees
// it. This module is the one place those checks live, so boundaries
// stop re-deriving them privately and disagreeing. It has no
// dependencies beyond the language.
//
// The contract these primitives serve (README, "Ingest contract"):
// a validation failure produces exactly one of three sanctioned
// outcomes — reject-and-retain, drop-the-element-and-count, or
// refuse-the-append — and never one of the three forbidden ones:
// throwing away valid siblings, fabricating rows/ids/zeros, or
// publishing an out-of-range value as live data. A genuine zero is a
// successful validation whose value is 0 (#1153's wording): nothing
// here treats 0 as missing.

/** A plain object (not null, not an array) — the shape every ingested
 * document and element is expected to have before its fields are read. */
export function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

/** A string with at least one character — ids, names, keys. */
export function isNonEmptyString(v: unknown): v is string {
  return typeof v === "string" && v.length > 0;
}

/** An array whose every element is a string. */
export function isStringArray(v: unknown): v is string[] {
  return Array.isArray(v) && v.every((e) => typeof e === "string");
}

/** A real number: finite, not NaN — the check `Number.isFinite` sites
 * used to re-derive privately (27 scattered call sites at design time). */
export function isFiniteNumber(v: unknown): v is number {
  return typeof v === "number" && Number.isFinite(v);
}

/** A finite number that is >= 0. Counts and amounts: a negative is
 * malformed input, never live data (#2553). Zero passes — a genuine
 * zero is a measurement, not an absence. */
export function isNonNegative(v: unknown): v is number {
  return isFiniteNumber(v) && v >= 0;
}

/** A percent of the limit the document itself carries: finite, >= 0,
 * and <= limit (the #2571 `limit: 100` case — a window at -20% or 1e9%
 * is malformed, not a measurement). Curried so a boundary builds the
 * guard for its document's limit once. */
export function isPercentOf(limit: number): (v: unknown) => boolean {
  return (v: unknown) => isFiniteNumber(v) && v >= 0 && v <= limit;
}

const ISO_DATE_RE =
  /^(\d{4})-(\d{2})-(\d{2})(?:[Tt ](\d{2}):(\d{2})(?::(\d{2})(?:\.(\d{1,9}))?)?(?:([Zz])|([+-])(\d{2}):?(\d{2}))?)?$/;

function daysInMonth(year: number, month: number): number {
  // month is 1-based here; day 0 of the next month is this month's last.
  return new Date(Date.UTC(year, month, 0)).getUTCDate();
}

/** Parse an ISO-8601 date or timestamp into a Date, or return null.
 *
 * Strict where `Date.parse` is not: components are extracted and
 * re-checked against the real calendar, so impossible dates —
 * 2026-02-30, 2026-13-01 — fail instead of rolling over into a
 * phantom March date (#2264's defect shape). A missing timezone on a
 * timestamp is read as UTC, matching how the fleet's producers stamp
 * `generatedAt`. Non-strings, non-ISO shapes, and out-of-range time
 * components all return null. */
export function parseIsoDate(v: unknown): Date | null {
  if (typeof v !== "string") return null;
  const m = ISO_DATE_RE.exec(v.trim());
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
  const ms = m[7] === undefined ? 0 : Number((m[7] + "000").slice(0, 3));
  let t = Date.UTC(year, month - 1, day, hour, minute, second, ms);
  if (m[9] !== undefined) {
    // An explicit ±hh:mm offset: shift the wall-clock components to UTC.
    const offH = Number(m[10]);
    const offM = Number(m[11]);
    if (offH > 23 || offM > 59) return null;
    const off = (offH * 60 + offM) * 60_000;
    t += m[9] === "+" ? -off : off;
  }
  const d = new Date(t);
  // Belt and braces: the constructed date's UTC components (before any
  // offset shift, which the regex-bounded components already make
  // safe) must be the components that were asked for.
  if (m[9] === undefined) {
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

/** A string holding a real calendar date (see parseIsoDate). */
export function isIsoDate(v: unknown): v is string {
  return parseIsoDate(v) !== null;
}

/** The default future-skew bound for generatedAt-style stamps:
 * five minutes. */
export const DEFAULT_FUTURE_SKEW_MS = 5 * 60_000;

/** A timestamp that is a real date and not implausibly in the future:
 * a `generatedAt` more than the skew bound ahead of now is not
 * "maximally fresh", it is invalid (#2242's defect shape). Curried so
 * a boundary can pin its own bound; `now` is injectable for tests. */
export function isTimestampNotFuture(
  boundMs: number = DEFAULT_FUTURE_SKEW_MS,
): (v: unknown, now?: Date) => boolean {
  return (v: unknown, now: Date = new Date()) => {
    const d = parseIsoDate(v);
    return d !== null && d.getTime() <= now.getTime() + boundMs;
  };
}

/** The result of validating a container's elements: the values that
 * passed the guard, and how many were dropped. The count is part of
 * the result so the drop is visible, never silent (the contract's
 * "drop the element, count it" outcome). */
export interface ArrayOfResult<T> {
  values: T[];
  dropped: number;
}

/** Validate a container with an element guard: returns the passing
 * elements and the drop count, or null when the container itself is
 * not an array (that is the caller's reject decision, not an empty
 * list — a string `items` must not fabricate an empty session list,
 * #2531). */
export function arrayOf<T>(
  guard: (v: unknown) => v is T,
): (v: unknown) => ArrayOfResult<T> | null {
  return (v: unknown) => {
    if (!Array.isArray(v)) return null;
    const values: T[] = [];
    let dropped = 0;
    for (const e of v) {
      if (guard(e)) values.push(e);
      else dropped++;
    }
    return { values, dropped };
  };
}
