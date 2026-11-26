/**
 * Agent credential metadata store, slice H4 (#14262, child of #9086,
 * design §4.5).
 *
 * Home: the CENTRAL tenancy DB, beside E's `credential` table — this
 * is control-plane metadata, never tenant content, so it is NOT a
 * pm tenant-store migration (the tenant plane holds no credential
 * rows at all; see the tenant-store suite's E §2.3 column scan).
 *
 * Metadata only: a row carries `jti`, the owning
 * `account_id`/`binding_id`, the single `tenant_id` scope, the
 * audience, lifetime, revocation stamp, and a SHA-256 *digest* of
 * the issued secret so the client-credentials exchange can verify a
 * presented secret. The secret itself is never stored — it is shown
 * exactly once at issuance (see `./agent-credentials.ts`) and no
 * code path here ever receives it. A digest is not a bearer secret:
 * presenting the digest does not authenticate (the exchange hashes
 * what is presented and compares digests).
 *
 * Reads are live: `getByJti` is called on every front-door request
 * and every exchange, never cached, so revocation by `jti` takes
 * effect within one request (design §4.5).
 */
import type { AgentCredentialMetadata } from "./agent-front-door.ts";
import type { SqlDb } from "./tenant-store/migrate.ts";

export interface StoredAgentCredential extends AgentCredentialMetadata {
  /** Hex SHA-256 of the issued secret. Never the secret itself. */
  secretDigest: string;
}

export interface AgentCredentialStore {
  put(record: StoredAgentCredential): Promise<void>;
  getByJti(jti: string): Promise<StoredAgentCredential | null>;
  /** Stamp `revokedAt`; returns false when no such live credential. */
  revoke(jti: string, revokedAt: number): Promise<boolean>;
}

/** In-memory store for tests and local tooling. */
export class InMemoryAgentCredentialStore implements AgentCredentialStore {
  readonly records = new Map<string, StoredAgentCredential>();

  async put(record: StoredAgentCredential): Promise<void> {
    this.records.set(record.jti, { ...record });
  }

  async getByJti(jti: string): Promise<StoredAgentCredential | null> {
    const record = this.records.get(jti);
    return record === undefined ? null : { ...record };
  }

  async revoke(jti: string, revokedAt: number): Promise<boolean> {
    const record = this.records.get(jti);
    if (record === undefined || record.revokedAt !== null) return false;
    this.records.set(jti, { ...record, revokedAt });
    return true;
  }
}

/**
 * DDL for the central tenancy DB (component E plane). Kept as a
 * string constant here, not a tenant migration file, so the tenant
 * store's "no credential columns" invariant is untouched.
 */
export const AGENT_CREDENTIAL_DDL = `CREATE TABLE IF NOT EXISTS agent_credential (
  jti TEXT PRIMARY KEY CHECK (trim(jti) <> ''),
  account_id TEXT NOT NULL CHECK (trim(account_id) <> ''),
  binding_id TEXT NOT NULL CHECK (trim(binding_id) <> ''),
  tenant_id TEXT NOT NULL CHECK (trim(tenant_id) <> '' AND tenant_id <> '*'),
  audience TEXT NOT NULL CHECK (trim(audience) <> ''),
  role TEXT NOT NULL CHECK (role = 'agent'),
  issued_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL CHECK (expires_at > issued_at),
  revoked_at INTEGER,
  secret_digest TEXT NOT NULL CHECK (length(secret_digest) = 64)
)`;

interface AgentCredentialRow {
  jti: string;
  account_id: string;
  binding_id: string;
  tenant_id: string;
  audience: string;
  role: string;
  issued_at: number;
  expires_at: number;
  revoked_at: number | null;
  secret_digest: string;
}

function rowToRecord(row: AgentCredentialRow): StoredAgentCredential {
  return {
    jti: row.jti,
    accountId: row.account_id,
    bindingId: row.binding_id,
    tenantId: row.tenant_id,
    audience: row.audience,
    role: "agent",
    issuedAt: row.issued_at,
    expiresAt: row.expires_at,
    revokedAt: row.revoked_at,
    secretDigest: row.secret_digest,
  };
}

/** SqlDb-backed central store; the caller owns the central DB handle. */
export class SqlAgentCredentialStore implements AgentCredentialStore {
  constructor(private readonly db: SqlDb) {
    this.db.exec(AGENT_CREDENTIAL_DDL);
  }

  async put(record: StoredAgentCredential): Promise<void> {
    this.db.run(
      `INSERT INTO agent_credential
         (jti, account_id, binding_id, tenant_id, audience, role, issued_at, expires_at, revoked_at, secret_digest)
       VALUES (?, ?, ?, ?, ?, 'agent', ?, ?, ?, ?)`,
      [
        record.jti,
        record.accountId,
        record.bindingId,
        record.tenantId,
        record.audience,
        record.issuedAt,
        record.expiresAt,
        record.revokedAt,
        record.secretDigest,
      ],
    );
  }

  async getByJti(jti: string): Promise<StoredAgentCredential | null> {
    const rows = this.db.all<AgentCredentialRow>("SELECT * FROM agent_credential WHERE jti = ?", [jti]);
    const row = rows[0];
    return row === undefined ? null : rowToRecord(row);
  }

  async revoke(jti: string, revokedAt: number): Promise<boolean> {
    const existing = await this.getByJti(jti);
    if (existing === null || existing.revokedAt !== null) return false;
    this.db.run("UPDATE agent_credential SET revoked_at = ? WHERE jti = ?", [revokedAt, jti]);
    return true;
  }
}
