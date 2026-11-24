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
// What remains is production wiring, owned elsewhere: #9074's
// metadata DB behind the store interfaces and #12583's deploy.

import type { AuthDeps } from "./auth.ts";
import { FixtureBackfillRunStore } from "./backfill.ts";
import { BindingBridgeSurface } from "./bridge.ts";
import { configFromEnv, type HostedEnv } from "./config.ts";
import { AppRotationGitHubWriter, FixtureGitHubWriter } from "./github.ts";
import { ingestionDepsFrom, runIngestionSweep } from "./ingestion.ts";
import { FixtureQueueStore } from "./queue-store.ts";
import { StoreQueueRecordSource } from "./queue-records.ts";
import { dispatch } from "./router.ts";
import { FixtureSessionRegistry } from "./sessions.ts";
import { FixtureMetadataStore } from "./stamping.ts";

// Session registry seam (#12584): the record's production home is
// the fleet metadata DB's session table (#9074's DB, design §4.2),
// behind the SessionRegistry interface in src/sessions.ts. Until that
// DB lands, the Worker holds one fixture instance at module scope —
// its state lives for the isolate's lifetime, which is the local /
// dev semantics this slice targets; a DB-backed registry replaces
// it here without touching a route.
const sessions = new FixtureSessionRegistry();

// Metadata store seam (#12585): same fixture posture as the session
// registry — the production home is #9074's metadata DB, behind the
// MetadataStore interface in src/stamping.ts.
const metadataStore = new FixtureMetadataStore();

// Backfill run store seam (#14312): the ingestion door's record of
// backfill runs, behind the BackfillRunStore interface in
// src/backfill.ts — same fixture posture as the other seams, with
// #9074's metadata DB as its production home.
const backfillRuns = new FixtureBackfillRunStore();

// Queue store seam (#12589): the queue door's backend, behind the
// QueueStore interface in src/queue-store.ts. The production home
// is the existing fleet-queue Turso DB (#6741) — the same backend
// the native broker serves — reached through a libSQL
// implementation of the same interface; until it lands, one
// fixture store at module scope, with the isolate-lifetime
// semantics of the other seams.
const queueStore = new FixtureQueueStore();

// The queue record source (#12589 wires the seam #12585 left):
// stamps that cite a session now read the queue's own claim
// records through StoreQueueRecordSource; a session with no queue
// binding still stamps its queue fields null (not queue-originated).
const queueRecords = new StoreQueueRecordSource(queueStore, sessions);

/** Assemble the door deps for one invocation (fetch or scheduled),
 * exactly as the Worker wires them in production. */
function buildDeps(env: HostedEnv): AuthDeps {
  const config = configFromEnv(env);
  // The hosted rotation point (#12585): with fleet App material
  // configured, GitHub writes go through the App installation-token
  // rotation; without it, the fixture writer stands in (local/dev
  // semantics — this package is not deployed, see wrangler.toml).
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
  const bridge =
    config.bridge && env.BRIDGE ? new BindingBridgeSurface(env.BRIDGE, config.bridge.token) : undefined;
  return {
    config,
    sessions,
    metadataStore,
    githubWriter,
    githubItems: githubWriter,
    queueRecords,
    queueStore,
    githubComments: githubWriter,
    githubEditor: githubWriter,
    githubProjects: githubWriter,
    backfillRunStore: backfillRuns,
    ...(bridge ? { bridge } : {}),
  };
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
