// Health for fleet-hosted (#12583). Public (no credential), and
// reports the hosted door's identity plus configuration presence as
// booleans — never a configured value, never a secret (the
// message-bridge /healthz fingerprint posture, coarsened to booleans
// because this door's config is URLs and fixture presence, not tokens
// a deploy must fingerprint).

import { SERVICE_NAME, type HostedConfig } from "./config.ts";
import { HOSTED_DOORS } from "./capabilities.ts";

export function buildHealthDocument(config: HostedConfig): Record<string, unknown> {
  return {
    status: "ok",
    service: SERVICE_NAME,
    version: config.version,
    time: new Date().toISOString(),
    doors: HOSTED_DOORS.map((d) => d.id),
    auth: {
      rp_configured: config.rp !== null,
      tenancy_configured: config.tenancy !== null,
    },
    // The kv door's readiness (#13316): whether a central primary is
    // configured at all. A boolean, never the URL or its token.
    kv: {
      primary_configured: config.kvLibsqlUrl !== null,
    },
  };
}
