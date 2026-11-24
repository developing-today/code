// Provisioning surface for hosted production enablement (#3403): the
// evidence-producing half of the provisioning gate in
// ./provisioning.ts. The gate is a pure validator — it consumes
// deployed-account evidence and never probes an account itself. This
// module is the prober it presupposes:
//
//   - R2 buckets are provisioned through the Cloudflare API route
//     with the `cf-r2-jurisdiction` request header — never through
//     wrangler, whose `--jurisdiction` flag the #2285 canary showed
//     can be silently ignored (it created a default bucket with no
//     warning), and never without the header.
//   - A bucket's jurisdiction is taken only from the API read-back.
//     A jurisdiction-less read sees only default-jurisdiction
//     buckets (the canary's jurisdiction-less GET of an eu bucket
//     returned "does not exist"), so the read-back probes each
//     supported jurisdiction with its header before concluding a
//     bucket is unverified.
//   - The #2285 canary's results are captured into the gate's
//     evidence shape verbatim: only what the canary actually proved
//     is recorded, and it is the gate — not this surface — that
//     judges it.
//
// Nothing here enables production by itself: `provisionAndValidate`
// returns the gate's validated plan or the gate's refusal, and the
// gate authorizes no deployment. This surface makes no residency
// claim; it records what the account's own API reports.

import { SUPPORTED_JURISDICTIONS, type Jurisdiction } from "./routing.ts";
import {
  validateEnablement,
  type AccountEvidence,
  type BucketEvidence,
  type EnablementConfig,
  type EnablementPlan,
  type JurisdictionEvidence,
  type SupportState,
} from "./provisioning.ts";

/** An operational failure of the provisioning surface itself (the
 * API route refused or could not be reached). Distinct from the
 * gate's FailClosed refusals: no evidence judgement was reached. */
export class ProvisioningError extends Error {
  readonly code: string;
  constructor(code: string, message: string) {
    super(message);
    this.code = code;
  }
}

/** The slice of the Cloudflare API this surface needs. Structural,
 * so tests can model the account's observed behaviour (#2285)
 * without network access. */
export interface CloudflareApiClient {
  /** Create a bucket through the API route, scoped by the
   * jurisdiction header. Resolves with the HTTP status: 2xx created,
   * 409 already exists, anything else a failure. */
  createR2Bucket(bucketName: string, jurisdiction: Jurisdiction): Promise<{ status: number }>;
  /** Read one bucket back. Without a jurisdiction the read sees
   * only default-jurisdiction buckets; with one it sees only that
   * jurisdiction's buckets. `jurisdiction` in the result is the
   * bucket's API-reported jurisdiction, or null when the response
   * carries none. */
  getR2Bucket(
    bucketName: string,
    jurisdiction?: Jurisdiction,
  ): Promise<{ status: number; jurisdiction: string | null }>;
}

/** Minimal structural fetch shape (the package carries no
 * workers-types dependency; the global fetch satisfies this). */
export type FetchFn = (
  url: string,
  init?: { method?: string; headers?: Record<string, string>; body?: string },
) => Promise<{ status: number; json(): Promise<unknown> }>;

export function createCloudflareApiClient(options: {
  accountId: string;
  apiToken: string;
  fetchFn?: FetchFn;
}): CloudflareApiClient {
  const fetchFn: FetchFn = options.fetchFn ?? ((url, init) => fetch(url, init));
  const base = `https://api.cloudflare.com/client/v4/accounts/${options.accountId}/r2/buckets`;
  const auth = { authorization: `Bearer ${options.apiToken}` };
  return {
    async createR2Bucket(bucketName, jurisdiction) {
      const res = await fetchFn(base, {
        method: "POST",
        headers: {
          ...auth,
          "content-type": "application/json",
          // The verified API route (#2285 mechanic 5): the header is
          // what places the bucket; there is no other creation path.
          "cf-r2-jurisdiction": jurisdiction,
        },
        body: JSON.stringify({ name: bucketName }),
      });
      return { status: res.status };
    },
    async getR2Bucket(bucketName, jurisdiction) {
      const headers = jurisdiction ? { ...auth, "cf-r2-jurisdiction": jurisdiction } : { ...auth };
      const res = await fetchFn(`${base}/${encodeURIComponent(bucketName)}`, { method: "GET", headers });
      if (res.status !== 200) return { status: res.status, jurisdiction: null };
      const body = (await res.json()) as { result?: { jurisdiction?: unknown } } | null;
      const reported = body?.result?.jurisdiction;
      return { status: 200, jurisdiction: typeof reported === "string" ? reported : null };
    },
  };
}

async function tryGet(
  client: CloudflareApiClient,
  bucketName: string,
  jurisdiction?: Jurisdiction,
): Promise<{ status: number; jurisdiction: string | null } | null> {
  try {
    return await client.getR2Bucket(bucketName, jurisdiction);
  } catch {
    // A failed read is not evidence either way; the caller records
    // the bucket as unverified and the gate fails closed on it.
    return null;
  }
}

