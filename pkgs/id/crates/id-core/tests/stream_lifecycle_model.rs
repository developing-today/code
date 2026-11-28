//! Stream descriptor, chunk chain and lifecycle state machine
//! (slice A9123-stream-records, issue #10246; design part A §#9123,
//! A-9123.1, statements S4.74–S4.82).
//!
//! Oracle: **model** — a finite lifecycle model with chain
//! sequence/hash invariants and exactly one terminal record. The
//! proptest at the end generates the operation list (append, replay,
//! conflict, gap, pause, resume, finalize, complete, abort, expire)
//! against an independent reference model written over plain values;
//! every resulting state and error class must match. Targeted tests
//! pin the cases the generator is unlikely to hit: tampered
//! signatures, sequencer authority, scope binding, canonical
//! round-trips and the descriptor's array address.
//!
//! The crate-root export of `stream_record` belongs to slice
//! 9110-02, so this test includes it, and the modules it builds on,
//! by path.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../src/canonical_cbor.rs"]
#[allow(dead_code)]
mod canonical_cbor;

#[path = "../src/id_shape.rs"]
#[allow(dead_code)]
mod id_shape;

#[path = "../src/hash_profile.rs"]
#[allow(dead_code)]
mod hash_profile;

#[path = "../src/signed_envelope.rs"]
#[allow(dead_code)]
mod signed_envelope;

#[path = "../src/array_address.rs"]
#[allow(dead_code)]
mod array_address;

#[path = "../src/stream_record.rs"]
mod stream_record;

use array_address::ArrayAddress;
use ed25519_dalek::SigningKey;
use id_shape::{ContentId, HashProfile};
use proptest::prelude::*;
use signed_envelope::PayloadId;
use stream_record::{
    AppendOutcome, ChunkRecord, ChunkRecordId, NamespaceId, Resolution, StreamDescriptor,
    StreamLifecycle, StreamState, TerminalKind, TerminalRecord,
};

const CREATOR: u8 = 7;
const SEQUENCER: u8 = 8;
const STRANGER: u8 = 9;
const SCOPE: &[u8] = b"tenant-a";
const MEDIA: &str = "text/plain";

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn public(seed: u8) -> [u8; 32] {
    key(seed).verifying_key().to_bytes()
}

fn descriptor() -> StreamDescriptor {
    StreamDescriptor::open(
        NamespaceId::new([3u8; 32]),
        MEDIA,
        public(CREATOR),
        1_700_000_000,
    )
    .unwrap()
}

fn machine() -> StreamLifecycle {
    StreamLifecycle::new(descriptor(), SCOPE.to_vec(), None).unwrap()
}

fn chunk(stream: &StreamLifecycle, payload: &[u8], signer: u8) -> ChunkRecord {
    let payload_id = PayloadId::of(HashProfile::RawBlake3_256, payload).unwrap();
    ChunkRecord::unsigned(
        stream.descriptor().stream_id,
        SCOPE.to_vec(),
        stream.descriptor().current_seq,
        stream.last_record_id(),
        &payload_id,
        MEDIA,
    )
    .unwrap()
    .sign(&key(signer))
    .unwrap()
}

fn final_ref() -> ContentId {
    ContentId::of_bytes(HashProfile::RawBlake3_256, b"final-manifest").unwrap()
}

fn terminal(stream: &StreamLifecycle, kind: TerminalKind, signer: u8) -> TerminalRecord {
    let final_ref = (kind == TerminalKind::Completed).then(final_ref);
    TerminalRecord::unsigned(kind, stream.descriptor().stream_id, final_ref)
        .unwrap()
        .sign(&key(signer))
        .unwrap()
}

fn class<T>(result: &Result<T, stream_record::StreamError>) -> String {
    match result {
        Ok(_) => "ok".to_owned(),
        Err(error) => error.class().to_owned(),
    }
}

