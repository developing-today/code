//! Orbit Arcade: steer a cannon and clear one wave before the formation
//! reaches the cannon row. The host supplies every input, including the
//! logical ticks: this level asks for no timer, so ticks are manual.

use plaza_sdk::World;
use plaza_sdk::json::{self, Value, quote};
use plaza_sdk::screen::{Screen, Style};

/// Board columns and rows. Row 0 is the top; the cannon row is the last.
const WIDTH: i64 = 4;
const HEIGHT: i64 = 4;
const CANNON_ROW: i64 = HEIGHT - 1;
/// The formation moves on every tick divisible by this.
const FORMATION_MOVE_EVERY: u64 = 4;
const SCORE_PER_HIT: u64 = 10;
/// Applied order ids remembered so a replayed delivery folds once.
const FOLDED_LIMIT: usize = 32;
const MAX_ID_BYTES: usize = 16;
const SNAPSHOT_VERSION: i64 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Piece {
    id: String,
    x: i64,
    y: i64,
}

/// A steer/fire pair for the single cannon seat, stamped with its tick.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Order {
    id: String,
    for_tick: u64,
    steer: i64,
    fire: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Active,
    WaveClear,
    Overrun,
}

impl Phase {
    fn name(self) -> &'static str {
        match self {
            Phase::Active => "active",
            Phase::WaveClear => "wave-clear",
            Phase::Overrun => "overrun",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "active" => Some(Phase::Active),
            "wave-clear" => Some(Phase::WaveClear),
            "overrun" => Some(Phase::Overrun),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Game {
    tick: u64,
    cannon: i64,
    targets: Vec<Piece>,
    /// +1 east, -1 west.
    heading: i64,
    shots: Vec<Piece>,
    pending: Vec<Order>,
    score: u64,
    phase: Phase,
    outcome: String,
    /// Host deduplication memory, kept in the snapshot but not in records.
    folded: Vec<String>,
}

impl Game {
    /// The edge-descent level: the end of tick 3, cannon at (2,3), one
    /// target T at (3,0), formation heading east.
    fn level() -> Self {
        Self {
            tick: 3,
            cannon: 2,
            targets: vec![Piece {
                id: "T".into(),
                x: 3,
                y: 0,
            }],
            heading: 1,
            shots: Vec::new(),
            pending: Vec::new(),
            score: 0,
            phase: Phase::Active,
            outcome: String::new(),
            folded: Vec::new(),
        }
    }

    fn knows(&self, id: &str) -> bool {
        self.pending.iter().any(|order| order.id == id) || self.folded.iter().any(|seen| seen == id)
    }

    fn queue(&mut self, input: &Value) -> &'static str {
        let Some(order) = parse_order(input) else {
            return "refused";
        };
        if self.knows(&order.id) {
            return "already-folded";
        }
        if self.phase != Phase::Active {
            return "refused-phase";
        }
        if Some(order.for_tick) != self.tick.checked_add(1) {
            return "refused-tick";
        }
        // One seat, one order per tick: a second distinct submission is refused.
        if self
            .pending
            .iter()
            .any(|queued| queued.for_tick == order.for_tick)
        {
            return "refused-occupied";
        }
        self.pending.push(order);
        "queued"
    }

    fn redeliver(&self, input: &Value) -> &'static str {
        match input.get("ref").and_then(Value::as_str) {
            Some(id) if self.knows(id) => "already-folded",
            Some(_) => "unknown-ref",
            None => "refused",
        }
    }

