//! #8307 acceptance, through the public include path: the
//! seeded-corpus determinism invariant of the shared workload
//! generator (#5891) — same seed, same workload sequence — plus the
//! scheduler's run-level determinism over a recording surface.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::time::Duration;

use common::loadgen::{
    Actor, Corpus, MixTable, OpError, OpKind, Profile, RunSpec, Surface, Target, Workload,
};

#[test]
fn same_seed_produces_the_same_corpus_sequence() {
    let corpus_a = Corpus::new(0x5891);
    let corpus_b = Corpus::new(0x5891);
    let sequence_a: Vec<Vec<u8>> = (0..64)
        .map(|index| corpus_a.payload(2, index, 48))
        .collect();
    let sequence_b: Vec<Vec<u8>> = (0..64)
        .map(|index| corpus_b.payload(2, index, 48))
        .collect();
    assert_eq!(sequence_a, sequence_b);
    // And the sequence is actually seed-driven, not constant.
    let other = Corpus::new(0x5892);
    assert_ne!(sequence_a[0], other.payload(2, 0, 48));
}

#[derive(Default)]
struct RecordingTarget {
    stream: Vec<(u32, OpKind, Vec<u8>)>,
}

struct Session {
    actor: u32,
}

impl Target<Session> for RecordingTarget {
    fn connect(&mut self, _seed: u64, actor: &Actor) -> Result<Session, OpError> {
        Ok(Session { actor: actor.index })
    }

    fn execute(
        &mut self,
        session: &mut Session,
        op: OpKind,
        payload: &[u8],
    ) -> Result<Duration, OpError> {
        self.stream.push((session.actor, op, payload.to_vec()));
        Ok(Duration::from_micros(5))
    }

    fn disconnect(&mut self, _session: Session) -> Result<(), OpError> {
        Ok(())
    }
}

fn spec(seed: u64) -> RunSpec {
    RunSpec {
        seed,
        workload: Workload {
            name: "determinism".to_owned(),
            mix: MixTable::new(&[
                (OpKind::JournalAppend, 500),
                (OpKind::CapabilityCheck, 250),
                (OpKind::WorldSync, 250),
            ])
            .unwrap(),
            ops_per_actor: 40,
            warm_up_ops: 0,
            payload_bytes: 24,
        },
        profile: Profile::honest("bench"),
        scale: "S0".to_owned(),
        surface: Surface::InProcess,
        actors: 3,
    }
}

#[test]
fn same_seed_produces_the_same_workload_sequence() {
    let mut a = RecordingTarget::default();
    let mut b = RecordingTarget::default();
    let summary_a = common::loadgen::run(&mut a, &spec(1234)).unwrap();
    let summary_b = common::loadgen::run(&mut b, &spec(1234)).unwrap();
    assert_eq!(a.stream, b.stream);
    assert_eq!(summary_a, summary_b);
    assert_eq!(summary_a.ops, 3 * 40);
}