/** Read one bucket's actual jurisdiction back from the R2 API.
 * Returns the gate's BucketEvidence: `actualJurisdiction` is what
 * the API reports ("default" for a bucket outside every supported
 * jurisdiction), or null when no read could verify the bucket. */
export async function readBucketEvidence(
  client: CloudflareApiClient,
  bucketName: string,
): Promise<BucketEvidence> {
  // A jurisdiction-less read sees default-jurisdiction buckets only.
  const plain = await tryGet(client, bucketName);
  if (plain?.status === 200) {
    return { bucketName, actualJurisdiction: plain.jurisdiction ?? "default" };
  }
  // Jurisdiction-scoped reads: the header is what makes a bucket in
  // that jurisdiction visible at all (#2285), so a 200 here is the
  // API attesting the bucket resides in the probed jurisdiction even
  // when the response omits the field.
  for (const jurisdiction of SUPPORTED_JURISDICTIONS) {
    const scoped = await tryGet(client, bucketName, jurisdiction);
    if (scoped?.status === 200) {
      return { bucketName, actualJurisdiction: scoped.jurisdiction ?? jurisdiction };
    }
  }
  return { bucketName, actualJurisdiction: null };
}

/** Provision one bucket through the verified API route, then read
 * it back. Idempotent: a bucket that already exists (409) is read
 * back like any other — where it actually resides is the read-back's
 * to say, and the gate refuses a mismatch. */
export async function provisionBucket(
  client: CloudflareApiClient,
  bucketName: string,
  jurisdiction: Jurisdiction,
): Promise<BucketEvidence> {
  let status: number;
  try {
    status = (await client.createR2Bucket(bucketName, jurisdiction)).status;
  } catch (e) {
    throw new ProvisioningError(
      "bucket_provision_failed",
      `creating bucket "${bucketName}" through the R2 API route failed: ${e instanceof Error ? e.message : String(e)}`,
    );
  }
  if ((status < 200 || status >= 300) && status !== 409) {
    throw new ProvisioningError(
      "bucket_provision_failed",
      `creating bucket "${bucketName}" through the R2 API route returned HTTP ${status}; not proceeding`,
    );
  }
  return readBucketEvidence(client, bucketName);
}

/** One jurisdiction's outcome from the deployed-account canary
 * (#2285 mechanics 1–4 for Durable Objects, plus the deployed
 * runtime/generated-types support check). Fields the canary did not
 * determine stay absent here and are captured as "unverified". */
export interface CanaryJurisdictionResult {
  jurisdiction: string;
  durableObjects?: SupportState;
  runtimeTypes?: SupportState;
}

/** Capture canary results into the gate's evidence shape, verbatim:
 * only supported jurisdictions are recorded, and only the states
 * the canary actually reported — this surface never upgrades a
 * result. */
export function captureCanaryEvidence(
  results: readonly CanaryJurisdictionResult[],
): Partial<Record<Jurisdiction, JurisdictionEvidence>> {
  const captured: Partial<Record<Jurisdiction, JurisdictionEvidence>> = {};
  for (const result of results) {
    if (!(SUPPORTED_JURISDICTIONS as readonly string[]).includes(result.jurisdiction)) continue;
    const jurisdiction = result.jurisdiction as Jurisdiction;
    captured[jurisdiction] = {
      durableObjects: result.durableObjects ?? "unverified",
      runtimeTypes: result.runtimeTypes ?? "unverified",
    };
  }
  return captured;
}

/** Gather the full deployed-account evidence the gate consumes:
 * canary results for the jurisdictions, API read-backs for the
 * buckets. */
export async function gatherAccountEvidence(
  client: CloudflareApiClient,
  input: { buckets: readonly string[]; canary: readonly CanaryJurisdictionResult[] },
): Promise<AccountEvidence> {
  const buckets: BucketEvidence[] = [];
  for (const bucketName of input.buckets) {
    buckets.push(await readBucketEvidence(client, bucketName));
  }
  return { jurisdictions: captureCanaryEvidence(input.canary), buckets };
}

/** The surface end to end: provision the configured bucket through
 * the API route, gather the evidence, and invoke the gate before
 * any production signup/migration enablement. Returns the gate's
 * validated plan; the gate's FailClosed refusal propagates
 * unchanged. When the configured jurisdiction is not a supported
 * one, no bucket is created — the gate refuses that configuration
 * on its own, and provisioning under an invalid jurisdiction would
 * be exactly the silent misplacement this gate exists to prevent. */
export async function provisionAndValidate(
  client: CloudflareApiClient,
  config: EnablementConfig,
  canary: readonly CanaryJurisdictionResult[],
): Promise<EnablementPlan> {
  const buckets: BucketEvidence[] = [];
  if (config.bucket) {
    const configured = config.jurisdiction;
    if (configured !== undefined && (SUPPORTED_JURISDICTIONS as readonly string[]).includes(configured)) {
      buckets.push(await provisionBucket(client, config.bucket.bucketName, configured as Jurisdiction));
    } else {
      buckets.push(await readBucketEvidence(client, config.bucket.bucketName));
    }
  }
  return validateEnablement(config, { jurisdictions: captureCanaryEvidence(canary), buckets });
}
