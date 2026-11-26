//! Local terminal capability detection scaffolding for interactive world views.
//!
//! The SSH and iroh adapters will share this module in later phases. Profiles
//! stay on the client side; they are not part of shell-contract frames or the
//! event journal.

use std::num::NonZeroU32;
use std::time::Duration;

use crate::kitty_graphics::{Action, GraphicsGate, KittyGraphicsError, PayloadClass, StillPayload};
use crate::world_presentation::ScreenSurface;

/// Total startup window for the batched capability queries.
pub(crate) const PROBE_TIMEOUT: Duration = Duration::from_millis(250);

/// Queries sent together so detection does not incur one round trip per feature.
///
/// XTGETTCAP's special `Co` color-count capability is queried alongside an
/// ANSI cursor-position report. The remaining
/// sequences are Kitty's keyboard and graphics queries, DEC mode 2026 via
/// DECRQM, and primary device attributes for Sixel detection. The burst
/// ends with the kitty graphics payload-class queries of
/// [`crate::kitty_graphics`] (IDs 41 to 43: PNG, raw RGBA, zlib RGBA), so
/// the same round trip learns which stills formats this terminal accepts
/// (#13623): detection alone is not support.
pub(crate) const PROBE_QUERIES: &[u8] = b"\x1bP+q436F\x1b\\\x1b[6n\x1b[?u\x1b[?2026$p\x1b[c\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\\x1b_Ga=q,t=d,f=100,i=41;iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==\x1b\\\x1b_Ga=q,t=d,f=32,s=1,v=1,i=42;AAAAAA==\x1b\\\x1b_Ga=q,t=d,f=32,s=1,v=1,o=z,i=43;eJxjYGBgAAAABAAB\x1b\\";

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
pub struct TerminalProfile {
    /// A complete ANSI cursor-position report was received.
    pub cursor_position_report: bool,
    /// XTGETTCAP's special `Co` capability reported at least 256 colors.
    pub indexed_color: bool,
    /// The Kitty graphics query answered with a successful response.
    pub kitty_graphics: bool,
    /// Primary device attributes advertised Sixel support (parameter 4).
    pub sixel_graphics: bool,
    /// DEC private mode 2026 was recognized and can be toggled.
    pub synchronized_output: bool,
    /// The Kitty keyboard flags query returned a syntactically valid report.
    pub kitty_keyboard: bool,
    /// The kitty graphics PNG (`f=100`) payload query answered OK.
    pub kitty_png: bool,
    /// The kitty graphics raw RGBA (`f=32`) payload query answered OK.
    pub kitty_rgba: bool,
    /// The kitty graphics zlib RGBA (`f=32,o=z`) payload query answered OK.
    pub kitty_rgba_zlib: bool,
}

impl TerminalProfile {
    /// Select the richest tier whose required features were positively seen.
    ///
    /// Kitty keyboard support is intentionally orthogonal: it is owned by the
    /// later keyboard-input phase and does not gate visual Tier 1 selection.
    pub fn tier(self) -> PresentationTier {
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
pub enum PresentationTier {
    /// Graphics and synchronized output were both positively confirmed.
    Tier1,
    /// Cursor addressing and indexed color were both positively confirmed.
    Tier2,
    /// No safe richer mode was positively confirmed.
    Tier3,
}

/// How a session's frames reach the terminal, chosen by the player at
/// run time (#14861, design decisions D2/D3). The mode is negotiated per
/// session — over SSH with the `ID_DISPLAY` environment request — and
/// never changes what a world program sends: the frame is data; the
/// mode only picks the painting.
///
/// - `Auto` is the absence of a choice: exactly the pre-mode behaviour
///   (alternate screen, probe-selected tier).
/// - `Alternate` and `Primary` are the cursor modes: both paint with
///   cursor addressing at at least Tier 2, because choosing one is the
///   player's statement that the terminal has it; Tier 1 extras still
///   require the probe's positive evidence. They differ in the screen
///   buffer and the exit framing: alternate leaves its buffer and
///   restores the shell as it was; primary paints in place and leaves
///   the final frame on show, cursor below it.
/// - `Text` is the low-write mode: no escape sequences at all, Tier 3
///   text deltas only.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DisplayMode {
    /// No explicit choice: the pre-mode behaviour.
    #[default]
    Auto,
    /// Cursor painting inside the alternate screen buffer.
    Alternate,
    /// Cursor painting in the primary buffer; the final frame stays.
    Primary,
    /// Line-oriented text deltas; never an escape byte.
    Text,
}

impl DisplayMode {
    /// The SSH environment variable that carries the choice (D3).
    pub const ENV_NAME: &str = "ID_DISPLAY";

    /// Parse a negotiated value. `None` is an unknown value: the shell
    /// refuses the join rather than guessing (D3).
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "auto" => Some(Self::Auto),
            "alternate" => Some(Self::Alternate),
            "primary" => Some(Self::Primary),
            "text" => Some(Self::Text),
            _ => None,
        }
    }

    /// The negotiated name of this mode.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Alternate => "alternate",
            Self::Primary => "primary",
            Self::Text => "text",
        }
    }

    /// Whether the session paints inside the alternate screen buffer.
    /// Only raw sessions consult this; command mode never enters it.
    #[must_use]
    pub fn uses_alt_screen(self) -> bool {
        matches!(self, Self::Auto | Self::Alternate)
    }

    /// The presentation tier a completed probe yields under this mode:
    /// the probe's own verdict for `Auto`; at least Tier 2 for the
    /// cursor modes, with Tier 1 kept only on the probe's positive
    /// evidence; Tier 3 for `Text`, however capable the terminal is.
    #[must_use]
    pub fn tier(self, outcome: &ProbeOutcome) -> PresentationTier {
        tier_for_mode(self, outcome.profile, outcome.tier())
    }
}

/// The tier rule behind [`DisplayMode::tier`], shared with the
/// presenter constructors that only hold a profile.
fn tier_for_mode(
    mode: DisplayMode,
    profile: TerminalProfile,
    probed: PresentationTier,
) -> PresentationTier {
    match mode {
        DisplayMode::Auto => probed,
        DisplayMode::Alternate | DisplayMode::Primary => match profile.tier() {
            PresentationTier::Tier1 => PresentationTier::Tier1,
            _ => PresentationTier::Tier2,
        },
        DisplayMode::Text => PresentationTier::Tier3,
    }
}

/// Profile plus input bytes that were not complete matching query replies.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProbeOutcome {
    /// Locally selected terminal profile.
    pub profile: TerminalProfile,
    /// Unmatched or incomplete bytes, preserved byte-for-byte for key input.
    pub replay: Vec<u8>,
    /// At least one capability received contradictory complete reports.
    pub(crate) conflicted: bool,
}

