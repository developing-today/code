// Environment configuration for fleet-hosted (#12583).
//
// Everything the auth layer needs is configuration, because its two
// upstreams are other components' deliverables: the #9077 relying-party
// provider (issuer / JWKS) and component E's tenancy plane (here, the
// fixture seam in tenancy.ts). Missing configuration never fails open:
// it makes authConfigured false, protected routes refuse with 503, and
// health reports the gap as booleans (never values).

import type { RpValidationConfig } from "./jwt.ts";
import { parseTenancyFixture, type TenancyDirectory } from "./tenancy.ts";

export const HOSTED_VERSION = "0.1.0";
export const SERVICE_NAME = "fleet-hosted";
export const DEFAULT_AUDIENCE = "fleet-hosted";

export interface HostedEnv {
  RP_ISSUER?: string;
  RP_AUDIENCE?: string;
  RP_JWKS_URI?: string;
  TENANCY_FIXTURE_JSON?: string;
}

export interface HostedConfig {
  version: string;
  /** RP validation config, or null when no issuer is configured. */
  rp: RpValidationConfig | null;
  /** Tenancy directory, or null when none is configured/readable. */
  tenancy: TenancyDirectory | null;
  /** True only when every auth input is present and parsed. */
  authConfigured: boolean;
}

export function configFromEnv(env: HostedEnv): HostedConfig {
  const rp: RpValidationConfig | null = env.RP_ISSUER
    ? {
        issuer: env.RP_ISSUER,
        audience: env.RP_AUDIENCE ?? DEFAULT_AUDIENCE,
        jwksUri: env.RP_JWKS_URI ?? `${env.RP_ISSUER.replace(/\/$/, "")}/.well-known/jwks.json`,
      }
    : null;

  let tenancy: TenancyDirectory | null = null;
  if (env.TENANCY_FIXTURE_JSON) {
    try {
      tenancy = parseTenancyFixture(env.TENANCY_FIXTURE_JSON);
    } catch {
      // A malformed tenancy fixture is a configuration failure, not a
      // per-request one: leave tenancy null so auth fails closed and
      // health shows tenancy unconfigured.
      tenancy = null;
    }
  }

  return {
    version: HOSTED_VERSION,
    rp,
    tenancy,
    authConfigured: rp !== null && tenancy !== null,
  };
}
