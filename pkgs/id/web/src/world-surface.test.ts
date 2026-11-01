import { describe, expect, it } from "vitest";
import { applyScreenPatch, renderWorldSurface, type ScreenSurface } from "./world-surface";

function makeScreen(cells: ScreenSurface["cells"]): ScreenSurface {
  return { kind: "screen", version: 1, width: cells.length, height: 1, cells };
}

describe("world screen presenter", () => {
  it("applies revision-independent cell runs to the same validated surface as a full frame", () => {
    const previous: ScreenSurface = {
      kind: "screen",
      version: 1,
      width: 4,
      height: 1,
      cells: [
        { symbol: "a", fg: 0, bg: 0, bold: false, columns: 1 },
        { symbol: "b", fg: 0, bg: 0, bold: false, columns: 1 },
        { symbol: "c", fg: 0, bg: 0, bold: false, columns: 1 },
        { symbol: "d", fg: 0, bg: 0, bold: false, columns: 1 },
      ],
    };
    const patch = {
      version: 1,
      width: 4,
      height: 1,
      runs: [
        { start: 0, cells: [{ symbol: "界", fg: 0, bg: 0, bold: false, columns: 2 }, { symbol: "", fg: 0, bg: 0, bold: false, columns: 0 }] },
        { start: 3, cells: [{ symbol: "x", fg: 4, bg: 0, bold: true, columns: 1 }] },
      ],
    };
    const full: ScreenSurface = { ...previous, cells: [...patch.runs[0].cells, previous.cells[2], ...patch.runs[1].cells] };
    const applied = applyScreenPatch(previous, patch);
    expect(applied).toEqual(full);
    const root = document.createElement("div");
    expect(applied && renderWorldSurface(root, applied)).toBe(true);
    expect(renderWorldSurface(document.createElement("div"), full)).toBe(true);
  });

  it("rejects malformed, overlapping, out-of-range, and invalid-cell patches", () => {
    const previous = makeScreen([
      { symbol: "a", fg: 0, bg: 0, bold: false, columns: 1 },
      { symbol: "b", fg: 0, bg: 0, bold: false, columns: 1 },
    ]);
    const patch = (runs: Array<{ start: number; cells: ScreenSurface["cells"] }>) => ({
      version: 1,
      width: 2,
      height: 1,
      runs,
    });
    expect(applyScreenPatch(previous, patch([{ start: 1, cells: [previous.cells[1]] }, { start: 1, cells: [previous.cells[0]] }]))).toBeNull();
    expect(applyScreenPatch(previous, patch([{ start: 2, cells: [previous.cells[0]] }]))).toBeNull();
    expect(
      applyScreenPatch(
        previous,
        patch([
          {
            start: 0,
            cells: [
              { symbol: "界", fg: 0, bg: 0, bold: false, columns: 2 },
              { symbol: "x", fg: 0, bg: 0, bold: false, columns: 1 },
            ],
          },
        ]),
      ),
    ).not.toBeNull();
    const invalid = applyScreenPatch(
      previous,
      patch([
        {
          start: 0,
          cells: [
            { symbol: "界", fg: 0, bg: 0, bold: false, columns: 2 },
            { symbol: "x", fg: 0, bg: 0, bold: false, columns: 1 },
          ],
        },
      ]),
    );
    expect(invalid && renderWorldSurface(document.createElement("div"), invalid)).toBe(false);
  });

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

  it("measures the #1581 remainder classes like Rust unicode-width 0.2.2", () => {
    const root = document.createElement("div");
    const expectWide = (symbol: string) => {
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
      expect(renderWorldSurface(root, makeScreen([{ symbol, fg: 0, bg: 0, bold: false, columns: 1 }]))).toBe(false);
    };
    // ⬛ U+2B1B, ⬜ U+2B1C, ⭕ U+2B55: wide in unicode-width, previously
    // missing from WIDE_RANGES and measured 1.
    for (const symbol of ["\u{2B1B}", "\u{2B1C}", "\u{2B55}"]) expectWide(symbol);
    // ☝🏻 U+261D U+1F3FB: a modifier sequence measures 2 even on a
    // narrow (width-1) base.
    expectWide("\u{261D}\u{1F3FB}");
    // Composed Hangul jamo: LV U+1100 U+1161 and LVT + U+11A8 measure
    // as the precomposed syllable — 2, not the per-component sum.
    expectWide("\u{1100}\u{1161}");
    expectWide("\u{1100}\u{1161}\u{11A8}");

    // A lone spacing mark (U+0903, category Mc) measures 1, not 0:
    // renders in 1 column and is refused in 2.
    expect(renderWorldSurface(root, makeScreen([{ symbol: "\u{0903}", fg: 0, bg: 0, bold: false, columns: 1 }]))).toBe(
      true,
    );
    const markInWide: ScreenSurface = {
      kind: "screen",
      version: 1,
      width: 2,
      height: 1,
      cells: [
        { symbol: "\u{0903}", fg: 0, bg: 0, bold: false, columns: 2 },
        { symbol: "", fg: 0, bg: 0, bold: false, columns: 0 },
      ],
    };
    expect(renderWorldSurface(root, markInWide)).toBe(false);

    // Spacing marks with Grapheme_Extend measure 0 in a cluster, like
    // unicode-width 0.2: Bengali কা (U+0995 U+09BE) and Oriya କା
    // (U+0B15 U+0B3E) are single graphemes measuring 1 column in total
    // — they render in 1 column and are refused in 2.
    for (const symbol of ["\u{0995}\u{09BE}", "\u{0B15}\u{0B3E}"]) {
      expect(renderWorldSurface(root, makeScreen([{ symbol, fg: 0, bg: 0, bold: false, columns: 1 }]))).toBe(true);
      const clusterInWide: ScreenSurface = {
        kind: "screen",
        version: 1,
        width: 2,
        height: 1,
        cells: [
          { symbol, fg: 0, bg: 0, bold: false, columns: 2 },
          { symbol: "", fg: 0, bg: 0, bold: false, columns: 0 },
        ],
      };
      expect(renderWorldSurface(root, clusterInWide)).toBe(false);
    }

    // One grapheme of 18 UTF-16 units (e + 17 combining acutes, width
    // 1) renders: there is no symbol-length cap, matching the Rust
    // validator.
    const longGrapheme = `e${"\u0301".repeat(17)}`;
    expect(longGrapheme.length).toBe(18);
    expect(renderWorldSurface(root, makeScreen([{ symbol: longGrapheme, fg: 0, bg: 0, bold: false, columns: 1 }]))).toBe(
      true,
    );
  });

  it("measures U+FE0E text-presentation sequences as width 1, like Rust unicode-width 0.2.2 (#1581)", () => {
    const root = document.createElement("div");
    const expectNarrow = (symbol: string) => {
      expect(renderWorldSurface(root, makeScreen([{ symbol, fg: 0, bg: 0, bold: false, columns: 1 }]))).toBe(true);
      const wideScreen: ScreenSurface = {
        kind: "screen",
        version: 1,
        width: 2,
        height: 1,
        cells: [
          { symbol, fg: 0, bg: 0, bold: false, columns: 2 },
          { symbol: "", fg: 0, bg: 0, bold: false, columns: 0 },
        ],
      };
      expect(renderWorldSurface(root, wideScreen)).toBe(false);
    };
    // Default-emoji-presentation bases narrow to 1 with U+FE0E (VS15):
    // ⬛︎ U+2B1B, ⬜︎ U+2B1C, ⭕︎ U+2B55 (the classes the wide fix added),
    // ⌚︎ U+231A, ⭐︎ U+2B50, and astral 🕐︎ U+1F550.
    for (const symbol of [
      "\u{2B1B}\u{FE0E}",
      "\u{2B1C}\u{FE0E}",
      "\u{2B55}\u{FE0E}",
      "\u{231A}\u{FE0E}",
      "\u{2B50}\u{FE0E}",
      "\u{1F550}\u{FE0E}",
      // A trailing ZWJ does not undo the narrowing: the ZWJ shortcut
      // must apply text-presentation narrowing first, so
      // U+2B1B U+FE0E U+200D still measures 1 (Codex review on #2945).
      "\u{2B1B}\u{FE0E}\u{200D}",
      "\u{1F550}\u{FE0E}\u{200D}",
    ]) {
      expectNarrow(symbol);
    }
    // The same bases alone (no U+FE0E) are wide: ⌚ U+231A and
    // ⭐ U+2B50 render in 2 columns and are refused in 1.
    for (const symbol of ["\u{231A}", "\u{2B50}"]) {
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
      expect(renderWorldSurface(root, makeScreen([{ symbol, fg: 0, bg: 0, bold: false, columns: 1 }]))).toBe(false);
    }
    // A combining mark between the base and U+FE0E breaks the narrowing:
    // U+2B1B U+0301 U+FE0E still measures 2 under unicode-width 0.2.2.
    const interrupted = "\u{2B1B}\u{0301}\u{FE0E}";
    const interruptedScreen: ScreenSurface = {
      kind: "screen",
      version: 1,
      width: 2,
      height: 1,
      cells: [
        { symbol: interrupted, fg: 0, bg: 0, bold: false, columns: 2 },
        { symbol: "", fg: 0, bg: 0, bold: false, columns: 0 },
      ],
    };
    expect(renderWorldSurface(root, interruptedScreen)).toBe(true);
    expect(renderWorldSurface(root, makeScreen([{ symbol: interrupted, fg: 0, bg: 0, bold: false, columns: 1 }]))).toBe(
      false,
    );
    // Wide code points outside the text-presentation set keep width 2
    // with U+FE0E: 😀︎ U+1F600 U+FE0E and 漢︎ U+6F22 U+FE0E.
    for (const symbol of ["\u{1F600}\u{FE0E}", "\u{6F22}\u{FE0E}"]) {
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
