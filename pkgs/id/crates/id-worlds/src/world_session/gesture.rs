//! Producer-side touch gesture translation for the Android environment
//! (#1831 slice S3a; input event model design
//! `doc/2026-11-08T05-48-47Z_design_input_event_model`).
//!
//! Slice S2 defined the pointer model host-side: the `pointer` frame,
//! the per-environment refusal table, and the rule that an input event
//! is never silently dropped. Nothing produced those frames — no code
//! synthesized `long_press`, measured a release's `held_duration_ms`,
//! classified `is_click`, or mapped a touchscreen gesture onto the
//! model. This module is that producer for the environment the S3
//! proof-of-concept Android app speaks. It is a pure state machine
//! over raw touch samples in surface cells on a monotonic millisecond
//! clock, so the app shell and these tests drive it identically; the
//! shell maps device pixels to cells at its edge, owns the real
//! timers, and must call [`GestureTranslator::advance_time`] on its
//! timer so a stationary hold can synthesize `long_press`.
//!
//! The mapping is the design's Decision 2, implemented as written:
//! tap is `press` + `release` (the app derives the click from the
//! release's `is_click`), a stationary hold past the long-press
//! threshold synthesizes `long_press` — Android's right-click
//! equivalent — touch drag is the `press` / `drag_move` / `release`
//! sequence (one `drag_move` per cell change, in order), and pan/fling
//! is `scroll`. Gestures outside the v1 subset are refused explicitly,
//! producer-side, with the design's reasons, so the app can surface
//! the refusal instead of the user tapping and watching nothing
//! happen: a second simultaneous contact (`multi_touch_deferred`,
//! Decision 4), pinch (`gesture_not_in_subset`), stylus hover
//! (`hover_unsupported`), and a secondary button
//! (`no_secondary_button`).

use super::types::{
    ClientFrame, InputEnvironment, InputRefusalReason, PointerButton, PointerEvent,
};

/// Click threshold in milliseconds: a stationary release held for
/// less than this is classified a click. A project default named by
/// the design, not a platform standard.
pub(crate) const CLICK_THRESHOLD_MS: u64 = 250;

/// Long-press threshold in milliseconds: the Android platform
/// default, named by the design.
pub(crate) const LONG_PRESS_THRESHOLD_MS: u64 = 500;

/// The pointer id a contact-free `scroll` (pan/fling) names: the
/// model's primary pointer.
const PRIMARY_POINTER_ID: u32 = 0;

/// One raw touch sample at the Android producer's edge. Coordinates
/// are surface cells — the shell has already mapped device pixels —
/// and `at_ms` is the shell's monotonic clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TouchInput {
    /// A contact touched down.
    Down {
        pointer_id: u32,
        x: u16,
        y: u16,
        at_ms: u64,
    },
    /// The active contact moved.
    Move { pointer_id: u32, x: u16, y: u16 },
    /// The active contact lifted.
    Up {
        pointer_id: u32,
        x: u16,
        y: u16,
        at_ms: u64,
    },
    /// The pointer stream was interrupted (OS gesture takeover,
    /// producer-side `pointercancel`).
    Cancel { pointer_id: u32 },
    /// A pan/fling delta not attached to a contact (the shell turns a
    /// fling's velocity into continued deltas, per the design).
    Pan { x: u16, y: u16, dx: i64, dy: i64 },
    /// A pinch gesture started: outside the v1 subset.
    PinchStart,
    /// A stylus hover sample: Android refuses hover in the model.
    StylusHover { x: u16, y: u16 },
    /// A secondary (right) button press: Android has no secondary
    /// button; `long_press` is its equivalent.
    SecondaryDown { x: u16, y: u16 },
}

/// One output of the translator, in the order the shell must act on
/// it.
#[derive(Debug)]
pub(crate) enum GestureOutput {
    /// A wire frame for the host.
    Frame(ClientFrame),
    /// A gesture the v1 subset refuses, surfaced producer-side with
    /// the design's reason. `gesture` names what was refused in the
    /// design's vocabulary.
    Refused {
        gesture: &'static str,
        reason: InputRefusalReason,
    },
}

