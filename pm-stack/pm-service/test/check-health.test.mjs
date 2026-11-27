// Oracle: deployedAt plus elapsed seconds must account for the health
// response age within five seconds of this check's clock.
import { expect, test } from "bun:test";
import { matchesHealth } from "../scripts/check-health.mjs";

const VERSION = "a".repeat(40);

test("the deploy gate accepts the exact version with an age matching its timestamp", () => {
  const now = Date.parse("2026-11-27T10:00:00.000Z");
  expect(matchesHealth({
    ok: true,
    version: VERSION,
    deployedAt: "2026-11-27T09:59:48.000Z",
    ageSeconds: 12,
  }, VERSION, now)).toBe(true);
});

test("the deploy gate rejects a missing, stale, or mismatched health age", () => {
  const now = Date.parse("2026-11-27T10:00:00.000Z");
  const current = { ok: true, version: VERSION, deployedAt: "2026-11-27T09:59:48.000Z", ageSeconds: 12 };
  expect(matchesHealth({ ...current, version: "b".repeat(40) }, VERSION, now)).toBe(false);
  expect(matchesHealth({ ...current, ageSeconds: 3 }, VERSION, now)).toBe(false);
  expect(matchesHealth({ ok: true, version: VERSION }, VERSION, now)).toBe(false);
  expect(matchesHealth({ ...current, deployedAt: "invalid" }, VERSION, now)).toBe(false);
});
