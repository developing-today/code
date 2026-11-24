//! Serve-proxy mux: the credit-accounting consumer of the
//! [`CreditWindow`](super::serve_proxy::CreditWindow) primitive
//! (#13694, closing the last open criterion of #8970; design record
//! #5792).
//!
//! The codec in [`super::serve_proxy`] defines the frames; this module
//! owns the per-leg accounting the codec's doc comments assign to "the
//! mux slice": one [`Mux`] per leg holds a per-stream window for every
//! open stream plus one aggregate per-connection window, and every
//! DATA accept and every WINDOW_UPDATE goes through it.
//!
//! Rules (decision §3, invariant 5):
//!
//! - DATA is accepted only within the granted per-stream credit *and*
//!   the granted aggregate per-connection credit. Acceptance consumes
//!   both windows atomically: a refusal changes neither window, so
//!   credit is never exceeded and never goes negative.
//! - A WINDOW_UPDATE grants credit back through
//!   [`CreditWindow::grant`](super::serve_proxy::CreditWindow::grant),
//!   bounded by each window's ceiling. A grant that would push a
//!   window past its ceiling (or overflow) is rejected with an ERROR
//!   frame carrying [`ErrorReason::CreditOverrun`] and leaves the
//!   window unchanged — never silently saturated (#8970).
//! - Credit returns only as bounded buffer space is released: the
//!   peer returns stream credit with a stream WINDOW_UPDATE and
//!   connection credit with a connection (stream 0) WINDOW_UPDATE.
//!   Closing a stream drops its stream window; it does *not* refund
//!   connection credit — that credit returns only via a connection
//!   WINDOW_UPDATE from the peer, as in HTTP/2-style flow control.
//!   A stream that closes with credit outstanding therefore holds
//!   that connection credit until the peer releases it.
//! - Ceilings stay parameterized from the effective configuration
//!   ([`Mux::with_credit`]); no universal deployment limit is
//!   invented here. The built-in defaults are the fixed-size windows
//!   of the decision record (§3): 64 KiB per stream, 1 MiB per
//!   connection.

use std::collections::HashMap;

use crate::serve_proxy::{
    CreditWindow, ErrorReason, Frame, FrameType, INITIAL_CONNECTION_CREDIT, INITIAL_STREAM_CREDIT,
};

/// Outcome of offering DATA to the mux for one stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcceptOutcome {
    /// Both windows covered the payload; both were consumed.
    Accepted,
    /// No window exists for the stream (never opened, or closed).
    UnknownStream,
    /// The payload exceeds the granted per-stream or aggregate
    /// per-connection credit; neither window changed.
    CreditRefused,
}

/// Per-leg mux accounting: per-stream windows plus one aggregate
/// per-connection window.
#[derive(Debug)]
pub struct Mux {
    connection: CreditWindow,
    streams: HashMap<u32, CreditWindow>,
    stream_initial: u64,
    stream_ceiling: u64,
}

impl Mux {
    /// A mux with the decision record's fixed-size windows: 64 KiB
    /// per stream, 1 MiB per connection.
    pub fn new() -> Self {
        Self::with_credit(
            INITIAL_STREAM_CREDIT,
            INITIAL_STREAM_CREDIT,
            INITIAL_CONNECTION_CREDIT,
            INITIAL_CONNECTION_CREDIT,
        )
    }

    /// A mux whose windows are parameterized from the effective
    /// configuration: each stream opens with `stream_initial` credit
    /// capped at `stream_ceiling`, the connection with
    /// `connection_initial` capped at `connection_ceiling`.
    pub fn with_credit(
        stream_initial: u64,
        stream_ceiling: u64,
        connection_initial: u64,
        connection_ceiling: u64,
    ) -> Self {
        Self {
            connection: CreditWindow::new(connection_initial, connection_ceiling),
            streams: HashMap::new(),
            stream_initial,
            stream_ceiling,
        }
    }

