//! Renderer oracles (issue #12818): the projected panels drawn into
//! ratatui's test backend. The rows checked here are what a terminal of
//! that size shows for the recorded session.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use id_tui_shell::present::{self, Panels};
use id_tui_shell::render::{self, Focus, Prompt};
use id_tui_shell::session::Session;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::Modifier;
use serde_json::Value;

fn recorded_panels() -> Panels {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../contract/sessions/tui.jsonl");
    let mut session = Session::new();
    for line in std::fs::read_to_string(path).unwrap().lines() {
        let entry: Value = serde_json::from_str(line).unwrap();
        if entry["from"] == "server" {
            session.apply(&entry["frame"]).unwrap();
        }
    }
    present::project(&session)
}

fn draw(width: u16, height: u16, panels: &Panels, prompt: &Prompt) -> Terminal<TestBackend> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| render::draw(frame, panels, prompt))
        .unwrap();
    terminal
}

fn row(buffer: &Buffer, y: u16) -> String {
    (0..buffer.area.width)
        .map(|x| buffer.cell((x, y)).unwrap().symbol().to_owned())
        .collect()
}

fn rows(buffer: &Buffer) -> Vec<String> {
    (0..buffer.area.height).map(|y| row(buffer, y)).collect()
}

fn shows(buffer: &Buffer, text: &str) -> bool {
    rows(buffer).iter().any(|row| row.contains(text))
}

#[test]
fn the_layout_splits_into_three_panels_and_an_input_line() {
    let areas = render::areas(Rect::new(0, 0, 80, 24));
    assert_eq!(areas.input, Rect::new(0, 21, 80, 3));
    assert_eq!(areas.selector, Rect::new(0, 0, 20, 21));
    assert_eq!(areas.document.x, 20);
    assert_eq!(areas.chat.x, 20);
    assert_eq!(areas.document.y + areas.document.height, areas.chat.y);
    assert_eq!(areas.chat.y + areas.chat.height, 21);
}

#[test]
fn the_recorded_session_is_drawn_in_its_panels() {
    let panels = recorded_panels();
    let terminal = draw(80, 24, &panels, &Prompt::default());
    let buffer = terminal.backend().buffer();
    for title in [
        render::SELECTOR_TITLE,
        render::DOCUMENT_TITLE,
        render::CHAT_TITLE,
        render::INPUT_TITLE,
    ] {
        assert!(shows(buffer, title), "missing {title:?}");
    }
    for line in panels
        .selector
        .iter()
        .chain(&panels.document)
        .chain(&panels.chat)
        .filter(|line| !line.is_empty())
    {
        assert!(
            shows(buffer, line),
            "missing {line:?}\n{}",
            rows(buffer).join("\n")
        );
    }
    let areas = render::areas(buffer.area);
    assert!(row(buffer, areas.selector.y + 1).contains("room/lobby  Lobby"));
    assert!(row(buffer, areas.document.y + 1).contains("Ship the update?"));
    assert!(row(buffer, areas.chat.y + 1).contains("[5] ann sent input (1 byte)"));
    assert!(row(buffer, areas.input.y + 1).contains(render::PROMPT.trim_end()));
}

#[test]
fn a_short_chat_panel_keeps_the_newest_line() {
    let panels = Panels {
        selector: vec![],
        document: vec![],
        chat: (1..=30).map(|n| format!("[{n}] line {n}")).collect(),
    };
    let terminal = draw(60, 16, &panels, &Prompt::default());
    let buffer = terminal.backend().buffer();
    assert!(shows(buffer, "[30] line 30"));
    assert!(!shows(buffer, "[1] line 1 "));
}

#[test]
fn the_cursor_follows_the_typed_text() {
    let prompt = Prompt {
        text: "hey".to_owned(),
        ..Prompt::default()
    };
    let mut terminal = draw(60, 16, &Panels::default(), &prompt);
    let areas = render::areas(Rect::new(0, 0, 60, 16));
    assert!(row(terminal.backend().buffer(), areas.input.y + 1).contains("> hey"));
    let cursor = terminal.get_cursor_position().unwrap();
    assert_eq!(
        cursor,
        Position::new(areas.input.x + 1 + 5, areas.input.y + 1)
    );
}

#[test]
fn typed_and_status_text_are_escape_free() {
    let prompt = Prompt {
        text: "a\u{1b}[2Jb".to_owned(),
        focus: Focus::Chat,
        status: Some("reconnecting\u{7}".to_owned()),
    };
    let terminal = draw(60, 16, &Panels::default(), &prompt);
    let buffer = terminal.backend().buffer();
    assert!(shows(buffer, "> a\u{fffd}[2Jb"));
    assert!(shows(buffer, "reconnecting\u{fffd}"));
    for row in rows(buffer) {
        assert!(!row.chars().any(char::is_control), "control in {row:?}");
    }
}

#[test]
fn the_focused_panel_has_a_bold_border() {
    let prompt = Prompt {
        focus: Focus::Selector,
        ..Prompt::default()
    };
    let terminal = draw(60, 16, &Panels::default(), &prompt);
    let buffer = terminal.backend().buffer();
    let areas = render::areas(buffer.area);
    let corner = |rect: Rect| buffer.cell((rect.x, rect.y)).unwrap().modifier;
    assert!(corner(areas.selector).contains(Modifier::BOLD));
    assert!(!corner(areas.chat).contains(Modifier::BOLD));
    assert!(!corner(areas.input).contains(Modifier::BOLD));
}
