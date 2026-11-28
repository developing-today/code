//! `id world join` line pump and its helpers (split from `world.rs`, #16255).
//!
//! The pump relays stdin lines and server frames; the helpers around it
//! check transcripts against the shell contract (`check_contract`,
//! `check_suite_file`), parse pump commands (`pump_command`,
//! `pump_plain`), and read the local execution profile and presentation
//! policy files. Behavior-preserving move: every item keeps its body.

use anyhow::{Context, Result, bail};
use tokio::io::{AsyncBufReadExt, BufReader};

use id_core::frames::{read_frame_from, write_frame_to};

/// Render one server frame through the TTY tier presenter when it is a
/// view carrying a screen or text surface (F13.4). Returns `None` for
/// every other frame — those keep the JSON-lines presentation — and
/// `Some("")` for a view the presenter renders as no change. Form,
/// scene, audio and widget views keep the JSON-lines presentation:
/// the pump has no form collector, no scene renderer, no audio
/// player and no widget presenter.
fn render_view_frame(
    presenter: &mut id_worlds::world_terminal::TierPresenter,
    body: &[u8],
) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    if value["type"] != "view" {
        return None;
    }
    let surface = value.get("surface")?;
    match serde_json::from_value::<id_worlds::world_presentation::WorldSurface>(surface.clone())
        .ok()?
    {
        id_worlds::world_presentation::WorldSurface::Screen(screen) => {
            Some(presenter.render_screen(&screen))
        }
        id_worlds::world_presentation::WorldSurface::Text(text) => {
            let rendered = text.render_text();
            let lines: Vec<String> = if rendered.is_empty() {
                Vec::new()
            } else {
                rendered.lines().map(str::to_owned).collect()
            };
            Some(presenter.render_lines(&lines))
        }
        id_worlds::world_presentation::WorldSurface::Form(_) => None,
        id_worlds::world_presentation::WorldSurface::Scene(_) => None,
        id_worlds::world_presentation::WorldSurface::Audio(_) => None,
        id_worlds::world_presentation::WorldSurface::Widget(_) => None,
        id_worlds::world_presentation::WorldSurface::Media(_) => None,
    }
}

/// Stdin that yields bytes captured during the startup terminal probe first.
///
/// The probe runs before the line pump takes stdin, so keystrokes typed while
/// the terminal was answering are replayed here, in order, ahead of live input.
struct PreloadedStdin {
    preload: std::io::Cursor<Vec<u8>>,
    stdin: tokio::io::Stdin,
}

impl PreloadedStdin {
    fn new(preload: Vec<u8>) -> Self {
        Self {
            preload: std::io::Cursor::new(preload),
            stdin: tokio::io::stdin(),
        }
    }
}

impl tokio::io::AsyncRead for PreloadedStdin {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
        buffer: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        if (self.preload.position() as usize) < self.preload.get_ref().len() {
            let target = buffer.initialize_unfilled();
            return match std::io::Read::read(&mut self.preload, target) {
                Ok(read) => {
                    buffer.advance(read);
                    std::task::Poll::Ready(Ok(()))
                }
                Err(error) => std::task::Poll::Ready(Err(error)),
            };
        }
        std::pin::Pin::new(&mut self.stdin).poll_read(context, buffer)
    }
}

