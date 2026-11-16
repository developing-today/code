//! Portable screen mirroring `Screen.roc`: a cell grid with per-cell
//! styles that renders to plain text or to the versioned JSON document
//! (`{"kind":"screen","version":1,...}`) the host validates. A column is
//! one Unicode scalar value. The default style (fg 0, bg 0, not bold)
//! writes no style span, exactly like the Roc renderer.

use crate::grid::Grid;
use crate::json::quote;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub fg: u8,
    pub bg: u8,
    pub bold: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Cell {
    ch: char,
    style: Style,
}

#[derive(Clone, Debug)]
pub struct Screen {
    pub width: usize,
    pub height: usize,
    cells: Grid<Cell>,
}

impl Screen {
    pub fn new(width: usize, height: usize) -> Self {
        Screen {
            width,
            height,
            cells: Grid::new(
                width,
                height,
                Cell {
                    ch: ' ',
                    style: Style::default(),
                },
            ),
        }
    }

    pub fn put_text(&mut self, x: usize, y: usize, text: &str, style: Style) {
        for (i, ch) in text.chars().enumerate() {
            self.cells.set(x + i, y, Cell { ch, style });
        }
    }

    pub fn put_grid<T: Copy>(
        &mut self,
        grid: &Grid<T>,
        x: usize,
        y: usize,
        fallback: T,
        glyph: impl Fn(T) -> char,
        style: Style,
    ) {
        for gy in 0..grid.height {
            for gx in 0..grid.width {
                let cell = Cell {
                    ch: glyph(grid.get(gx, gy, fallback)),
                    style,
                };
                self.cells.set(x + gx, y + gy, cell);
            }
        }
    }

    /// Draw a box: `+---+` edges with the title on the top edge and
    /// `|` sides, like `Screen.boxed`.
    pub fn boxed(&mut self, x: usize, y: usize, w: usize, h: usize, title: &str) {
        if w < 2 || h < 2 {
            return;
        }
        let style = Style::default();
        let top = box_edge(w, title);
        let bottom = box_edge(w, "");
        for (i, ch) in top.chars().enumerate() {
            self.cells.set(x + i, y, Cell { ch, style });
        }
        for row in 1..h - 1 {
            self.cells.set(x, y + row, Cell { ch: '|', style });
            self.cells.set(x + w - 1, y + row, Cell { ch: '|', style });
        }
        for (i, ch) in bottom.chars().enumerate() {
            self.cells.set(x + i, y + h - 1, Cell { ch, style });
        }
    }

    fn row_text(&self, row: usize) -> String {
        self.cells
            .row(row)
            .iter()
            .map(|c| c.ch)
            .collect::<String>()
            .trim_end_matches(|c| c == '\u{0}')
            .to_string()
    }

    pub fn render_plain(&self) -> String {
        (0..self.height)
            .map(|row| {
                let text: String = self.cells.row(row).iter().map(|c| c.ch).collect();
                text
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn row_styles_json(&self, row: usize) -> String {
        let cells = self.cells.row(row);
        let mut spans: Vec<String> = Vec::new();
        let mut run_start = 0usize;
        let mut run_len = 0usize;
        let mut run_style = Style::default();
        let close = |spans: &mut Vec<String>, start: usize, len: usize, style: Style| {
            if len > 0 && style != Style::default() {
                spans.push(format!(
                    "{{\"x\":{},\"length\":{},\"fg\":{},\"bg\":{},\"bold\":{}}}",
                    start, len, style.fg, style.bg, style.bold
                ));
            }
        };
        for (x, cell) in cells.iter().enumerate() {
            if run_len > 0 && cell.style == run_style {
                run_len += 1;
            } else {
                close(&mut spans, run_start, run_len, run_style);
                run_start = x;
                run_len = 1;
                run_style = cell.style;
            }
        }
        close(&mut spans, run_start, run_len, run_style);
        format!("[{}]", spans.join(","))
    }

    pub fn render_json(&self) -> String {
        let rows = (0..self.height)
            .map(|row| quote(&self.row_text(row)))
            .collect::<Vec<_>>()
            .join(",");
        let styles = (0..self.height)
            .map(|row| self.row_styles_json(row))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "{{\"kind\":\"screen\",\"version\":1,\"width\":{},\"height\":{},\"rows\":[{}],\"styles\":[{}]}}",
            self.width, self.height, rows, styles
        )
    }
}

/// One box edge, like `Screen.box_edge`: `+-` + title + `-` fill + `+`,
/// with the title cut so the edge is exactly `width` columns.
pub fn box_edge(width: usize, title: &str) -> String {
    if width < 2 {
        return "+".repeat(width);
    }
    let mut edge = String::from("+-");
    let room = width.saturating_sub(3);
    for ch in title.chars().take(room) {
        edge.push(ch);
    }
    while edge.chars().count() < width - 1 {
        edge.push('-');
    }
    edge.push('+');
    edge
}

/// Quote a string as JSON, exported for parity with `Screen.json_quote`.
pub fn json_quote(s: &str) -> String {
    quote(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_json_matches_roc_screen_expects() {
        // Screen.roc `expect` values, byte for byte.
        let mut s = Screen::new(2, 1);
        s.put_text(0, 0, "ok", Style::default());
        assert_eq!(
            s.render_json(),
            "{\"kind\":\"screen\",\"version\":1,\"width\":2,\"height\":1,\"rows\":[\"ok\"],\"styles\":[[]]}"
        );
        let mut s = Screen::new(3, 1);
        s.put_text(
            0,
            0,
            "abc",
            Style {
                fg: 196,
                bg: 0,
                bold: true,
            },
        );
        assert_eq!(
            s.render_json(),
            "{\"kind\":\"screen\",\"version\":1,\"width\":3,\"height\":1,\"rows\":[\"abc\"],\"styles\":[[{\"x\":0,\"length\":3,\"fg\":196,\"bg\":0,\"bold\":true}]]}"
        );
        assert_eq!(json_quote("a\"\\\n"), "\"a\\\"\\\\\\n\"");
    }

    #[test]
    fn boxed_and_plain_match_roc_screen_expects() {
        let mut s = Screen::new(6, 3);
        s.boxed(0, 0, 6, 3, "t");
        assert_eq!(s.render_plain(), "+-t--+\n|    |\n+----+");
        assert_eq!(box_edge(8, "ti"), "+-ti---+");
        assert_eq!(box_edge(6, "toolong"), "+-too+");
        assert_eq!(box_edge(4, ""), "+--+");
        let mut s = Screen::new(4, 2);
        s.put_text(1, 0, "hi", Style::default());
        assert_eq!(s.render_plain(), " hi \n    ");
    }
}
