//! Bounded terminal input token assembly for raw world connections.
//!
//! Slice B of #787 (`doc/2026-11-01_design_id787.md` §4.2 and §6 row B). A raw
//! transport delivers arbitrary byte chunks, but a key decoder must only ever
//! see complete logical key tokens. This module retains an incomplete UTF-8
//! scalar, CSI sequence or SS3 sequence across chunk boundaries and emits one
//! token per complete key, independent of how the transport split the stream.
//! Plain text emits one token per Unicode scalar.
//!
//! The module is deliberately **inert**: `world_ssh` does not call it, the
//! durable `Input(Vec<u8>)` event is unchanged and no transport behaviour moves
//! yet. Slice E owns activation after F13.4 supplies detection and the reply
//! demultiplexer; [`ReplyFilter`] is the interface that demultiplexer plugs into.
//!
//! Token bytes are never rewritten: what the assembler emits is exactly the
//! bytes the transport received. Framing failures are fatal by design, because
//! continuing after a missing prefix cannot resynchronise the stream. The
//! caller must diagnose the error and close the active session.

use std::time::{Duration, Instant};

/// Largest single pending key token, in bytes.
///
/// Real kitty, CSI and SS3 forms are far below this; the cap bounds memory to
/// O(connections). Activation must also keep this at or below the world's
/// `input_bytes` limit.
pub const MAX_TOKEN_BYTES: usize = 256;

/// How long an incomplete token may stay pending before it is fatal.
pub const INCOMPLETE_TOKEN_DEADLINE: Duration = Duration::from_secs(5);

const ESC: u8 = 0x1b;
const CSI: u8 = b'[';
const SS3: u8 = b'O';

/// Assembly limits for one connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Maximum size of one pending token, inclusive.
    pub max_token_bytes: usize,
    /// Maximum age of an incomplete token before it is fatal.
    pub incomplete_deadline: Duration,
}

impl Limits {
    /// Build limits explicitly.
    ///
    /// A zero cap or deadline fails closed on the first pending token; callers
    /// should normally use [`Limits::default`].
    #[must_use]
    pub const fn new(max_token_bytes: usize, incomplete_deadline: Duration) -> Self {
        Self {
            max_token_bytes,
            incomplete_deadline,
        }
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self::new(MAX_TOKEN_BYTES, INCOMPLETE_TOKEN_DEADLINE)
    }
}

/// Why an incomplete token was rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IncompleteReason {
    /// The transport reached end of input with a token still pending.
    Eof,
    /// The token stayed pending longer than the configured deadline.
    Deadline,
}

/// A fatal framing error. There is no safe way to resume after any variant:
/// the active session must be diagnosed and closed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssemblyError {
    /// An escape sequence is structurally invalid (bad parameter/final byte or
    /// a second escape where a sequence body belongs).
    MalformedEscape {
        /// Bytes of the token when the fault was detected.
        token: Vec<u8>,
    },
    /// A UTF-8 scalar was truncated, overlong or otherwise invalid.
    InvalidUtf8 {
        /// Bytes of the token when the fault was detected.
        token: Vec<u8>,
    },
    /// One token would exceed [`Limits::max_token_bytes`].
    TokenTooLong {
        /// The configured cap.
        limit: usize,
    },
    /// End of input or the deadline arrived while a token was still incomplete.
    IncompleteToken {
        /// The bytes that were pending.
        token: Vec<u8>,
        /// Which boundary ended the wait.
        reason: IncompleteReason,
    },
    /// The transport dropped queued bytes the assembler never saw.
    QueueLoss,
}

impl std::fmt::Display for AssemblyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MalformedEscape { token } => {
                write!(formatter, "malformed escape token ({} bytes)", token.len())
            }
            Self::InvalidUtf8 { token } => {
                write!(
                    formatter,
                    "invalid UTF-8 input token ({} bytes)",
                    token.len()
                )
            }
            Self::TokenTooLong { limit } => {
                write!(formatter, "input token exceeds {limit} bytes")
            }
            Self::IncompleteToken { token, reason } => {
                let boundary = match reason {
                    IncompleteReason::Eof => "end of input",
                    IncompleteReason::Deadline => "incomplete-token deadline",
                };
                write!(
                    formatter,
                    "incomplete input token ({} bytes) at {boundary}",
                    token.len()
                )
            }
            Self::QueueLoss => formatter.write_str("transport lost queued input bytes"),
        }
    }
}

impl std::error::Error for AssemblyError {}

