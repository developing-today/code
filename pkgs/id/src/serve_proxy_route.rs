//! Serve-proxy per-leg routing: compose stream credit accounting with the
//! scheduler so accepted DATA cannot bypass mux limits and control frames
//! can preempt queued DATA.
//!
//! This is the current routing seam, not the cross-leg stream-ID mapping or
//! a live connection pump. Each instance owns one leg's stream registry,
//! credit windows, and output order.

use crate::serve_proxy::{ErrorReason, Frame, FrameType};
use crate::serve_proxy_mux::{AcceptOutcome, Mux};
use crate::serve_proxy_scheduler::{Scheduler, StreamPriority};

/// One leg's flow-control and output scheduler.
#[derive(Debug, Default)]
pub struct LegRoute {
    mux: Mux,
    scheduler: Scheduler,
}

impl LegRoute {
    /// Register a stream for both credit accounting and priority scheduling.
    pub fn open_stream(&mut self, stream_id: u32, priority: StreamPriority) -> bool {
        if !self.mux.open_stream(stream_id) {
            return false;
        }
        if !self.scheduler.register_stream(stream_id, priority) {
            self.mux.close_stream(stream_id);
            return false;
        }
        true
    }

    /// Close a stream and discard any DATA queued for it.
    pub fn close_stream(&mut self, stream_id: u32) -> bool {
        let mux_closed = self.mux.close_stream(stream_id);
        let scheduler_closed = self.scheduler.close_stream(stream_id);
        debug_assert_eq!(mux_closed, scheduler_closed);
        mux_closed
    }

    /// Accept a frame into the output schedule. DATA first consumes both
    /// mux windows; control frames enter the scheduler's priority queue.
    pub fn enqueue(&mut self, frame: Frame) -> Result<(), Frame> {
        if frame.ty == FrameType::Data {
            match self
                .mux
                .accept_data(frame.stream_id, frame.payload.len() as u64)
            {
                AcceptOutcome::Accepted => {}
                AcceptOutcome::UnknownStream => {
                    return Err(Frame::error(
                        ErrorReason::ProtocolViolation,
                        "DATA frame for an unopened route stream",
                    ));
                }
                AcceptOutcome::CreditRefused => {
                    return Err(Frame::error(
                        ErrorReason::CreditOverrun,
                        "DATA frame exceeds granted mux credit",
                    ));
                }
            }
        }
        self.scheduler.enqueue(frame)
    }

    /// Apply an inbound WINDOW_UPDATE to this leg's mux windows.
    pub fn apply_window_update(&mut self, frame: &Frame) -> Result<(), Frame> {
        self.mux.apply_window_update(frame)
    }

    /// Take the next outbound frame using control/interactive/bulk ordering.
    pub fn pop_next(&mut self) -> Option<Frame> {
        self.scheduler.pop_next()
    }

    /// Number of queued outbound frames.
    pub fn queued_frames(&self) -> usize {
        self.scheduler.queued_frames()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn data(stream_id: u32, payload: &[u8]) -> Frame {
        Frame::data(stream_id, 1, payload.to_vec()).unwrap()
    }

    #[test]
    fn leg_route_enforces_credit_and_control_preempts_bulk_data() {
        let mut route = LegRoute::default();
        assert!(route.open_stream(1, StreamPriority::Bulk));

        // Oracle: the mux's configured initial per-stream credit bounds
        // accepted DATA; a refusal must not enter the scheduled output.
        let too_large = vec![0; crate::serve_proxy::INITIAL_STREAM_CREDIT as usize + 1];
        let error = route.enqueue(data(1, &too_large)).unwrap_err();
        assert_eq!(
            error.error_reason().unwrap(),
            Some(ErrorReason::CreditOverrun)
        );
        assert_eq!(route.queued_frames(), 0);

        route.enqueue(data(1, b"ciphertext")).unwrap();
        route
            .enqueue(Frame::new(FrameType::Ping, 0, 0, Vec::new()).unwrap())
            .unwrap();
        assert_eq!(route.pop_next().unwrap().ty, FrameType::Ping);
        assert_eq!(route.pop_next().unwrap().ty, FrameType::Data);
        assert_eq!(route.pop_next(), None);
    }

    #[test]
    fn closing_route_stream_removes_queued_data() {
        let mut route = LegRoute::default();
        assert!(route.open_stream(1, StreamPriority::Interactive));
        route.enqueue(data(1, b"pending")).unwrap();
        assert!(route.close_stream(1));
        assert_eq!(route.queued_frames(), 0);
        assert_eq!(
            route
                .enqueue(data(1, b"late"))
                .unwrap_err()
                .error_reason()
                .unwrap(),
            Some(ErrorReason::ProtocolViolation)
        );
    }
}
