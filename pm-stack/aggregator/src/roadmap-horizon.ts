/** Canonical values accepted by the Grist Program roadmap Horizon field. */
export type RoadmapHorizonValidation =
  | { valid: true; canonical: string; changed: boolean }
  | { valid: false; error: string };

const QUARTER = /^Q([1-4])\s+(\d{4})$/i;
const DATE = /^(\d{4})-(\d{2})-(\d{2})$/;
const BUCKETS = new Map([
  ["now", "Now"],
  ["next", "Next"],
  ["later", "Later"],
  ["ongoing", "Ongoing"],
  ["unscheduled", "Unscheduled"],
]);
const GRAMMAR = "Use YYYY-MM-DD, Q1–Q4 YYYY, Now, Next, Later, Ongoing, or Unscheduled.";

function validDate(year: number, month: number, day: number): boolean {
  if (month < 1 || month > 12 || day < 1) return false;
  const leap = (year % 4 === 0 && year % 100 !== 0) || year % 400 === 0;
  const days = [31, month === 2 && leap ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
  return day <= days[month - 1];
}

/**
 * Validate without rewriting the source value. `canonical` is for grouping
 * and display only; existing Grist cells are never migrated on read.
 */
export function validateRoadmapHorizon(value: unknown): RoadmapHorizonValidation {
  if (typeof value !== "string") return { valid: false, error: `Horizon must be text. ${GRAMMAR}` };
  const trimmed = value.trim();
  if (!trimmed) return { valid: false, error: `Horizon is blank; use Unscheduled for an intentionally undated row. ${GRAMMAR}` };

  const quarter = QUARTER.exec(trimmed);
  if (quarter) {
    const canonical = `Q${quarter[1]} ${quarter[2]}`;
    return { valid: true, canonical, changed: canonical !== value };
  }

  const date = DATE.exec(trimmed);
  if (date && validDate(Number(date[1]), Number(date[2]), Number(date[3]))) {
    return { valid: true, canonical: trimmed, changed: trimmed !== value };
  }

  const bucket = BUCKETS.get(trimmed.toLowerCase());
  if (bucket) return { valid: true, canonical: bucket, changed: bucket !== value };

  return { valid: false, error: `Unrecognized Horizon value. ${GRAMMAR}` };
}
