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
pub struct ProbeOutcome {
    /// Locally selected terminal profile.
    pub(crate) profile: TerminalProfile,
    /// Unmatched or incomplete bytes, preserved byte-for-byte for key input.
    pub replay: Vec<u8>,
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

/// Streaming demultiplexer used by the SSH adapter.
///
/// Bytes are forwarded as user input in arrival order. A possible reply prefix
/// holds back every byte behind it until it completes or the startup deadline
/// expires, so unmatched keystrokes can never be reordered around a reply.
#[derive(Debug, Default)]
pub(crate) struct ProbeDemux {
    evidence: Evidence,
    pending: Vec<u8>,
    expired: bool,
}

impl ProbeDemux {
    /// Start an empty demultiplexer for one startup probe.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Feed received bytes; returns the bytes that are user input, in order.
    pub(crate) fn push(&mut self, bytes: &[u8]) -> Vec<u8> {
        if self.expired {
            return bytes.to_vec();
        }
        self.pending.extend_from_slice(bytes);
        let mut input = Vec::new();
        let mut consumed = 0;
        while consumed < self.pending.len() {
            match scan_step(&self.pending[consumed..], &mut self.evidence) {
                ScanStep::Reply(len) => consumed += len,
                ScanStep::Unmatched(len) => {
                    input.extend_from_slice(&self.pending[consumed..consumed + len]);
                    consumed += len;
                }
                ScanStep::Incomplete => break,
            }
        }
        self.pending.drain(..consumed);
        input
    }

    /// Close the startup window: held prefixes are user input after all.
    ///
    /// Every later [`ProbeDemux::push`] forwards bytes untouched.
    pub(crate) fn expire(&mut self) -> Vec<u8> {
        self.expired = true;
        std::mem::take(&mut self.pending)
    }

    /// Evidence gathered so far, including bytes still held.
    pub(crate) fn outcome(&self) -> ProbeOutcome {
        ProbeOutcome {
            profile: self.evidence.profile(),
            replay: self.pending.clone(),
            conflicted: self.evidence.conflicted,
        }
    }

    /// Tier selected from the replies seen so far.
    pub(crate) fn tier(&self) -> PresentationTier {
        self.outcome().tier()
    }
}

/// Probe a local interactive terminal once, restoring its mode before return.
///
/// Returns `None` when either standard stream is not a terminal (a piped or
/// scripted caller must send no probe bytes), or when the terminal cannot be
/// switched to the non-canonical mode needed to read replies. Only the Tier 3
/// fallback presenter consumes this result in the current slice.
#[cfg(unix)]
pub fn probe_local_tty() -> Option<ProbeOutcome> {
    use std::io::IsTerminal;

    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return None;
    }
    let _raw = RawMode::enable()?;
    let mut io = TtyIo;
    TerminalNegotiator::probe(&mut io).ok()
}

/// Non-Unix builds have no TTY probe; callers keep the piped behavior.
#[cfg(not(unix))]
pub fn probe_local_tty() -> Option<ProbeOutcome> {
    None
}

/// Byte I/O for [`probe_local_tty`], bounded by the caller's deadline.
#[cfg(unix)]
#[derive(Debug)]
struct TtyIo;

#[cfg(unix)]
impl TerminalIo for TtyIo {
    type Error = std::io::Error;

    fn write(&mut self, bytes: &[u8]) -> Result<(), Self::Error> {
        use std::io::Write as _;

        let mut out = std::io::stdout().lock();
        out.write_all(bytes)?;
        out.flush()
    }

    #[allow(unsafe_code)] // libc poll/read are the only way to bound a TTY read.
    fn read_until(&mut self, timeout: Duration) -> Result<Vec<u8>, Self::Error> {
        let deadline = std::time::Instant::now() + timeout;
        let mut collected = Vec::new();
        loop {
            let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) else {
                return Ok(collected);
            };
            // Round up so a sub-millisecond remainder still gets one poll.
            let millis = remaining
                .as_millis()
                .saturating_add(u128::from(remaining.subsec_nanos() > 0))
                .min(i32::MAX as u128) as i32;
            let mut pollfd = libc::pollfd {
                fd: libc::STDIN_FILENO,
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: `pollfd` points at one initialized descriptor owned by
            // this call; poll only reads it and writes `revents`.
            let ready = unsafe { libc::poll(&raw mut pollfd, 1, millis) };
            if ready < 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error);
            }
            if ready == 0 {
                return Ok(collected);
            }
            let mut buffer = [0_u8; 256];
            // SAFETY: the buffer is writable for its full length and stdin was
            // polled readable above; read never writes past `buffer.len()`.
            let read =
                unsafe { libc::read(libc::STDIN_FILENO, buffer.as_mut_ptr().cast(), buffer.len()) };
            if read < 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error);
            }
            if read == 0 {
                return Ok(collected);
            }
            collected.extend_from_slice(&buffer[..read as usize]);
        }
    }
}