fn append_class(result: &Result<AppendOutcome, stream_record::StreamError>) -> String {
    match result {
        Ok(AppendOutcome::Appended) => "appended".to_owned(),
        Ok(AppendOutcome::Duplicate) => "duplicate".to_owned(),
        Err(error) => error.class().to_owned(),
    }
}

#[test]
fn happy_path_lifecycle_with_exactly_one_terminal_record() {
    let mut stream = machine();
    let mut prev = None;
    for i in 0..3u8 {
        let record = chunk(&stream, &[b'a' + i], CREATOR);
        assert_eq!(record.sequence, u64::from(i));
        assert_eq!(record.prev_record_id, prev);
        // Chain hash invariant: the ID recomputes from the encoding,
        // including after a canonical round-trip.
        let decoded = ChunkRecord::decode(&record.canonical_bytes().unwrap()).unwrap();
        assert_eq!(decoded, record);
        assert_eq!(decoded.record_id().unwrap(), record.record_id().unwrap());
        assert_eq!(
            stream.append_chunk(&record).unwrap(),
            AppendOutcome::Appended
        );
        prev = Some(record.record_id().unwrap());
    }
    assert_eq!(stream.descriptor().current_seq, 3);
    assert_eq!(stream.last_record_id(), prev);

    stream.pause().unwrap();
    let blocked = chunk(&stream, b"while-paused", CREATOR);
    assert_eq!(append_class(&stream.append_chunk(&blocked)), "not_open");
    stream.resume().unwrap();

    // Completion is a CAS: it cannot fire from `open`.
    let completion = terminal(&stream, TerminalKind::Completed, CREATOR);
    assert_eq!(class(&stream.complete(completion)), "bad_transition");

    stream.begin_finalize().unwrap();
    let completion = terminal(&stream, TerminalKind::Completed, CREATOR);
    stream.complete(completion).unwrap();
    assert_eq!(stream.state(), StreamState::Completed);
    assert_eq!(
        stream.resolve(),
        Resolution::Completed {
            final_id: final_ref()
        }
    );
    assert!(stream.terminal_record().is_some());

    // Exactly one terminal record: everything mutating now fails.
    let late = chunk(&stream, b"late", CREATOR);
    assert_eq!(append_class(&stream.append_chunk(&late)), "terminal");
    let again = terminal(&stream, TerminalKind::Aborted, CREATOR);
    assert_eq!(class(&stream.abort(again)), "terminal");
    assert_eq!(class(&stream.pause()), "terminal");
}

#[test]
fn duplicate_is_idempotent_and_differing_record_is_conflict() {
    let mut stream = machine();
    let first = chunk(&stream, b"one", CREATOR);
    assert_eq!(
        stream.append_chunk(&first).unwrap(),
        AppendOutcome::Appended
    );
    assert_eq!(
        stream.append_chunk(&first).unwrap(),
        AppendOutcome::Duplicate
    );
    assert_eq!(stream.descriptor().current_seq, 1);

    // Same (stream, sequence) but a different payload: equivocation.
    let payload_id = PayloadId::of(HashProfile::RawBlake3_256, b"uno").unwrap();
    let conflict = ChunkRecord::unsigned(
        stream.descriptor().stream_id,
        SCOPE.to_vec(),
        0,
        None,
        &payload_id,
        MEDIA,
    )
    .unwrap()
    .sign(&key(CREATOR))
    .unwrap();
    assert_eq!(
        append_class(&stream.append_chunk(&conflict)),
        "sequence_conflict"
    );
    assert_eq!(stream.descriptor().current_seq, 1);
}

