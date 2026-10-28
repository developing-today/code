// Tests for the pure parsing core of meta-quota-export.mjs.
// Run: node --test pm-stack/meta-exporter/
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  failureDocument,
  parseBillingBanner,
  parsePaygBalance,
  parseSubscriptionQuota,
  successDocument,
} from "./meta-quota-export.mjs";

// The live payload captured 2026-10-27 (#1167): 1.02% / 8.83%.
const QUOTA = {
  subscription_quota: {
    tier_id: "27681527378179523",
    tier: "Muse Code High Usage",
    as_of: 1793077399,
    window_weighted_limit: "100000000000",
    window_duration_secs: 18000,
    window_weighted_used: "1023570000",
    window_resets_at: 1793090405,
    weekly_weighted_limit: "300000000000",
    weekly_weighted_used: "26484231500",
    weekly_resets_at: 1793577600,
  },
};

test("parseSubscriptionQuota: percentages and resets match the console", () => {
  const parsed = parseSubscriptionQuota(QUOTA);
  assert.equal(parsed.tier, "Muse Code High Usage");
  assert.deepEqual(parsed.windows, [
    { name: "5h", usedPct: 1.02, resetAt: "2026-10-27T08:40:05.000Z" },
    { name: "weekly", usedPct: 8.83, resetAt: "2026-11-02T00:00:00.000Z" },
  ]);
  assert.equal(parsed.consoleAsOf, "2026-10-27T05:03:19.000Z"); // 00:03:19 CDT
});

test("parseSubscriptionQuota: shape drift fails closed", () => {
  assert.throws(() => parseSubscriptionQuota({}), /unparseable|missing/);
  assert.throws(() => parseSubscriptionQuota({ subscription_quota: { window_weighted_used: "1" } }));
  const zeroLimit = structuredClone(QUOTA);
  zeroLimit.subscription_quota.window_weighted_limit = "0";
  assert.throws(() => parseSubscriptionQuota(zeroLimit), /zero limit/);
  // window_duration_secs is validated like every other numeric field:
  // missing, non-numeric, zero, or negative durations fail closed
  // instead of exporting a "window-NaNs" window as a successful reading.
  for (const bad of [undefined, "abc", 0, -3600]) {
    const drifted = structuredClone(QUOTA);
    if (bad === undefined) delete drifted.subscription_quota.window_duration_secs;
    else drifted.subscription_quota.window_duration_secs = bad;
    assert.throws(() => parseSubscriptionQuota(drifted), /window_duration_secs/);
  }
});

test("parseSubscriptionQuota: window naming for valid durations is unchanged", () => {
  const oneHour = structuredClone(QUOTA);
  oneHour.subscription_quota.window_duration_secs = 3600;
  assert.equal(parseSubscriptionQuota(oneHour).windows[0].name, "1h");
  const twoHours = structuredClone(QUOTA);
  twoHours.subscription_quota.window_duration_secs = 7200;
  assert.equal(parseSubscriptionQuota(twoHours).windows[0].name, "window-7200s");
});

test("parseBillingBanner: minor units to USD, currency checked", () => {
  assert.equal(parseBillingBanner({ kind: "free_partial", remaining_minor_units: 14857, currency: "USD" }), 148.57);
  assert.throws(() => parseBillingBanner({ remaining_minor_units: 100, currency: "EUR" }), /currency/);
  assert.throws(() => parseBillingBanner({}));
});

test("parsePaygBalance: reads the iframe text, absence stays null", () => {
  assert.equal(parsePaygBalance("Pay as you go\nCurrent balance\n$0.00\nYour balance won't increase"), 0);
  assert.equal(parsePaygBalance("Current balance\n$1,234.56"), 1234.56);
  assert.equal(parsePaygBalance("nothing relevant here"), null);
  assert.equal(parsePaygBalance(undefined), null);
});

test("documents: success carries the spec fields; failure carries none", () => {
  const ok = successDocument({
    measuredAt: "2026-10-27T05:00:00.000Z",
    parsed: parseSubscriptionQuota(QUOTA),
    freeCreditsUsd: 148.57,
    paygBalanceUsd: 0,
  });
  assert.equal(ok.service, "meta");
  assert.equal(ok.source, "file-export/meta-laptop");
  assert.equal(ok.status, "ok");
  assert.equal(ok.windows.length, 2);

  const bad = failureDocument({ attemptedAt: "2026-10-27T05:00:00.000Z", status: "session-expired", detail: "HTTP 401" });
  assert.equal(bad.measuredAt, null);
  assert.equal("windows" in bad, false);
  assert.equal("freeCreditsUsd" in bad, false);
});
