//! Form prompt and answer helpers for the SSH terminal presenter: the
//! local input manifest, the line-oriented form prompt, and the
//! typed `form_result` frame built from one JSON answer line. Split
//! out of `world_ssh.rs` to keep that module under the production
//! line limit (#1411).

use serde_json::json;

use super::terminal_text;

/// Local input affordances of the SSH terminal presenter. This is deliberately
/// distinct from the headless server's `device.info` manifest.
pub(super) fn terminal_form_input_manifest() -> serde_json::Value {
    json!({
        "keys": 128,
        "encoder": false,
        "pointer": false,
        "text_entry": "keyboard",
    })
}

/// Safe, line-oriented prompt for the SSH presenter. The user's response is
/// parsed as JSON data and sent in a typed session frame; it is never shell or
/// markup input.
pub(super) fn terminal_form_prompt(form: &crate::world_presentation::FormSurface) -> String {
    use crate::world_presentation::FormActionKind;
    use crate::world_presentation::FormField;

    let actions = form
        .actions
        .iter()
        .map(|action| {
            let kind = match action.kind {
                FormActionKind::Submit => "submit",
                FormActionKind::Cancel => "cancel",
            };
            format!("{} — {} ({kind})", action.id, action.label)
        })
        .collect::<Vec<_>>()
        .join("\n");
    let fields = form
        .fields
        .iter()
        .map(|field| {
            let (id, label, details) = match field {
                FormField::Text {
                    id,
                    label,
                    required,
                    max_length,
                } => (
                    id,
                    label,
                    format!("text; required={required}; max_length={max_length}"),
                ),
                FormField::Choice {
                    id,
                    label,
                    required,
                    options,
                } => {
                    let options = options
                        .iter()
                        .map(|option| terminal_json_string(option))
                        .collect::<Vec<_>>()
                        .join(", ");
                    (
                        id,
                        label,
                        format!("choice; required={required}; options=[{options}]"),
                    )
                }
                FormField::Number {
                    id,
                    label,
                    required,
                    min,
                    max,
                } => (
                    id,
                    label,
                    format!("number; required={required}; min={min}; max={max}"),
                ),
                FormField::Date {
                    id,
                    label,
                    required,
                    min,
                    max,
                } => (
                    id,
                    label,
                    format!("date; required={required}; min={min:?}; max={max:?}"),
                ),
                FormField::Confirm {
                    id,
                    label,
                    required,
                } => (id, label, format!("confirm; required={required}")),
            };
            format!(
                "{} — {} [{}]",
                terminal_text(label),
                terminal_text(id),
                terminal_text(&details)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "{}\nFields (values are keyed by field ID):\n{}\nActions:\n{}\nEnter {{\"action\":\"<id>\",\"values\":{{...}}}} or /cancel.\nform> ",
        terminal_text(&form.title),
        fields,
        actions
    )
}

/// JSON-quote an option while escaping terminal-invisible format characters,
/// so the displayed value remains copyable and compares exactly on submit.
pub(super) fn terminal_json_string(value: &str) -> String {
    let quoted = serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_owned());
    let mut escaped = String::with_capacity(quoted.len());
    for ch in quoted.chars() {
        if crate::is_format_char(ch) {
            let mut units = [0; 2];
            for unit in ch.encode_utf16(&mut units).iter() {
                use std::fmt::Write as _;
                let _ = write!(escaped, "\\u{unit:04x}");
            }
        } else {
            escaped.push(ch);
        }
    }
    escaped
}

/// Validate one complete JSON response line and encode it as a typed session frame.
pub(super) fn terminal_form_answer_frame(
    form: &crate::world_presentation::FormSurface,
    line: &str,
) -> Result<String, String> {
    let (action, values) = if line.trim() == "/cancel" {
        let action = form
            .actions
            .iter()
            .find(|action| action.kind == crate::world_presentation::FormActionKind::Cancel)
            .ok_or_else(|| "this form has no cancel action".to_owned())?;
        (action.id.clone(), serde_json::Map::new())
    } else {
        let value: serde_json::Value = serde_json::from_str(line)
            .map_err(|_| "enter a JSON object with action and values".to_owned())?;
        let object = value
            .as_object()
            .ok_or_else(|| "form response must be a JSON object".to_owned())?;
        if object.len() != 2 || !object.contains_key("action") || !object.contains_key("values") {
            return Err("form response requires exactly action and values".to_owned());
        }
        let action = object["action"]
            .as_str()
            .ok_or_else(|| "form response action must be a string".to_owned())?;
        let values = object["values"]
            .as_object()
            .cloned()
            .ok_or_else(|| "form response values must be an object".to_owned())?;
        (action.to_owned(), values)
    };
    let token = form
        .presentation_token
        .as_deref()
        .ok_or_else(|| "form has no active presentation token".to_owned())?;
    let frame = serde_json::json!({
        "type": "form_result",
        "form_id": form.form_id,
        "presentation_token": token,
        "action": action,
        "values": values,
    })
    .to_string();
    if frame.len() > crate::frames::MAX_FRAME_BYTES {
        return Err("form response exceeds the frame size limit".to_owned());
    }
    Ok(frame)
}
