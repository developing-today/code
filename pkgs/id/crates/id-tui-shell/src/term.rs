//! Crossterm events onto the shell's backend-independent [`Key`]s
//! (#12818).
//!
//! Only key presses count: release and repeat events from terminals
//! that report them are ignored, so a key is never sent twice. Control
//! combinations other than the ones the shell binds are dropped rather
//! than typed.

use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::keys::Key;

/// The shell key for one terminal event, if it is one the shell uses.
#[must_use]
pub fn key(event: &Event) -> Option<Key> {
    match event {
        Event::Key(key) => key_press(key),
        Event::Paste(text) => Some(Key::Paste(text.clone())),
        Event::Resize(width, height) => Some(Key::Resize(*width, *height)),
        _ => None,
    }
}

fn key_press(key: &KeyEvent) -> Option<Key> {
    if key.kind != KeyEventKind::Press {
        return None;
    }
    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char('c') if control => Some(Key::CtrlC),
        KeyCode::Char('r') if control => Some(Key::CtrlR),
        KeyCode::Char(_) if control => None,
        KeyCode::Char(c) => Some(Key::Char(c)),
        KeyCode::Enter => Some(Key::Enter),
        KeyCode::Backspace => Some(Key::Backspace),
        KeyCode::Tab => Some(Key::Tab),
        KeyCode::BackTab => Some(Key::BackTab),
        KeyCode::Esc => Some(Key::Esc),
        _ => None,
    }
}
