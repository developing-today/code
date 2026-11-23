// Tenancy mapping tests (#12583). Oracle: design §4.1 + component E's
// keying rule — accounts are keyed by the minted binding_id, never by
// email or provider subject; the directory record is the only mapping,
// so an unmapped subject resolves to nothing (fail closed), whatever
// its string looks like.

import assert from "node:assert/strict";
import { test } from "node:test";
import { sha256Hex } from "../src/auth.ts";
import { BINDING_ID_RE, FixtureTenancyDirectory, parseTenancyFixture } from "../src/tenancy.ts";
import { BINDING_A, ISSUER, SERVICE_TOKEN } from "./helpers.ts";

test("a mapped subject resolves to its binding_id and active flag", async () => {
  const dir = new FixtureTenancyDirectory({
    subjects: [{ issuer: ISSUER, subject: "user-sub-1", binding_id: BINDING_A, active: true }],
  });
  const record = await dir.resolveSubject(ISSUER, "user-sub-1");
  assert.deepEqual(record, { bindingId: BINDING_A, active: true });
});

test("an unmapped subject resolves to null, even an email-shaped one", async () => {
  const dir = new FixtureTenancyDirectory({ subjects: [] });
  // Keying rule: identity is never derived from the subject string —
  // an email-shaped subject with no directory record is nothing.
  assert.equal(await dir.resolveSubject(ISSUER, "drewrypope@gmail.com"), null);
  assert.equal(await dir.resolveSubject(ISSUER, "user-sub-1"), null);
});

test("the same subject under another issuer is a different mapping", async () => {
  const dir = new FixtureTenancyDirectory({
    subjects: [{ issuer: ISSUER, subject: "user-sub-1", binding_id: BINDING_A, active: true }],
  });
  assert.equal(await dir.resolveSubject("https://other-idp.test", "user-sub-1"), null);
});

test("a service token is found by its SHA-256, never by plaintext shape", async () => {
  const dir = new FixtureTenancyDirectory({
    service_tokens: [
      {
        sha256: await sha256Hex(SERVICE_TOKEN),
        binding_id: BINDING_A,
        routes: ["stamping"],
        active: true,
        label: "pm-service-test",
      },
    ],
  });
  const hit = await dir.findServiceToken(await sha256Hex(SERVICE_TOKEN));
  assert.equal(hit?.bindingId, BINDING_A);
  assert.deepEqual(hit?.routes, ["stamping"]);
  assert.equal(await dir.findServiceToken(await sha256Hex("some-other-token")), null);
});

test("a fixture with a non-minted binding_id is rejected at construction", () => {
  assert.throws(
    () =>
      new FixtureTenancyDirectory({
        subjects: [{ issuer: ISSUER, subject: "s", binding_id: "not-a-binding", active: true }],
      }),
    /not a minted binding id/,
  );
  assert.throws(
    () => parseTenancyFixture('{"subjects": [{"issuer":"i","subject":"s","binding_id":"nope","active":true}]}'),
    /not a minted binding id/,
  );
});

test("the binding_id shape is the minted 32-lowercase-hex contract", () => {
  assert.ok(BINDING_ID_RE.test(BINDING_A));
  assert.ok(!BINDING_ID_RE.test("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"));
  assert.ok(!BINDING_ID_RE.test("aaaa"));
  assert.ok(!BINDING_ID_RE.test("drewrypope@gmail.com"));
});
