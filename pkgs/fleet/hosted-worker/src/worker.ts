// fleet-hosted — the hosted fleet front door (#12583; design
// doc/2026-11-22T14-28-45Z_design_fleet-tool-workers-edition §3.1).
//
// One Cloudflare Worker whose route modules (stamping, query, queue,
// ingestion, sync) share the auth/tenancy plumbing in src/auth.ts,
// src/jwt.ts and src/tenancy.ts. The skeleton (#12583) landed the
// plumbing with stub doors; the session registry (#12584), the
// stamping create path (#12585), the Metadata query surface
// (#12587), the queue front door (#12589), the ingestion
// receiver + sweeper (#12590), and the sync door (#12591) are
// real over the same plumbing — every component-C door is live.
// QUEUE_D1 wires durable metadata, backfill, session and queue stores
// (#15456, #14866); unbound fixtures are for local development only.

import type { AuthDeps } from "./auth.ts";
import { FixtureBackfillRunStore, type BackfillRunStore } from "./backfill.ts";
import { BindingBridgeSurface } from "./bridge.ts";
import { configFromEnv, type HostedConfig, type HostedEnv } from "./config.ts";
import { AppRotationGitHubWriter, FixtureGitHubWriter } from "./github.ts";
import { ingestionDepsFrom, runIngestionSweep } from "./ingestion.ts";
import { LibSQLQueueStore, type SyncSqlDb } from "./libsql-queue-store.ts";
import { openHranaQueueStore } from "./libsql-hrana-queue-store.ts";
import { openD1QueueStore } from "./d1-queue-store.ts";
import { openD1BackfillRunStore, openD1MetadataStore } from "./d1-metadata-stores.ts";
import { openD1SessionRegistry } from "./d1-session-registry.ts";
import { r2KvBlobBucket } from "./kv-blobs.ts";
import { HttpKvPrimary, type KvPrimary } from "./kv-door.ts";
import { FixtureQueueStore, type QueueStore } from "./queue-store.ts";
import { StoreQueueRecordSource } from "./queue-records.ts";
import { dispatch } from "./router.ts";
import { FixtureSessionRegistry, type SessionRegistry } from "./sessions.ts";
import { FixtureMetadataStore, type MetadataStore } from "./stamping.ts";

// Session registry seam (#12584), behind the SessionRegistry
// interface in src/sessions.ts. With the QUEUE_D1 binding present,
// buildDeps resolves D1SessionRegistry (src/d1-session-registry.ts):
// the fleet_sessions table in that bound database, so records
// outlive the isolate. Without it, the Worker holds this one fixture
// instance at module scope — its state lives for the isolate's
// lifetime, the local / dev semantics.
const sessions = new FixtureSessionRegistry();

// Metadata store seam (#12585), behind the MetadataStore interface
// in src/stamping.ts. With the QUEUE_D1 binding present, buildDeps
// resolves D1MetadataStore (src/d1-metadata-stores.ts, #15456); this
// fixture is the unbound fallback.
const metadataStore = new FixtureMetadataStore();

// Backfill run store seam (#14312): the ingestion door's record of
// backfill runs, behind the BackfillRunStore interface in
// src/backfill.ts. With the QUEUE_D1 binding present, buildDeps
// resolves D1BackfillRunStore (#15456); this fixture is the unbound
// fallback.
const backfillRuns = new FixtureBackfillRunStore();

// Queue store seam (#12589): the queue door's backend, behind the
// QueueStore interface in src/queue-store.ts. The production home
// is the existing fleet-queue Turso DB (#6741) — the same backend
// the native broker serves — reached through LibSQLQueueStore (the
// same interface, no second queue, no schema fork); until it lands,
// one fixture store at module scope, with the isolate-lifetime
// semantics of the other seams. The #14866 move adds the D1 home:
// when the QUEUE_D1 binding is present, buildDeps resolves
// D1QueueStore over the same schema instead (see below).
const queueStore = new FixtureQueueStore();

