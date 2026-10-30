import { describe, expect, it } from "vitest";
import { renderWorldSurface, type ScreenSurface } from "./world-surface";

function makeScreen(cells: ScreenSurface["cells"]): ScreenSurface {
  return { kind: "screen", version: 1, width: cells.length, height: 1, cells };
}

describe("world screen presenter", () => {
  it("uses text nodes and fixed palette styles, not guest HTML", () => {
    const root = document.createElement("div");
    const markup = "<";
    expect(renderWorldSurface(root, makeScreen([{ symbol: markup, fg: 196, bg: 0, bold: true, columns: 1 }]))).toBe(
      true,
    );
    expect(root.querySelector("img")).toBeNull();
    expect(root.querySelector("span")?.textContent).toBe(markup);
    expect(root.querySelector("span")?.style.color).toBe("#ff0000");
    expect(root.querySelector("span")?.style.fontWeight).toBe("bold");
  });

  it("maps wide glyph continuations to a two-column span", () => {
    const root = document.createElement("div");
    const screen: ScreenSurface = {
      kind: "screen",
      version: 1,
      width: 3,
      height: 1,
      cells: [
        { symbol: "界", fg: 0, bg: 0, bold: false, columns: 2 },
        { symbol: "", fg: 0, bg: 0, bold: false, columns: 0 },
        { symbol: "x", fg: 0, bg: 0, bold: false, columns: 1 },
      ],
    };
    expect(renderWorldSurface(root, screen)).toBe(true);
    expect(root.querySelectorAll("span")).toHaveLength(2);
    expect(root.querySelector("span")?.style.gridColumn).toBe("1 / span 2");
  });

  it("refuses bad bounds, controls, and mismatched wide cells without changing the view", () => {
    const root = document.createElement("div");
    root.textContent = "safe legacy view";
    expect(renderWorldSurface(root, makeScreen([]))).toBe(false);
    expect(renderWorldSurface(root, makeScreen([{ symbol: "\u001b[2J", fg: 0, bg: 0, bold: false, columns: 1 }]))).toBe(
      false,
    );
    expect(renderWorldSurface(root, makeScreen([{ symbol: "界", fg: 0, bg: 0, bold: false, columns: 2 }]))).toBe(false);
    expect(root.textContent).toBe("safe legacy view");
  });

  it("refuses grapheme count violations and mismatched symbol widths", () => {
    const root = document.createElement("div");
    // wide glyph in 1 column
    expect(renderWorldSurface(root, makeScreen([{ symbol: "界", fg: 0, bg: 0, bold: false, columns: 1 }]))).toBe(false);

    // narrow glyph in 2 columns
    const screenNarrowInWide: ScreenSurface = {
      kind: "screen",
      version: 1,
      width: 2,
      height: 1,
      cells: [
        { symbol: "x", fg: 0, bg: 0, bold: false, columns: 2 },
        { symbol: "", fg: 0, bg: 0, bold: false, columns: 0 },
      ],
    };
    expect(renderWorldSurface(root, screenNarrowInWide)).toBe(false);

    // narrow glyph in 2 columns (also with non-empty continuation)
    const screenNarrowNonEmptyContinuation: ScreenSurface = {
      kind: "screen",
      version: 1,
      width: 2,
      height: 1,
      cells: [
        { symbol: "x", fg: 0, bg: 0, bold: false, columns: 2 },
        { symbol: "Z", fg: 0, bg: 0, bold: false, columns: 0 },
      ],
    };
    expect(renderWorldSurface(root, screenNarrowNonEmptyContinuation)).toBe(false);

    // two graphemes in one cell
    expect(renderWorldSurface(root, makeScreen([{ symbol: "ab", fg: 0, bg: 0, bold: false, columns: 1 }]))).toBe(false);

    // zero graphemes in one cell
    expect(renderWorldSurface(root, makeScreen([{ symbol: "", fg: 0, bg: 0, bold: false, columns: 1 }]))).toBe(false);
  });

  it("measures emoji-modifier and tag sequences as their base, like Rust unicode-width (#1431)", () => {
    const root = document.createElement("div");
    // Each of these is exactly one grapheme whose Rust unicode-width 0.2 width is 2:
    // 👍🏽 U+1F44D U+1F3FD (emoji modifier), 🏴󐁧󐁢󐁥󐁮󐁧󐁿 England flag (U+1F3F4 + tag sequence).
    const sequenceGlyphs = ["\u{1F44D}\u{1F3FD}", "\u{1F3F4}\u{E0067}\u{E0062}\u{E0065}\u{E006E}\u{E0067}\u{E007F}"];
    for (const symbol of sequenceGlyphs) {
      const screen: ScreenSurface = {
        kind: "screen",
        version: 1,
        width: 2,
        height: 1,
        cells: [
          { symbol, fg: 0, bg: 0, bold: false, columns: 2 },
          { symbol: "", fg: 0, bg: 0, bold: false, columns: 0 },
        ],
      };
      expect(renderWorldSurface(root, screen)).toBe(true);
      // …and the same glyph at a mismatched columns value must be refused.
      expect(renderWorldSurface(root, makeScreen([{ symbol, fg: 0, bg: 0, bold: false, columns: 1 }]))).toBe(false);
    }
  });

  it("measures astral-plane, ZWJ and variation-selector glyphs as width 2, like Rust unicode-width (#761)", () => {
    const root = document.createElement("div");
    // Each of these is exactly one grapheme whose Rust unicode-width 0.2 width is 2:
    // 😀 U+1F600, 𠮷 U+20BB7 (CJK Ext. B), 👨‍👩‍👧 (ZWJ sequence), ⚠️ U+26A0 U+FE0F.
    const wideGlyphs = ["\u{1F600}", "\u{20BB7}", "\u{1F468}\u200D\u{1F469}\u200D\u{1F467}", "\u26A0\uFE0F"];
    for (const symbol of wideGlyphs) {
      const screen: ScreenSurface = {
        kind: "screen",
        version: 1,
        width: 2,
        height: 1,
        cells: [
          { symbol, fg: 0, bg: 0, bold: false, columns: 2 },
          { symbol: "", fg: 0, bg: 0, bold: false, columns: 0 },
        ],
      };
      expect(renderWorldSurface(root, screen)).toBe(true);
      // …and the same glyph squeezed into 1 column must be refused.
      expect(renderWorldSurface(root, makeScreen([{ symbol, fg: 0, bg: 0, bold: false, columns: 1 }]))).toBe(false);
    }
  });
});
