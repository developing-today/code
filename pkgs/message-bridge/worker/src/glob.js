// One matching rule for bridge addressing (design
// doc/2026-11-24_design_message-bridge-buildout.md §4.1): a glob is a
// name segment in which "*" matches any run of segment characters
// (including none); every other character matches itself. Matching is
// case-insensitive — callers lower-case names and patterns first, as
// every bridge surface already does — and resolution enumerates
// matches in deterministic name order, so a limit always picks the
// same first N.

/** A valid glob segment: name characters plus "*", 1-64 long, starting with a letter, digit or "*". */
export const GLOB_SEGMENT = /^[a-z0-9*][a-z0-9._*-]{0,63}$/;

/** True when the segment is a glob (contains "*") rather than an exact name. */
export const isGlob = (s) => typeof s === "string" && s.includes("*");

const escape = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

/** True when `value` matches the glob `pattern` (both already lower-cased). */
export function globMatch(pattern, value) {
  if (!isGlob(pattern)) return pattern === value;
  const re = new RegExp(`^${pattern.split("*").map(escape).join(".*")}$`);
  return re.test(value);
}
