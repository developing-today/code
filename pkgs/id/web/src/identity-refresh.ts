/** Parsing for the identity-token refresh timestamp stored in localStorage. */

/**
 * Decide whether the identity token needs refreshing from the raw stored
 * `id_identity_refreshed_at` value. localStorage is untrusted, user-writable
 * state: a non-numeric or non-finite value parses to `NaN`/`Infinity`, and a
 * bare `Date.now() - Number(raw) >= interval` comparison against `NaN` is
 * always `false`, which suppressed refresh forever (#847). A timestamp in
 * the future is likewise untrustworthy — it would suppress refresh until
 * wall-clock time catches up. Both cases count as "needs refresh", as does
 * a missing value. Only a finite, past timestamp is compared against the
 * interval.
 */
export function identityNeedsRefresh(refreshedAt: string | null, now: number, intervalMs: number): boolean {
  if (!refreshedAt) return true;
  const parsed = Number(refreshedAt);
  if (!Number.isFinite(parsed)) return true;
  const elapsed = now - parsed;
  if (elapsed < 0) return true;
  return elapsed >= intervalMs;
}