/// Print server frames as JSON lines while sending stdin lines as chat.
/// Local command responses go to stderr unless JSON transcript mode is enabled.
///
/// The transport is any byte stream carrying the length-prefixed envelope
/// (an iroh stream or an SSH exec channel); `first` is the join frame.
/// `replay` holds bytes captured by the optional startup terminal probe, which
/// are fed to the line reader before live stdin so typing order is preserved.
pub(crate) async fn pump<R, W>(
    mut recv: R,
    mut send: W,
    first: &str,
    command_mode: bool,
    json: bool,
    capability: &str,
    replay: Vec<u8>,
    presenter: Option<id_worlds::world_terminal::TierPresenter>,
) -> Result<()>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
    W: tokio::io::AsyncWrite + Unpin,
{
    write_frame_to(&mut send, first.as_bytes()).await?;
    // Set once the reader has printed at least one server frame. The
    // stdin-EOF arm below reads it: EOF before the first frame must
    // not end the pump (see that arm); EOF after frames is the
    // scripted client's normal leave.
    let frames_printed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let frames_flag = frames_printed.clone();
    let presenter = std::sync::Arc::new(std::sync::Mutex::new(presenter));
    let reader_presenter = presenter.clone();
    let mut reader = tokio::spawn(async move {
        while let Some(body) = read_frame_from(&mut recv).await? {
            let rendered = {
                let mut guard = reader_presenter
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                guard
                    .as_mut()
                    .and_then(|presenter| render_view_frame(presenter, &body))
            };
            match rendered {
                Some(text) => {
                    if !text.is_empty() {
                        use std::io::Write as _;
                        let mut out = std::io::stdout().lock();
                        let _ = out.write_all(text.as_bytes());
                        let _ = out.flush();
                    }
                }
                None => println!("{}", String::from_utf8_lossy(&body)),
            }
            frames_flag.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        anyhow::Ok(())
    });
    let mut lines = BufReader::new(PreloadedStdin::new(replay)).lines();
    // True once stdin hit EOF before the first server frame: the
    // stdin arm is then disabled (it would return EOF forever) and
    // the reader arm ends the pump when the stream closes.
    let mut eof_before_first_frame = false;
    let result = loop {
        tokio::select! {
            line = lines.next_line(), if !eof_before_first_frame => match line? {
                Some(line) => {
                    let line = line.trim();
                    if line.is_empty() { continue; }
                    let action = if command_mode {
                        pump_command(line, capability)
                    } else {
                        pump_plain(line)
                    };
                    let frame = match action {
                        PumpLine::Send(frame) => frame,
                        PumpLine::Say { level, message } => {
                            if json {
                                println!("{}", local_transcript_record(level, &message));
                            } else {
                                eprintln!("{message}");
                            }
                            continue;
                        }
                        PumpLine::Quit => break Ok(()),
                    };
                    if let Err(e) = write_frame_to(&mut send, frame.to_string().as_bytes()).await {
                        break Err(e);
                    }
                }
                // Stdin EOF ends the session: a scripted client leaves by
                // closing stdin (interactive users have Ctrl-C). Keeping
                // the pump alive past EOF hung every scripted join until
                // the server happened to close the connection first.
                // One exception to that rule: EOF before the first server
                // frame. The join request
                // has been sent but its answer, a snapshot or a refusal
                // frame such as "join denied", is still in flight, and
                // breaking here aborts the reader before the answer can
                // print, so a scripted join with stdin already closed
                // (the one-shot probe shape) could never observe a
                // refusal. A refusal closes the stream right after the
                // error frame, so the pump waits: the stdin arm is
                // disabled and the reader arm ends the pump when the
                // stream closes, after the refusal has printed.
                None => {
                    if frames_printed.load(std::sync::atomic::Ordering::SeqCst) {
                        break Ok(());
                    }
                    eof_before_first_frame = true;
                    continue;
                }
            },
            ended = &mut reader => {
                break ended.context("reader task")?;
            }
            _ = tokio::signal::ctrl_c() => break Ok(()),
        }
    };
    reader.abort();
    // Restore presenter modes (synchronized output) on every exit path.
    let restore = {
        let mut guard = presenter
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.as_mut().map_or_else(
            String::new,
            id_worlds::world_terminal::TierPresenter::finish,
        )
    };
    if !restore.is_empty() {
        use std::io::Write as _;
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(restore.as_bytes());
        let _ = out.flush();
    }
    result
}

/// Check a transcript against the shell contract, returning a summary.
///
/// # Errors
///
/// Fails on an unreadable transcript, or with every violation listed.
pub(crate) fn check_contract(text: &str) -> Result<String> {
    let contract =
        id_worlds::shell_contract::ShellContract::embedded().map_err(anyhow::Error::msg)?;
    let violations = id_worlds::shell_contract::check_transcript(contract, text)?;
    let frames = text.lines().filter(|line| !line.trim().is_empty()).count();
    if violations.is_empty() {
        return Ok(format!(
            "{frames} frames conform to shell contract v{}.{}",
            contract.version, contract.revision
        ));
    }
    let listed: Vec<String> = violations.iter().map(ToString::to_string).collect();
    bail!(
        "{} of {frames} frames violate shell contract v{}.{}:\n{}",
        violations.len(),
        contract.version,
        contract.revision,
        listed.join("\n")
    )
}

/// Run a conformance-suite manifest: every shell's capability
/// statement validated, every recorded session checked, in one
/// pass. Returns the per-shell report.
///
/// # Errors
///
/// Fails on an unreadable or invalid manifest, or with the full
/// report as the error when any shell fails the suite.
pub(crate) fn check_suite_file(path: &std::path::Path) -> Result<String> {
    use id_worlds::shell_contract::{SessionSource, ShellSuite, SuiteEntry, check_suite};
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("read suite manifest {}", path.display()))?;
    let suite = ShellSuite::parse(&text).map_err(anyhow::Error::msg)?;
    let contract =
        id_worlds::shell_contract::ShellContract::embedded().map_err(anyhow::Error::msg)?;
    let base = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let loaded: Vec<Option<Result<String, String>>> = suite
        .shells
        .iter()
        .map(|shell| {
            shell.session.as_ref().map(|relative| {
                let full = base.join(relative);
                std::fs::read_to_string(&full)
                    .map_err(|error| format!("read {}: {error}", full.display()))
            })
        })
        .collect();
    let entries: Vec<SuiteEntry<'_>> = suite
        .shells
        .iter()
        .zip(&loaded)
        .map(|(shell, loaded)| SuiteEntry {
            name: &shell.name,
            statement: shell.statement.as_ref(),
            session: match loaded {
                None => SessionSource::Unrecorded,
                Some(Ok(text)) => SessionSource::Recorded(text),
                Some(Err(why)) => SessionSource::Unreadable(why),
            },
        })
        .collect();
    let verdicts = check_suite(contract, &entries);
    let mut lines = Vec::new();
    let mut failed = 0_usize;
    let mut recorded = 0_usize;
    for verdict in &verdicts {
        if verdict.recorded {
            recorded += 1;
        }
        if verdict.passed() {
            if verdict.recorded {
                lines.push(format!(
                    "{}: PASS ({} frames, recorded session)",
                    verdict.name, verdict.frames
                ));
            } else {
                lines.push(format!(
                    "{}: UNRECORDED (capability statement ok, no recorded session yet)",
                    verdict.name
                ));
            }
        } else {
            failed += 1;
            lines.push(format!("{}: FAIL", verdict.name));
            for failure in &verdict.failures {
                lines.push(format!("  {failure}"));
            }
        }
    }
    lines.push(format!(
        "suite: {} of {} shells pass, {recorded} recorded session(s) checked against shell contract v{}.{}",
        verdicts.len() - failed,
        verdicts.len(),
        contract.version,
        contract.revision
    ));
    let report = lines.join("\n");
    if failed > 0 {
        bail!("{report}");
    }
    Ok(report)
}

