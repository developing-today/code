/** Parsing for the world install/compile seed field. */

/**
 * Parse the raw seed input value into a non-negative safe integer seed.
 * Returns null for anything the admin did not actually choose — including an
 * empty or whitespace-only field, which `Number("")` would silently turn
 * into seed 0 (#707). Only a run of decimal digits counts: `Number()` alone
 * would also reinterpret hex/binary/octal/scientific forms the field never
 * showed as that value ("0x10" -> 16, "1e3" -> 1000) (#832). An explicitly
 * typed "0" is a valid seed.
 */
export function parseWorldSeed(raw: string): number | null {
  const trimmed = raw.trim();
  if (!/^\d+$/.test(trimmed)) return null;
  const seed = Number(trimmed);
  return Number.isSafeInteger(seed) && seed >= 0 ? seed : null;
}
