#!/usr/bin/env bun
// Usage: copy-push-registry-to-d1.ts [--source-url URL] (--copy | --verify-only)
// Strict arguments before I/O. Refreshes the five CF-5e (#15561) push
// registry tables from the live fleet-platform Turso store into D1
// fleet-platform, then hash-verifies source against destination.
//
// --copy: apply PUSH_REGISTRY_DDL to D1 (idempotent), delete+insert every
//   registry row from the source, then verify. Re-run at flip time to
//   close any drift from Turso writes since the last copy.
// --verify-only: hash both sides and report; writes nothing.
// --source-url: libsql HTTP URL of the source store (default: the
//   fleet-platform Turso database). The D1 target comes from
//   CLOUDFLARE_ACCOUNT_ID + PM_AGG_D1_DATABASE_ID with the
//   cloudflare.api-token-account-wide token, resolved by name.
//
// Each table is keyset-paginated independently: this is NOT an atomic
// cross-table snapshot. Quiesce writers when a consistent set matters.
// Output is counts + SHA-256 hashes only, never row content.
const DEFAULT_SOURCE_URL = "https://fleet-platform-dezren39.aws-us-east-1.turso.io";

const raw = process.argv.slice(2);
let sourceUrl = DEFAULT_SOURCE_URL;
let mode: "copy" | "verify-only" | null = null;
for (let i = 0; i < raw.length; i++) {
  const arg = raw[i] as string;
  if (arg === "--source-url") {
    const value = raw[++i];
    if (!value || value.startsWith("-") || !/^https:\/\/[^/@\s]+(\/[^@\s]*)?$/.test(value)) {
      console.error("Usage: copy-push-registry-to-d1.ts [--source-url URL] (--copy | --verify-only)");
      process.exit(2);
    }
    sourceUrl = value;
  } else if (arg === "--copy" || arg === "--verify-only") {
    if (mode !== null) {
      console.error("Usage: copy-push-registry-to-d1.ts [--source-url URL] (--copy | --verify-only)");
      process.exit(2);
    }
    mode = arg.slice(2) as "copy" | "verify-only";
  } else {
    console.error("Usage: copy-push-registry-to-d1.ts [--source-url URL] (--copy | --verify-only)");
    process.exit(2);
  }
}
if (mode === null) {
  console.error("Usage: copy-push-registry-to-d1.ts [--source-url URL] (--copy | --verify-only)");
  process.exit(2);
}

const { createTursoStoreClient } = await import("../src/turso-stores.ts");
const { createD1StoreClient } = await import("../src/d1-stores.ts");
const { resolveBwsSecret } = await import("../src/secrets.ts");
const { PUSH_REGISTRY_DDL } = await import("../src/push-registry.ts");
const { copyPushRegistry, verifyPushRegistryCopy } = await import("../src/push-registry-copy.ts");

const sourceToken = await resolveBwsSecret("turso.fleet-platform-rw-token");
if (!sourceToken) throw new Error("source token unresolved by name (turso.fleet-platform-rw-token)");
const source = createTursoStoreClient(sourceUrl, sourceToken);

const accountId = process.env.CLOUDFLARE_ACCOUNT_ID;
const databaseId = process.env.PM_AGG_D1_DATABASE_ID;
if (!accountId || !databaseId) throw new Error("CLOUDFLARE_ACCOUNT_ID / PM_AGG_D1_DATABASE_ID unset");
const d1Token = await resolveBwsSecret("cloudflare.api-token-account-wide");
if (!d1Token) throw new Error("D1 token unresolved by name (cloudflare.api-token-account-wide)");
const dest = createD1StoreClient(accountId, databaseId, d1Token);

if (mode === "copy") await copyPushRegistry(source, dest, PUSH_REGISTRY_DDL);
const report = await verifyPushRegistryCopy(source, dest);
for (const table of report.tables) {
  console.log(
    `${table.table} source=${table.sourceCount} dest=${table.destCount} match=${table.match ? "yes" : "NO"} sha=${table.destHash.slice(0, 12)}`,
  );
}
console.log(report.match ? "VERIFY MATCH" : "VERIFY MISMATCH");
process.exit(report.match ? 0 : 1);
