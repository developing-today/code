import { test } from "node:test";
import assert from "node:assert/strict";
import { FailClosed, type Jurisdiction } from "../src/routing.ts";
import type { EnablementConfig } from "../src/provisioning.ts";
import {
  captureCanaryEvidence,
  createCloudflareApiClient,
  gatherAccountEvidence,
  provisionAndValidate,
  provisionBucket,
  readBucketEvidence,
  ProvisioningError,
  type CloudflareApiClient,
} from "../src/provisioning-surface.ts";

// Tests for the #3403 provisioning surface — the evidence-producing
// half of the gate. The fake client models the account behaviour the
// #2285 canary actually observed: a jurisdiction-less GET of a
// jurisdiction-scoped bucket returns 404, a header-scoped GET sees
// only its own jurisdiction's buckets, and bucket creation without
// the API header route is not a thing this surface does.

interface RecordedCall {
  method: "create" | "get";
  bucketName: string;
  jurisdiction?: Jurisdiction;
}

class FakeCloudflare implements CloudflareApiClient {
  calls: RecordedCall[] = [];
  /** bucket name -> actual jurisdiction; null = default jurisdiction. */
  buckets = new Map<string, string | null>();
  createStatus: number | null = null;
  failGets = false;

  async createR2Bucket(bucketName: string, jurisdiction: Jurisdiction) {
    this.calls.push({ method: "create", bucketName, jurisdiction });
    if (this.createStatus !== null) return { status: this.createStatus };
    if (this.buckets.has(bucketName)) return { status: 409 };
    this.buckets.set(bucketName, jurisdiction);
    return { status: 200 };
  }

  async getR2Bucket(bucketName: string, jurisdiction?: Jurisdiction) {
    this.calls.push({ method: "get", bucketName, jurisdiction });
    if (this.failGets) throw new Error("network unreachable");
    if (!this.buckets.has(bucketName)) return { status: 404, jurisdiction: null };
    const actual = this.buckets.get(bucketName) ?? null;
    if (jurisdiction === undefined) {
      // Jurisdiction-less reads see default-jurisdiction buckets only.
      if (actual !== null) return { status: 404, jurisdiction: null };
      return { status: 200, jurisdiction: null };
    }
    if (actual !== jurisdiction) return { status: 404, jurisdiction: null };
    return { status: 200, jurisdiction: actual };
  }
}

const EU_CONFIG: EnablementConfig = {
  jurisdiction: "eu",
  restrictedNamespace: true,
  durableObjectBinding: "USERS",
  bucket: { binding: "BLOBS_EU", bucketName: "user-blobs-eu", declaredJurisdiction: "eu" },
};

const US_CONFIG: EnablementConfig = {
  jurisdiction: "us",
  restrictedNamespace: true,
  durableObjectBinding: "USERS",
  bucket: { binding: "BLOBS_US", bucketName: "user-blobs-us", declaredJurisdiction: "us" },
};

const EU_CANARY = [{ jurisdiction: "eu", durableObjects: "supported", runtimeTypes: "supported" }] as const;

function refusalCode(fn: () => Promise<unknown>): Promise<string> {
  return fn().then(
    () => {
      throw new Error("expected a FailClosed refusal, but validation passed");
    },
    (e: unknown) => {
      if (e instanceof FailClosed) return e.code;
      throw e;
    },
  );
}

test("provisionBucket creates through the API route with the jurisdiction and returns the read-back", async () => {
  const fake = new FakeCloudflare();
  const evidence = await provisionBucket(fake, "user-blobs-eu", "eu");
  assert.deepEqual(evidence, { bucketName: "user-blobs-eu", actualJurisdiction: "eu" });
  assert.deepEqual(fake.calls[0], { method: "create", bucketName: "user-blobs-eu", jurisdiction: "eu" });
});

test("provisionBucket is idempotent: an existing bucket is read back, not recreated or trusted", async () => {
  const fake = new FakeCloudflare();
  fake.buckets.set("user-blobs-eu", "eu");
  const evidence = await provisionBucket(fake, "user-blobs-eu", "eu");
  assert.deepEqual(evidence, { bucketName: "user-blobs-eu", actualJurisdiction: "eu" });
});

test("provisionBucket surfaces a hard create failure instead of proceeding", async () => {
  const fake = new FakeCloudflare();
  fake.createStatus = 500;
  await assert.rejects(provisionBucket(fake, "user-blobs-eu", "eu"), (e: unknown) => {
    assert.ok(e instanceof ProvisioningError);
    assert.equal(e.code, "bucket_provision_failed");
    return true;
  });
  // No read-back was attempted after the failed create.
  assert.equal(fake.calls.filter((c) => c.method === "get").length, 0);
});

test("readBucketEvidence records a default-jurisdiction bucket as default, never as the claim", async () => {
  const fake = new FakeCloudflare();
  fake.buckets.set("user-blobs-eu", null); // exists, but in no jurisdiction
  const evidence = await readBucketEvidence(fake, "user-blobs-eu");
  assert.deepEqual(evidence, { bucketName: "user-blobs-eu", actualJurisdiction: "default" });
});

