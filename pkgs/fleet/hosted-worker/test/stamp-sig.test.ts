// Stamp signature tests (#12586). Oracles: the issue's acceptance —
// (1) a service-stamped item verifies; the same block with one
// provenance field edited fails verification and is reported as
// divergence; (2) a native-stamped item and a hosted-stamped item
// verify through the same code path; (3) unsigned legacy blocks are
// reported as unsigned, not as forgeries — and design §4.3/§5 (the
// signature covers the canonical provenance object, fleet-owned fields
// only; caller-owned fields are outside it).
//
// The two GOLDEN vectors below are the cross-edition conformance
// fixtures (design §6's shared-fixture discipline): fixed test keys, a
// fixed provenance object per edition, and the exact canonical bytes +
// stamp_sig each produces. Ed25519 is deterministic, so the native
// implementation (#9073's signing hook) must reproduce these bytes
// exactly for the same inputs; if a vector here ever changes, the
// format changed and both editions must move together. The keys are
// test-only material, generated for these fixtures, guarding nothing.

import assert from "node:assert/strict";
import { test } from "node:test";
import { configFromEnv } from "../src/config.ts";
import { FailClosed } from "../src/errors.ts";
import {
  b64urlToBytes,
  canonicalProvenanceBytes,
  importStampSigner,
  importStampVerifyKeys,
  requireStampSigner,
  signStamp,
  verifyStamp,
  type Provenance,
} from "../src/stamp-sig.ts";

const HOSTED_KID = "hosted-2026-11";
const HOSTED_PKCS8 = "MC4CAQAwBQYDK2VwBCIEIAABAgMEBQYHCAkKCwwNDg8QERITFBUWFxgZGhscHR4f";
const HOSTED_PUB = "A6EHv_POEL4dcN0Y50vAmWfk1jCbpQ1fHdyGZBJVMbg";
const NATIVE_KID = "native-2026-11";
const NATIVE_PKCS8 = "MC4CAQAwBQYDK2VwBCIEIKChoqOkpaanqKmhChGhKhOhShWhahehihmiCiGiKiOi";
const NATIVE_PUB = "C0rwl7rmIyCh6BbtAC971q4Cep4qVSKBtRZpZOCR1Ss";

const HOSTED_PROVENANCE: Provenance = {
  queue_id: 21092,
  session_id: "ses_0123456789abcdef0123456789abcdef",
  lane: "code",
  host: "fleet-hosted",
  worktree: null,
  workspace_ref: {
    kind: "worker",
    id: "fleet-hosted",
    repo: "developing-today/code",
    branch: null,
    sha: null,
  },
  picked_up_at: "2026-11-22T14:31:02Z",
  work_started_at: "2026-11-22T14:31:05Z",
  created_at: "2026-11-22T14:31:09Z",
  model: "claude-opus-4-5",
  lane_start_issues: [12584, 12586],
  auth_subject: "9f2c1a44b7e54d0f8c3a5b6d7e8f9012",
  stamped_by: "fleet-hosted@0.1.0",
};
const HOSTED_CANONICAL =
  '{"auth_subject":"9f2c1a44b7e54d0f8c3a5b6d7e8f9012","created_at":"2026-11-22T14:31:09Z","host":"fleet-hosted","lane":"code","lane_start_issues":[12584,12586],"model":"claude-opus-4-5","picked_up_at":"2026-11-22T14:31:02Z","queue_id":21092,"session_id":"ses_0123456789abcdef0123456789abcdef","stamped_by":"fleet-hosted@0.1.0","work_started_at":"2026-11-22T14:31:05Z","workspace_ref":{"branch":null,"id":"fleet-hosted","kind":"worker","repo":"developing-today/code","sha":null},"worktree":null}';
const HOSTED_STAMP_SIG =
  "v1.ed25519.hosted-2026-11.NiP0TffLEBFw6qLqVfQlRajtgwKghveQNSoqcIpxMuo8sYYVSFw1Qi1-zBhZIG1usHSz5tL1-oO64fBS84EeAA";