/// What a control-reply filter decides for one complete token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplyDisposition {
    /// A recognized reply to an outstanding probe: consume it, never forward it
    /// as application key input.
    Consume,
    /// Application key input: forward it unchanged.
    Forward,
}

/// Connection-local control-reply classifier interface.
///
/// Slice B defines only the seam. F13.4's demultiplexer implements it with
/// bounded state for its own outstanding probes; an implementation must return
/// [`ReplyDisposition::Forward`] for anything it does not positively recognize
/// (an unrestricted filter that swallows arbitrary keys is forbidden by §4.3).
pub trait ReplyFilter {
    /// Classify one complete token.
    fn classify(&mut self, token: &[u8]) -> ReplyDisposition;
}

/// Per-connection assembler state.
///
/// Create one per active raw connection. Pending state is never shared, so
/// interleaved connections cannot mix partial tokens.
#[derive(Debug)]
pub struct InputAssembler {
    limits: Limits,
    pending: Vec<u8>,
    pending_since: Option<Instant>,
}

impl InputAssembler {
    /// Create an assembler with explicit limits.
    #[must_use]
    pub fn new(limits: Limits) -> Self {
        Self {
            limits,
            pending: Vec::new(),
            pending_since: None,
        }
    }

    /// Feed one transport chunk, returning every token it completed in order.
    ///
    /// A trailing partial token stays pending for the next call. The function
    /// fails on a malformed token, an invalid UTF-8 scalar, a token over the
    /// cap, or a pending token older than the deadline.
    pub fn push(&mut self, bytes: &[u8], now: Instant) -> Result<Vec<Vec<u8>>, AssemblyError> {
        self.check_deadline(now)?;
        let mut complete = Vec::new();
        for &byte in bytes {
            if self.pending.is_empty() {
                self.pending_since = Some(now);
            }
            self.pending.push(byte);
            if let Some(length) = self.scan()? {
                if length > self.limits.max_token_bytes {
                    return Err(AssemblyError::TokenTooLong {
                        limit: self.limits.max_token_bytes,
                    });
                }
                complete.push(std::mem::take(&mut self.pending));
                self.pending_since = None;
            } else if self.pending.len() > self.limits.max_token_bytes {
                return Err(AssemblyError::TokenTooLong {
                    limit: self.limits.max_token_bytes,
                });
            }
        }
        Ok(complete)
    }

    /// Feed a chunk and drop recognized control replies before returning keys.
    ///
    /// The surviving tokens keep their original order, so a reply coalesced
    /// with keys cannot reorder or hide a key.
    pub fn push_filtered<F: ReplyFilter>(
        &mut self,
        bytes: &[u8],
        now: Instant,
        filter: &mut F,
    ) -> Result<Vec<Vec<u8>>, AssemblyError> {
        let mut tokens = self.push(bytes, now)?;
        tokens.retain(|token| filter.classify(token) == ReplyDisposition::Forward);
        Ok(tokens)
    }

    /// Fail if a pending token has outlived the deadline.
    ///
    /// Transports that can be idle should call this on their own timer; [`Self::push`]
    /// calls it before consuming new bytes.
    pub fn check_deadline(&mut self, now: Instant) -> Result<(), AssemblyError> {
        let Some(since) = self.pending_since else {
            return Ok(());
        };
        if now.saturating_duration_since(since) >= self.limits.incomplete_deadline {
            let token = std::mem::take(&mut self.pending);
            self.pending_since = None;
            return Err(AssemblyError::IncompleteToken {
                token,
                reason: IncompleteReason::Deadline,
            });
        }
        Ok(())
    }

    /// Close the stream, failing if a token is still incomplete.
    pub fn finish(&mut self) -> Result<(), AssemblyError> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let token = std::mem::take(&mut self.pending);
        self.pending_since = None;
        Err(AssemblyError::IncompleteToken {
            token,
            reason: IncompleteReason::Eof,
        })
    }

    /// Signal that the transport dropped queued bytes before assembly.
    ///
    /// The active session must close; a missing prefix cannot be resynchronised.
    /// This mirrors the existing `try_send` loss path in `world_ssh`.
    pub fn queue_lost(&mut self) -> AssemblyError {
        self.pending.clear();
        self.pending_since = None;
        AssemblyError::QueueLoss
    }

    /// Bytes currently buffered for the incomplete token.
    #[must_use]
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    /// Whether no bytes are pending.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Length of the complete token at the front of `pending`, if there is one.
    fn scan(&self) -> Result<Option<usize>, AssemblyError> {
        let bytes = self.pending.as_slice();
        let Some(&first) = bytes.first() else {
            return Ok(None);
        };
        if first == ESC {
            return scan_escape(bytes);
        }
        if first < 0x80 {
            // C0 controls (other than ESC), DEL and ASCII text are one byte.
            return Ok(Some(1));
        }
        utf8_scalar(bytes, 0)
    }
}

