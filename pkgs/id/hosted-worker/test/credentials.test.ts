// E4 (#12799). Oracle: hosted-tenancy design §7 — issuance needs a live
// session and active account, binding and tenant (in that order), tokens
// are per-database and short-lived, and metadata stores no token value.
import { test } from "node:test";
import assert from "node:assert/strict";
import { DatabaseSync } from "node:sqlite";
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { issueCredential, revokeCredential, type CredentialSql } from "../src/credentials.ts";

const migrationsDir = join(dirname(fileURLToPath(import.meta.url)), "..", "tenancy", "migrations");
const binding = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const at = new Date("2026-11-24T00:00:00.000Z");

function setup() {
  const db = new DatabaseSync(":memory:");
  db.exec("PRAGMA foreign_keys = ON");
  for (const file of readdirSync(migrationsDir).filter((f) => f.endsWith(".sql")).sort())
    db.exec(readFileSync(join(migrationsDir, file), "utf8"));
  db.prepare("INSERT INTO account (account_id, sub, created_at, state, tier) VALUES ('a', 'sub', ?, 'active', 'standard')").run(at.toISOString());
  db.prepare("INSERT INTO binding (binding_id, account_id, iroh_id, status, bound_at) VALUES (?, 'a', 'iroh', 'active', ?)").run(binding, at.toISOString());
  db.prepare("INSERT INTO tenant_db (tenant_id, participant_kind, turso_db_id, turso_db_url, jurisdiction, backend, state, schema_version, created_at) VALUES (?, 'user_hosted', 'db-a', 'https://db-a.turso.io', 'us', 'turso', 'active', 1, ?)").run(binding, at.toISOString());
  const sql: CredentialSql = { exec(query, ...params) {
    const stmt = db.prepare(query);
    if (/^\s*select/i.test(query)) return { toArray: () => stmt.all(...params as never[]) as Record<string, unknown>[] };
    stmt.run(...params as never[]); return { toArray: () => [] };
  } };
  const calls: { audience: string; scope: string; ttlSeconds: number; jti: string }[] = [];
  const revoked: string[] = [];
  const deps = { now: () => at, newId: (() => { let n = 0; return () => `id-${++n}`; })(), issuer: {
    async issue(input: typeof calls[number]) { calls.push(input); return "opaque-secret-token"; },
    async revoke(jti: string) { revoked.push(jti); },
  } };
  return { db, sql, deps, calls, revoked };
}

const session = { accountId: "a", bindingId: binding, live: true as const };

test("issues a short-lived token scoped to the active tenant DB and stores metadata, not token", async () => {
  const { db, sql, deps, calls } = setup();
  const result = await issueCredential(sql, session, "readwrite", deps);
  assert.equal(result.ok, true);
  if (!result.ok) return;
  assert.equal(result.audience, "db-a");
  assert.equal(result.expiresAt, "2026-11-24T00:15:00.000Z");
  assert.deepEqual(calls[0] && { audience: calls[0].audience, scope: calls[0].scope, ttlSeconds: calls[0].ttlSeconds }, { audience: "db-a", scope: "readwrite", ttlSeconds: 900 });
  const row = db.prepare("SELECT tenant_id, audience, scope, jti FROM credential").get() as Record<string, unknown>;
  assert.equal(row.tenant_id, binding);
  assert.equal(row.audience, "db-a");
  assert.equal("token" in row, false);
  assert.equal("opaque-secret-token".includes(String(row.jti)), false);
});

test("refuses in the §7 prerequisite order and rejects ddl", async () => {
  const { db, sql, deps, calls } = setup();
  assert.deepEqual(await issueCredential(sql, { ...session, live: false } as never, "read", deps), { ok: false, reason: "session_not_live" });
  db.prepare("UPDATE account SET state = 'suspended'").run();
  assert.deepEqual(await issueCredential(sql, session, "read", deps), { ok: false, reason: "account_not_active" });
  db.prepare("UPDATE account SET state = 'active'").run();
  db.prepare("UPDATE binding SET status = 'superseded'").run();
  assert.deepEqual(await issueCredential(sql, session, "read", deps), { ok: false, reason: "binding_not_active" });
  db.prepare("UPDATE binding SET status = 'active'").run();
  db.prepare("UPDATE tenant_db SET state = 'suspended'").run();
  assert.deepEqual(await issueCredential(sql, session, "read", deps), { ok: false, reason: "tenant_not_active" });
  db.prepare("UPDATE tenant_db SET state = 'active'").run();
  assert.deepEqual(await issueCredential(sql, session, "ddl", deps), { ok: false, reason: "invalid_scope" });
  assert.equal(calls.length, 0);
});

test("revokes idempotently by jti without retaining the bearer token", async () => {
  const { sql, deps, revoked } = setup();
  const result = await issueCredential(sql, session, "read", deps);
  assert.equal(result.ok, true);
  if (!result.ok) return;
  assert.equal(await revokeCredential(sql, result.credentialId, deps.issuer, at), true);
  assert.deepEqual(revoked, ["id-2"]);
  assert.equal(await revokeCredential(sql, result.credentialId, deps.issuer, at), true);
  assert.deepEqual(revoked, ["id-2"]);
});

test("retries issuer revocation after a failed attempt", async () => {
  const { db, sql, deps } = setup();
  const result = await issueCredential(sql, session, "read", deps);
  assert.equal(result.ok, true);
  if (!result.ok) return;

  let attempts = 0;
  const issuer = { async revoke(jti: string) {
    assert.equal(jti, "id-2");
    attempts++;
    if (attempts === 1) throw new Error("issuer unavailable");
  } };

  await assert.rejects(revokeCredential(sql, result.credentialId, issuer, at), /issuer unavailable/);
  assert.equal(attempts, 1);
  assert.equal((db.prepare("SELECT revoked_at FROM credential WHERE credential_id = ?").get(result.credentialId) as { revoked_at: unknown }).revoked_at, null);

  assert.equal(await revokeCredential(sql, result.credentialId, issuer, at), true);
  assert.equal(attempts, 2);
  assert.equal((db.prepare("SELECT revoked_at FROM credential WHERE credential_id = ?").get(result.credentialId) as { revoked_at: unknown }).revoked_at, at.toISOString());
});