const NATIVE_PROVENANCE: Provenance = {
  queue_id: null,
  session_id: "ses_fedcba9876543210fedcba9876543210",
  lane: "code",
  host: "amd-6",
  worktree: "/home/user/code-12586",
  picked_up_at: null,
  work_started_at: "2026-11-22T15:02:11Z",
  created_at: "2026-11-22T15:02:14Z",
  model: "claude-opus-4-5",
  lane_start_issues: [],
  auth_subject: "9f2c1a44b7e54d0f8c3a5b6d7e8f9012",
  stamped_by: "fleet 0.9.1 (native)",
};
const NATIVE_CANONICAL =
  '{"auth_subject":"9f2c1a44b7e54d0f8c3a5b6d7e8f9012","created_at":"2026-11-22T15:02:14Z","host":"amd-6","lane":"code","lane_start_issues":[],"model":"claude-opus-4-5","picked_up_at":null,"queue_id":null,"session_id":"ses_fedcba9876543210fedcba9876543210","stamped_by":"fleet 0.9.1 (native)","work_started_at":"2026-11-22T15:02:11Z","worktree":"/home/user/code-12586"}';
const NATIVE_STAMP_SIG =
  "v1.ed25519.native-2026-11.TNCmecEcLsL9SgyxUPg8cJ_affdmD3RYYu3Ukuyqzb107E9mbm_EfIwHjW_FJBVtxjWrGCYfbmP64OOmpjGaDQ";

const hostedSigner = await importStampSigner({ keyId: HOSTED_KID, pkcs8: b64urlToBytes(HOSTED_PKCS8) });
const nativeSigner = await importStampSigner({ keyId: NATIVE_KID, pkcs8: b64urlToBytes(NATIVE_PKCS8) });
const bothKeys = await importStampVerifyKeys(
  new Map([
    [HOSTED_KID, b64urlToBytes(HOSTED_PUB)],
    [NATIVE_KID, b64urlToBytes(NATIVE_PUB)],
  ]),
);

test("golden vector (hosted): canonical bytes and stamp_sig are byte-exact", async () => {
  assert.equal(new TextDecoder().decode(canonicalProvenanceBytes(HOSTED_PROVENANCE)), HOSTED_CANONICAL);
  const stamped = await signStamp(HOSTED_PROVENANCE, hostedSigner);
  assert.equal(stamped["stamp_sig"], HOSTED_STAMP_SIG);
});

test("golden vector (native): canonical bytes and stamp_sig are byte-exact", async () => {
  assert.equal(new TextDecoder().decode(canonicalProvenanceBytes(NATIVE_PROVENANCE)), NATIVE_CANONICAL);
  const stamped = await signStamp(NATIVE_PROVENANCE, nativeSigner);
  assert.equal(stamped["stamp_sig"], NATIVE_STAMP_SIG);
});

test("a service-stamped item verifies (acceptance 1, first half)", async () => {
  const stamped = await signStamp(HOSTED_PROVENANCE, hostedSigner);
  assert.deepEqual(await verifyStamp(stamped, bothKeys), { status: "verified", keyId: HOSTED_KID });
});

test("native-stamped and hosted-stamped items verify through the same code path (acceptance 2)", async () => {
  const hostedStamped = await signStamp(HOSTED_PROVENANCE, hostedSigner);
  const nativeStamped = await signStamp(NATIVE_PROVENANCE, nativeSigner);
  assert.deepEqual(await verifyStamp(hostedStamped, bothKeys), { status: "verified", keyId: HOSTED_KID });
  assert.deepEqual(await verifyStamp(nativeStamped, bothKeys), { status: "verified", keyId: NATIVE_KID });
});

test("one edited provenance field fails verification as divergence (acceptance 1, second half)", async () => {
  const stamped = await signStamp(HOSTED_PROVENANCE, hostedSigner);
  for (const edit of [
    { lane: "other-lane" },
    { created_at: "2026-11-22T14:31:10Z" },
    { auth_subject: "00000000000000000000000000000000" },
    { queue_id: 1 },
    { lane_start_issues: [12584] },
    { workspace_ref: { kind: "worker", id: "fleet-hosted", repo: "developing-today/code", branch: "main", sha: null } },
  ]) {
    assert.deepEqual(
      await verifyStamp({ ...stamped, ...edit }, bothKeys),
      { status: "diverged", reason: "signature_mismatch" },
      `edit ${JSON.stringify(edit)} must diverge`,
    );
  }
  // Removing a signed field is an edit too.
  const { lane: _dropped, ...stripped } = stamped;
  assert.deepEqual(await verifyStamp(stripped, bothKeys), {
    status: "diverged",
    reason: "signature_mismatch",
  });
});

