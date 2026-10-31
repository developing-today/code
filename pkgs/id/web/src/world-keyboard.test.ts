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
    expect(key("Dead")).toBeNull();
    expect(key("F5")).toBeNull();
  });

  it("encodes modified CSI tilde keys with the xterm modifier parameter", () => {
    expect(key("Delete", { ctrlKey: true })).toBe("\x1b[3;5~");
    expect(key("Delete", { shiftKey: true })).toBe("\x1b[3;2~");
    expect(key("PageUp", { altKey: true })).toBe("\x1b[5;3~");
    expect(key("PageDown", { ctrlKey: true, shiftKey: true })).toBe("\x1b[6;6~");
  });

  it("encodes modified CSI letter keys with the xterm modifier parameter", () => {
    expect(key("ArrowUp", { altKey: true })).toBe("\x1b[1;3A");
    expect(key("ArrowLeft", { ctrlKey: true })).toBe("\x1b[1;5D");
    expect(key("ArrowDown", { shiftKey: true })).toBe("\x1b[1;2B");
    expect(key("ArrowRight", { ctrlKey: true, altKey: true })).toBe("\x1b[1;7C");
    expect(key("Home", { shiftKey: true })).toBe("\x1b[1;2H");
    expect(key("End", { shiftKey: true })).toBe("\x1b[1;2F");
    expect(key("Home", { ctrlKey: true, altKey: true, shiftKey: true })).toBe("\x1b[1;8H");
  });

  it("encodes Shift+Tab as backtab and keeps other modifiers on it", () => {
    expect(key("Tab", { shiftKey: true })).toBe("\x1b[Z");
    expect(key("Tab", { shiftKey: true, ctrlKey: true })).toBe("\x1b[1;6Z");
  });

  it("ESC-prefixes modified special keys that have no CSI form", () => {
    expect(key("Backspace", { altKey: true })).toBe("\x1b\x7f");
    expect(key("Enter", { altKey: true })).toBe("\x1b\r");
    expect(key("Enter", { shiftKey: true })).toBe("\x1b\r");
    expect(key("Tab", { altKey: true })).toBe("\x1b\t");
    expect(key("Escape", { ctrlKey: true })).toBe("\x1b\x1b");
  });

  it("never returns the plain bytes for a modified special key", () => {
    const specials = [
      "Enter",
      "Tab",
      "Backspace",
      "Escape",
      "Delete",
      "Home",
      "End",
      "PageUp",
      "PageDown",
      "ArrowUp",
      "ArrowDown",
      "ArrowRight",
      "ArrowLeft",
    ];
    const modifierSets = [
      { shiftKey: true },
      { altKey: true },
      { ctrlKey: true },
      { ctrlKey: true, shiftKey: true },
      { altKey: true, shiftKey: true },
      { ctrlKey: true, altKey: true },
      { ctrlKey: true, altKey: true, shiftKey: true },
    ];
    for (const special of specials) {
      const plain = key(special);
      for (const modifiers of modifierSets) {
        const encoded = key(special, modifiers);
        expect(encoded, `${special} with ${JSON.stringify(modifiers)}`).not.toBeNull();
        expect(encoded, `${special} with ${JSON.stringify(modifiers)}`).not.toBe(plain);
      }
    }
  });
});
