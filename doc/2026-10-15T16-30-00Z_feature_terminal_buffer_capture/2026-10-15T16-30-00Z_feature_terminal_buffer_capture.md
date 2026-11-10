# Terminal buffer capture interface

## Intent

Let Rust terminal applications capture a terminal buffer into the same portable screen DTO used by the world SSH and browser presenters, without making Ratatui a runtime or mandatory dependency of the identity crate.

## Capture contract

`ScreenSurface::from_cells(width, height, cells)` accepts a row-major `Vec<ScreenCell>`, validates the existing 120×50 bounds, verifies the cell count and wide-glyph pairing, and returns the version-1 DTO. A terminal-library adapter maps its glyphs to `symbol`, maps supported foreground/background colors to xterm-256 indices, preserves bold, and emits an empty-symbol continuation cell after a two-column glyph. Unsupported modifiers should be documented as dropped; invalid or unrepresentable cells must fail capture rather than creating unsafe terminal output.

For a Ratatui buffer, the integration boundary is its `area` and row-major `content`: capture the area dimensions, map each cell and style into `ScreenCell`, then call `from_cells`. This keeps Ratatui optional to applications and the platform, and leaves the application responsible for its state/update loop and terminal event handling.

## Presenters

The existing `examples/roc-screen` demonstrates an app producing the same versioned screen wire model. The SSH presenter emits allowlisted ANSI from validated cells; the browser presenter creates text-only DOM cells with CSS styles. Neither executes guest-provided terminal controls, HTML, or CSS.

## Acceptance

- A Rust caller has a documented, validated buffer-to-screen capture boundary.
- Plain and styled cells, indexed colors, bold, wide glyph continuations, and rejected dimensions are covered by focused tests.
- Ratatui remains an optional application-side adapter, not a required identity runtime dependency.
- Applications still own their event loop; the DTO is only the portable presentation boundary.
