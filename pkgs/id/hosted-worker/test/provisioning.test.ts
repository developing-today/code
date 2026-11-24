import { test } from "node:test";
import assert from "node:assert/strict";
import { FailClosed } from "../src/routing.ts";
import { validateEnablement, type AccountEvidence, type EnablementConfig } from "../src/provisioning.ts";

// Configuration tests for the #3403 provisioning gate. Every refusal
// case is one of the acceptance failure modes: production must not
// enable signup or migration with a missing/unverified jurisdiction,
// unsupported runtime/types, or a missing/mismatched bucket — and
// never via an unrestricted fallback.

const US_CONFIG: EnablementConfig = {
  jurisdiction: "us",
  restrictedNamespace: true,
  durableObjectBinding: "USERS",
  bucket: { binding: "BLOBS_US", bucketName: "user-blobs-us", declaredJurisdiction: "us" },
};

const EU_CONFIG: EnablementConfig = {
  jurisdiction: "eu",
  restrictedNamespace: true,
  durableObjectBinding: "USERS",
  bucket: { binding: "BLOBS_EU", bucketName: "user-blobs-eu", declaredJurisdiction: "eu" },
};

function evidence(jurisdiction: "us" | "eu", bucketName: string, actualJurisdiction: string | null): AccountEvidence {
  return {
    jurisdictions: {
      [jurisdiction]: { durableObjects: "supported", runtimeTypes: "supported" },
    },
    buckets: [{ bucketName, actualJurisdiction }],
  };
}

const US_EVIDENCE = evidence("us", "user-blobs-us", "us");
const EU_EVIDENCE = evidence("eu", "user-blobs-eu", "eu");

function refusalCode(fn: () => unknown): string {
  try {
    fn();
  } catch (e) {
    if (e instanceof FailClosed) return e.code;
    throw e;
  }
  throw new Error("expected a FailClosed refusal, but validation passed");
}

test("verified us configuration yields the exact enablement plan", () => {
  assert.deepEqual(validateEnablement(US_CONFIG, US_EVIDENCE), {
    jurisdiction: "us",
    durableObjectBinding: "USERS",
    bucket: { binding: "BLOBS_US", bucketName: "user-blobs-us", jurisdiction: "us" },
  });
});

test("verified eu configuration yields the exact enablement plan", () => {
  assert.deepEqual(validateEnablement(EU_CONFIG, EU_EVIDENCE), {
    jurisdiction: "eu",
    durableObjectBinding: "USERS",
    bucket: { binding: "BLOBS_EU", bucketName: "user-blobs-eu", jurisdiction: "eu" },
  });
});

test("missing jurisdiction fails closed (no silent policy default)", () => {
  const { jurisdiction: _drop, ...noJurisdiction } = US_CONFIG;
  assert.equal(
    refusalCode(() => validateEnablement(noJurisdiction, US_EVIDENCE)),
    "missing_jurisdiction",
  );
  assert.equal(
    refusalCode(() => validateEnablement({ ...US_CONFIG, jurisdiction: "" }, US_EVIDENCE)),
    "missing_jurisdiction",
  );
});

test("unsupported jurisdiction fails closed", () => {
  assert.equal(
    refusalCode(() => validateEnablement({ ...US_CONFIG, jurisdiction: "apac" }, US_EVIDENCE)),
    "unsupported_jurisdiction",
  );
});

test("no unrestricted fallback: restricted namespace must be required", () => {
  assert.equal(
    refusalCode(() => validateEnablement({ ...US_CONFIG, restrictedNamespace: false }, US_EVIDENCE)),
    "unrestricted_fallback_forbidden",
  );
  const { restrictedNamespace: _drop, ...noRestriction } = US_CONFIG;
  assert.equal(
    refusalCode(() => validateEnablement(noRestriction, US_EVIDENCE)),
    "unrestricted_fallback_forbidden",
  );
});

test("jurisdiction with no deployed-account evidence fails closed as unverified", () => {
  const empty: AccountEvidence = { jurisdictions: {}, buckets: US_EVIDENCE.buckets };
  assert.equal(
    refusalCode(() => validateEnablement(US_CONFIG, empty)),
    "jurisdiction_unverified",
  );
});

