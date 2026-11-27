//! Colony hauling world (#13605): a playable Plaza ABI v1 guest.
//!
//! The pure domain: a worker hauls one ore at a time from a 4x2 board to
//! a stockpile. Mining takes one adjacent ore and leaves the worker in
//! place; a full pack refuses mining; deposit is allowed only on the
//! stockpile. Two ore are conserved across cells, pack and stockpile.
//!
//! Oracle: `colony/two-deliveries` in `pkgs/id/book/research/evidence/app-vectors.json`
//! at commit b9d8177a6cf. Expected states are literal projections of that
//! vector; they are not derived from this guest.
//!
//! The pure domain (`Colony::step`) sits on top; the Plaza entry points
//! (`ColonyWorld`) wrap it with input parsing, replay folding by event id,
//! records, a versioned snapshot and a text view.

use plaza_sdk::World;
use plaza_sdk::json::{self, Value, quote};
use plaza_sdk::screen::{Screen, Style};

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
    /// The supply goal is met; the colony takes no more actions.
    AlreadySupplied,
}

impl Outcome {
    pub fn name(self) -> &'static str {
        match self {
            Outcome::Extracted => "extracted",
            Outcome::PackFull => "pack-full",
            Outcome::Moved => "moved",
            Outcome::Blocked => "blocked",
            Outcome::Stored => "stored",
            Outcome::SupplyGoalMet => "supply-goal-met",
            Outcome::NotAtStockpile => "not-at-stockpile",
            Outcome::Empty => "empty",
            Outcome::AlreadySupplied => "already-supplied",
        }
    }
}

impl Phase {
    pub fn name(self) -> &'static str {
        match self {
            Phase::Working => "working",
            Phase::Supplied => "supplied",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "working" => Some(Phase::Working),
            "supplied" => Some(Phase::Supplied),
            _ => None,
        }
    }
}

impl Dir {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "N" => Some(Dir::N),
            "E" => Some(Dir::E),
            "S" => Some(Dir::S),
            "W" => Some(Dir::W),
            _ => None,
        }
    }
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
            return (self.clone(), Outcome::AlreadySupplied);
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

/// Event ids remembered so a replayed delivery folds once.
const FOLDED_LIMIT: usize = 32;
const MAX_ID_BYTES: usize = 16;
const SNAPSHOT_VERSION: i64 = 1;
/// Ore in play: two on the board at the start, conserved afterwards.
const TOTAL_ORE: i64 = 2;

/// The guest model: the domain state, the last outcome, and the event ids
/// already applied (host-side replay memory, kept in the snapshot only).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Game {
    colony: Colony,
    outcome: String,
    folded: Vec<String>,
}

impl Game {
    fn level() -> Self {
        Self {
            colony: Colony::initial(),
            outcome: String::new(),
            folded: Vec::new(),
        }
    }

    fn apply(&mut self, input: &Value) -> String {
        let Some(event) = input.get("event").and_then(Value::as_str) else {
            return "refused".into();
        };
        if !valid_id(event) {
            return "refused".into();
        }
        let direction = || {
            input
                .get("direction")
                .and_then(Value::as_str)
                .and_then(Dir::parse)
        };
        let action = match input.get("kind").and_then(Value::as_str) {
            Some("Mine") => direction().map(Action::Mine),
            Some("Walk") => direction().map(Action::Walk),
            Some("Deposit") => Some(Action::Deposit),
            _ => None,
        };
        let Some(action) = action else {
            return "refused".into();
        };
        if self.folded.iter().any(|seen| seen == event) {
            return "already-folded".into();
        }
        let (next, outcome) = self.colony.step(action);
        self.colony = next;
        if self.folded.len() == FOLDED_LIMIT {
            self.folded.remove(0);
        }
        self.folded.push(event.into());
        outcome.name().into()
    }

