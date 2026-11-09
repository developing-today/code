import { describe, expect, it, vi } from "vitest";

import { verifyDeployment } from "../deploy-verification.mjs";

const healthUrl = "https://example.workers.dev/healthz";
const deployments = [
  {
    created_on: "2026-10-31T00:00:00Z",
    versions: [{ version_id: "old-version", percentage: 100 }],
  },
  {
    created_on: "2026-11-01T00:00:00Z",
    versions: [{ version_id: "new-version", percentage: 100 }],
  },
];

describe("post-deploy verification", () => {
  it("accepts the requested active version when /healthz returns ok", async () => {
    const fetchHealth = vi.fn(async () => ({ ok: true, status: 200, text: async () => "ok\n" }));

    await expect(
      verifyDeployment({ versionId: "new-version", deployments, healthUrl, fetchHealth })
    ).resolves.toEqual({ versionId: "new-version", healthUrl });
    expect(fetchHealth).toHaveBeenCalledWith(healthUrl);
  });

  it("rejects when a different version is active and does not probe health", async () => {
    const fetchHealth = vi.fn();

    await expect(
      verifyDeployment({ versionId: "requested-version", deployments, healthUrl, fetchHealth })
    ).rejects.toThrow(/expected version requested-version at 100%/);
    expect(fetchHealth).not.toHaveBeenCalled();
  });

  it("rejects an unhealthy HTTP response", async () => {
    const fetchHealth = vi.fn(async () => ({ ok: false, status: 503, text: async () => "unavailable" }));

    await expect(
      verifyDeployment({ versionId: "new-version", deployments, healthUrl, fetchHealth })
    ).rejects.toThrow(/health check failed.*HTTP 503/);
  });

  it("rejects a healthy status with an unexpected response body", async () => {
    const fetchHealth = vi.fn(async () => ({ ok: true, status: 200, text: async () => "not ready" }));

    await expect(
      verifyDeployment({ versionId: "new-version", deployments, healthUrl, fetchHealth })
    ).rejects.toThrow(/unexpected body/);
  });

  it("reports a failed health request clearly", async () => {
    const fetchHealth = vi.fn(async () => {
      throw new Error("connection reset");
    });

    await expect(
      verifyDeployment({ versionId: "new-version", deployments, healthUrl, fetchHealth })
    ).rejects.toThrow(/health check request failed.*connection reset/);
  });
});