impl ProbeOutcome {
    /// Return the conservative Tier 3 profile when complete reports conflict.
    pub fn tier(&self) -> PresentationTier {
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
    pub fn tier(&self) -> PresentationTier {
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
pub struct PresenterCounters {
    /// Number of frames emitted by this presenter.
    pub frames: u64,
    /// Number of those frames emitted as complete renders.
    pub full_render_frames: u64,
    /// Total bytes written by this presenter.
    pub presenter_bytes: u64,
}

impl PresenterCounters {
    /// Record one successful presenter write.
    pub fn record(&mut self, bytes: usize, full_render: bool) {
        self.frames = self.frames.saturating_add(1);
        self.presenter_bytes = self.presenter_bytes.saturating_add(bytes as u64);
        if full_render {
            self.full_render_frames = self.full_render_frames.saturating_add(1);
        }
    }
}

/// Synchronized-output begin sequence (DEC private mode 2026, set).
pub const SYNC_OUTPUT_BEGIN: &str = "\x1b[?2026h";
/// Synchronized-output end sequence (DEC private mode 2026, reset).
pub const SYNC_OUTPUT_END: &str = "\x1b[?2026l";
/// Largest image a presenter will encode, in pixels. Presenter-owned
/// fixtures only: F13.4 adds no guest-authored image payload.
pub const MAX_GRAPHICS_PIXELS: usize = 65_536;
/// Largest Sixel palette a presenter will emit.
pub const MAX_SIXEL_COLORS: usize = 256;
/// Largest single Kitty graphics chunk, in base64 characters.
const KITTY_CHUNK_CHARS: usize = 4096;

/// A line-mode delta between two consecutive text views (F13.4 Tier 3).
///
/// The delta is the logical form of the Tier 3 presentation: applying it
/// to the previous view's lines yields exactly the next view's lines, so
/// parity against the full text is checkable without a terminal. The
/// rendered form emits only the changed lines, in order; a line-mode
/// terminal scrolls, so removals are carried by `line_count` in the
/// logical form rather than by erase sequences in the byte stream.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TextDelta {
    /// Line count of the next view (apply resizes to this first).
    pub line_count: usize,
    /// Replacement lines, by index, in ascending order.
    pub changed: Vec<(usize, String)>,
    /// True when the previous view was empty: this delta is a full render.
    pub full: bool,
}

impl TextDelta {
    /// The delta that turns `previous` into `next`.
    #[must_use]
    pub fn between(previous: &[String], next: &[String]) -> Self {
        let changed = next
            .iter()
            .enumerate()
            .filter(|(index, line)| previous.get(*index) != Some(line))
            .map(|(index, line)| (index, line.clone()))
            .collect();
        Self {
            line_count: next.len(),
            changed,
            full: previous.is_empty(),
        }
    }

    /// Apply this delta to `previous`, yielding the next view's lines.
    /// Oracle for Tier 3 parity: `delta.apply(prev) == next` whenever
    /// `delta == TextDelta::between(prev, next)`.
    #[must_use]
    pub fn apply(&self, previous: &[String]) -> Vec<String> {
        let mut lines = previous.to_vec();
        lines.resize(self.line_count, String::new());
        for (index, line) in &self.changed {
            if let Some(slot) = lines.get_mut(*index) {
                *slot = line.clone();
            }
        }
        lines
    }

    /// True when nothing changed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty()
    }

    /// Rendered form: the changed lines only, each CRLF-terminated.
    #[must_use]
    pub fn render(&self) -> String {
        let mut output = String::new();
        for (_, line) in &self.changed {
            output.push_str(line);
            output.push_str("\r\n");
        }
        output
    }
}

/// The tier presenter shared by the SSH PTY and interactive iroh TTY
/// adapters (F13.4 phases 3–4). One presenter owns one terminal: it is
/// built from the probe outcome, renders each coalesced view for its
/// tier, and restores terminal modes on [`TierPresenter::finish`].
///
/// - Tier 1 renders the cell grid like Tier 2, bracketed in synchronized
///   output, and may encode presenter-owned images — only behind the
///   positive feature evidence that selected the tier.
/// - Tier 2 renders the cell grid: a full render first, then row diffs
///   against the previous screen (full repaint when a diff is impossible,
///   e.g. after a resize or [`TierPresenter::invalidate`]).
/// - Tier 3 renders text only: no full-screen escapes, and after the
///   first view only the changed lines (a [`TextDelta`]). A view whose
///   dimensions differ from the previous one is a full frame again
///   (the join/resize form, #14937).
///
/// Under [`DisplayMode::Primary`] the cell-grid tiers paint with the
/// primary-mode contract instead ([`ScreenSurface::paint_primary`]):
/// no alternate screen is involved, the previous frame stays in
/// scrollback, and each transition is cursor-addressed erase/rewrite
/// runs over it. The mode never selects Tier 3, so the text path is
/// unreachable for it.
///
/// Incremental state is committed only after a render is produced, so a
/// caller that drops a frame can re-render the same view safely.
#[derive(Debug)]
pub struct TierPresenter {
    profile: TerminalProfile,
    tier: PresentationTier,
    mode: DisplayMode,
    previous_screen: Option<ScreenSurface>,
    previous_text: Vec<String>,
    /// Dimensions of the last screen this presenter projected to text;
    /// a change forces the text path's full-frame form (#14937).
    previous_text_dims: Option<(u16, u16)>,
    counters: PresenterCounters,
    sync_used: bool,
}

impl TierPresenter {
    /// A presenter for a directly supplied profile (tests, adapters that
    /// probed through [`TerminalNegotiator`]).
    #[must_use]
    pub fn new(profile: TerminalProfile) -> Self {
        Self {
            tier: profile.tier(),
            profile,
            mode: DisplayMode::Auto,
            previous_screen: None,
            previous_text: Vec::new(),
            previous_text_dims: None,
            counters: PresenterCounters::default(),
            sync_used: false,
        }
    }

    /// A presenter for a completed probe. A conflicted outcome selects
    /// the conservative Tier 3, exactly as [`ProbeOutcome::tier`] rules.
    #[must_use]
    pub fn from_outcome(outcome: &ProbeOutcome) -> Self {
        Self {
            tier: outcome.tier(),
            profile: outcome.profile,
            mode: DisplayMode::Auto,
            previous_screen: None,
            previous_text: Vec::new(),
            previous_text_dims: None,
            counters: PresenterCounters::default(),
            sync_used: false,
        }
    }