/** The file → connection opener the libSQL queue door needs: it
 *  turns QUEUE_LIBSQL_FILE into the store's SyncSqlDb (tests and
 *  local dev inject node:sqlite here — the Worker bundle itself
 *  never imports it, so it stays runtime-agnostic). */
export type QueueSqlOpener = (file: string) => SyncSqlDb;

/** The URL → store opener for the remote fleet-queue Turso DB
 *  (#14592): turns QUEUE_LIBSQL_URL (+ its token) into the Hrana
 *  queue store. The default is the real fetch transport; tests
 *  inject a scripted peer the same way the file opener injects. */
export type QueueHranaOpener = (url: string, authToken: string | null) => QueueStore;

/** Resolve the queue backend for one configuration: the Hrana
 *  store over the remote Turso DB when QUEUE_LIBSQL_URL is set
 *  (the production backend — it takes precedence), else the libSQL
 *  store over the opener's connection when QUEUE_LIBSQL_FILE is
 *  set and an opener is injected, else the given fallback (the
 *  module fixture by default — tests/local-dev semantics). */
export function queueStoreFor(
  config: Pick<HostedConfig, "queueLibsqlFile" | "queueLibsqlUrl" | "queueLibsqlAuthToken">,
  opener?: QueueSqlOpener,
  fallback?: QueueStore,
  hranaOpener: QueueHranaOpener = openHranaQueueStore,
): QueueStore {
  if (config.queueLibsqlUrl) {
    return hranaOpener(config.queueLibsqlUrl, config.queueLibsqlAuthToken);
  }
  if (config.queueLibsqlFile && opener) {
    return new LibSQLQueueStore(opener(config.queueLibsqlFile));
  }
  return fallback ?? queueStore;
}

// The queue record source (#12589 wires the seam #12585 left):
// stamps that cite a session now read the queue's own claim
// records through StoreQueueRecordSource; a session with no queue
// binding still stamps its queue fields null (not queue-originated).
const queueRecords = new StoreQueueRecordSource(queueStore, sessions);

/** Assemble the door deps for one invocation (fetch or scheduled),
 * exactly as the Worker wires them in production. The optional
 * opener carries QUEUE_LIBSQL_FILE to a connection (local dev /
 * tests); without it the fixture store stands in. */