    /// The vector's domain projection in its key order, then the last
    /// outcome; `v` and `folded` are added only to the snapshot.
    fn document(&self, snapshot: bool) -> String {
        let c = &self.colony;
        let ore = c
            .ore_cells
            .iter()
            .map(|(x, y)| format!("[{x},{y}]"))
            .collect::<Vec<_>>()
            .join(",");
        let mut doc = String::from("{");
        if snapshot {
            doc.push_str(&format!(r#""v":{SNAPSHOT_VERSION},"#));
        }
        doc.push_str(&format!(
            r#""worker":[{},{}],"ore_cells":[{ore}],"pack_ore":{},"stored_ore":{},"phase":"{}","outcome":{}"#,
            c.worker.0,
            c.worker.1,
            c.pack_ore,
            c.stored_ore,
            c.phase.name(),
            quote(&self.outcome)
        ));
        if snapshot {
            let folded = self.folded.iter().map(|id| quote(id)).collect::<Vec<_>>();
            doc.push_str(&format!(r#","folded":[{}]"#, folded.join(",")));
        }
        doc.push('}');
        doc
    }

    /// A snapshot is accepted only when it describes a reachable colony:
    /// cells on the board, no ore under the worker, ore conserved, and the
    /// phase matching the stockpile.
    fn decode(bytes: &[u8]) -> Option<Self> {
        let doc = json::decode(std::str::from_utf8(bytes).ok()?).ok()?;
        if doc.get("v")?.as_i64()? != SNAPSHOT_VERSION {
            return None;
        }
        let worker = cell(doc.get("worker")?)?;
        let ore_cells = doc
            .get("ore_cells")?
            .as_array()?
            .iter()
            .map(cell)
            .collect::<Option<Vec<_>>>()?;
        let pack_ore = doc.get("pack_ore")?.as_i64()?;
        let stored_ore = doc.get("stored_ore")?.as_i64()?;
        let phase = Phase::parse(doc.get("phase")?.as_str()?)?;
        let unique = ore_cells
            .iter()
            .enumerate()
            .all(|(i, c)| !ore_cells[..i].contains(c));
        if !unique
            || ore_cells.contains(&worker)
            || !(0..=PACK_CAPACITY).contains(&pack_ore)
            || !(0..=DELIVERY_GOAL).contains(&stored_ore)
            || ore_cells.len() as i64 + pack_ore + stored_ore != TOTAL_ORE
            || (phase == Phase::Supplied) != (stored_ore >= DELIVERY_GOAL)
        {
            return None;
        }
        let folded = doc
            .get("folded")?
            .as_array()?
            .iter()
            .map(|id| id.as_str().filter(|id| valid_id(id)).map(String::from))
            .collect::<Option<Vec<_>>>()?;
        if folded.len() > FOLDED_LIMIT {
            return None;
        }
        Some(Self {
            colony: Colony {
                worker,
                ore_cells,
                pack_ore,
                stored_ore,
                phase,
            },
            outcome: doc.get("outcome")?.as_str()?.chars().take(64).collect(),
            folded,
        })
    }

    fn glyph(&self, at: (i64, i64)) -> char {
        if at == self.colony.worker {
            '@'
        } else if self.colony.ore_cells.contains(&at) {
            '*'
        } else if at == STOCKPILE {
            'S'
        } else {
            '.'
        }
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID_BYTES
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn cell(value: &Value) -> Option<(i64, i64)> {
    match value.as_array()? {
        [x, y] => {
            let at = (x.as_i64()?, y.as_i64()?);
            in_bounds(at).then_some(at)
        }
        _ => None,
    }
}

/// The Plaza world entry points shared by the terminal, web and other hosts.
pub struct ColonyWorld;

impl World for ColonyWorld {
    type Model = Game;

    fn init(_seed: u64) -> Game {
        Game::level()
    }

    fn update(mut model: Game, participant: u32, event: &[u8]) -> Game {
        // Participant 0 is the host (capability results, subscriptions);
        // the colony asks for neither, so host events never drive it.
        if participant == 0 {
            return model;
        }
        let outcome = match std::str::from_utf8(event).ok().map(json::decode) {
            Some(Ok(input)) => model.apply(&input),
            _ => "refused".into(),
        };
        model.outcome = outcome;
        model
    }

    fn view(model: &Game) -> Vec<u8> {
        let rows = HEIGHT as usize;
        let mut screen = Screen::new(32, rows + 4);
        let style = Style::default();
        screen.put_text(0, 0, "Colony", style);
        for y in 0..HEIGHT {
            let row: Vec<String> = (0..WIDTH)
                .map(|x| model.glyph((x, y)).to_string())
                .collect();
            screen.put_text(0, y as usize + 1, &row.join(" "), style);
        }
        let c = &model.colony;
        let status = format!(
            "pack={}/{PACK_CAPACITY} stored={}/{DELIVERY_GOAL}",
            c.pack_ore, c.stored_ore
        );
        screen.put_text(0, rows + 1, &status, style);
        screen.put_text(0, rows + 2, "@ worker * ore S stockpile", style);
        let last = if model.outcome.is_empty() {
            "-"
        } else {
            model.outcome.as_str()
        };
        screen.put_text(0, rows + 3, &format!("{} {last}", c.phase.name()), style);
        screen.render_json().into_bytes()
    }

    fn records(model: &Game) -> Option<Vec<u8>> {
        Some(model.document(false).into_bytes())
    }

    fn snapshot(model: &Game) -> Option<Vec<u8>> {
        Some(model.document(true).into_bytes())
    }

    fn restore(bytes: &[u8]) -> Game {
        Game::decode(bytes).unwrap_or_else(Game::level)
    }
}

plaza_sdk::plaza_guest!(ColonyWorld);

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
            (
                Action::Mine(Dir::E),
                Outcome::Extracted,
                (1, 1),
                cells(&[(1, 0)]),
                1,
                0,
                Phase::Working,
            ),
            (
                Action::Mine(Dir::N),
                Outcome::PackFull,
                (1, 1),
                cells(&[(1, 0)]),
                1,
                0,
                Phase::Working,
            ),
            (
                Action::Walk(Dir::W),
                Outcome::Moved,
                (0, 1),
                cells(&[(1, 0)]),
                1,
                0,
                Phase::Working,
            ),
            (
                Action::Deposit,
                Outcome::Stored,
                (0, 1),
                cells(&[(1, 0)]),
                0,
                1,
                Phase::Working,
            ),
            (
                Action::Walk(Dir::E),
                Outcome::Moved,
                (1, 1),
                cells(&[(1, 0)]),
                0,
                1,
                Phase::Working,
            ),
            (
                Action::Mine(Dir::N),
                Outcome::Extracted,
                (1, 1),
                cells(&[]),
                1,
                1,
                Phase::Working,
            ),
            (
                Action::Walk(Dir::W),
                Outcome::Moved,
                (0, 1),
                cells(&[]),
                1,
                1,
                Phase::Working,
            ),
            (
                Action::Deposit,
                Outcome::SupplyGoalMet,
                (0, 1),
                cells(&[]),
                0,
                2,
                Phase::Supplied,
            ),
        ];
        for (n, (action, outcome, worker, ore, pack, stored, phase)) in
            steps.into_iter().enumerate()
        {
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
        for action in [Action::Mine(Dir::E), Action::Walk(Dir::W), Action::Deposit] {
            state = state.step(action).0;
            assert_eq!(
                state.ore_cells.len() as i64 + state.pack_ore + state.stored_ore,
                2
            );
        }
    }

    fn play(game: Game, input: &str) -> Game {
        ColonyWorld::update(game, 1, input.as_bytes())
    }

    fn record(game: &Game) -> Value {
        json::decode(&String::from_utf8(ColonyWorld::records(game).unwrap()).unwrap()).unwrap()
    }

    /// The vector's literal after-state with the outcome appended.
    fn expected(after: &str, outcome: &str) -> Value {
        let text = format!(
            "{},\"outcome\":{}}}",
            after.strip_suffix('}').unwrap(),
            quote(outcome)
        );
        json::decode(&text).unwrap()
    }

    // Oracle: colony/two-deliveries in app-vectors.json (revision b9d8177).
    // Inputs, outcomes and after-states are literal copies.
    const VECTOR: [(&str, &str, &str); 8] = [
        (
            r#"{"kind":"Mine","direction":"E","event":"m1"}"#,
            "extracted",
            r#"{"worker":[1,1],"ore_cells":[[1,0]],"pack_ore":1,"stored_ore":0,"phase":"working"}"#,
        ),
        (
            r#"{"kind":"Mine","direction":"N","event":"m2"}"#,
            "pack-full",
            r#"{"worker":[1,1],"ore_cells":[[1,0]],"pack_ore":1,"stored_ore":0,"phase":"working"}"#,
        ),
        (
            r#"{"kind":"Walk","direction":"W","event":"w1"}"#,
            "moved",
            r#"{"worker":[0,1],"ore_cells":[[1,0]],"pack_ore":1,"stored_ore":0,"phase":"working"}"#,
        ),
        (
            r#"{"kind":"Deposit","event":"d1"}"#,
            "stored",
            r#"{"worker":[0,1],"ore_cells":[[1,0]],"pack_ore":0,"stored_ore":1,"phase":"working"}"#,
        ),
        (
            r#"{"kind":"Walk","direction":"E","event":"w2"}"#,
            "moved",
            r#"{"worker":[1,1],"ore_cells":[[1,0]],"pack_ore":0,"stored_ore":1,"phase":"working"}"#,
        ),
        (
            r#"{"kind":"Mine","direction":"N","event":"m3"}"#,
            "extracted",
            r#"{"worker":[1,1],"ore_cells":[],"pack_ore":1,"stored_ore":1,"phase":"working"}"#,
        ),
        (
            r#"{"kind":"Walk","direction":"W","event":"w3"}"#,
            "moved",
            r#"{"worker":[0,1],"ore_cells":[],"pack_ore":1,"stored_ore":1,"phase":"working"}"#,
        ),
        (
            r#"{"kind":"Deposit","event":"d2"}"#,
            "supply-goal-met",
            r#"{"worker":[0,1],"ore_cells":[],"pack_ore":0,"stored_ore":2,"phase":"supplied"}"#,
        ),
    ];

    #[test]
    fn guest_replays_two_deliveries_through_the_plaza_entry_points() {
        let mut game = ColonyWorld::init(0);
        assert_eq!(
            record(&game),
            expected(
                r#"{"worker":[1,1],"ore_cells":[[1,0],[2,1]],"pack_ore":0,"stored_ore":0,"phase":"working"}"#,
                ""
            )
        );
        for (input, outcome, after) in VECTOR {
            game = play(game, input);
            assert_eq!(record(&game), expected(after, outcome), "{input}");
        }
        let done = play(
            game.clone(),
            r#"{"kind":"Walk","direction":"E","event":"w4"}"#,
        );
        assert_eq!(done.outcome, "already-supplied");
        assert_eq!(done.colony, game.colony);
    }

    #[test]
    fn replayed_events_fold_once_and_bad_input_is_refused() {
        let game = play(ColonyWorld::init(0), VECTOR[0].0);
        let again = play(game.clone(), VECTOR[0].0);
        assert_eq!(again.outcome, "already-folded");
        assert_eq!(again.colony, game.colony);
        for bad in [
            r#"{"kind":"Mine","direction":"E"}"#,
            r#"{"kind":"Mine","direction":"X","event":"m9"}"#,
            r#"{"kind":"Fly","event":"f1"}"#,
            r#"{"kind":"Deposit","event":""}"#,
            r#"{"kind":"Deposit","event":"this-id-is-far-too-long"}"#,
            "not json",
        ] {
            let after = play(game.clone(), bad);
            assert_eq!(after.outcome, "refused", "{bad}");
            assert_eq!(after.colony, game.colony, "{bad}");
        }
        let host = ColonyWorld::update(game.clone(), 0, VECTOR[2].0.as_bytes());
        assert_eq!(host, game);
    }

    #[test]
    fn walls_ore_and_the_stockpile_constrain_moves() {
        let start = ColonyWorld::init(0);
        let cases = [
            (r#"{"kind":"Walk","direction":"E","event":"a"}"#, "blocked"),
            (r#"{"kind":"Walk","direction":"S","event":"b"}"#, "blocked"),
            (r#"{"kind":"Walk","direction":"N","event":"c"}"#, "blocked"),
            (r#"{"kind":"Deposit","event":"d"}"#, "not-at-stockpile"),
            (r#"{"kind":"Mine","direction":"W","event":"e"}"#, "empty"),
        ];
        for (input, outcome) in cases {
            assert_eq!(play(start.clone(), input).outcome, outcome, "{input}");
        }
        let at_stock = play(start, r#"{"kind":"Walk","direction":"W","event":"w"}"#);
        assert_eq!(
            play(at_stock, r#"{"kind":"Deposit","event":"d"}"#).outcome,
            "empty"
        );
    }

    #[test]
    fn snapshot_round_trips_and_restore_is_total() {
        let mut game = ColonyWorld::init(0);
        for (input, _, _) in &VECTOR[..3] {
            game = play(game, input);
        }
        let bytes = ColonyWorld::snapshot(&game).unwrap();
        let back = ColonyWorld::restore(&bytes);
        assert_eq!(back, game);
        assert_eq!(play(back, VECTOR[3].0), play(game.clone(), VECTOR[3].0));
        // Replay memory survives the round trip.
        let restored = ColonyWorld::restore(&bytes);
        assert_eq!(play(restored, VECTOR[0].0).outcome, "already-folded");
        let fresh = Game::level();
        for bad in [
            &b"garbage"[..],
            br#"{"v":2,"worker":[1,1],"ore_cells":[[1,0],[2,1]],"pack_ore":0,"stored_ore":0,"phase":"working","outcome":"","folded":[]}"#,
            br#"{"v":1,"worker":[1,1],"ore_cells":[[1,0],[2,1]],"pack_ore":1,"stored_ore":0,"phase":"working","outcome":"","folded":[]}"#,
            br#"{"v":1,"worker":[1,0],"ore_cells":[[1,0],[2,1]],"pack_ore":0,"stored_ore":0,"phase":"working","outcome":"","folded":[]}"#,
            br#"{"v":1,"worker":[5,1],"ore_cells":[[1,0],[2,1]],"pack_ore":0,"stored_ore":0,"phase":"working","outcome":"","folded":[]}"#,
            br#"{"v":1,"worker":[0,1],"ore_cells":[],"pack_ore":0,"stored_ore":2,"phase":"working","outcome":"","folded":[]}"#,
            br#"{"v":1,"worker":[0,1],"ore_cells":[[1,0],[1,0]],"pack_ore":0,"stored_ore":0,"phase":"working","outcome":"","folded":[]}"#,
        ] {
            assert_eq!(ColonyWorld::restore(bad), fresh);
        }
    }

    #[test]
    fn view_is_presentation_only() {
        let game = play(ColonyWorld::init(0), VECTOR[0].0);
        let before = record(&game);
        let view = String::from_utf8(ColonyWorld::view(&game)).unwrap();
        assert!(view.contains("Colony"));
        assert!(view.contains("pack=1/1 stored=0/2"));
        assert_eq!(record(&game), before);
    }
}
