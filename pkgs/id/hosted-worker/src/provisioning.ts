// Provisioning gate for hosted production enablement (#3403; design
// doc/2026-11-01_jurisdiction_aware_user_store_design.md, "Provisioning
// gate" step; decision record on #3403, 2026-11-22).
//
// Production signup and migration stay disabled until a deployment
// configuration is validated against deployed-account evidence (the
// #2285 canary results and Cloudflare API read-backs). This module is
// that validator: pure, synchronous, fail-closed. It consumes
// evidence; it never probes an account itself, and it never treats
// configuration claims as evidence. The #2285 Half 1 results showed a
// plain Worker binding can reach a bucket outside its declared
// jurisdiction and that wrangler's `--jurisdiction` flag can be
// silently ignored, so a binding's declared `jurisdiction` field is
// checked here only for consistency — verification is the bucket's
// actual jurisdiction as read back from the R2 API, plus the canary's
// result for restricted Durable Object namespaces on the actual
// account, deployed runtime, and generated types.
//
// There is no unrestricted fallback: enablement is granted only for
// the jurisdiction-restricted namespace and the one verified bucket
// for the configured jurisdiction. Nothing in this module authorizes
// a deployment by itself — it returns the exact plan a deployment may
// enable, or refuses with an explicit diagnostic.

import { FailClosed, SUPPORTED_JURISDICTIONS, type Jurisdiction } from "./routing.ts";

/** What the deployed-account evidence says about one capability:
 * "supported" only when the canary / API read-back proved it on the
 * actual account; "unavailable" when it was checked and is not there;
 * "unverified" when it was not (or could not be) checked. Anything
 * other than "supported" fails closed. */
export type SupportState = "supported" | "unavailable" | "unverified";

export interface JurisdictionEvidence {
  /** Restricted DO namespace creation and runtime-reported
   * jurisdiction on the actual account (#2285 mechanics 1–4). */
  durableObjects: SupportState;
  /** Deployed runtime and generated types support for the
   * restriction on this account / compatibility date. */
  runtimeTypes: SupportState;
}

export interface BucketEvidence {
  bucketName: string;
  /** The bucket's actual jurisdiction as read back from the R2 API.
   * null when the bucket does not exist or its metadata could not be
   * read — both fail closed as unverified. */
  actualJurisdiction: string | null;
}

/** Deployed-account evidence, gathered by provisioning tooling from
 * the Cloudflare API and the #2285 canary. Never derived from the
 * deployment configuration being validated. */
export interface AccountEvidence {
  jurisdictions: Partial<Record<Jurisdiction, JurisdictionEvidence>>;
  buckets: BucketEvidence[];
}

export interface BucketConfig {
  binding: string;
  bucketName: string;
  /** The configuration's own claim (e.g. a wrangler binding field).
   * Checked for consistency only; never verification (#2285). */
  declaredJurisdiction?: string;
}

/** A request to enable hosted production signup/migration for one
 * configured jurisdiction. Every field is a claim to be validated;
 * absent fields fail closed. */
export interface EnablementConfig {
  jurisdiction?: string;
  /** Must be exactly true: enablement is granted only on the
   * jurisdiction-restricted DO namespace. Anything else asks for an
   * unrestricted fallback, which is forbidden. */
  restrictedNamespace?: boolean;
  durableObjectBinding?: string;
  bucket?: BucketConfig;
}

/** The validated enablement plan: the exact jurisdiction, DO binding,
 * and verified bucket a production deployment may enable. */
export interface EnablementPlan {
  jurisdiction: Jurisdiction;
  durableObjectBinding: string;
  bucket: { binding: string; bucketName: string; jurisdiction: Jurisdiction };
}

function refuse(code: string, message: string): never {
  throw new FailClosed(503, code, message);
}

/** The plans this module has issued. Module-private: a value carried
 * here cannot be constructed outside `validateEnablement`, so a
 * production enablement path can require the gate's verdict itself
 * rather than a look-alike object that merely spells its fields. */
const issuedPlans = new WeakSet<object>();

/** True only for a plan object returned by `validateEnablement`.
 * Structural equality is not enough — the gate's verdict is what
 * authorizes enablement. */
export function isValidatedEnablementPlan(value: unknown): value is EnablementPlan {
  return typeof value === "object" && value !== null && issuedPlans.has(value);
}

/** Validate a configuration against deployed-account evidence and
 * return the frozen plan a production deployment may enable. The
 * returned object is registered here, so
 * `isValidatedEnablementPlan` distinguishes it from a look-alike: the
 * production enablement point (./enablement.ts) accepts only this
 * verdict, never a re-spelled configuration. */
