// Probe matrix for the #5762 home-resolution contract on the fleet-hosted
// CLI's token file (#8220 unit 3, #14757): HOME unset and HOME empty are
// each a HomeUnresolvedError naming FLEET_HOSTED_TOKEN_FILE; a set HOME
// gives the unchanged default path; the FLEET_HOSTED_TOKEN_FILE override
// never needs HOME.

import assert from "node:assert/strict";
import { test } from "node:test";
import { join } from "node:path";
import { callerTokenFile } from "../src/cli.ts";
import { HomeUnresolvedError, requireHome } from "../src/home.ts";

test("requireHome: unset HOME is a named error naming the setting", () => {
  assert.throws(
    () => requireHome("FLEET_HOSTED_TOKEN_FILE", {}),
    (e: unknown) =>
      e instanceof HomeUnresolvedError &&
      e.setting === "FLEET_HOSTED_TOKEN_FILE" &&
      /HOME is unset/.test(e.message) &&
      /set HOME or set FLEET_HOSTED_TOKEN_FILE/.test(e.message),
  );
});

test("requireHome: empty and blank HOME are named errors", () => {
  for (const home of ["", "   "]) {
    assert.throws(
      () => requireHome("FLEET_HOSTED_TOKEN_FILE", { HOME: home }),
      (e: unknown) => e instanceof HomeUnresolvedError && /HOME is empty/.test(e.message),
    );
  }
});

test("requireHome: a set HOME is returned unchanged", () => {
  assert.equal(requireHome("FLEET_HOSTED_TOKEN_FILE", { HOME: "/home/caller" }), "/home/caller");
});

test("callerTokenFile: the default path is derived from HOME unchanged", () => {
  assert.equal(callerTokenFile({ HOME: "/home/caller" }), join("/home/caller", ".config", "fleet-hosted", "token"));
});

test("callerTokenFile: HOME unset or empty without the override is a named error", () => {
  for (const env of [{}, { HOME: "" }]) {
    assert.throws(
      () => callerTokenFile(env),
      (e: unknown) => e instanceof HomeUnresolvedError && e.setting === "FLEET_HOSTED_TOKEN_FILE",
    );
  }
});

test("callerTokenFile: the override needs no HOME", () => {
  assert.equal(callerTokenFile({ FLEET_HOSTED_TOKEN_FILE: "/run/token" }), "/run/token");
  assert.equal(callerTokenFile({ HOME: "", FLEET_HOSTED_TOKEN_FILE: "/run/token" }), "/run/token");
});
