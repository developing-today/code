import { describe, expect, it } from "vitest";
import { kittyBytesForKey, terminalBytesForKey } from "./world-keyboard";

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

  it("rejects every Object.prototype member name", () => {
    for (const name of Object.getOwnPropertyNames(Object.prototype)) {
      expect(key(name), name).toBeNull();
    }
  });

  it("preserves all unmodified special-key encodings", () => {
    const encodings = {
      Enter: "\r",
      Tab: "\t",
      Backspace: "\x7f",
      Escape: "\x1b",
      Delete: "\x1b[3~",
      Home: "\x1b[H",
      End: "\x1b[F",
      PageUp: "\x1b[5~",
      PageDown: "\x1b[6~",
      ArrowUp: "\x1b[A",
      ArrowDown: "\x1b[B",
      ArrowRight: "\x1b[C",
      ArrowLeft: "\x1b[D",
    };

    for (const [name, expected] of Object.entries(encodings)) {
      expect(key(name), name).toBe(expected);
    }
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

describe("kitty-disambiguate-v1 world key input (Slice C, unused by the UI)", () => {
  // The expected byte strings below are the shared rich fixtures: each one
  // is an `expect` line for the Roc decoder in
  // pkgs/id/examples/roc-world/Key.roc (`Key.parse_events`), so this encoder
  // is tested against the Slice A decoder's expectations, not against
  // itself. Slice D replaces this mirroring with the compiled parity oracle.
  function rich(
    value: string,
    modifiers: Partial<Pick<KeyboardEvent, "ctrlKey" | "altKey" | "metaKey" | "shiftKey">> = {},
  ): string | null {
    const bytes = kittyBytesForKey({
      key: value,
      ctrlKey: false,
      altKey: false,
      metaKey: false,
      shiftKey: false,
      ...modifiers,
    });
    return bytes ? Array.from(bytes, (byte) => String.fromCharCode(byte)).join("") : null;
  }

  it("keeps unmodified keys on their legacy bytes, except Escape", () => {
    expect(rich("a")).toBe("a");
    expect(rich("A", { shiftKey: true })).toBe("A");
    expect(rich("Enter")).toBe("\r");
    expect(rich("Tab")).toBe("\t");
    expect(rich("Backspace")).toBe("\x7f");
    expect(rich("ArrowUp")).toBe("\x1b[A");
    expect(rich("Delete")).toBe("\x1b[3~");
    // A bare ESC byte decodes as EventUnknown in active mode; CSI-u 27
    // decodes to NamedEscape with no modifiers.
    expect(rich("Escape")).toBe("\x1b[27u");
  });

  it("leaves browser shortcuts and unsupported keys alone", () => {
    expect(rich("s", { metaKey: true })).toBeNull();
    expect(rich("Dead")).toBeNull();
    expect(rich("F5")).toBeNull();
    for (const name of Object.getOwnPropertyNames(Object.prototype)) {
      expect(rich(name), name).toBeNull();
    }
  });

  it("makes Escape vs Alt unambiguous", () => {
    expect(rich("Escape")).toBe("\x1b[27u");
    expect(rich("Escape", { altKey: true })).toBe("\x1b[27;3u");
    expect(rich("x", { altKey: true })).toBe("\x1b[120;3u");
    expect(rich("Escape", { ctrlKey: true })).toBe("\x1b[27;5u");
  });

  it("distinguishes Tab from Ctrl+i and Enter from Ctrl+m", () => {
    expect(rich("i", { ctrlKey: true })).toBe("\x1b[105;5u");
    expect(rich("Tab", { ctrlKey: true })).toBe("\x1b[9;5u");
    expect(rich("m", { ctrlKey: true })).toBe("\x1b[109;5u");
    expect(rich("Enter", { ctrlKey: true })).toBe("\x1b[13;5u");
    expect(rich("Tab", { shiftKey: true })).toBe("\x1b[9;2u");
    expect(rich("Tab", { shiftKey: true, ctrlKey: true })).toBe("\x1b[9;6u");
  });

  it("encodes modified named and CSI keys with explicit modifiers", () => {
    expect(rich("c", { ctrlKey: true })).toBe("\x1b[99;5u");
    expect(rich("Enter", { shiftKey: true })).toBe("\x1b[13;2u");
    expect(rich("Backspace", { altKey: true })).toBe("\x1b[127;3u");
    expect(rich("Delete", { ctrlKey: true, altKey: true })).toBe("\x1b[3;7~");
    expect(rich("PageDown", { ctrlKey: true, shiftKey: true })).toBe("\x1b[6;6~");
    expect(rich("ArrowUp", { altKey: true })).toBe("\x1b[1;3A");
    expect(rich("Home", { ctrlKey: true, altKey: true, shiftKey: true })).toBe("\x1b[1;8H");
  });

  it("sends the unshifted base scalar for modified printable keys", () => {
    // Ctrl+Shift+i: the browser key is `I`; a kitty terminal sends the
    // base code 105 with modifier 6, and the decoder normalizes it back
    // to scalar `I` with [Control].
    expect(rich("I", { ctrlKey: true, shiftKey: true })).toBe("\x1b[105;6u");
    expect(rich("X", { altKey: true, shiftKey: true })).toBe("\x1b[120;4u");
    expect(rich("!", { ctrlKey: true, shiftKey: true })).toBe("\x1b[49;6u");
    expect(rich("1", { ctrlKey: true })).toBe("\x1b[49;5u");
  });

  it("preserves AltGr printable text and plain multi-scalar text", () => {
    expect(
      kittyBytesForKey({ key: "€", ctrlKey: true, altKey: true, metaKey: false, shiftKey: false }),
    ).toEqual(new TextEncoder().encode("€"));
    expect(
      kittyBytesForKey({ key: "😀", ctrlKey: false, altKey: false, metaKey: false, shiftKey: false }),
    ).toEqual(new TextEncoder().encode("😀"));
    expect(rich("😀", { altKey: true })).toBe("\x1b[128512;3u");
  });
});
