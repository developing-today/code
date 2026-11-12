//! Form-answer input helpers for the SSH shell: the retry-token
//! alias and the startup-probe-aware byte fetch. Split out of
//! `world_ssh.rs` to keep that module under the production line
//! limit (#1411).

use tokio::sync::mpsc;

use super::{SshInputFrame, SshProbe};

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
