import { describe, expect, it } from "vitest";

import { parseArgs, USAGE } from "../deploy-args.mjs";

describe("deploy argument parsing", () => {
  it("accepts no arguments", () => {
    expect(parseArgs([])).toEqual({ dryRun: false, deployVersion: null });
  });

  it("accepts --dry-run", () => {
    expect(parseArgs(["--dry-run"])).toEqual({ dryRun: true, deployVersion: null });
  });

  it("accepts --deploy-version with a separate value", () => {
    expect(parseArgs(["--deploy-version", "v-rollback-123"])).toEqual({
      dryRun: false,
      deployVersion: "v-rollback-123",
    });
  });

  it("accepts the --deploy-version=<id> equals form", () => {
    expect(parseArgs(["--deploy-version=v-rollback-123"])).toEqual({
      dryRun: false,
      deployVersion: "v-rollback-123",
    });
  });

  it("keeps both flags when combined, in either order", () => {
    const expected = { dryRun: true, deployVersion: "v-rollback-123" };
    expect(parseArgs(["--dry-run", "--deploy-version", "v-rollback-123"])).toEqual(expected);
    expect(parseArgs(["--deploy-version=v-rollback-123", "--dry-run"])).toEqual(expected);
  });

  it("refuses a misspelled safety flag instead of deploying", () => {
    expect(() => parseArgs(["--dry-rnu"])).toThrow(/unrecognized argument "--dry-rnu"/);
  });

  it("refuses an unknown flag", () => {
    expect(() => parseArgs(["--prod"])).toThrow(/unrecognized argument "--prod"/);
  });

  it("refuses --dry-run with a value", () => {
    expect(() => parseArgs(["--dry-run=true"])).toThrow(/unrecognized argument "--dry-run=true"/);
  });

  it("refuses a bare positional argument", () => {
    expect(() => parseArgs(["deploy"])).toThrow(/unrecognized argument "deploy"/);
  });

  it("refuses --deploy-version without a value", () => {
    expect(() => parseArgs(["--deploy-version"])).toThrow(/needs a version id/);
  });

  it("refuses --deploy-version followed by another flag", () => {
    expect(() => parseArgs(["--deploy-version", "--dry-run"])).toThrow(/needs a version id/);
  });

  it("refuses an empty equals-form version id", () => {
    expect(() => parseArgs(["--deploy-version="])).toThrow(/needs a version id/);
  });

  it("prints usage with every refusal", () => {
    for (const argv of [["--dry-rnu"], ["--deploy-version"], ["--deploy-version="]]) {
      expect(() => parseArgs(argv)).toThrow(USAGE);
    }
  });
});
