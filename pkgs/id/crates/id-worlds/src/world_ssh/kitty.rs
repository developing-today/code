//! Kitty keyboard activation state for one raw SSH connection.
//!
//! Slice E of #787 (`doc/2026-11-01_design_id787.md` §4.3 and §6 row E),
//! implementing the `kitty-disambiguate-v1` profile
//! (`doc/2026-11-02T_reference_kitty_disambiguate_v1.md`): push the
//! disambiguation flag, verify with the flags query before admitting any
//! active key, frame the stream with the Slice B assembler, and pop exactly
//! the owned push on teardown or fallback.
//!
//! This state exists only on a raw session whose server opted in AND whose
//! terminal reported kitty keyboard support to the F13.4 startup probe.
//! Every other session keeps the legacy chunk forwarding untouched. Token
//! bytes are never rewritten, and nothing here is journaled: probe,
//! verification and push/pop bytes are control plane only, so the durable
//! `Input(Vec<u8>)` event keeps carrying exactly the key bytes it always
//! did — now framed one complete key token per event instead of one
//! transport chunk.

use std::collections::VecDeque;
use std::time::Duration;

use crate::input_framing::{AssemblyError, InputAssembler, Limits, ReplyDisposition, ReplyFilter};

/// Push the disambiguation flag of `kitty-disambiguate-v1` (`CSI > 1 u`).
pub const KITTY_PUSH: &[u8] = b"\x1b[>1u";
/// Pop one keyboard-mode stack entry (`CSI < u`): removes exactly the push
/// above, never a mode the terminal or another client owned.
pub const KITTY_POP: &[u8] = b"\x1b[<u";
/// Ask the terminal to report its current keyboard flags (`CSI ? u`); the
/// reply is `CSI ? <flags> u`. The F13.4 probe burst already asks this once
/// for support detection; activation asks again after the push, because only
/// the post-push report proves the push took effect.
pub const KITTY_QUERY: &[u8] = b"\x1b[?u";
/// The one flag this profile requests: disambiguate escape codes.
const DISAMBIGUATE_FLAG: u32 = 1;
/// How long the terminal gets to answer the post-push flags query before
/// the session falls back to legacy input. One round trip, on the scale of
/// the F13.4 probe window (250 ms) with headroom for a loaded terminal.
pub const VERIFY_TIMEOUT: Duration = Duration::from_millis(500);

/// Where one connection sits in the activation lifecycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KittyPhase {
    /// No push outstanding; input follows the legacy path.
    Inactive,
    /// Push sent and query outstanding; assembled tokens are held, not
    /// admitted, until the flags report proves the push took effect.
    Verifying,
    /// Verified: tokens are admitted as they complete.
    Active,
}

/// Classifies the one outstanding verification reply.
///
/// Only the exact `CSI ? <flags> u` answer to our own query is consumed,
/// and only while that query is outstanding; every other token — including
/// a flags-shaped token at any other time — is application input, per the
/// design's ban on unrestricted reply filters (§4.3).
#[derive(Debug, Default)]
struct VerifyFilter {
    /// A flags query is outstanding and its answer is still due.
    outstanding: bool,
    /// The answer, once seen: whether it carried the disambiguation flag.
    answered: Option<bool>,
}

impl VerifyFilter {
    /// Parse a complete token as a kitty flags report, returning its flags.
    fn flags_report(token: &[u8]) -> Option<u32> {
        let body = token.strip_prefix(b"\x1b[?")?.strip_suffix(b"u")?;
        if body.is_empty() || !body.iter().all(u8::is_ascii_digit) {
            return None;
        }
        body.iter().try_fold(0_u32, |value, digit| {
            value.checked_mul(10)?.checked_add(u32::from(*digit - b'0'))
        })
    }
}

impl ReplyFilter for VerifyFilter {
    fn classify(&mut self, token: &[u8]) -> ReplyDisposition {
        if self.outstanding
            && let Some(flags) = Self::flags_report(token)
        {
            self.outstanding = false;
            self.answered = Some(flags & DISAMBIGUATE_FLAG != 0);
            return ReplyDisposition::Consume;
        }
        ReplyDisposition::Forward
    }
}