export function validateEnablement(config: EnablementConfig, evidence: AccountEvidence): EnablementPlan {
  // 1. The jurisdiction must be explicitly configured and supported.
  // The design's policy constant is not a silent runtime fallback: an
  // absent value is a configuration error, not a default.
  const rawJurisdiction = config.jurisdiction;
  if (rawJurisdiction === undefined || rawJurisdiction === "") {
    refuse(
      "missing_jurisdiction",
      "no jurisdiction is configured; production enablement requires an explicit jurisdiction",
    );
  }
  if (!(SUPPORTED_JURISDICTIONS as readonly string[]).includes(rawJurisdiction)) {
    refuse(
      "unsupported_jurisdiction",
      `configured jurisdiction "${rawJurisdiction}" is not supported; failing closed`,
    );
  }
  const jurisdiction = rawJurisdiction as Jurisdiction;

  // 2. No unrestricted fallback: enablement exists only on the
  // jurisdiction-restricted namespace.
  if (config.restrictedNamespace !== true) {
    refuse(
      "unrestricted_fallback_forbidden",
      "production enablement requires the jurisdiction-restricted Durable Object namespace; unrestricted fallback is forbidden",
    );
  }

  // 3. Deployed-account evidence must prove the restriction works on
  // this account, runtime, and generated types.
  const jx = evidence.jurisdictions[jurisdiction];
  if (!jx) {
    refuse(
      "jurisdiction_unverified",
      `no deployed-account evidence for jurisdiction "${jurisdiction}"; failing closed`,
    );
  }
  if (jx.durableObjects === "unavailable") {
    refuse(
      "jurisdiction_unavailable",
      `jurisdiction-restricted Durable Objects are unavailable for "${jurisdiction}" on the deployed account; failing closed`,
    );
  }
  if (jx.durableObjects !== "supported") {
    refuse(
      "jurisdiction_unverified",
      `restricted Durable Object support for "${jurisdiction}" is unverified on the deployed account; failing closed`,
    );
  }
  if (jx.runtimeTypes === "unavailable") {
    refuse(
      "unsupported_runtime_types",
      `the deployed runtime/types do not support jurisdiction restriction for "${jurisdiction}"; failing closed`,
    );
  }
  if (jx.runtimeTypes !== "supported") {
    refuse(
      "runtime_types_unverified",
      `runtime/types support for jurisdiction restriction for "${jurisdiction}" is unverified; failing closed`,
    );
  }

  // 4. The DO binding must be configured.
  if (!config.durableObjectBinding) {
    refuse(
      "missing_do_binding",
      "no Durable Object binding is configured for the restricted namespace; failing closed",
    );
  }

  // 5. The bucket for this jurisdiction must be configured, and its
  // ACTUAL jurisdiction — read back from the R2 API, never the
  // binding's declared field — must match the configured one.
  const bucket = config.bucket;
  if (!bucket) {
    refuse(
      "bucket_missing",
      `no R2 bucket is configured for jurisdiction "${jurisdiction}"; failing closed`,
    );
  }
  if (bucket.declaredJurisdiction !== undefined && bucket.declaredJurisdiction !== jurisdiction) {
    refuse(
      "bucket_jurisdiction_mismatch",
      `bucket binding "${bucket.binding}" declares jurisdiction "${bucket.declaredJurisdiction}" but the configured jurisdiction is "${jurisdiction}"; failing closed`,
    );
  }
  const bucketEvidence = evidence.buckets.find((b) => b.bucketName === bucket.bucketName);
  if (!bucketEvidence || bucketEvidence.actualJurisdiction === null) {
    refuse(
      "bucket_unverified",
      `bucket "${bucket.bucketName}" has no verified jurisdiction read-back from the R2 API; a declared binding jurisdiction is not verification; failing closed`,
    );
  }
  if (bucketEvidence.actualJurisdiction !== jurisdiction) {
    refuse(
      "bucket_jurisdiction_mismatch",
      `bucket "${bucket.bucketName}" actually resides in jurisdiction "${bucketEvidence.actualJurisdiction}" per the R2 API, not the configured "${jurisdiction}"; failing closed`,
    );
  }

  const plan: EnablementPlan = Object.freeze({
    jurisdiction,
    durableObjectBinding: config.durableObjectBinding,
    bucket: Object.freeze({ binding: bucket.binding, bucketName: bucket.bucketName, jurisdiction }),
  });
  issuedPlans.add(plan);
  return plan;
}