    /// A presenter for a completed probe under a chosen [`DisplayMode`]
    /// (#14861 D2). `Auto` is exactly [`TierPresenter::from_outcome`];
    /// the explicit modes apply the mode's tier rule
    /// ([`DisplayMode::tier`]) to the probed profile.
    #[must_use]
    pub fn from_outcome_for_mode(outcome: &ProbeOutcome, mode: DisplayMode) -> Self {
        let mut presenter = Self::from_outcome(outcome);
        presenter.tier = mode.tier(outcome);
        presenter.mode = mode;
        presenter
    }

    /// A presenter for a directly supplied profile under a chosen
    /// [`DisplayMode`], for adapters that must paint before any probe
    /// completes: a text session may never emit an escape byte, so it
    /// cannot wait for the probe to learn it is Tier 3.
    #[must_use]
    pub fn for_mode(profile: TerminalProfile, mode: DisplayMode) -> Self {
        let mut presenter = Self::new(profile);
        presenter.tier = tier_for_mode(mode, profile, profile.tier());
        presenter.mode = mode;
        presenter
    }

    /// The tier this presenter renders.
    #[must_use]
    pub fn tier(&self) -> PresentationTier {
        self.tier
    }

    /// The display mode this presenter paints for.
    #[must_use]
    pub fn mode(&self) -> DisplayMode {
        self.mode
    }

    /// The profile this presenter was built from.
    #[must_use]
    pub fn profile(&self) -> TerminalProfile {
        self.profile
    }

    /// Frames and bytes emitted so far, for baseline comparisons.
    #[must_use]
    pub fn counters(&self) -> PresenterCounters {
        self.counters
    }

    /// Force the next frame to be a full render (attach, resize, or any
    /// invalidation of the diff state).
    pub fn invalidate(&mut self) {
        self.previous_screen = None;
        self.previous_text = Vec::new();
        self.previous_text_dims = None;
    }

    /// Render one screen view for this presenter's tier.
    pub fn render_screen(&mut self, screen: &ScreenSurface) -> String {
        if self.tier == PresentationTier::Tier3 {
            // A size change is answered with the terse full-frame form
            // (#14937): the text path's full form is a diff against
            // nothing, so a resize drops the remembered lines before
            // diffing, exactly as a join does. The shell forwards the
            // resize to the world, and the world's answering view
            // carries the new dimensions, so the presenter sees every
            // resize here.
            let dims = (screen.width, screen.height);
            if self
                .previous_text_dims
                .is_some_and(|previous| previous != dims)
            {
                self.previous_text.clear();
            }
            self.previous_text_dims = Some(dims);
            let projection = screen.text_projection();
            let lines: Vec<String> = if projection.is_empty() {
                Vec::new()
            } else {
                projection.lines().map(str::to_owned).collect()
            };
            return self.render_lines(&lines);
        }
        let (body, full) = if self.mode == DisplayMode::Primary {
            // Primary mode paints with the slice-1 harness contract
            // (Term.paint_primary): cursor-addressed erase/rewrite runs
            // over the previous frame, which stays in scrollback. A
            // dimension or version change is answered with a full
            // repaint, per the contract's host guidance; an unchanged
            // frame paints as the empty string.
            let previous = self.previous_screen.as_ref().filter(|previous| {
                previous.width == screen.width
                    && previous.height == screen.height
                    && previous.version == screen.version
            });
            match previous.and_then(|previous| screen.paint_primary(previous)) {
                Some(diff) => (diff, false),
                None => match screen.paint_primary_full() {
                    Some(full) => (full, true),
                    None => return String::new(),
                },
            }
        } else {
            match self
                .previous_screen
                .as_ref()
                .and_then(|previous| screen.render_ansi_diff(previous))
            {
                Some(diff) => (diff, false),
                None => match screen.render_ansi() {
                    Some(full) => (full, true),
                    None => return String::new(),
                },
            }
        };
        self.previous_screen = Some(screen.clone());
        if body.is_empty() {
            return String::new();
        }
        let output = if self.tier == PresentationTier::Tier1 && self.profile.synchronized_output {
            self.sync_used = true;
            format!("{SYNC_OUTPUT_BEGIN}{body}{SYNC_OUTPUT_END}")
        } else {
            body
        };
        self.counters.record(output.len(), full);
        output
    }

    /// Render one text view as a Tier 3 delta and return the rendered
    /// bytes (empty when nothing changed).
    pub fn render_lines(&mut self, lines: &[String]) -> String {
        self.render_text_delta(lines).render()
    }

    /// The Tier 3 delta for one text view; the logical form behind
    /// [`TierPresenter::render_lines`], exposed so adapters and tests
    /// can verify apply-parity against the full text.
    pub fn render_text_delta(&mut self, lines: &[String]) -> TextDelta {
        let delta = TextDelta::between(&self.previous_text, lines);
        self.previous_text = lines.to_vec();
        if !delta.is_empty() {
            self.counters.record(delta.render().len(), delta.full);
        }
        delta
    }

    /// Encode a presenter-owned image fixture for this terminal, or
    /// `None` when the tier or the confirmed features do not allow
    /// graphics. Kitty graphics is preferred when both were confirmed;
    /// Sixel is the fallback. Never called with guest-supplied payloads:
    /// F13.4 keeps the guest view format unchanged.
    pub fn present_image(&mut self, rgb: &[u8], width: usize, height: usize) -> Option<String> {
        if self.tier != PresentationTier::Tier1 {
            return None;
        }
        let encoded = if self.profile.kitty_graphics {
            encode_kitty_graphics(rgb, width, height)?
        } else if self.profile.sixel_graphics {
            encode_sixel(rgb, width, height)?
        } else {
            return None;
        };
        let output = if self.profile.synchronized_output {
            self.sync_used = true;
            format!("{SYNC_OUTPUT_BEGIN}{encoded}{SYNC_OUTPUT_END}")
        } else {
            encoded
        };
        self.counters.record(output.len(), false);
        Some(output)
    }

    /// The kitty graphics stills gate for this session (#13623): open for a
    /// payload class only when the probe saw kitty graphics AND that
    /// class's own query in the probe burst answered OK.
    #[must_use]
    pub fn kitty_gate(&self) -> GraphicsGate {
        let mut gate = GraphicsGate::new(self.profile.kitty_graphics);
        for (class, accepted) in [
            (PayloadClass::Png, self.profile.kitty_png),
            (PayloadClass::Rgba, self.profile.kitty_rgba),
            (PayloadClass::RgbaZlib, self.profile.kitty_rgba_zlib),
        ] {
            gate.record_answer(class, accepted);
        }
        gate
    }