/// Per-connection kitty keyboard state: phase, assembler, held tokens.
///
/// Held tokens are admitted in arrival order. On a verification fallback
/// the held tokens stay queued and the caller drains them as legacy input
/// frames — their bytes are exactly what the terminal sent, so the
/// fallback loses framing, never bytes.
#[derive(Debug)]
pub struct KittyKeyboard {
    phase: KittyPhase,
    assembler: InputAssembler,
    filter: VerifyFilter,
    tokens: VecDeque<Vec<u8>>,
    /// Whether our push was written to the terminal. Tracked across
    /// fallback and teardown so exactly one pop is owed, and only then.
    pushed: bool,
}

impl Default for KittyKeyboard {
    fn default() -> Self {
        Self::new()
    }
}

impl KittyKeyboard {
    /// A fresh, inactive state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            phase: KittyPhase::Inactive,
            assembler: InputAssembler::new(Limits::default()),
            filter: VerifyFilter::default(),
            tokens: VecDeque::new(),
            pushed: false,
        }
    }

    /// The current lifecycle phase.
    #[must_use]
    pub const fn phase(&self) -> KittyPhase {
        self.phase
    }

    /// Whether the push/verify machinery is engaged (verifying or active).
    /// Inactive sessions never route input through here.
    #[must_use]
    pub const fn is_engaged(&self) -> bool {
        !matches!(self.phase, KittyPhase::Inactive)
    }

    /// Whether our push is outstanding and a pop is owed on teardown.
    #[must_use]
    pub const fn pushed(&self) -> bool {
        self.pushed
    }

    /// Held tokens are still draining (post-fallback legacy frames).
    #[must_use]
    pub fn has_held_tokens(&self) -> bool {
        !self.tokens.is_empty()
    }

    /// Begin verification after the caller has written the push and query.
    /// The caller owns the write (and its failure); this records that the
    /// push is owed a pop and that one flags answer is due.
    pub fn begin_verify(&mut self) {
        self.phase = KittyPhase::Verifying;
        self.pushed = true;
        self.filter.outstanding = true;
        self.filter.answered = None;
    }

    /// Feed one transport chunk through the assembler and the verification
    /// filter, holding every completed token in order. A flags answer that
    /// carries the disambiguation flag flips the phase to active at once;
    /// one without it is left for the caller to observe via
    /// [`Self::verify_failed`] so it can pop and fall back.
    ///
    /// # Errors
    ///
    /// Returns the assembler's fatal framing error; the caller must close
    /// the active session (a missing prefix cannot be resynchronised).
    pub fn feed(&mut self, bytes: &[u8], now: std::time::Instant) -> Result<(), AssemblyError> {
        let tokens = self.assembler.push_filtered(bytes, now, &mut self.filter)?;
        self.tokens.extend(tokens);
        if self.phase == KittyPhase::Verifying && self.filter.answered == Some(true) {
            self.phase = KittyPhase::Active;
        }
        Ok(())
    }

    /// The verification query was answered and the flags do NOT include
    /// disambiguation: the push did not take effect.
    #[must_use]
    pub const fn verify_failed(&self) -> bool {
        matches!(self.phase, KittyPhase::Verifying) && matches!(self.filter.answered, Some(false))
    }

    /// Take the next held token, in arrival order.
    pub fn take_token(&mut self) -> Option<Vec<u8>> {
        self.tokens.pop_front()
    }

    /// Fail if a partial token has outlived the assembler's deadline.
    ///
    /// # Errors
    ///
    /// Returns [`AssemblyError::IncompleteToken`] past the deadline; the
    /// caller must close the active session.
    pub fn check_token_deadline(&mut self, now: std::time::Instant) -> Result<(), AssemblyError> {
        self.assembler.check_deadline(now)
    }

    /// Whether a partial token is pending (its deadline is worth a timer).
    #[must_use]
    pub const fn has_partial_token(&self) -> bool {
        !self.assembler.is_empty()
    }

    /// End of input: fail if a token is still incomplete, so a truncated
    /// escape tail never becomes ordinary application text.
    ///
    /// # Errors
    ///
    /// Returns [`AssemblyError::IncompleteToken`] when bytes are pending.
    pub fn finish(&mut self) -> Result<(), AssemblyError> {
        self.assembler.finish()
    }

    /// Signal transport queue loss: queued bytes the assembler never saw
    /// are a fatal gap in active mode.
    pub fn queue_lost(&mut self) -> AssemblyError {
        self.assembler.queue_lost()
    }

    /// Abandon activation and return to the legacy path: the assembler and
    /// filter reset, held tokens stay queued for the caller to drain as
    /// legacy frames, and the push (if any) is handed back as still owed —
    /// the caller writes the pop. Returns whether a pop is owed.
    pub fn fallback(&mut self) -> bool {
        self.phase = KittyPhase::Inactive;
        self.assembler = InputAssembler::new(Limits::default());
        self.filter = VerifyFilter::default();
        self.pushed
    }

    /// Full reset for teardown, after the caller has written any owed pop.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Record that the owed pop has been written.
    pub fn pop_written(&mut self) {
        self.pushed = false;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn now() -> std::time::Instant {
        std::time::Instant::now()
    }

    #[test]
    fn flags_reports_parse_only_the_exact_shape() {
        assert_eq!(VerifyFilter::flags_report(b"\x1b[?0u"), Some(0));
        assert_eq!(VerifyFilter::flags_report(b"\x1b[?1u"), Some(1));
        assert_eq!(VerifyFilter::flags_report(b"\x1b[?15u"), Some(15));
        // Not flags reports: the push form, a key event, a partial token.
        assert_eq!(VerifyFilter::flags_report(b"\x1b[>1u"), None);
        assert_eq!(VerifyFilter::flags_report(b"\x1b[97;5u"), None);
        assert_eq!(VerifyFilter::flags_report(b"\x1b[?u"), None);
        assert_eq!(VerifyFilter::flags_report(b"\x1b[?1;2u"), None);
    }

    #[test]
    fn the_filter_consumes_only_the_outstanding_answer() {
        let mut filter = VerifyFilter {
            outstanding: true,
            answered: None,
        };
        // A key coalesced before the answer stays input.
        assert_eq!(filter.classify(b"x"), ReplyDisposition::Forward);
        assert_eq!(filter.classify(b"\x1b[?1u"), ReplyDisposition::Consume);
        assert_eq!(filter.answered, Some(true));
        // Nothing is outstanding now: a later flags-shaped token is input.
        assert_eq!(filter.classify(b"\x1b[?1u"), ReplyDisposition::Forward);
    }

    #[test]
    fn verification_flips_active_only_on_the_disambiguate_flag() {
        let mut kitty = KittyKeyboard::new();
        kitty.begin_verify();
        kitty.feed(b"\x1b[?1u", now()).unwrap();
        assert_eq!(kitty.phase(), KittyPhase::Active);
        // The answer itself never becomes a held token.
        assert!(!kitty.has_held_tokens());

        let mut refused = KittyKeyboard::new();
        refused.begin_verify();
        refused.feed(b"\x1b[?4u", now()).unwrap();
        assert_eq!(refused.phase(), KittyPhase::Verifying);
        assert!(refused.verify_failed());
    }

    #[test]
    fn keys_coalesced_with_the_answer_keep_their_order() {
        let mut kitty = KittyKeyboard::new();
        kitty.begin_verify();
        kitty.feed(b"a\x1b[?1u\x1b[97;5u", now()).unwrap();
        assert_eq!(kitty.phase(), KittyPhase::Active);
        assert_eq!(kitty.take_token().unwrap(), b"a".to_vec());
        assert_eq!(kitty.take_token().unwrap(), b"\x1b[97;5u".to_vec());
        assert_eq!(kitty.take_token(), None);
    }

    #[test]
    fn fallback_keeps_held_token_bytes_and_the_owed_pop() {
        let mut kitty = KittyKeyboard::new();
        kitty.begin_verify();
        kitty.feed(b"q\x1b[9", now()).unwrap();
        assert!(kitty.fallback());
        assert!(!kitty.is_engaged());
        // The complete key survives as a legacy frame; the partial escape
        // tail was assembler state and is gone with the reset.
        assert_eq!(kitty.take_token().unwrap(), b"q".to_vec());
        kitty.pop_written();
        assert!(!kitty.pushed());
    }

    #[test]
    fn split_tokens_complete_across_feeds() {
        let mut kitty = KittyKeyboard::new();
        kitty.begin_verify();
        kitty.feed(b"\x1b[?1u", now()).unwrap();
        kitty.feed(b"\x1b[97;", now()).unwrap();
        assert!(!kitty.has_held_tokens());
        assert!(kitty.has_partial_token());
        kitty.feed(b"5u", now()).unwrap();
        assert_eq!(kitty.take_token().unwrap(), b"\x1b[97;5u".to_vec());
    }
}
