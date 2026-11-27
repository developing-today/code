// Production enablement point for hosted signup and migration
// (#3403; decision record on #3403, 2026-11-22). The provisioning
// gate (./provisioning.ts) validates a configuration against
// deployed-account evidence and returns a plan; this module is the
// only place that plan becomes production enablement, and the only
// place that answers whether production signup or migration may run.
//
// Visibility is the acceptance criterion. Today there is no live
// production signup/migration enable path — the deployment surface
// is local-only — so the criterion "production cannot enable
// signup or migration with missing/unverified jurisdiction,
// unsupported runtime/types, or missing/mismatched bucket" holds
// vacuously. The failure mode this module closes is the inverse:
// if a caller could enable production with a structurally identical
// but never-validated plan, the gate's verdict would be decorative.
// So creation accepts only a value issued by `validateEnablement`
// (established by identity, never by re-spelling the fields), and
// the per-mode check accepts only a value issued here. A forged
// look-alike, however exact, is refused as `unvalidated_plan`; an
// absent enablement leaves every mode `production_disabled`.
//
// Nothing here authorizes a deployment or makes a residency claim:
// the plan carries the gate's consumed evidence shape, not a vendor
// guarantee.

import { FailClosed } from "./routing.ts";
import { isValidatedEnablementPlan } from "./provisioning.ts";
import type { EnablementPlan } from "./provisioning.ts";
import type { Jurisdiction } from "./routing.ts";

/** The production hosted modes this enablement point knows. Signup
 * and migration enable independently; anything else is refused as
 * `unknown_enablement_mode`. */
export const HOSTED_PRODUCTION_MODES = ["signup", "migration"] as const;
export type HostedProductionMode = (typeof HOSTED_PRODUCTION_MODES)[number];

/** An active production enablement: the gate's plan plus the modes
 * it authorizes. Frozen at creation along with its plan; enabling a
 * mode outside `HOSTED_PRODUCTION_MODES` is impossible. */
export interface ProductionEnablement {
  readonly jurisdiction: Jurisdiction;
  readonly durableObjectBinding: string;
  readonly bucket: { readonly binding: string; readonly bucketName: string; readonly jurisdiction: Jurisdiction };
  readonly modes: readonly HostedProductionMode[];
}

/** The enablements this module has issued. Module-private: a value
 * carried here has passed through `createProductionEnablement` with
 * the gate's own verdict, so `requireProductionEnablement`
 * answers on authority rather than on shape. */
const issuedEnablements = new WeakSet<object>();

function orderModes(modes: readonly HostedProductionMode[]): HostedProductionMode[] {
  return HOSTED_PRODUCTION_MODES.filter((m) => modes.includes(m));
}

/** Create the production enablement for the gate's plan. Accepts
 * only a plan object returned by `validateEnablement` — a forged
 * copy of its fields throws FailClosed(503, "unvalidated_plan").
 * Modes outside `HOSTED_PRODUCTION_MODES` throw `unknown_enablement_mode`;
 * an empty mode list throws `no_enablement_mode`. */
export function createProductionEnablement(plan: unknown, modes: readonly string[]): ProductionEnablement {
  if (!isValidatedEnablementPlan(plan)) {
    throw new FailClosed(
      503,
      "unvalidated_plan",
      "production enablement requires the provisioning gate's validated plan, not a re-spelled configuration",
    );
  }
  const unknown = modes.filter((m): m is string => !(HOSTED_PRODUCTION_MODES as readonly string[]).includes(m));
  if (unknown.length > 0) {
    throw new FailClosed(
      503,
      "unknown_enablement_mode",
      `unknown production enablement mode "${unknown[0]}"; failing closed`,
    );
  }
  if (modes.length === 0) {
    throw new FailClosed(503, "no_enablement_mode", "production enablement requires at least one mode");
  }
  const enablement: ProductionEnablement = Object.freeze({
    jurisdiction: plan.jurisdiction,
    durableObjectBinding: plan.durableObjectBinding,
    bucket: plan.bucket,
    modes: Object.freeze(orderModes(modes as HostedProductionMode[])),
  });
  issuedEnablements.add(enablement);
  return enablement;
}

/** Answer whether a production mode may run. Refuses with
 * `production_disabled` when no enablement issued here is active,
 * and with `mode_not_enabled` when this enablement does not cover
 * the requested mode. Returns the enablement for callers that need
 * its plan details. */
export function requireProductionEnablement(enablement: unknown, mode: HostedProductionMode): ProductionEnablement {
  if (typeof enablement !== "object" || enablement === null || !issuedEnablements.has(enablement)) {
    throw new FailClosed(
      503,
      "production_disabled",
      `production ${mode} is disabled: no provisioning-gate enablement is active`,
    );
  }
  const active = enablement as ProductionEnablement;
  if (!active.modes.includes(mode)) {
    throw new FailClosed(503, "mode_not_enabled", `production ${mode} is not enabled; failing closed`);
  }
  return active;
}
