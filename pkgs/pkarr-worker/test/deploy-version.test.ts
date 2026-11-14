import { describe, expect, it, vi } from "vitest";

import { parseArgs } from "../deploy-args.mjs";
import { deployExistingVersion } from "../deploy-version.mjs";

// The flag x path matrix for the --deploy-version branch (#3252):
// --dry-run is a rollback PREVIEW and must never reach createDeployment;
// without it the branch deploys exactly as it did before the flag was
// honored (create, verify/report, live line, in that order).
const harness = () => {
  const calls: string[] = [];
  const logs: string[] = [];
  return {
    calls,
    logs,
    createDeployment: vi.fn((versionId: string) => {
      calls.push(`create:${versionId}`);
    }),
    reportLive: vi.fn(async (versionId: string) => {
      calls.push(`report:${versionId}`);
    }),
    log: (message: string) => {
      logs.push(message);
    },
  };
};

const base = { worker: "pkarr", workerUrl: "https://pkarr.example" };

describe("deploy-version branch (#3252)", () => {
  it("--deploy-version with --dry-run creates no deployment and reports the preview", async () => {
    const h = harness();
    const result = await deployExistingVersion({
      ...base,
      dryRun: true,
      deployVersion: "v-rollback-123",
      createDeployment: h.createDeployment,
      reportLive: h.reportLive,
      log: h.log,
    });
    expect(h.createDeployment).not.toHaveBeenCalled();
    expect(h.reportLive).not.toHaveBeenCalled();
    expect(result).toEqual({ deployed: false, dryRun: true, versionId: "v-rollback-123" });
    expect(h.logs.join("\n")).toMatch(/dry run/);
    expect(h.logs.join("\n")).toContain("pkarr");
    expect(h.logs.join("\n")).toContain("v-rollback-123");
  });

  it("--deploy-version without --dry-run deploys as before: create, report, live line", async () => {
    const h = harness();
    const result = await deployExistingVersion({
      ...base,
      dryRun: false,
      deployVersion: "v-rollback-123",
      createDeployment: h.createDeployment,
      reportLive: h.reportLive,
      log: h.log,
    });
    expect(h.createDeployment).toHaveBeenCalledTimes(1);
    expect(h.createDeployment).toHaveBeenCalledWith("v-rollback-123");
    expect(h.reportLive).toHaveBeenCalledTimes(1);
    expect(h.reportLive).toHaveBeenCalledWith("v-rollback-123");
    expect(h.calls).toEqual(["create:v-rollback-123", "report:v-rollback-123"]);
    expect(result).toEqual({ deployed: true, dryRun: false, versionId: "v-rollback-123" });
    expect(h.logs.join("\n")).toContain("live: https://pkarr.example at version v-rollback-123");
  });

  it("the parsed flag matrix drives the branch: preview only when --dry-run is parsed", async () => {
    for (const argv of [
      ["--deploy-version", "v-rollback-123", "--dry-run"],
      ["--dry-run", "--deploy-version=v-rollback-123"],
    ]) {
      const h = harness();
      const { dryRun, deployVersion } = parseArgs(argv);
      await deployExistingVersion({
        ...base,
        dryRun,
        deployVersion: deployVersion ?? "",
        createDeployment: h.createDeployment,
        reportLive: h.reportLive,
        log: h.log,
      });
      expect(h.createDeployment).not.toHaveBeenCalled();
    }
    const h = harness();
    const parsed = parseArgs(["--deploy-version", "v-rollback-123"]);
    await deployExistingVersion({
      ...base,
      dryRun: parsed.dryRun,
      deployVersion: parsed.deployVersion ?? "",
      createDeployment: h.createDeployment,
      reportLive: h.reportLive,
      log: h.log,
    });
    expect(h.createDeployment).toHaveBeenCalledTimes(1);
  });
});
