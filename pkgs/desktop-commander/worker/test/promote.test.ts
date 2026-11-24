import { describe, expect, it } from "vitest";
import { deploymentStatus } from "../promote.mjs";

// Oracle: the promote drift contract in pkgs/desktop-commander/worker/README.md —
// promotion proceeds only at the manifest baseline, is a no-op success when the
// verified upload is already the sole 100% deployment, and refuses any other
// live state (#13472: the target version had already been deployed and the
// baseline-only check misread that success as drift).

const manifest = {
  versionId: "37e2f6a3-f2ed-4f77-a84a-a7acf67f6ff4",
  baselineVersionId: "c2ac67bb-f7ab-4912-a820-aa2ead4aa908",
};

describe("promotion drift classification", () => {
  it("treats the verified upload already serving 100% as already-promoted", () => {
    expect(deploymentStatus({ versions: [{ version_id: manifest.versionId, percentage: 100 }] }, manifest)).toBe(
      "already-promoted",
    );
  });

  it("allows promotion when the live deployment still matches the baseline", () => {
    expect(
      deploymentStatus({ versions: [{ version_id: manifest.baselineVersionId, percentage: 100 }] }, manifest),
    ).toBe("at-baseline");
  });

  it("refuses drift to an unrelated active version", () => {
    expect(
      deploymentStatus(
        { versions: [{ version_id: "00000000-0000-0000-0000-000000000000", percentage: 100 }] },
        manifest,
      ),
    ).toBe("drifted");
  });

  it("refuses a partial rollout of the verified upload instead of guessing", () => {
    expect(
      deploymentStatus(
        {
          versions: [
            { version_id: manifest.versionId, percentage: 50 },
            { version_id: manifest.baselineVersionId, percentage: 50 },
          ],
        },
        manifest,
      ),
    ).toBe("drifted");
  });

  it("refuses an empty or malformed deployment record", () => {
    expect(deploymentStatus(undefined, manifest)).toBe("drifted");
    expect(deploymentStatus({}, manifest)).toBe("drifted");
    expect(deploymentStatus({ versions: [] }, manifest)).toBe("drifted");
  });
});
