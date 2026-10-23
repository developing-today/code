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
});