    /// Open a stream's credit window. Returns `false` (and changes
    /// nothing) for stream 0 or an already-open stream.
    pub fn open_stream(&mut self, stream_id: u32) -> bool {
        if stream_id == 0 || self.streams.contains_key(&stream_id) {
            return false;
        }
        self.streams.insert(
            stream_id,
            CreditWindow::new(self.stream_initial, self.stream_ceiling),
        );
        true
    }

    /// Close a stream's credit window, dropping it. Returns `false`
    /// when the stream was not open. Connection credit is not
    /// refunded: it returns only via a connection WINDOW_UPDATE.
    pub fn close_stream(&mut self, stream_id: u32) -> bool {
        self.streams.remove(&stream_id).is_some()
    }

    /// Offer DATA of `bytes` payload bytes for `stream_id`: accept
    /// only when both the stream window and the aggregate connection
    /// window cover it, consuming both atomically. A refusal changes
    /// neither window.
    pub fn accept_data(&mut self, stream_id: u32, bytes: u64) -> AcceptOutcome {
        let Some(stream) = self.streams.get_mut(&stream_id) else {
            return AcceptOutcome::UnknownStream;
        };
        if bytes > stream.available() || bytes > self.connection.available() {
            return AcceptOutcome::CreditRefused;
        }
        // Both windows cover the payload, so neither consume can fail.
        // (The consumes run unconditionally: a debug_assert! alone
        // would vanish from release builds and leak credit.)
        let consumed_stream = stream.try_consume(bytes);
        let consumed_connection = self.connection.try_consume(bytes);
        debug_assert!(consumed_stream && consumed_connection);
        AcceptOutcome::Accepted
    }

    /// Apply a received WINDOW_UPDATE frame: grant its amount to the
    /// named stream window, or to the connection window when the
    /// frame names stream 0. On success the window holds the
    /// returned credit; on failure the returned ERROR frame carries
    /// the reason and the window is unchanged:
    ///
    /// - over-ceiling or overflowing grant ->
    ///   [`ErrorReason::CreditOverrun`];
    /// - a stream WINDOW_UPDATE for an unknown stream, or a frame
    ///   that is not a WINDOW_UPDATE at all ->
    ///   [`ErrorReason::ProtocolViolation`].
    pub fn apply_window_update(&mut self, frame: &Frame) -> Result<(), Frame> {
        if frame.ty != FrameType::WindowUpdate {
            return Err(Frame::error(
                ErrorReason::ProtocolViolation,
                "mux received a non-WINDOW_UPDATE frame as a credit grant",
            ));
        }
        let amount = match frame.window_update_amount() {
            Ok(Some(amount)) => u64::from(amount),
            _ => {
                return Err(Frame::error(
                    ErrorReason::ProtocolViolation,
                    "WINDOW_UPDATE payload is not a 4-byte credit amount",
                ));
            }
        };
        if frame.stream_id == 0 {
            return self.grant_connection(amount);
        }
        let Some(stream) = self.streams.get_mut(&frame.stream_id) else {
            return Err(Frame::error(
                ErrorReason::ProtocolViolation,
                "WINDOW_UPDATE for an unknown stream",
            ));
        };
        stream.grant(amount).map_err(|_| {
            Frame::error(
                ErrorReason::CreditOverrun,
                "WINDOW_UPDATE would push the stream window past its ceiling",
            )
        })
    }

    fn grant_connection(&mut self, amount: u64) -> Result<(), Frame> {
        self.connection.grant(amount).map_err(|_| {
            Frame::error(
                ErrorReason::CreditOverrun,
                "WINDOW_UPDATE would push the connection window past its ceiling",
            )
        })
    }

    /// Granted credit still available on one stream, or `None` when
    /// the stream is not open.
    pub fn stream_available(&self, stream_id: u32) -> Option<u64> {
        self.streams.get(&stream_id).map(CreditWindow::available)
    }

    /// Granted credit still available on the aggregate connection window.
    pub fn connection_available(&self) -> u64 {
        self.connection.available()
    }

    /// Number of open streams.
    pub fn open_stream_count(&self) -> usize {
        self.streams.len()
    }
}

