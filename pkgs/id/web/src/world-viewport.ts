/** Viewport cell measurement for the world screen pane (#15170). */

export type PaneMetrics = {
  clientWidth: number;
  clientHeight: number;
  fontSize: number;
  paddingLeft: number;
  paddingRight: number;
  paddingTop: number;
  paddingBottom: number;
};

export type ViewportCells = { width: number; height: number };

/**
 * Convert the pane's pixel size to the cell viewport reported to the host.
 *
 * The surface renders inside the pane's content box, so the pane's padding
 * must come off the client size first: clientWidth/clientHeight include
 * padding, and measuring with it overstates the viewport by the padding on
 * each axis. The pane's p-4 overstated the width by ~3.6 cells — at a
 * 320px-wide window the client reported 32 columns for a content box that
 * fits 29, so the host never saw a sub-30 width and the program never
 * reflowed to its narrow layout (#15170).
 *
 * A cell is estimated at 0.6em wide (the surface grid lays cells out at
 * 1ch) and 1em tall (leading-none rows). The clamps mirror the host's
 * resize validation (1..=500 by 1..=200).
 */
export function viewportCellsForPane(metrics: PaneMetrics): ViewportCells {
  const fontSize = metrics.fontSize > 0 ? metrics.fontSize : 16;
  const contentWidth = metrics.clientWidth - metrics.paddingLeft - metrics.paddingRight;
  const contentHeight = metrics.clientHeight - metrics.paddingTop - metrics.paddingBottom;
  const width = Math.max(1, Math.min(500, Math.floor(contentWidth / (fontSize * 0.6))));
  const height = Math.max(1, Math.min(200, Math.floor(contentHeight / fontSize)));
  return { width, height };
}