test("readBucketEvidence finds a jurisdiction bucket only through its header-scoped probe (#2285)", async () => {
  const fake = new FakeCloudflare();
  fake.buckets.set("user-blobs-eu", "eu");
  const evidence = await readBucketEvidence(fake, "user-blobs-eu");
  assert.equal(evidence.actualJurisdiction, "eu");
  // The jurisdiction-less read came first and missed; an eu-scoped
  // read is what found it.
  assert.equal(fake.calls[0]?.jurisdiction, undefined);
  assert.ok(fake.calls.some((c) => c.method === "get" && c.jurisdiction === "eu"));
});

test("readBucketEvidence records an unverifiable bucket as null (missing or unreadable)", async () => {
  const missing = new FakeCloudflare();
  assert.equal((await readBucketEvidence(missing, "nope")).actualJurisdiction, null);
  const down = new FakeCloudflare();
  down.buckets.set("user-blobs-eu", "eu");
  down.failGets = true;
  assert.equal((await readBucketEvidence(down, "user-blobs-eu")).actualJurisdiction, null);
});

test("captureCanaryEvidence passes results through verbatim and defaults the undetermined to unverified", () => {
  const captured = captureCanaryEvidence([
    { jurisdiction: "eu", durableObjects: "supported", runtimeTypes: "supported" },
    { jurisdiction: "us", durableObjects: "unavailable" },
    { jurisdiction: "apac", durableObjects: "supported", runtimeTypes: "supported" },
  ]);
  assert.deepEqual(captured, {
    eu: { durableObjects: "supported", runtimeTypes: "supported" },
    us: { durableObjects: "unavailable", runtimeTypes: "unverified" },
  });
});

test("gatherAccountEvidence combines canary capture with bucket read-backs", async () => {
  const fake = new FakeCloudflare();
  fake.buckets.set("user-blobs-eu", "eu");
  const evidence = await gatherAccountEvidence(fake, { buckets: ["user-blobs-eu"], canary: EU_CANARY });
  assert.deepEqual(evidence, {
    jurisdictions: { eu: { durableObjects: "supported", runtimeTypes: "supported" } },
    buckets: [{ bucketName: "user-blobs-eu", actualJurisdiction: "eu" }],
  });
});

test("provisionAndValidate returns the gate's plan on the full verified path", async () => {
  const fake = new FakeCloudflare();
  const plan = await provisionAndValidate(fake, EU_CONFIG, EU_CANARY);
  assert.deepEqual(plan, {
    jurisdiction: "eu",
    durableObjectBinding: "USERS",
    bucket: { binding: "BLOBS_EU", bucketName: "user-blobs-eu", jurisdiction: "eu" },
  });
  // The bucket was provisioned through the API route on the way.
  assert.ok(fake.calls.some((c) => c.method === "create" && c.jurisdiction === "eu"));
});

test("provisionAndValidate refuses when the bucket actually resides elsewhere, however it was created", async () => {
  const fake = new FakeCloudflare();
  // The bucket already exists in "us"; the eu create 409s, and the
  // read-back — not the create or the declared field — decides.
  fake.buckets.set("user-blobs-eu", "us");
  assert.equal(
    await refusalCode(() => provisionAndValidate(fake, EU_CONFIG, EU_CANARY)),
    "bucket_jurisdiction_mismatch",
  );
});

test("provisionAndValidate refuses on the canary's unavailable jurisdiction (explicit US diagnostic)", async () => {
  const fake = new FakeCloudflare();
  const canary = [{ jurisdiction: "us", durableObjects: "unavailable", runtimeTypes: "supported" }] as const;
  assert.equal(await refusalCode(() => provisionAndValidate(fake, US_CONFIG, canary)), "jurisdiction_unavailable");
});

test("provisionAndValidate creates nothing under an unsupported jurisdiction", async () => {
  const fake = new FakeCloudflare();
  const config: EnablementConfig = { ...EU_CONFIG, jurisdiction: "apac" };
  assert.equal(await refusalCode(() => provisionAndValidate(fake, config, EU_CANARY)), "unsupported_jurisdiction");
  assert.equal(fake.calls.filter((c) => c.method === "create").length, 0);
});

test("the API client sends the jurisdiction header on create and reads jurisdiction from the response", async () => {
  const seen: { url: string; headers?: Record<string, string> }[] = [];
  const client = createCloudflareApiClient({
    accountId: "acct-1",
    apiToken: "token-1",
    fetchFn: async (url, init) => {
      seen.push({ url, headers: init?.headers });
      return { status: 200, json: async () => ({ success: true, result: { name: "b", jurisdiction: "eu" } }) };
    },
  });
  await client.createR2Bucket("b", "eu");
  assert.equal(seen[0]?.headers?.["cf-r2-jurisdiction"], "eu");
  assert.match(seen[0]?.url ?? "", /\/accounts\/acct-1\/r2\/buckets$/);
  const read = await client.getR2Bucket("b", "eu");
  assert.deepEqual(read, { status: 200, jurisdiction: "eu" });
  assert.equal(seen[1]?.headers?.["cf-r2-jurisdiction"], "eu");
});

test("the API client reports a response without a jurisdiction field as null, not as a claim", async () => {
  const client = createCloudflareApiClient({
    accountId: "acct-1",
    apiToken: "token-1",
    fetchFn: async () => ({ status: 200, json: async () => ({ success: true, result: { name: "b" } }) }),
  });
  assert.deepEqual(await client.getR2Bucket("b"), { status: 200, jurisdiction: null });
});