impl Default for Mux {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn window_update(stream_id: u32, amount: u32) -> Frame {
        Frame::new(
            FrameType::WindowUpdate,
            stream_id,
            0,
            amount.to_be_bytes().to_vec(),
        )
        .unwrap()
    }

    fn credit_overrun(frame: &Frame) -> bool {
        frame.error_reason().unwrap() == Some(ErrorReason::CreditOverrun)
    }

    #[test]
    fn data_accepted_within_stream_and_connection_credit() {
        // #13694 criterion 1: acceptance consumes both windows.
        let mut mux = Mux::new();
        assert!(mux.open_stream(1));
        assert_eq!(mux.accept_data(1, 1_000), AcceptOutcome::Accepted);
        assert_eq!(mux.stream_available(1), Some(INITIAL_STREAM_CREDIT - 1_000));
        assert_eq!(
            mux.connection_available(),
            INITIAL_CONNECTION_CREDIT - 1_000
        );
    }

    #[test]
    fn data_over_stream_credit_is_refused_without_state_change() {
        // #13694 criterion 1: the stream window bounds acceptance.
        let mut mux = Mux::new();
        assert!(mux.open_stream(1));
        assert_eq!(
            mux.accept_data(1, INITIAL_STREAM_CREDIT + 1),
            AcceptOutcome::CreditRefused
        );
        assert_eq!(mux.stream_available(1), Some(INITIAL_STREAM_CREDIT));
        assert_eq!(mux.connection_available(), INITIAL_CONNECTION_CREDIT);
        // An exact-fit accept succeeds; the next byte is refused.
        assert_eq!(
            mux.accept_data(1, INITIAL_STREAM_CREDIT),
            AcceptOutcome::Accepted
        );
        assert_eq!(mux.stream_available(1), Some(0));
        assert_eq!(mux.accept_data(1, 1), AcceptOutcome::CreditRefused);
        assert_eq!(mux.stream_available(1), Some(0));
    }

    #[test]
    fn data_over_connection_credit_is_refused_without_state_change() {
        // #13694 criterion 1: the aggregate window bounds acceptance
        // even when the stream window still has room — and a refusal
        // after the stream check must not partially consume it.
        let mut mux = Mux::with_credit(INITIAL_STREAM_CREDIT, INITIAL_STREAM_CREDIT, 1_000, 1_000);
        assert!(mux.open_stream(1));
        assert!(mux.open_stream(3));
        assert_eq!(mux.accept_data(1, 600), AcceptOutcome::Accepted);
        assert_eq!(mux.accept_data(3, 500), AcceptOutcome::CreditRefused);
        assert_eq!(mux.stream_available(3), Some(INITIAL_STREAM_CREDIT));
        assert_eq!(mux.connection_available(), 400);
        assert_eq!(mux.stream_available(1), Some(INITIAL_STREAM_CREDIT - 600));
    }

    #[test]
    fn data_for_unknown_or_closed_stream_is_refused() {
        let mut mux = Mux::new();
        assert_eq!(mux.accept_data(1, 1), AcceptOutcome::UnknownStream);
        assert!(mux.open_stream(1));
        assert_eq!(mux.accept_data(1, 1), AcceptOutcome::Accepted);
        assert!(mux.close_stream(1));
        assert_eq!(mux.accept_data(1, 1), AcceptOutcome::UnknownStream);
        assert!(!mux.close_stream(1));
    }

    #[test]
    fn window_update_returns_consumed_credit_up_to_the_ceiling() {
        // #13694 criterion 2: the normal credit-return path.
        let mut mux = Mux::new();
        assert!(mux.open_stream(1));
        assert_eq!(mux.accept_data(1, 1_000), AcceptOutcome::Accepted);
        mux.apply_window_update(&window_update(1, 1_000)).unwrap();
        assert_eq!(mux.stream_available(1), Some(INITIAL_STREAM_CREDIT));
        // A connection WINDOW_UPDATE (stream 0) returns connection credit.
        mux.apply_window_update(&window_update(0, 1_000)).unwrap();
        assert_eq!(mux.connection_available(), INITIAL_CONNECTION_CREDIT);
    }

