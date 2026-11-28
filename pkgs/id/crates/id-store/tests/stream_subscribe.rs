//! Model test for resumable stream subscription and
//! completed-target resolution (slice A9123-stream-subscribe,
//! issue #10282; design part A §#9123, A-9123.1, statements
//! S4.79–S4.82).
//!
//! Oracle: **model** — expected `Reference stream chain
//! prefix/suffix model plus current authorization result.` The
//! reference model below is an independent fold over plain
//! values: it tracks the accepted chain's record IDs, the
//! lifecycle state, the retention floor, the subscription
//! position, the cancellation/terminal flags and the *current
//! authorization result*, and derives every expected page,
//! resolution and error class from those alone. Generated
//! operations (append, pause/resume, finalize/complete, abort,
//! expire, poll, resolve, cancel, prune, resume, tampered
//! resume, authorization flips) run against the model and
//! against the module, and every resulting page, state and
//! error class is compared after every operation.
//!
//! Fixed tests pin the design's named cases: resume from a
//! cursor delivers the exact suffix, backpressure pages under
//! `max_items` with no silent drop, cancellation is an
//! observable terminal page, a completed target is re-checked
//! against the current authorization result before its final
//! reference is revealed, and resolution is a typed value —
//! there is no redirect to follow implicitly.
//!
//! Why the modules are included by path: slice 9110-02 owns the
//! crate roots, so neither `stream_record` (id-core) nor
//! `stream_subscribe` (id-store) is exported yet; the test
//! includes them, and the modules `stream_record` builds on, by
//! path — the same discipline as the stream-records model test.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../../id-core/src/canonical_cbor.rs"]
#[allow(dead_code)]
mod canonical_cbor;

#[path = "../../id-core/src/id_shape.rs"]
#[allow(dead_code)]
mod id_shape;

#[path = "../../id-core/src/hash_profile.rs"]
#[allow(dead_code)]
mod hash_profile;

#[path = "../../id-core/src/signed_envelope.rs"]
#[allow(dead_code)]
mod signed_envelope;

#[path = "../../id-core/src/array_address.rs"]
#[allow(dead_code)]
mod array_address;

#[path = "../../id-core/src/stream_record.rs"]
mod stream_record;

#[path = "../src/stream_subscribe.rs"]
mod stream_subscribe;

use id_shape::{ContentId, HashProfile};
use signed_envelope::PayloadId;
use stream_record::{
    AppendOutcome, ChunkRecord, NamespaceId, StreamDescriptor, StreamLifecycle, StreamState,
    TerminalKind, TerminalRecord,
};
use stream_subscribe::{
    Delivery, ResolutionView, StreamCursor, StreamSubscription, SubscribeError, SubscriberContext,
    TerminalOutcome,
};

const CREATOR: u8 = 7;
const SCOPE: &[u8] = b"tenant-a";
const MEDIA: &str = "text/plain";
const NAMESPACE: [u8; 32] = [3u8; 32];

fn key(seed: u8) -> ed25519_dalek::SigningKey {
    ed25519_dalek::SigningKey::from_bytes(&[seed; 32])
}

fn public(seed: u8) -> [u8; 32] {
    key(seed).verifying_key().to_bytes()
}

fn descriptor() -> StreamDescriptor {
    StreamDescriptor::open(
        NamespaceId::new(NAMESPACE),
        MEDIA,
        public(CREATOR),
        1_700_000_000,
    )
    .unwrap()
}

fn machine() -> StreamLifecycle {
    StreamLifecycle::new(descriptor(), SCOPE.to_vec(), None).unwrap()
}

