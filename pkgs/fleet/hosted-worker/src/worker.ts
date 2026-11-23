// fleet-hosted — the hosted fleet front door (#12583; design
// doc/2026-11-22T14-28-45Z_design_fleet-tool-workers-edition §3.1).
//
// One Cloudflare Worker whose route modules (stamping, query, queue,
// ingestion, sync) share the auth/tenancy plumbing in src/auth.ts,
// src/jwt.ts and src/tenancy.ts. The skeleton (#12583) landed the
// plumbing with stub doors; the session registry (#12584) is real
// over the same plumbing. The remaining doors are filled by their
// own issues (#12585 stamping, #12587 query, #12589 queue door,
// #12590 ingestion, #12591 sync).

import { configFromEnv, type HostedEnv } from "./config.ts";
import { dispatch } from "./router.ts";
import { FixtureSessionRegistry } from "./sessions.ts";

// Session registry seam (#12584): the record's production home is
// the fleet metadata DB's session table (#9074's DB, design §4.2),
// behind the SessionRegistry interface in src/sessions.ts. Until that
// DB lands, the Worker holds one fixture instance at module scope —
// its state lives for the isolate's lifetime, which is the local /
// dev semantics this slice targets; a DB-backed registry replaces
// it here without touching a route.
const sessions = new FixtureSessionRegistry();

export default {
  async fetch(req: Request, env: HostedEnv): Promise<Response> {
    return dispatch(req, { config: configFromEnv(env), sessions });
  },
};
