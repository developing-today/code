//! Colony hauling world (#13605), the Rust twin of the Roc colony guest.
//!
//! The pure domain: a worker hauls one ore at a time from a 4x2 board to
//! a stockpile. Mining takes one adjacent ore and leaves the worker in
//! place; a full pack refuses mining; deposit is allowed only on the
//! stockpile. Two ore are conserved across cells, pack and stockpile.
//!
//! Oracle: `colony/two-deliveries` in `pkgs/id/book/research/evidence/app-vectors.json`
//! at commit b9d8177a6cf. Expected states are literal projections of that
//! vector; they are not derived from this guest.

pub const WIDTH: i64 = 4;
pub const HEIGHT: i64 = 2;
pub const STOCKPILE: (i64, i64) = (0, 1);
pub const PACK_CAPACITY: i64 = 1;
pub const DELIVERY_GOAL: i64 = 2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Colony {
    pub worker: (i64, i64),
    pub ore_cells: Vec<(i64, i64)>,
    pub pack_ore: i64,
    pub stored_ore: i64,
    pub phase: Phase,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Working,
    Supplied,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Mine(Dir),
    Walk(Dir),
    Deposit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    N,
    E,
    S,
    W,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Extracted,
    PackFull,
    Moved,
    Blocked,
    Stored,
    SupplyGoalMet,
    NotAtStockpile,
    Empty,
}

pub fn offset(at: (i64, i64), dir: Dir) -> (i64, i64) {
    match dir {
        Dir::N => (at.0, at.1 - 1),
        Dir::S => (at.0, at.1 + 1),
        Dir::E => (at.0 + 1, at.1),
        Dir::W => (at.0 - 1, at.1),
    }
}

pub fn in_bounds(at: (i64, i64)) -> bool {
    at.0 >= 0 && at.0 < WIDTH && at.1 >= 0 && at.1 < HEIGHT
}

impl Colony {
    pub fn initial() -> Self {
        Colony {
            worker: (1, 1),
            ore_cells: vec![(1, 0), (2, 1)],
            pack_ore: 0,
            stored_ore: 0,
            phase: Phase::Working,
        }
    }

    pub fn step(&self, action: Action) -> (Colony, Outcome) {
        if self.phase == Phase::Supplied {
            return (self.clone(), Outcome::Empty);
        }
        let mut next = self.clone();
        let outcome = match action {
            Action::Mine(dir) => {
                let target = offset(self.worker, dir);
                match self.ore_cells.iter().position(|c| *c == target) {
                    None => Outcome::Empty,
                    Some(_) if self.pack_ore >= PACK_CAPACITY => Outcome::PackFull,
                    Some(i) => {
                        next.ore_cells.remove(i);
                        next.pack_ore += 1;
                        Outcome::Extracted
                    }
                }
            }
            Action::Walk(dir) => {
                let target = offset(self.worker, dir);
                if !in_bounds(target) || self.ore_cells.contains(&target) {
                    Outcome::Blocked
                } else {
                    next.worker = target;
                    Outcome::Moved
                }
            }
            Action::Deposit => {
                if self.worker != STOCKPILE {
                    Outcome::NotAtStockpile
                } else if self.pack_ore == 0 {
                    Outcome::Empty
                } else {
                    next.pack_ore -= 1;
                    next.stored_ore += 1;
                    if next.stored_ore >= DELIVERY_GOAL {
                        next.phase = Phase::Supplied;
                        Outcome::SupplyGoalMet
                    } else {
                        Outcome::Stored
                    }
                }
            }
        };
        (next, outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cells(v: &[(i64, i64)]) -> Vec<(i64, i64)> {
        v.to_vec()
    }

    #[test]
    fn two_deliveries_replays_the_fixture_oracle() {
        let mut state = Colony::initial();
        let steps = [
            (Action::Mine(Dir::E), Outcome::Extracted, (1, 1), cells(&[(1, 0)]), 1, 0, Phase::Working),
            (Action::Mine(Dir::N), Outcome::PackFull, (1, 1), cells(&[(1, 0)]), 1, 0, Phase::Working),
            (Action::Walk(Dir::W), Outcome::Moved, (0, 1), cells(&[(1, 0)]), 1, 0, Phase::Working),
            (Action::Deposit, Outcome::Stored, (0, 1), cells(&[(1, 0)]), 0, 1, Phase::Working),
            (Action::Walk(Dir::E), Outcome::Moved, (1, 1), cells(&[(1, 0)]), 0, 1, Phase::Working),
            (Action::Mine(Dir::N), Outcome::Extracted, (1, 1), cells(&[]), 1, 1, Phase::Working),
            (Action::Walk(Dir::W), Outcome::Moved, (0, 1), cells(&[]), 1, 1, Phase::Working),
            (Action::Deposit, Outcome::SupplyGoalMet, (0, 1), cells(&[]), 0, 2, Phase::Supplied),
        ];
        for (n, (action, outcome, worker, ore, pack, stored, phase)) in steps.into_iter().enumerate() {
            let (next, got) = state.step(action);
            assert_eq!(got, outcome, "step {}", n + 1);
            assert_eq!(next.worker, worker, "step {} worker", n + 1);
            assert_eq!(next.ore_cells, ore, "step {} ore_cells", n + 1);
            assert_eq!(next.pack_ore, pack, "step {} pack_ore", n + 1);
            assert_eq!(next.stored_ore, stored, "step {} stored_ore", n + 1);
            assert_eq!(next.phase, phase, "step {} phase", n + 1);
            state = next;
        }
    }

    #[test]
    fn ore_is_conserved_across_cells_pack_and_stockpile() {
        let mut state = Colony::initial();
        for action in [
            Action::Mine(Dir::E),
            Action::Walk(Dir::W),
            Action::Deposit,
        ] {
            state = state.step(action).0;
            assert_eq!(
                state.ore_cells.len() as i64 + state.pack_ore + state.stored_ore,
                2
            );
        }
    }
}