/// The press currently in flight, if any. v1 guarantees only the
/// primary pointer (design Decision 4), so at most one press is ever
/// active.
#[derive(Clone, Copy, Debug)]
struct ActivePress {
    pointer_id: u32,
    down_x: u16,
    down_y: u16,
    last_x: u16,
    last_y: u16,
    down_at_ms: u64,
    moved: bool,
    long_press_sent: bool,
}

/// Translates raw Android touch samples into pointer frames. Inert
/// until the S3 app shell wires it to a live session (slice S3b);
/// the translation contract is proven here, ahead of the shell, the
/// same way `input_framing` was proven ahead of its activation.
#[derive(Clone, Debug)]
pub(crate) struct GestureTranslator {
    click_threshold_ms: u64,
    long_press_threshold_ms: u64,
    active: Option<ActivePress>,
}

impl Default for GestureTranslator {
    fn default() -> Self {
        Self::new()
    }
}

impl GestureTranslator {
    /// A translator with the design's default thresholds.
    pub(crate) fn new() -> Self {
        Self::with_thresholds(CLICK_THRESHOLD_MS, LONG_PRESS_THRESHOLD_MS)
    }

    /// A translator with explicit thresholds (the shell may adopt
    /// platform-reported values; the defaults are the design's).
    pub(crate) fn with_thresholds(click_threshold_ms: u64, long_press_threshold_ms: u64) -> Self {
        Self {
            click_threshold_ms,
            long_press_threshold_ms,
            active: None,
        }
    }

    /// Feed one raw touch sample; returns the frames and refusals it
    /// produces, in order (usually zero or one).
    pub(crate) fn handle(&mut self, input: TouchInput) -> Vec<GestureOutput> {
        match input {
            TouchInput::Down {
                pointer_id,
                x,
                y,
                at_ms,
            } => {
                if self.active.is_some() {
                    // Decision 4: v1 guarantees only the primary
                    // pointer; a second simultaneous contact is
                    // refused, never merged into the active press.
                    return vec![GestureOutput::Refused {
                        gesture: "press",
                        reason: InputRefusalReason::MultiTouchDeferred,
                    }];
                }
                self.active = Some(ActivePress {
                    pointer_id,
                    down_x: x,
                    down_y: y,
                    last_x: x,
                    last_y: y,
                    down_at_ms: at_ms,
                    moved: false,
                    long_press_sent: false,
                });
                vec![GestureOutput::Frame(pointer_frame(
                    PointerEvent::Press,
                    pointer_id,
                    x,
                    y,
                ))]
            }
            TouchInput::Move { pointer_id, x, y } => {
                let Some(active) = self.active.as_mut() else {
                    return Vec::new();
                };
                if active.pointer_id != pointer_id || (x == active.last_x && y == active.last_y) {
                    // A non-active contact's motion belongs to a
                    // contact v1 already refused; a same-cell move
                    // changes nothing. Neither is an event.
                    return Vec::new();
                }
                active.last_x = x;
                active.last_y = y;
                active.moved = true;
                vec![GestureOutput::Frame(pointer_frame(
                    PointerEvent::DragMove,
                    pointer_id,
                    x,
                    y,
                ))]
            }
            TouchInput::Up {
                pointer_id,
                x,
                y,
                at_ms,
            } => {
                let Some(active) = self.active else {
                    return Vec::new();
                };
                if active.pointer_id != pointer_id {
                    return Vec::new();
                }
                self.active = None;
                // The producer measures the hold and resolves the
                // click classification so replay derives the same
                // click semantic without a wall clock (design
                // Decision 3). The classification bands are
                // exhaustive: a stationary hold at or past the click
                // threshold but below the long-press threshold is a
                // deliberate slow press — a plain release, not a
                // click and not a long press.
                let held_duration_ms = at_ms.saturating_sub(active.down_at_ms);
                let is_click = !active.moved && held_duration_ms < self.click_threshold_ms;
                vec![GestureOutput::Frame(ClientFrame::Pointer {
                    event: PointerEvent::Release,
                    environment: InputEnvironment::Android,
                    pointer_id,
                    x,
                    y,
                    button: Some(PointerButton::Primary),
                    held_duration_ms: Some(held_duration_ms),
                    is_click: Some(is_click),
                    dx: None,
                    dy: None,
                    screen_revision: None,
                })]
            }
            TouchInput::Cancel { pointer_id } => {
                let Some(active) = self.active else {
                    return Vec::new();
                };
                if active.pointer_id != pointer_id {
                    return Vec::new();
                }
                self.active = None;
                vec![GestureOutput::Frame(pointer_frame(
                    PointerEvent::Cancel,
                    pointer_id,
                    active.last_x,
                    active.last_y,
                ))]
            }
            TouchInput::Pan { x, y, dx, dy } => vec![GestureOutput::Frame(ClientFrame::Pointer {
                event: PointerEvent::Scroll,
                environment: InputEnvironment::Android,
                pointer_id: PRIMARY_POINTER_ID,
                x,
                y,
                button: None,
                held_duration_ms: None,
                is_click: None,
                dx: Some(dx),
                dy: Some(dy),
                screen_revision: None,
            })],
            TouchInput::PinchStart => vec![GestureOutput::Refused {
                gesture: "pinch",
                reason: InputRefusalReason::GestureNotInSubset,
            }],
            TouchInput::StylusHover { .. } => vec![GestureOutput::Refused {
                gesture: "hover",
                reason: InputRefusalReason::HoverUnsupported,
            }],
            TouchInput::SecondaryDown { .. } => vec![GestureOutput::Refused {
                gesture: "secondary_press",
                reason: InputRefusalReason::NoSecondaryButton,
            }],
        }
    }

