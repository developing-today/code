import { describe, expect, test } from "bun:test";
import { hostedDecisionPageUrl } from "../src/decision-links.ts";

describe("hosted decision links", () => {
  test("use the canonical read surface without carrying the per-set credential — oracle: exact URL", () => {
    expect(hostedDecisionPageUrl("set/with space?&"))
      .toBe("https://pm.security.cab/decisions?set=set%2Fwith%20space%3F%26");
  });
});
