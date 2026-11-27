/**
 * Oracle: rollback capture must return the only version serving 100% of the
 * current deployment, and must reject missing, gradual, or malformed state
 * before a new version is uploaded.
 */
import { describe, expect, test } from "bun:test";
import { activeVersionId } from "../scripts/active-version.mjs";

const VERSION_ID = "a61e815c-6ab5-4190-b438-54e75cbe26fc";

function deployment(versions: unknown) {
  return { versions };
}

describe("active pm-service deployment version", () => {
  test("returns the sole version serving all production traffic", () => {
    expect(activeVersionId(deployment([{ version_id: VERSION_ID, percentage: 100 }]))).toBe(VERSION_ID);
  });

  test("rejects split deployments instead of choosing a rollback target", () => {
    expect(() =>
      activeVersionId(
        deployment([
          { version_id: VERSION_ID, percentage: 50 },
          { version_id: "00000000-0000-4000-8000-000000000001", percentage: 50 },
        ]),
      ),
    ).toThrow("expected one active pm-service deployment version");
  });

  test("rejects an empty deployment state before uploading", () => {
    expect(() => activeVersionId(deployment([]))).toThrow("expected one active pm-service deployment version");
  });

  test("rejects malformed version identifiers before they reach wrangler", () => {
    expect(() => activeVersionId(deployment([{ version_id: "latest", percentage: 100 }]))).toThrow(
      "pm-service deployment has an invalid version_id",
    );
  });
});
