// Publisher for the hosted /dns snapshot (#14968 slice 2).
//
// The #902 register stays the source of truth: the fleet-feed
// collector already builds and validates a DnsSection from
// register.json / the register Markdown on its ~5-minute cadence.
// This module publishes that exact section — never edited, never
// invented — as a single-row snapshot in the fleet-platform D1
// database, through Cloudflare's D1 REST API (the transport
// d1-stores.ts established for laptop-side writers), so the hosted
// console edge can serve /dns from its LEDGER_DB binding.
//
// The snapshot row carries the register's own generatedAt and source
// beside the publisher's receivedAt: the hosted reader exposes both
// and fails visibly once the snapshot is older than
// DNS_SNAPSHOT_MAX_AGE_MS (three missed collector intervals), so a
// dead publisher can never leave current-looking data behind.
//
// The API token is resolved by name only (D1_API_TOKEN_NAME, the
// account-wide Cloudflare token the D1 store writers already use)
// and is never logged: every error this module raises names only an
// HTTP status or a malformed-result shape.
import { isDnsSection } from "./dns.ts";
import { D1_API_TOKEN_NAME } from "./d1-stores.ts";
import { resolveBwsSecret } from "./secrets.ts";
import type { DnsSection } from "./model.ts";

export const DNS_SNAPSHOT_TABLE = "dns_snapshots";
/** The snapshot is a single row; this is its id. */
export const DNS_SNAPSHOT_ID = 1;
/** Freshness limit the hosted reader enforces (15 minutes). */
export const DNS_SNAPSHOT_MAX_AGE_MS = 15 * 60_000;

export const DNS_SNAPSHOT_CREATE_SQL =
  `create table if not exists ${DNS_SNAPSHOT_TABLE} ` +
  "(id integer primary key, section_json text not null, generated_at text not null, " +
  "received_at text not null, source text not null)";

/** Idempotent by construction: re-publishing the same section with
 * the same receivedAt writes the same row. */
export const DNS_SNAPSHOT_UPSERT_SQL =
  `insert into ${DNS_SNAPSHOT_TABLE} (id, section_json, generated_at, received_at, source) ` +
  "values (?, ?, ?, ?, ?) on conflict(id) do update set section_json = excluded.section_json, " +
  "generated_at = excluded.generated_at, received_at = excluded.received_at, source = excluded.source";

export interface DnsSnapshotStatement {
  sql: string;
  params: unknown[];
}

/** The exact statements one publication runs, in order. Refuses a
 * section the shared gate (isDnsSection) would not serve, and a
 * receivedAt that is not a parseable timestamp: the register remains
 * authoritative, so a malformed section is a collector bug to fix,
 * not data to publish. */
export function dnsSnapshotStatements(section: DnsSection, receivedAt: string): DnsSnapshotStatement[] {
  if (!isDnsSection(section)) throw new Error("refusing to publish an invalid DNS section");
  if (!Number.isFinite(Date.parse(receivedAt))) throw new Error("refusing to publish a DNS snapshot without a valid receivedAt");
  return [
    { sql: DNS_SNAPSHOT_CREATE_SQL, params: [] },
    {
      sql: DNS_SNAPSHOT_UPSERT_SQL,
      params: [DNS_SNAPSHOT_ID, JSON.stringify(section), section.generatedAt, receivedAt, section.source],
    },
  ];
}

export interface DnsPublisher {
  publish: (section: DnsSection, receivedAt: string) => Promise<void>;
}

type D1QueryResponse = {
  success?: boolean;
  result?: Array<{ success?: boolean }>;
};

/** Build a publisher for one D1 database over the REST query API.
 * `fetchFn` is injectable so tests can stand a fake endpoint in as
 * the oracle for the endpoint, the bearer credential, and the exact
 * statements sent. */
export function createDnsPublisher(opts: {
  accountId: string;
  databaseId: string;
  token: string;
  fetchFn?: typeof fetch;
}): DnsPublisher {
  const { accountId, databaseId, token } = opts;
  if (!/^[a-f\d]{32}$/i.test(accountId)) throw new Error("invalid Cloudflare account ID");
  if (!/^[a-f\d-]{36}$/i.test(databaseId)) throw new Error("invalid D1 database ID");
  if (!token.trim()) throw new Error("missing Cloudflare API token");
  const fetchFn = opts.fetchFn ?? fetch;
  const endpoint = `https://api.cloudflare.com/client/v4/accounts/${accountId}/d1/database/${databaseId}/query`;
  return {
    async publish(section: DnsSection, receivedAt: string): Promise<void> {
      for (const statement of dnsSnapshotStatements(section, receivedAt)) {
        const response = await fetchFn(endpoint, {
          method: "POST",
          headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
          body: JSON.stringify({ sql: statement.sql, params: statement.params }),
          signal: AbortSignal.timeout(60_000),
        });
        if (!response.ok) throw new Error(`DNS snapshot publish failed (HTTP ${response.status})`);
        let body: D1QueryResponse;
        try {
          body = (await response.json()) as D1QueryResponse;
        } catch {
          throw new Error("DNS snapshot publish returned malformed JSON");
        }
        // Cloudflare error text can contain request details; keep it
        // out of the error, like the D1 store transport does.
        if (body.success !== true || body.result?.[0]?.success !== true) {
          throw new Error("DNS snapshot publish failed or returned a malformed result");
        }
      }
    },
  };
}

/** Resolve the configured publisher, or null when this machine does
 * not name a D1 home for the aggregator (the same env names the D1
 * store getter uses: CLOUDFLARE_ACCOUNT_ID + PM_AGG_D1_DATABASE_ID,
 * token by secret name). Null is a configuration fact the caller
 * logs — the hosted surface's freshness rule is what fails visibly
 * when no snapshot ever lands. Mirrors createD1StoreGetter's
 * caching: a null result is retried on the next run, a built
 * publisher is kept. */
export function createDnsPublisherGetter(
  resolveSecret = resolveBwsSecret,
  env: NodeJS.ProcessEnv = process.env,
): () => Promise<DnsPublisher | null> {
  let cached: Promise<DnsPublisher | null> | null = null;
  return () => {
    if (!cached) {
      const pending = (async () => {
        const accountId = env.CLOUDFLARE_ACCOUNT_ID;
        const databaseId = env.PM_AGG_D1_DATABASE_ID;
        if (!accountId || !databaseId) return null;
        const token = await resolveSecret(D1_API_TOKEN_NAME);
        return token ? createDnsPublisher({ accountId, databaseId, token }) : null;
      })();
      cached = pending;
      void pending.then(
        (publisher) => {
          if (!publisher && cached === pending) cached = null;
        },
        () => {
          if (cached === pending) cached = null;
        },
      );
    }
    return cached;
  };
}

export const getDnsPublisher = createDnsPublisherGetter();