/// What one stdin line does in the `id world join` pump.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PumpLine {
    /// Send this session frame.
    Send(serde_json::Value),
    /// Answer the user locally; nothing is sent.
    Say {
        level: &'static str,
        message: String,
    },
    /// End the session.
    Quit,
}

const PUMP_HELP: &str = "Commands: a line is chat; /input <hex>, /records [prefix], /stats, /caps, /logs, /anchor <target> [type], /anchor confirm <n>, /anchor list, /talk <target>, /anchors <target>, /resolve <thread> [into <target>], /queue, /flush, /help, /quit. Server frames print to stdout as JSON lines.";

/// Stable JSONL record for a response that the join client handles locally.
pub(crate) fn local_transcript_record(level: &str, message: &str) -> serde_json::Value {
    serde_json::json!({"type": "local", "level": level, "message": message})
}

/// Map one line of the shared grammar ([`id_worlds::world_line`]) to what the
/// pump does. Every frame it sends is in the shell contract's joined phase;
/// the commands that need the host's own store (`/documents`) or that the
/// pump already does (`/logs`) are answered locally.
pub(crate) fn pump_command(line: &str, capability: &str) -> PumpLine {
    use id_worlds::world_line::{WorldLine, parse_line};
    let Some(action) = parse_line(line) else {
        return PumpLine::Quit;
    };
    PumpLine::Send(match action {
        WorldLine::Chat(text) => serde_json::json!({"type": "chat", "text": text}),
        WorldLine::Input(hex) => serde_json::json!({"type": "input", "data_hex": hex}),
        WorldLine::Records { prefix } => serde_json::json!({
            "type": "records", "capability": capability, "prefix": prefix, "after": null
        }),
        WorldLine::Stats => serde_json::json!({"type": "info", "capability": capability}),
        WorldLine::Caps => serde_json::json!({"type": "caps", "capability": capability}),
        WorldLine::Documents { .. } | WorldLine::Document(_) => {
            return PumpLine::Say {
                level: "error",
                message: "documents are read from the host's own store over SSH (`ssh cmd:<world>@host`); `id world join` has no document reader".to_owned(),
            };
        }
        WorldLine::Logs { .. } => {
            return PumpLine::Say {
                level: "info",
                message: "events already print live here as JSON lines; use `id world logs` for a bounded tail".to_owned(),
            };
        }
        WorldLine::Anchor { .. }
        | WorldLine::AnchorConfirm { .. }
        | WorldLine::AnchorList
        | WorldLine::Talk { .. }
        | WorldLine::Anchors { .. }
        | WorldLine::Resolve { .. }
        | WorldLine::Queue
        | WorldLine::Flush => {
            // The flow verbs parse identically on every surface (#5873
            // section 3), but their operations (chat.anchor.v1, talk
            // bindings, the outbox) are not wired into the join pump yet;
            // say so rather than sending a frame the host cannot mean.
            return PumpLine::Say {
                level: "error",
                message: "that command is recognised, but its operation is not available in `id world join` yet".to_owned(),
            };
        }
        WorldLine::Help => {
            return PumpLine::Say {
                level: "info",
                message: PUMP_HELP.to_owned(),
            };
        }
        WorldLine::Quit => return PumpLine::Quit,
        WorldLine::Message(message) => {
            return PumpLine::Say {
                level: "error",
                message,
            };
        }
    })
}

