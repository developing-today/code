//! Rust counter world — the playable F12.8 example, twin of
//! `examples/roc-counter/main.roc` on the Plaza ABI v1 contract:
//!
//! - `init` ignores the seed; the model is `{ count: i64 }` starting at 0.
//! - `update` increments on a trimmed `inc` event, like the Roc program
//!   (events are matched after a text trim, so `" inc "` counts too).
//! - `view` renders the boxed 20x3 screen through the SDK's screen
//!   helper; `layout` widens the box to 30 columns at width >= 30.
//! - `records` publishes `{"count":N}`; `wants` is the empty document.
//! - `snapshot` is the decimal count; `restore` parses it back and is
//!   total (invalid text restores count 0), so
//!   `restore(snapshot(m))` behaves exactly like `m`.

use plaza_sdk::screen::{Screen, Style};
use plaza_sdk::{text, World};

pub struct Counter {
    pub count: i64,
}

pub struct CounterWorld;

impl World for CounterWorld {
    type Model = Counter;

    fn init(_seed: u64) -> Counter {
        Counter { count: 0 }
    }

    fn update(model: Counter, _participant: u32, event: &[u8]) -> Counter {
        let text_event = String::from_utf8_lossy(event);
        if text::trim(&text_event) == "inc" {
            Counter {
                count: model.count + 1,
            }
        } else {
            model
        }
    }

    fn view(model: &Counter) -> Vec<u8> {
        render(model, 20, 3).into_bytes()
    }

    fn layout(model: &Counter, width: u64, _height: u64) -> Option<Vec<u8>> {
        let w = if width >= 30 { 30 } else { 20 };
        Some(render(model, w, 3).into_bytes())
    }

    fn records(model: &Counter) -> Option<Vec<u8>> {
        Some(format!("{{\"count\":{}}}", model.count).into_bytes())
    }

    fn wants(_model: &Counter) -> Option<Vec<u8>> {
        Some(b"{}".to_vec())
    }

    fn snapshot(model: &Counter) -> Option<Vec<u8>> {
        Some(model.count.to_string().into_bytes())
    }

    fn restore(snapshot: &[u8]) -> Counter {
        let text = String::from_utf8_lossy(snapshot);
        Counter {
            count: text.trim().parse::<i64>().unwrap_or(0),
        }
    }
}

fn render(model: &Counter, w: usize, h: usize) -> String {
    let mut screen = Screen::new(w, h);
    screen.boxed(0, 0, w, h, "counter");
    screen.put_text(2, 1, &format!("count={}", model.count), Style::default());
    screen.render_json()
}

plaza_sdk::plaza_guest!(CounterWorld);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_matches_the_roc_program() {
        let m = CounterWorld::init(7);
        assert_eq!(m.count, 0);
        let m = CounterWorld::update(m, 1, b"inc");
        assert_eq!(m.count, 1);
        let m = CounterWorld::update(m, 1, b"  inc \n");
        assert_eq!(m.count, 2);
        let m = CounterWorld::update(m, 1, b"dec");
        assert_eq!(m.count, 2);
        let view = String::from_utf8(CounterWorld::view(&m)).unwrap();
        assert!(view.contains("count=2"), "view: {view}");
        assert_eq!(
            CounterWorld::records(&m),
            Some(b"{\"count\":2}".to_vec())
        );
        // Snapshot/restore round-trips byte-identically, and restore is
        // total on invalid text.
        let snap = CounterWorld::snapshot(&m).unwrap();
        assert_eq!(snap, b"2");
        let restored = CounterWorld::restore(&snap);
        assert_eq!(CounterWorld::snapshot(&restored).unwrap(), snap);
        assert_eq!(CounterWorld::restore(b"junk").count, 0);
    }

    #[test]
    fn view_is_the_boxed_screen_document() {
        let m = Counter { count: 1 };
        assert_eq!(
            render(&m, 20, 3),
            "{\"kind\":\"screen\",\"version\":1,\"width\":20,\"height\":3,\"rows\":[\"+-counter----------+\",\"| count=1          |\",\"+------------------+\"],\"styles\":[[],[],[]]}"
        );
    }
}