fn context() -> SubscriberContext {
    SubscriberContext::new(SCOPE.to_vec(), NamespaceId::new(NAMESPACE)).unwrap()
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

/// A stream with `n` chunks appended, plus the retained records.
fn grown(n: u8) -> (StreamLifecycle, Vec<ChunkRecord>) {
    let mut stream = machine();
    let mut records = Vec::new();
    for i in 0..n {
        let record = chunk(&stream, &[b'a' + i], CREATOR);
        assert_eq!(
            stream.append_chunk(&record).unwrap(),
            AppendOutcome::Appended
        );
        records.push(record);
    }
    (stream, records)
}

fn class<T>(result: &Result<T, SubscribeError>) -> &'static str {
    match result {
        Ok(_) => "ok",
        Err(error) => error.class(),
    }
}

/// A page reduced to plain comparable values: event summaries in
/// order, the truncated flag, the terminal class, and the next
/// cursor's sequence.
#[derive(Debug, PartialEq)]
struct PageSummary {
    events: Vec<String>,
    truncated: bool,
    terminal: Option<&'static str>,
    next_sequence: u64,
}

fn summarize(page: &stream_subscribe::SubscribePage) -> PageSummary {
    let events = page
        .events()
        .iter()
        .map(|event| match event {
            Delivery::Chunk { sequence, .. } => format!("chunk:{sequence}"),
            Delivery::Heartbeat { state, .. } => format!("heartbeat:{}", state.token()),
            Delivery::Terminal { outcome } => format!("terminal:{}", outcome_name(*outcome)),
        })
        .collect();
    PageSummary {
        events,
        truncated: page.truncated(),
        terminal: page.terminal().map(outcome_name),
        next_sequence: page.next_cursor().sequence(),
    }
}

fn outcome_name(outcome: TerminalOutcome) -> &'static str {
    match outcome {
        TerminalOutcome::Completed { .. } => "completed",
        TerminalOutcome::Aborted => "aborted",
        TerminalOutcome::Expired => "expired",
        TerminalOutcome::Cancelled => "cancelled",
    }
}

#[test]
fn genesis_replay_exact_suffix_then_heartbeat() {
    let (stream, records) = grown(3);
    let ctx = context();
    assert_eq!(ctx.tenant_scope(), SCOPE);
    assert_eq!(ctx.namespace_id(), NamespaceId::new(NAMESPACE));
    let mut sub = StreamSubscription::subscribe(&stream, &records, 0, ctx, true).unwrap();
    assert_eq!(sub.stream_id(), stream.descriptor().stream_id);
    assert_eq!(sub.state(), StreamState::Open);
    assert_eq!(sub.position(), StreamCursor::genesis());

    let page = sub.poll(2, true).unwrap();
    assert_eq!(
        summarize(&page),
        PageSummary {
            events: vec!["chunk:0".into(), "chunk:1".into()],
            truncated: true,
            terminal: None,
            next_sequence: 2,
        }
    );
    assert_eq!(
        page.next_cursor().record_id(),
        Some(records[1].record_id().unwrap())
    );

    let page = sub.poll(2, true).unwrap();
    assert_eq!(
        summarize(&page),
        PageSummary {
            events: vec!["chunk:2".into(), "heartbeat:open".into()],
            truncated: false,
            terminal: None,
            next_sequence: 3,
        }
    );

    // At the head of a live stream, a poll is a bare heartbeat.
    let page = sub.poll(2, true).unwrap();
    assert_eq!(page.events().len(), 1);
    assert!(matches!(page.events()[0], Delivery::Heartbeat { .. }));
}

#[test]
fn resume_from_cursor_delivers_exact_suffix() {
    let (stream, records) = grown(4);
    let cursor = StreamCursor::new(2, Some(records[1].record_id().unwrap())).unwrap();
    // The cursor survives its canonical round-trip.
    let decoded = StreamCursor::decode(&cursor.canonical_bytes().unwrap()).unwrap();
    assert_eq!(decoded, cursor);

    let mut sub =
        StreamSubscription::resume(&stream, &records, 0, context(), true, decoded).unwrap();
    let page = sub.poll(10, true).unwrap();
    assert_eq!(
        summarize(&page).events,
        vec!["chunk:2", "chunk:3", "heartbeat:open"]
    );
}

