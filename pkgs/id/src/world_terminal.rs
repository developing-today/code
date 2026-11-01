//! Local terminal capability detection scaffolding for interactive world views.
//!
//! The SSH and iroh adapters will share this module in later phases. Profiles
//! stay on the client side; they are not part of shell-contract frames or the
//! event journal.

use std::time::Duration;

/// Total startup window for the batched capability queries.
pub(crate) const PROBE_TIMEOUT: Duration = Duration::from_millis(250);

/// Queries sent together so detection does not incur one round trip per feature.
///
/// XTGETTCAP's special `Co` color-count capability is queried alongside an
/// ANSI cursor-position report. The remaining
/// sequences are Kitty's keyboard and graphics queries, DEC mode 2026 via
/// DECRQM, and primary device attributes for Sixel detection.
pub(crate) const PROBE_QUERIES: &[u8] =
    b"\x1bP+q436F\x1b\\\x1b[6n\x1b[?u\x1b[?2026$p\x1b[c\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\";

/// Injectable byte I/O used by the SSH PTY and interactive iroh TTY adapters.
///
/// `read_until` must stop at the supplied total timeout and return all bytes
/// observed before it expires. A TTY-less caller does not invoke the probe.
pub(crate) trait TerminalIo {
    /// The adapter's I/O error type.
    type Error;

    /// Write a query burst to the local terminal.
    fn write(&mut self, bytes: &[u8]) -> Result<(), Self::Error>;

    /// Read bytes until the single startup deadline expires.
    fn read_until(&mut self, timeout: Duration) -> Result<Vec<u8>, Self::Error>;
}

/// Independently confirmed terminal features. Unknown is represented as false.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct TerminalProfile {
    /// A complete ANSI cursor-position report was received.
    pub(crate) cursor_position_report: bool,
    /// XTGETTCAP's special `Co` capability reported at least 256 colors.
    pub(crate) indexed_color: bool,
    /// The Kitty graphics query answered with a successful response.
    pub(crate) kitty_graphics: bool,
    /// Primary device attributes advertised Sixel support (parameter 4).
    pub(crate) sixel_graphics: bool,
    /// DEC private mode 2026 was recognized and can be toggled.
    pub(crate) synchronized_output: bool,
    /// The Kitty keyboard flags query returned a syntactically valid report.
    pub(crate) kitty_keyboard: bool,
}

impl TerminalProfile {
    /// Select the richest tier whose required features were positively seen.
    ///
    /// Kitty keyboard support is intentionally orthogonal: it is owned by the
    /// later keyboard-input phase and does not gate visual Tier 1 selection.
    pub(crate) fn tier(self) -> PresentationTier {
        if (self.kitty_graphics || self.sixel_graphics) && self.synchronized_output {
            PresentationTier::Tier1
        } else if self.cursor_position_report && self.indexed_color {
            PresentationTier::Tier2
        } else {
            PresentationTier::Tier3
        }
    }
}

/// Presentation class selected from a local [`TerminalProfile`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PresentationTier {
    /// Graphics and synchronized output were both positively confirmed.
    Tier1,
    /// Cursor addressing and indexed color were both positively confirmed.
    Tier2,
    /// No safe richer mode was positively confirmed.
    Tier3,
}

/// Profile plus input bytes that were not complete matching query replies.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ProbeOutcome {
    /// Locally selected terminal profile.
    pub(crate) profile: TerminalProfile,
    /// Unmatched or incomplete bytes, preserved byte-for-byte for key input.
    pub(crate) replay: Vec<u8>,
    /// At least one capability received contradictory complete reports.
    pub(crate) conflicted: bool,
}

impl ProbeOutcome {
    /// Return the conservative Tier 3 profile when complete reports conflict.
    pub(crate) fn tier(&self) -> PresentationTier {
        if self.conflicted {
            PresentationTier::Tier3
        } else {
            self.profile.tier()
        }
    }
}

/// One bounded query/reply exchange, kept injectable for transcript tests.
pub(crate) struct TerminalNegotiator;

impl TerminalNegotiator {
    /// Probe once, using a single deadline and returning unmatched input for replay.
    pub(crate) fn probe<I: TerminalIo>(io: &mut I) -> Result<ProbeOutcome, I::Error> {
        io.write(PROBE_QUERIES)?;
        let transcript = io.read_until(PROBE_TIMEOUT)?;
        Ok(parse_transcript(&transcript))
    }
}

/// Byte and frame counters used to compare presenters with a full-render baseline.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct PresenterCounters {
    /// Number of frames emitted by this presenter.
    pub(crate) frames: u64,
    /// Number of those frames emitted as complete renders.
    pub(crate) full_render_frames: u64,
    /// Total bytes written by this presenter.
    pub(crate) presenter_bytes: u64,
}