    fn advance(&mut self, input: &Value) -> String {
        let Some(number) = input.get("number").and_then(Value::as_u64) else {
            return "refused".into();
        };
        if number <= self.tick {
            return "already-advanced".into();
        }
        if self.phase != Phase::Active {
            return "refused-phase".into();
        }
        if Some(number) != self.tick.checked_add(1) {
            return "refused-gap".into();
        }
        self.tick = number;
        let mut parts: Vec<String> = Vec::new();

        // 1. Steering (clamped), 2. a shot for a requested fire.
        let (due, later): (Vec<Order>, Vec<Order>) = self
            .pending
            .drain(..)
            .partition(|order| order.for_tick == number);
        self.pending = later;
        for order in due {
            let steered = (self.cannon + order.steer).clamp(0, WIDTH - 1);
            if steered != self.cannon {
                self.cannon = steered;
                parts.push("steer".into());
            }
            if order.fire {
                self.shots.push(Piece {
                    id: order.id.clone(),
                    x: self.cannon,
                    y: CANNON_ROW,
                });
                parts.push("fire".into());
            }
            self.folded.push(order.id);
        }
        let excess = self.folded.len().saturating_sub(FOLDED_LIMIT);
        self.folded.drain(..excess);

        // 3. Every shot moves north one square; shots leaving the board go.
        for shot in &mut self.shots {
            shot.y -= 1;
        }
        self.shots.retain(|shot| shot.y >= 0);

        // 4. On every fourth tick the formation moves, or descends and turns
        // when any target would leave the board.
        if number % FORMATION_MOVE_EVERY == 0 && !self.targets.is_empty() {
            let blocked = self
                .targets
                .iter()
                .any(|target| !(0..WIDTH).contains(&(target.x + self.heading)));
            if blocked {
                for target in &mut self.targets {
                    target.y += 1;
                }
                self.heading = -self.heading;
                parts.push("descend".into());
            } else {
                for target in &mut self.targets {
                    target.x += self.heading;
                }
                parts.push("shift".into());
            }
        }

        // 5. Collisions at the resulting positions, shots in stable ID order;
        // each target is defeated once and each shot stops at one target.
        self.shots.sort_by(|a, b| a.id.cmp(&b.id));
        let mut hits = Vec::new();
        let mut spent = Vec::new();
        for shot in &self.shots {
            if let Some(index) = self
                .targets
                .iter()
                .position(|target| target.x == shot.x && target.y == shot.y)
            {
                hits.push(self.targets.remove(index).id);
                spent.push(shot.id.clone());
            }
        }
        self.shots.retain(|shot| !spent.contains(&shot.id));
        if !hits.is_empty() {
            self.score = self
                .score
                .saturating_add(SCORE_PER_HIT.saturating_mul(hits.len() as u64));
            parts.push(format!("hit-{}", hits.join("-")));
        }

        // 6. The wave ends cleared, or overrun by a target on the cannon row.
        if self.targets.is_empty() {
            self.phase = Phase::WaveClear;
            parts.push("clear".into());
        } else if self.targets.iter().any(|target| target.y >= CANNON_ROW) {
            self.phase = Phase::Overrun;
            parts.push("overrun".into());
        }
        sentence(&parts)
    }

