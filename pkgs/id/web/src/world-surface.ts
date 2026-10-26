/** Safe browser presenter for the world screen wire model. */

export type ScreenSurface = {
  kind: "screen";
  version: number;
  width: number;
  height: number;
  cells: ScreenCell[];
};

export type ScreenCell = {
  symbol: string;
  fg: number;
  bg: number;
  bold: boolean;
  columns: number;
};

const MAX_SCREEN_WIDTH = 120;
const MAX_SCREEN_HEIGHT = 50;
const MAX_SCREEN_CELLS = MAX_SCREEN_WIDTH * MAX_SCREEN_HEIGHT;

function xtermColor(index: number): string | null {
  if (!Number.isInteger(index) || index < 0 || index > 255 || index === 0) return null;
  const basic = [
    "#000000",
    "#800000",
    "#008000",
    "#808000",
    "#000080",
    "#800080",
    "#008080",
    "#c0c0c0",
    "#808080",
    "#ff0000",
    "#00ff00",
    "#ffff00",
    "#0000ff",
    "#ff00ff",
    "#00ffff",
    "#ffffff",
  ];
  if (index <= 15) return basic[index];
  if (index <= 231) {
    const value = index - 16;
    const level = (component: number) => (component === 0 ? 0 : 55 + component * 40);
    const r = level(Math.floor(value / 36));
    const g = level(Math.floor((value % 36) / 6));
    const b = level(value % 6);
    return `#${[r, g, b].map((part) => part.toString(16).padStart(2, "0")).join("")}`;
  }
  const gray = 8 + (index - 232) * 10;
  return `#${gray.toString(16).padStart(2, "0").repeat(3)}`;
}

const isEastAsianWide =
  /[\u1100-\u115F\u2329\u232A\u2E80-\u303E\u3040-\u33FF\u3400-\u4DBF\u4E00-\u9FFF\uA960-\uA97F\uAC00-\uD7A3\uF900-\uFAFF\uFE10-\uFE19\uFE30-\uFE6F\uFF01-\uFF60\uFFE0-\uFFE6]/;

const segmenter =
  typeof Intl !== "undefined" && Intl.Segmenter ? new Intl.Segmenter("en", { granularity: "grapheme" }) : null;

/**
 * Render a validated cell grid as text-only DOM nodes; return false on invalid wire data.
 * Width measurement attempts to match Rust's unicode-width semantics (ASCII = 1, CJK/wide = 2)
 * using a regex for wide characters and Intl.Segmenter for grapheme boundaries.
 */
export function renderWorldSurface(root: HTMLElement, surface: ScreenSurface): boolean {
  if (
    surface?.kind !== "screen" ||
    surface.version !== 1 ||
    !Number.isInteger(surface.width) ||
    !Number.isInteger(surface.height) ||
    surface.width < 1 ||
    surface.width > MAX_SCREEN_WIDTH ||
    surface.height < 1 ||
    surface.height > MAX_SCREEN_HEIGHT ||
    !Array.isArray(surface.cells) ||
    surface.cells.length !== surface.width * surface.height ||
    surface.cells.length > MAX_SCREEN_CELLS
  ) {
    return false;
  }

  const grid = document.createElement("div");
  grid.className = "id-world-screen";
  grid.setAttribute("role", "img");
  grid.setAttribute("aria-label", `World screen, ${surface.width} columns by ${surface.height} rows`);
  grid.style.display = "grid";
  grid.style.gridTemplateColumns = `repeat(${surface.width}, 1ch)`;
  grid.style.gridTemplateRows = `repeat(${surface.height}, 1em)`;
  grid.style.width = `${surface.width}ch`;
  grid.style.whiteSpace = "pre";

  for (let index = 0; index < surface.cells.length; index++) {
    const cell = surface.cells[index];
    if (
      !cell ||
      typeof cell.symbol !== "string" ||
      cell.symbol.length > 16 ||
      Array.from(cell.symbol).some((character) => {
        const codePoint = character.codePointAt(0) ?? 0;
        return codePoint <= 0x1f || (codePoint >= 0x7f && codePoint <= 0x9f);
      }) ||
      !Number.isInteger(cell.fg) ||
      !Number.isInteger(cell.bg) ||
      cell.fg < 0 ||
      cell.fg > 255 ||
      cell.bg < 0 ||
      cell.bg > 255 ||
      typeof cell.bold !== "boolean"
    ) {
      return false;
    }
    const x = index % surface.width;
    const y = Math.floor(index / surface.width);
    if (cell.columns === 0) {
      if (x === 0 || cell.symbol !== "") return false;
      const previous = surface.cells[index - 1];
      if (
        previous?.columns !== 2 ||
        previous.fg !== cell.fg ||
        previous.bg !== cell.bg ||
        previous.bold !== cell.bold
      ) {
        return false;
      }
      continue;
    }
    if (cell.columns !== 1 && cell.columns !== 2) return false;

    const graphemes = segmenter ? Array.from(segmenter.segment(cell.symbol)) : Array.from(cell.symbol);
    if (graphemes.length !== 1) return false;

    const terminalWidth = isEastAsianWide.test(cell.symbol) ? 2 : 1;
    if (terminalWidth !== cell.columns) return false;

    if (cell.columns === 2) {
      const continuation = surface.cells[index + 1];
      if (
        x + 1 >= surface.width ||
        !continuation ||
        continuation.columns !== 0 ||
        continuation.fg !== cell.fg ||
        continuation.bg !== cell.bg ||
        continuation.bold !== cell.bold ||
        continuation.symbol !== ""
      ) {
        return false;
      }
    }

    const span = document.createElement("span");
    span.textContent = cell.symbol;
    span.style.gridColumn = `${x + 1} / span ${cell.columns}`;
    span.style.gridRow = String(y + 1);
    const foreground = xtermColor(cell.fg);
    const background = xtermColor(cell.bg);
    if (foreground) span.style.color = foreground;
    if (background) span.style.backgroundColor = background;
    if (cell.bold) span.style.fontWeight = "bold";
    grid.append(span);
  }

  root.replaceChildren(grid);
  return true;
}
