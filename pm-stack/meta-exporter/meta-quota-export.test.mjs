// Tests for the pure parsing core of meta-quota-export.mjs.
// Run: node --test pm-stack/meta-exporter/
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  ExportError,
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

// The live payload captured 2026-10-29 (#1301): after the short window
// rolled at 16:22Z it went inactive — window_weighted_used "0" and NO
// window_resets_at key at all — while the weekly window stayed live at
// 23.62%. Every export tick failed on this shape (16:15Z onward) until
// the parser learned to tolerate a missing reset on a not-yet-started
// (zero-usage) window.
const QUOTA_INACTIVE_SHORT = {
  subscription_quota: {
    tier_id: "27681527378179523",
    tier: "Muse Code High Usage",
    as_of: 1793328541,
    window_weighted_limit: "100000000000",
    window_duration_secs: 18000,
    weekly_weighted_limit: "300000000000",
    weekly_resets_at: 1793577600,
    window_weighted_used: "0",
    weekly_weighted_used: "70873201100",
  },
};

test("parseSubscriptionQuota: inactive zero-usage window with absent reset is tolerated", () => {
  const parsed = parseSubscriptionQuota(QUOTA_INACTIVE_SHORT);
  assert.deepEqual(parsed.windows, [
    { name: "5h", usedPct: 0, resetAt: null },
    { name: "weekly", usedPct: 23.62, resetAt: "2026-11-02T00:00:00.000Z" },
  ]);
});

test("parseSubscriptionQuota: inactive zero-usage window with explicit null reset is tolerated", () => {
  const payload = structuredClone(QUOTA_INACTIVE_SHORT);
  payload.subscription_quota.window_resets_at = null;
  const parsed = parseSubscriptionQuota(payload);
  assert.equal(parsed.windows[0].resetAt, null);
  assert.equal(parsed.windows[1].usedPct, 23.62);
  // The same tolerance applies to the weekly window when it has not
  // started either.
  const weeklyInactive = structuredClone(QUOTA);
  weeklyInactive.subscription_quota.weekly_weighted_used = "0";
  weeklyInactive.subscription_quota.weekly_resets_at = null;
  assert.equal(parseSubscriptionQuota(weeklyInactive).windows[1].resetAt, null);
});

test("parseSubscriptionQuota: null reset on an active window still fails closed", () => {
  // Active = the window has started, i.e. it carries recorded usage
  // (the 2026-10-27 capture above); a reset must exist for it. The
  // inactive-window tolerance must not become blanket null tolerance.
  const shortActive = structuredClone(QUOTA);
  shortActive.subscription_quota.window_resets_at = null;
  assert.throws(() => parseSubscriptionQuota(shortActive), /windowReset/);
  const weeklyActive = structuredClone(QUOTA);
  delete weeklyActive.subscription_quota.weekly_resets_at;
  assert.throws(() => parseSubscriptionQuota(weeklyActive), /weeklyReset/);
});

test("parseSubscriptionQuota: absent reset with usage above zero still fails closed", () => {
  const payload = structuredClone(QUOTA_INACTIVE_SHORT);
  payload.subscription_quota.window_weighted_used = "1"; // any usage at all
  assert.throws(() => parseSubscriptionQuota(payload), /windowReset/);
});

test("parseSubscriptionQuota: malformed reset fails closed even at zero usage", () => {
  for (const bad of ["soon", -5, true, {}]) {
    const payload = structuredClone(QUOTA_INACTIVE_SHORT);
    payload.subscription_quota.window_resets_at = bad;
    assert.throws(() => parseSubscriptionQuota(payload), /windowReset not a finite number/);
  }
});

// #2789: num() used to be a bare Number() coercion, so drifted
// spellings became real measurements ("" -> 0% window, "0x10" -> 16,
// "1e2" -> 100). Only plain decimal spellings are accepted now.
test("parseSubscriptionQuota: non-decimal numeric spellings fail closed", () => {
  for (const bad of ["", "   ", "0x10", "0b101", "0o17", "1e2", "1E2", "abc", "12,000", " 123", "123 ", "1023570000n"]) {
    for (const field of ["window_weighted_used", "window_weighted_limit", "window_duration_secs"]) {
      const payload = structuredClone(QUOTA);
      payload.subscription_quota[field] = bad;
      assert.throws(() => parseSubscriptionQuota(payload), /not a finite|window_duration_secs/);
    }
  }
  // The same spellings are rejected on the weekly fields too.
  for (const bad of ["", "0x10", "1e2"]) {
    const payload = structuredClone(QUOTA);
    payload.subscription_quota.weekly_weighted_used = bad;
    assert.throws(() => parseSubscriptionQuota(payload), /weeklyUsed/);
  }
});

