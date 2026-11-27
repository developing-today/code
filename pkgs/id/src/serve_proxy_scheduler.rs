//! Priority and round-robin scheduling for serve-proxy DATA frames.
//!
//! Implements the scheduling part of #14918 / decision #5792 §3 without
//! coupling it to transport or mux-credit accounting. Control frames always
//! drain first; interactive DATA drains before bulk DATA; streams within each
//! DATA class take turns one frame at a time.

use std::collections::{HashMap, VecDeque};

use crate::serve_proxy::{ErrorReason, Frame, FrameType, SchedulingClass};

#[derive(Debug, Default)]
struct ClassQueue {
    ready: VecDeque<u32>,
    frames: HashMap<u32, VecDeque<Frame>>,
}

impl ClassQueue {
    fn push(&mut self, stream_id: u32, frame: Frame) {
        let queue = self.frames.entry(stream_id).or_default();
        if queue.is_empty() {
            self.ready.push_back(stream_id);
        }
        queue.push_back(frame);
    }

    fn pop(&mut self) -> Option<Frame> {
        while let Some(stream_id) = self.ready.pop_front() {
            let Some(queue) = self.frames.get_mut(&stream_id) else {
                continue;
            };
            let frame = queue.pop_front()?;
            if queue.is_empty() {
                self.frames.remove(&stream_id);
            } else {
                self.ready.push_back(stream_id);
            }
            return Some(frame);
        }
        None
    }

    fn remove_stream(&mut self, stream_id: u32) {
        self.frames.remove(&stream_id);
        self.ready.retain(|ready_id| *ready_id != stream_id);
    }
}

/// A DATA stream's priority. Control is reserved for non-DATA frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamPriority {
    /// SSH-class and ID query traffic.
    Interactive,
    /// Bulk synchronization traffic.
    Bulk,
}

/// A leg-local scheduler. Control frames are FIFO and preempt DATA; DATA
/// streams are round-robin within their priority class.
#[derive(Debug, Default)]
pub struct Scheduler {
    control: VecDeque<Frame>,
    priorities: HashMap<u32, StreamPriority>,
    interactive: ClassQueue,
    bulk: ClassQueue,
}

impl Scheduler {
    /// Register a nonzero stream ID. A duplicate registration is rejected.
    pub fn register_stream(&mut self, stream_id: u32, priority: StreamPriority) -> bool {
        if stream_id == 0 || self.priorities.contains_key(&stream_id) {
            return false;
        }
        self.priorities.insert(stream_id, priority);
        true
    }

    /// Remove a stream and any DATA queued for it. Returns whether it existed.
    pub fn close_stream(&mut self, stream_id: u32) -> bool {
        let Some(priority) = self.priorities.remove(&stream_id) else {
            return false;
        };
        match priority {
            StreamPriority::Interactive => self.interactive.remove_stream(stream_id),
            StreamPriority::Bulk => self.bulk.remove_stream(stream_id),
        }
        true
    }

    /// Enqueue one frame. DATA requires a registered stream; all other frame
    /// types enter the FIFO control queue. Errors are protocol ERROR frames.
    pub fn enqueue(&mut self, frame: Frame) -> Result<(), Frame> {
        if frame.ty != FrameType::Data {
            debug_assert_eq!(
                frame.ty.scheduling_class(),
                SchedulingClass::Control,
                "every non-DATA frame is a control frame"
            );
            self.control.push_back(frame);
            return Ok(());
        }

        let Some(priority) = self.priorities.get(&frame.stream_id).copied() else {
            return Err(Frame::error(
                ErrorReason::ProtocolViolation,
                "DATA frame for an unregistered stream",
            ));
        };
        match priority {
            StreamPriority::Interactive => self.interactive.push(frame.stream_id, frame),
            StreamPriority::Bulk => self.bulk.push(frame.stream_id, frame),
        }
        Ok(())
    }

    /// Take the next frame: FIFO control first, then interactive DATA, then
    /// bulk DATA. Each DATA stream gets at most one frame per turn.
    pub fn pop_next(&mut self) -> Option<Frame> {
        self.control
            .pop_front()
            .or_else(|| self.interactive.pop())
            .or_else(|| self.bulk.pop())
    }