    fn new_wave(&mut self) -> &'static str {
        if self.phase == Phase::Active {
            return "refused-phase";
        }
        let level = Game::level();
        self.cannon = level.cannon;
        self.targets = level.targets;
        self.heading = level.heading;
        self.shots.clear();
        self.pending.clear();
        self.phase = Phase::Active;
        "new-wave"
    }

    fn apply(&mut self, input: &Value) -> String {
        match input.get("kind").and_then(Value::as_str) {
            Some("QueueControls") => self.queue(input).into(),
            Some("Redeliver") => self.redeliver(input).into(),
            Some("Tick") => self.advance(input),
            // Redraw requests are presentation only; they never tick.
            Some("RenderFrames") => "presentation-only".into(),
            Some("NewWave") => self.new_wave().into(),
            _ => "refused".into(),
        }
    }

    /// The domain projection in the acceptance vector's key order, then the
    /// last outcome; `folded` is added only to the snapshot.
    fn document(&self, snapshot: bool) -> String {
        let pieces = |items: &[Piece]| {
            items
                .iter()
                .map(|p| format!(r#"{{"id":{},"at":[{},{}]}}"#, quote(&p.id), p.x, p.y))
                .collect::<Vec<_>>()
                .join(",")
        };
        let pending = self
            .pending
            .iter()
            .map(|o| {
                format!(
                    r#"{{"id":{},"for_tick":{},"steer":{},"fire":{}}}"#,
                    quote(&o.id),
                    o.for_tick,
                    o.steer,
                    o.fire
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let mut doc = String::from("{");
        if snapshot {
            doc.push_str(&format!(r#""v":{SNAPSHOT_VERSION},"#));
        }
        doc.push_str(&format!(
            r#""tick":{},"cannon":[{},{}],"targets":[{}],"formation_direction":"{}","shots":[{}],"pending":[{}],"score":{},"phase":"{}","outcome":{}"#,
            self.tick,
            self.cannon,
            CANNON_ROW,
            pieces(&self.targets),
            if self.heading > 0 { "E" } else { "W" },
            pieces(&self.shots),
            pending,
            self.score,
            self.phase.name(),
            quote(&self.outcome)
        ));
        if snapshot {
            let folded = self.folded.iter().map(|id| quote(id)).collect::<Vec<_>>();
            doc.push_str(&format!(r#","folded":[{}]"#, folded.join(",")));
        }
        doc.push('}');
        doc
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        let doc = json::decode(std::str::from_utf8(bytes).ok()?).ok()?;
        if doc.get("v")?.as_i64()? != SNAPSHOT_VERSION {
            return None;
        }
        let tick = doc.get("tick")?.as_u64()?;
        let [cannon, row] = cell(doc.get("cannon")?)?;
        if row != CANNON_ROW {
            return None;
        }
        let heading = match doc.get("formation_direction")?.as_str()? {
            "E" => 1,
            "W" => -1,
            _ => return None,
        };
        let pending = doc
            .get("pending")?
            .as_array()?
            .iter()
            .map(parse_order)
            .collect::<Option<Vec<_>>>()?;
        if pending.len() > 1
            || pending
                .iter()
                .any(|order| Some(order.for_tick) != tick.checked_add(1))
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
            tick,
            cannon,
            targets: pieces(doc.get("targets")?)?,
            heading,
            shots: pieces(doc.get("shots")?)?,
            pending,
            score: doc.get("score")?.as_u64()?,
            phase: Phase::parse(doc.get("phase")?.as_str()?)?,
            outcome: doc.get("outcome")?.as_str()?.chars().take(64).collect(),
            folded,
        })
    }

    fn glyph(&self, x: i64, y: i64) -> char {
        if y == CANNON_ROW && x == self.cannon {
            'A'
        } else if self.targets.iter().any(|t| t.x == x && t.y == y) {
            'T'
        } else if self.shots.iter().any(|s| s.x == x && s.y == y) {
            '|'
        } else {
            '.'
        }
    }
}

/// Join outcome parts as "a-b-and-c"; a tick with nothing to report is
/// simply "advanced".
fn sentence(parts: &[String]) -> String {
    match parts {
        [] => "advanced".into(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{}-and-{last}", rest.join("-")),
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID_BYTES
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn parse_order(input: &Value) -> Option<Order> {
    let id = input.get("id")?.as_str().filter(|id| valid_id(id))?;
    let steer = input
        .get("steer")?
        .as_i64()
        .filter(|s| (-1..=1).contains(s))?;
    Some(Order {
        id: id.into(),
        for_tick: input.get("for_tick")?.as_u64()?,
        steer,
        fire: input.get("fire")?.as_bool()?,
    })
}

fn cell(value: &Value) -> Option<[i64; 2]> {
    match value.as_array()? {
        [x, y] => {
            let (x, y) = (x.as_i64()?, y.as_i64()?);
            ((0..WIDTH).contains(&x) && (0..HEIGHT).contains(&y)).then_some([x, y])
        }
        _ => None,
    }
}

fn pieces(value: &Value) -> Option<Vec<Piece>> {
    let items = value.as_array()?;
    if items.len() > (WIDTH * HEIGHT) as usize {
        return None;
    }
    items
        .iter()
        .map(|item| {
            let id = item.get("id")?.as_str().filter(|id| valid_id(id))?;
            let [x, y] = cell(item.get("at")?)?;
            Some(Piece {
                id: id.into(),
                x,
                y,
            })
        })
        .collect()
}

/// The Plaza world entry points shared by the terminal, web and other hosts.
pub struct Orbit;

impl World for Orbit {
    type Model = Game;

    fn init(_seed: u64) -> Game {
        Game::level()
    }

    fn update(mut model: Game, participant: u32, event: &[u8]) -> Game {
        // Participant 0 is the host (capability results, subscriptions);
        // the wave asks for neither, so host events never drive it.
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
        let mut screen = Screen::new(32, HEIGHT as usize + 4);
        let style = Style::default();
        screen.put_text(0, 0, "Orbit Arcade", style);
        for y in 0..HEIGHT {
            let row: Vec<String> = (0..WIDTH).map(|x| model.glyph(x, y).to_string()).collect();
            screen.put_text(0, y as usize + 1, &row.join(" "), style);
        }
        let heading = if model.heading > 0 { "E" } else { "W" };
        let status = format!(
            "tick={} score={} heading={heading}",
            model.tick, model.score
        );
        screen.put_text(0, HEIGHT as usize + 1, &status, style);
        let order = match model.pending.first() {
            Some(o) => format!(
                "next t{} steer={:+} fire={}",
                o.for_tick,
                o.steer,
                if o.fire { "yes" } else { "no" }
            ),
            None => "next: no order".into(),
        };
        screen.put_text(0, HEIGHT as usize + 2, &order, style);
        let last = if model.outcome.is_empty() {
            "-"
        } else {
            model.outcome.as_str()
        };
        screen.put_text(
            0,
            HEIGHT as usize + 3,
            &format!("{} {last}", model.phase.name()),
            style,
        );
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

plaza_sdk::plaza_guest!(Orbit);

#[cfg(test)]
mod tests {
    use super::*;

    fn step(game: Game, input: &str) -> Game {
        Orbit::update(game, 1, input.as_bytes())
    }

    fn record(game: &Game) -> Value {
        json::decode(&String::from_utf8(Orbit::records(game).unwrap()).unwrap()).unwrap()
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

    // Oracle: orbit-arcade/edge-descent in
    // pkgs/id/book/research/evidence/app-vectors.json (revision b9d8177).
    // Inputs and after-states are literal copies; nothing is derived from
    // the guest.
    const QUEUED: &str = r#"{"tick":3,"cannon":[2,3],"targets":[{"id":"T","at":[3,0]}],"formation_direction":"E","shots":[],"pending":[{"id":"q4","for_tick":4,"steer":1,"fire":true}],"score":0,"phase":"active"}"#;
    const AFTER_T4: &str = r#"{"tick":4,"cannon":[3,3],"targets":[{"id":"T","at":[3,1]}],"formation_direction":"W","shots":[{"id":"q4","at":[3,2]}],"pending":[],"score":0,"phase":"active"}"#;
    const AFTER_T5: &str = r#"{"tick":5,"cannon":[3,3],"targets":[],"formation_direction":"W","shots":[],"pending":[],"score":10,"phase":"wave-clear"}"#;

    #[test]
    fn test_edge_descent_vector() {
        let mut game = Orbit::init(0);
        assert_eq!(
            record(&game),
            expected(
                r#"{"tick":3,"cannon":[2,3],"targets":[{"id":"T","at":[3,0]}],"formation_direction":"E","shots":[],"pending":[],"score":0,"phase":"active"}"#,
                ""
            )
        );
        let steps = [
            (
                r#"{"kind":"QueueControls","id":"q4","for_tick":4,"steer":1,"fire":true}"#,
                "queued",
                QUEUED,
            ),
            (
                r#"{"kind":"Redeliver","ref":"q4"}"#,
                "already-folded",
                QUEUED,
            ),
            (
                r#"{"kind":"Tick","number":4,"event":"t4"}"#,
                "steer-fire-and-descend",
                AFTER_T4,
            ),
            (
                r#"{"kind":"RenderFrames","count":120}"#,
                "presentation-only",
                AFTER_T4,
            ),
            (
                r#"{"kind":"Tick","number":5,"event":"t5"}"#,
                "hit-T-and-clear",
                AFTER_T5,
            ),
        ];
        for (input, outcome, after) in steps {
            game = step(game, input);
            assert_eq!(record(&game), expected(after, outcome), "{input}");
        }
    }

    #[test]
    fn test_one_order_per_seat_and_tick() {
        let order = r#"{"kind":"QueueControls","id":"q4","for_tick":4,"steer":1,"fire":true}"#;
        let mut game = step(Orbit::init(0), order);
        // The same event delivered again folds into the one order.
        game = step(game, order);
        assert_eq!(game.outcome, "already-folded");
        // A second distinct submission for the occupied tick is refused.
        game = step(
            game,
            r#"{"kind":"QueueControls","id":"r4","for_tick":4,"steer":-1,"fire":false}"#,
        );
        assert_eq!(game.outcome, "refused-occupied");
        assert_eq!(record(&game), expected(QUEUED, "refused-occupied"));
        // Orders must name the next logical tick and a steer of at most one lane.
        for (input, outcome) in [
            (
                r#"{"kind":"QueueControls","id":"s5","for_tick":5,"steer":0,"fire":true}"#,
                "refused-tick",
            ),
            (
                r#"{"kind":"QueueControls","id":"s4","for_tick":4,"steer":2,"fire":true}"#,
                "refused",
            ),
            (r#"{"kind":"Redeliver","ref":"zz"}"#, "unknown-ref"),
            (r#"{"kind":"Launch"}"#, "refused"),
            ("not json", "refused"),
        ] {
            game = step(game, input);
            assert_eq!(record(&game), expected(QUEUED, outcome), "{input}");
        }
        // After the tick folds the order, a late redelivery still folds once.
        game = step(game, r#"{"kind":"Tick","number":4,"event":"t4"}"#);
        game = step(game, order);
        assert_eq!(record(&game), expected(AFTER_T4, "already-folded"));
    }

    #[test]
    fn test_ticks_are_the_only_clock() {
        let mut game = step(Orbit::init(0), r#"{"kind":"Tick","number":4,"event":"t4"}"#);
        // No order: the formation descends alone and nothing else moves.
        assert_eq!(game.outcome, "descend");
        // A replayed tick and a skipped tick change nothing.
        let before = record(&game);
        game = step(game, r#"{"kind":"Tick","number":4,"event":"t4"}"#);
        assert_eq!(game.outcome, "already-advanced");
        game = step(game, r#"{"kind":"Tick","number":6,"event":"t6"}"#);
        assert_eq!(game.outcome, "refused-gap");
        game = step(game, r#"{"kind":"RenderFrames","count":120}"#);
        let mut after = before.clone();
        if let Value::Object(pairs) = &mut after {
            pairs.last_mut().unwrap().1 = Value::Str("presentation-only".into());
        }
        assert_eq!(record(&game), after);
        // Host deliveries never drive the wave, not even its outcome.
        let host = Orbit::update(game.clone(), 0, br#"{"kind":"Tick","number":5}"#);
        assert_eq!(host, game);
    }

    #[test]
    fn test_steering_clamps_and_overrun_ends_the_wave() {
        let mut game = Orbit::init(0);
        // Hold right without firing. The target T walks the formation path:
        // descend at 4, west at 8, 12 and 16, descend at 20, east at 24,
        // 28 and 32, and the descent at 36 reaches the cannon row.
        for tick in 4..=36 {
            game = step(
                game,
                &format!(
                    r#"{{"kind":"QueueControls","id":"o{tick}","for_tick":{tick},"steer":1,"fire":false}}"#
                ),
            );
            game = step(game, &format!(r#"{{"kind":"Tick","number":{tick}}}"#));
            assert_eq!(game.cannon, 3, "tick {tick} clamps at the east edge");
            let expected = match tick {
                4 => "steer-and-descend",
                20 => "descend",
                36 => "descend-and-overrun",
                t if t % 4 == 0 => "shift",
                _ => "advanced",
            };
            assert_eq!(game.outcome, expected, "tick {tick}");
        }
        assert_eq!(game.phase, Phase::Overrun);
        assert_eq!(
            game.targets,
            vec![Piece {
                id: "T".into(),
                x: 3,
                y: 3
            }]
        );
        // The folded-id memory stays bounded.
        assert_eq!(game.folded.len(), FOLDED_LIMIT);
        game = step(game, r#"{"kind":"Tick","number":37}"#);
        assert_eq!(game.outcome, "refused-phase");
        game = step(game, r#"{"kind":"NewWave"}"#);
        assert_eq!(game.outcome, "new-wave");
        assert_eq!((game.tick, game.cannon, game.phase), (36, 2, Phase::Active));
        assert_eq!(game.targets, Game::level().targets);
        game = step(game, r#"{"kind":"NewWave"}"#);
        assert_eq!(game.outcome, "refused-phase");
    }

    #[test]
    fn test_snapshot_round_trip_and_total_restore() {
        let mut game = step(
            Orbit::init(0),
            r#"{"kind":"QueueControls","id":"q4","for_tick":4,"steer":1,"fire":true}"#,
        );
        game = step(game, r#"{"kind":"Tick","number":4,"event":"t4"}"#);
        let snapshot = Orbit::snapshot(&game).unwrap();
        let restored = Orbit::restore(&snapshot);
        assert_eq!(restored, game);
        assert_eq!(Orbit::snapshot(&restored).unwrap(), snapshot);
        let level = Game::level();
        for bad in [
            &b""[..],
            &[0xff],
            br#"{"v":2}"#,
            br#"{"v":1,"tick":3,"cannon":[9,3],"targets":[],"formation_direction":"E","shots":[],"pending":[],"score":0,"phase":"active","outcome":"","folded":[]}"#,
            br#"{"v":1,"tick":3,"cannon":[2,3],"targets":[],"formation_direction":"N","shots":[],"pending":[],"score":0,"phase":"active","outcome":"","folded":[]}"#,
            br#"{"v":1,"tick":3,"cannon":[2,3],"targets":[],"formation_direction":"E","shots":[],"pending":[{"id":"x","for_tick":9,"steer":0,"fire":true}],"score":0,"phase":"active","outcome":"","folded":[]}"#,
        ] {
            assert_eq!(Orbit::restore(bad), level);
        }
    }

    #[test]
    fn test_view_is_presentation() {
        let game = step(
            Orbit::init(0),
            r#"{"kind":"QueueControls","id":"q4","for_tick":4,"steer":1,"fire":true}"#,
        );
        let game = step(game, r#"{"kind":"Tick","number":4,"event":"t4"}"#);
        let view = String::from_utf8(Orbit::view(&game)).unwrap();
        for row in [
            "Orbit Arcade",
            ". . . .",
            ". . . T",
            ". . . |",
            ". . . A",
            "tick=4 score=0 heading=W",
            "next: no order",
            "active steer-fire-and-descend",
        ] {
            assert!(view.contains(row), "{row:?} missing from {view}");
        }
        let text = String::from_utf8(Orbit::records(&game).unwrap()).unwrap();
        assert!(!text.contains("heading"), "{text}");
        assert!(!text.contains("folded"), "{text}");
    }
}
