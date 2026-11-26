import { describe, expect, it } from "vitest";
import { viewportCellsForPane, type PaneMetrics } from "./world-viewport";

// Oracle: the cell-grid geometry itself — the reported viewport is the
// pane's content box divided by the cell size (0.6em by 1em), floored and
// clamped to the host's resize limits. The pixel inputs below are the
// values measured in a real Chromium against the served /world page
// (fleet-command run 45689663668, #15170).
const pane = (overrides: Partial<PaneMetrics>): PaneMetrics => ({
  clientWidth: 317,
  clientHeight: 104,
  fontSize: 14.625,
  paddingLeft: 16,
  paddingRight: 16,
  paddingTop: 16,
  paddingBottom: 16,
  ...overrides,
});

describe("world pane viewport measurement", () => {
  it("excludes the pane padding: the 320px-window pane reports 29 columns, not 32", () => {
    // At a 320px window the pane measures 289px including p-4 padding.
    // Counting the padding reported 32 columns, so the host kept the
    // program at its 30-column desktop layout and it never reflowed.
    const cells = viewportCellsForPane(pane({ clientWidth: 289 }));
    expect(cells.width).toBe(29);
  });

  it("reports the desktop pane at 32 columns and 4 rows", () => {
    const cells = viewportCellsForPane(pane({}));
    expect(cells).toEqual({ width: 32, height: 4 });
  });

  it("matches the unpadded measurement when the pane has no padding", () => {
    const cells = viewportCellsForPane(pane({ paddingLeft: 0, paddingRight: 0, paddingTop: 0, paddingBottom: 0 }));
    expect(cells).toEqual({ width: 36, height: 7 });
  });

  it("clamps to the host's resize limits and never reports zero", () => {
    expect(viewportCellsForPane(pane({ clientWidth: 100000, clientHeight: 100000 }))).toEqual({
      width: 500,
      height: 200,
    });
    expect(viewportCellsForPane(pane({ clientWidth: 20, clientHeight: 20 }))).toEqual({
      width: 1,
      height: 1,
    });
  });

  it("falls back to a 16px font size when the computed size is unusable", () => {
    const cells = viewportCellsForPane(pane({ fontSize: 0 }));
    expect(cells).toEqual({ width: 29, height: 4 });
  });
});
