//! Three-panel text projection for the TUI shell (#12818, issue #1071).
//!
//! The statement's `text_and_form` presentation is a deterministic
//! text projection across three panels: the room/object selector, the
//! room/document, and the chatroom. [`project`] builds all three from a
//! folded [`Session`] and nothing else, so the same fold always yields
//! the same lines. The ratatui layer draws these lines; it adds layout,
//! not content.
//!
//! Every line is escape-free: control characters from the host (chat
//! text, record values, surface projections) are replaced with U+FFFD
//! before they reach a panel, so a host frame can never move the
//! terminal's cursor or change its modes.
//!
//! - **Selector:** the last `records` reply, one line per record in key
//!   order, then the entities the session's `entity_input` events
//!   touched.
//! - **Document:** the presented view. A view with a surface the shell
//!   understands shows that surface's own text projection (a form shows
//!   its fields and actions); otherwise the view's `data` text, the same
//!   fallback rule the terminal shell follows. While the view presents a
//!   form, the host's last `form_result_status` for that form follows it,
//!   and while the form is in force, how to answer it.
//! - **Chat:** the durable events since the last snapshot, oldest first,
//!   each named by sequence and by the participant's display name from
//!   the snapshot.

use id::world_presentation::WorldSurface;
use serde_json::Value;

use crate::form;
use crate::session::Session;

/// Shown in the selector before any `records` reply has arrived.
pub const NO_RECORDS: &str = "(no records yet)";

/// Shown in the document before any view has arrived.
pub const NO_VIEW: &str = "(no view yet)";

/// Shown in the document when a reconnect could not prove continuity:
/// the shell holds no view until the host's fresh snapshot arrives.
pub const DIVERGED: &str = "(history diverged: waiting for a fresh snapshot)";

/// Shown in the chat before any event has been folded.
pub const NO_MESSAGES: &str = "(no messages yet)";

/// The three panels' lines, top to bottom within each panel.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Panels {
    /// Room/object selector.
    pub selector: Vec<String>,
    /// Room/document: the presented view.
    pub document: Vec<String>,
    /// Chatroom: the folded events.
    pub chat: Vec<String>,
}

/// Project a folded session onto the three panels.
#[must_use]
pub fn project(session: &Session) -> Panels {
    Panels {
        selector: selector_lines(session),
        document: document_lines(session),
        chat: chat_lines(session),
    }
}

/// Replace every control character with U+FFFD so a line carries no
/// terminal escape and no line break.
#[must_use]
pub fn escape_free(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { '\u{FFFD}' } else { c })
        .collect()
}

/// Split multi-line text into escape-free lines.
fn lines_of(text: &str) -> impl Iterator<Item = String> + '_ {
    text.lines().map(escape_free)
}

/// A record value as one line: strings verbatim, anything else as
/// compact JSON.
fn value_text(value: &Value) -> String {
    value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_owned)
}

/// The room/object selector's lines.
#[must_use]
pub fn selector_lines(session: &Session) -> Vec<String> {
    let mut lines = Vec::new();
    match session
        .records()
        .and_then(|frame| frame["records"].as_object())
    {
        Some(records) if !records.is_empty() => {
            let mut keys: Vec<&String> = records.keys().collect();
            keys.sort();
            for key in keys {
                lines.push(escape_free(&format!(
                    "{key}  {}",
                    value_text(&records[key.as_str()])
                )));
            }
        }
        _ => lines.push(NO_RECORDS.to_owned()),
    }
    if let Some(after) = session
        .records()
        .and_then(|frame| frame["next_after"].as_str())
    {
        lines.push(escape_free(&format!("more after {after}")));
    }
    let mut objects: Vec<&str> = session
        .events()
        .iter()
        .filter(|event| event["kind"]["kind"] == "entity_input")
        .filter_map(|event| event["kind"]["data"]["entity_ref"].as_str())
        .collect();
    objects.sort_unstable();
    objects.dedup();
    for object in objects {
        lines.push(escape_free(&format!("object {object}")));
    }
    lines
}

/// The room/document's lines.
#[must_use]
pub fn document_lines(session: &Session) -> Vec<String> {
    let mut lines: Vec<String> = match session.view() {
        Some(view) => {
            let surface = view
                .get("surface")
                .filter(|surface| !surface.is_null())
                .and_then(|surface| serde_json::from_value::<WorldSurface>(surface.clone()).ok());
            match surface {
                Some(surface) => lines_of(&surface.text_projection()).collect(),
                None => lines_of(view["data"].as_str().unwrap_or_default()).collect(),
            }
        }
        None if session.diverged() => vec![DIVERGED.to_owned()],
        None => vec![NO_VIEW.to_owned()],
    };
    let presented_form = session
        .view()
        .and_then(|view| view["surface"]["form_id"].as_str());
    if let Some(status) = session
        .last_form_status()
        .filter(|status| presented_form.is_some() && status["form_id"].as_str() == presented_form)
    {
        let form = status["form_id"].as_str().unwrap_or_default();
        let line = if status["ok"].as_bool() == Some(true) {
            format!("form {form}: accepted")
        } else {
            let why = status["error"]
                .as_str()
                .or_else(|| status["reason"].as_str())
                .unwrap_or("no reason given");
            format!("form {form}: rejected: {why}")
        };
        lines.push(String::new());
        lines.push(escape_free(&line));
    }
    if form::active(session).is_some() {
        lines.push(String::new());
        lines.push(form::ANSWER_HINT.to_owned());
    }
    lines
}

/// The display name for a participant: the snapshot's name, `host` for
/// host-authored events (participant 0), otherwise `#<id>`.
fn participant_name(session: &Session, id: Option<u64>) -> String {
    match id {
        Some(0) => "host".to_owned(),
        Some(id) => session
            .participants()
            .get(&id)
            .cloned()
            .unwrap_or_else(|| format!("#{id}")),
        None => "someone".to_owned(),
    }
}

/// One folded event as one chat line.
fn event_line(session: &Session, event: &Value) -> String {
    let sequence = event["sequence"].as_u64().unwrap_or_default();
    let who = participant_name(session, event["participant_id"].as_u64());
    let kind = &event["kind"];
    let data = &kind["data"];
    let what = match kind["kind"].as_str().unwrap_or_default() {
        "chat" => format!("{who}: {}", data.as_str().unwrap_or_default()),
        "input" => match data.as_array().map_or(0, Vec::len) {
            1 => format!("{who} sent input (1 byte)"),
            bytes => format!("{who} sent input ({bytes} bytes)"),
        },
        "entity_input" => format!(
            "{who} acted on {}",
            data["entity_ref"].as_str().unwrap_or_default()
        ),
        "notification" => format!("notice: {}", data["text"].as_str().unwrap_or_default()),
        "system" => "system event".to_owned(),
        "program_installed" => "program changed".to_owned(),
        other => format!("event: {other}"),
    };
    escape_free(&format!("[{sequence}] {what}"))
}

/// The chatroom's lines.
#[must_use]
pub fn chat_lines(session: &Session) -> Vec<String> {
    if session.events().is_empty() {
        return vec![NO_MESSAGES.to_owned()];
    }
    session
        .events()
        .iter()
        .map(|event| event_line(session, event))
        .collect()
}