    #[test]
    fn over_ceiling_window_update_is_a_credit_overrun_error_frame() {
        // #13694 criterion 2: over-ceiling and repeated grants are
        // rejected with an ERROR frame and leave the window unchanged.
        let mut mux = Mux::new();
        assert!(mux.open_stream(1));
        let err = mux
            .apply_window_update(&window_update(1, 1))
            .expect_err("grant past the ceiling must fail");
        assert!(credit_overrun(&err));
        assert_eq!(mux.stream_available(1), Some(INITIAL_STREAM_CREDIT));
        for _ in 0..4 {
            let err = mux
                .apply_window_update(&window_update(1, u32::MAX))
                .expect_err("repeated over-ceiling grant must fail");
            assert!(credit_overrun(&err));
        }
        assert_eq!(mux.stream_available(1), Some(INITIAL_STREAM_CREDIT));
        // The same holds for the connection window.
        let err = mux
            .apply_window_update(&window_update(0, 1))
            .expect_err("connection grant past the ceiling must fail");
        assert!(credit_overrun(&err));
        assert_eq!(mux.connection_available(), INITIAL_CONNECTION_CREDIT);
        // The ERROR frame itself round-trips through the codec.
        let encoded = err.encode().unwrap();
        let (decoded, _) = Frame::decode_prefix(&encoded).unwrap().unwrap();
        assert_eq!(
            decoded.error_reason().unwrap(),
            Some(ErrorReason::CreditOverrun)
        );
    }

    #[test]
    fn window_update_for_unknown_stream_is_a_protocol_violation() {
        let mut mux = Mux::new();
        let err = mux
            .apply_window_update(&window_update(7, 1))
            .expect_err("grant to an unknown stream must fail");
        assert_eq!(
            err.error_reason().unwrap(),
            Some(ErrorReason::ProtocolViolation)
        );
        let not_an_update = Frame::new(FrameType::Ping, 0, 0, Vec::new()).unwrap();
        let err = mux
            .apply_window_update(&not_an_update)
            .expect_err("a non-WINDOW_UPDATE frame must fail");
        assert_eq!(
            err.error_reason().unwrap(),
            Some(ErrorReason::ProtocolViolation)
        );
    }

    #[test]
    fn closing_a_stream_drops_its_window_without_refunding_connection_credit() {
        // #13694 criterion 3: credit returns only via WINDOW_UPDATE,
        // never silently on close.
        let mut mux = Mux::new();
        assert!(mux.open_stream(1));
        assert_eq!(mux.accept_data(1, 1_000), AcceptOutcome::Accepted);
        assert!(mux.close_stream(1));
        assert_eq!(mux.stream_available(1), None);
        assert_eq!(
            mux.connection_available(),
            INITIAL_CONNECTION_CREDIT - 1_000
        );
        assert!(!mux.open_stream(0));
        // A closed stream can be reopened with a fresh window.
        assert!(mux.open_stream(1));
        assert!(!mux.open_stream(1));
    }

    #[test]
    fn configured_ceilings_parameterize_the_windows() {
        // #13694 constraint: ceilings come from configuration, not a
        // universal deployment limit.
        let mut mux = Mux::with_credit(100, 250, 1_000, 1_500);
        assert!(mux.open_stream(1));
        assert_eq!(mux.accept_data(1, 100), AcceptOutcome::Accepted);
        // Consumed 100, so a grant of 250 refills the window exactly
        // to its ceiling.
        mux.apply_window_update(&window_update(1, 250)).unwrap();
        assert_eq!(mux.stream_available(1), Some(250));
        assert!(mux.apply_window_update(&window_update(1, 1)).is_err());
        assert_eq!(mux.stream_available(1), Some(250));
    }

    /// Model of one mux for the property test: per-stream available
    /// credit, connection available credit, and the open set.
    #[derive(Debug, Default)]
    struct Model {
        streams: HashMap<u32, u64>,
        connection: u64,
    }