    /// Advance the producer's clock. A press that has stayed in its
    /// cell for the long-press threshold synthesizes `long_press`
    /// exactly once; the shell calls this on its timer while a press
    /// is open.
    pub(crate) fn advance_time(&mut self, now_ms: u64) -> Vec<GestureOutput> {
        let Some(active) = self.active.as_mut() else {
            return Vec::new();
        };
        if active.moved || active.long_press_sent {
            return Vec::new();
        }
        if now_ms.saturating_sub(active.down_at_ms) < self.long_press_threshold_ms {
            return Vec::new();
        }
        active.long_press_sent = true;
        vec![GestureOutput::Frame(pointer_frame(
            PointerEvent::LongPress,
            active.pointer_id,
            active.down_x,
            active.down_y,
        ))]
    }

    /// The producer is going away. A press still open ends with a
    /// synthesized `cancel`, so no pressed state can leak into
    /// replay; if the connection drops before this frame flushes,
    /// the host synthesizes the same `cancel` for the disconnect —
    /// whichever arrives first ends the gesture.
    pub(crate) fn disconnect(&mut self) -> Vec<GestureOutput> {
        let Some(active) = self.active.take() else {
            return Vec::new();
        };
        vec![GestureOutput::Frame(pointer_frame(
            PointerEvent::Cancel,
            active.pointer_id,
            active.last_x,
            active.last_y,
        ))]
    }
}

