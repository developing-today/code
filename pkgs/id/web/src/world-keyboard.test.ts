import { describe, expect, it } from "vitest";
import { terminalBytesForKey } from "./world-keyboard";

function key(
  value: string,
  modifiers: Partial<Pick<KeyboardEvent, "ctrlKey" | "altKey" | "metaKey" | "shiftKey">> = {},
): string | null {
  const bytes = terminalBytesForKey({
    key: value,
    ctrlKey: false,
    altKey: false,
    metaKey: false,
    shiftKey: false,
    ...modifiers,
  });
  return bytes ? Array.from(bytes, (byte) => String.fromCharCode(byte)).join("") : null;
}

describe("terminal-compatible world key input", () => {
  it("encodes UTF-8 printable input and common terminal controls", () => {
    expect(
      terminalBytesForKey({
        key: "λ",
        ctrlKey: false,
        altKey: false,
        metaKey: false,
        shiftKey: false,
      }),
    ).toEqual(new TextEncoder().encode("λ"));
    expect(key("Enter")).toBe("\r");
    expect(key("Tab")).toBe("\t");
    expect(key("Backspace")).toBe("\x7f");
    expect(key("ArrowUp")).toBe("\x1b[A");
    expect(key("Home")).toBe("\x1b[H");
    expect(key("PageDown")).toBe("\x1b[6~");
  });

  it("encodes control letters and Alt-prefixed printable keys", () => {
    expect(key("c", { ctrlKey: true })).toBe("\x03");
    expect(key("x", { altKey: true })).toBe("\x1bx");
    expect(
      terminalBytesForKey({
        key: "😀",
        ctrlKey: false,
        altKey: false,
        metaKey: false,
        shiftKey: false,
      }),
    ).toEqual(new TextEncoder().encode("😀"));
    expect(
      terminalBytesForKey({
        key: "€",
        ctrlKey: true,
        altKey: true,
        metaKey: false,
        shiftKey: false,
      }),
    ).toEqual(new TextEncoder().encode("€"));
  });

  it("leaves browser shortcuts and unsupported keys alone", () => {
    expect(key("s", { metaKey: true })).toBeNull();
    expect(key("ArrowLeft", { ctrlKey: true })).toBeNull();
    expect(key("Dead")).toBeNull();
    expect(key("F5")).toBeNull();
  });
});