test("unavailable US jurisdiction fails closed with an explicit diagnostic", () => {
  const usUnavailable: AccountEvidence = {
    jurisdictions: { us: { durableObjects: "unavailable", runtimeTypes: "supported" } },
    buckets: US_EVIDENCE.buckets,
  };
  assert.equal(
    refusalCode(() => validateEnablement(US_CONFIG, usUnavailable)),
    "jurisdiction_unavailable",
  );
});

test("unverified restricted-namespace support fails closed", () => {
  const unverified: AccountEvidence = {
    jurisdictions: { us: { durableObjects: "unverified", runtimeTypes: "supported" } },
    buckets: US_EVIDENCE.buckets,
  };
  assert.equal(
    refusalCode(() => validateEnablement(US_CONFIG, unverified)),
    "jurisdiction_unverified",
  );
});

test("unsupported runtime/types fail closed", () => {
  const unsupported: AccountEvidence = {
    jurisdictions: { us: { durableObjects: "supported", runtimeTypes: "unavailable" } },
    buckets: US_EVIDENCE.buckets,
  };
  assert.equal(
    refusalCode(() => validateEnablement(US_CONFIG, unsupported)),
    "unsupported_runtime_types",
  );
});

test("unverified runtime/types fail closed", () => {
  const unverified: AccountEvidence = {
    jurisdictions: { us: { durableObjects: "supported", runtimeTypes: "unverified" } },
    buckets: US_EVIDENCE.buckets,
  };
  assert.equal(
    refusalCode(() => validateEnablement(US_CONFIG, unverified)),
    "runtime_types_unverified",
  );
});

test("missing Durable Object binding fails closed", () => {
  const { durableObjectBinding: _drop, ...noBinding } = US_CONFIG;
  assert.equal(
    refusalCode(() => validateEnablement(noBinding, US_EVIDENCE)),
    "missing_do_binding",
  );
});

test("missing bucket fails closed", () => {
  const { bucket: _drop, ...noBucket } = US_CONFIG;
  assert.equal(
    refusalCode(() => validateEnablement(noBucket, US_EVIDENCE)),
    "bucket_missing",
  );
});

test("bucket whose declared jurisdiction disagrees with the configuration fails closed", () => {
  const config: EnablementConfig = {
    ...US_CONFIG,
    bucket: { binding: "BLOBS_EU", bucketName: "user-blobs-eu", declaredJurisdiction: "eu" },
  };
  assert.equal(
    refusalCode(() => validateEnablement(config, US_EVIDENCE)),
    "bucket_jurisdiction_mismatch",
  );
});

test("bucket with no R2 API read-back fails closed as unverified", () => {
  const noReadback: AccountEvidence = { jurisdictions: US_EVIDENCE.jurisdictions, buckets: [] };
  assert.equal(
    refusalCode(() => validateEnablement(US_CONFIG, noReadback)),
    "bucket_unverified",
  );
});

test("bucket whose API metadata cannot be read fails closed as unverified", () => {
  assert.equal(
    refusalCode(() => validateEnablement(US_CONFIG, evidence("us", "user-blobs-us", null))),
    "bucket_unverified",
  );
});

test("bucket actually residing in another jurisdiction fails closed (#2285 mixed R2 result)", () => {
  // #2285 Half 1: a plain binding reached the EU bucket from a US
  // configuration. The API read-back, not the binding, decides.
  assert.equal(
    refusalCode(() => validateEnablement(US_CONFIG, evidence("us", "user-blobs-us", "eu"))),
    "bucket_jurisdiction_mismatch",
  );
});

test("a declared binding jurisdiction that agrees is still not verification (#2285)", () => {
  // The config claims "us" everywhere, but there is no API read-back
  // for the bucket: the declared field must not substitute for it.
  const claimsOnly: AccountEvidence = { jurisdictions: US_EVIDENCE.jurisdictions, buckets: [] };
  assert.equal(
    refusalCode(() => validateEnablement(US_CONFIG, claimsOnly)),
    "bucket_unverified",
  );
});