impl Default for InputAssembler {
    fn default() -> Self {
        Self::new(Limits::default())
    }
}

/// Scan `ESC ...`.
fn scan_escape(bytes: &[u8]) -> Result<Option<usize>, AssemblyError> {
    let Some(&second) = bytes.get(1) else {
        return Ok(None);
    };
    match second {
        ESC => Err(malformed(bytes)),
        CSI | SS3 => scan_sequence(bytes),
        byte if byte < 0x80 => Ok(Some(2)),
        _ => match utf8_scalar(bytes, 1)? {
            Some(width) => Ok(Some(1 + width)),
            None => Ok(None),
        },
    }
}

/// Scan the parameter and final bytes of a CSI or SS3 sequence.
///
/// Parameters and intermediates are `0x20..=0x3f`; the first `0x40..=0x7e` byte
/// is the final byte. Anything else is not a sequence and is fatal.
fn scan_sequence(bytes: &[u8]) -> Result<Option<usize>, AssemblyError> {
    for (index, &byte) in bytes.iter().enumerate().skip(2) {
        match byte {
            0x40..=0x7e => return Ok(Some(index + 1)),
            0x20..=0x3f => {}
            _ => return Err(malformed(bytes)),
        }
    }
    Ok(None)
}

/// Scan one complete UTF-8 scalar starting at `start`.
///
/// Returns `None` while a prefix is still incomplete. Continuation bytes are
/// validated as they arrive; [`std::str::from_utf8`] rejects overlong forms,
/// surrogates and scalars above U+10FFFF when the scalar is complete.
fn utf8_scalar(bytes: &[u8], start: usize) -> Result<Option<usize>, AssemblyError> {
    let Some(&lead) = bytes.get(start) else {
        return Ok(None);
    };
    let width = match lead {
        0x00..=0x7f => 1,
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => {
            return Err(AssemblyError::InvalidUtf8 {
                token: bytes.to_vec(),
            });
        }
    };
    let end = start + width;
    let available_end = end.min(bytes.len());
    if bytes[start + 1..available_end]
        .iter()
        .any(|byte| byte & 0xc0 != 0x80)
    {
        return Err(AssemblyError::InvalidUtf8 {
            token: bytes.to_vec(),
        });
    }
    if bytes.len() < end {
        return Ok(None);
    }
    match std::str::from_utf8(&bytes[start..end]) {
        Ok(_) => Ok(Some(width)),
        Err(_) => Err(AssemblyError::InvalidUtf8 {
            token: bytes.to_vec(),
        }),
    }
}

