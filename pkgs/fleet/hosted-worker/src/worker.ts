// fleet-hosted — the hosted fleet front door (#12583; design
// doc/2026-11-22T14-28-45Z_design_fleet-tool-workers-edition §3.1).
//
// One Cloudflare Worker whose route modules (stamping, query, queue,
// ingestion, sync) share the auth/tenancy plumbing in src/auth.ts,
// src/jwt.ts and src/tenancy.ts. This slice is the skeleton: plumbing
// is real, the doors are stubs the later component-C issues fill
// (#12585 stamping, #12587 query, #12589 queue door, #12590 ingestion,
// #12591 sync; the session registry is #12584).

import { configFromEnv, type HostedEnv } from "./config.ts";
import { dispatch } from "./router.ts";

export default {
  async fetch(req: Request, env: HostedEnv): Promise<Response> {
    return dispatch(req, { config: configFromEnv(env) });
  },
};
