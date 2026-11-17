/**
 * Web/terminal parity harness for #796 item 9-2 (issue #8340).
 *
 * The checked-in corpus at `pkgs/id/test-data/screen-parity-corpus.json`
 * is the oracle: its verdicts were generated from the real Rust
 * presenter (crates/id-core/tests/screen_parity.rs). This test renders
 * every corpus screen with the web renderer and asserts the DOM it
 * produces carries exactly the corpus verdicts — same text projection,
 * same presented cells with the same effective fg/bg/bold — so the
 * web renderer and the Rust presenter cannot drift apart silently.
 */
import { existsSync, readFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { renderWorldSurface, type ScreenSurface } from "./world-surface";

type CorpusCell = {
  symbol: string;
  fg: number;
  bg: number;
  bold: boolean;
  columns: number;
  row: number;
  col: number;
};

type CorpusCase = {
  name: string;
  width: number;
  height: number;
  cells: unknown[];
  verdict: { text: string; cells: CorpusCell[] };
};

// The corpus lives at pkgs/id/test-data/, one level above this
// package; vitest runs from the package root, and import.meta.url is
// not a file: URL under the vitest module runner, so resolve from
// the working directory with an explicit search list.
const corpusCandidates = [
  path.resolve(process.cwd(), "../test-data/screen-parity-corpus.json"),
  path.resolve(process.cwd(), "test-data/screen-parity-corpus.json"),
];
const corpusPath = corpusCandidates.find((candidate) => existsSync(candidate));
if (!corpusPath) {
  throw new Error(`screen-parity corpus not found; tried ${corpusCandidates.join(", ")}`);
}
const corpus = JSON.parse(readFileSync(corpusPath, "utf8")) as { cases: CorpusCase[] };

/** The standard xterm-256 palette as hex strings (index 0 = default). */
function xtermHex(index: number): string | null {
  if (index === 0) return null;
  const basic = [
    "#000000", "#800000", "#008000", "#808000", "#000080", "#800080", "#008080", "#c0c0c0",
    "#808080", "#ff0000", "#00ff00", "#ffff00", "#0000ff", "#ff00ff", "#00ffff", "#ffffff",
  ];
  if (index < 16) return basic[index] ?? null;
  if (index <= 231) {
    const value = index - 16;
    const level = (component: number) => (component === 0 ? 0 : 55 + component * 40);
    const parts = [level(Math.floor(value / 36)), level(Math.floor((value % 36) / 6)), level(value % 6)];
    return `#${parts.map((part) => part.toString(16).padStart(2, "0")).join("")}`;
  }
  const gray = 8 + (index - 232) * 10;
  return `#${gray.toString(16).padStart(2, "0").repeat(3)}`;
}

// Parity is judged at the presented-colour level: the xterm palette
// maps several indices to the same hex (bright basic colours equal
// the cube corners, e.g. 11 and 226 are both #ffff00), so a hex can
// never be reverse-mapped to one index. The web side therefore
// compares the span's presented hex against the corpus index's hex.
function presentedHex(cssColor: string): string | null {
  return cssColor ? cssColor.toLowerCase() : null;
}

describe("screen parity corpus (Rust presenter oracle)", () => {
  it("carries the six recorded cases", () => {
    expect(corpus.cases.map((entry) => entry.name)).toEqual([
      "plain_text",
      "styled_row",
      "wide_glyph",
      "emoji_sequences",
      "blank_screen",
      "mixed_rows",
    ]);
  });

  for (const entry of corpus.cases) {
    it(`web renderer matches the Rust verdict: ${entry.name}`, () => {
      const surface = {
        kind: "screen",
        version: 1,
        width: entry.width,
        height: entry.height,
        cells: entry.cells,
      } as unknown as ScreenSurface;
      const root = document.createElement("div");
      expect(renderWorldSurface(root, surface)).toBe(true);

      const spans = Array.from(root.querySelectorAll("span"));
      const extracted = spans.map((span) => {
        const gridColumn = span.style.gridColumn;
        const match = /^(\d+) \/ span (\d+)$/.exec(gridColumn);
        if (!match) throw new Error(`unparsable gridColumn ${gridColumn}`);
        return {
          symbol: span.textContent ?? "",
          fgHex: presentedHex(span.style.color),
          bgHex: presentedHex(span.style.backgroundColor),
          bold: span.style.fontWeight === "bold",
          columns: Number(match[2]),
          row: Number(span.style.gridRow) - 1,
          col: Number(match[1]) - 1,
        };
      });
      extracted.sort((a, b) => a.row - b.row || a.col - b.col);
      const expected = entry.verdict.cells.map((cell) => ({
        symbol: cell.symbol,
        fgHex: xtermHex(cell.fg),
        bgHex: xtermHex(cell.bg),
        bold: cell.bold,
        columns: cell.columns,
        row: cell.row,
        col: cell.col,
      }));
      expect(extracted).toEqual(expected);

      const rows: string[] = [];
      for (let row = 0; row < entry.height; row++) {
        const line = extracted
          .filter((cell) => cell.row === row)
          .sort((a, b) => a.col - b.col)
          .map((cell) => cell.symbol)
          .join("");
        rows.push(line.trimEnd());
      }
      expect(rows.join("\n")).toBe(entry.verdict.text);
    });
  }
});
