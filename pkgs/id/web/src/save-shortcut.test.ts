/**
 * Tests for the save-shortcut predicate (#2951).
 */

import { describe, expect, it } from "vitest";
import { isSaveShortcut } from "./save-shortcut";

function ev(partial: Partial<{
  ctrlKey: boolean; metaKey: boolean; shiftKey: boolean; code: string; key: string;
}>) {
  return {
    ctrlKey: false, metaKey: false, shiftKey: false, code: "", key: "",
    ...partial,
  };
}

describe("isSaveShortcut", () => {
  it("matches normal Ctrl+S", () => {
    expect(isSaveShortcut(ev({ ctrlKey: true, code: "KeyS", key: "s" }))).toBe(true);
  });

  it("matches Cmd+S", () => {
    expect(isSaveShortcut(ev({ metaKey: true, code: "KeyS", key: "s" }))).toBe(true);
  });

  it("matches CapsLock Ctrl+S (key 'S', shiftKey false)", () => {
    expect(isSaveShortcut(ev({ ctrlKey: true, code: "KeyS", key: "S" }))).toBe(true);
  });

  it("matches non-Latin layout Ctrl+S (key 'ы', code 'KeyS')", () => {
    expect(isSaveShortcut(ev({ ctrlKey: true, code: "KeyS", key: "ы" }))).toBe(true);
  });

  it("excludes Ctrl+Shift+S (strikethrough)", () => {
    expect(isSaveShortcut(ev({ ctrlKey: true, shiftKey: true, code: "KeyS", key: "S" }))).toBe(false);
  });

  it("excludes plain s without a modifier", () => {
    expect(isSaveShortcut(ev({ code: "KeyS", key: "s" }))).toBe(false);
  });

  it("excludes Ctrl with a different physical key", () => {
    expect(isSaveShortcut(ev({ ctrlKey: true, code: "KeyA", key: "a" }))).toBe(false);
  });
});
