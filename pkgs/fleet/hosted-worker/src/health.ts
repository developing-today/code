// Health for fleet-hosted (#12583). Public (no credential), and
// reports the hosted door's identity plus configuration presence as
// booleans — never a configured value, never a secret (the
// message-bridge /healthz fingerprint posture, coarsened to booleans
// because this door's config is URLs and fixture presence, not tokens
// a deploy must fingerprint).

import { SERVICE_NAME, type HostedConfig } from "./config.ts";
import { HOSTED_DOORS } from "./capabilities.ts";

/** The /version answer (#15793 criterion 1): the shape the edge
 * probe reads (`version` field, `commitOf` in
 * pm-stack/console/edge/src/worker-checks.ts). Pure: names only the
 * deployed commit stamped as the VERSION var at deploy time, or
 * "unknown" when unstamped — the probe records that honestly.
 * Deliberately separate from /health, whose `version` is the
 * package version, not a commit. */
export function versionAnswer(config: Pick<HostedConfig, "deployedCommit">): Record<string, unknown> {
  return {
    ok: true,
    service: SERVICE_NAME,
    version: config.deployedCommit ?? "unknown",
  };
}

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
      service_auth_configured: config.serviceAuthConfigured,
    },
    // The kv door's readiness (#13316): whether a central primary is
    // configured at all. A boolean, never the URL or its token.
    kv: {
      primary_configured: config.kvLibsqlUrl !== null,
    },
    // Metadata durability (#16096): whether QUEUE_D1 is bound, which
    // is exactly when buildDeps resolves D1MetadataStore over the
    // fixture (#15456, #14866). Booleans, never the binding.
    storage: {
      d1_bound: config.d1Bound,
      metadata_durable: config.d1Bound,
    },
  };
}
