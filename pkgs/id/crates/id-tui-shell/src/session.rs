//! Session fold for the TUI shell (#12818 slice 1).
//!
//! [`Session`] folds the server frames the statement presents
//! (`snapshot`, `event`, `view`, `records`, `form_result_status`,
//! `error`) into shell state. Every frame is checked against the
//! embedded contract on the way in. An `error` frame refuses the
//! session; it is never content.
//!
//! Reconnect follows the v1 history-identity rule: a reconnect
//! presents the last applied sequence and the session's resume
//! token. If the host continues contiguously from that sequence the
//! session resumes; otherwise continuity is unprovable and the
//! session is reset to the host's fresh snapshot. Divergence is never
//! merged.

use std::collections::BTreeMap;

use id::shell_contract::ShellContract;
use serde_json::Value;

/// Why a host frame could not be folded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionError {
    /// The embedded F13.1 contract could not be loaded.
    Contract(String),
    /// The frame violates the F13.1 contract.
    OutOfContract(String),
    /// The host refused the session with an `error` frame.
    Refused(String),
    /// A contract-valid frame is missing a field the fold requires.
    Malformed(&'static str),
    /// A frame kind the statement does not present.
    NotPresented(String),
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Contract(why) => write!(f, "shell contract unavailable: {why}"),
            Self::OutOfContract(why) => write!(f, "host frame outside the F13.1 contract: {why}"),
            Self::Refused(message) => write!(f, "world host refused the session: {message}"),
            Self::Malformed(what) => write!(f, "host frame is malformed: {what}"),
            Self::NotPresented(kind) => write!(f, "host frame `{kind}` is not presented"),
        }
    }
}

impl std::error::Error for SessionError {}

/// What one folded host frame changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Update {
    /// A fresh snapshot replaced the session's state.
    Snapshot,
    /// A durable event was folded, advancing the history position.
    Event,
    /// A view replaced the presented screen.
    View,
    /// A records reply arrived.
    Records,
    /// The host answered a `form_result`.
    FormResultStatus,
}

/// The TUI shell's folded session state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Session {
    last_sequence: u64,
    resume_token: Option<String>,
    resume_expect: Option<u64>,
    diverged: bool,
    events: Vec<Value>,
    view: Option<Value>,
    records: Option<Value>,
    last_form_status: Option<Value>,
    participants: BTreeMap<u64, String>,
}

impl Session {
    /// A session with no folded state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The last applied sequence: the history position the shell
    /// presents on reconnect.
    #[must_use]
    pub const fn last_sequence(&self) -> u64 {
        self.last_sequence
    }

    /// The resume token the host last issued with a view, if any.
    #[must_use]
    pub fn resume_token(&self) -> Option<&str> {
        self.resume_token.as_deref()
    }

    /// Whether the last reconnect found its continuity unprovable.
    #[must_use]
    pub const fn diverged(&self) -> bool {
        self.diverged
    }

    /// Durable events folded since the last snapshot, oldest first.
    #[must_use]
    pub fn events(&self) -> &[Value] {
        &self.events
    }

    /// The presented view frame, if one arrived.
    #[must_use]
    pub const fn view(&self) -> Option<&Value> {
        self.view.as_ref()
    }

    /// The most recent `records` reply, if any.
    #[must_use]
    pub const fn records(&self) -> Option<&Value> {
        self.records.as_ref()
    }

    /// The most recent `form_result_status` reply, if any.
    #[must_use]
    pub const fn last_form_status(&self) -> Option<&Value> {
        self.last_form_status.as_ref()
    }

    /// Display names from the last snapshot, keyed by participant
    /// id. A snapshot without a participant list leaves this empty.
    #[must_use]
    pub const fn participants(&self) -> &BTreeMap<u64, String> {
        &self.participants
    }

    /// Prepare a reconnect: remember that the next durable event must
    /// continue contiguously from the last applied sequence. The
    /// caller builds the `join` with [`crate::frames::join`] using
    /// [`Session::last_sequence`] and [`Session::resume_token`].
    pub fn begin_reconnect(&mut self) {
        self.resume_expect = Some(self.last_sequence + 1);
    }

    /// Fold one host frame.
    ///
    /// # Errors
    ///
    /// [`SessionError::OutOfContract`] for a frame the contract
    /// rejects, [`SessionError::Refused`] for an `error` frame,
    /// [`SessionError::Malformed`] for a missing required field, and
    /// [`SessionError::NotPresented`] for a contract-valid frame the
    /// statement does not present.
    pub fn apply(&mut self, frame: &Value) -> Result<Update, SessionError> {
        let contract = ShellContract::embedded().map_err(SessionError::Contract)?;
        let kind = contract
            .check_server_frame(frame)
            .map_err(SessionError::OutOfContract)?;
        match kind {
            "snapshot" => {
                let snapshot = &frame["snapshot"];
                self.last_sequence = snapshot["current_sequence"]
                    .as_u64()
                    .ok_or(SessionError::Malformed("snapshot has no current_sequence"))?;
                self.participants = snapshot["participants"]
                    .as_array()
                    .map(|list| {
                        list.iter()
                            .filter_map(|p| {
                                Some((p["id"].as_u64()?, p["display_name"].as_str()?.to_owned()))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                self.events.clear();
                self.view = None;
                self.resume_expect = None;
                self.diverged = false;
                Ok(Update::Snapshot)
            }
            "event" => {
                let event = &frame["event"];
                let sequence = event["sequence"]
                    .as_u64()
                    .ok_or(SessionError::Malformed("event has no unsigned sequence"))?;
                if let Some(expect) = self.resume_expect.take()
                    && sequence != expect
                {
                    self.diverged = true;
                    self.events.clear();
                    self.view = None;
                    self.last_sequence = self.last_sequence.max(sequence);
                    return Ok(Update::Event);
                }
                self.last_sequence = self.last_sequence.max(sequence);
                self.events.push(event.clone());
                Ok(Update::Event)
            }
            "view" => {
                let sequence = frame["sequence"]
                    .as_u64()
                    .ok_or(SessionError::Malformed("view has no unsigned sequence"))?;
                self.last_sequence = self.last_sequence.max(sequence);
                self.resume_token = frame["resume_token"].as_str().map(str::to_owned);
                self.view = Some(frame.clone());
                Ok(Update::View)
            }
            "records" => {
                self.records = Some(frame.clone());
                Ok(Update::Records)
            }
            "form_result_status" => {
                self.last_form_status = Some(frame.clone());
                Ok(Update::FormResultStatus)
            }
            "error" => Err(SessionError::Refused(
                frame["message"]
                    .as_str()
                    .unwrap_or("unspecified error")
                    .to_owned(),
            )),
            other => Err(SessionError::NotPresented(other.to_owned())),
        }
    }
}
