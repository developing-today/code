//! The TUI shell's session loop, without a terminal or a network
//! (#12818).
//!
//! [`App`] owns the session fold and the input state, and turns each
//! [`Input`] (a host frame, the host closing the stream, or a key) into
//! at most one [`Step`] for the binary to carry out: send a frame,
//! redial, quit, or end with the host's refusal. Keeping this pure
//! means the join, read, act and reconnect workflow is tested by
//! feeding it the recorded sessions, and the binary is only plumbing.
//!
//! Reconnect follows the v1 history-identity rule through the session:
//! when the host closes the stream the app marks the session as
//! reconnecting, and the next [`App::join_frame`] presents the last
//! applied sequence and resume token. A host that cannot continue
//! contiguously sends a fresh snapshot, and the fold resets.

use ratatui::Frame;
use serde_json::Value;

use crate::frames::{self, FrameError};
use crate::keys::{Action, Key, Keys, ROOM_PREFIX};
use crate::present::{self, Panels};
use crate::render;
use crate::session::{Session, SessionError};

/// Shown in the input line's title while the shell redials.
pub const RECONNECTING: &str = "reconnecting";

/// Something the session loop reacts to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Input {
    /// One frame from the host.
    Host(Value),
    /// The host ended the session stream, or it broke.
    HostClosed,
    /// A key from the terminal.
    Key(Key),
}

/// What the binary should do next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Send this contract-checked frame on the session stream.
    Send(Value),
    /// Redial the host and send [`App::opening_frames`] again.
    Reconnect,
    /// The user quit.
    Quit,
    /// The host refused the session; end with this message.
    End(String),
}

/// The session loop's state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct App {
    session: Session,
    keys: Keys,
    capability: String,
    joined: bool,
}

impl App {
    /// A session loop that will join with `capability`.
    #[must_use]
    pub fn new(capability: &str) -> Self {
        Self {
            session: Session::new(),
            keys: Keys::new(capability),
            capability: capability.to_owned(),
            joined: false,
        }
    }

    /// The folded session.
    #[must_use]
    pub const fn session(&self) -> &Session {
        &self.session
    }

    /// The input state: typed text, focus and status.
    #[must_use]
    pub const fn keys(&self) -> &Keys {
        &self.keys
    }

    /// The `join` for the next connection: fresh the first time, then a
    /// resume from the last applied sequence with the session's resume
    /// token.
    ///
    /// # Errors
    ///
    /// The [`FrameError`] of a capability or token the contract refuses.
    pub fn join_frame(&self) -> Result<Value, FrameError> {
        if self.joined {
            frames::join(
                &self.capability,
                Some(self.session.last_sequence()),
                self.session.resume_token(),
            )
        } else {
            frames::join(&self.capability, None, None)
        }
    }

    /// The frames that open a connection: the join, the terminal's size,
    /// and a request for the room records the selector lists. Marks the
    /// session as joined, so the next connection resumes.
    ///
    /// # Errors
    ///
    /// The [`FrameError`] of a frame the contract refuses, such as a
    /// terminal size outside its bounds.
    pub fn opening_frames(&mut self, width: u16, height: u16) -> Result<Vec<Value>, FrameError> {
        let frames = vec![
            self.join_frame()?,
            frames::resize(width.clamp(1, 500), height.clamp(1, 200))?,
            frames::records(&self.capability, Some(ROOM_PREFIX), None)?,
        ];
        self.joined = true;
        Ok(frames)
    }

    /// React to one input.
    pub fn handle(&mut self, input: Input) -> Option<Step> {
        match input {
            Input::Host(frame) => match self.session.apply(&frame) {
                Ok(_) => {
                    if self.keys.prompt.status.as_deref() == Some(RECONNECTING) {
                        self.keys.prompt.status = None;
                    }
                    None
                }
                Err(SessionError::Refused(message)) => Some(Step::End(message)),
                Err(error) => {
                    self.keys.prompt.status = Some(error.to_string());
                    None
                }
            },
            Input::HostClosed => {
                self.session.begin_reconnect();
                self.keys.prompt.status = Some(RECONNECTING.to_owned());
                Some(Step::Reconnect)
            }
            Input::Key(key) => match self.keys.handle(key) {
                Ok(Action::Redraw) => None,
                Ok(Action::Send(frame)) => Some(Step::Send(frame)),
                Ok(Action::Quit) => Some(Step::Quit),
                Err(error) => {
                    self.keys.prompt.status = Some(error.to_string());
                    None
                }
            },
        }
    }

    /// Show a status line in the input line's title, such as a failed
    /// redial.
    pub fn set_status(&mut self, status: Option<String>) {
        self.keys.prompt.status = status;
    }

    /// The three panels for the current fold.
    #[must_use]
    pub fn panels(&self) -> Panels {
        present::project(&self.session)
    }

    /// Draw the panels and the input line.
    pub fn draw(&self, frame: &mut Frame<'_>) {
        render::draw(frame, &self.panels(), &self.keys.prompt);
    }
}