#[test]
fn cursor_rejection_classes() {
    let (stream, records) = grown(4);
    // Past the head.
    let ahead = StreamCursor::new(5, Some(records[3].record_id().unwrap())).unwrap();
    assert_eq!(
        class(&StreamSubscription::resume(
            &stream,
            &records,
            0,
            context(),
            true,
            ahead
        )),
        "cursor_ahead"
    );
    // Record ID does not match the chain at that sequence.
    let forged_id = stream_record::ChunkRecordId::from_content_id(
        ContentId::of_bytes(HashProfile::RawBlake3_256, b"forged").unwrap(),
    );
    let forged = StreamCursor::new(2, Some(forged_id)).unwrap();
    assert_eq!(
        class(&StreamSubscription::resume(
            &stream,
            &records,
            0,
            context(),
            true,
            forged
        )),
        "cursor_mismatch"
    );
    // Below the retention floor.
    let low = StreamCursor::new(1, Some(records[0].record_id().unwrap())).unwrap();
    assert_eq!(
        class(&StreamSubscription::resume(
            &stream,
            &records[2..],
            2,
            context(),
            true,
            low
        )),
        "cursor_expired"
    );
    // Genesis discipline: sequence 0 carries no record ID.
    assert_eq!(
        class(&StreamCursor::new(0, Some(records[0].record_id().unwrap()))),
        "malformed"
    );
    assert_eq!(class(&StreamCursor::new(1, None)), "malformed");
}

#[test]
fn backpressure_pages_of_one_chain_cursors() {
    let (stream, records) = grown(3);
    let mut sub = StreamSubscription::subscribe(&stream, &records, 0, context(), true).unwrap();
    let mut seen = Vec::new();
    for expected_seq in 0..3u64 {
        let page = sub.poll(1, true).unwrap();
        assert_eq!(page.next_cursor().sequence(), expected_seq + 1);
        assert_eq!(
            page.next_cursor().record_id(),
            Some(
                records[usize::try_from(expected_seq).unwrap()]
                    .record_id()
                    .unwrap()
            )
        );
        seen.extend(summarize(&page).events);
        if expected_seq < 2 {
            assert!(page.truncated());
        }
    }
    assert_eq!(
        seen,
        vec!["chunk:0", "chunk:1", "chunk:2", "heartbeat:open"]
    );
    assert_eq!(class(&sub.poll(0, true)), "invalid_max_items");
}

#[test]
fn completed_target_access_is_rechecked() {
    let (mut stream, records) = grown(2);
    stream.begin_finalize().unwrap();
    stream
        .complete(terminal(&stream, TerminalKind::Completed, CREATOR))
        .unwrap();

    // Resolution under a holding authorization reveals the typed
    // final target — a value, not a redirect.
    let sub = StreamSubscription::subscribe(&stream, &records, 0, context(), true).unwrap();
    assert_eq!(
        sub.resolve_target(true).unwrap(),
        ResolutionView::Completed {
            final_id: final_ref()
        }
    );
    // The same resolution under a revoked authorization is
    // refused: the final reference is not revealed.
    assert_eq!(class(&sub.resolve_target(false)), "unauthorized");
    assert_eq!(
        class(&StreamSubscription::subscribe(
            &stream,
            &records,
            0,
            context(),
            false
        )),
        "unauthorized"
    );

    // Replay delivers every chunk before the terminal event, and
    // the terminal outcome carries the same final reference.
    let mut sub = sub;
    let page = sub.poll(1, true).unwrap();
    assert_eq!(summarize(&page).events, vec!["chunk:0"]);
    // The terminal event rides the page that delivers the last
    // chunk: a page caps chunk events, never the terminal signal.
    let page = sub.poll(1, true).unwrap();
    assert_eq!(
        summarize(&page).events,
        vec!["chunk:1", "terminal:completed"]
    );
    match page.events()[1] {
        Delivery::Terminal {
            outcome: TerminalOutcome::Completed { final_id },
        } => {
            assert_eq!(final_id, final_ref());
        }
        ref other => panic!("unexpected event {other:?}"),
    }
    assert_eq!(class(&sub.poll(1, true)), "already_terminal");

    // Authorization is re-checked on every poll, not just at
    // subscribe time.
    let (stream2, records2) = grown(1);
    let mut sub2 = StreamSubscription::subscribe(&stream2, &records2, 0, context(), true).unwrap();
    assert_eq!(class(&sub2.poll(1, false)), "unauthorized");
}

