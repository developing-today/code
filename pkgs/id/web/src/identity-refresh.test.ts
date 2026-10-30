import { describe, expect, it } from "vitest";
import { identityNeedsRefresh } from "./identity-refresh";

const INTERVAL = 24 * 60 * 60 * 1000;
const NOW = 1_800_000_000_000;

describe("identityNeedsRefresh", () => {
  it("needs refresh when nothing is stored", () => {
    expect(identityNeedsRefresh(null, NOW, INTERVAL)).toBe(true);
    expect(identityNeedsRefresh("", NOW, INTERVAL)).toBe(true);
  });

  it("needs refresh for a non-numeric or non-finite stored value (#847)", () => {
    // Number("garbage") is NaN and NaN >= interval is false, so the old
    // inline check returned false forever for these values.
    expect(identityNeedsRefresh("garbage", NOW, INTERVAL)).toBe(true);
    expect(identityNeedsRefresh("12x", NOW, INTERVAL)).toBe(true);
    expect(identityNeedsRefresh("Infinity", NOW, INTERVAL)).toBe(true);
    expect(identityNeedsRefresh("NaN", NOW, INTERVAL)).toBe(true);
  });

  it("needs refresh for a timestamp in the future", () => {
    expect(identityNeedsRefresh(String(NOW + 1), NOW, INTERVAL)).toBe(true);
    expect(identityNeedsRefresh(String(NOW + INTERVAL * 365), NOW, INTERVAL)).toBe(true);
  });

  it("does not need refresh for a recent timestamp", () => {
    expect(identityNeedsRefresh(String(NOW), NOW, INTERVAL)).toBe(false);
    expect(identityNeedsRefresh(String(NOW - INTERVAL + 1), NOW, INTERVAL)).toBe(false);
  });

  it("needs refresh once the interval has elapsed", () => {
    expect(identityNeedsRefresh(String(NOW - INTERVAL), NOW, INTERVAL)).toBe(true);
    expect(identityNeedsRefresh(String(NOW - INTERVAL * 30), NOW, INTERVAL)).toBe(true);
  });
});
