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

// ============================================================================
// Identity endpoint response validation (#7839)
// ============================================================================

/** Validated `/api/identity/me` response: a rotated token and/or the current display name. */
export interface IdentityMeResponse {
  token: string | null;
  name: string | null;
}

/** Validated `/api/identity/register` response: the newly issued identity. */
export interface IdentityRegisterResponse {
  token: string;
  clientId: string;
  name: string | null;
}

/** Validated `/api/identity/name` response: the stored display name. */
export interface IdentityNameResponse {
  name: string | null;
}

function asIdentityRecord(payload: unknown, endpoint: string): Record<string, unknown> {
  if (typeof payload !== "object" || payload === null || Array.isArray(payload)) {
    throw new Error(`malformed ${endpoint} response (expected an object)`);
  }
  return payload as Record<string, unknown>;
}

function identityNameField(value: unknown, endpoint: string): string | null {
  if (value === undefined || value === null) return null;
  if (typeof value !== "string") {
    throw new Error(`malformed ${endpoint} response (name must be a string or null)`);
  }
  return value;
}

function requiredIdentityString(value: unknown, field: string, endpoint: string): string {
  if (typeof value !== "string" || value.length === 0) {
    throw new Error(`malformed ${endpoint} response (${field} must be a non-empty string)`);
  }
  return value;
}

/**
 * Validate a `/api/identity/me` payload at the fetch boundary (#7839).
 *
 * The token is optional (present only when the server rotates it) and the
 * name is optional, but either one, when present, must be a non-empty
 * string / a string-or-null. Anything else throws, so the caller's error
 * path leaves the stored identity untouched: without this check a
 * wrong-shape 200 overwrote a valid stored token with the coerced string
 * "[object Object]".
 */
export function parseIdentityMeResponse(payload: unknown): IdentityMeResponse {
  const record = asIdentityRecord(payload, "/api/identity/me");
  const token =
    record.token === undefined ? null : requiredIdentityString(record.token, "token", "/api/identity/me");
  return { token, name: identityNameField(record.name, "/api/identity/me") };
}

/**
 * Validate a `/api/identity/register` payload at the fetch boundary
 * (#7839). Both `token` and `client_id` are required non-empty strings:
 * anything else throws before the caller can persist a fake identity
 * (the unvalidated cast stored the literal strings "undefined" /
 * "[object Object]" in localStorage as token/client_id).
 */
export function parseIdentityRegisterResponse(payload: unknown): IdentityRegisterResponse {
  const record = asIdentityRecord(payload, "/api/identity/register");
  return {
    token: requiredIdentityString(record.token, "token", "/api/identity/register"),
    clientId: requiredIdentityString(record.client_id, "client_id", "/api/identity/register"),
    name: identityNameField(record.name, "/api/identity/register"),
  };
}

/**
 * Validate a `/api/identity/name` payload at the fetch boundary (#7839).
 * The response's only field is the stored display name, so it must be
 * present and a string or null; anything else throws and the caller
 * leaves the stored name unchanged.
 */
export function parseIdentityNameResponse(payload: unknown): IdentityNameResponse {
  const record = asIdentityRecord(payload, "/api/identity/name");
  if (!("name" in record)) {
    throw new Error("malformed /api/identity/name response (name must be a string or null)");
  }
  return { name: identityNameField(record.name, "/api/identity/name") };
}