#[test]
fn aborted_and_expired_claim_no_final_content() {
    for (kind, token) in [
        (TerminalKind::Aborted, "aborted"),
        (TerminalKind::Expired, "expired"),
    ] {
        let (mut stream, records) = grown(1);
        match kind {
            TerminalKind::Aborted => stream.abort(terminal(&stream, kind, CREATOR)).unwrap(),
            _ => stream.expire(terminal(&stream, kind, CREATOR)).unwrap(),
        }
        let mut sub = StreamSubscription::subscribe(&stream, &records, 0, context(), true).unwrap();
        let page = sub.poll(10, true).unwrap();
        assert_eq!(
            summarize(&page).events,
            vec!["chunk:0".to_string(), format!("terminal:{token}")]
        );
        // Resolution stays Live: no final reference exists to
        // follow for an aborted or expired stream.
        assert!(matches!(
            sub.resolve_target(true).unwrap(),
            ResolutionView::Live { .. }
        ));
    }
}

#[test]
fn cancel_is_an_observable_terminal_page() {
    let (stream, records) = grown(3);
    let mut sub = StreamSubscription::subscribe(&stream, &records, 0, context(), true).unwrap();
    let page = sub.poll(1, true).unwrap();
    assert_eq!(summarize(&page).events, vec!["chunk:0"]);
    sub.cancel();
    let page = sub.poll(10, true).unwrap();
    assert_eq!(
        summarize(&page),
        PageSummary {
            events: vec!["terminal:cancelled".into()],
            truncated: false,
            terminal: Some("cancelled"),
            next_sequence: 1,
        }
    );
    assert_eq!(class(&sub.poll(10, true)), "already_terminal");
}

#[test]
fn scope_and_fold_validation() {
    let (stream, records) = grown(2);
    // A context in another namespace is refused.
    let other_ns = SubscriberContext::new(SCOPE.to_vec(), NamespaceId::new([9u8; 32])).unwrap();
    assert_eq!(
        class(&StreamSubscription::subscribe(
            &stream, &records, 0, other_ns, true
        )),
        "scope_mismatch"
    );
    // A context under another tenant scope is refused: the
    // records bind the stream's scope, not the caller's.
    let other_scope =
        SubscriberContext::new(b"tenant-b".to_vec(), NamespaceId::new(NAMESPACE)).unwrap();
    assert_eq!(
        class(&StreamSubscription::subscribe(
            &stream,
            &records,
            0,
            other_scope,
            true
        )),
        "scope_mismatch"
    );
    // A retained window that does not reach the head does not
    // fold.
    assert_eq!(
        class(&StreamSubscription::subscribe(
            &stream,
            &records[..1],
            0,
            context(),
            true
        )),
        "chain_mismatch"
    );
    // A record rebuilt at sequence 1 with the original prev link
    // but a different payload does not fold onto the chain.
    let mut tampered = records.clone();
    let payload_id = PayloadId::of(HashProfile::RawBlake3_256, b"swapped").unwrap();
    tampered[1] = ChunkRecord::unsigned(
        stream.descriptor().stream_id,
        SCOPE.to_vec(),
        1,
        Some(records[0].record_id().unwrap()),
        &payload_id,
        MEDIA,
    )
    .unwrap()
    .sign(&key(CREATOR))
    .unwrap();
    assert_eq!(
        class(&StreamSubscription::subscribe(
            &stream,
            &tampered,
            0,
            context(),
            true
        )),
        "chain_mismatch"
    );
}

