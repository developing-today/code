//! Serve-proxy keepalive and idle timeouts — the keepalive slice of
//! issue #14918 (decision record #5792 §4, with the §5 resume window).
//!
//! The codec in [`super::serve_proxy`] defines PING and PONG; the legs
//! in [`super::serve_proxy_leg`] carry them. This module decides *when*
//! they are sent and when a leg is given up, as two pure state machines
//! driven by an injected clock, so every timing rule is testable with a
//! fake clock and no sleeping:
//!
//! - [`EndpointKeepalive`] (each endpoint, per leg): a PING goes out
//!   every [`PING_INTERVAL`] (25 s) with ±[`PING_JITTER_PERCENT`] (20 %)
//!   jitter whenever the interval passes with no DATA or `WINDOW_UPDATE`
//!   frame on the leg; either of those frames, in either direction,
//!   restarts the interval. A received PONG answers the outstanding
//!   PINGs. After [`MAX_UNANSWERED_PINGS`] (3) consecutive unanswered
//!   PINGs the leg is dead — about 75 s of silence — which opens a
//!   [`ResumeWindow`] rather than failing the session.
//! - [`BrokerIdle`] (broker side, per leg): a leg that receives no frame
//!   of any kind for [`BROKER_IDLE_TIMEOUT`] (120 s) is closed, and its
//!   session enters the resume window.
//!
//! [`ResumeWindow`] is the 5-minute ([`RESUME_WINDOW`]) span from leg
//! death in which RESUME is accepted (§5). Signing, replay and the
//! session token are the resumption slice; this module only says
//! whether the window is still open.
//!
//! Time is a [`Duration`] since any fixed origin the caller chooses
//! (for example `tokio::time::Instant::now() - start`); the machines
//! never read a clock themselves. Jitter comes from an injected `u32`
//! sample, so a fleet draws from its own RNG and tests pin the
//! extremes.
//!
//! Testing oracle (repo testing doctrine): with a fake clock, the PING
//! schedule stays inside 20–30 s of the last reset for every jitter
//! sample, DATA and `WINDOW_UPDATE` restart it while PING/PONG/OPEN do
//! not, the leg is dead on the deadline after the third unanswered
//! PING and not before, the broker closes at exactly 120 s of silence
//! and any frame refreshes it, and the resume window is open until
//! exactly 5 minutes after death.

use std::time::Duration;

use anyhow::Result;

use super::serve_proxy::{Frame, FrameType};

/// Base keepalive interval on a quiet leg (decision §4).
pub const PING_INTERVAL: Duration = Duration::from_millis(PING_INTERVAL_MS);

/// [`PING_INTERVAL`] in whole milliseconds, for integer jitter maths.
const PING_INTERVAL_MS: u64 = 25_000;

/// Jitter applied to [`PING_INTERVAL`], as a percentage either way, so
/// fleets of clients do not synchronize (decision §4).
pub const PING_JITTER_PERCENT: u32 = 20;

/// Consecutive unanswered PINGs after which a leg is dead (decision §4).
pub const MAX_UNANSWERED_PINGS: u32 = 3;

/// Broker-side silence after which a leg is closed (decision §4).
pub const BROKER_IDLE_TIMEOUT: Duration = Duration::from_secs(120);

/// How long after leg death RESUME is accepted (decision §5).
pub const RESUME_WINDOW: Duration = Duration::from_mins(5);

/// The jittered keepalive interval for one uniform `u32` sample.
///
/// `sample = 0` gives the shortest interval (−20 %), `u32::MAX` the
/// longest (+20 %), and the mapping is monotonic in between. Whole
/// milliseconds, so the result is exact and reproducible.
pub fn jittered_interval(sample: u32) -> Duration {
    let base = PING_INTERVAL_MS;
    let spread = base * u64::from(PING_JITTER_PERCENT) / 100;
    let low = base - spread;
    // Scale the sample onto [0, 2·spread] without floating point.
    let offset = u64::from(sample) * (2 * spread) / u64::from(u32::MAX);
    Duration::from_millis(low + offset)
}

/// A PING frame: leg-level, stream 0, empty payload.
pub fn ping_frame() -> Result<Frame> {
    Frame::new(FrameType::Ping, 0, 0, Vec::new())
}

/// A PONG frame answering a PING: leg-level, stream 0, empty payload.
pub fn pong_frame() -> Result<Frame> {
    Frame::new(FrameType::Pong, 0, 0, Vec::new())
}

/// Whether a frame of this type restarts the keepalive interval:
/// DATA and `WINDOW_UPDATE` do (decision §4); keepalive traffic itself
/// and other control frames do not.
pub const fn resets_keepalive(ty: FrameType) -> bool {
    matches!(ty, FrameType::Data | FrameType::WindowUpdate)
}

