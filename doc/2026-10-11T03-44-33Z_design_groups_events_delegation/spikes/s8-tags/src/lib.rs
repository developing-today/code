//! S8: observed-remove tag set with dots and a per-device version vector.
//!
//! Each device signs one gapless chain of events, numbered 1, 2, 3, ... A set creates the dot
//! `(device, seq)`, and a clear takes the next number too. A clear names the dots it removes and
//! carries `clock`, its author's version vector when it was issued (the whole causal past). A
//! named dot is removed only if `clock` covers it, so a clear cannot remove what its author did
//! not see. A dot the state covers but does not hold was removed, so no tombstones are kept.

use std::collections::{BTreeMap, BTreeSet};

use cbor_core::Value;

pub type Device = [u8; 32];
pub type Subject = [u8; 32];

/// For each device, the highest seq seen. Every seq below it is seen too, because chains are gapless.
pub type Vv = BTreeMap<Device, u64>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Dot {
    pub device: Device,
    pub seq: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Entry {
    pub subject: Subject,
    pub name: String,
    pub value: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Body {
    Set(Entry),
    Clear { names: BTreeSet<Dot>, clock: Vv },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Op {
    pub dot: Dot,
    pub body: Body,
}

/// The live add dots, and what has been seen (removed dots included).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub live: BTreeMap<Dot, Entry>,
    pub vv: Vv,
}

pub fn covers(vv: &Vv, dot: &Dot) -> bool {
    dot.seq <= vv.get(&dot.device).copied().unwrap_or(0)
}

fn merge_vv(into: &mut Vv, other: &Vv) {
    for (device, seq) in other {
        let s = into.entry(*device).or_insert(0);
        *s = (*s).max(*seq);
    }
}

/// Join of two states. A dot survives if both hold it, or if one holds it and the other has not seen it.
pub fn join(a: &State, b: &State) -> State {
    let mut live = BTreeMap::new();
    for (dot, entry) in &a.live {
        if b.live.contains_key(dot) || !covers(&b.vv, dot) {
            live.insert(*dot, entry.clone());
        }
    }
    for (dot, entry) in &b.live {
        if !a.live.contains_key(dot) && !covers(&a.vv, dot) {
            live.insert(*dot, entry.clone());
        }
    }
    let mut vv = a.vv.clone();
    merge_vv(&mut vv, &b.vv);
    State { live, vv }
}

/// The state that a set of ops must produce, computed from the definitions alone.
pub fn reference<'a>(ops: impl IntoIterator<Item = &'a Op>) -> State {
    let mut vv = Vv::new();
    let mut sets = BTreeMap::new();
    let mut removed = BTreeSet::new();
    for op in ops {
        merge_vv(&mut vv, &Vv::from([(op.dot.device, op.dot.seq)]));
        match &op.body {
            Body::Set(entry) => {
                sets.insert(op.dot, entry.clone());
            }
            Body::Clear { names, clock } => {
                merge_vv(&mut vv, clock);
                removed.extend(names.iter().filter(|dot| covers(clock, dot)).copied());
            }
        }
    }
    let live = sets.into_iter().filter(|(dot, _)| !removed.contains(dot)).collect();
    State { live, vv }
}

/// One device's copy: the applied state, plus received ops that wait for their causal past.
#[derive(Clone, Debug)]
pub struct Replica {
    pub device: Device,
    pub state: State,
    /// Every applied op, by dot. The oracle check is `state == reference(applied.values())`.
    pub applied: BTreeMap<Dot, Op>,
    pending: Vec<Op>,
}

impl Replica {
    pub fn new(device: Device) -> Self {
        Self {
            device,
            state: State::default(),
            applied: BTreeMap::new(),
            pending: Vec::new(),
        }
    }

    /// Issue a set. It is applied locally at once.
    pub fn set(&mut self, entry: Entry) -> Op {
        let op = Op {
            dot: self.next_dot(),
            body: Body::Set(entry),
        };
        self.receive(op.clone());
        op
    }

    /// Issue a clear of `names`. Its clock is this replica's whole version vector.
    pub fn clear(&mut self, names: BTreeSet<Dot>) -> Op {
        let op = Op {
            dot: self.next_dot(),
            body: Body::Clear {
                names,
                clock: self.state.vv.clone(),
            },
        };
        self.receive(op.clone());
        op
    }

    /// Deliver one op in any order. Duplicates are dropped, and an op waits for its causal past.
    pub fn receive(&mut self, op: Op) {
        if !covers(&self.state.vv, &op.dot) {
            self.pending.push(op);
        }
        self.drain();
    }

    /// State-based sync: join another replica's applied state.
    pub fn merge(&mut self, other: &Replica) {
        self.state = join(&self.state, &other.state);
        for (dot, op) in &other.applied {
            self.applied.entry(*dot).or_insert_with(|| op.clone());
        }
        self.drain();
    }

    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    fn next_dot(&self) -> Dot {
        Dot {
            device: self.device,
            seq: self.state.vv.get(&self.device).copied().unwrap_or(0) + 1,
        }
    }

    /// An op is ready when it is next in its chain and, for a clear, its whole clock is applied.
    fn is_ready(&self, op: &Op) -> bool {
        let vv = &self.state.vv;
        if op.dot.seq != vv.get(&op.dot.device).copied().unwrap_or(0) + 1 {
            return false;
        }
        match &op.body {
            Body::Set(_) => true,
            Body::Clear { clock, .. } => clock
                .iter()
                .all(|(device, seq)| vv.get(device).copied().unwrap_or(0) >= *seq),
        }
    }

    fn drain(&mut self) {
        loop {
            self.pending.retain(|op| !covers(&self.state.vv, &op.dot));
            let Some(i) = self.pending.iter().position(|op| self.is_ready(op)) else {
                return;
            };
            let op = self.pending.swap_remove(i);
            self.apply(op);
        }
    }

    fn apply(&mut self, op: Op) {
        match &op.body {
            Body::Set(entry) => {
                self.state.live.insert(op.dot, entry.clone());
            }
            Body::Clear { names, clock } => {
                for dot in names.iter().filter(|dot| covers(clock, dot)) {
                    self.state.live.remove(dot);
                }
            }
        }
        self.state.vv.insert(op.dot.device, op.dot.seq);
        self.applied.insert(op.dot, op);
    }
}

/// Small deterministic generator, so that simulations replay from a seed.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self((seed ^ 0x9E37_79B9_7F4A_7C15) | 1)
    }

    /// xorshift64*.
    pub fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// An index below `n`, which must be non-zero.
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            items.swap(i, self.below(i + 1));
        }
    }
}

