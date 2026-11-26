//! Terminal event mapping oracles (issue #12818): crossterm presses
//! become the shell's keys, and nothing else is typed.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use id_tui_shell::keys::Key;
use id_tui_shell::term;
use ratatui::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers,
};

fn press(code: KeyCode, modifiers: KeyModifiers) -> Event {
    Event::Key(KeyEvent::new(code, modifiers))
}

#[test]
fn presses_map_onto_shell_keys() {
    let plain = KeyModifiers::NONE;
    assert_eq!(
        term::key(&press(KeyCode::Char('a'), plain)),
        Some(Key::Char('a'))
    );
    assert_eq!(
        term::key(&press(KeyCode::Char('A'), KeyModifiers::SHIFT)),
        Some(Key::Char('A'))
    );
    assert_eq!(term::key(&press(KeyCode::Enter, plain)), Some(Key::Enter));
    assert_eq!(
        term::key(&press(KeyCode::Backspace, plain)),
        Some(Key::Backspace)
    );
    assert_eq!(term::key(&press(KeyCode::Tab, plain)), Some(Key::Tab));
    assert_eq!(
        term::key(&press(KeyCode::BackTab, KeyModifiers::SHIFT)),
        Some(Key::BackTab)
    );
    assert_eq!(term::key(&press(KeyCode::Esc, plain)), Some(Key::Esc));
}

#[test]
fn bound_control_keys_map_and_others_are_dropped() {
    let control = KeyModifiers::CONTROL;
    assert_eq!(
        term::key(&press(KeyCode::Char('c'), control)),
        Some(Key::CtrlC)
    );
    assert_eq!(
        term::key(&press(KeyCode::Char('r'), control)),
        Some(Key::CtrlR)
    );
    assert_eq!(term::key(&press(KeyCode::Char('x'), control)), None);
    assert_eq!(term::key(&press(KeyCode::F(1), KeyModifiers::NONE)), None);
}

#[test]
fn releases_and_repeats_are_ignored() {
    for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
        let event = Event::Key(KeyEvent::new_with_kind_and_state(
            KeyCode::Char('a'),
            KeyModifiers::NONE,
            kind,
            KeyEventState::NONE,
        ));
        assert_eq!(term::key(&event), None);
    }
}

#[test]
fn paste_and_resize_pass_through() {
    assert_eq!(
        term::key(&Event::Paste("hi\nthere".to_owned())),
        Some(Key::Paste("hi\nthere".to_owned()))
    );
    assert_eq!(
        term::key(&Event::Resize(120, 40)),
        Some(Key::Resize(120, 40))
    );
    assert_eq!(term::key(&Event::FocusGained), None);
}