    /// Encode a presenter-owned still for this terminal through the kitty
    /// graphics stills encoder, or `Ok(None)` when nothing may be written:
    /// the presenter is not Tier 1, or this terminal did not accept the
    /// payload's class. As with [`TierPresenter::present_image`], guest
    /// payloads never reach this; the frame is bracketed in synchronized
    /// output when the terminal confirmed it.
    ///
    /// # Errors
    ///
    /// [`KittyGraphicsError`] for a malformed or over-cap payload, checked
    /// only once the gate is open. Nothing is emitted on error.
    pub fn present_still(
        &mut self,
        image_id: NonZeroU32,
        payload: StillPayload<'_>,
        action: Action,
    ) -> Result<Option<String>, KittyGraphicsError> {
        if self.tier != PresentationTier::Tier1 {
            return Ok(None);
        }
        let Some(encoded) = self.kitty_gate().encode_still(image_id, payload, action)? else {
            return Ok(None);
        };
        let output = if self.profile.synchronized_output {
            self.sync_used = true;
            format!("{SYNC_OUTPUT_BEGIN}{encoded}{SYNC_OUTPUT_END}")
        } else {
            encoded
        };
        self.counters.record(output.len(), false);
        Ok(Some(output))
    }

    /// End the presentation: returns the mode-restore bytes the adapter
    /// must write on every exit path (success, error, disconnect) and
    /// resets the diff state. Synchronized output, once used, is always
    /// left here even though frames are individually bracketed — a write
    /// interrupted mid-frame must not strand the terminal in mode 2026.
    pub fn finish(&mut self) -> String {
        self.previous_screen = None;
        if self.sync_used {
            self.sync_used = false;
            SYNC_OUTPUT_END.to_owned()
        } else {
            String::new()
        }
    }
}

/// Encode an RGB image (3 bytes per pixel, row-major) as a chunked
/// Kitty graphics transmission (`a=T`, `f=24`, id 31 — the id the probe
/// query used). `None` for empty, oversized, or mis-sized input.
#[must_use]
pub fn encode_kitty_graphics(rgb: &[u8], width: usize, height: usize) -> Option<String> {
    let pixels = width.checked_mul(height)?;
    if pixels == 0 || pixels > MAX_GRAPHICS_PIXELS || rgb.len() != pixels.checked_mul(3)? {
        return None;
    }
    let payload = base64_encode(rgb);
    let mut output = String::new();
    let mut chunks = payload.as_bytes().chunks(KITTY_CHUNK_CHARS).peekable();
    let mut first = true;
    while let Some(chunk) = chunks.next() {
        let more = u8::from(chunks.peek().is_some());
        if first {
            output.push_str(&format!(
                "\x1b_Ga=T,f=24,s={width},v={height},i=31,m={more};"
            ));
            first = false;
        } else {
            output.push_str(&format!("\x1b_Gm={more};"));
        }
        output.push_str(std::str::from_utf8(chunk).ok()?);
        output.push_str("\x1b\\");
    }
    Some(output)
}

/// Encode an RGB image as Sixel graphics. Each distinct color becomes
/// one palette register (at most [`MAX_SIXEL_COLORS`]); each 6-row band
/// emits one pass per color present in the band. `None` for empty,
/// oversized, mis-sized, or over-palette input.
#[must_use]
pub fn encode_sixel(rgb: &[u8], width: usize, height: usize) -> Option<String> {
    let pixels = width.checked_mul(height)?;
    if pixels == 0 || pixels > MAX_GRAPHICS_PIXELS || rgb.len() != pixels.checked_mul(3)? {
        return None;
    }
    let mut colors: Vec<(u8, u8, u8)> = Vec::new();
    let mut index = vec![0_usize; pixels];
    for (i, pixel) in rgb.as_chunks::<3>().0.iter().enumerate() {
        let key = (pixel[0], pixel[1], pixel[2]);
        index[i] = match colors.iter().position(|color| *color == key) {
            Some(position) => position,
            None => {
                if colors.len() >= MAX_SIXEL_COLORS {
                    return None;
                }
                colors.push(key);
                colors.len() - 1
            }
        };
    }
    let percent = |value: u8| u32::from(value) * 100 / 255;
    let mut output = String::from("\x1bPq");
    output.push_str(&format!("\"1;1;{width};{height}"));
    for (i, (r, g, b)) in colors.iter().enumerate() {
        output.push_str(&format!(
            "#{i};2;{};{};{}",
            percent(*r),
            percent(*g),
            percent(*b)
        ));
    }
    for (band, top) in (0..height).step_by(6).enumerate() {
        if band > 0 {
            output.push('-');
        }
        let rows = (height - top).min(6);
        let mut band_colors: Vec<usize> = Vec::new();
        for y in top..top + rows {
            for x in 0..width {
                let color = index[y * width + x];
                if !band_colors.contains(&color) {
                    band_colors.push(color);
                }
            }
        }
        band_colors.sort_unstable();
        for (pass, color) in band_colors.iter().enumerate() {
            if pass > 0 {
                output.push('$');
            }
            output.push_str(&format!("#{color}"));
            for x in 0..width {
                let mut bits = 0_u8;
                for dy in 0..rows {
                    if index[(top + dy) * width + x] == *color {
                        bits |= 1 << dy;
                    }
                }
                output.push(char::from(0x3f + bits));
            }
        }
    }
    output.push_str("\x1b\\");
    Some(output)
}

/// Standard base64 (RFC 4648, with padding), the Kitty payload encoding.
fn base64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = u32::from(*chunk.get(1).unwrap_or(&0));
        let b2 = u32::from(*chunk.get(2).unwrap_or(&0));
        let triple = (b0 << 16) | (b1 << 8) | b2;
        output.push(char::from(ALPHABET[(triple >> 18) as usize & 0x3f]));
        output.push(char::from(ALPHABET[(triple >> 12) as usize & 0x3f]));
        output.push(if chunk.len() > 1 {
            char::from(ALPHABET[(triple >> 6) as usize & 0x3f])
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            char::from(ALPHABET[triple as usize & 0x3f])
        } else {
            '='
        });
    }
    output
}

