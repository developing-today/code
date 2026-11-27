//! Key handling for the TUI shell (#12818): terminal keys into the
//! frames the statement sends.
//!
//! [`Keys::handle`] is pure: it takes one [`Key`] and returns what the
//! shell should do, building every outbound frame with
//! [`crate::frames`], so each frame is checked against the contract
//! before it can leave. The terminal loop maps its backend's key events
//! onto [`Key`]; nothing here knows about a particular terminal.
//!
//! - **Chat focus:** typing edits the input line, Enter sends it as a
//!   `chat` frame, a paste is inserted into the line.
//! - **Document focus:** keys go to the world as `input` bytes (Enter
//!   is a carriage return, Backspace is DEL), a paste is a `paste`
//!   frame. This is the act surface the world's program reads.
//! - **Anywhere:** Tab and Shift-Tab move focus, Ctrl-R asks for the
//!   room records, a terminal resize sends `resize`, Esc and Ctrl-C
//!   quit.
//!
//! Human shells send no `screen_revision`; that token is for agents.

use serde_json::Value;

use crate::frames::{self, FrameError};
use crate::render::{Focus, Prompt};

/// The record prefix the room selector lists.
pub const ROOM_PREFIX: &str = "room/";

/// A terminal key, independent of the terminal backend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Key {
    /// A printable character.
    Char(char),
    /// Enter / Return.
    Enter,
    /// Backspace.
    Backspace,
    /// Tab: focus the next panel.
    Tab,
    /// Shift-Tab: focus the previous panel.
    BackTab,
    /// Escape: quit.
    Esc,
    /// Ctrl-C: quit.
    CtrlC,
    /// Ctrl-R: refresh the room records.
    CtrlR,
    /// Bracketed paste.
    Paste(String),
    /// The terminal was resized to this many columns and rows.
    Resize(u16, u16),
}

/// What the shell should do after a key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Only local state changed; redraw.
    Redraw,
    /// Send this contract-checked frame to the host, then redraw.
    Send(Value),
    /// Leave the session.
    Quit,
}

/// Local input state: the prompt the renderer draws, plus the join
/// capability that `records` requests carry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Keys {
    /// What the input line shows and where focus is.
    pub prompt: Prompt,
    capability: String,
}

const fn next(focus: Focus) -> Focus {
    match focus {
        Focus::Selector => Focus::Document,
        Focus::Document => Focus::Chat,
        Focus::Chat => Focus::Selector,
    }
}

const fn previous(focus: Focus) -> Focus {
    match focus {
        Focus::Selector => Focus::Chat,
        Focus::Document => Focus::Selector,
        Focus::Chat => Focus::Document,
    }
}

impl Keys {
    /// Input state for a session joined with `capability`, focused on
    /// the chat.
    #[must_use]
    pub fn new(capability: &str) -> Self {
        Self {
            prompt: Prompt::default(),
            capability: capability.to_owned(),
        }
    }

    /// Handle one key.
    ///
    /// # Errors
    ///
    /// The [`FrameError`] of a frame the contract refuses, such as an
    /// oversize paste. The typed text is kept, so nothing is lost.
    pub fn handle(&mut self, key: Key) -> Result<Action, FrameError> {
        match key {
            Key::Esc | Key::CtrlC => return Ok(Action::Quit),
            Key::Tab => {
                self.prompt.focus = next(self.prompt.focus);
                return Ok(Action::Redraw);
            }
            Key::BackTab => {
                self.prompt.focus = previous(self.prompt.focus);
                return Ok(Action::Redraw);
            }
            Key::CtrlR => {
                return frames::records(&self.capability, Some(ROOM_PREFIX), None)
                    .map(Action::Send);
            }
            Key::Resize(width, height) => {
                return frames::resize(width, height).map(Action::Send);
            }
            _ => {}
        }
        match self.prompt.focus {
            Focus::Chat => self.chat_key(key),
            Focus::Document => Self::document_key(key),
            Focus::Selector => Ok(Action::Redraw),
        }
    }

    fn chat_key(&mut self, key: Key) -> Result<Action, FrameError> {
        match key {
            Key::Char(c) => self.prompt.text.push(c),
            Key::Paste(text) => self
                .prompt
                .text
                .extend(text.chars().filter(|c| !c.is_control())),
            Key::Backspace => {
                self.prompt.text.pop();
            }
            Key::Enter => {
                let text = self.prompt.text.trim();
                if text.is_empty() {
                    return Ok(Action::Redraw);
                }
                let frame = frames::chat(text, None)?;
                self.prompt.text.clear();
                return Ok(Action::Send(frame));
            }
            _ => {}
        }
        Ok(Action::Redraw)
    }

    fn document_key(key: Key) -> Result<Action, FrameError> {
        let mut buf = [0_u8; 4];
        let bytes: &[u8] = match &key {
            Key::Char(c) => c.encode_utf8(&mut buf).as_bytes(),
            Key::Enter => b"\r",
            Key::Backspace => b"\x7f",
            Key::Paste(text) => return frames::paste(text, None).map(Action::Send),
            _ => return Ok(Action::Redraw),
        };
        frames::input(bytes, None).map(Action::Send)
    }
}
