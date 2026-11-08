import { describe, expect, it } from "vitest";

import { isNotFoundProbeFailure, probeWorkerExists } from "../deploy-probe.mjs";

// Simulated execFileSync failures: with stdio piped, the thrown error
// carries the cf output on .stderr and a generic line on .message.
const cfFailure = (stderr: string) =>
  Object.assign(new Error("Command failed: cf -q workers get pkarr"), { status: 1, stderr });

describe("worker existence probe (#4865)", () => {
  it("reports exists when the probe succeeds", () => {
    expect(probeWorkerExists(() => "{}", "pkarr")).toBe(true);
  });

  it("reports does-not-exist only for the not-found response (API code 10007)", () => {
    const err = cfFailure('{"errors":[{"code":10007,"message":"workers.api.error.not_found"}]}');
    expect(probeWorkerExists(() => { throw err; }, "pkarr")).toBe(false);
  });

  it("reports does-not-exist for cf's rendered not-found message", () => {
    const err = cfFailure("error: script not found: pkarr");
    expect(probeWorkerExists(() => { throw err; }, "pkarr")).toBe(false);
  });

  it("aborts on an authentication failure, with the cf error visible", () => {
    const err = cfFailure('{"errors":[{"code":10000,"message":"Authentication error"}]}');
    expect(() => probeWorkerExists(() => { throw err; }, "pkarr")).toThrow(/aborting before any upload/);
    expect(() => probeWorkerExists(() => { throw err; }, "pkarr")).toThrow(/Authentication error/);
  });

  it("aborts on a network failure instead of bootstrapping", () => {
    const err = cfFailure("error: request to https://api.cloudflare.com failed: connection reset");
    expect(() => probeWorkerExists(() => { throw err; }, "pkarr")).toThrow(/aborting before any upload/);
  });

  it("aborts on a spawn failure (broken cf install)", () => {
    const err = Object.assign(new Error("spawnSync cf ENOENT"), { code: "ENOENT" });
    expect(() => probeWorkerExists(() => { throw err; }, "pkarr")).toThrow(/ENOENT/);
  });

  it("does not treat an unrelated not-found as the Worker missing", () => {
    const err = cfFailure('{"errors":[{"code":10003,"message":"account not found"}]}');
    expect(isNotFoundProbeFailure(err.stderr)).toBe(false);
    expect(() => probeWorkerExists(() => { throw err; }, "pkarr")).toThrow(/aborting before any upload/);
  });
});