#[test]
fn chain_and_authorization_rejections_carry_their_classes() {
    let mut stream = machine();

    // Wrong signer: neither creator nor a configured sequencer.
    let stranger = chunk(&stream, b"x", STRANGER);
    assert_eq!(
        append_class(&stream.append_chunk(&stranger)),
        "unauthorized_signer"
    );

    // Tampered signature from the right signer.
    let mut tampered = chunk(&stream, b"x", CREATOR);
    tampered.signature[0] ^= 0xff;
    assert_eq!(
        append_class(&stream.append_chunk(&tampered)),
        "bad_signature"
    );

    // Sequence gap: sequence 1 while the chain is empty.
    let payload_id = PayloadId::of(HashProfile::RawBlake3_256, b"x").unwrap();
    let gap = ChunkRecord::unsigned(
        stream.descriptor().stream_id,
        SCOPE.to_vec(),
        1,
        None,
        &payload_id,
        MEDIA,
    )
    .unwrap()
    .sign(&key(CREATOR))
    .unwrap();
    assert_eq!(append_class(&stream.append_chunk(&gap)), "sequence_gap");

    // Chain break: right sequence, wrong previous-record link.
    let payload_id = PayloadId::of(HashProfile::RawBlake3_256, b"x").unwrap();
    let broken = ChunkRecord::unsigned(
        stream.descriptor().stream_id,
        SCOPE.to_vec(),
        0,
        Some(ChunkRecordId::from_content_id(
            ContentId::of_bytes(HashProfile::RawBlake3_256, b"not-the-head").unwrap(),
        )),
        &payload_id,
        MEDIA,
    )
    .unwrap()
    .sign(&key(CREATOR))
    .unwrap();
    assert_eq!(append_class(&stream.append_chunk(&broken)), "chain_break");

    // Scope binding: a chunk under another tenant scope is rejected.
    let payload_id = PayloadId::of(HashProfile::RawBlake3_256, b"x").unwrap();
    let foreign = ChunkRecord::unsigned(
        stream.descriptor().stream_id,
        b"tenant-b".to_vec(),
        0,
        None,
        &payload_id,
        MEDIA,
    )
    .unwrap()
    .sign(&key(CREATOR))
    .unwrap();
    assert_eq!(
        append_class(&stream.append_chunk(&foreign)),
        "scope_mismatch"
    );

    // None of the rejections moved the chain.
    assert_eq!(stream.descriptor().current_seq, 0);
    assert!(stream.chain().is_empty());
}

#[test]
fn sequencer_authority_admits_multiwriter_chunks() {
    let mut stream =
        StreamLifecycle::new(descriptor(), SCOPE.to_vec(), Some(public(SEQUENCER))).unwrap();
    let by_sequencer = chunk(&stream, b"sequenced", SEQUENCER);
    assert_eq!(
        stream.append_chunk(&by_sequencer).unwrap(),
        AppendOutcome::Appended
    );
    let by_creator = chunk(&stream, b"creator-too", CREATOR);
    assert_eq!(
        stream.append_chunk(&by_creator).unwrap(),
        AppendOutcome::Appended
    );
    let by_stranger = chunk(&stream, b"nope", STRANGER);
    assert_eq!(
        append_class(&stream.append_chunk(&by_stranger)),
        "unauthorized_signer"
    );
}

#[test]
fn abort_and_expiry_claim_no_final_content() {
    let mut aborted = machine();
    let record = chunk(&aborted, b"one", CREATOR);
    aborted.append_chunk(&record).unwrap();
    let abort = terminal(&aborted, TerminalKind::Aborted, CREATOR);
    aborted.abort(abort).unwrap();
    assert_eq!(aborted.state(), StreamState::Aborted);
    assert_eq!(aborted.descriptor().final_ref, None);
    assert_eq!(
        aborted.resolve(),
        Resolution::Live {
            state: StreamState::Aborted
        }
    );

    // An abort record that claims a final reference is not even
    // constructible as a valid terminal record for the machine.
    let claiming = TerminalRecord {
        kind: TerminalKind::Aborted,
        stream_id: aborted.descriptor().stream_id,
        final_ref: Some(final_ref()),
        signer: [0u8; 32],
        signature: [0u8; 64],
    };
    assert_eq!(
        class(&claiming.sign(&key(CREATOR)).map(|_| ())),
        "final_ref_mismatch"
    );

    let mut expired = machine();
    expired.pause().unwrap();
    let expiry = terminal(&expired, TerminalKind::Expired, CREATOR);
    expired.expire(expiry).unwrap();
    assert_eq!(expired.state(), StreamState::Expired);
    assert_eq!(expired.descriptor().final_ref, None);
}

