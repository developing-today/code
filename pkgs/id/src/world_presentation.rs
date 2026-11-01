//! Validated, transport-neutral presentations emitted by world programs.
//!
//! Legacy UTF-8 text remains valid. A program opts into a portable screen by
//! returning the versioned `kind: screen` JSON document parsed here. Guests
//! never supply terminal escapes, HTML, or CSS: adapters render this value
//! using their own trusted presentation code.

use std::fmt::Write as _;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Maximum dimensions accepted from one world screen.
pub const MAX_SCREEN_WIDTH: usize = 120;
/// Maximum height accepted from one world screen.
pub const MAX_SCREEN_HEIGHT: usize = 50;
/// Maximum row-major cells accepted from one world screen.
pub const MAX_SCREEN_CELLS: usize = MAX_SCREEN_WIDTH * MAX_SCREEN_HEIGHT;
/// Maximum coalesced style ranges accepted from one world screen.
pub const MAX_SCREEN_STYLE_SPANS: usize = 4096;

/// A validated view understood by more than one transport presenter.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorldSurface {
    /// A fixed-size terminal cell buffer, suitable for an SSH terminal or web DOM.
    Screen(ScreenSurface),
}

/// Normalized screen buffer. `cells` is row-major and has exactly width × height entries.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScreenSurface {
    /// Screen format version. P9.1 defines version 1.
    pub version: u8,
    /// Number of terminal columns.
    pub width: u16,
    /// Number of terminal rows.
    pub height: u16,
    /// Normalized row-major cells.
    pub cells: Vec<ScreenCell>,
}

impl ScreenSurface {
    /// Build a validated portable screen from a terminal buffer's row-major
    /// cells. Integrations should translate their native colors and modifiers
    /// to the DTO's xterm-256 indices and bold flag before calling this.
    ///
    /// # Errors
    ///
    /// Rejects empty or oversized dimensions, a mismatched cell count, unsafe
    /// glyphs, invalid wide-glyph continuations, and invalid style values.
    pub fn from_cells(width: usize, height: usize, cells: Vec<ScreenCell>) -> Result<Self> {
        ensure!(
            (1..=MAX_SCREEN_WIDTH).contains(&width),
            "screen width must be 1..={MAX_SCREEN_WIDTH}"
        );
        ensure!(
            (1..=MAX_SCREEN_HEIGHT).contains(&height),
            "screen height must be 1..={MAX_SCREEN_HEIGHT}"
        );
        ensure!(
            cells.len() == width * height,
            "screen cell count does not match its dimensions"
        );
        let screen = Self {
            version: 1,
            width: u16::try_from(width)?,
            height: u16::try_from(height)?,
            cells,
        };
        ensure!(
            screen.render_ansi().is_some(),
            "screen cells do not form a valid portable screen"
        );
        Ok(screen)
    }

    /// Crop a screen to a connection-local viewport without changing world state.
    /// Cropping through the second column of a wide glyph replaces that partial
    /// glyph with a blank cell so the returned grid remains valid.
    pub fn fit(&self, width: usize, height: usize) -> Self {
        let width = width.min(usize::from(self.width));
        let height = height.min(usize::from(self.height));
        let mut cells = Vec::with_capacity(width * height);
        for y in 0..height {
            let start = y * usize::from(self.width);
            let mut row = self.cells[start..start + width].to_vec();
            if row.last().is_some_and(|cell| cell.columns == 2) {
                if let Some(last) = row.last_mut() {
                    *last = ScreenCell {
                        symbol: " ".to_owned(),
                        fg: 0,
                        bg: 0,
                        bold: false,
                        columns: 1,
                    };
                }
            }
            cells.extend(row);
        }
        Self {
            version: self.version,
            width: u16::try_from(width).unwrap_or(self.width),
            height: u16::try_from(height).unwrap_or(self.height),
            cells,
        }
    }
}