export function buildDeps(env: HostedEnv, opener?: QueueSqlOpener): AuthDeps {
  const config = configFromEnv(env);
  // Queue backend selection (#14866): a bound D1 database wins over
  // the Turso/libSQL backends \u2014 the binding's presence is the
  // operator's explicit Cloudflare-first choice. wrangler.toml binds
  // the deployed Worker to fleet-queue.
  const resolvedQueueStore = env.QUEUE_D1 ? openD1QueueStore(env.QUEUE_D1) : queueStoreFor(config, opener);
  // Session registry selection (#12584): the same bound D1
  // database holds the fleet_sessions table; absent the binding,
  // the module fixture stands in.
  const resolvedSessions: SessionRegistry = env.QUEUE_D1 ? openD1SessionRegistry(env.QUEUE_D1) : sessions;
  // The metadata and backfill run stores follow the same binding
  // (#15456): fleet_metadata and fleet_backfill_runs in that database.
  const resolvedMetadataStore: MetadataStore = env.QUEUE_D1 ? openD1MetadataStore(env.QUEUE_D1) : metadataStore;
  const resolvedBackfillRuns: BackfillRunStore = env.QUEUE_D1 ? openD1BackfillRunStore(env.QUEUE_D1) : backfillRuns;
  const resolvedQueueRecords =
    resolvedQueueStore === queueStore && resolvedSessions === sessions
      ? queueRecords
      : new StoreQueueRecordSource(resolvedQueueStore, resolvedSessions);
  // The hosted rotation point (#12585): with fleet App material
  // configured, GitHub writes go through the App installation-token
  // rotation; without it, the fixture writer stands in (local/dev
  // semantics; production requires real App configuration).
  const githubWriter = config.github ? new AppRotationGitHubWriter(config.github) : new FixtureGitHubWriter();
  // The query door's read path (#12587), the queue door's
  // fallback-claim surface (#12589), the ingestion door's
  // write-back (#12590) and the sync door's project surface
  // (#12591) are the same object: both writer classes also
  // implement GitHubItemSource, GitHubCommentSurface,
  // GitHubBodyEditor and GitHubProjectSurface over the same
  // rotation / recorded state.
  // The bridge door's surface (#14373): the message bridge over its
  // service binding, presenting the PM credential — the same binding
  // and credential the console's edge Worker uses, so the PM service
  // API and the console share one send path. Absent the binding or
  // the credential, the door fails closed (503) per request.
  const bridge = config.bridge && env.BRIDGE ? new BindingBridgeSurface(env.BRIDGE, config.bridge.token) : undefined;
  const kvPrimary = kvPrimaryFor(config);
  return {
    config,
    sessions: resolvedSessions,
    metadataStore: resolvedMetadataStore,
    githubWriter,
    githubItems: githubWriter,
    queueRecords: resolvedQueueRecords,
    queueStore: resolvedQueueStore,
    githubComments: githubWriter,
    githubEditor: githubWriter,
    githubProjects: githubWriter,
    backfillRunStore: resolvedBackfillRuns,
    ...(bridge ? { bridge } : {}),
    ...(kvPrimary ? { kvPrimary } : {}),
    ...(env.KV_BLOBS ? { kvBlobs: r2KvBlobBucket(env.KV_BLOBS) } : {}),
    ...(env.KV_BLOB_CAPABILITY_KEY ? { kvBlobCapabilityKey: env.KV_BLOB_CAPABILITY_KEY } : {}),
  };
}

// The fleet-kv central primary (#13316): one client per configured
// URL for the isolate's lifetime, so the door's once-per-isolate
// schema migration (src/kv-door.ts) is not repeated per request.
const kvPrimaries = new Map<string, KvPrimary>();

/** The kv door's primary for one configuration, or undefined when no
 *  KV_LIBSQL_URL is set (the door then fails closed with 503). */
export function kvPrimaryFor(config: Pick<HostedConfig, "kvLibsqlUrl" | "kvLibsqlAuthToken">): KvPrimary | undefined {
  if (!config.kvLibsqlUrl) return undefined;
  const key = JSON.stringify([config.kvLibsqlUrl, config.kvLibsqlAuthToken]);
  let primary = kvPrimaries.get(key);
  if (!primary) {
    primary = new HttpKvPrimary(config.kvLibsqlUrl, config.kvLibsqlAuthToken);
    kvPrimaries.set(key, primary);
  }
  return primary;
}

/** The scheduled invocation's context slice this module uses. */
interface ScheduledContext {
  waitUntil(promise: Promise<unknown>): void;
}

export default {
  async fetch(req: Request, env: HostedEnv): Promise<Response> {
    return dispatch(req, buildDeps(env));
  },

  /** The interval sweeper (#12590; design §7.3): on the cron
   * trigger (wrangler.toml), re-scan the configured in-scope
   * repos (INGESTION_REPOS) with the same check the webhook
   * route runs per delivery. No configured scope → nothing to
   * scan. A failed sweep is logged, never silent. */
  scheduled(_controller: unknown, env: HostedEnv, ctx: ScheduledContext): void {
    const deps = buildDeps(env);
    if (deps.config.ingestionRepos.length === 0) return;
    ctx.waitUntil(
      (async () => {
        try {
          const doc = await runIngestionSweep(ingestionDepsFrom(deps), deps.config.ingestionRepos);
          console.log(`fleet-hosted: ingestion sweep ${JSON.stringify(doc.counts)} over ${doc.repos.join(", ")}`);
        } catch (e) {
          console.error("fleet-hosted: ingestion sweep failed", e);
        }
      })(),
    );
  },
};