test("unsigned legacy blocks are reported as unsigned, not forgeries (acceptance 3)", async () => {
  assert.deepEqual(await verifyStamp(HOSTED_PROVENANCE, bothKeys), { status: "unsigned" });
  assert.deepEqual(await verifyStamp({ ...HOSTED_PROVENANCE, stamp_sig: null }, bothKeys), {
    status: "unsigned",
  });
});

test("canonical form is independent of key order and of the stamp_sig itself", async () => {
  const reordered: Provenance = {};
  for (const key of Object.keys(HOSTED_PROVENANCE).reverse()) {
    reordered[key] = HOSTED_PROVENANCE[key];
  }
  assert.deepEqual(canonicalProvenanceBytes(reordered), canonicalProvenanceBytes(HOSTED_PROVENANCE));
  const stamped = await signStamp(HOSTED_PROVENANCE, hostedSigner);
  assert.deepEqual(canonicalProvenanceBytes(stamped), canonicalProvenanceBytes(HOSTED_PROVENANCE));
});

test("caller-owned block fields never enter the signed object", async () => {
  // direct_parent / links / lineage live beside provenance in the
  // block, not inside it: the functions here only ever see the
  // provenance object, so a caller editing its own fields cannot
  // invalidate a stamp — and cannot extend the signature over them.
  const stamped = await signStamp(HOSTED_PROVENANCE, hostedSigner);
  const block = {
    pm_meta: 1,
    direct_parent: { repo: "developing-today/code", number: 9081, kind: "epic" },
    links: [],
    lineage: { design: null, roadmap_item: null, priority: null },
    provenance: stamped,
  };
  block.direct_parent = { repo: "developing-today/code", number: 9078, kind: "epic" };
  assert.deepEqual(await verifyStamp(block.provenance, bothKeys), {
    status: "verified",
    keyId: HOSTED_KID,
  });
});

test("tampered or malformed stamp_sig values diverge with named reasons", async () => {
  const stamped = await signStamp(HOSTED_PROVENANCE, hostedSigner);
  const sig = stamped["stamp_sig"] as string;

  // Signature bytes flipped (still well-formed base64url, right
  // length). Flip the FIRST signature character: the last character of
  // an unpadded 64-byte encoding carries unused low bits, so flipping
  // it could decode to the identical bytes.
  const sigStart = sig.lastIndexOf(".") + 1;
  const flipped = sig.slice(0, sigStart) + (sig[sigStart] === "A" ? "B" : "A") + sig.slice(sigStart + 1);
  assert.deepEqual(await verifyStamp({ ...stamped, stamp_sig: flipped }, bothKeys), {
    status: "diverged",
    reason: "signature_mismatch",
  });
  // Truncated / not the v1 shape at all.
  for (const bad of ["", "garbage", "v1.ed25519", `${sig}.extra`, "v1.ed25519..abc"]) {
    assert.deepEqual(
      await verifyStamp({ ...stamped, stamp_sig: bad }, bothKeys),
      { status: "diverged", reason: "malformed_stamp_sig" },
      `stamp_sig ${JSON.stringify(bad)} must be malformed`,
    );
  }
  // A future/foreign version is named, not lumped in with forgery.
  assert.deepEqual(await verifyStamp({ ...stamped, stamp_sig: sig.replace("v1.ed25519", "v2.ed25519") }, bothKeys), {
    status: "diverged",
    reason: "unsupported_stamp_version",
  });
  // Non-string stamp_sig cannot verify.
  assert.deepEqual(await verifyStamp({ ...stamped, stamp_sig: 42 }, bothKeys), {
    status: "diverged",
    reason: "malformed_stamp_sig",
  });
});

test("wrong key and unknown key id are refused", async () => {
  const stamped = await signStamp(HOSTED_PROVENANCE, hostedSigner);
  // Same key id, different key material (the native key's bytes).
  const wrongKey = await importStampVerifyKeys(new Map([[HOSTED_KID, b64urlToBytes(NATIVE_PUB)]]));
  assert.deepEqual(await verifyStamp(stamped, wrongKey), {
    status: "diverged",
    reason: "signature_mismatch",
  });
  // The signing key id is not among the verifier's keys at all.
  const otherKeys = await importStampVerifyKeys(new Map([[NATIVE_KID, b64urlToBytes(NATIVE_PUB)]]));
  assert.deepEqual(await verifyStamp(stamped, otherKeys), {
    status: "diverged",
    reason: "unknown_key",
  });
});

