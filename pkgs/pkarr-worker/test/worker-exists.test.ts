import { describe, expect, it } from "vitest";

import { workerExists } from "../worker-exists.mjs";

function failedProbe(message: string, stderr = "") {
  const error = new Error(message) as Error & { stderr?: Buffer };
  if (stderr) error.stderr = Buffer.from(stderr);
  return error;
}

describe("Cloudflare Worker existence probe", () => {
  it("returns true when the probe succeeds", () => {
    expect(workerExists("pkarr", () => ({ result: { id: "pkarr" } }))).toBe(true);
  });

  it("selects bootstrap only for cf's typed HTTP 404 response", () => {
    const error = failedProbe(
      "Command failed: cf -q workers get pkarr",
      'NotFoundError\nStatus code: 404\nBody: {"errors":[{"code":10007,"message":"Worker not found"}]}',
    );

    expect(workerExists("pkarr", () => { throw error; })).toBe(false);
  });

  it.each([
    ["authentication failure", failedProbe("ApiError", "Status code: 401\nBody: unauthorized"), /Status code: 401/],
    ["permission failure", failedProbe("ApiError", "Status code: 403\nBody: forbidden"), /Status code: 403/],
    ["unexpected 404", failedProbe("ApiError", "Status code: 404\nBody: unrelated endpoint"), /Status code: 404/],
    ["network failure", failedProbe("connect ETIMEDOUT while probing Worker"), /ETIMEDOUT/],
    ["missing cf executable", failedProbe("spawnSync cf ENOENT"), /ENOENT/],
  ])("fails closed on %s", (_kind, error, diagnostic) => {
    expect(() => workerExists("pkarr", () => { throw error; })).toThrow(diagnostic);
  });
});