/// Map the plain `id world join` input line, validating `/input` while
/// leaving every other line as chat.
pub(crate) fn pump_plain(line: &str) -> PumpLine {
    use id_worlds::world_line::{WorldLine, parse_line};

    if line != "/input" && !line.starts_with("/input ") {
        return PumpLine::Send(serde_json::json!({"type": "chat", "text": line}));
    }
    match parse_line(line) {
        Some(WorldLine::Input(hex)) => {
            PumpLine::Send(serde_json::json!({"type": "input", "data_hex": hex}))
        }
        Some(WorldLine::Message(message)) => PumpLine::Say {
            level: "error",
            message,
        },
        _ => PumpLine::Say {
            level: "info",
            message: "usage: /input <hex>".to_owned(),
        },
    }
}

/// The execution profile in the file at `path`, for `id world doctor`.
///
/// A duplicated key is refused rather than read as its last value
/// (#8794).
pub(crate) fn read_execution_profile(
    path: &std::path::Path,
) -> Result<id_caps::world_limits::WorldExecutionProfile> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("read execution profile {}", path.display()))?;
    id_core::json_unique::from_slice_without_duplicate_keys(text.as_bytes())
        .with_context(|| format!("parse execution profile {}", path.display()))
}

/// The presentation policy in the file at `path`, for
/// `id world presentation --set`. A duplicated key is refused rather
/// than read as its last value (#8794), and the policy is validated
/// here so an oversized one is refused before it is sent.
pub(crate) fn read_presentation_policy(
    path: &std::path::Path,
) -> Result<id_core::presentation_policy::PresentationPolicy> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("read presentation policy {}", path.display()))?;
    let policy: id_core::presentation_policy::PresentationPolicy =
        id_core::json_unique::from_slice_without_duplicate_keys(text.as_bytes())
            .with_context(|| format!("parse presentation policy {}", path.display()))?;
    policy
        .validate()
        .with_context(|| format!("invalid presentation policy {}", path.display()))?;
    Ok(policy)
}