/// What an endpoint must do after [`EndpointKeepalive::poll`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeepaliveAction {
    /// Nothing is due yet.
    Wait,
    /// Send a PING now.
    SendPing,
    /// The leg is dead; resume within the returned window.
    Dead(ResumeWindow),
}

/// What an endpoint must do after receiving a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiveAction {
    /// Nothing to send.
    None,
    /// Answer with a PONG.
    SendPong,
}

/// Per-leg keepalive state on an endpoint (decision §4).
pub struct EndpointKeepalive {
    next_ping_at: Duration,
    unanswered: u32,
    dead: Option<ResumeWindow>,
    jitter: Box<dyn FnMut() -> u32 + Send>,
}

impl std::fmt::Debug for EndpointKeepalive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EndpointKeepalive")
            .field("next_ping_at", &self.next_ping_at)
            .field("unanswered", &self.unanswered)
            .field("dead", &self.dead)
            .finish_non_exhaustive()
    }
}

impl EndpointKeepalive {
    /// Starts keepalive for a leg established at `now`, drawing jitter
    /// samples from `jitter`.
    pub fn new(now: Duration, mut jitter: impl FnMut() -> u32 + Send + 'static) -> Self {
        let next_ping_at = now + jittered_interval(jitter());
        Self {
            next_ping_at,
            unanswered: 0,
            dead: None,
            jitter: Box::new(jitter),
        }
    }

    /// When [`poll`](Self::poll) next has something to do, for the
    /// caller's timer. `None` once the leg is dead.
    pub fn next_deadline(&self) -> Option<Duration> {
        self.dead.is_none().then_some(self.next_ping_at)
    }

    /// PINGs sent since the last PONG.
    pub const fn unanswered(&self) -> u32 {
        self.unanswered
    }

    /// Whether the leg has been declared dead.
    pub const fn is_dead(&self) -> bool {
        self.dead.is_some()
    }

    fn reschedule(&mut self, now: Duration) {
        self.next_ping_at = now + jittered_interval((self.jitter)());
    }

    /// Records a frame this endpoint sent on the leg.
    pub fn on_sent(&mut self, ty: FrameType, now: Duration) {
        if self.dead.is_none() && resets_keepalive(ty) {
            self.reschedule(now);
        }
    }

    /// Records a frame this endpoint received on the leg, and says
    /// whether it must be answered.
    pub fn on_received(&mut self, ty: FrameType, now: Duration) -> ReceiveAction {
        if self.dead.is_some() {
            return ReceiveAction::None;
        }
        if resets_keepalive(ty) {
            self.reschedule(now);
        }
        match ty {
            FrameType::Ping => ReceiveAction::SendPong,
            FrameType::Pong => {
                self.unanswered = 0;
                ReceiveAction::None
            }
            _ => ReceiveAction::None,
        }
    }

    /// Advances the clock to `now`: sends a PING when the interval has
    /// passed, or declares the leg dead when the interval passes with
    /// [`MAX_UNANSWERED_PINGS`] already outstanding. Once dead, every
    /// later poll returns the same [`KeepaliveAction::Dead`].
    pub fn poll(&mut self, now: Duration) -> KeepaliveAction {
        if let Some(window) = self.dead {
            return KeepaliveAction::Dead(window);
        }
        if now < self.next_ping_at {
            return KeepaliveAction::Wait;
        }
        if self.unanswered >= MAX_UNANSWERED_PINGS {
            let window = ResumeWindow::opened_at(now);
            self.dead = Some(window);
            return KeepaliveAction::Dead(window);
        }
        self.unanswered += 1;
        self.reschedule(now);
        KeepaliveAction::SendPing
    }
}

/// Broker-side idle state for one leg (decision §4): any received frame
/// keeps the leg open; [`BROKER_IDLE_TIMEOUT`] of silence closes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrokerIdle {
    last_frame_at: Duration,
    closed: Option<ResumeWindow>,
}

impl BrokerIdle {
    /// Starts idle tracking for a leg established at `now`.
    pub const fn new(now: Duration) -> Self {
        Self {
            last_frame_at: now,
            closed: None,
        }
    }

    /// Records a received frame of any type.
    pub fn on_received(&mut self, now: Duration) {
        if self.closed.is_none() && now > self.last_frame_at {
            self.last_frame_at = now;
        }
    }

    /// When the leg closes if nothing more arrives; `None` once closed.
    pub fn deadline(&self) -> Option<Duration> {
        self.closed
            .is_none()
            .then_some(self.last_frame_at + BROKER_IDLE_TIMEOUT)
    }

