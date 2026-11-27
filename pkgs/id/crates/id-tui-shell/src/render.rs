//! ratatui renderer for the TUI shell's three panels (#12818, #1071).
//!
//! [`draw`] lays out the [`Panels`] the session projects
//! ([`crate::present::project`]) and the input line into one terminal
//! frame:
//!
//! ```text
//! ┌ Rooms ──┐┌ Document ─────────────┐
//! │ room/…  ││ view text             │
//! │         │└───────────────────────┘
//! │         │┌ Chat ─────────────────┐
//! │         ││ [6] ann: hello        │
//! └─────────┘└───────────────────────┘
//! ┌ Say ─────────────────────────────┐
//! │ > typed text▏                    │
//! └──────────────────────────────────┘
//! ```
//!
//! The renderer adds layout only, never content: every line it draws
//! is a projected line or the user's own input, both escape-free. The
//! document wraps; the chat shows its newest lines, so a long room
//! keeps the latest message in view. Drawing is backend-agnostic, so
//! the same function renders to a real terminal and to ratatui's test
//! backend.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Text};
use ratatui::widgets::{Block, Paragraph, Wrap};

use crate::present::{Panels, escape_free};

/// Title of the room/object selector panel.
pub const SELECTOR_TITLE: &str = " Rooms ";
/// Title of the room/document panel.
pub const DOCUMENT_TITLE: &str = " Document ";
/// Title of the chatroom panel.
pub const CHAT_TITLE: &str = " Chat ";
/// Title of the input line.
pub const INPUT_TITLE: &str = " Say ";
/// Prompt drawn before the user's input.
pub const PROMPT: &str = "> ";

/// Which panel has keyboard focus.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Focus {
    /// The room/object selector.
    Selector,
    /// The room/document.
    Document,
    /// The chatroom and its input line.
    #[default]
    Chat,
}

/// What the user is typing, and where focus is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Prompt {
    /// The text typed so far.
    pub text: String,
    /// The focused panel; drawn with a bold border.
    pub focus: Focus,
    /// A one-line status shown in the input line's title, such as a
    /// refused session or a reconnect in progress.
    pub status: Option<String>,
}

/// The screen areas of one frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Areas {
    /// Room/object selector.
    pub selector: Rect,
    /// Room/document.
    pub document: Rect,
    /// Chatroom.
    pub chat: Rect,
    /// Input line.
    pub input: Rect,
}

/// Split a frame area into the three panels and the input line.
#[must_use]
pub fn areas(area: Rect) -> Areas {
    let [body, input] = Layout::vertical([Constraint::Fill(1), Constraint::Length(3)]).areas(area);
    let [selector, right] =
        Layout::horizontal([Constraint::Percentage(25), Constraint::Fill(1)]).areas(body);
    let [document, chat] =
        Layout::vertical([Constraint::Percentage(60), Constraint::Fill(1)]).areas(right);
    Areas {
        selector,
        document,
        chat,
        input,
    }
}

fn block(title: &str, focused: bool) -> Block<'_> {
    let block = Block::bordered().title(title);
    if focused {
        block.border_style(Style::new().add_modifier(Modifier::BOLD))
    } else {
        block
    }
}

fn text(lines: &[String]) -> Text<'_> {
    Text::from(
        lines
            .iter()
            .map(|line| Line::raw(line.as_str()))
            .collect::<Vec<_>>(),
    )
}

/// The newest lines that fit in `height` rows.
fn tail(lines: &[String], height: u16) -> &[String] {
    let keep = usize::from(height).min(lines.len());
    &lines[lines.len() - keep..]
}

/// Draw the panels and the input line into `frame`, and place the
/// cursor after the typed text.
pub fn draw(frame: &mut Frame<'_>, panels: &Panels, prompt: &Prompt) {
    let areas = areas(frame.area());

    frame.render_widget(
        Paragraph::new(text(&panels.selector))
            .block(block(SELECTOR_TITLE, prompt.focus == Focus::Selector)),
        areas.selector,
    );
    frame.render_widget(
        Paragraph::new(text(&panels.document))
            .wrap(Wrap { trim: false })
            .block(block(DOCUMENT_TITLE, prompt.focus == Focus::Document)),
        areas.document,
    );
    let chat_rows = areas.chat.height.saturating_sub(2);
    frame.render_widget(
        Paragraph::new(text(tail(&panels.chat, chat_rows)))
            .block(block(CHAT_TITLE, prompt.focus == Focus::Chat)),
        areas.chat,
    );

    let title = prompt.status.as_ref().map_or_else(
        || INPUT_TITLE.to_owned(),
        |status| format!("{INPUT_TITLE}· {} ", escape_free(status)),
    );
    let typed = Line::raw(format!("{PROMPT}{}", escape_free(&prompt.text)));
    let width = u16::try_from(typed.width()).unwrap_or(u16::MAX);
    frame.render_widget(
        Paragraph::new(typed).block(block(&title, prompt.focus == Focus::Chat)),
        areas.input,
    );
    if prompt.focus == Focus::Chat {
        let inner_right = areas.input.right().saturating_sub(2);
        frame.set_cursor_position(Position::new(
            areas
                .input
                .x
                .saturating_add(1)
                .saturating_add(width)
                .min(inner_right),
            areas.input.y.saturating_add(1),
        ));
    }
}
