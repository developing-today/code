import { describe, expect, it } from "vitest";
import { renderCapabilityGrants } from "./capability-grants";

describe("renderCapabilityGrants", () => {
  it("renders fixed permission text and the baseline timeline without raw keys", () => {
    const root = document.createElement("div");
    renderCapabilityGrants(root, [
      {
        world_id: "arena",
        current: [
          {
            class: "device.location",
            message: "Use this device's location.",
            deniable: true,
            load_bearing: false,
          },
        ],
        history: [
          { at: "2026-11-25T08:00:00Z", kind: "accept", message_set: ["device.location"] },
          { at: "2026-11-25T09:00:00Z", kind: "narrow", message_set: [] },
        ],
      },
    ]);

    expect(root.textContent).toContain("arena");
    expect(root.textContent).toContain("Use this device's location.");
    expect(root.textContent).toContain("Remembered after connecting");
    expect(root.textContent).toContain("Narrowed after a permission was removed");
    expect(root.textContent).not.toContain("baseline_key");
    expect(root.textContent).not.toContain("release_identity");
  });

  it("renders untrusted permission copy as text rather than markup", () => {
    const root = document.createElement("div");
    renderCapabilityGrants(root, [
      {
        world_id: "arena",
        current: [{ class: "device.location", message: "<img src=x onerror=alert(1)>", deniable: true, load_bearing: false }],
        history: [],
      },
    ]);

    expect(root.querySelector("img")).toBeNull();
    expect(root.textContent).toContain("<img src=x onerror=alert(1)>");
  });

  it("shows an explicit empty state", () => {
    const root = document.createElement("div");
    renderCapabilityGrants(root, []);
    expect(root.textContent).toBe("No remembered permission decisions yet.");
  });
});