#[test]
fn descriptor_round_trip_and_array_address() {
    let desc = descriptor();
    let decoded = StreamDescriptor::decode(&desc.canonical_bytes().unwrap()).unwrap();
    assert_eq!(decoded, desc);

    let address = desc.address();
    let slots = address.slots();
    assert_eq!(slots.len(), 2);
    assert_eq!(slots[0].as_deref(), Some(b"id-stream".as_slice()));
    assert_eq!(
        slots[1].as_deref(),
        Some(desc.stream_id.content_id().digest().as_slice())
    );
    // The A9118 codec round-trips the address losslessly.
    let bytes = address.encode();
    assert_eq!(ArrayAddress::decode(&bytes).unwrap(), address);
}

/// The reference model: the lifecycle restated over plain values.
/// It decides each operation from the specification alone; the
/// machine must agree on the outcome class and the resulting state.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Model {
    state: StreamState,
    seq: u64,
    chain: Vec<ChunkRecordId>,
    terminal: Option<TerminalKind>,
    generation: u64,
}

impl Model {
    fn new() -> Self {
        Self {
            state: StreamState::Open,
            seq: 0,
            chain: Vec::new(),
            terminal: None,
            generation: 0,
        }
    }

    fn append(&mut self, record: &ChunkRecord) -> String {
        let record_id = record.record_id().unwrap();
        if record.sequence < self.seq {
            return if self.chain.get(record.sequence as usize) == Some(&record_id) {
                "duplicate".to_owned()
            } else {
                "sequence_conflict".to_owned()
            };
        }
        if self.terminal.is_some() {
            return "terminal".to_owned();
        }
        if self.state != StreamState::Open {
            return "not_open".to_owned();
        }
        if record.tenant_scope != SCOPE {
            return "scope_mismatch".to_owned();
        }
        if record.signer != public(CREATOR) {
            return "unauthorized_signer".to_owned();
        }
        if record.verify_signature().is_err() {
            return "bad_signature".to_owned();
        }
        if record.sequence != self.seq {
            return "sequence_gap".to_owned();
        }
        if record.prev_record_id != self.chain.last().copied() {
            return "chain_break".to_owned();
        }
        self.chain.push(record_id);
        self.seq += 1;
        "appended".to_owned()
    }

    fn transition(&mut self, from: StreamState, to: StreamState) -> String {
        if self.terminal.is_some() {
            return "terminal".to_owned();
        }
        if self.state != from {
            return "bad_transition".to_owned();
        }
        self.state = to;
        self.generation += 1;
        "ok".to_owned()
    }

    fn terminate(&mut self, kind: TerminalKind, allowed: &[StreamState]) -> String {
        if self.terminal.is_some() {
            return "terminal".to_owned();
        }
        if !allowed.contains(&self.state) {
            return "bad_transition".to_owned();
        }
        self.state = kind.lands_in();
        self.terminal = Some(kind);
        self.generation += 1;
        "ok".to_owned()
    }
}

#[derive(Debug, Clone)]
enum Op {
    AppendNext { payload: u8, signer: u8 },
    Replay { index: usize },
    Conflict { payload: u8 },
    Pause,
    Resume,
    Finalize,
    Complete,
    Abort,
    Expire,
}