fn dot_value(dot: &Dot) -> Value<'static> {
    Value::array(vec![
        Value::byte_string(dot.device.to_vec()),
        Value::from(dot.seq),
    ])
}

fn vv_value(vv: &Vv) -> Value<'static> {
    Value::array(
        vv.iter()
            .map(|(device, seq)| {
                dot_value(&Dot {
                    device: *device,
                    seq: *seq,
                })
            })
            .collect::<Vec<_>>(),
    )
}

fn entry_value(entry: &Entry) -> Value<'static> {
    Value::array(vec![
        Value::byte_string(entry.subject.to_vec()),
        Value::text_string(entry.name.clone()),
        entry.value.clone().map_or_else(Value::null, Value::text_string),
    ])
}

/// Canonical CBOR of an op: `[0, dot, entry]` for a set, `[1, dot, [names, clock]]` for a clear.
pub fn encode_op(op: &Op) -> Vec<u8> {
    let (kind, body) = match &op.body {
        Body::Set(entry) => (0, entry_value(entry)),
        Body::Clear { names, clock } => (
            1,
            Value::array(vec![
                Value::array(names.iter().map(dot_value).collect::<Vec<_>>()),
                vv_value(clock),
            ]),
        ),
    };
    Value::array(vec![Value::from(kind), dot_value(&op.dot), body]).encode()
}

/// Content address of an op: BLAKE3 of its canonical encoding.
pub fn op_id(op: &Op) -> [u8; 32] {
    *blake3::hash(&encode_op(op)).as_bytes()
}

/// Canonical CBOR of a state: `[[[entry, [dots]], ...], vv]`, with entries sorted.
pub fn encode_state(state: &State) -> Vec<u8> {
    let mut by_entry: BTreeMap<&Entry, Vec<Dot>> = BTreeMap::new();
    for (dot, entry) in &state.live {
        by_entry.entry(entry).or_default().push(*dot);
    }
    let entries = by_entry
        .into_iter()
        .map(|(entry, dots)| {
            Value::array(vec![
                entry_value(entry),
                Value::array(dots.iter().map(dot_value).collect::<Vec<_>>()),
            ])
        })
        .collect::<Vec<_>>();
    Value::array(vec![Value::array(entries), vv_value(&state.vv)]).encode()
}
