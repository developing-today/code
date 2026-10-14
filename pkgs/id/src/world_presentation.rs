//! Validated, transport-neutral presentations emitted by world programs.
//!
//! Legacy UTF-8 text remains valid. A program opts into a portable screen by
//! returning the versioned `kind: screen` JSON document parsed here. Guests
//! never supply terminal escapes, HTML, or CSS: adapters render this value
//! using their own trusted presentation code.

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
        let width = usize::from(self.width);
        let height = usize::from(self.height);
        if width == 0
            || width > MAX_SCREEN_WIDTH
            || height == 0
            || height > MAX_SCREEN_HEIGHT
            || self.cells.len() != width.checked_mul(height)?
        {
            return None;
        }
        for (index, cell) in self.cells.iter().enumerate() {
            let x = index % width;
            if cell.columns == 0 {
                if x == 0 {
                    return None;
                }
                let previous = self.cells.get(index.checked_sub(1)?)?;
                if previous.columns != 2
                    || previous.fg != cell.fg
                    || previous.bg != cell.bg
                    || previous.bold != cell.bold
                    || !cell.symbol.is_empty()
                {
                    return None;
                }
            } else if cell.columns == 1 || cell.columns == 2 {
                if cell.symbol.graphemes(true).count() != 1
                    || cell.symbol.chars().any(char::is_control)
                    || UnicodeWidthStr::width(cell.symbol.as_str()) != usize::from(cell.columns)
                    || (cell.columns == 2 && x + 1 == width)
                {
                    return None;
                }
                if cell.columns == 2 {
                    let continuation = self.cells.get(index.checked_add(1)?)?;
                    if continuation.columns != 0
                        || continuation.fg != cell.fg
                        || continuation.bg != cell.bg
                        || continuation.bold != cell.bold
                        || !continuation.symbol.is_empty()
                    {
                        return None;
                    }
                }
            } else {
                return None;
            }
        }
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

    #[test]
    fn plain_text_stays_on_the_legacy_path() {
        assert_eq!(parse_surface("hello\nworld").unwrap(), None);
        assert_eq!(
            parse_surface(r#"{"kind":"other","version":1}"#).unwrap(),
            None
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
