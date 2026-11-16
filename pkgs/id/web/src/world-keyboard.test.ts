import { describe, expect, it } from "vitest";
import { richTerminalBytesForKey, terminalBytesForKey } from "./world-keyboard";

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

describe("kitty-disambiguate-v1 rich world key input (Slice C)", () => {
  // Expectations mirror doc/kitty-disambiguate-v1-fixtures.json, the
  // language-neutral mirror of the Slice A Roc `Key.parse_events`
  // corpus. Parity is decoded-event equality (design §4.4), so where
  // the profile accepts two byte forms for one event (F1: letter `P`
  // or tilde `11~`; Shift+Tab: canonical CSI-u or btab `CSI Z`), these
  // tests pin the form this encoder emits. Slice D's oracle decodes
  // this encoder's output with the actual Roc decoder.
  function rich(
    value: string,
    modifiers: Partial<Pick<KeyboardEvent, "ctrlKey" | "altKey" | "metaKey" | "shiftKey">> = {},
  ): string | null {
    const bytes = richTerminalBytesForKey({
      key: value,
      ctrlKey: false,
      altKey: false,
      metaKey: false,
      shiftKey: false,
      ...modifiers,
    });
    return bytes ? Array.from(bytes, (byte) => String.fromCharCode(byte)).join("") : null;
  }

  it("disambiguates Escape from Alt (fixture: escape, alt-escape, alt-x)", () => {
    expect(rich("Escape")).toBe("\x1b[27u");
    expect(rich("Escape", { altKey: true })).toBe("\x1b[27;3u");
    expect(rich("x", { altKey: true })).toBe("\x1b[120;3u");
    // Escape followed by x is a different byte stream from Alt+x.
    expect(rich("Escape") + "x").not.toBe(rich("x", { altKey: true }));
  });

  it("keeps legacy bytes unmodified and disambiguates Ctrl pairs (fixtures: tab-byte, ctrl-i, enter-byte, ctrl-m)", () => {
    expect(rich("Tab")).toBe("\t");
    expect(rich("i", { ctrlKey: true })).toBe("\x1b[105;5u");
    expect(rich("Tab")).not.toBe(rich("i", { ctrlKey: true }));
    expect(rich("Enter")).toBe("\r");
    expect(rich("m", { ctrlKey: true })).toBe("\x1b[109;5u");
    expect(rich("Enter")).not.toBe(rich("m", { ctrlKey: true }));
    expect(rich("Backspace")).toBe("\x7f");
  });

  it("uses CSI-u functional codes for modified Enter, Tab, Backspace and Escape", () => {
    expect(rich("Enter", { ctrlKey: true })).toBe("\x1b[13;5u");
    expect(rich("Tab", { ctrlKey: true })).toBe("\x1b[9;5u");
    expect(rich("Backspace", { altKey: true })).toBe("\x1b[127;3u");
    expect(rich("Escape", { ctrlKey: true })).toBe("\x1b[27;5u");
  });

  it("encodes Shift+Tab in the canonical CSI-u form (fixture: shift-tab-canonical)", () => {
    expect(rich("Tab", { shiftKey: true })).toBe("\x1b[9;2u");
    expect(rich("Tab", { shiftKey: true, ctrlKey: true })).toBe("\x1b[9;6u");
  });

  it("encodes named keys in letter and tilde forms (fixtures: ctrl-right, up-letter, ctrl-alt-delete, insert-tilde)", () => {
    expect(rich("ArrowUp")).toBe("\x1b[A");
    expect(rich("ArrowRight", { ctrlKey: true })).toBe("\x1b[1;5C");
    expect(rich("Home", { shiftKey: true })).toBe("\x1b[1;2H");
    expect(rich("End", { ctrlKey: true, altKey: true, shiftKey: true })).toBe("\x1b[1;8F");
    expect(rich("Delete")).toBe("\x1b[3~");
    expect(rich("Delete", { ctrlKey: true, altKey: true })).toBe("\x1b[3;7~");
    expect(rich("Insert")).toBe("\x1b[2~");
    expect(rich("PageDown", { ctrlKey: true })).toBe("\x1b[6;5~");
  });

  it("encodes F1–F12 in tilde forms the legacy encoder does not have (fixtures: f5, shift-f5)", () => {
    expect(rich("F1")).toBe("\x1b[11~");
    expect(rich("F5")).toBe("\x1b[15~");
    expect(rich("F5", { shiftKey: true })).toBe("\x1b[15;2~");
    expect(rich("F12")).toBe("\x1b[24~");
    expect(rich("F13")).toBeNull();
  });

  it("keeps plain and shifted text as UTF-8 (fixtures: plain-ascii, latin-e-acute, emoji, shifted-text-plain)", () => {
    expect(rich("h")).toBe("h");
    expect(rich("A", { shiftKey: true })).toBe("A");
    expect(rich("!", { shiftKey: true })).toBe("!");
    expect(
      richTerminalBytesForKey({ key: "é", ctrlKey: false, altKey: false, metaKey: false, shiftKey: false }),
    ).toEqual(new TextEncoder().encode("é"));
    expect(
      richTerminalBytesForKey({ key: "😀", ctrlKey: false, altKey: false, metaKey: false, shiftKey: false }),
    ).toEqual(new TextEncoder().encode("😀"));
  });

  it("sends Ctrl/Alt scalars as CSI-u with the unshifted codepoint (fixtures: ctrl-shift-a, shifted-ascii-punctuation)", () => {
    expect(rich("A", { ctrlKey: true, shiftKey: true })).toBe("\x1b[97;6u");
    expect(rich("!", { ctrlKey: true, shiftKey: true })).toBe("\x1b[49;6u");
    expect(rich("X", { altKey: true, shiftKey: true })).toBe("\x1b[120;4u");
    expect(rich("c", { ctrlKey: true })).toBe("\x1b[99;5u");
    // Non-ASCII scalars have no unshifted form: their own codepoint is
    // used (fixture: shifted-non-ascii-retains-shift is decoder-side).
    expect(rich("ą", { altKey: true })).toBe("\x1b[261;3u");
  });

  it("does not unshift a scalar Shift did not produce (non-US layout base keys)", () => {
    // AZERTY: "&" is an unshifted base key, so Ctrl+"&" keeps the
    // resulting scalar's codepoint instead of the US-table "7".
    expect(rich("&", { ctrlKey: true })).toBe("\x1b[38;5u");
    expect(rich("&", { ctrlKey: true, shiftKey: false })).toBe("\x1b[38;5u");
    // With Shift held, "&" on a US layout is Shift+7: the protocol
    // Shift bit is set and the decoder's normalize_shift restores "&".
    expect(rich("&", { ctrlKey: true, shiftKey: true })).toBe("\x1b[55;6u");
  });

  it("keeps multi-scalar input on the text path under Ctrl/Alt (design §4.4)", () => {
    const combining = "e\u0301";
    expect(
      richTerminalBytesForKey({ key: combining, ctrlKey: false, altKey: true, metaKey: false, shiftKey: false }),
    ).toEqual(new TextEncoder().encode(combining));
    expect(
      richTerminalBytesForKey({ key: combining, ctrlKey: true, altKey: false, metaKey: false, shiftKey: false }),
    ).toEqual(new TextEncoder().encode(combining));
  });

  it("preserves AltGr printable input as text (fixture: altgr-a-ogonek)", () => {
    expect(richTerminalBytesForKey({ key: "ą", ctrlKey: true, altKey: true, metaKey: false, shiftKey: false })).toEqual(
      new TextEncoder().encode("ą"),
    );
    expect(richTerminalBytesForKey({ key: "€", ctrlKey: true, altKey: true, metaKey: false, shiftKey: false })).toEqual(
      new TextEncoder().encode("€"),
    );
  });

  it("leaves excluded and unsupported keys alone", () => {
    expect(rich("s", { metaKey: true })).toBeNull();
    expect(rich("Dead")).toBeNull();
    expect(rich("Process")).toBeNull();
    expect(rich("CapsLock")).toBeNull();
    expect(rich("Shift", { shiftKey: true })).toBeNull();
    expect(rich("Control", { ctrlKey: true })).toBeNull();
    // Multi-scalar committed text is not a physical key.
    expect(rich("ab")).toBeNull();
    for (const name of Object.getOwnPropertyNames(Object.prototype)) {
      expect(rich(name), name).toBeNull();
    }
  });
});
