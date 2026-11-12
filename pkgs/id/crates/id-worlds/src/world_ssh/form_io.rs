//! Form-answer input helpers for the SSH shell: the retry-token
//! alias and the startup-probe-aware byte fetch. Split out of
//! `world_ssh.rs` to keep that module under the production line
//! limit (#1411).

use std::collections::VecDeque;

use tokio::sync::mpsc;

use super::{Inbound, MAX_FORM_RESPONSE_BYTES, SshInputFrame, SshProbe, terminal_text};

/// The retry alias recorded for a rejected form answer:
/// `(form_id, view_token, current_token)`.
///
/// `view_token` is the token the guest's view still renders. A guest may
/// keep rendering its original token across repeated rejections while
/// each rejection advances the host-issued retry token, so when the
/// rejected answer carried the current retry token, the ORIGINAL view
/// token stays the alias; otherwise the rejected token starts a fresh
/// alias. Losing the original alias makes the next unchanged redraw
/// overwrite the active form with the stale token, and even a corrected
/// answer is then refused as `stale_presentation` (Codex P2, PR #8928).
pub(super) fn form_retry_alias(
    existing: Option<&(String, String, String)>,
    form_id: &str,
    rejected_token: &str,
    new_token: &str,
) -> (String, String, String) {
    let view_token = match existing {
        Some((retry_id, view_token, current_token))
            if retry_id.as_str() == form_id && current_token.as_str() == rejected_token =>
        {
            view_token.clone()
        }
        _ => rejected_token.to_owned(),
    };
    (form_id.to_owned(), view_token, new_token.to_owned())
}

/// Tokens that must remain suppressed after a form is accepted: its current
/// host token and both sides of the active retry alias, when present.
pub(super) fn form_closed_aliases(
    form_id: &str,
    current_token: &str,
    retry_alias: Option<&(String, String, String)>,
) -> Vec<String> {
    let mut tokens = vec![current_token.to_owned()];
    if let Some((retry_id, view_token, retry_token)) = retry_alias
        && retry_id == form_id
    {
        for token in [view_token, retry_token] {
            if !tokens.iter().any(|known| known == token) {
                tokens.push(token.clone());
            }
        }
    }
    tokens
}

/// When a form answer ended with CR, swallow its LF before resuming normal
/// input. Returns false only when the LF may still arrive in a later packet.
pub(super) fn discard_form_lf_after_cr(
    backlog: &mut std::collections::VecDeque<u8>,
    after_cr: &mut bool,
) -> bool {
    if !*after_cr {
        return true;
    }
    match backlog.front() {
        Some(b'\n') => {
            backlog.pop_front();
        }
        Some(_) => {}
        None => return false,
    }
    *after_cr = false;
    true
}

/// Take an encoded form answer that was persisted before an await, making the
/// next `recv` return it even if the previous receive future was cancelled.
pub(super) fn take_pending_form_answer(pending: &mut Option<String>) -> Option<Inbound> {
    pending.take().map(Inbound::Text)
}

pub(super) enum FormInputProgress {
    Pending {
        echo: Vec<u8>,
    },
    Complete {
        line: Vec<u8>,
        echo: Vec<u8>,
    },
    Invalid {
        message: &'static str,
        echo: Vec<u8>,
    },
}

/// Echo a printable ASCII byte immediately or complete only the pending UTF-8
/// scalar. The accumulated answer is validated once, when its line is submitted.
fn echo_form_byte(byte: u8, pending: &mut Vec<u8>, echo: &mut Vec<u8>) {
    if pending.is_empty() {
        if byte.is_ascii() {
            if byte.is_ascii_graphic() || byte == b' ' {
                echo.push(byte);
            }
            return;
        }
        if matches!(byte, 0xc2..=0xdf | 0xe0..=0xef | 0xf0..=0xf4) {
            pending.push(byte);
        }
        return;
    }

    if byte & 0xc0 != 0x80 {
        pending.clear();
        echo_form_byte(byte, pending, echo);
        return;
    }
    pending.push(byte);
    let expected = match pending[0] {
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => {
            pending.clear();
            return;
        }
    };
    if pending.len() < expected {
        return;
    }
    if let Ok(text) = std::str::from_utf8(pending) {
        echo.extend_from_slice(terminal_text(text).as_bytes());
    }
    pending.clear();
}

/// Discard an invalid/overlong input suffix through one line terminator,
/// preserving later bytes and CRLF state for the next read.
pub(super) fn discard_form_input_line(backlog: &mut VecDeque<u8>, after_cr: &mut bool) -> bool {
    while let Some(byte) = backlog.pop_front() {
        let swallowed_lf = std::mem::take(after_cr);
        if byte == b'\n' && swallowed_lf {
            continue;
        }
        match byte {
            b'\r' => {
                *after_cr = true;
                return true;
            }
            b'\n' => return true,
            _ => {}
        }
    }
    false
}

