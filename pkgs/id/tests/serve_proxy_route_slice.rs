//! Integration oracles for live serve-proxy mux/leg routing (#14918).

use id::serve_proxy::{Frame, FrameType};
use id::serve_proxy_route::LiveLeg;
use id::serve_proxy_scheduler::StreamPriority;

#[test]
fn controls_preempt_data_and_priority_and_round_robin_are_composed() {
    // Oracle: the scheduler contract is control FIFO, then interactive,
    // then bulk; within a class each stream gets one frame per turn.
    let mut leg = LiveLeg::new();
    for (id, priority) in [
        (1, StreamPriority::Interactive),
        (3, StreamPriority::Interactive),
        (2, StreamPriority::Bulk),
    ] {
        assert!(leg.open_stream(id, priority));
    }
    leg.send_data(Frame::data(1, 1, vec![1]).unwrap()).unwrap();
    leg.send_data(Frame::data(1, 2, vec![2]).unwrap()).unwrap();
    leg.send_data(Frame::data(3, 1, vec![3]).unwrap()).unwrap();
    leg.send_data(Frame::data(2, 1, vec![4]).unwrap()).unwrap();
    leg.enqueue_control(Frame::new(FrameType::Ping, 0, 0, Vec::new()).unwrap())
        .unwrap();

    assert_eq!(leg.pop_next().unwrap().ty, FrameType::Ping);
    let frame = leg.pop_next().unwrap();
    assert_eq!((frame.stream_id, frame.seq), (1, 1));
    let frame = leg.pop_next().unwrap();
    assert_eq!((frame.stream_id, frame.seq), (3, 1));
    let frame = leg.pop_next().unwrap();
    assert_eq!((frame.stream_id, frame.seq), (1, 2));
    assert_eq!(leg.pop_next().unwrap().stream_id, 2);
    assert_eq!(leg.pop_next(), None);
}

#[test]
fn credit_refusal_does_not_queue_data_or_consume_connection_credit() {
    // Oracle: rejected DATA leaves mux windows untouched and is not emitted.
    let mut leg = LiveLeg::new();
    assert!(leg.open_stream(1, StreamPriority::Bulk));
    let payload = vec![0; 16 * 1024];
    for seq in 0..4 {
        leg.send_data(Frame::data(1, seq, payload.clone()).unwrap())
            .unwrap();
    }
    let before_connection = leg.connection_available();
    let error = leg
        .send_data(Frame::data(1, 4, payload).unwrap())
        .unwrap_err();
    assert_eq!(
        error.error_reason().unwrap(),
        Some(id::serve_proxy::ErrorReason::CreditOverrun)
    );
    assert_eq!(leg.stream_available(1), Some(0));
    assert_eq!(leg.connection_available(), before_connection);
    assert_eq!(leg.queued_frames(), 4);
}

#[test]
fn close_discards_scheduled_frames_and_rejects_later_data() {
    // Oracle: closing removes scheduler state and mux state together.
    let mut leg = LiveLeg::new();
    assert!(leg.open_stream(5, StreamPriority::Interactive));
    leg.send_data(Frame::data(5, 0, vec![9]).unwrap()).unwrap();
    assert!(leg.close_stream(5));
    assert!(!leg.close_stream(5));
    assert_eq!(leg.stream_available(5), None);
    assert_eq!(leg.queued_frames(), 0);
    let error = leg
        .send_data(Frame::data(5, 1, vec![10]).unwrap())
        .unwrap_err();
    assert_eq!(
        error.error_reason().unwrap(),
        Some(id::serve_proxy::ErrorReason::ProtocolViolation)
    );
}