impl PresenterCounters {
    /// Record one successful presenter write.
    pub(crate) fn record(&mut self, bytes: usize, full_render: bool) {
        self.frames = self.frames.saturating_add(1);
        self.presenter_bytes = self.presenter_bytes.saturating_add(bytes as u64);
        if full_render {
            self.full_render_frames = self.full_render_frames.saturating_add(1);
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Evidence {
    cursor_position_report: Option<bool>,
    indexed_color: Option<bool>,
    kitty_graphics: Option<bool>,
    sixel_graphics: Option<bool>,
    synchronized_output: Option<bool>,
    kitty_keyboard: Option<bool>,
    conflicted: bool,
}

impl Evidence {
    fn record(slot: &mut Option<bool>, value: bool, conflicted: &mut bool) {
        if slot.is_some_and(|previous| previous != value) {
            *conflicted = true;
        }
        *slot = Some(value);
    }

    fn profile(self) -> TerminalProfile {
        if self.conflicted {
            return TerminalProfile::default();
        }
        TerminalProfile {
            cursor_position_report: self.cursor_position_report.unwrap_or(false),
            indexed_color: self.indexed_color.unwrap_or(false),
            kitty_graphics: self.kitty_graphics.unwrap_or(false),
            sixel_graphics: self.sixel_graphics.unwrap_or(false),
            synchronized_output: self.synchronized_output.unwrap_or(false),
            kitty_keyboard: self.kitty_keyboard.unwrap_or(false),
        }
    }
}

/// Parse complete replies from the batched transcript and preserve everything else.
///
/// Partial sequences are intentionally not consumed. This parser has no access
/// to transport frames, guest input, or journal state.
fn parse_transcript(transcript: &[u8]) -> ProbeOutcome {
    let mut evidence = Evidence::default();
    let mut replay = Vec::new();
    let mut offset = 0;

    while offset < transcript.len() {
        if transcript[offset..].starts_with(b"\x1bP") || transcript[offset..].starts_with(b"\x1b_")
        {
            let introducer = transcript[offset + 1];
            let Some(end) = find_st(transcript, offset + 2) else {
                replay.extend_from_slice(&transcript[offset..]);
                break;
            };
            let body = &transcript[offset + 2..end];
            let matched = if introducer == b'P' {
                parse_xtgettcap(body, &mut evidence)
            } else {
                parse_kitty_graphics(body, &mut evidence)
            };
            if !matched {
                replay.extend_from_slice(&transcript[offset..end + 2]);
            }
            offset = end + 2;
            continue;
        }

        if transcript[offset..].starts_with(b"\x1b[") {
            let Some(final_offset) = (offset + 2..transcript.len())
                .find(|&index| (0x40..=0x7e).contains(&transcript[index]))
            else {
                replay.extend_from_slice(&transcript[offset..]);
                break;
            };
            let body = &transcript[offset + 2..final_offset];
            let final_byte = transcript[final_offset];
            if !parse_csi(body, final_byte, &mut evidence) {
                replay.extend_from_slice(&transcript[offset..=final_offset]);
            }
            offset = final_offset + 1;
            continue;
        }

        replay.push(transcript[offset]);
        offset += 1;
    }

    ProbeOutcome {
        profile: evidence.profile(),
        replay,
        conflicted: evidence.conflicted,
    }
}

fn find_st(input: &[u8], from: usize) -> Option<usize> {
    input
        .get(from..)?
        .windows(2)
        .position(|window| window == b"\x1b\\")
        .map(|relative| from + relative)
}

fn parse_xtgettcap(body: &[u8], evidence: &mut Evidence) -> bool {
    // XTerm ctlseqs: DCS 1 + r name=value ST, or DCS 0 + r ST on failure.
    // Capability names and returned strings are hex encoded.
    let Some(rest) = body.strip_prefix(b"1+r") else {
        return body == b"0+r";
    };
    let mut recognized = false;
    for pair in rest.split(|byte| *byte == b';') {
        let Some((name, value)) = pair.split_once(|byte| *byte == b'=') else {
            continue;
        };
        let Some(name) = decode_hex(name) else {
            continue;
        };
        let value = decode_hex(value);
        match name.as_slice() {
            b"Co" => {
                let indexed = value
                    .as_deref()
                    .and_then(parse_decimal)
                    .is_some_and(|colors| colors >= 256);
                Evidence::record(
                    &mut evidence.indexed_color,
                    indexed,
                    &mut evidence.conflicted,
                );
                recognized = true;
            }
            _ => {}
        }
    }
    recognized
}

fn parse_kitty_graphics(body: &[u8], evidence: &mut Evidence) -> bool {
    // Kitty graphics protocol query responses echo the request id and report OK
    // on success; an error response is still a complete, matching negative.
    let Some(status) = body.strip_prefix(b"Gi=31;") else {
        return false;
    };
    let supported = status == b"OK";
    Evidence::record(
        &mut evidence.kitty_graphics,
        supported,
        &mut evidence.conflicted,
    );
    true
}

fn parse_csi(body: &[u8], final_byte: u8, evidence: &mut Evidence) -> bool {
    match (body.first(), final_byte) {
        (Some(b'0'..=b'9'), b'R') => {
            // CPR (CSI row ; column R) is a positive response to CSI 6 n.
            let Some(parameters) = parse_parameter_list(body) else {
                return false;
            };
            if parameters.len() != 2 || parameters.contains(&0) {
                return false;
            }
            Evidence::record(
                &mut evidence.cursor_position_report,
                true,
                &mut evidence.conflicted,
            );
            true
        }
        (Some(b'?'), b'u') => {
            // Kitty keyboard protocol reports the current flags, including 0.
            if parse_decimal(&body[1..]).is_none() {
                return false;
            }
            Evidence::record(&mut evidence.kitty_keyboard, true, &mut evidence.conflicted);
            true
        }
        (Some(b'?'), b'y') => {
            // DECRPM: CSI ? 2026 ; Pm $ y. Pm 1/2 means mode recognized;
            // 0, 3, and 4 do not establish a safely toggleable sync mode.
            let Some(parameters) = body.strip_suffix(b"$") else {
                return false;
            };
            let mut parts = parameters[1..].split(|byte| *byte == b';');
            let Some(mode) = parts.next().and_then(parse_decimal) else {
                return false;
            };
            let Some(state) = parts.next().and_then(parse_decimal) else {
                return false;
            };
            if parts.next().is_some() || mode != 2026 || state > 4 {
                return false;
            }
            Evidence::record(
                &mut evidence.synchronized_output,
                matches!(state, 1 | 2),
                &mut evidence.conflicted,
            );
            true
        }
        (Some(b'?'), b'c') => {
            // XTerm primary DA: parameter 4 advertises Sixel graphics.
            let Some(parameters) = parse_parameter_list(&body[1..]) else {
                return false;
            };
            if parameters.is_empty() {
                return false;
            }
            Evidence::record(
                &mut evidence.sixel_graphics,
                parameters.contains(&4),
                &mut evidence.conflicted,
            );
            true
        }
        _ => false,
    }
}

fn parse_parameter_list(input: &[u8]) -> Option<Vec<u32>> {
    input
        .split(|byte| *byte == b';')
        .map(parse_decimal)
        .collect()
}

fn parse_decimal(input: &[u8]) -> Option<u32> {
    if input.is_empty() || !input.iter().all(u8::is_ascii_digit) {
        return None;
    }
    input.iter().try_fold(0_u32, |value, digit| {
        value.checked_mul(10)?.checked_add(u32::from(*digit - b'0'))
    })
}

fn decode_hex(input: &[u8]) -> Option<Vec<u8>> {
    if input.is_empty() || input.len() % 2 != 0 {
        return None;
    }
    input
        .chunks_exact(2)
        .map(|pair| {
            let high = hex_digit(pair[0])?;
            let low = hex_digit(pair[1])?;
            Some(high * 16 + low)
        })
        .collect()
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    const COLOR_COUNT_REPLY: &[u8] = b"\x1bP1+r436F=323536\x1b\\";
    const CURSOR_POSITION_REPLY: &[u8] = b"\x1b[12;40R";
    const KITTY_KEYBOARD_REPLY: &[u8] = b"\x1b[?1u";
    const SYNC_OUTPUT_REPLY: &[u8] = b"\x1b[?2026;2$y";
    const SIXEL_DA_REPLY: &[u8] = b"\x1b[?1;2;4c";
    const KITTY_GRAPHICS_REPLY: &[u8] = b"\x1b_Gi=31;OK\x1b\\";

    struct TranscriptIo {
        replies: VecDeque<Vec<u8>>,
        delay: Duration,
        writes: Vec<Vec<u8>>,
    }

    impl TranscriptIo {
        fn new(delay: Duration, replies: impl IntoIterator<Item = &'static [u8]>) -> Self {
            Self {
                replies: replies.into_iter().map(<[u8]>::to_vec).collect(),
                delay,
                writes: Vec::new(),
            }
        }
    }

    impl TerminalIo for TranscriptIo {
        type Error = ();

        fn write(&mut self, bytes: &[u8]) -> Result<(), Self::Error> {
            self.writes.push(bytes.to_vec());
            Ok(())
        }

        fn read_until(&mut self, timeout: Duration) -> Result<Vec<u8>, Self::Error> {
            if self.delay > timeout {
                return Ok(Vec::new());
            }
            Ok(self.replies.drain(..).flatten().collect())
        }
    }

    #[test]
    fn tier_one_transcript_selects_rich_profile_and_replays_user_input() {
        let mut io = TranscriptIo::new(
            Duration::from_millis(5),
            [
                b"j".as_slice(),
                COLOR_COUNT_REPLY,
                b"k".as_slice(),
                CURSOR_POSITION_REPLY,
                KITTY_KEYBOARD_REPLY,
                SYNC_OUTPUT_REPLY,
                KITTY_GRAPHICS_REPLY,
            ],
        );

        let outcome = TerminalNegotiator::probe(&mut io).expect("scripted I/O succeeds");

        assert_eq!(io.writes, vec![PROBE_QUERIES.to_vec()]);
        assert_eq!(outcome.tier(), PresentationTier::Tier1);
        assert_eq!(
            outcome.profile,
            TerminalProfile {
                cursor_position_report: true,
                indexed_color: true,
                kitty_graphics: true,
                sixel_graphics: false,
                synchronized_output: true,
                kitty_keyboard: true,
            }
        );
        assert_eq!(outcome.replay, b"jk");
    }

    #[test]
    fn sixel_plus_sync_selects_tier_one_without_kitty_keyboard() {
        let outcome = parse_transcript(&[SIXEL_DA_REPLY, SYNC_OUTPUT_REPLY].concat());

        assert_eq!(outcome.tier(), PresentationTier::Tier1);
        assert!(outcome.profile.sixel_graphics);
        assert!(!outcome.profile.kitty_keyboard);
    }

    #[test]
    fn cell_features_select_tier_two_and_keyboard_is_orthogonal() {
        let outcome = parse_transcript(
            &[
                COLOR_COUNT_REPLY,
                CURSOR_POSITION_REPLY,
                KITTY_KEYBOARD_REPLY,
            ]
            .concat(),
        );

        assert_eq!(outcome.tier(), PresentationTier::Tier2);
        assert!(outcome.profile.kitty_keyboard);
        assert!(!outcome.profile.synchronized_output);
        assert!(!outcome.profile.kitty_graphics);
    }

    #[test]
    fn no_reply_and_malformed_reply_fall_back_to_tier_three() {
        let no_reply = parse_transcript(&[]);
        let malformed = parse_transcript(b"\x1b[?x u\x1b[?2026;9$y\x1b_Gi=other;OK\x1b\\");

        assert_eq!(no_reply.tier(), PresentationTier::Tier3);
        assert_eq!(malformed.tier(), PresentationTier::Tier3);
        assert_eq!(
            malformed.replay,
            b"\x1b[?x u\x1b[?2026;9$y\x1b_Gi=other;OK\x1b\\"
        );
    }

    #[test]
    fn partial_late_and_interleaved_input_are_not_promoted_to_capabilities() {
        let partial = parse_transcript(b"\x1bP1+r436F=32");
        let interleaved = parse_transcript(b"x\x1b[?1u\x1b[A\x1bP1+r436F=323536\x1b\\");
        let mut late_io = TranscriptIo::new(Duration::from_millis(251), [SYNC_OUTPUT_REPLY]);
        let late = TerminalNegotiator::probe(&mut late_io).expect("scripted I/O succeeds");

        assert_eq!(partial.tier(), PresentationTier::Tier3);
        assert_eq!(partial.replay, b"\x1bP1+r436F=32");
        assert_eq!(interleaved.tier(), PresentationTier::Tier3);
        assert_eq!(interleaved.replay, b"x\x1b[A");
        assert_eq!(late.tier(), PresentationTier::Tier3);
        assert!(late.replay.is_empty());
    }

    #[test]
    fn conflicting_capability_reports_force_safe_fallback() {
        let outcome = parse_transcript(
            [KITTY_GRAPHICS_REPLY, b"\x1b_Gi=31;ENOENT\x1b\\"]
                .concat()
                .as_slice(),
        );

        assert!(outcome.conflicted);
        assert_eq!(outcome.tier(), PresentationTier::Tier3);
        assert_eq!(outcome.profile, TerminalProfile::default());
    }

    #[test]
    fn render_counters_track_full_frames_and_total_bytes() {
        let mut counters = PresenterCounters::default();

        counters.record(12, true);
        counters.record(5, false);

        assert_eq!(
            counters,
            PresenterCounters {
                frames: 2,
                full_render_frames: 1,
                presenter_bytes: 17,
            }
        );
    }
}