fn malformed(token: &[u8]) -> AssemblyError {
    AssemblyError::MalformedEscape {
        token: token.to_vec(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// Feed a whole byte stream in one chunk.
    fn tokens(input: &[u8]) -> Vec<Vec<u8>> {
        let mut assembler = InputAssembler::default();
        assembler.push(input, Instant::now()).unwrap()
    }

    /// Feed every chunk in order, accumulating every emitted token.
    fn feed(chunks: &[&[u8]]) -> Result<Vec<Vec<u8>>, AssemblyError> {
        let mut assembler = InputAssembler::default();
        let now = Instant::now();
        let mut output = Vec::new();
        for chunk in chunks {
            output.extend(assembler.push(chunk, now)?);
        }
        Ok(output)
    }

    /// The inputs every split must reassemble. Each entry is one transport
    /// stream, not necessarily one token.
    const CORPUS: &[&[u8]] = &[
        b"h",
        b"hi",
        "é".as_bytes(),
        "😀".as_bytes(),
        b"\x1b[27u",    // Escape
        b"\x1b[120;3u", // Alt+x
        b"\x1b[27;3u",  // Alt+Escape
        b"\x1b[1;5C",   // Ctrl+Right
        b"\x1b[3;7~",   // Alt+Ctrl+Delete
        b"\x1b[9;2u",   // Shift+Tab
        b"\x1bOP",      // F1, SS3
        b"\x1bO1;2P",   // Modified SS3
        b"\x1bx",       // Legacy Alt+x
        b"\t",
        b"\r",
        b"\x7f",
        b"hi\x1b[27u\x1b[1;5C", // Coalesced keys
        b"\x1b[?1u\x1b[120;3u", // Probe reply coalesced with a key
    ];

    #[test]
    fn plain_text_emits_one_token_per_scalar() {
        assert_eq!(tokens(b"hi"), vec![b"h".to_vec(), b"i".to_vec()]);
        assert_eq!(tokens("é".as_bytes()), vec!["é".as_bytes().to_vec()]);
        assert_eq!(tokens("😀".as_bytes()), vec!["😀".as_bytes().to_vec()]);
    }

    #[test]
    fn coalesced_input_preserves_order_and_bytes() {
        assert_eq!(
            tokens(b"hi\x1b[27u"),
            vec![b"h".to_vec(), b"i".to_vec(), b"\x1b[27u".to_vec()]
        );
    }

    #[test]
    fn every_split_point_reassembles_the_same_tokens() {
        for input in CORPUS {
            let whole = tokens(input);
            for split in 0..=input.len() {
                let got = feed(&[&input[..split], &input[split..]]).unwrap();
                assert_eq!(
                    got,
                    whole,
                    "split at {split} of {:?} changed the token stream",
                    String::from_utf8_lossy(input)
                );
            }
        }
    }

    #[test]
    fn every_byte_boundary_reassembles_the_same_tokens() {
        for input in CORPUS {
            let whole = tokens(input);
            let chunks: Vec<&[u8]> = input.chunks(1).collect();
            let got = feed(&chunks).unwrap();
            assert_eq!(
                got,
                whole,
                "byte-by-byte feed of {:?} changed the token stream",
                String::from_utf8_lossy(input)
            );
        }
    }

    #[test]
    fn incomplete_token_completes_before_the_deadline() {
        let now = Instant::now();
        let mut assembler = InputAssembler::default();
        assert!(assembler.push(&[0xc3], now).unwrap().is_empty());
        assert_eq!(assembler.pending_len(), 1);
        let output = assembler
            .push(&[0xa9], now + Duration::from_millis(4999))
            .unwrap();
        assert_eq!(output, vec!["é".as_bytes().to_vec()]);
    }

    #[test]
    fn incomplete_token_at_the_deadline_is_fatal() {
        let now = Instant::now();
        let mut assembler = InputAssembler::default();
        assert!(assembler.push(b"\x1b[1;", now).unwrap().is_empty());
        let error = assembler
            .push(b"", now + INCOMPLETE_TOKEN_DEADLINE)
            .unwrap_err();
        assert!(matches!(
            error,
            AssemblyError::IncompleteToken {
                reason: IncompleteReason::Deadline,
                ..
            }
        ));
        assert!(assembler.is_empty());
    }

    #[test]
    fn lone_escape_is_incomplete_until_the_deadline() {
        let now = Instant::now();
        let mut assembler = InputAssembler::default();
        assert!(assembler.push(b"\x1b", now).unwrap().is_empty());
        assert!(
            assembler
                .check_deadline(now + Duration::from_secs(4))
                .is_ok()
        );
        assert!(matches!(
            assembler.check_deadline(now + INCOMPLETE_TOKEN_DEADLINE),
            Err(AssemblyError::IncompleteToken {
                reason: IncompleteReason::Deadline,
                ..
            })
        ));
    }

    #[test]
    fn eof_with_a_pending_token_is_fatal() {
        let mut assembler = InputAssembler::default();
        assembler.push(b"\x1b[1", Instant::now()).unwrap();
        assert!(matches!(
            assembler.finish(),
            Err(AssemblyError::IncompleteToken {
                reason: IncompleteReason::Eof,
                ..
            })
        ));
        assert!(assembler.is_empty());
    }

    #[test]
    fn eof_with_no_pending_token_is_clean() {
        let mut assembler = InputAssembler::default();
        assembler.push(b"hi", Instant::now()).unwrap();
        assert!(assembler.finish().is_ok());
    }

    #[test]
    fn a_token_at_the_cap_completes_but_one_over_is_fatal() {
        let now = Instant::now();
        let at_cap = {
            let mut input = vec![ESC, b'['];
            input.extend(std::iter::repeat_n(b'1', MAX_TOKEN_BYTES - 3));
            input.push(b'u');
            input
        };
        assert_eq!(at_cap.len(), MAX_TOKEN_BYTES);
        let mut assembler = InputAssembler::default();
        assert_eq!(assembler.push(&at_cap, now).unwrap(), vec![at_cap.clone()]);

        let over_cap = {
            let mut input = vec![ESC, b'['];
            input.extend(std::iter::repeat_n(b'1', MAX_TOKEN_BYTES - 2));
            input.push(b'u');
            input
        };
        let mut assembler = InputAssembler::default();
        assert!(matches!(
            assembler.push(&over_cap, now),
            Err(AssemblyError::TokenTooLong {
                limit: MAX_TOKEN_BYTES
            })
        ));
    }

    #[test]
    fn an_unterminated_token_over_the_cap_is_fatal() {
        let input = vec![ESC, b'['];
        let input: Vec<u8> = input
            .into_iter()
            .chain(std::iter::repeat_n(b'1', MAX_TOKEN_BYTES + 1))
            .collect();
        let mut assembler = InputAssembler::default();
        assert!(matches!(
            assembler.push(&input, Instant::now()),
            Err(AssemblyError::TokenTooLong {
                limit: MAX_TOKEN_BYTES
            })
        ));
    }

    #[test]
    fn invalid_utf8_is_fatal() {
        for input in [
            vec![0x80],
            vec![0xc0, 0x80],
            vec![0xe2, 0x41],
            vec![0xed, 0xa0, 0x80],
            vec![0xf5, 0x80, 0x80, 0x80],
        ] {
            let mut assembler = InputAssembler::default();
            assert!(
                matches!(
                    assembler.push(&input, Instant::now()),
                    Err(AssemblyError::InvalidUtf8 { .. })
                ),
                "expected invalid UTF-8 for {input:02x?}"
            );
        }
    }

    #[test]
    fn malformed_escape_sequences_are_fatal() {
        for input in [
            b"\x1b\x1b".as_slice(),
            b"\x1b[\x01".as_slice(),
            b"\x1b[\xff".as_slice(),
            b"\x1bO\x01".as_slice(),
        ] {
            let mut assembler = InputAssembler::default();
            assert!(
                matches!(
                    assembler.push(input, Instant::now()),
                    Err(AssemblyError::MalformedEscape { .. })
                ),
                "expected malformed escape for {input:02x?}"
            );
        }
    }

    #[test]
    fn interleaved_connections_keep_pending_state_separate() {
        let now = Instant::now();
        let mut first = InputAssembler::default();
        let mut second = InputAssembler::default();
        assert!(first.push(&[0xc3], now).unwrap().is_empty());
        assert!(second.push(b"\x1b", now).unwrap().is_empty());
        assert_eq!(
            first.push(&[0xa9], now).unwrap(),
            vec!["é".as_bytes().to_vec()]
        );
        assert_eq!(second.push(b"[A", now).unwrap(), vec![b"\x1b[A".to_vec()]);
        assert!(first.is_empty());
        assert!(second.is_empty());
    }

    #[test]
    fn reply_filter_consumes_probe_replies_and_preserves_key_order() {
        struct ProbeReplies;
        impl ReplyFilter for ProbeReplies {
            fn classify(&mut self, token: &[u8]) -> ReplyDisposition {
                if token.starts_with(b"\x1b[?") {
                    ReplyDisposition::Consume
                } else {
                    ReplyDisposition::Forward
                }
            }
        }

        let input = b"\x1b[?1u\x1b[120;3uX\x1b[?u";
        let expected = vec![b"\x1b[120;3u".to_vec(), b"X".to_vec()];
        let now = Instant::now();
        let mut assembler = InputAssembler::default();
        assert_eq!(
            assembler
                .push_filtered(input, now, &mut ProbeReplies)
                .unwrap(),
            expected
        );

        // The same holds at every transport split.
        for split in 0..=input.len() {
            let mut assembler = InputAssembler::default();
            let mut filter = ProbeReplies;
            let mut got = assembler
                .push_filtered(&input[..split], now, &mut filter)
                .unwrap();
            got.extend(
                assembler
                    .push_filtered(&input[split..], now, &mut filter)
                    .unwrap(),
            );
            assert_eq!(got, expected, "split at {split}");
        }
    }

    #[test]
    fn queue_loss_is_a_fatal_signal() {
        let mut assembler = InputAssembler::default();
        assert!(
            assembler
                .push(b"\x1b[1", Instant::now())
                .unwrap()
                .is_empty()
        );
        assert_eq!(assembler.queue_lost(), AssemblyError::QueueLoss);
        assert!(assembler.is_empty());
    }
}