    /// Number of frames currently waiting across all classes.
    pub fn queued_frames(&self) -> usize {
        self.control.len()
            + self
                .interactive
                .frames
                .values()
                .map(VecDeque::len)
                .sum::<usize>()
            + self.bulk.frames.values().map(VecDeque::len).sum::<usize>()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn data(stream_id: u32, seq: u64) -> Frame {
        Frame::data(stream_id, seq, vec![seq as u8]).unwrap()
    }

    #[test]
    fn control_and_interactive_preempt_queued_bulk() {
        let mut scheduler = Scheduler::default();
        assert!(scheduler.register_stream(1, StreamPriority::Bulk));
        assert!(scheduler.register_stream(3, StreamPriority::Interactive));
        scheduler.enqueue(data(1, 1)).unwrap();
        scheduler.enqueue(data(3, 1)).unwrap();
        scheduler
            .enqueue(Frame::new(FrameType::Ping, 0, 0, Vec::new()).unwrap())
            .unwrap();

        assert_eq!(scheduler.pop_next().unwrap().ty, FrameType::Ping);
        assert_eq!(scheduler.pop_next().unwrap().stream_id, 3);
        assert_eq!(scheduler.pop_next().unwrap().stream_id, 1);
        assert_eq!(scheduler.pop_next(), None);
    }

    #[test]
    fn streams_take_one_frame_each_turn_within_a_class() {
        let mut scheduler = Scheduler::default();
        for id in [1, 3] {
            assert!(scheduler.register_stream(id, StreamPriority::Interactive));
        }
        scheduler.enqueue(data(1, 1)).unwrap();
        scheduler.enqueue(data(1, 2)).unwrap();
        scheduler.enqueue(data(3, 1)).unwrap();

        let output: Vec<_> = std::iter::from_fn(|| scheduler.pop_next())
            .map(|frame| (frame.stream_id, frame.seq))
            .collect();
        assert_eq!(output, [(1, 1), (3, 1), (1, 2)]);
    }

    #[test]
    fn invalid_data_and_duplicate_streams_are_rejected() {
        let mut scheduler = Scheduler::default();
        assert!(!scheduler.register_stream(0, StreamPriority::Bulk));
        assert!(scheduler.register_stream(1, StreamPriority::Bulk));
        assert!(!scheduler.register_stream(1, StreamPriority::Interactive));
        let error = scheduler.enqueue(data(7, 1)).unwrap_err();
        assert_eq!(
            error.error_reason().unwrap(),
            Some(ErrorReason::ProtocolViolation)
        );
        assert_eq!(scheduler.queued_frames(), 0);
    }

    #[test]
    fn closing_stream_discards_its_pending_frames_without_affecting_others() {
        let mut scheduler = Scheduler::default();
        assert!(scheduler.register_stream(1, StreamPriority::Bulk));
        assert!(scheduler.register_stream(3, StreamPriority::Bulk));
        scheduler.enqueue(data(1, 1)).unwrap();
        scheduler.enqueue(data(3, 1)).unwrap();
        assert!(scheduler.close_stream(1));
        assert!(!scheduler.close_stream(1));
        assert_eq!(scheduler.queued_frames(), 1);
        assert_eq!(scheduler.pop_next().unwrap().stream_id, 3);
        assert_eq!(scheduler.pop_next(), None);
    }

    proptest! {
        /// Oracle: every accepted frame is emitted exactly once and each stream's
        /// sequence remains FIFO, regardless of the interleaving of enqueue calls.
        #[test]
        fn scheduler_preserves_each_stream_fifo_and_emits_every_frame(
            events in prop::collection::vec((0u8..3, 0u8..8), 0..80)
        ) {
            let mut scheduler = Scheduler::default();
            let mut expected: HashMap<u32, Vec<u64>> = HashMap::new();
            for stream_id in 1..=3 {
                scheduler.register_stream(stream_id, StreamPriority::Interactive);
            }
            for (stream, seq) in events {
                let stream_id = u32::from(stream % 3) + 1;
                let seq = u64::from(seq);
                scheduler.enqueue(data(stream_id, seq)).unwrap();
                expected.entry(stream_id).or_default().push(seq);
            }

            let mut actual: HashMap<u32, Vec<u64>> = HashMap::new();
            while let Some(frame) = scheduler.pop_next() {
                actual.entry(frame.stream_id).or_default().push(frame.seq);
            }
            prop_assert_eq!(actual, expected);
            prop_assert_eq!(scheduler.queued_frames(), 0);
        }
    }
}
