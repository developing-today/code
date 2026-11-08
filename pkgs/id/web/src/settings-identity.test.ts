import { describe, expect, it } from "vitest";
import { clientIdForDisplay } from "./settings-identity";

describe("clientIdForDisplay", () => {
  it("prefers the in-memory identity", () => {
    expect(clientIdForDisplay("abcd1234", "ffff0000")).toBe("abcd1234");
  });

  it("falls back to the persisted identity (stable across navigation)", () => {
    expect(clientIdForDisplay(null, "ffff0000")).toBe("ffff0000");
    expect(clientIdForDisplay(undefined, "ffff0000")).toBe("ffff0000");
  });

  it("never renders a missing or all-zeros id (#1792: 'shows 0s')", () => {
    expect(clientIdForDisplay(null, null)).toBe("Not registered yet");
    expect(clientIdForDisplay("", "")).toBe("Not registered yet");
    expect(clientIdForDisplay("0", null)).toBe("Not registered yet");
    expect(clientIdForDisplay("0000000000000000", null)).toBe(
      "Not registered yet",
    );
  });
});
