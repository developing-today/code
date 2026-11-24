/** E4: short-lived, per-database credentials (hosted-tenancy design §7).
 * The caller resolves a live session before calling this service; the
 * signer is the only seam that handles the opaque token value. */

export type CredentialScope = "read" | "readwrite";

export interface CredentialSql {
  exec(query: string, ...params: (string | null)[]): { toArray(): Record<string, unknown>[] };
}

export interface CredentialSession {
  accountId: string;
  bindingId: string;
  /** Set only by the authenticated session resolver. */
  live: true;
}

export interface TenantTokenIssuer {
  issue(input: { audience: string; scope: CredentialScope; ttlSeconds: number; jti: string }): Promise<string>;
  revoke?(jti: string): Promise<void>;
}

export interface CredentialDeps {
  now?: () => Date;
  newId?: () => string;
  ttlSeconds?: number;
  issuer: TenantTokenIssuer;
}

export type CredentialResult =
  | { ok: true; token: string; credentialId: string; audience: string; scope: CredentialScope; expiresAt: string }
  | { ok: false; reason: "session_not_live" | "account_not_active" | "binding_not_active" | "tenant_not_active" | "not_owner" | "invalid_scope" };

const MAX_TTL_SECONDS = 15 * 60;

function one(sql: CredentialSql, query: string, ...params: (string | null)[]) {
  return sql.exec(query, ...params).toArray()[0] ?? null;
}

/** Issue only to the session's own active tenant. Refusal order follows §7. */
export async function issueCredential(
  sql: CredentialSql,
  session: CredentialSession,
  scope: string,
  deps: CredentialDeps,
): Promise<CredentialResult> {
  if (!session || session.live !== true) return { ok: false, reason: "session_not_live" };
  const account = one(sql, "SELECT state FROM account WHERE account_id = ?", session.accountId);
  if (account?.state !== "active") return { ok: false, reason: "account_not_active" };
  const binding = one(sql, "SELECT account_id, status FROM binding WHERE binding_id = ?", session.bindingId);
  if (binding?.status !== "active") return { ok: false, reason: "binding_not_active" };
  if (binding.account_id !== session.accountId) return { ok: false, reason: "not_owner" };
  const tenant = one(sql, "SELECT state, turso_db_id FROM tenant_db WHERE tenant_id = ?", session.bindingId);
  if (tenant?.state !== "active" || typeof tenant.turso_db_id !== "string" || !tenant.turso_db_id)
    return { ok: false, reason: "tenant_not_active" };
  if (scope !== "read" && scope !== "readwrite") return { ok: false, reason: "invalid_scope" };

  const ttlSeconds = Math.max(1, Math.min(deps.ttlSeconds ?? MAX_TTL_SECONDS, MAX_TTL_SECONDS));
  const issuedAt = (deps.now ?? (() => new Date()))();
  const expiresAt = new Date(issuedAt.getTime() + ttlSeconds * 1000).toISOString();
  const credentialId = (deps.newId ?? (() => crypto.randomUUID()))();
  const jti = (deps.newId ?? (() => crypto.randomUUID()))();
  const token = await deps.issuer.issue({ audience: tenant.turso_db_id, scope, ttlSeconds, jti });
  try {
    sql.exec(
      "INSERT INTO credential (credential_id, tenant_id, audience, scope, issued_at, expires_at, revoked_at, issuer, jti) VALUES (?, ?, ?, ?, ?, ?, NULL, 'tenancy-worker', ?)",
      credentialId, session.bindingId, tenant.turso_db_id, scope, issuedAt.toISOString(), expiresAt, jti,
    );
  } catch (error) {
    // Do not leave a usable token behind when metadata could not be recorded.
    await deps.issuer.revoke?.(jti);
    throw error;
  }
  return { ok: true, token, credentialId, audience: tenant.turso_db_id, scope, expiresAt };
}

/** Revoke one credential by its metadata handle; token values are never stored. */
export async function revokeCredential(
  sql: CredentialSql,
  credentialId: string,
  issuer?: Pick<TenantTokenIssuer, "revoke">,
  now = new Date(),
): Promise<boolean> {
  const row = one(sql, "SELECT jti, revoked_at FROM credential WHERE credential_id = ?", credentialId);
  if (!row) return false;
  if (row.revoked_at !== null) return true;
  await issuer?.revoke?.(row.jti as string);
  sql.exec("UPDATE credential SET revoked_at = ? WHERE credential_id = ? AND revoked_at IS NULL", now.toISOString(), credentialId);
  return true;
}