    /// Advances the clock to `now`. Returns the session's resume window
    /// once the leg has been silent for [`BROKER_IDLE_TIMEOUT`]; the
    /// leg is then closed and stays closed.
    pub fn poll(&mut self, now: Duration) -> Option<ResumeWindow> {
        if self.closed.is_none() && now >= self.last_frame_at + BROKER_IDLE_TIMEOUT {
            self.closed = Some(ResumeWindow::opened_at(now));
        }
        self.closed
    }
}

/// The span after leg death in which RESUME is accepted (decision §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResumeWindow {
    died_at: Duration,
}

impl ResumeWindow {
    /// The window for a leg that died at `died_at`.
    pub const fn opened_at(died_at: Duration) -> Self {
        Self { died_at }
    }

    /// When the leg died.
    pub const fn died_at(&self) -> Duration {
        self.died_at
    }

    /// The first instant at which RESUME is refused.
    pub fn expires_at(&self) -> Duration {
        self.died_at + RESUME_WINDOW
    }

    /// Whether RESUME is still accepted at `now`.
    pub fn is_open(&self, now: Duration) -> bool {
        now < self.expires_at()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serve_proxy::SchedulingClass;
    use proptest::prelude::*;

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    fn ms(m: u64) -> Duration {
        Duration::from_millis(m)
    }

    /// A keepalive whose jitter is pinned to one sample.
    fn fixed(now: Duration, sample: u32) -> EndpointKeepalive {
        EndpointKeepalive::new(now, move || sample)
    }

    #[test]
    fn jitter_spans_twenty_percent_either_way() {
        assert_eq!(jittered_interval(0), secs(20));
        assert_eq!(jittered_interval(u32::MAX), secs(30));
        let mid = jittered_interval(u32::MAX / 2);
        assert!(mid >= ms(24_999) && mid <= ms(25_001), "{mid:?}");
    }

    proptest! {
        #[test]
        fn jitter_stays_in_bounds_and_is_monotonic(a in any::<u32>(), b in any::<u32>()) {
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            let (x, y) = (jittered_interval(lo), jittered_interval(hi));
            prop_assert!(x >= secs(20) && y <= secs(30));
            prop_assert!(x <= y);
        }

        #[test]
        fn a_quiet_leg_pings_inside_the_jitter_band(sample in any::<u32>(), start in 0u64..1_000_000) {
            let t0 = ms(start);
            let mut k = fixed(t0, sample);
            let due = k.next_deadline().unwrap();
            prop_assert!(due >= t0 + secs(20) && due <= t0 + secs(30));
            prop_assert_eq!(k.poll(due - ms(1)), KeepaliveAction::Wait);
            prop_assert_eq!(k.poll(due), KeepaliveAction::SendPing);
        }
    }

    #[test]
    fn keepalive_frames_are_control_class_on_stream_zero() {
        for frame in [ping_frame().unwrap(), pong_frame().unwrap()] {
            assert_eq!(frame.stream_id, 0);
            assert_eq!(frame.seq, 0);
            assert_eq!(frame.ty.scheduling_class(), SchedulingClass::Control);
            let wire = frame.encode().unwrap();
            let (back, used) = Frame::decode_prefix(&wire).unwrap().unwrap();
            assert_eq!(used, wire.len());
            assert_eq!(back, frame);
        }
    }

    #[test]
    fn data_and_window_update_restart_the_interval_either_way() {
        let mut k = fixed(secs(0), 0); // 20 s interval
        assert_eq!(k.next_deadline(), Some(secs(20)));
        k.on_sent(FrameType::Data, secs(15));
        assert_eq!(k.next_deadline(), Some(secs(35)));
        assert_eq!(k.poll(secs(34)), KeepaliveAction::Wait);
        k.on_received(FrameType::WindowUpdate, secs(30));
        assert_eq!(k.next_deadline(), Some(secs(50)));
        k.on_received(FrameType::Data, secs(45));
        k.on_sent(FrameType::WindowUpdate, secs(46));
        assert_eq!(k.next_deadline(), Some(secs(66)));
        assert_eq!(k.poll(secs(66)), KeepaliveAction::SendPing);
    }

    #[test]
    fn keepalive_and_other_control_frames_do_not_restart_it() {
        let mut k = fixed(secs(0), 0);
        for ty in [
            FrameType::Ping,
            FrameType::Pong,
            FrameType::Open,
            FrameType::Close,
            FrameType::Resume,
            FrameType::ResumeAck,
            FrameType::Error,
        ] {
            k.on_sent(ty, secs(10));
            k.on_received(ty, secs(11));
        }
        assert_eq!(k.next_deadline(), Some(secs(20)));
    }

    #[test]
    fn a_ping_is_answered_and_a_pong_clears_the_count() {
        let mut k = fixed(secs(0), 0);
        assert_eq!(
            k.on_received(FrameType::Ping, secs(1)),
            ReceiveAction::SendPong
        );
        assert_eq!(k.poll(secs(20)), KeepaliveAction::SendPing);
        assert_eq!(k.poll(secs(40)), KeepaliveAction::SendPing);
        assert_eq!(k.unanswered(), 2);
        assert_eq!(
            k.on_received(FrameType::Pong, secs(41)),
            ReceiveAction::None
        );
        assert_eq!(k.unanswered(), 0);
        // The count starts over: three more unanswered PINGs are needed.
        assert_eq!(k.poll(secs(60)), KeepaliveAction::SendPing);
        assert_eq!(k.poll(secs(80)), KeepaliveAction::SendPing);
        assert_eq!(k.poll(secs(100)), KeepaliveAction::SendPing);
        assert!(!k.is_dead());
    }

    #[test]
    fn three_unanswered_pings_kill_the_leg_on_the_next_deadline() {
        let mut k = fixed(secs(0), u32::MAX / 2); // ~25 s interval
        let mut pings = Vec::new();
        for _ in 0..MAX_UNANSWERED_PINGS {
            let due = k.next_deadline().unwrap();
            assert_eq!(k.poll(due), KeepaliveAction::SendPing);
            pings.push(due);
        }
        let due = k.next_deadline().unwrap();
        assert_eq!(k.poll(due - ms(1)), KeepaliveAction::Wait);
        let window = ResumeWindow::opened_at(due);
        assert_eq!(k.poll(due), KeepaliveAction::Dead(window));
        // About 75 s of silence after the first PING, as §4 states.
        let silence = due - pings[0];
        assert!(
            silence >= ms(74_990) && silence <= ms(75_010),
            "{silence:?}"
        );
        // Dead is final.
        assert!(k.is_dead());
        assert_eq!(k.next_deadline(), None);
        assert_eq!(
            k.on_received(FrameType::Pong, due + secs(1)),
            ReceiveAction::None
        );
        assert_eq!(k.poll(due + secs(500)), KeepaliveAction::Dead(window));
    }

    proptest! {
        #[test]
        fn death_lands_between_sixty_and_ninety_seconds_after_the_first_ping(
            samples in proptest::collection::vec(any::<u32>(), 4),
        ) {
            let mut it = samples.clone().into_iter().cycle();
            let mut k = EndpointKeepalive::new(secs(0), move || it.next().unwrap());
            let first = k.next_deadline().unwrap();
            prop_assert_eq!(k.poll(first), KeepaliveAction::SendPing);
            for _ in 1..MAX_UNANSWERED_PINGS {
                let due = k.next_deadline().unwrap();
                prop_assert_eq!(k.poll(due), KeepaliveAction::SendPing);
            }
            let due = k.next_deadline().unwrap();
            prop_assert!(matches!(k.poll(due), KeepaliveAction::Dead(_)));
            let silence = due - first;
            prop_assert!(silence >= secs(60) && silence <= secs(90));
        }
    }

    #[test]
    fn the_broker_closes_a_leg_silent_for_exactly_120_seconds() {
        let mut b = BrokerIdle::new(secs(10));
        assert_eq!(b.deadline(), Some(secs(130)));
        assert_eq!(b.poll(secs(130) - ms(1)), None);
        let window = b.poll(secs(130)).unwrap();
        assert_eq!(window.died_at(), secs(130));
        // Closed stays closed; a late frame does not reopen it.
        b.on_received(secs(131));
        assert_eq!(b.poll(secs(131)), Some(window));
        assert_eq!(b.deadline(), None);
    }

    #[test]
    fn any_frame_keeps_a_broker_leg_open() {
        let mut b = BrokerIdle::new(secs(0));
        for t in (100..=1_000).step_by(100) {
            b.on_received(secs(t));
            assert_eq!(b.poll(secs(t) + secs(119)), None);
        }
        assert_eq!(b.deadline(), Some(secs(1_120)));
        // A frame stamped earlier than the latest one never moves the
        // deadline back.
        b.on_received(secs(500));
        assert_eq!(b.deadline(), Some(secs(1_120)));
    }

    #[test]
    fn the_resume_window_is_open_for_exactly_five_minutes() {
        let w = ResumeWindow::opened_at(secs(1_000));
        assert_eq!(w.expires_at(), secs(1_300));
        assert!(w.is_open(secs(1_000)));
        assert!(w.is_open(secs(1_300) - ms(1)));
        assert!(!w.is_open(secs(1_300)));
    }
}
