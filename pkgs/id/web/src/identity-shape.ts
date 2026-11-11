/**
 * Shape validation for the `/api/identity/*` JSON bodies (#7838).
 *
 * `ensureIdentity` and `updateDisplayName` in main.ts persist whatever
 * these endpoints return into localStorage, and the stored token is
 * then trusted as a real identity (validation, collab connection).
 * The bodies are therefore untrusted shape: a proxy, a stale server,
 * or an error page answering 200 with `{}` used to persist the literal
 * string `'undefined'` as the token, a numeric `token` was coerced to
 * a string, a `null` body crashed with a raw TypeError, and a `{}`
 * `/me` body silently wiped the stored display name. Every field is
 * checked here, before anything is saved: a wrong-shape body changes
 * nothing.
 */

function asRecord(data: unknown): Record<string, unknown> | null {
  if (typeof data !== "object" || data === null || Array.isArray(data)) return null;
  return data as Record<string, unknown>;
}

function nonEmptyString(value: unknown): string | null {
  return typeof value === "string" && value.length > 0 ? value : null;
}

/**
 * The `name` field of an identity body: `ok` only when the field is
 * present and is a string or an explicit null (the server saying the
 * identity has no display name). A missing or wrong-typed field is
 * not ok — callers must leave the stored name untouched, never clear
 * it on a wrong-shape body.
 */
export function parseNameBody(
  data: unknown,
): { ok: true; name: string | null } | { ok: false } {
  const rec = asRecord(data);
  if (!rec || !("name" in rec)) return { ok: false };
  if (typeof rec.name === "string") return { ok: true, name: rec.name };
  if (rec.name === null) return { ok: true, name: null };
  return { ok: false };
}

export interface RegisterIdentity {
  token: string;
  clientId: string;
  name: string | null;
}

/**
 * A `/api/identity/register` body is usable only as a non-null object
 * with non-empty string `token` and `client_id`, and a `name` that is
 * a string, an explicit null, or absent (treated as null). Anything
 * else returns null and nothing may be persisted.
 */
export function parseRegisterBody(data: unknown): RegisterIdentity | null {
  const rec = asRecord(data);
  if (!rec) return null;
  const token = nonEmptyString(rec.token);
  const clientId = nonEmptyString(rec.client_id);
  if (!token || !clientId) return null;
  let name: string | null = null;
  if ("name" in rec) {
    if (typeof rec.name === "string") name = rec.name;
    else if (rec.name !== null) return null;
  }
  return { token, clientId, name };
}

export interface MeUpdate {
  /** Refreshed token to apply, or null to keep the stored token. */
  token: string | null;
  /** Whether the body's `name` field is authoritative (see parseNameBody). */
  nameOk: boolean;
  /** The authoritative name when `nameOk` is true. */
  name: string | null;
}

/**
 * A `/api/identity/me` body, as the updates it may make to the stored
 * identity: a token refresh applies only for a non-empty string
 * token, and the name is authoritative only when the field is a
 * string or an explicit null. A non-object body updates nothing.
 */
export function parseMeBody(data: unknown): MeUpdate {
  const rec = asRecord(data);
  if (!rec) return { token: null, nameOk: false, name: null };
  const name = parseNameBody(rec);
  return {
    token: nonEmptyString(rec.token),
    nameOk: name.ok,
    name: name.ok ? name.name : null,
  };
}