#[derive(Clone, Copy, Debug, Default)]
struct Evidence {
    cursor_position_report: Option<bool>,
    indexed_color: Option<bool>,
    kitty_graphics: Option<bool>,
    sixel_graphics: Option<bool>,
    synchronized_output: Option<bool>,
    kitty_keyboard: Option<bool>,
    kitty_png: Option<bool>,
    kitty_rgba: Option<bool>,
    kitty_rgba_zlib: Option<bool>,
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
            kitty_png: self.kitty_png.unwrap_or(false),
            kitty_rgba: self.kitty_rgba.unwrap_or(false),
            kitty_rgba_zlib: self.kitty_rgba_zlib.unwrap_or(false),
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
    // ID 31 is the detection query; 41 to 43 are the payload-class queries.
    let Some((id, status)) = body.strip_prefix(b"Gi=").and_then(|rest| {
        rest.iter()
            .position(|&b| b == b';')
            .map(|at| rest.split_at(at))
    }) else {
        return false;
    };
    let slot = match id {
        b"31" => &mut evidence.kitty_graphics,
        b"41" => &mut evidence.kitty_png,
        b"42" => &mut evidence.kitty_rgba,
        b"43" => &mut evidence.kitty_rgba_zlib,
        _ => return false,
    };
    Evidence::record(slot, &status[1..] == b"OK", &mut evidence.conflicted);
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
        .as_chunks::<2>()
        .0
        .iter()
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
                b"\x1b_Gi=41;OK\x1b\\".as_slice(),
                b"\x1b_Gi=43;ENOTSUPPORTED:no zlib\x1b\\".as_slice(),
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
                kitty_png: true,
                kitty_rgba: false,
                kitty_rgba_zlib: false,
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
    fn the_probe_burst_ends_with_the_payload_class_queries() {
        let tail = [
            crate::kitty_graphics::PNG_QUERY,
            crate::kitty_graphics::RGBA_QUERY,
            crate::kitty_graphics::RGBA_ZLIB_QUERY,
        ]
        .concat();
        assert!(PROBE_QUERIES.ends_with(tail.as_bytes()));
    }

    #[test]
    fn payload_class_replies_are_consumed_and_recorded_per_class() {
        let outcome = parse_transcript(
            &[
                KITTY_GRAPHICS_REPLY,
                b"\x1b_Gi=42;OK\x1b\\".as_slice(),
                b"\x1b_Gi=43;OK\x1b\\".as_slice(),
                b"x".as_slice(),
                // Not one of our query IDs: user input, not evidence.
                b"\x1b_Gi=44;OK\x1b\\".as_slice(),
            ]
            .concat(),
        );
        assert!(outcome.profile.kitty_graphics);
        assert!(!outcome.profile.kitty_png);
        assert!(outcome.profile.kitty_rgba);
        assert!(outcome.profile.kitty_rgba_zlib);
        assert_eq!(outcome.replay, b"x\x1b_Gi=44;OK\x1b\\");

        // Contradictory answers to one class are a conflict: Tier 3 defaults.
        let conflicted = parse_transcript(
            &[
                KITTY_GRAPHICS_REPLY,
                b"\x1b_Gi=41;OK\x1b\\".as_slice(),
                b"\x1b_Gi=41;EINVAL\x1b\\".as_slice(),
            ]
            .concat(),
        );
        assert_eq!(conflicted.profile, TerminalProfile::default());
    }