    #[derive(Debug, Clone, Copy)]
    enum Op {
        Open(u32),
        Accept(u32, u64),
        GrantStream(u32, u32),
        GrantConnection(u32),
        Close(u32),
    }

    proptest! {
        /// Oracle: the mux agrees with a model of the #13694 rules
        /// under any interleaving of opens, DATA accepts, credit
        /// returns, and closes — credit is never exceeded, no window
        /// ever holds more than its ceiling, and every refusal leaves
        /// the mux state exactly as the model says.
        #[test]
        fn mux_accounting_matches_the_model(
            ops in prop::collection::vec(
                (
                    0u8..5,
                    prop::sample::select(vec![1u32, 3, 5]),
                    0u64..200_000,
                ),
                0..48,
            ),
        ) {
            let mut mux = Mux::new();
            let mut model = Model {
                streams: HashMap::new(),
                connection: INITIAL_CONNECTION_CREDIT,
            };
            for (kind, stream_id, amount) in ops {
                let op = match kind {
                    0 => Op::Open(stream_id),
                    1 => Op::Accept(stream_id, amount),
                    2 => Op::GrantStream(stream_id, amount as u32),
                    3 => Op::GrantConnection(amount as u32),
                    _ => Op::Close(stream_id),
                };
                match op {
                    Op::Open(id) => {
                        let expected = !model.streams.contains_key(&id);
                        prop_assert_eq!(mux.open_stream(id), expected);
                        if expected {
                            model.streams.insert(id, INITIAL_STREAM_CREDIT);
                        }
                    }
                    Op::Accept(id, bytes) => {
                        let outcome = mux.accept_data(id, bytes);
                        match model.streams.get(&id) {
                            None => prop_assert_eq!(outcome, AcceptOutcome::UnknownStream),
                            Some(&available) => {
                                if bytes <= available && bytes <= model.connection {
                                    prop_assert_eq!(outcome, AcceptOutcome::Accepted);
                                    model.streams.insert(id, available - bytes);
                                    model.connection -= bytes;
                                } else {
                                    prop_assert_eq!(outcome, AcceptOutcome::CreditRefused);
                                }
                            }
                        }
                    }
                    Op::GrantStream(id, amount) => {
                        let result = mux.apply_window_update(&window_update(id, amount));
                        match model.streams.get(&id) {
                            None => prop_assert!(result.is_err()),
                            Some(&available) => {
                                let next = available.checked_add(u64::from(amount));
                                match next {
                                    Some(next) if next <= INITIAL_STREAM_CREDIT => {
                                        prop_assert!(result.is_ok());
                                        model.streams.insert(id, next);
                                    }
                                    _ => {
                                        prop_assert!(result.is_err());
                                        prop_assert!(credit_overrun(&result.unwrap_err()));
                                    }
                                }
                            }
                        }
                    }
                    Op::GrantConnection(amount) => {
                        let result = mux.apply_window_update(&window_update(0, amount));
                        let next = model.connection.checked_add(u64::from(amount));
                        match next {
                            Some(next) if next <= INITIAL_CONNECTION_CREDIT => {
                                prop_assert!(result.is_ok());
                                model.connection = next;
                            }
                            _ => {
                                prop_assert!(result.is_err());
                                prop_assert!(credit_overrun(&result.unwrap_err()));
                            }
                        }
                    }
                    Op::Close(id) => {
                        let expected = model.streams.remove(&id).is_some();
                        prop_assert_eq!(mux.close_stream(id), expected);
                    }
                }
                prop_assert_eq!(mux.connection_available(), model.connection);
                prop_assert!(mux.connection_available() <= INITIAL_CONNECTION_CREDIT);
                prop_assert_eq!(mux.open_stream_count(), model.streams.len());
                for (id, available) in &model.streams {
                    prop_assert_eq!(mux.stream_available(*id), Some(*available));
                    prop_assert!(*available <= INITIAL_STREAM_CREDIT);
                }
            }
        }
    }
}