#[test]
fn prune_past_position_expires_the_subscription() {
    let (stream, records) = grown(4);
    let mut sub = StreamSubscription::subscribe(&stream, &records, 0, context(), true).unwrap();
    let page = sub.poll(1, true).unwrap();
    assert_eq!(page.next_cursor().sequence(), 1);
    // Retention moves to 2: the position at 1 is pruned away.
    assert_eq!(
        class(&sub.refresh(&stream, &records[2..], 2)),
        "cursor_expired"
    );
    assert_eq!(class(&sub.poll(1, true)), "cursor_expired");
    // A fresh resume at the floor works and starts exactly there.
    let floor = StreamCursor::new(2, Some(records[1].record_id().unwrap())).unwrap();
    let mut sub2 =
        StreamSubscription::resume(&stream, &records[2..], 2, context(), true, floor).unwrap();
    let page = sub2.poll(10, true).unwrap();
    assert_eq!(
        summarize(&page).events,
        vec!["chunk:2", "chunk:3", "heartbeat:open"]
    );
}

#[test]
fn cursor_canonical_round_trip_and_rejections() {
    let genesis = StreamCursor::genesis();
    assert_eq!(
        StreamCursor::decode(&genesis.canonical_bytes().unwrap()).unwrap(),
        genesis
    );
    // Garbage bytes and a truncated encoding are malformed.
    assert_eq!(class(&StreamCursor::decode(b"not cbor")), "malformed");
    let bytes = genesis.canonical_bytes().unwrap();
    assert_eq!(
        class(&StreamCursor::decode(&bytes[..bytes.len() - 1])),
        "malformed"
    );
}

// ---------------------------------------------------------------------------
// Reference model + generated operations
// ---------------------------------------------------------------------------

/// The reference model: the chain prefix/suffix fold plus the
/// current authorization result, over plain values only. Every
/// expected page, resolution and error class is derived here,
/// independently of the module under test.
struct Model {
    chain_len: u64,
    state: StreamState,
    final_ref: Option<ContentId>,
    authorized: bool,
    retained_from: u64,
    position: u64,
    cancelled: bool,
    terminal_delivered: bool,
    expired: bool,
}

impl Model {
    fn poll(&mut self, max_items: usize) -> Result<PageSummary, &'static str> {
        if !self.authorized {
            return Err("unauthorized");
        }
        if max_items == 0 {
            return Err("invalid_max_items");
        }
        if self.terminal_delivered {
            return Err("already_terminal");
        }
        if self.expired {
            return Err("cursor_expired");
        }
        if self.cancelled {
            self.terminal_delivered = true;
            return Ok(PageSummary {
                events: vec!["terminal:cancelled".into()],
                truncated: false,
                terminal: Some("cancelled"),
                next_sequence: self.position,
            });
        }
        let mut events = Vec::new();
        while self.position < self.chain_len && events.len() < max_items {
            events.push(format!("chunk:{}", self.position));
            self.position += 1;
        }
        let truncated = self.position < self.chain_len;
        let mut terminal = None;
        if !truncated {
            if self.state.is_terminal() {
                let name = match self.state {
                    StreamState::Completed => "completed",
                    StreamState::Aborted => "aborted",
                    _ => "expired",
                };
                events.push(format!("terminal:{name}"));
                terminal = Some(name);
                self.terminal_delivered = true;
            } else {
                events.push(format!("heartbeat:{}", self.state.token()));
            }
        }
        Ok(PageSummary {
            events,
            truncated,
            terminal,
            next_sequence: self.position,
        })
    }

    fn resolve(&self) -> Result<String, &'static str> {
        if !self.authorized {
            return Err("unauthorized");
        }
        match (self.state, self.final_ref) {
            (StreamState::Completed, Some(_)) => Ok("completed".into()),
            (state, _) => Ok(format!("live:{}", state.token())),
        }
    }

    /// The class a refresh must produce after the floor moves to
    /// `floor` (the fold itself always succeeds in the driver:
    /// the window it passes is the true retained chain).
    fn refresh_class(&mut self, floor: u64) -> &'static str {
        self.retained_from = floor;
        if self.position < floor {
            self.expired = true;
            "cursor_expired"
        } else {
            "ok"
        }
    }
}

