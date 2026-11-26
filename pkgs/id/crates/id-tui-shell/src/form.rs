//! Answering a presented form from the input line (#12818).
//!
//! The TUI follows the SSH terminal presenter, the reference text
//! presenter for forms: while a form is in force, an answer line is
//! either `/cancel` or one JSON object `{"action": "<id>", "values":
//! {...}}` keyed by field id. The answer is checked against the form's
//! own closed field model before it is sent, so a malformed answer
//! stays local as a status and never reaches the host.
//!
//! A form is in force while the view presents it with a presentation
//! token and the host has not settled it. A rejection that carries a
//! fresh token re-presents the form with that token; an acceptance or
//! a rejection without one closes it.

use id::world_presentation::{FormActionKind, FormResult, FormSurface, WorldSurface};
use serde_json::{Map, Value};

use crate::frames;
use crate::session::Session;

/// The command that cancels the form in force.
pub const CANCEL: &str = "/cancel";

/// Shown under a form in force: how to answer it.
pub const ANSWER_HINT: &str =
    "answer: {\"action\":\"<id>\",\"values\":{\"<field id>\":...}} or /cancel";

/// The form in force, with the presentation token an answer must carry.
#[must_use]
pub fn active(session: &Session) -> Option<FormSurface> {
    let surface = session.view()?.get("surface")?;
    let WorldSurface::Form(mut form) = serde_json::from_value(surface.clone()).ok()? else {
        return None;
    };
    form.presentation_token.as_ref()?;
    if let Some(status) = session
        .last_form_status()
        .filter(|status| status["form_id"].as_str() == Some(form.form_id.as_str()))
    {
        if status["ok"].as_bool() == Some(true) {
            return None;
        }
        let token = status["presentation_token"].as_str()?;
        form.presentation_token = Some(token.to_owned());
    }
    Some(form)
}

/// Whether a typed line is meant as a form answer rather than chat.
#[must_use]
pub fn is_answer(line: &str) -> bool {
    let line = line.trim();
    line == CANCEL || line.starts_with('{')
}

/// The `form_result` frame for one answer line.
///
/// # Errors
///
/// A short reason when the line is not an answer the form accepts: not
/// a JSON object of exactly `action` and `values`, an undeclared action
/// or field, a missing required value, a value of the wrong type or out
/// of bounds, or a form without a cancel action for `/cancel`.
pub fn answer(form: &FormSurface, line: &str) -> Result<Value, String> {
    let (action, values) = if line.trim() == CANCEL {
        let action = form
            .actions
            .iter()
            .find(|action| action.kind == FormActionKind::Cancel)
            .ok_or_else(|| "this form has no cancel action".to_owned())?;
        (action.id.clone(), Map::new())
    } else {
        let value: Value = serde_json::from_str(line.trim())
            .map_err(|_| "enter a JSON object with action and values".to_owned())?;
        let object = value
            .as_object()
            .ok_or_else(|| "a form answer must be a JSON object".to_owned())?;
        if object.len() != 2 || !object.contains_key("action") || !object.contains_key("values") {
            return Err("a form answer has exactly action and values".to_owned());
        }
        let action = object["action"]
            .as_str()
            .ok_or_else(|| "the answer's action must be a string".to_owned())?;
        let values = object["values"]
            .as_object()
            .cloned()
            .ok_or_else(|| "the answer's values must be an object".to_owned())?;
        (action.to_owned(), values)
    };
    let token = form
        .presentation_token
        .clone()
        .ok_or_else(|| "this form has no presentation token".to_owned())?;
    let result = FormResult {
        form_id: form.form_id.clone(),
        presentation_token: token,
        action,
        values,
    };
    form.validate_result(&result)
        .map_err(|error| error.to_string())?;
    frames::form_result(
        &result.form_id,
        &result.presentation_token,
        &result.action,
        &result.values,
        None,
    )
    .map_err(|error| error.to_string())
}