/// A cell-run delta between two consecutive screens (F12.11, first slice).
///
/// A patch carries only the cells that changed, grouped into maximal runs of
/// consecutive row-major indices. It is transport-neutral: the same patch can
/// ride any shell transport, and a client that applies it to the previous
/// screen holds exactly the next full frame. Binary encoding, compression,
/// and resume tokens are later F12.11 slices; this type is the delta they
/// will encode.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScreenPatch {
    /// Screen format version; matches the screens it patches.
    pub version: u8,
    /// Number of terminal columns; matches the screens it patches.
    pub width: u16,
    /// Number of terminal rows; matches the screens it patches.
    pub height: u16,
    /// The changed cell runs, in row-major order, non-overlapping.
    pub runs: Vec<CellRun>,
}

/// One maximal run of consecutive row-major cells that changed.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CellRun {
    /// Row-major index of the run's first cell.
    pub start: u32,
    /// The replacement cells, in row-major order.
    pub cells: Vec<ScreenCell>,
}

impl ScreenPatch {
    /// The patch that turns `prev` into `next`: the maximal runs of cells
    /// that differ. Returns `None` when the screens do not share a version
    /// and dimensions — the caller sends a full frame instead.
    #[must_use]
    pub fn diff(prev: &ScreenSurface, next: &ScreenSurface) -> Option<Self> {
        if prev.version != next.version || prev.width != next.width || prev.height != next.height {
            return None;
        }
        let mut runs = Vec::new();
        let mut current: Option<(usize, Vec<ScreenCell>)> = None;
        for (index, (before, after)) in prev.cells.iter().zip(&next.cells).enumerate() {
            if before == after {
                if let Some((start, cells)) = current.take() {
                    runs.push(CellRun {
                        start: u32::try_from(start).unwrap_or(u32::MAX),
                        cells,
                    });
                }
                continue;
            }
            match &mut current {
                Some((_, cells)) => cells.push(after.clone()),
                None => current = Some((index, vec![after.clone()])),
            }
        }
        if let Some((start, cells)) = current.take() {
            runs.push(CellRun {
                start: u32::try_from(start).unwrap_or(u32::MAX),
                cells,
            });
        }
        Some(Self {
            version: next.version,
            width: next.width,
            height: next.height,
            runs,
        })
    }

    /// The screen this patch produces from `prev`. Returns `None` when the
    /// patch does not belong to `prev` (version or dimensions differ) or a
    /// run is out of order, overlaps an earlier run, or runs past the last
    /// cell — the caller falls back to a full frame.
    #[must_use]
    pub fn apply(&self, prev: &ScreenSurface) -> Option<ScreenSurface> {
        if self.version != prev.version || self.width != prev.width || self.height != prev.height {
            return None;
        }
        let mut cells = prev.cells.clone();
        let mut covered_until = 0usize;
        for run in &self.runs {
            let start = usize::try_from(run.start).ok()?;
            let end = start.checked_add(run.cells.len())?;
            if start < covered_until || end > cells.len() {
                return None;
            }
            cells[start..end].clone_from_slice(&run.cells);
            covered_until = end;
        }
        Some(ScreenSurface {
            version: prev.version,
            width: prev.width,
            height: prev.height,
            cells,
        })
    }

    /// Whether the patch changes nothing (the two screens are identical).
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }
}

