import { test } from "node:test";
import assert from "node:assert/strict";
import { FailClosed } from "../src/routing.ts";
import {
  validateEnablement,
  type AccountEvidence,
  type EnablementConfig,
  type EnablementPlan,
} from "../src/provisioning.ts";
import {
  createProductionEnablement,
  requireProductionEnablement,
  HOSTED_PRODUCTION_MODES,
} from "../src/enablement.ts";

// Tests for the #3403 production enablement point: the place any
// production hosted signup or migration path must pass through. The
// acceptance criterion is that production cannot enable signup or
// migration without the provisioning gate's verdict, so an
// enablement that was not produced by `validateEnablement` — a
// hand-built look-alike plan, a configuration object, a literal — is
// not enablement at all, and an absent enablement leaves both modes
// disabled.

const US_CONFIG: EnablementConfig = {
  jurisdiction: "us",
  restrictedNamespace: true,
  durableObjectBinding: "USERS",
  bucket: { binding: "BLOBS_US", bucketName: "user-blobs-us", declaredJurisdiction: "us" },
};

const US_EVIDENCE: AccountEvidence = {
  jurisdictions: { us: { durableObjects: "supported", runtimeTypes: "supported" } },
  buckets: [{ bucketName: "user-blobs-us", actualJurisdiction: "us" }],
};

function refusal(fn: () => unknown): { code: string; status: number } {
  try {
    fn();
  } catch (e) {
    if (e instanceof FailClosed) return { code: e.code, status: e.status };
    throw e;
  }
  throw new Error("expected a FailClosed refusal, but the call was allowed");
}

test("a plan issued by the provisioning gate enables the requested modes", () => {
  const plan = validateEnablement(US_CONFIG, US_EVIDENCE);
  const enablement = createProductionEnablement(plan, ["signup", "migration"]);
  assert.deepEqual(enablement.modes, ["signup", "migration"]);
  assert.equal(enablement.jurisdiction, "us");
  assert.equal(enablement.durableObjectBinding, "USERS");
  assert.deepEqual(enablement.bucket, { binding: "BLOBS_US", bucketName: "user-blobs-us", jurisdiction: "us" });
  assert.equal(Object.isFrozen(enablement), true);
  assert.equal(Object.isFrozen(enablement.modes), true);
});

test("a hand-built plan that copies the gate's fields is not a validated plan", () => {
  // The production path is reachable only with the gate's verdict,
  // and structural equality must not stand in for it: otherwise
  // anything that can spell the fields could enable production
  // without ever consuming deployed-account evidence.
  const forged: EnablementPlan = {
    jurisdiction: "us",
    durableObjectBinding: "USERS",
    bucket: { binding: "BLOBS_US", bucketName: "user-blobs-us", jurisdiction: "us" },
  };
  assert.deepEqual(validateEnablement(US_CONFIG, US_EVIDENCE), forged);
  assert.deepEqual(refusal(() => createProductionEnablement(forged, ["signup"])), {
    code: "unvalidated_plan",
    status: 503,
  });
});

test("a configuration object is not a validated plan", () => {
  assert.equal(refusal(() => createProductionEnablement(US_CONFIG, ["signup"])).code, "unvalidated_plan");
  assert.equal(refusal(() => createProductionEnablement(null, ["signup"])).code, "unvalidated_plan");
  assert.equal(refusal(() => createProductionEnablement(undefined, ["signup"])).code, "unvalidated_plan");
});

test("enablement must name at least one production mode", () => {
  const plan = validateEnablement(US_CONFIG, US_EVIDENCE);
  assert.equal(refusal(() => createProductionEnablement(plan, [])).code, "no_enablement_mode");
});

test("enablement rejects modes other than signup and migration", () => {
  const plan = validateEnablement(US_CONFIG, US_EVIDENCE);
  assert.equal(refusal(() => createProductionEnablement(plan, ["backfill"])).code, "unknown_enablement_mode");
  assert.equal(
    refusal(() => createProductionEnablement(plan, ["signup", "unrestricted"])).code,
    "unknown_enablement_mode",
  );
  assert.deepEqual([...HOSTED_PRODUCTION_MODES], ["signup", "migration"]);
});

test("duplicate modes collapse to the canonical order", () => {
  const plan = validateEnablement(US_CONFIG, US_EVIDENCE);
  assert.deepEqual(createProductionEnablement(plan, ["migration", "signup", "signup"]).modes, [
    "signup",
    "migration",
  ]);
});

test("with no enablement active, production signup and migration are both disabled", () => {
  for (const mode of HOSTED_PRODUCTION_MODES) {
    assert.deepEqual(refusal(() => requireProductionEnablement(null, mode)), {
      code: "production_disabled",
      status: 503,
    });
    assert.equal(refusal(() => requireProductionEnablement(undefined, mode)).code, "production_disabled");
  }
});

test("a hand-built enablement is not an enablement", () => {
  const forged = {
    jurisdiction: "us",
    durableObjectBinding: "USERS",
    bucket: { binding: "BLOBS_US", bucketName: "user-blobs-us", jurisdiction: "us" },
    modes: ["signup", "migration"],
  };
  assert.equal(refusal(() => requireProductionEnablement(forged, "signup")).code, "production_disabled");
});

test("a mode that was not enabled is refused even when the other is", () => {
  const plan = validateEnablement(US_CONFIG, US_EVIDENCE);
  const signupOnly = createProductionEnablement(plan, ["signup"]);
  assert.equal(requireProductionEnablement(signupOnly, "signup"), signupOnly);
  assert.deepEqual(refusal(() => requireProductionEnablement(signupOnly, "migration")), {
    code: "mode_not_enabled",
    status: 503,
  });
});

test("a gate-issued plan cannot be edited to widen what it enables", () => {
  const plan = validateEnablement(US_CONFIG, US_EVIDENCE);
  assert.equal(Object.isFrozen(plan), true);
  assert.equal(Object.isFrozen(plan.bucket), true);
  assert.throws(() => {
    (plan as { jurisdiction: string }).jurisdiction = "fedramp";
  }, TypeError);
  assert.equal(plan.jurisdiction, "us");
});