/// A deterministic xorshift generator: the same seeds replay the
/// same operation lists on every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, bound: u64) -> u64 {
        if bound == 0 { 0 } else { self.next() % bound }
    }
}

#[test]
fn generated_operations_match_reference_model() {
    for seed in [11u64, 23, 37] {
        run_generated(seed);
    }
}

fn run_generated(seed: u64) {
    let mut rng = Rng(seed ^ 0x9e37_79b9_7f4a_7c15);
    let (mut stream, mut records) = grown(2);
    let mut sub = StreamSubscription::subscribe(&stream, &records, 0, context(), true).unwrap();
    let mut model = Model {
        chain_len: 2,
        state: StreamState::Open,
        final_ref: None,
        authorized: true,
        retained_from: 0,
        position: 0,
        cancelled: false,
        terminal_delivered: false,
        expired: false,
    };
    let mut floor = 0u64;

    // Refresh both sides after a lifecycle mutation and compare
    // the class; the model always folds the true window.
    macro_rules! refresh_both {
        () => {{
            let got =
                class(&sub.refresh(&stream, &records[usize::try_from(floor).unwrap()..], floor));
            let want = model.refresh_class(floor);
            assert_eq!(got, want, "seed {seed}: refresh class");
            model.state = stream.state();
            model.chain_len = stream.descriptor().current_seq;
            model.final_ref = stream.descriptor().final_ref;
        }};
    }

    for step in 0..140u64 {
        let roll = rng.below(100);
        match roll {
            // Append a valid chunk (the lifecycle may refuse:
            // paused / finalizing / terminal — the model chain
            // only grows on Appended).
            0..=24 => {
                let payload = format!("op-{seed}-{step}");
                let record = chunk(&stream, payload.as_bytes(), CREATOR);
                if let Ok(AppendOutcome::Appended) = stream.append_chunk(&record) {
                    records.push(record);
                }
                refresh_both!();
            }
            // Poll with a small page cap (backpressure).
            25..=39 => {
                let max_items = 1 + usize::try_from(rng.below(3)).unwrap();
                let want = model.poll(max_items);
                let got = sub.poll(max_items, model.authorized);
                match (want, got) {
                    (Ok(want_page), Ok(got_page)) => {
                        assert_eq!(summarize(&got_page), want_page, "seed {seed} step {step}");
                    }
                    (Err(want_class), Err(error)) => {
                        assert_eq!(error.class(), want_class, "seed {seed} step {step}");
                    }
                    (want, got) => {
                        panic!(
                            "seed {seed} step {step}: model {want:?} vs module {:?}",
                            got.map(|p| summarize(&p))
                        );
                    }
                }
            }
            // Pause / resume toggle.
            40..=47 => {
                match stream.state() {
                    StreamState::Open => {
                        stream.pause().unwrap();
                    }
                    StreamState::Paused => {
                        stream.resume().unwrap();
                    }
                    _ => {}
                }
                refresh_both!();
            }
            // Drive to a terminal state by whichever path the
            // current state allows.
            48..=59 => {
                match stream.state() {
                    StreamState::Open => {
                        if roll % 2 == 0 {
                            stream.begin_finalize().unwrap();
                            stream
                                .complete(terminal(&stream, TerminalKind::Completed, CREATOR))
                                .unwrap();
                        } else {
                            stream
                                .abort(terminal(&stream, TerminalKind::Aborted, CREATOR))
                                .unwrap();
                        }
                    }
                    StreamState::Paused => {
                        stream
                            .expire(terminal(&stream, TerminalKind::Expired, CREATOR))
                            .unwrap();
                    }
                    StreamState::Finalizing => {
                        stream
                            .complete(terminal(&stream, TerminalKind::Completed, CREATOR))
                            .unwrap();
                    }
                    _ => {}
                }
                refresh_both!();
            }
            // Flip the current authorization result.
            60..=66 => {
                model.authorized = !model.authorized;
            }
            // Resolve under the current authorization result.
            67..=72 => {
                let want = model.resolve();
                let got = sub.resolve_target(model.authorized);
                match (want, got) {
                    (Ok(want), Ok(ResolutionView::Completed { final_id })) => {
                        assert_eq!(want, "completed", "seed {seed} step {step}");
                        assert_eq!(final_id, final_ref());
                    }
                    (Ok(want), Ok(ResolutionView::Live { state, .. })) => {
                        assert_eq!(
                            want,
                            format!("live:{}", state.token()),
                            "seed {seed} step {step}"
                        );
                    }
                    (Err(want_class), Err(error)) => {
                        assert_eq!(error.class(), want_class, "seed {seed} step {step}");
                    }
                    (want, got) => {
                        panic!("seed {seed} step {step}: model {want:?} vs module {got:?}")
                    }
                }
            }
            // Cancel the subscription.
            73..=77 => {
                sub.cancel();
                model.cancelled = true;
            }
            // Resume from a valid historical cursor: replaces the
            // subscription and resets its delivery flags.
            78..=84 => {
                let span = model.chain_len - floor + 1;
                let seq = floor + rng.below(span);
                let cursor = StreamCursor::new(
                    seq,
                    if seq == 0 {
                        None
                    } else {
                        Some(
                            records[usize::try_from(seq - 1).unwrap()]
                                .record_id()
                                .unwrap(),
                        )
                    },
                )
                .unwrap();
                let got = StreamSubscription::resume(
                    &stream,
                    &records[usize::try_from(floor).unwrap()..],
                    floor,
                    context(),
                    model.authorized,
                    cursor,
                );
                if model.authorized {
                    sub = got.unwrap();
                    model.position = seq;
                    model.cancelled = false;
                    model.terminal_delivered = false;
                    model.expired = false;
                } else {
                    assert_eq!(class(&got), "unauthorized", "seed {seed} step {step}");
                }
            }
            // Resume with a forged record ID: always a mismatch
            // (validation runs before the authorization check).
            85..=89 => {
                if model.chain_len > floor {
                    let seq = floor + 1 + rng.below(model.chain_len - floor);
                    let forged = StreamCursor::new(
                        seq,
                        Some(stream_record::ChunkRecordId::from_content_id(
                            ContentId::of_bytes(
                                HashProfile::RawBlake3_256,
                                format!("forged-{seed}-{step}").as_bytes(),
                            )
                            .unwrap(),
                        )),
                    )
                    .unwrap();
                    let got = StreamSubscription::resume(
                        &stream,
                        &records[usize::try_from(floor).unwrap()..],
                        floor,
                        context(),
                        model.authorized,
                        forged,
                    );
                    assert_eq!(class(&got), "cursor_mismatch", "seed {seed} step {step}");
                }
            }
            // Prune the retention floor upward.
            90..=94 => {
                floor += rng.below(model.chain_len - floor + 1);
                refresh_both!();
            }
            // A zero page cap is a caller bug, never an empty
            // page (unless authorization fails first — the model
            // poll orders the checks identically).
            _ => {
                let want = model.poll(0);
                let got = sub.poll(0, model.authorized);
                match (want, got) {
                    (Err(want_class), Err(error)) => {
                        assert_eq!(error.class(), want_class, "seed {seed} step {step}");
                    }
                    (want, got) => {
                        panic!(
                            "seed {seed} step {step}: model {want:?} vs module {:?}",
                            got.map(|p| summarize(&p))
                        );
                    }
                }
            }
        }
        assert_eq!(
            sub.position().sequence(),
            model.position,
            "seed {seed} step {step}: position"
        );
    }
}
