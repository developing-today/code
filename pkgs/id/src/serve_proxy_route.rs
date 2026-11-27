//! Live per-leg routing that joins mux credit accounting to frame scheduling.
//!
//! Stream IDs are local to one `LiveLeg`; callers must create one instance per
//! transport leg and map IDs at the broker boundary. DATA enters the scheduler
//! only after both stream and connection credit have been consumed.

use crate::serve_proxy::{ErrorReason, Frame, FrameType};
use crate::serve_proxy_mux::{AcceptOutcome, Mux};
use crate::serve_proxy_scheduler::{Scheduler, StreamPriority};

/// Credit-accounted, priority-scheduled state for one serve-proxy leg.
#[derive(Debug, Default)]
pub struct LiveLeg {
    mux: Mux,
    scheduler: Scheduler,
}

impl LiveLeg {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register the same leg-local stream with credit accounting and scheduling.
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

    /// Account DATA against both credit windows before it becomes schedulable.
    pub fn send_data(&mut self, frame: Frame) -> Result<(), Frame> {
        if frame.ty != FrameType::Data {
            return Err(Frame::error(
                ErrorReason::ProtocolViolation,
                "live leg send_data requires a DATA frame",
            ));
        }
        match self
            .mux
            .accept_data(frame.stream_id, frame.payload.len() as u64)
        {
            AcceptOutcome::Accepted => self.scheduler.enqueue(frame),
            AcceptOutcome::UnknownStream => Err(Frame::error(
                ErrorReason::ProtocolViolation,
                "DATA frame for an unopened leg stream",
            )),
            AcceptOutcome::CreditRefused => Err(Frame::error(
                ErrorReason::CreditOverrun,
                "DATA frame exceeds granted leg credit",
            )),
        }
    }

    /// Queue a non-DATA control frame; controls remain ahead of queued DATA.
    pub fn enqueue_control(&mut self, frame: Frame) -> Result<(), Frame> {
        if frame.ty == FrameType::Data {
            return Err(Frame::error(
                ErrorReason::ProtocolViolation,
                "DATA frame must enter through send_data",
            ));
        }
        self.scheduler.enqueue(frame)
    }

    /// Apply an incoming WINDOW_UPDATE to this leg's mux windows.
    pub fn apply_window_update(&mut self, frame: &Frame) -> Result<(), Frame> {
        self.mux.apply_window_update(frame)
    }

    /// Close a stream in both the credit and scheduling layers.
    pub fn close_stream(&mut self, stream_id: u32) -> bool {
        let closed = self.mux.close_stream(stream_id);
        let scheduled = self.scheduler.close_stream(stream_id);
        debug_assert_eq!(closed, scheduled);
        closed
    }

    /// Take the next frame to write to this leg's transport.
    pub fn pop_next(&mut self) -> Option<Frame> {
        self.scheduler.pop_next()
    }

    pub fn queued_frames(&self) -> usize {
        self.scheduler.queued_frames()
    }

    pub fn stream_available(&self, stream_id: u32) -> Option<u64> {
        self.mux.stream_available(stream_id)
    }

    pub fn connection_available(&self) -> u64 {
        self.mux.connection_available()
    }
}