/// Non-canonical mode for the probe window, restored on every return path.
#[cfg(unix)]
struct RawMode {
    original: libc::termios,
}

#[cfg(unix)]
impl std::fmt::Debug for RawMode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("RawMode")
    }
}

#[cfg(unix)]
impl RawMode {
    /// Disable line buffering and echo so terminal replies arrive immediately.
    #[allow(unsafe_code)] // termios calls are the only way to read query replies.
    fn enable() -> Option<Self> {
        // SAFETY: `termios` is plain old data; tcgetattr fills it on success.
        let mut term = unsafe { std::mem::zeroed::<libc::termios>() };
        // SAFETY: `term` points at writable storage for one termios and stdin
        // is a valid descriptor for this process.
        if unsafe { libc::tcgetattr(libc::STDIN_FILENO, &raw mut term) } != 0 {
            return None;
        }
        let original = term;
        term.c_lflag &= !(libc::ICANON | libc::ECHO);
        term.c_cc[libc::VMIN] = 0;
        term.c_cc[libc::VTIME] = 0;
        // SAFETY: `term` is the termios just read for this descriptor.
        if unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw const term) } != 0 {
            return None;
        }
        Some(Self { original })
    }
}

#[cfg(unix)]
impl Drop for RawMode {
    #[allow(unsafe_code)] // restoring termios is the inverse tcsetattr call.
    fn drop(&mut self) {
        // SAFETY: `original` came from a successful tcgetattr on this
        // descriptor; restoring it is the documented inverse operation.
        unsafe {
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw const self.original);
        }
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

/// One step of the shared query/reply grammar.
///
/// The same scanner is used to parse a whole transcript and to demultiplex a
/// live byte stream, so both adapters classify bytes identically.
enum ScanStep {
    /// A complete, matching reply consuming this many bytes.
    Reply(usize),
    /// Complete bytes that match no outstanding query: user input.
    Unmatched(usize),
    /// A possible reply prefix; wait for more bytes or the deadline.
    Incomplete,
}

/// Classify the sequence at the start of `input`, recording any reply evidence.
///
/// A single plain byte is always user input. A lead-in that could still grow
/// into a reply (`ESC`, `ESC [`, an unterminated DCS/APC) is incomplete until
/// it resolves or the caller's deadline expires.
fn scan_step(input: &[u8], evidence: &mut Evidence) -> ScanStep {
    let Some(&first) = input.first() else {
        return ScanStep::Incomplete;
    };
    if first == 0x1b {
        match input.get(1) {
            None => return ScanStep::Incomplete,
            Some(&b'P') | Some(&b'_') => {
                let introducer = input[1];
                let Some(end) = find_st(input, 2) else {
                    return ScanStep::Incomplete;
                };
                let body = &input[2..end];
                let matched = if introducer == b'P' {
                    parse_xtgettcap(body, evidence)
                } else {
                    parse_kitty_graphics(body, evidence)
                };
                return if matched {
                    ScanStep::Reply(end + 2)
                } else {
                    ScanStep::Unmatched(end + 2)
                };
            }
            Some(&b'[') => {
                let Some(final_offset) =
                    (2..input.len()).find(|&index| (0x40..=0x7e).contains(&input[index]))
                else {
                    return ScanStep::Incomplete;
                };
                let body = &input[2..final_offset];
                let matched = parse_csi(body, input[final_offset], evidence);
                return if matched {
                    ScanStep::Reply(final_offset + 1)
                } else {
                    ScanStep::Unmatched(final_offset + 1)
                };
            }
            Some(_) => return ScanStep::Unmatched(2),
        }
    }
    ScanStep::Unmatched(1)
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
        match scan_step(&transcript[offset..], &mut evidence) {
            ScanStep::Reply(consumed) => offset += consumed,
            ScanStep::Unmatched(consumed) => {
                replay.extend_from_slice(&transcript[offset..offset + consumed]);
                offset += consumed;
            }
            ScanStep::Incomplete => {
                replay.extend_from_slice(&transcript[offset..]);
                break;
            }
        }
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
        let Some(separator) = pair.iter().position(|byte| *byte == b'=') else {
            continue;
        };
        let (name, value) = pair.split_at(separator);
        let value = &value[1..];
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
#[allow(clippy::unwrap_used, clippy::expect_used)]
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
        fn new<'a>(delay: Duration, replies: impl IntoIterator<Item = &'a [u8]>) -> Self {
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

    /// The SSH stream demultiplexer and the iroh `TerminalIo` seam share one
    /// grammar: equivalent transcripts must select the same tier and replay
    /// the same user bytes, in order (F13.4 Phase 2).
    #[test]
    fn ssh_demux_and_iroh_seam_agree_on_the_transcript_matrix() {
        let cases: [(&str, Vec<u8>); 6] = [
            (
                "tier one",
                [
                    b"j".as_slice(),
                    COLOR_COUNT_REPLY,
                    b"k".as_slice(),
                    CURSOR_POSITION_REPLY,
                    KITTY_KEYBOARD_REPLY,
                    SYNC_OUTPUT_REPLY,
                    KITTY_GRAPHICS_REPLY,
                ]
                .concat(),
            ),
            (
                "tier two",
                [COLOR_COUNT_REPLY, CURSOR_POSITION_REPLY].concat(),
            ),
            ("no reply", Vec::new()),
            (
                "malformed",
                b"x\x1b[?x u\x1b[?2026;9$y\x1b_Gi=other;OK\x1b\\".to_vec(),
            ),
            ("partial then deadline", b"q\x1b[1;2".to_vec()),
            (
                "interleaved keys",
                b"x\x1b[?1u\x1b[A\x1bP1+r436F=323536\x1b\\y".to_vec(),
            ),
        ];

        for (name, transcript) in cases {
            let mut iroh_io = TranscriptIo::new(Duration::from_millis(5), [transcript.as_slice()]);
            let iroh = TerminalNegotiator::probe(&mut iroh_io).expect("scripted I/O succeeds");

            let mut demux = ProbeDemux::new();
            let mut replay = demux.push(&transcript);
            replay.extend(demux.expire());
            let ssh = demux.outcome();

            assert_eq!(iroh.tier(), ssh.tier(), "{name}: tier differs");
            assert_eq!(iroh.profile, ssh.profile, "{name}: profile differs");
            assert_eq!(iroh.conflicted, ssh.conflicted, "{name}: conflict differs");
            assert_eq!(iroh.replay, replay, "{name}: replay differs");
        }
    }

    #[test]
    fn demux_streams_plain_input_and_holds_only_a_possible_reply_prefix() {
        let mut demux = ProbeDemux::new();

        assert_eq!(demux.push(b"ab"), b"ab".to_vec());
        assert_eq!(demux.push(b"\x1b"), Vec::<u8>::new());
        assert_eq!(demux.push(b"["), Vec::<u8>::new());
        assert_eq!(demux.push(b"12;40R"), Vec::<u8>::new());
        assert!(demux.outcome().profile.cursor_position_report);
        assert_eq!(demux.expire(), Vec::<u8>::new());
        // One positive report is not enough for Tier 2.
        assert_eq!(demux.tier(), PresentationTier::Tier3);
    }

    #[test]
    fn demux_deadline_flushes_held_prefix_without_reordering() {
        let mut demux = ProbeDemux::new();

        assert_eq!(demux.push(b"q\x1b[1;2"), b"q".to_vec());
        assert_eq!(demux.expire(), b"\x1b[1;2".to_vec());
        // After the deadline every byte is user input again.
        assert_eq!(demux.push(b"later"), b"later".to_vec());
        assert_eq!(demux.outcome().replay, Vec::<u8>::new());
    }

    #[test]
    fn demux_never_forwards_a_consumed_reply() {
        let mut demux = ProbeDemux::new();

        let input = demux.push(&[b"z".as_slice(), SYNC_OUTPUT_REPLY, b"w".as_slice()].concat());

        assert_eq!(input, b"zw".to_vec());
        assert!(demux.outcome().profile.synchronized_output);
        assert_eq!(demux.tier(), PresentationTier::Tier3);
    }
}