fn op_strategy() -> impl Strategy<Value = Op> {
    prop_oneof![
        (any::<u8>(), any::<bool>()).prop_map(|(payload, creator)| Op::AppendNext {
            payload,
            signer: if creator { CREATOR } else { STRANGER },
        }),
        any::<usize>().prop_map(|index| Op::Replay { index }),
        any::<u8>().prop_map(|payload| Op::Conflict { payload }),
        Just(Op::Pause),
        Just(Op::Resume),
        Just(Op::Finalize),
        Just(Op::Complete),
        Just(Op::Abort),
        Just(Op::Expire),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn generated_operations_match_the_reference_model(ops in prop::collection::vec(op_strategy(), 1..24)) {
        let mut stream = machine();
        let mut model = Model::new();
        let mut pool: Vec<ChunkRecord> = Vec::new();

        for op in ops {
            match op {
                Op::AppendNext { payload, signer } => {
                    let record = chunk(&stream, &[payload], signer);
                    let expected = model.append(&record);
                    let actual = append_class(&stream.append_chunk(&record));
                    prop_assert_eq!(&actual, &expected);
                    pool.push(record);
                }
                Op::Replay { index } => {
                    prop_assume!(!pool.is_empty());
                    let record = pool[index % pool.len()].clone();
                    let expected = model.append(&record);
                    let actual = append_class(&stream.append_chunk(&record));
                    prop_assert_eq!(&actual, &expected);
                }
                Op::Conflict { payload } => {
                    prop_assume!(model.seq > 0);
                    let sequence = u64::try_from(payload as usize % usize::try_from(model.seq).unwrap()).unwrap();
                    let prev = if sequence == 0 {
                        None
                    } else {
                        Some(model.chain[(sequence - 1) as usize])
                    };
                    let payload_id = PayloadId::of(
                        HashProfile::RawBlake3_256,
                        &[payload, 0xee],
                    ).unwrap();
                    let record = ChunkRecord::unsigned(
                        stream.descriptor().stream_id,
                        SCOPE.to_vec(),
                        sequence,
                        prev,
                        &payload_id,
                        MEDIA,
                    ).unwrap().sign(&key(CREATOR)).unwrap();
                    let expected = model.append(&record);
                    let actual = append_class(&stream.append_chunk(&record));
                    prop_assert_eq!(&actual, &expected);
                    pool.push(record);
                }
                Op::Pause => {
                    let expected = model.transition(StreamState::Open, StreamState::Paused);
                    prop_assert_eq!(class(&stream.pause()), expected);
                }
                Op::Resume => {
                    let expected = model.transition(StreamState::Paused, StreamState::Open);
                    prop_assert_eq!(class(&stream.resume()), expected);
                }
                Op::Finalize => {
                    let expected = model.transition(StreamState::Open, StreamState::Finalizing);
                    prop_assert_eq!(class(&stream.begin_finalize()), expected);
                }
                Op::Complete => {
                    let record = terminal(&stream, TerminalKind::Completed, CREATOR);
                    let expected = model.terminate(TerminalKind::Completed, &[StreamState::Finalizing]);
                    prop_assert_eq!(class(&stream.complete(record)), expected);
                }
                Op::Abort => {
                    let record = terminal(&stream, TerminalKind::Aborted, CREATOR);
                    let expected = model.terminate(
                        TerminalKind::Aborted,
                        &[StreamState::Open, StreamState::Paused, StreamState::Finalizing],
                    );
                    prop_assert_eq!(class(&stream.abort(record)), expected);
                }
                Op::Expire => {
                    let record = terminal(&stream, TerminalKind::Expired, CREATOR);
                    let expected = model.terminate(
                        TerminalKind::Expired,
                        &[StreamState::Open, StreamState::Paused],
                    );
                    prop_assert_eq!(class(&stream.expire(record)), expected);
                }
            }

            // Every resulting state must match the model exactly.
            prop_assert_eq!(stream.state(), model.state);
            prop_assert_eq!(stream.descriptor().current_seq, model.seq);
            prop_assert_eq!(stream.chain(), model.chain.as_slice());
            prop_assert_eq!(
                stream.terminal_record().map(|record| record.kind),
                model.terminal
            );
            prop_assert_eq!(stream.descriptor().generation, model.generation);
            // The terminal invariant: at most one terminal record, and
            // a completed stream always carries its final reference.
            if stream.state() == StreamState::Completed {
                prop_assert!(stream.descriptor().final_ref.is_some());
            } else {
                prop_assert!(stream.descriptor().final_ref.is_none());
            }
        }
    }
}