/// A pointer frame carrying only the fields every activation event
/// shares; `release` and `scroll` add their own fields at their
/// construction sites.
fn pointer_frame(event: PointerEvent, pointer_id: u32, x: u16, y: u16) -> ClientFrame {
    ClientFrame::Pointer {
        event,
        environment: InputEnvironment::Android,
        pointer_id,
        x,
        y,
        button: None,
        held_duration_ms: None,
        is_click: None,
        dx: None,
        dy: None,
        screen_revision: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fields of one pointer frame, destructured for assertion.
    /// Every frame this translator emits must name the Android
    /// environment — that is asserted here, once, for all tests.
    #[allow(clippy::type_complexity)]
    fn pointer_fields(
        frame: ClientFrame,
    ) -> (
        PointerEvent,
        u32,
        u16,
        u16,
        Option<PointerButton>,
        Option<u64>,
        Option<bool>,
        Option<i64>,
        Option<i64>,
    ) {
        match frame {
            ClientFrame::Pointer {
                event,
                environment,
                pointer_id,
                x,
                y,
                button,
                held_duration_ms,
                is_click,
                dx,
                dy,
                ..
            } => {
                assert_eq!(environment, InputEnvironment::Android);
                (
                    event,
                    pointer_id,
                    x,
                    y,
                    button,
                    held_duration_ms,
                    is_click,
                    dx,
                    dy,
                )
            }
            other => panic!("expected a pointer frame, got {other:?}"),
        }
    }

    fn only_frame(outputs: Vec<GestureOutput>) -> ClientFrame {
        assert_eq!(outputs.len(), 1, "expected one output, got {outputs:?}");
        match outputs.into_iter().next().unwrap() {
            GestureOutput::Frame(frame) => frame,
            other => panic!("expected a frame, got {other:?}"),
        }
    }

    fn only_refusal(outputs: Vec<GestureOutput>) -> (&'static str, InputRefusalReason) {
        assert_eq!(outputs.len(), 1, "expected one output, got {outputs:?}");
        match outputs.into_iter().next().unwrap() {
            GestureOutput::Refused { gesture, reason } => (gesture, reason),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    fn down(translator: &mut GestureTranslator, pointer_id: u32, x: u16, y: u16, at_ms: u64) {
        let frame = only_frame(translator.handle(TouchInput::Down {
            pointer_id,
            x,
            y,
            at_ms,
        }));
        let (event, id, fx, fy, ..) = pointer_fields(frame);
        assert_eq!((event, id, fx, fy), (PointerEvent::Press, pointer_id, x, y));
    }

    // Oracle: design Decision 2 — tap is press + release, and the
    // app derives click from the release's is_click.
    #[test]
    fn tap_is_press_then_release_classified_as_click() {
        let mut translator = GestureTranslator::new();
        down(&mut translator, 0, 3, 4, 1_000);
        let frame = only_frame(translator.handle(TouchInput::Up {
            pointer_id: 0,
            x: 3,
            y: 4,
            at_ms: 1_100,
        }));
        let (event, _, x, y, button, held, is_click, ..) = pointer_fields(frame);
        assert_eq!(event, PointerEvent::Release);
        assert_eq!((x, y), (3, 4));
        assert_eq!(button, Some(PointerButton::Primary));
        assert_eq!(held, Some(100));
        assert_eq!(is_click, Some(true));
    }

    // Oracle: design "The common core" — the click band is
    // exhaustive; a release at exactly the click threshold is not a
    // click.
    #[test]
    fn release_at_the_click_threshold_is_not_a_click() {
        let mut translator = GestureTranslator::new();
        down(&mut translator, 0, 1, 1, 0);
        let frame = only_frame(translator.handle(TouchInput::Up {
            pointer_id: 0,
            x: 1,
            y: 1,
            at_ms: CLICK_THRESHOLD_MS,
        }));
        let (_, _, _, _, _, held, is_click, ..) = pointer_fields(frame);
        assert_eq!(held, Some(CLICK_THRESHOLD_MS));
        assert_eq!(is_click, Some(false));
    }

    // Oracle: design "The common core" — a stationary hold between
    // the click and long-press thresholds is a deliberate slow press:
    // a plain release, not a click and not a long press.
    #[test]
    fn slow_press_band_is_a_plain_release() {
        let mut translator = GestureTranslator::new();
        down(&mut translator, 0, 2, 2, 0);
        let frame = only_frame(translator.handle(TouchInput::Up {
            pointer_id: 0,
            x: 2,
            y: 2,
            at_ms: 300,
        }));
        let (event, _, _, _, _, held, is_click, ..) = pointer_fields(frame);
        assert_eq!(event, PointerEvent::Release);
        assert_eq!(held, Some(300));
        assert_eq!(is_click, Some(false));
    }

    // Oracle: design Decision 2 — long press (>= 500 ms, no move) is
    // its own synthesized event, delivered once.
    #[test]
    fn stationary_hold_synthesizes_long_press_at_the_threshold() {
        let mut translator = GestureTranslator::new();
        down(&mut translator, 0, 5, 5, 0);
        assert!(translator.advance_time(499).is_empty());
        let frame = only_frame(translator.advance_time(500));
        let (event, _, x, y, ..) = pointer_fields(frame);
        assert_eq!(event, PointerEvent::LongPress);
        assert_eq!((x, y), (5, 5));
        // Exactly once, and the gesture still ends with a release
        // whose classification is not a click.
        assert!(translator.advance_time(900).is_empty());
        let frame = only_frame(translator.handle(TouchInput::Up {
            pointer_id: 0,
            x: 5,
            y: 5,
            at_ms: 1_000,
        }));
        let (_, _, _, _, _, held, is_click, ..) = pointer_fields(frame);
        assert_eq!(held, Some(1_000));
        assert_eq!(is_click, Some(false));
    }

    // Oracle: design Decision 2 — long press requires no move; a
    // press that became a drag never synthesizes one.
    #[test]
    fn movement_before_the_threshold_suppresses_long_press() {
        let mut translator = GestureTranslator::new();
        down(&mut translator, 0, 1, 1, 0);
        let _ = only_frame(translator.handle(TouchInput::Move {
            pointer_id: 0,
            x: 2,
            y: 1,
        }));
        assert!(translator.advance_time(600).is_empty());
    }

    // Oracle: design "The common core" — a drag is delivered as
    // press, one drag_move per cell change in order, and release.
    #[test]
    fn drag_emits_one_drag_move_per_cell_change_in_order() {
        let mut translator = GestureTranslator::new();
        down(&mut translator, 0, 1, 1, 0);
        // Same-cell motion is not a move.
        assert!(
            translator
                .handle(TouchInput::Move {
                    pointer_id: 0,
                    x: 1,
                    y: 1
                })
                .is_empty()
        );
        let frame = only_frame(translator.handle(TouchInput::Move {
            pointer_id: 0,
            x: 2,
            y: 1,
        }));
        let (event, _, x, y, ..) = pointer_fields(frame);
        assert_eq!((event, x, y), (PointerEvent::DragMove, 2, 1));
        let frame = only_frame(translator.handle(TouchInput::Move {
            pointer_id: 0,
            x: 2,
            y: 3,
        }));
        let (event, _, x, y, ..) = pointer_fields(frame);
        assert_eq!((event, x, y), (PointerEvent::DragMove, 2, 3));
        // A release after motion is never a click.
        let frame = only_frame(translator.handle(TouchInput::Up {
            pointer_id: 0,
            x: 2,
            y: 3,
            at_ms: 120,
        }));
        let (_, _, _, _, _, _, is_click, ..) = pointer_fields(frame);
        assert_eq!(is_click, Some(false));
    }

    // Oracle: design Decision 2 — pan/fling maps to scroll with the
    // deltas carried on the frame.
    #[test]
    fn pan_emits_scroll_with_deltas() {
        let mut translator = GestureTranslator::new();
        let frame = only_frame(translator.handle(TouchInput::Pan {
            x: 5,
            y: 5,
            dx: 0,
            dy: -3,
        }));
        let (event, pointer_id, x, y, _, _, _, dx, dy) = pointer_fields(frame);
        assert_eq!(event, PointerEvent::Scroll);
        assert_eq!(pointer_id, PRIMARY_POINTER_ID);
        assert_eq!((x, y), (5, 5));
        assert_eq!((dx, dy), (Some(0), Some(-3)));
    }

    // Oracle: design Decision 4 — a second simultaneous contact is
    // refused multi_touch_deferred; the primary press is undisturbed.
    #[test]
    fn second_contact_is_refused_and_the_primary_press_survives() {
        let mut translator = GestureTranslator::new();
        down(&mut translator, 0, 1, 1, 0);
        let (gesture, reason) = only_refusal(translator.handle(TouchInput::Down {
            pointer_id: 1,
            x: 6,
            y: 6,
            at_ms: 10,
        }));
        assert_eq!(gesture, "press");
        assert_eq!(reason, InputRefusalReason::MultiTouchDeferred);
        // The refused contact's motion and lift produce nothing.
        assert!(
            translator
                .handle(TouchInput::Move {
                    pointer_id: 1,
                    x: 7,
                    y: 6
                })
                .is_empty()
        );
        assert!(
            translator
                .handle(TouchInput::Up {
                    pointer_id: 1,
                    x: 7,
                    y: 6,
                    at_ms: 50
                })
                .is_empty()
        );
        // The primary press still completes as a click.
        let frame = only_frame(translator.handle(TouchInput::Up {
            pointer_id: 0,
            x: 1,
            y: 1,
            at_ms: 60,
        }));
        let (_, _, _, _, _, _, is_click, ..) = pointer_fields(frame);
        assert_eq!(is_click, Some(true));
    }

    // Oracle: design refusal table — pinch is refused
    // gesture_not_in_subset in the Android column.
    #[test]
    fn pinch_is_refused_gesture_not_in_subset() {
        let mut translator = GestureTranslator::new();
        let (gesture, reason) = only_refusal(translator.handle(TouchInput::PinchStart));
        assert_eq!(gesture, "pinch");
        assert_eq!(reason, InputRefusalReason::GestureNotInSubset);
    }

    // Oracle: design refusal table — Android hover is refused
    // hover_unsupported.
    #[test]
    fn stylus_hover_is_refused_hover_unsupported() {
        let mut translator = GestureTranslator::new();
        let (gesture, reason) =
            only_refusal(translator.handle(TouchInput::StylusHover { x: 1, y: 1 }));
        assert_eq!(gesture, "hover");
        assert_eq!(reason, InputRefusalReason::HoverUnsupported);
    }

    // Oracle: design Decision 2 — Android has no right click; a
    // secondary press is refused no_secondary_button and long_press
    // is the equivalent.
    #[test]
    fn secondary_down_is_refused_no_secondary_button() {
        let mut translator = GestureTranslator::new();
        let (gesture, reason) =
            only_refusal(translator.handle(TouchInput::SecondaryDown { x: 1, y: 1 }));
        assert_eq!(gesture, "secondary_press");
        assert_eq!(reason, InputRefusalReason::NoSecondaryButton);
    }

    // Oracle: design "The common core" — cancel ends the gesture and
    // is never a click; nothing of the press remains afterwards.
    #[test]
    fn cancel_ends_the_gesture() {
        let mut translator = GestureTranslator::new();
        down(&mut translator, 0, 4, 4, 0);
        let _ = only_frame(translator.handle(TouchInput::Move {
            pointer_id: 0,
            x: 5,
            y: 4,
        }));
        let frame = only_frame(translator.handle(TouchInput::Cancel { pointer_id: 0 }));
        let (event, _, x, y, ..) = pointer_fields(frame);
        assert_eq!(event, PointerEvent::Cancel);
        // The cancel lands at the last known cell.
        assert_eq!((x, y), (5, 4));
        assert!(
            translator
                .handle(TouchInput::Up {
                    pointer_id: 0,
                    x: 5,
                    y: 4,
                    at_ms: 80
                })
                .is_empty()
        );
    }

    // Oracle: design "The common core" — when a producer disconnects
    // with a press still open, a cancel ends the gesture so no
    // pressed state can leak into replay.
    #[test]
    fn disconnect_with_an_open_press_synthesizes_cancel() {
        let mut translator = GestureTranslator::new();
        down(&mut translator, 0, 2, 2, 0);
        let frame = only_frame(translator.disconnect());
        let (event, _, x, y, ..) = pointer_fields(frame);
        assert_eq!(event, PointerEvent::Cancel);
        assert_eq!((x, y), (2, 2));
        assert!(translator.disconnect().is_empty());
    }
}
