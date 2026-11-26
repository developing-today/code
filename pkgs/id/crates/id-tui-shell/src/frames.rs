//! Client frame builders for the TUI shell (#12818 slice 1).
//!
//! Every frame the shell sends is built here and checked against the
//! embedded F13.1 contract in the phase it will be sent, so a frame
//! the contract would refuse is an error here, never a wire message.
//! The set of frame types these builders produce is exactly the
//! statement's `sends` list (pinned by `tests/capability_statement.rs`).

use id::shell_contract::{Phase, ShellContract};
use serde_json::{Map, Value, json};

/// Largest `paste` text the contract accepts, in bytes.
pub const MAX_PASTE_BYTES: usize = 4096;
/// Largest `join` capability, in bytes.
pub const MAX_CAPABILITY_BYTES: usize = 256;
/// Largest `join` resume token, in bytes.
pub const MAX_RESUME_TOKEN_BYTES: usize = 64;
/// Largest form presentation token, in bytes (hex-encoded in the frame).
pub const MAX_PRESENTATION_TOKEN_BYTES: usize = 32;
/// Largest form id, in bytes.
pub const MAX_FORM_ID_BYTES: usize = 64;
/// Largest form action id, in bytes.
pub const MAX_ACTION_BYTES: usize = 64;

/// Why a client frame could not be built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrameError {
    /// The embedded F13.1 contract could not be loaded.
    Contract(String),
    /// The frame violates the F13.1 contract, or a bound it names.
    OutOfContract(String),
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Contract(why) => write!(f, "shell contract unavailable: {why}"),
            Self::OutOfContract(why) => write!(f, "frame outside the F13.1 contract: {why}"),
        }
    }
}

impl std::error::Error for FrameError {}

/// Check a built client frame against the embedded contract in the
/// phase it will be sent, and return it.
///
/// # Errors
///
/// [`FrameError::OutOfContract`] when the contract refuses the frame.
pub fn checked(frame: Value, phase: Phase) -> Result<Value, FrameError> {
    let contract = ShellContract::embedded().map_err(FrameError::Contract)?;
    contract
        .check_client_frame(&frame, phase)
        .map_err(FrameError::OutOfContract)?;
    Ok(frame)
}

/// Build the opening `join` frame. `after` and `resume_token` are
/// both absent for a fresh session (the host answers with a
/// snapshot); both present resume from the recorded history
/// position under the v1 history-identity rule.
///
/// # Errors
///
/// [`FrameError::OutOfContract`] for an oversize capability or token,
/// or any other contract refusal.
pub fn join(
    capability: &str,
    after: Option<u64>,
    resume_token: Option<&str>,
) -> Result<Value, FrameError> {
    if capability.len() > MAX_CAPABILITY_BYTES {
        return Err(FrameError::OutOfContract(format!(
            "capability exceeds {MAX_CAPABILITY_BYTES} bytes"
        )));
    }
    if resume_token.is_some_and(|token| token.len() > MAX_RESUME_TOKEN_BYTES) {
        return Err(FrameError::OutOfContract(format!(
            "resume_token exceeds {MAX_RESUME_TOKEN_BYTES} bytes"
        )));
    }
    let frame = json!({
        "type": "join",
        "capability": capability,
        "after": after,
        "resume_token": resume_token,
    });
    checked(frame, Phase::Opening)
}

/// Build a `records` query for the joined session.
///
/// # Errors
///
/// [`FrameError::OutOfContract`] on a contract refusal.
pub fn records(capability: &str, prefix: Option<&str>, after: Option<&str>) -> Result<Value, FrameError> {
    checked(
        json!({
            "type": "records",
            "capability": capability,
            "prefix": prefix,
            "after": after,
        }),
        Phase::Joined,
    )
}

/// Build a `chat` frame posting `text` to the world.
///
/// # Errors
///
/// [`FrameError::OutOfContract`] on a contract refusal.
pub fn chat(text: &str, screen_revision: Option<u64>) -> Result<Value, FrameError> {
    checked(
        json!({"type": "chat", "text": text, "screen_revision": screen_revision}),
        Phase::Joined,
    )
}

/// Build an `input` frame carrying raw key bytes, hex-encoded.
///
/// # Errors
///
/// [`FrameError::OutOfContract`] on a contract refusal.
pub fn input(bytes: &[u8], screen_revision: Option<u64>) -> Result<Value, FrameError> {
    checked(
        json!({
            "type": "input",
            "data_hex": hex(bytes),
            "screen_revision": screen_revision,
        }),
        Phase::Joined,
    )
}

/// Build a `paste` frame. Text longer than [`MAX_PASTE_BYTES`] is
/// refused rather than truncated.
///
/// # Errors
///
/// [`FrameError::OutOfContract`] for oversize text or a contract refusal.
pub fn paste(text: &str, screen_revision: Option<u64>) -> Result<Value, FrameError> {
    if text.len() > MAX_PASTE_BYTES {
        return Err(FrameError::OutOfContract(format!(
            "paste exceeds {MAX_PASTE_BYTES} bytes"
        )));
    }
    checked(
        json!({"type": "paste", "text": text, "screen_revision": screen_revision}),
        Phase::Joined,
    )
}

/// Build a `form_result` answering a presented form.
///
/// # Errors
///
/// [`FrameError::OutOfContract`] for oversize ids or a contract refusal.
pub fn form_result(
    form_id: &str,
    presentation_token: &str,
    action: &str,
    values: &Map<String, Value>,
    screen_revision: Option<u64>,
) -> Result<Value, FrameError> {
    if form_id.len() > MAX_FORM_ID_BYTES {
        return Err(FrameError::OutOfContract(format!(
            "form_id exceeds {MAX_FORM_ID_BYTES} bytes"
        )));
    }
    if presentation_token.len() > MAX_PRESENTATION_TOKEN_BYTES {
        return Err(FrameError::OutOfContract(format!(
            "presentation_token exceeds {MAX_PRESENTATION_TOKEN_BYTES} bytes"
        )));
    }
    if action.len() > MAX_ACTION_BYTES {
        return Err(FrameError::OutOfContract(format!(
            "action exceeds {MAX_ACTION_BYTES} bytes"
        )));
    }
    checked(
        json!({
            "type": "form_result",
            "form_id": form_id,
            "presentation_token": presentation_token,
            "action": action,
            "values": Value::Object(values.clone()),
            "screen_revision": screen_revision,
        }),
        Phase::Joined,
    )
}

/// Build a `resize` frame for the terminal's cell grid.
///
/// # Errors
///
/// [`FrameError::OutOfContract`] for a size outside the contract's
/// bounds (1..=500 columns, 1..=200 rows).
pub fn resize(width: u16, height: u16) -> Result<Value, FrameError> {
    checked(
        json!({"type": "resize", "width": width, "height": height}),
        Phase::Joined,
    )
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}
