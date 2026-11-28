//! A manually stepped Conway world. The host supplies every input.

use plaza_sdk::World;
use plaza_sdk::grid::Grid;
use plaza_sdk::screen::{Screen, Style};

const SIZE: usize = 5;

pub struct Board {
    cells: Grid<bool>,
    generation: u32,
}

impl Board {
    fn glider() -> Self {
        let mut cells = Grid::new(SIZE, SIZE, false);
        for (x, y) in [(1, 0), (2, 1), (0, 2), (1, 2), (2, 2)] {
            cells.set(x, y, true);
        }
        Self {
            cells,
            generation: 0,
        }
    }

    fn step(&mut self) {
        let Some(generation) = self.generation.checked_add(1) else {
            return;
        };
        let mut next = Grid::new(SIZE, SIZE, false);
        for y in 0..SIZE {
            for x in 0..SIZE {
                let mut neighbors = 0;
                for row in y.saturating_sub(1)..=(y + 1).min(SIZE - 1) {
                    for col in x.saturating_sub(1)..=(x + 1).min(SIZE - 1) {
                        if (x, y) != (col, row) && self.cells.get(col, row, false) {
                            neighbors += 1;
                        }
                    }
                }
                let alive = neighbors == 3 || (neighbors == 2 && self.cells.get(x, y, false));
                next.set(x, y, alive);
            }
        }
        self.cells = next;
        self.generation = generation;
    }

    fn row(&self, y: usize) -> String {
        self.cells
            .row(y)
            .into_iter()
            .map(|alive| if alive { '#' } else { '.' })
            .collect()
    }

    fn snapshot(&self) -> String {
        let rows: String = (0..SIZE).map(|y| self.row(y)).collect();
        format!("1:{}:{rows}", self.generation)
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        let mut fields = std::str::from_utf8(bytes).ok()?.splitn(3, ':');
        if fields.next()? != "1" {
            return None;
        }
        let generation = fields.next()?.parse().ok()?;
        let cells_text = fields.next()?.as_bytes();
        if cells_text.len() != SIZE * SIZE {
            return None;
        }
        let mut cells = Grid::new(SIZE, SIZE, false);
        for (i, &ch) in cells_text.iter().enumerate() {
            match ch {
                b'#' => cells.set(i % SIZE, i / SIZE, true),
                b'.' => (),
                _ => return None,
            }
        }
        Some(Self { cells, generation })
    }
}

/// The Plaza world entry points shared by the terminal, web and other hosts.
pub struct Cells;

impl World for Cells {
    type Model = Board;

    fn init(_seed: u64) -> Board {
        Board::glider()
    }

    fn update(mut model: Board, _participant: u32, event: &[u8]) -> Board {
        let Ok(input) = std::str::from_utf8(event) else {
            return model;
        };
        let mut words = input.split_ascii_whitespace();
        match (words.next(), words.next(), words.next(), words.next()) {
            (Some("step"), None, None, None) => model.step(),
            (Some("reset"), None, None, None) => return Board::glider(),
            (Some("toggle"), Some(col), Some(row), None) if model.generation == 0 => {
                if let (Ok(x), Ok(y)) = (col.parse::<usize>(), row.parse::<usize>()) {
                    if model.cells.in_bounds(x, y) {
                        model.cells.set(x, y, !model.cells.get(x, y, false));
                    }
                }
            }
            _ => (),
        }
        model
    }

    fn view(model: &Board) -> Vec<u8> {
        let mut screen = Screen::new(24, SIZE + 2);
        let style = Style::default();
        screen.put_text(0, 0, &format!("Cells / {}", model.generation), style);
        for y in 0..SIZE {
            screen.put_text(0, y + 1, &model.row(y), style);
        }
        screen.put_text(0, SIZE + 1, "step | reset", style);
        screen.render_json().into_bytes()
    }

    fn records(model: &Board) -> Option<Vec<u8>> {
        let mut live = Vec::new();
        for y in 0..SIZE {
            for x in 0..SIZE {
                if model.cells.get(x, y, false) {
                    live.push(format!("[{x},{y}]"));
                }
            }
        }
        Some(
            format!(
                r#"{{"generation":{},"live":[{}]}}"#,
                model.generation,
                live.join(",")
            )
            .into_bytes(),
        )
    }

    fn snapshot(model: &Board) -> Option<Vec<u8>> {
        Some(model.snapshot().into_bytes())
    }

    fn restore(bytes: &[u8]) -> Board {
        Board::decode(bytes).unwrap_or_else(Board::glider)
    }
}

plaza_sdk::plaza_guest!(Cells);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dead_exterior_and_in_place_update_oracle() {
        // The top-edge blinker loses its out-of-bounds northern birth.
        let mut m = Cells::restore(b"1:0:###......................");
        m.step();
        assert_eq!(m.snapshot(), "1:1:.#....#..................");
    }

    #[test]
    fn test_versioned_restore_is_total() {
        let start = Cells::init(0).snapshot();
        for bad in [
            &b""[..],
            &b"2:0:........................."[..],
            &b"1:0:short"[..],
            &[0xff],
        ] {
            assert_eq!(Cells::restore(bad).snapshot(), start);
        }
    }
}