    #[test]
    fn present_still_needs_tier_one_and_an_accepted_class() {
        use crate::kitty_graphics::{Action, StillPayload};
        let png: Vec<u8> = {
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD
                .decode(
                    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==",
                )
                .expect("fixture decodes")
        };
        let id = NonZeroU32::new(7).expect("non-zero");
        let rich = TerminalProfile {
            kitty_graphics: true,
            synchronized_output: true,
            ..TerminalProfile::default()
        };

        // Detection without the class answer: nothing.
        let mut detected = TierPresenter::new(rich);
        assert_eq!(detected.tier(), PresentationTier::Tier1);
        assert_eq!(
            detected.present_still(id, StillPayload::Png(&png), Action::Store),
            Ok(None)
        );

        // The class answered OK but the presenter is not Tier 1: nothing.
        let mut not_tier_one = TierPresenter::new(TerminalProfile {
            kitty_graphics: true,
            kitty_png: true,
            ..TerminalProfile::default()
        });
        assert_eq!(
            not_tier_one.present_still(id, StillPayload::Png(&png), Action::Store),
            Ok(None)
        );

        // Tier 1 and PNG accepted: one quiet transmission in sync brackets.
        let mut accepted = TierPresenter::new(TerminalProfile {
            kitty_png: true,
            ..rich
        });
        let frame = accepted
            .present_still(id, StillPayload::Png(&png), Action::Store)
            .expect("valid payload")
            .expect("gate open");
        assert!(frame.starts_with("\x1b[?2026h\x1b_Ga=t,f=100,t=d,i=7,q=2,m=0;iVBOR"));
        assert!(frame.ends_with("\x1b\\\x1b[?2026l"));
        assert_eq!(accepted.finish(), SYNC_OUTPUT_END);

        // An accepted class does not open another one.
        let rgba = [0_u8; 4];
        assert_eq!(
            accepted.present_still(
                id,
                StillPayload::Rgba {
                    data: &rgba,
                    width: 1,
                    height: 1,
                    compress: false,
                },
                Action::Store,
            ),
            Ok(None)
        );
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
    fn display_mode_names_round_trip_and_unknown_values_are_refused() {
        // Oracle: design D3 names exactly four negotiated values; any
        // other value is unknown and must not parse, so the shell can
        // refuse the join instead of guessing a mode.
        for (value, mode) in [
            ("auto", DisplayMode::Auto),
            ("alternate", DisplayMode::Alternate),
            ("primary", DisplayMode::Primary),
            ("text", DisplayMode::Text),
        ] {
            assert_eq!(DisplayMode::parse(value), Some(mode));
            assert_eq!(mode.name(), value);
        }
        assert_eq!(DisplayMode::parse(""), None);
        assert_eq!(DisplayMode::parse("PRIMARY"), None);
        assert_eq!(DisplayMode::parse("sideways"), None);
    }

    #[test]
    fn display_mode_tier_rules_follow_the_design_table() {
        // Oracle: design D2 — cursor modes paint at at least Tier 2,
        // Tier 1 extras only on the probe's positive evidence, and
        // text forces Tier 3 however capable the terminal is.
        let tier1 = parse_transcript(&[SIXEL_DA_REPLY, SYNC_OUTPUT_REPLY].concat());
        let tier2 = parse_transcript(&[COLOR_COUNT_REPLY, CURSOR_POSITION_REPLY].concat());
        let tier3 = parse_transcript(&[]);

        assert_eq!(DisplayMode::Auto.tier(&tier1), PresentationTier::Tier1);
        assert_eq!(DisplayMode::Auto.tier(&tier3), PresentationTier::Tier3);

        for cursor_mode in [DisplayMode::Alternate, DisplayMode::Primary] {
            assert_eq!(cursor_mode.tier(&tier3), PresentationTier::Tier2);
            assert_eq!(cursor_mode.tier(&tier2), PresentationTier::Tier2);
            assert_eq!(cursor_mode.tier(&tier1), PresentationTier::Tier1);
        }

        assert_eq!(DisplayMode::Text.tier(&tier1), PresentationTier::Tier3);
        assert_eq!(DisplayMode::Text.tier(&tier3), PresentationTier::Tier3);

        // The presenter constructors apply the same rule.
        assert_eq!(
            TierPresenter::from_outcome_for_mode(&tier3, DisplayMode::Primary).tier(),
            PresentationTier::Tier2
        );
        assert_eq!(
            TierPresenter::from_outcome_for_mode(&tier1, DisplayMode::Text).tier(),
            PresentationTier::Tier3
        );
        assert_eq!(
            TierPresenter::from_outcome_for_mode(&tier1, DisplayMode::Auto).tier(),
            TierPresenter::from_outcome(&tier1).tier()
        );
        assert_eq!(
            TierPresenter::for_mode(TerminalProfile::default(), DisplayMode::Text).tier(),
            PresentationTier::Tier3
        );
        assert_eq!(
            TierPresenter::for_mode(TerminalProfile::default(), DisplayMode::Alternate).tier(),
            PresentationTier::Tier2
        );
    }

    #[test]
    fn text_mode_presenter_output_never_contains_an_escape_byte() {
        // Oracle: design D2 — in text mode no escape sequence is ever
        // written, even for the first (full) frame.
        let mut presenter = TierPresenter::for_mode(TerminalProfile::default(), DisplayMode::Text);
        let first = presenter.render_screen(&screen(&["hello", "world"]));
        let second = presenter.render_screen(&screen(&["hello", "there"]));

        assert!(!first.contains('\x1b'), "first frame escaped: {first:?}");
        assert!(!second.contains('\x1b'), "delta frame escaped: {second:?}");
        assert!(
            first.contains("hello"),
            "first frame lost its text: {first:?}"
        );
    }

    #[test]
    fn text_presenter_answers_a_resize_with_a_full_frame() {
        // Oracle: issue #14937 — a text session renders each frame as
        // plain text lines, emitting only the lines that changed, with
        // a terse full-frame form on join and on resize. The full form
        // is exactly the changed-lines form computed against nothing,
        // so applying it over any prior frame state yields the new
        // frame (TextDelta::apply parity).
        let mut presenter = TierPresenter::for_mode(TerminalProfile::default(), DisplayMode::Text);

        let joined = presenter.render_screen(&screen(&["alpha", "beta"]));
        assert_eq!(joined, "alpha\r\nbeta\r\n", "join is a full frame");

        let quiet = presenter.render_screen(&screen(&["alpha", "beta"]));
        assert_eq!(quiet, "", "an unchanged frame costs zero bytes");

        let delta = presenter.render_screen(&screen(&["alpha", "gamma"]));
        assert_eq!(delta, "gamma\r\n", "only the changed line is emitted");

        // Resize: the same leading lines at a new size still repaints
        // in full, because the reader's frame state no longer matches.
        let resized = presenter.render_screen(&screen(&["alpha", "beta", "extra"]));
        assert_eq!(
            resized, "alpha\r\nbeta\r\nextra\r\n",
            "a resize answers with the full-frame form"
        );
        assert!(!resized.contains('\x1b'), "full frame escaped: {resized:?}");

        // After the resize frame, deltas resume against the new size.
        let after = presenter.render_screen(&screen(&["alpha", "beta", "extra"]));
        assert_eq!(after, "", "post-resize frame is a delta again");
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

    fn screen(rows: &[&str]) -> ScreenSurface {
        let height = rows.len();
        let width = rows
            .iter()
            .map(|row| row.chars().count())
            .max()
            .unwrap_or(0);
        let mut cells = Vec::new();
        for row in rows {
            let mut count = 0;
            for ch in row.chars() {
                cells.push(crate::world_presentation::ScreenCell {
                    symbol: ch.to_string(),
                    fg: 0,
                    bg: 0,
                    bold: false,
                    columns: 1,
                    token: None,
                });
                count += 1;
            }
            for _ in count..width {
                cells.push(crate::world_presentation::ScreenCell {
                    symbol: " ".to_string(),
                    fg: 0,
                    bg: 0,
                    bold: false,
                    columns: 1,
                    token: None,
                });
            }
        }
        ScreenSurface::from_cells(width, height, cells).expect("valid screen fixture")
    }

    fn base64_decode(payload: &str) -> Vec<u8> {
        let value = |byte: u8| -> u32 {
            match byte {
                b'A'..=b'Z' => u32::from(byte - b'A'),
                b'a'..=b'z' => u32::from(byte - b'a') + 26,
                b'0'..=b'9' => u32::from(byte - b'0') + 52,
                b'+' => 62,
                b'/' => 63,
                _ => 0,
            }
        };
        let mut output = Vec::new();
        for chunk in payload.as_bytes().chunks(4) {
            let pad = chunk.iter().filter(|byte| **byte == b'=').count();
            let triple = chunk
                .iter()
                .fold(0_u32, |acc, byte| (acc << 6) | value(*byte));
            output.push((triple >> 16) as u8);
            if pad < 2 {
                output.push((triple >> 8) as u8);
            }
            if pad < 1 {
                output.push(triple as u8);
            }
        }
        output
    }

    #[test]
    fn tier_two_presenter_diffs_against_previous_and_matches_the_patch_oracle() {
        let profile = TerminalProfile {
            cursor_position_report: true,
            indexed_color: true,
            ..TerminalProfile::default()
        };
        let mut presenter = TierPresenter::new(profile);
        assert_eq!(presenter.tier(), PresentationTier::Tier2);

        let first = screen(&["abc", "def"]);
        let second = screen(&["abc", "dXf"]);
        let full = presenter.render_screen(&first);
        assert_eq!(full, first.render_ansi().expect("full render"));
        let diff = presenter.render_screen(&second);
        assert_eq!(diff, second.render_ansi_diff(&first).expect("diff render"));
        assert!(diff.len() < second.render_ansi().expect("full render").len());
        // Oracle: the transport-neutral patch applied to frame 1 is frame 2.
        let patch =
            crate::world_presentation::ScreenPatch::diff(&first, &second).expect("patch exists");
        assert_eq!(patch.apply(&first), Some(second.clone()));
        // An unchanged view renders nothing and records no frame.
        assert_eq!(presenter.render_screen(&second), "");
        assert_eq!(presenter.counters().frames, 2);
        assert_eq!(presenter.counters().full_render_frames, 1);
        // Invalidation (resize/attach) forces a full repaint.
        presenter.invalidate();
        assert_eq!(
            presenter.render_screen(&second),
            second.render_ansi().expect("full render")
        );
    }

    /// Slice 2 of #14861: a primary-mode presenter paints the Term
    /// contract bytes — a full primary repaint first, then
    /// cursor-addressed erase/rewrite transitions over the previous
    /// frame — and never touches the alternate screen.
    #[test]
    fn primary_presenter_paints_the_term_contract_over_the_previous_frame() {
        let profile = TerminalProfile {
            cursor_position_report: true,
            indexed_color: true,
            ..TerminalProfile::default()
        };
        let mut presenter = TierPresenter::for_mode(profile, DisplayMode::Primary);
        assert_eq!(presenter.mode(), DisplayMode::Primary);
        assert_eq!(presenter.tier(), PresentationTier::Tier2);

        let first = screen(&["abc", "def"]);
        let second = screen(&["abc", "dXf"]);
        // First frame: a full primary repaint (page erase + runs painted
        // from a blank screen).
        assert_eq!(
            presenter.render_screen(&first),
            first.paint_primary_full().expect("full primary paint")
        );
        // Transition: exactly the contract bytes over the previous frame.
        assert_eq!(
            presenter.render_screen(&second),
            second.paint_primary(&first).expect("primary transition")
        );
        assert_eq!(
            second.paint_primary(&first).expect("primary transition"),
            "\x1b[2;2H\x1b[0mX\x1b[0m"
        );
        // An unchanged view paints nothing and records no frame.
        assert_eq!(presenter.render_screen(&second), "");
        assert_eq!(presenter.counters().frames, 2);
        assert_eq!(presenter.counters().full_render_frames, 1);
        // A resize is answered with a full repaint, per the contract's
        // host guidance.
        let resized = screen(&["abc"]);
        assert_eq!(
            presenter.render_screen(&resized),
            resized.paint_primary_full().expect("full primary paint")
        );
        // Invalidation (attach) also forces a full repaint.
        presenter.invalidate();
        assert_eq!(
            presenter.render_screen(&resized),
            resized.paint_primary_full().expect("full primary paint")
        );
    }

    #[test]
    fn primary_presenter_never_enters_the_alternate_screen() {
        let mut presenter =
            TierPresenter::for_mode(TerminalProfile::default(), DisplayMode::Primary);
        let out = presenter.render_screen(&screen(&["hi"]));
        assert!(!out.contains("\x1b[?1049h"), "{out:?}");
        assert!(!out.contains("\x1b[?1049l"), "{out:?}");
        // The alternate painter's full render homes before erasing; the
        // primary full repaint erases in place.
        assert!(out.starts_with("\x1b[2J"), "{out:?}");
    }

    #[test]
    fn tier_one_presenter_brackets_synchronized_output_and_restores_on_finish() {
        let profile = TerminalProfile {
            cursor_position_report: true,
            indexed_color: true,
            kitty_graphics: true,
            synchronized_output: true,
            ..TerminalProfile::default()
        };
        let mut presenter = TierPresenter::new(profile);
        assert_eq!(presenter.tier(), PresentationTier::Tier1);

        let frame = presenter.render_screen(&screen(&["hi"]));
        assert!(frame.starts_with(SYNC_OUTPUT_BEGIN), "{frame:?}");
        assert!(frame.ends_with(SYNC_OUTPUT_END), "{frame:?}");
        // finish() leaves mode 2026 exactly once, on every exit path.
        assert_eq!(presenter.finish(), SYNC_OUTPUT_END);
        assert_eq!(presenter.finish(), "");
    }

    #[test]
    fn tier_three_presenter_emits_only_changed_lines_with_apply_parity() {
        let mut presenter = TierPresenter::new(TerminalProfile::default());
        assert_eq!(presenter.tier(), PresentationTier::Tier3);

        let v1 = vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()];
        let d1 = presenter.render_text_delta(&v1);
        assert!(d1.full);
        assert_eq!(d1.apply(&[]), v1);

        let v2 = vec!["alpha".to_string(), "BETA".to_string()];
        let d2 = presenter.render_text_delta(&v2);
        assert_eq!(d2.apply(&v1), v2);
        assert_eq!(d2.changed, vec![(1, "BETA".to_string())]);
        assert_eq!(d2.render(), "BETA\r\n");
        assert_eq!(presenter.render_lines(&v2), "");

        // The screen path is escape-free text, first frame in full.
        let mut screen_presenter = TierPresenter::new(TerminalProfile::default());
        let out = screen_presenter.render_screen(&screen(&["ab", "cd"]));
        assert_eq!(out, "ab\r\ncd\r\n");
        assert!(!out.contains('\x1b'));
    }

    /// F11.9's two coalesced views must use incremental rendering at every
    /// presentation tier. The full ANSI/text renderers are the byte-level
    /// oracles; the second identical view must emit nothing.
    #[test]
    fn every_tier_renders_coalesced_views_incrementally() {
        let first = screen(&["abc", "def"]);
        let second = screen(&["abc", "dXf"]);
        let profiles = [
            (
                "tier one",
                TerminalProfile {
                    kitty_graphics: true,
                    synchronized_output: true,
                    ..TerminalProfile::default()
                },
                PresentationTier::Tier1,
            ),
            (
                "tier two",
                TerminalProfile {
                    cursor_position_report: true,
                    indexed_color: true,
                    ..TerminalProfile::default()
                },
                PresentationTier::Tier2,
            ),
            (
                "tier three",
                TerminalProfile::default(),
                PresentationTier::Tier3,
            ),
        ];

        for (name, profile, tier) in profiles {
            let mut presenter = TierPresenter::new(profile);
            assert_eq!(presenter.tier(), tier, "{name}: selected tier");

            let first_output = presenter.render_screen(&first);
            let second_output = presenter.render_screen(&second);
            let unchanged_output = presenter.render_screen(&second);

            match tier {
                PresentationTier::Tier1 => {
                    assert_eq!(
                        first_output,
                        format!(
                            "{SYNC_OUTPUT_BEGIN}{}{SYNC_OUTPUT_END}",
                            first.render_ansi().expect("full ANSI render")
                        ),
                        "{name}: first frame uses full-render oracle"
                    );
                    assert_eq!(
                        second_output,
                        format!(
                            "{SYNC_OUTPUT_BEGIN}{}{SYNC_OUTPUT_END}",
                            second.render_ansi_diff(&first).expect("ANSI diff")
                        ),
                        "{name}: next frame uses diff oracle inside sync output"
                    );
                }
                PresentationTier::Tier2 => {
                    assert_eq!(
                        first_output,
                        first.render_ansi().expect("full ANSI render"),
                        "{name}: first frame uses full-render oracle"
                    );
                    assert_eq!(
                        second_output,
                        second.render_ansi_diff(&first).expect("ANSI diff"),
                        "{name}: next frame uses diff oracle"
                    );
                }
                PresentationTier::Tier3 => {
                    assert_eq!(first_output, "abc\r\ndef\r\n", "{name}: full text oracle");
                    assert_eq!(second_output, "dXf\r\n", "{name}: changed-line oracle");
                }
            }

            assert_eq!(unchanged_output, "", "{name}: unchanged view is suppressed");
            assert_eq!(presenter.counters().frames, 2, "{name}: two emitted frames");
            assert_eq!(
                presenter.counters().full_render_frames,
                1,
                "{name}: one full frame"
            );
        }
    }

    #[test]
    fn graphics_are_gated_on_positive_tier_one_evidence() {
        // No evidence: nothing is emitted.
        let mut plain = TierPresenter::new(TerminalProfile::default());
        assert_eq!(plain.present_image(&[255, 0, 0], 1, 1), None);
        // Kitty evidence alone is not Tier 1 (synchronized output missing).
        let mut kitty_only = TierPresenter::new(TerminalProfile {
            kitty_graphics: true,
            ..TerminalProfile::default()
        });
        assert_eq!(kitty_only.tier(), PresentationTier::Tier3);
        assert_eq!(kitty_only.present_image(&[255, 0, 0], 1, 1), None);

        // Tier 1 via Kitty: one exact chunk inside sync brackets.
        let mut rich = TierPresenter::new(TerminalProfile {
            kitty_graphics: true,
            synchronized_output: true,
            ..TerminalProfile::default()
        });
        assert_eq!(rich.tier(), PresentationTier::Tier1);
        assert_eq!(
            rich.present_image(&[255, 0, 0], 1, 1).as_deref(),
            Some("\x1b[?2026h\x1b_Ga=T,f=24,s=1,v=1,i=31,m=0;/wAA\x1b\\\x1b[?2026l")
        );
        // Tier 1 via Sixel: the sixel encoder is selected instead.
        let mut sixel = TierPresenter::new(TerminalProfile {
            sixel_graphics: true,
            synchronized_output: true,
            ..TerminalProfile::default()
        });
        let frame = sixel
            .present_image(&[255, 0, 0], 1, 1)
            .expect("sixel frame");
        assert!(frame.contains("\x1bPq"), "{frame:?}");

        // Exact sixel oracle: a 2x6 all-red image is one color, one band.
        assert_eq!(
            encode_sixel(&[255, 0, 0].repeat(12), 2, 6).as_deref(),
            Some("\x1bPq\"1;1;2;6#0;2;100;0;0#0~~\x1b\\")
        );
        // Mis-sized and oversized inputs are refused.
        assert_eq!(encode_kitty_graphics(&[0, 0], 1, 1), None);
        assert_eq!(
            encode_sixel(
                &vec![0; 3 * (MAX_GRAPHICS_PIXELS + 1)],
                MAX_GRAPHICS_PIXELS + 1,
                1
            ),
            None
        );
    }

    #[test]
    fn kitty_graphics_chunks_round_trip_to_the_source_pixels() {
        let rgb: Vec<u8> = (0..64 * 64 * 3).map(|i| (i % 251) as u8).collect();
        let encoded = encode_kitty_graphics(&rgb, 64, 64).expect("encode");
        let mut payload = Vec::new();
        let mut rest = encoded.as_str();
        let mut chunks = 0;
        while let Some(start) = rest.find("\x1b_G") {
            let end = rest[start..].find("\x1b\\").expect("chunk terminator") + start;
            let chunk = &rest[start..end];
            let (head, data) = chunk.split_once(';').expect("chunk header");
            assert!(data.len() <= 4096, "chunk payload is bounded");
            if chunks == 0 {
                assert!(head.contains("a=T,f=24,s=64,v=64"), "{head}");
                assert!(head.contains("m=1"), "{head}");
            }
            payload.extend(base64_decode(data));
            chunks += 1;
            rest = &rest[end + 2..];
        }
        assert!(chunks > 1, "a 64x64 image needs several chunks");
        assert_eq!(payload, rgb);
    }

    /// The cross-transport acceptance harness: the same terminal
    /// transcript, driven through the iroh seam (`parse_transcript`) and
    /// the SSH seam (`ProbeDemux`), selects the same tier and the two
    /// presenters emit byte-identical frames for the same views.
    #[test]
    fn cross_transport_presenters_agree_on_the_transcript_matrix() {
        let cases: [(&str, Vec<u8>, PresentationTier); 6] = [
            (
                "tier one",
                [
                    COLOR_COUNT_REPLY,
                    CURSOR_POSITION_REPLY,
                    SYNC_OUTPUT_REPLY,
                    KITTY_GRAPHICS_REPLY,
                ]
                .concat(),
                PresentationTier::Tier1,
            ),
            (
                "tier two",
                [COLOR_COUNT_REPLY, CURSOR_POSITION_REPLY].concat(),
                PresentationTier::Tier2,
            ),
            ("no reply", Vec::new(), PresentationTier::Tier3),
            (
                "malformed",
                b"\x1b[?x u\x1b[?2026;9$y".to_vec(),
                PresentationTier::Tier3,
            ),
            (
                "conflicting",
                [
                    KITTY_GRAPHICS_REPLY,
                    b"\x1b_Gi=31;ENOENT\x1b\\".as_slice(),
                    SYNC_OUTPUT_REPLY,
                ]
                .concat(),
                PresentationTier::Tier3,
            ),
            (
                "interleaved keys",
                b"j\x1b[12;40Rk".to_vec(),
                PresentationTier::Tier3,
            ),
        ];
        let frames = [screen(&["abc", "def"]), screen(&["abc", "dXf"])];
        for (name, transcript, tier) in cases {
            let iroh = parse_transcript(&transcript);
            let mut demux = ProbeDemux::new();
            demux.push(&transcript);
            demux.expire();
            let ssh = demux.outcome();
            assert_eq!(iroh.tier(), tier, "{name}: iroh tier");
            assert_eq!(ssh.tier(), tier, "{name}: ssh tier");

            let mut a = TierPresenter::from_outcome(&iroh);
            let mut b = TierPresenter::from_outcome(&ssh);
            for frame in &frames {
                assert_eq!(
                    a.render_screen(frame),
                    b.render_screen(frame),
                    "{name}: presenter output differs across transports"
                );
            }
            assert_eq!(a.counters(), b.counters(), "{name}: counters differ");
            assert_eq!(a.finish(), b.finish(), "{name}: restore differs");
        }
    }
}
