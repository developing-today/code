//! Row-major byte grid mirroring `RocGrid.roc`: bounds-checked and
//! total — a read outside the grid returns a caller-supplied fallback,
//! a write outside it returns the grid unchanged. Never traps.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grid<T: Copy> {
    pub width: usize,
    pub height: usize,
    cells: Vec<T>,
}

impl<T: Copy> Grid<T> {
    pub fn new(width: usize, height: usize, fill: T) -> Self {
        Grid {
            width,
            height,
            cells: vec![fill; width.saturating_mul(height)],
        }
    }

    pub fn in_bounds(&self, x: usize, y: usize) -> bool {
        x < self.width && y < self.height
    }

    pub fn index(&self, x: usize, y: usize) -> Option<usize> {
        if self.in_bounds(x, y) {
            Some(y * self.width + x)
        } else {
            None
        }
    }

    pub fn get(&self, x: usize, y: usize, fallback: T) -> T {
        self.index(x, y).map(|i| self.cells[i]).unwrap_or(fallback)
    }

    pub fn set(&mut self, x: usize, y: usize, value: T) {
        if let Some(i) = self.index(x, y) {
            self.cells[i] = value;
        }
    }

    pub fn count(&self, value: T) -> usize
    where
        T: PartialEq,
    {
        self.cells.iter().filter(|c| **c == value).count()
    }

    /// The four orthogonal neighbours that are inside the grid,
    /// in RocGrid order: up, right, down, left.
    pub fn neighbors4(&self, x: usize, y: usize) -> Vec<(usize, usize)> {
        let mut out = Vec::with_capacity(4);
        if y > 0 && self.in_bounds(x, y - 1) {
            out.push((x, y - 1));
        }
        if self.in_bounds(x + 1, y) {
            out.push((x + 1, y));
        }
        if self.in_bounds(x, y + 1) {
            out.push((x, y + 1));
        }
        if x > 0 && self.in_bounds(x - 1, y) {
            out.push((x - 1, y));
        }
        out
    }

    /// One row as a Vec, or an empty Vec for an out-of-range row.
    pub fn row(&self, y: usize) -> Vec<T> {
        if y >= self.height {
            return Vec::new();
        }
        self.cells[y * self.width..(y + 1) * self.width].to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_is_total_at_the_edges() {
        let mut g = Grid::new(3, 2, 0u8);
        assert!(g.in_bounds(2, 1));
        assert!(!g.in_bounds(3, 0));
        assert_eq!(g.get(9, 9, 7), 7);
        g.set(9, 9, 5);
        assert_eq!(g.count(5), 0);
        g.set(1, 1, 5);
        assert_eq!(g.get(1, 1, 0), 5);
        assert_eq!(g.count(5), 1);
        assert_eq!(g.row(0), vec![0, 0, 0]);
        assert_eq!(g.row(5), Vec::<u8>::new());
        assert_eq!(g.neighbors4(0, 0), vec![(1, 0), (0, 1)]);
        assert_eq!(g.neighbors4(1, 0), vec![(2, 0), (1, 1), (0, 0)]);
    }
}