test("verification without configured keys reports unverifiable, never verified", async () => {
  const stamped = await signStamp(HOSTED_PROVENANCE, hostedSigner);
  assert.deepEqual(await verifyStamp(stamped, null), {
    status: "unverifiable",
    reason: "no_verify_keys_configured",
  });
  assert.deepEqual(await verifyStamp(stamped, new Map()), {
    status: "unverifiable",
    reason: "no_verify_keys_configured",
  });
});

test("uncanonicalizable provenance refuses instead of signing ambiguously", async () => {
  await assert.rejects(
    () => signStamp({ ...HOSTED_PROVENANCE, queue_id: 1.5 }, hostedSigner),
    (e: unknown) => e instanceof FailClosed && e.code === "uncanonicalizable_provenance",
  );
  assert.throws(
    () => canonicalProvenanceBytes({ ...HOSTED_PROVENANCE, model: undefined }),
    (e: unknown) => e instanceof FailClosed && e.code === "uncanonicalizable_provenance",
  );
});

test("signing is unconfigured-closed: no key material, no stamp (503 idiom)", async () => {
  assert.throws(
    () => requireStampSigner(null),
    (e: unknown) => e instanceof FailClosed && e.status === 503 && e.code === "stamp_signing_not_configured",
  );
  const config = configFromEnv({
    STAMP_SIGNING_KEY_PKCS8: HOSTED_PKCS8,
    STAMP_SIGNING_KEY_ID: HOSTED_KID,
  });
  const signer = await requireStampSigner(config.stamp.signing);
  const stamped = await signStamp(HOSTED_PROVENANCE, signer);
  assert.equal(stamped["stamp_sig"], HOSTED_STAMP_SIG);
});

test("config parses stamp material by name and fails closed on malformed values", async () => {
  const good = configFromEnv({
    STAMP_SIGNING_KEY_PKCS8: HOSTED_PKCS8,
    STAMP_SIGNING_KEY_ID: HOSTED_KID,
    STAMP_VERIFY_KEYS_JSON: JSON.stringify({ [HOSTED_KID]: HOSTED_PUB, [NATIVE_KID]: NATIVE_PUB }),
  });
  assert.equal(good.stamp.signing?.keyId, HOSTED_KID);
  assert.deepEqual([...(good.stamp.verifyKeys?.keys() ?? [])].sort(), [HOSTED_KID, NATIVE_KID].sort());
  // The config seam feeds verification end to end.
  const keys = await importStampVerifyKeys(good.stamp.verifyKeys!);
  const stamped = await signStamp(NATIVE_PROVENANCE, nativeSigner);
  assert.deepEqual(await verifyStamp(stamped, keys), { status: "verified", keyId: NATIVE_KID });

  const none = configFromEnv({});
  assert.equal(none.stamp.signing, null);
  assert.equal(none.stamp.verifyKeys, null);

  for (const env of [
    { STAMP_SIGNING_KEY_PKCS8: "AAAA", STAMP_SIGNING_KEY_ID: HOSTED_KID }, // wrong length
    { STAMP_SIGNING_KEY_PKCS8: HOSTED_PKCS8, STAMP_SIGNING_KEY_ID: "bad.kid" }, // separator in id
    { STAMP_SIGNING_KEY_PKCS8: HOSTED_PKCS8 }, // id missing
    { STAMP_VERIFY_KEYS_JSON: "not json" },
    { STAMP_VERIFY_KEYS_JSON: JSON.stringify({ [HOSTED_KID]: "AAAA" }) }, // wrong length
    { STAMP_VERIFY_KEYS_JSON: JSON.stringify({ "bad.kid": HOSTED_PUB }) },
  ]) {
    const config = configFromEnv(env);
    assert.equal(config.stamp.signing, null, JSON.stringify(env));
    assert.equal(config.stamp.verifyKeys, null, JSON.stringify(env));
  }
});