/// Consume queued form bytes through at most one line terminator. Remaining
/// packet bytes stay in the queue for the next form answer or world input.
pub(super) fn consume_form_input(
    backlog: &mut VecDeque<u8>,
    line: &mut Vec<u8>,
    after_cr: &mut bool,
    echo_pending: &mut Vec<u8>,
) -> FormInputProgress {
    let mut echo = Vec::new();
    while let Some(byte) = backlog.pop_front() {
        let swallowed_lf = std::mem::take(after_cr);
        if byte == b'\n' && swallowed_lf {
            continue;
        }
        match byte {
            b'\r' | b'\n' => {
                *after_cr = byte == b'\r';
                echo_pending.clear();
                echo.extend_from_slice(b"\r\n");
                return FormInputProgress::Complete {
                    line: std::mem::take(line),
                    echo,
                };
            }
            0x08 | 0x7f => {
                echo_pending.clear();
                if std::str::from_utf8(line).is_err() {
                    while !line.is_empty() && std::str::from_utf8(line).is_err() {
                        line.pop();
                    }
                } else if let Some(last) = std::str::from_utf8(line)
                    .ok()
                    .and_then(|line| line.char_indices().next_back().map(|(index, _)| index))
                {
                    line.truncate(last);
                    echo.extend_from_slice(b"\x08 \x08");
                }
            }
            byte if byte.is_ascii_control() => {
                line.clear();
                echo_pending.clear();
                return FormInputProgress::Invalid {
                    message: "form input cannot contain control characters",
                    echo,
                };
            }
            byte if line.len() < MAX_FORM_RESPONSE_BYTES => {
                line.push(byte);
                echo_form_byte(byte, echo_pending, &mut echo);
            }
            _ => {
                line.clear();
                echo_pending.clear();
                return FormInputProgress::Invalid {
                    message: "form response line is too large",
                    echo,
                };
            }
        }
    }
    FormInputProgress::Pending { echo }
}

/// One fetched input unit for the form collector.
pub(super) enum FormBytes {
    Bytes(Vec<u8>),
    Resize(SshInputFrame),
    Closed,
}

/// Fetch the next input unit while the startup probe window is open,
/// routing bytes through the probe demultiplexer so terminal capability
/// replies never reach the caller as input.
///
/// A form can become active before the terminal answers the startup
/// probe burst (the burst is sent before `run_session`). Feeding those
/// replies to the form collector would flag a normal cursor-position
/// reply as invalid control input and leave its tail in the answer
/// line, corrupting the user's first answer (Codex P2, PR #8928). On
/// the probe deadline the window closes and any held bytes — partial
/// prefixes that turned out to be user input — are returned. Without
/// an open probe this is a plain frame read.
pub(super) async fn recv_bytes_through_probe(
    frames: &mut mpsc::Receiver<SshInputFrame>,
    probe: &mut Option<SshProbe>,
) -> FormBytes {
    loop {
        let Some(deadline) = probe.as_ref().map(|probe| probe.deadline) else {
            return match frames.recv().await {
                Some(SshInputFrame::Bytes(bytes)) => FormBytes::Bytes(bytes),
                Some(resize @ SshInputFrame::Resize(..)) => FormBytes::Resize(resize),
                None => FormBytes::Closed,
            };
        };
        // Cancel-safe like the raw-mode probe loop in `recv`: the probe
        // stays in place across the await, and its demux holds any
        // partial reply bytes between calls.
        let received = tokio::select! {
            frame = frames.recv() => Some(frame),
            () = tokio::time::sleep_until(deadline) => None,
        };
        match received {
            Some(Some(SshInputFrame::Bytes(bytes))) => {
                let input = probe
                    .as_mut()
                    .map_or_else(Vec::new, |probe| probe.demux.push(&bytes));
                if !input.is_empty() {
                    return FormBytes::Bytes(input);
                }
            }
            Some(Some(resize @ SshInputFrame::Resize(..))) => {
                return FormBytes::Resize(resize);
            }
            Some(None) => return FormBytes::Closed,
            None => {
                let held = match probe.take() {
                    Some(mut probe) => {
                        let tier = probe.demux.tier();
                        tracing::debug!(
                            ?tier,
                            "terminal capability probe closed; Tier 3 fallback stays active"
                        );
                        probe.demux.expire()
                    }
                    None => Vec::new(),
                };
                if !held.is_empty() {
                    return FormBytes::Bytes(held);
                }
            }
        }
    }
}