test("parseSubscriptionQuota: empty-string reset on a used window fails closed", () => {
  for (const bad of ["", "   "]) {
    const shortUsed = structuredClone(QUOTA);
    shortUsed.subscription_quota.window_resets_at = bad;
    assert.throws(() => parseSubscriptionQuota(shortUsed), /windowReset/);
    const weeklyUsed = structuredClone(QUOTA);
    weeklyUsed.subscription_quota.weekly_resets_at = bad;
    assert.throws(() => parseSubscriptionQuota(weeklyUsed), /weeklyReset/);
  }
});

test("parseSubscriptionQuota: empty-string reset on a zero-usage window is tolerated like absent", () => {
  const payload = structuredClone(QUOTA_INACTIVE_SHORT);
  payload.subscription_quota.window_resets_at = "";
  assert.equal(parseSubscriptionQuota(payload).windows[0].resetAt, null);
});

test("parseSubscriptionQuota: decimal spellings still parse, including fractions", () => {
  const frac = structuredClone(QUOTA);
  frac.subscription_quota.window_weighted_used = "1023570000.5";
  assert.equal(parseSubscriptionQuota(frac).windows[0].usedPct, 1.02);
});

test("parseSubscriptionQuota: empty as_of stays null; non-decimal as_of fails closed", () => {
  const empty = structuredClone(QUOTA);
  empty.subscription_quota.as_of = "";
  assert.equal(parseSubscriptionQuota(empty).consoleAsOf, null);
  const blank = structuredClone(QUOTA);
  delete blank.subscription_quota.as_of;
  assert.equal(parseSubscriptionQuota(blank).consoleAsOf, null);
  for (const bad of ["0x10", "1e2", "soon"]) {
    const payload = structuredClone(QUOTA);
    payload.subscription_quota.as_of = bad;
    assert.throws(() => parseSubscriptionQuota(payload), /unparseable|finite/);
  }
});

test("parseSubscriptionQuota: epoch fields reject values outside the pinned seconds window", () => {
  const firstAllowed = 1_577_836_800; // 2020-01-01T00:00:00Z
  const lastAllowed = 4_102_444_800; // 2100-01-01T00:00:00Z
  const cases = [
    ["window_resets_at", [firstAllowed - 1, lastAllowed + 1, 0, 1_793_090_405_000, 1e300]],
    ["weekly_resets_at", [firstAllowed - 1, lastAllowed + 1, 0, 1_793_090_405_000, 1e300]],
    ["as_of", [firstAllowed - 1, lastAllowed + 1, 0, 1_793_000_000_000, 1e300]],
  ];
  for (const [field, values] of cases) {
    for (const value of values) {
      const payload = structuredClone(QUOTA);
      payload.subscription_quota[field] = value;
      assert.throws(
        () => parseSubscriptionQuota(payload),
        (error) => {
          assert.ok(error instanceof ExportError);
          assert.equal(error.status, "unparseable");
          assert.equal(error.exitCode, 6);
          return true;
        },
        `${field}=${value} should fail as unparseable`,
      );
    }
  }
});

test("parseSubscriptionQuota: epoch window endpoints are inclusive", () => {
  const endpoints = [
    [1_577_836_800, "2020-01-01T00:00:00.000Z"],
    [4_102_444_800, "2100-01-01T00:00:00.000Z"],
  ];
  for (const [epoch, expected] of endpoints) {
    for (const field of ["window_resets_at", "weekly_resets_at"]) {
      const payload = structuredClone(QUOTA);
      payload.subscription_quota[field] = epoch;
      const parsed = parseSubscriptionQuota(payload);
      assert.equal(parsed.windows[field === "window_resets_at" ? 0 : 1].resetAt, expected);
    }
    const payload = structuredClone(QUOTA);
    payload.subscription_quota.as_of = epoch;
    assert.equal(parseSubscriptionQuota(payload).consoleAsOf, expected);
  }
});
