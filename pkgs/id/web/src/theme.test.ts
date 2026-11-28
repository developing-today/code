import { afterEach, describe, expect, it, vi } from "vitest";
import { initTheme } from "./theme";

function captureThemeKeydownHandler(): (event: KeyboardEvent) => void {
  let handler: ((event: KeyboardEvent) => void) | undefined;
  vi.spyOn(document, "addEventListener").mockImplementation(((
    type: string,
    listener: EventListenerOrEventListenerObject,
  ) => {
    if (type === "keydown") {
      handler = listener as (event: KeyboardEvent) => void;
    }
  }) as typeof document.addEventListener);

  initTheme();
  if (!handler) throw new Error("theme keydown handler was not registered");
  return handler;
}

afterEach(() => {
  vi.restoreAllMocks();
  localStorage.clear();
  document.documentElement.removeAttribute("data-theme");
});

describe("initTheme Alt+T shortcut", () => {
  it.each([
    ["t", "normal T"],
    ["†", "macOS Option+T"],
    ["ˇ", "macOS Option+Shift+T"],
    ["T", "Shift/CapsLock T"],
  ])("cycles the theme for %s from physical KeyT (%s)", (key) => {
    const handler = captureThemeKeydownHandler();
    const event = new KeyboardEvent("keydown", {
      key,
      code: "KeyT",
      altKey: true,
      cancelable: true,
    });

    handler(event);

    expect(event.defaultPrevented).toBe(true);
    expect(document.documentElement.getAttribute("data-theme")).toBe("arch");
  });

  it("does not intercept plain t without Alt", () => {
    const handler = captureThemeKeydownHandler();
    const event = new KeyboardEvent("keydown", {
      key: "t",
      code: "KeyT",
      cancelable: true,
    });

    handler(event);

    expect(event.defaultPrevented).toBe(false);
    expect(document.documentElement.getAttribute("data-theme")).toBe("sneak");
  });

  it("does not intercept Alt with a different physical key", () => {
    const handler = captureThemeKeydownHandler();
    const event = new KeyboardEvent("keydown", {
      key: "a",
      code: "KeyA",
      altKey: true,
      cancelable: true,
    });

    handler(event);

    expect(event.defaultPrevented).toBe(false);
    expect(document.documentElement.getAttribute("data-theme")).toBe("sneak");
  });
});