/// One normalized cell. `columns == 0` marks the continuation of a two-column glyph.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScreenCell {
    /// One safe Unicode grapheme, or empty for a wide glyph's continuation cell.
    pub symbol: String,
    /// Xterm-256 foreground index; zero selects the presenter default.
    pub fg: u8,
    /// Xterm-256 background index; zero selects the presenter default.
    pub bg: u8,
    /// Whether the cell is bold.
    pub bold: bool,
    /// Terminal columns occupied by this grapheme (zero for continuation cells).
    pub columns: u8,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScreenPayload {
    kind: String,
    version: u8,
    width: usize,
    height: usize,
    rows: Vec<String>,
    #[serde(default)]
    styles: Vec<Vec<StyleSpan>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StyleSpan {
    x: usize,
    length: usize,
    fg: u8,
    bg: u8,
    bold: bool,
}

/// Parse the opt-in screen contract; untagged program views remain legacy text.
///
/// Malformed values that identify themselves as a screen are rejected. Other
/// JSON and ordinary strings remain plain text for compatibility.
pub fn parse_surface(data: &str) -> Result<Option<WorldSurface>> {
    let value: Value = match serde_json::from_str(data) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    if value.get("kind").and_then(Value::as_str) != Some("screen") {
        return Ok(None);
    }

    let payload: ScreenPayload =
        serde_json::from_value(value).context("decode world screen presentation")?;
    ensure!(payload.kind == "screen", "invalid world screen kind");
    ensure!(
        payload.version == 1,
        "unsupported world screen version {}",
        payload.version
    );
    ensure!(
        (1..=MAX_SCREEN_WIDTH).contains(&payload.width),
        "world screen width must be 1..={MAX_SCREEN_WIDTH}"
    );
    ensure!(
        (1..=MAX_SCREEN_HEIGHT).contains(&payload.height),
        "world screen height must be 1..={MAX_SCREEN_HEIGHT}"
    );
    let cell_count = payload
        .width
        .checked_mul(payload.height)
        .context("world screen dimensions overflow")?;
    ensure!(
        cell_count <= MAX_SCREEN_CELLS,
        "world screen has too many cells"
    );
    ensure!(
        payload.rows.len() == payload.height,
        "world screen row count does not match height"
    );
    ensure!(
        payload.styles.is_empty() || payload.styles.len() == payload.height,
        "world screen style rows do not match height"
    );
    ensure!(
        payload.styles.iter().map(Vec::len).sum::<usize>() <= MAX_SCREEN_STYLE_SPANS,
        "world screen has too many style spans"
    );

    let default_cell = ScreenCell {
        symbol: " ".to_owned(),
        fg: 0,
        bg: 0,
        bold: false,
        columns: 1,
    };
    let mut cells = vec![default_cell; cell_count];

    for (row_index, row) in payload.rows.iter().enumerate() {
        let row_start = row_index * payload.width;
        let mut column = 0_usize;
        let mut boundaries = vec![false; payload.width + 1];
        boundaries[0] = true;
        for grapheme in row.graphemes(true) {
            ensure!(
                !grapheme.chars().any(char::is_control),
                "world screen row {row_index} contains a control character"
            );
            let columns = UnicodeWidthStr::width(grapheme);
            ensure!(
                (1..=2).contains(&columns),
                "world screen glyph must occupy one or two columns"
            );
            let end = column
                .checked_add(columns)
                .context("world screen row width overflow")?;
            ensure!(
                end <= payload.width,
                "world screen row {row_index} exceeds its width"
            );
            boundaries[column] = true;
            boundaries[end] = true;
            cells[row_start + column] = ScreenCell {
                symbol: grapheme.to_owned(),
                fg: 0,
                bg: 0,
                bold: false,
                columns: u8::try_from(columns)
                    .context("screen glyph width is not representable")?,
            };
            if columns == 2 {
                cells[row_start + column + 1] = ScreenCell {
                    symbol: String::new(),
                    fg: 0,
                    bg: 0,
                    bold: false,
                    columns: 0,
                };
            }
            column = end;
        }
        ensure!(
            column == payload.width,
            "world screen row {row_index} does not fill its width"
        );

        if let Some(style_spans) = payload.styles.get(row_index) {
            let mut styled = vec![false; payload.width];
            let mut previous_end = 0;
            for span in style_spans {
                ensure!(span.length > 0, "world screen style span must not be empty");
                let end = span
                    .x
                    .checked_add(span.length)
                    .context("world screen style span overflows")?;
                ensure!(
                    end <= payload.width,
                    "world screen style span exceeds its row"
                );
                ensure!(
                    span.x >= previous_end,
                    "world screen style spans overlap or are unsorted"
                );
                ensure!(
                    boundaries[span.x] && boundaries[end],
                    "world screen style span splits a wide glyph"
                );
                for column in span.x..end {
                    ensure!(!styled[column], "world screen style spans overlap");
                    styled[column] = true;
                    let cell = &mut cells[row_start + column];
                    cell.fg = span.fg;
                    cell.bg = span.bg;
                    cell.bold = span.bold;
                }
                previous_end = end;
            }
        }
    }

    for row in 0..payload.height {
        let start = row * payload.width;
        for column in 0..payload.width {
            let cell = &cells[start + column];
            if cell.columns == 0 {
                ensure!(column > 0, "world screen continuation has no leading glyph");
                let leading = &cells[start + column - 1];
                ensure!(
                    leading.columns == 2
                        && leading.fg == cell.fg
                        && leading.bg == cell.bg
                        && leading.bold == cell.bold,
                    "world screen continuation does not match its wide glyph"
                );
            }
        }
    }

    Ok(Some(WorldSurface::Screen(ScreenSurface {
        version: 1,
        width: u16::try_from(payload.width).context("screen width is not representable")?,
        height: u16::try_from(payload.height).context("screen height is not representable")?,
        cells,
    })))
}

impl ScreenSurface {
    /// Render a safe terminal frame. Escape sequences are generated only from
    /// validated numeric style fields; guest-provided glyphs contain no controls.
    #[must_use]
    pub fn render_ansi(&self) -> Option<String> {
        if !self.is_valid() {
            return None;
        }
        let width = usize::from(self.width);
        let height = usize::from(self.height);
        let mut output = String::from("\x1b[H\x1b[2J");
        for row in 0..height {
            let mut previous_style: Option<(u8, u8, bool)> = None;
            for cell in &self.cells[row * width..(row + 1) * width] {
                if cell.columns == 0 {
                    continue;
                }
                let style = (cell.fg, cell.bg, cell.bold);
                if previous_style != Some(style) {
                    output.push_str(&style_sequence(style.0, style.1, style.2));
                    previous_style = Some(style);
                }
                output.push_str(&cell.symbol);
            }
            output.push_str("\x1b[0m");
            if row + 1 < height {
                output.push_str("\r\n");
            }
        }
        Some(output)
    }

    /// Render only rows that differ from the preceding screen. A changed row
    /// is rewritten from column one so wide-to-narrow glyph transitions cannot
    /// leave a stale continuation cell behind. `None` means the caller must
    /// use a full repaint (invalid screen or changed dimensions/version).
    #[must_use]
    pub(crate) fn render_ansi_diff(&self, previous: &Self) -> Option<String> {
        if !self.is_valid()
            || !previous.is_valid()
            || self.version != previous.version
            || self.width != previous.width
            || self.height != previous.height
        {
            return None;
        }

        let width = usize::from(self.width);
        let height = usize::from(self.height);
        let mut output = String::new();
        for row in 0..height {
            let start = row * width;
            let end = start + width;
            if self.cells[start..end] == previous.cells[start..end] {
                continue;
            }

            // Repaint a complete changed row. This also replaces both cells
            // when a glyph changes between narrow and wide forms.
            write!(&mut output, "\x1b[{};1H", row + 1).ok()?;
            let mut previous_style: Option<(u8, u8, bool)> = None;
            for cell in &self.cells[start..end] {
                if cell.columns == 0 {
                    continue;
                }
                let style = (cell.fg, cell.bg, cell.bold);
                if previous_style != Some(style) {
                    output.push_str(&style_sequence(style.0, style.1, style.2));
                    previous_style = Some(style);
                }
                output.push_str(&cell.symbol);
            }
            output.push_str("\x1b[0m");
        }
        if !output.is_empty() {
            // An absolute move clears the terminal's pending-wrap state after
            // writing a row through its last column, without changing cells.
            write!(&mut output, "\x1b[{};{}H", height, width).ok()?;
        }
        Some(output)
    }

    fn is_valid(&self) -> bool {
        let width = usize::from(self.width);
        let height = usize::from(self.height);
        if width == 0
            || width > MAX_SCREEN_WIDTH
            || height == 0
            || height > MAX_SCREEN_HEIGHT
            || width.checked_mul(height) != Some(self.cells.len())
        {
            return false;
        }
        for (index, cell) in self.cells.iter().enumerate() {
            let x = index % width;
            if cell.columns == 0 {
                if x == 0 {
                    return false;
                }
                let Some(previous) = index
                    .checked_sub(1)
                    .and_then(|previous| self.cells.get(previous))
                else {
                    return false;
                };
                if previous.columns != 2
                    || previous.fg != cell.fg
                    || previous.bg != cell.bg
                    || previous.bold != cell.bold
                    || !cell.symbol.is_empty()
                {
                    return false;
                }
            } else if cell.columns == 1 || cell.columns == 2 {
                if cell.symbol.graphemes(true).count() != 1
                    || cell.symbol.chars().any(char::is_control)
                    || UnicodeWidthStr::width(cell.symbol.as_str()) != usize::from(cell.columns)
                    || (cell.columns == 2 && x + 1 == width)
                {
                    return false;
                }
                if cell.columns == 2 {
                    let Some(continuation) = index
                        .checked_add(1)
                        .and_then(|continuation| self.cells.get(continuation))
                    else {
                        return false;
                    };
                    if continuation.columns != 0
                        || continuation.fg != cell.fg
                        || continuation.bg != cell.bg
                        || continuation.bold != cell.bold
                        || !continuation.symbol.is_empty()
                    {
                        return false;
                    }
                }
            } else {
                return false;
            }
        }
        true
    }
}

fn style_sequence(fg: u8, bg: u8, bold: bool) -> String {
    let mut codes = vec!["0".to_owned()];
    if bold {
        codes.push("1".to_owned());
    }
    if fg != 0 {
        codes.push(format!("38;5;{fg}"));
    }
    if bg != 0 {
        codes.push(format!("48;5;{bg}"));
    }
    format!("\x1b[{}m", codes.join(";"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use serde_json::json;

    fn screen(value: Value) -> Result<Option<WorldSurface>> {
        parse_surface(&value.to_string())
    }

    fn cell(symbol: &str) -> ScreenCell {
        ScreenCell {
            symbol: symbol.to_owned(),
            fg: 0,
            bg: 0,
            bold: false,
            columns: 1,
        }
    }

    fn grid(width: usize, height: usize, fill: &str) -> ScreenSurface {
        ScreenSurface::from_cells(width, height, vec![cell(fill); width * height]).unwrap()
    }

    #[test]
    fn patch_parity_patched_client_equals_full_frame_client() {
        let prev = grid(4, 2, " ");
        let mut cells = prev.cells.clone();
        cells[0] = cell("a");
        cells[3] = ScreenCell { fg: 9, ..cell(" ") };
        cells[5] = cell("x");
        cells[6] = cell("y");
        let next = ScreenSurface::from_cells(4, 2, cells).unwrap();

        let patch = ScreenPatch::diff(&prev, &next).unwrap();
        assert_eq!(patch.runs.len(), 3, "single cell, style-only, and a pair");
        assert_eq!(patch.apply(&prev), Some(next));

        let identical = ScreenPatch::diff(&prev, &prev).unwrap();
        assert!(identical.is_empty());
        assert_eq!(identical.apply(&prev), Some(prev));
    }

    #[test]
    fn patch_for_a_small_change_is_under_a_tenth_of_the_full_frame() {
        let prev = grid(80, 24, " ");
        let mut cells = prev.cells.clone();
        cells[1000] = cell("x");
        let next = ScreenSurface::from_cells(80, 24, cells).unwrap();

        let patch = ScreenPatch::diff(&prev, &next).unwrap();
        let patch_bytes = serde_json::to_string(&patch).unwrap().len();
        let full_bytes = serde_json::to_string(&next).unwrap().len();
        assert!(
            patch_bytes * 10 < full_bytes,
            "patch {patch_bytes} bytes vs full frame {full_bytes} bytes"
        );
        assert_eq!(patch.apply(&prev), Some(next));
    }

    #[test]
    fn patch_refuses_mismatched_screens_and_bad_runs() {
        let small = grid(4, 2, " ");
        let other = grid(2, 2, " ");
        assert_eq!(ScreenPatch::diff(&small, &other), None);

        let patch = ScreenPatch::diff(&small, &small).unwrap();
        assert_eq!(patch.apply(&other), None);

        let bad = ScreenPatch {
            version: 1,
            width: 4,
            height: 2,
            runs: vec![CellRun {
                start: 7,
                cells: vec![cell("x"), cell("y")],
            }],
        };
        assert_eq!(bad.apply(&small), None, "run past the last cell");
    }

    #[test]
    fn ansi_diff_repaints_changed_rows_with_terminal_parity_and_byte_savings() {
        let previous = grid(80, 24, " ");
        let mut cells = previous.cells.clone();
        cells[10 * 80 + 30] = cell("x");
        let next = ScreenSurface::from_cells(80, 24, cells).unwrap();

        let diff = next.render_ansi_diff(&previous).unwrap();
        let full = next.render_ansi().unwrap();
        assert!(diff.contains("\x1b[11;1H"));
        assert!(!diff.contains("\x1b[10;1H"));
        assert!(!diff.contains("\x1b[2J"));
        assert!(
            diff.len() * 10 < full.len(),
            "diff {diff:?} vs full repaint"
        );

        let mut patched = vt100::Parser::new(24, 80, 0);
        patched.process(previous.render_ansi().unwrap().as_bytes());
        patched.process(diff.as_bytes());
        let mut repainted = vt100::Parser::new(24, 80, 0);
        repainted.process(full.as_bytes());
        assert_eq!(patched.screen().contents(), repainted.screen().contents());
    }

    #[test]
    fn ansi_diff_handles_wide_glyph_transitions_and_style_changes() {
        let previous = screen(json!({
            "kind": "screen",
            "version": 1,
            "width": 6,
            "height": 1,
            "rows": ["界ab  "],
            "styles": [[{"x": 0, "length": 2, "fg": 1, "bg": 0, "bold": false}]]
        }))
        .unwrap()
        .unwrap();
        let WorldSurface::Screen(previous) = previous else {
            panic!("expected screen");
        };
        let next = screen(json!({
            "kind": "screen",
            "version": 1,
            "width": 6,
            "height": 1,
            "rows": ["x界b  "],
            "styles": [[{"x": 1, "length": 2, "fg": 2, "bg": 0, "bold": true}]]
        }))
        .unwrap()
        .unwrap();
        let WorldSurface::Screen(next) = next else {
            panic!("expected screen");
        };

        let diff = next.render_ansi_diff(&previous).unwrap();
        assert!(diff.contains("\x1b[0;1;38;5;2m"));
        let mut patched = vt100::Parser::new(1, 6, 0);
        patched.process(previous.render_ansi().unwrap().as_bytes());
        patched.process(diff.as_bytes());
        let mut repainted = vt100::Parser::new(1, 6, 0);
        repainted.process(next.render_ansi().unwrap().as_bytes());
        assert_eq!(patched.screen().contents(), repainted.screen().contents());
        assert_eq!(patched.screen().cell(0, 1), repainted.screen().cell(0, 1));
    }

    #[test]
    fn ansi_diff_is_empty_for_identical_screens_and_requests_full_fallback_on_resize() {
        let screen = grid(4, 2, " ");
        assert_eq!(screen.render_ansi_diff(&screen), Some(String::new()));
        assert_eq!(screen.render_ansi_diff(&grid(2, 2, " ")), None);
    }

    #[test]
    fn plain_text_stays_on_the_legacy_path() {
        assert_eq!(parse_surface("hello\nworld").unwrap(), None);
        assert_eq!(
            parse_surface(r#"{"kind":"other","version":1}"#).unwrap(),
            None
        );
    }

    #[test]
    fn viewport_crops_rows_and_does_not_split_wide_graphemes() {
        let parsed = parse_surface(
            r#"{"kind":"screen","version":1,"width":4,"height":2,"rows":["界ab","wxyz"]}"#,
        )
        .unwrap()
        .unwrap();
        let WorldSurface::Screen(screen) = parsed;

        let cropped = screen.fit(1, 1);
        assert_eq!(cropped.width, 1);
        assert_eq!(cropped.height, 1);
        assert_eq!(cropped.cells[0].symbol, " ");
        assert_eq!(cropped.cells[0].columns, 1);
        assert!(cropped.render_ansi().is_some());
    }

    #[test]
    fn buffer_capture_constructor_enforces_the_portable_cell_policy() {
        let cells = vec![
            ScreenCell {
                symbol: "界".to_owned(),
                fg: 196,
                bg: 0,
                bold: true,
                columns: 2,
            },
            ScreenCell {
                symbol: String::new(),
                fg: 196,
                bg: 0,
                bold: true,
                columns: 0,
            },
        ];
        let captured = ScreenSurface::from_cells(2, 1, cells).unwrap();
        assert_eq!(captured.width, 2);
        assert_eq!(captured.cells[0].fg, 196);
        assert!(captured.render_ansi().is_some());

        assert!(ScreenSurface::from_cells(2, 1, vec![]).is_err());
        assert!(
            ScreenSurface::from_cells(
                121,
                1,
                vec![
                    ScreenCell {
                        symbol: " ".to_owned(),
                        fg: 0,
                        bg: 0,
                        bold: false,
                        columns: 1,
                    };
                    121
                ]
            )
            .is_err()
        );
    }

    #[test]
    fn validates_and_normalizes_a_styled_unicode_grid() {
        let parsed = screen(json!({
            "kind": "screen",
            "version": 1,
            "width": 4,
            "height": 1,
            "rows": ["界ab"],
            "styles": [[{"x": 0, "length": 2, "fg": 196, "bg": 0, "bold": true}]],
        }))
        .unwrap()
        .unwrap();
        let WorldSurface::Screen(buffer) = parsed;
        assert_eq!(buffer.cells.len(), 4);
        assert_eq!(buffer.cells[0].symbol, "界");
        assert_eq!(buffer.cells[0].columns, 2);
        assert_eq!(buffer.cells[1].symbol, "");
        assert_eq!(buffer.cells[1].columns, 0);
        assert_eq!(buffer.cells[1].fg, 196);
        assert!(buffer.cells[0].bold);
        assert_eq!(buffer.cells[2].symbol, "a");
    }

    #[test]
    fn terminal_renderer_emits_only_presenter_owned_escape_codes() {
        let parsed = screen(json!({
            "kind": "screen",
            "version": 1,
            "width": 2,
            "height": 1,
            "rows": ["ok"],
            "styles": [[{"x": 0, "length": 2, "fg": 196, "bg": 0, "bold": true}]],
        }))
        .unwrap()
        .unwrap();
        let WorldSurface::Screen(buffer) = parsed;
        assert_eq!(
            buffer.render_ansi().unwrap(),
            "\x1b[H\x1b[2J\x1b[0;1;38;5;196mok\x1b[0m"
        );
    }

    #[test]
    fn terminal_renderer_refuses_unvalidated_escape_sequences() {
        let screen = ScreenSurface {
            version: 1,
            width: 1,
            height: 1,
            cells: vec![ScreenCell {
                symbol: "\u{001b}".to_owned(),
                fg: 0,
                bg: 0,
                bold: false,
                columns: 1,
            }],
        };
        assert_eq!(screen.render_ansi(), None);
    }

    #[test]
    fn rejects_guest_control_sequences_and_malformed_grid_shapes() {
        let mut control = json!({
            "kind": "screen", "version": 1, "width": 1, "height": 1,
            "rows": ["x"], "styles": [[]],
        });
        control["rows"][0] = json!("\u{001b}[2J");
        assert!(screen(control).is_err());

        let too_wide = json!({
            "kind": "screen", "version": 1, "width": 2, "height": 1,
            "rows": ["x"], "styles": [[]],
        });
        assert!(screen(too_wide).is_err());

        let split_wide_glyph = json!({
            "kind": "screen", "version": 1, "width": 3, "height": 1,
            "rows": ["界x"],
            "styles": [[{"x": 0, "length": 1, "fg": 2, "bg": 0, "bold": false}]],
        });
        assert!(screen(split_wide_glyph).is_err());
    }

    #[test]
    fn rejects_overlapping_unsorted_spans_and_unsupported_versions() {
        let overlap = json!({
            "kind": "screen", "version": 1, "width": 4, "height": 1,
            "rows": ["abcd"],
            "styles": [[
                {"x": 0, "length": 3, "fg": 2, "bg": 0, "bold": false},
                {"x": 2, "length": 2, "fg": 3, "bg": 0, "bold": false}
            ]],
        });
        assert!(screen(overlap).is_err());

        let unsupported = json!({
            "kind": "screen", "version": 2, "width": 1, "height": 1,
            "rows": ["x"], "styles": [[]],
        });
        assert!(screen(unsupported).is_err());
    }

    #[test]
    fn enforces_screen_dimension_and_style_span_caps() {
        let too_large = json!({
            "kind": "screen", "version": 1, "width": MAX_SCREEN_WIDTH + 1,
            "height": 1, "rows": ["x"], "styles": [[]],
        });
        assert!(screen(too_large).is_err());
    }
}
