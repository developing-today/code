//! The clear's clock must cover its whole causal past, not only the immediate frontier. With the
//! frontier alone, an add the clearer did see is not covered, so it survives.

use std::collections::BTreeSet;

use s8_tags::{Body, Device, Dot, Entry, Op, Replica, reference};

fn device(i: usize) -> Device {
    let mut d = [0u8; 32];
    d[0] = u8::try_from(i + 1).expect("few devices");
    d
}

fn entry(subject: u8) -> Entry {
    let mut s = [0u8; 32];
    s[0] = subject;
    Entry {
        subject: s,
        name: "tag".into(),
        value: None,
    }
}

#[test]
fn frontier_only_clock_leaves_an_add_the_clearer_saw() {
    // A adds x. B sees x and then adds y, so x is in y's causal past. C sees x and y.
    let mut a = Replica::new(device(0));
    let mut b = Replica::new(device(1));
    let mut c = Replica::new(device(2));
    let x = a.set(entry(0));
    b.receive(x.clone());
    let y = b.set(entry(1));
    c.receive(x.clone());
    c.receive(y.clone());
    assert!(c.state.live.contains_key(&x.dot));

    // C clears x with a clock naming only the frontier (y, from B). C has seen x, but the clock does not cover it.
    let frontier_clock = [(device(1), 1)].into_iter().collect();
    let frontier_clear = Op {
        dot: Dot { device: device(2), seq: 1 },
        body: Body::Clear { names: BTreeSet::from([x.dot]), clock: frontier_clock },
    };
    c.receive(frontier_clear.clone());
    assert!(c.state.live.contains_key(&x.dot), "frontier clock: x survives a clear that saw it");

    // The same clear with the full clock. C's version vector covers x, so x is removed.
    let mut c_full = Replica::new(device(2));
    c_full.receive(x.clone());
    c_full.receive(y.clone());
    let full_clear = c_full.clear(BTreeSet::from([x.dot]));
    assert!(!c_full.state.live.contains_key(&x.dot));
    let history = [x.clone(), y, full_clear];
    assert!(!reference(&history).live.contains_key(&x.dot));
}
