// fleet-hosted — the hosted fleet front door (#12583; design
// doc/2026-11-22T14-28-45Z_design_fleet-tool-workers-edition §3.1).
//
// One Cloudflare Worker whose route modules (stamping, query, queue,
// ingestion, sync) share the auth/tenancy plumbing in src/auth.ts,
// src/jwt.ts and src/tenancy.ts. The skeleton (#12583) landed the
// plumbing with stub doors; the session registry (#12584), the
// stamping create path (#12585), and the Metadata query surface
// (#12587) are real over the same plumbing. The remaining doors are
// filled by their own issues (#12589 queue door, #12590 ingestion,
// #12591 sync).

import { configFromEnv, type HostedEnv } from "./config.ts";
import { AppRotationGitHubWriter, FixtureGitHubWriter } from "./github.ts";
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

export default {
  async fetch(req: Request, env: HostedEnv): Promise<Response> {
    const config = configFromEnv(env);
    // The hosted rotation point (#12585): with fleet App material
    // configured, GitHub writes go through the App installation-token
    // rotation; without it, the fixture writer stands in (local/dev
    // semantics — this package is not deployed, see wrangler.toml).
    // The queue record source is deliberately not wired: the queue
    // front door (#12589) owns claim records and wires its source
    // behind the QueueRecordSource seam; until then queue fields
    // stamp null (not queue-originated).
    const githubWriter = config.github
      ? new AppRotationGitHubWriter(config.github)
      : new FixtureGitHubWriter();
    // The query door's read path (#12587) is the same object: both
    // writer classes also implement GitHubItemSource over the same
    // rotation / recorded items.
    return dispatch(req, { config, sessions, metadataStore, githubWriter, githubItems: githubWriter });
  },
};
