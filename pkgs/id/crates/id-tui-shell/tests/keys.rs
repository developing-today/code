//! Key handling oracles (issue #12818): each key either changes local
//! state or yields exactly the frame the frame builders make, so every
//! outbound frame is one the contract accepts.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use id_tui_shell::frames::{self, FrameError, MAX_PASTE_BYTES};
use id_tui_shell::keys::{Action, Key, Keys, ROOM_PREFIX};
use id_tui_shell::render::Focus;

const CAPABILITY: &str = "tui-capability";

fn typed(keys: &mut Keys, text: &str) {
    for c in text.chars() {
        assert_eq!(keys.handle(Key::Char(c)).unwrap(), Action::Redraw);
    }
}

#[test]
fn chat_focus_edits_the_line_and_enter_sends_it() {
    let mut keys = Keys::new(CAPABILITY);
    assert_eq!(keys.prompt.focus, Focus::Chat);
    typed(&mut keys, "helo");
    keys.handle(Key::Backspace).unwrap();
    typed(&mut keys, "lo");
    assert_eq!(keys.prompt.text, "hello");
    assert_eq!(
        keys.handle(Key::Enter).unwrap(),
        Action::Send(frames::chat("hello", None).unwrap())
    );
    assert!(keys.prompt.text.is_empty(), "the line clears once sent");
}

#[test]
fn an_empty_or_blank_line_sends_nothing() {
    let mut keys = Keys::new(CAPABILITY);
    assert_eq!(keys.handle(Key::Enter).unwrap(), Action::Redraw);
    typed(&mut keys, "   ");
    assert_eq!(keys.handle(Key::Enter).unwrap(), Action::Redraw);
}

#[test]
fn a_chat_paste_joins_the_line_without_control_characters() {
    let mut keys = Keys::new(CAPABILITY);
    keys.handle(Key::Paste("one\ntwo\u{1b}".to_owned()))
        .unwrap();
    assert_eq!(keys.prompt.text, "onetwo");
}

#[test]
fn tab_cycles_focus_both_ways() {
    let mut keys = Keys::new(CAPABILITY);
    keys.handle(Key::Tab).unwrap();
    assert_eq!(keys.prompt.focus, Focus::Selector);
    keys.handle(Key::Tab).unwrap();
    assert_eq!(keys.prompt.focus, Focus::Document);
    keys.handle(Key::Tab).unwrap();
    assert_eq!(keys.prompt.focus, Focus::Chat);
    keys.handle(Key::BackTab).unwrap();
    assert_eq!(keys.prompt.focus, Focus::Document);
}

#[test]
fn document_focus_sends_keys_to_the_world_as_input() {
    let mut keys = Keys::new(CAPABILITY);
    keys.prompt.focus = Focus::Document;
    for (key, bytes) in [
        (Key::Char('l'), "l".as_bytes()),
        (Key::Char('é'), "é".as_bytes()),
        (Key::Enter, b"\r".as_slice()),
        (Key::Backspace, b"\x7f".as_slice()),
    ] {
        assert_eq!(
            keys.handle(key).unwrap(),
            Action::Send(frames::input(bytes, None).unwrap())
        );
    }
    assert_eq!(
        keys.handle(Key::Paste("text".to_owned())).unwrap(),
        Action::Send(frames::paste("text", None).unwrap())
    );
    assert!(keys.prompt.text.is_empty(), "the chat line is untouched");
}

#[test]
fn an_oversize_paste_is_refused_before_it_leaves() {
    let mut keys = Keys::new(CAPABILITY);
    keys.prompt.focus = Focus::Document;
    let refused = keys.handle(Key::Paste("x".repeat(MAX_PASTE_BYTES + 1)));
    assert!(matches!(refused, Err(FrameError::OutOfContract(_))));
}

#[test]
fn selector_focus_sends_nothing_for_typing() {
    let mut keys = Keys::new(CAPABILITY);
    keys.prompt.focus = Focus::Selector;
    assert_eq!(keys.handle(Key::Char('x')).unwrap(), Action::Redraw);
    assert_eq!(keys.handle(Key::Enter).unwrap(), Action::Redraw);
    assert!(keys.prompt.text.is_empty());
}

#[test]
fn global_keys_work_from_every_focus() {
    for focus in [Focus::Selector, Focus::Document, Focus::Chat] {
        let mut keys = Keys::new(CAPABILITY);
        keys.prompt.focus = focus;
        assert_eq!(
            keys.handle(Key::CtrlR).unwrap(),
            Action::Send(frames::records(CAPABILITY, Some(ROOM_PREFIX), None).unwrap())
        );
        assert_eq!(
            keys.handle(Key::Resize(80, 24)).unwrap(),
            Action::Send(frames::resize(80, 24).unwrap())
        );
        assert_eq!(keys.handle(Key::Esc).unwrap(), Action::Quit);
        assert_eq!(keys.handle(Key::CtrlC).unwrap(), Action::Quit);
    }
}

#[test]
fn a_resize_outside_the_contract_is_refused() {
    let mut keys = Keys::new(CAPABILITY);
    assert!(keys.handle(Key::Resize(0, 24)).is_err());
}
