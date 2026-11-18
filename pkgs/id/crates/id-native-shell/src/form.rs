//! Trusted form widgets for the native shell (#8938, the native half).
//!
//! The reference presenter is the SSH terminal
//! ([`id::world_presentation`] + `world_ssh`'s form prompt): a form is
//! rendered only when the view carries a presentation token AND this
//! presenter's own input manifest can collect every field, a
//! `form_result` frame carries the view's `form_id` and
//! `presentation_token`, and a `form_result_status` rejection carrying a
//! fresh token re-presents the form with that token.
//!
//! Everything here is pure: no Bevy, no I/O. The Bevy layer renders the
//! widget rows this module produces, and the session folds host frames
//! through [`FormPresenter`] so the native shell and the web presenter
//! (`pkgs/id/web/src/world-form.ts`) answer the contract identically.

use id::world_presentation::{FormActionKind, FormSurface, WorldSurface};
use serde_json::{Map, Value, json};

/// Local input affordances of the native shell presenter.
///
/// Deliberately this presenter's own manifest, not the headless server's
/// `device.info`: the shell reads typed key bytes and clipboard pastes
/// ([`crate::events`]) and has no pointer surface, so it can collect
/// text/number/date fields but not choice or confirm controls. Forms
/// that need a discrete control therefore render as text here, exactly
/// as the SSH terminal without an encoder does.
#[must_use]
pub fn native_form_input_manifest() -> Value {
    json!({
        "keys": 128,
        "encoder": false,
        "pointer": false,
        "text_entry": "keyboard",
    })
}

/// Maximum bytes of one `form_result` frame.
///
/// The contract's `max_frame_bytes` (1 MiB) is the bound the host
/// enforces; the web presenter uses the same value.
pub const MAX_FORM_RESULT_BYTES: usize = 1024 * 1024;

/// One rendered widget row derived from a trusted form field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormWidgetRow {
    /// Field id the answer is keyed by.
    pub id: String,
    /// Visible caption (`label`, plus a required marker).
    pub caption: String,
    /// Hint shown beside the caption (bounds/options), never markup.
    pub hint: String,
    /// Whether the field must be answered.
    pub required: bool,
}

/// One rendered action button derived from a trusted form action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormWidgetAction {
    /// Action id sent back in `form_result.action`.
    pub id: String,
    /// Visible button label.
    pub label: String,
    /// Whether this action abandons the form.
    pub cancel: bool,
}

/// A trusted form ready for the renderer: widgets built only from this
/// client's manifest and the closed field/action model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormWidgets {
    /// Form id the presentation answers.
    pub form_id: String,
    /// Presentation token in force right now.
    pub presentation_token: String,
    /// Escape-free title.
    pub title: String,
    /// One row per field.
    pub rows: Vec<FormWidgetRow>,
    /// One button per action.
    pub actions: Vec<FormWidgetAction>,
}

/// Outcome of one `form_result_status` reply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormStatusOutcome {
    /// The answer was accepted; the form is finished.
    Accepted,
    /// The answer was rejected and the form was re-presented with a
    /// host-issued token; the renderer must redraw.
    RePresented(Box<FormSurface>),
    /// The form is gone (closed, expired, or never ours).
    Closed {
        /// Host error code (`form_unavailable` when absent).
        error: String,
        /// Host reason (`unknown` when absent).
        reason: String,
    },
}

/// Alias pairing the token a view still carries with the host's current
/// token, so a redraw of the program's stale view does not clobber the
/// rotated one (the same rule as `world_ssh`'s `form_retry_alias`).
#[derive(Clone, Debug, PartialEq, Eq)]
struct RetryAlias {
    form_id: String,
    view_token: String,
    current_token: String,
}

/// Pure presenter state: the form currently trusted, if any.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FormPresenter {
    active: Option<FormSurface>,
    retry_alias: Option<RetryAlias>,
    manifest: Option<Value>,
}

impl FormPresenter {
    /// A presenter with no form, using [`native_form_input_manifest`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A presenter whose trust decisions use `manifest` instead of the
    /// shipped shell's own — for a shell variant with different
    /// affordances (and for the untrusted-manifest tests).
    #[must_use]
    pub fn with_manifest(manifest: Value) -> Self {
        Self {
            manifest: Some(manifest),
            ..Self::default()
        }
    }

    /// The form currently rendered, or `None` when none is trusted.
    #[must_use]
    pub fn active(&self) -> Option<&FormSurface> {
        self.active.as_ref()
    }

    /// The presentation token currently in force.
    #[must_use]
    pub fn token(&self) -> Option<&str> {
        self.active.as_ref()?.presentation_token.as_deref()
    }

    /// Apply one presented surface.
    ///
    /// Returns the widgets to render when this presenter can safely
    /// render the form, or `None` when the view is not a form or the
    /// form is not trusted (invalid document, missing token, or a field
    /// this presenter's manifest cannot collect) — the caller then
    /// renders the surface's escape-free text projection instead.
    pub fn present(&mut self, surface: Option<&WorldSurface>) -> Option<FormWidgets> {
        let Some(WorldSurface::Form(form)) = surface else {
            // Any non-form view replaces the form.
            self.active = None;
            self.retry_alias = None;
            return None;
        };
        let manifest = self
            .manifest
            .clone()
            .unwrap_or_else(native_form_input_manifest);
        if form.presentation_token.is_none() || form.validate_device_input(&manifest).is_err() {
            self.active = None;
            self.retry_alias = None;
            return None;
        }
        let mut form = form.clone();
        // A redraw may still carry the program's pre-rejection token
        // while a retry token is in force; keep the host-issued one.
        if let Some(alias) = &self.retry_alias
            && alias.form_id == form.form_id
        {
            if form.presentation_token.as_deref() == Some(alias.view_token.as_str()) {
                form.presentation_token = Some(alias.current_token.clone());
            } else {
                self.retry_alias = None;
            }
        }
        let widgets = widgets_for(&form);
        self.active = Some(form);
        widgets
    }

    /// Apply one `form_result_status` reply, mirroring the SSH
    /// presenter's token rotation.
    ///
    /// Returns `None` when the reply names some other form (nothing to
    /// do), otherwise the outcome for this presenter's form.
    pub fn settle(&mut self, status: &Value) -> Option<FormStatusOutcome> {
        let form_id = status.get("form_id").and_then(Value::as_str)?;
        let active = self.active.as_ref()?;
        if active.form_id != form_id {
            return None;
        }
        if status.get("ok").and_then(Value::as_bool) == Some(true) {
            self.active = None;
            self.retry_alias = None;
            return Some(FormStatusOutcome::Accepted);
        }
        if let Some(token) = status.get("presentation_token").and_then(Value::as_str) {
            let mut form = self.active.clone()?;
            if let Some(previous) = form.presentation_token.clone() {
                self.retry_alias = Some(RetryAlias {
                    form_id: form.form_id.clone(),
                    view_token: self
                        .retry_alias
                        .as_ref()
                        .filter(|alias| {
                            alias.form_id == form.form_id && alias.current_token == previous
                        })
                        .map_or_else(|| previous.clone(), |alias| alias.view_token.clone()),
                    current_token: token.to_owned(),
                });
            }
            form.presentation_token = Some(token.to_owned());
            self.active = Some(form.clone());
            return Some(FormStatusOutcome::RePresented(Box::new(form)));
        }
        self.active = None;
        self.retry_alias = None;
        Some(FormStatusOutcome::Closed {
            error: status
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("form_unavailable")
                .to_owned(),
            reason: status
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_owned(),
        })
    }
}

impl FormPresenter {
    /// The widgets for the form currently in force, if any.
    #[must_use]
    pub fn widgets(&self) -> Option<FormWidgets> {
        self.active.as_ref().and_then(widgets_for)
    }

    /// The `form_result` frame for one action of the form in force.
    ///
    /// # Errors
    ///
    /// Same failures as [`form_result_frame`], plus "no active form".
    pub fn answer(&self, action_id: &str, values: &Map<String, Value>) -> Result<Value, String> {
        let form = self
            .active
            .as_ref()
            .ok_or_else(|| "no form is presented".to_owned())?;
        form_result_frame(form, action_id, values)
    }
}

/// Build the widget model for an already-trusted form.
#[must_use]
pub fn widgets_for(form: &FormSurface) -> Option<FormWidgets> {
    use id::world_presentation::FormField;
    let token = form.presentation_token.clone()?;
    let rows = form
        .fields
        .iter()
        .map(|field| {
            let (id, label, required, hint) = match field {
                FormField::Text {
                    id,
                    label,
                    required,
                    max_length,
                } => (id, label, *required, format!("text, max {max_length}")),
                FormField::Choice {
                    id,
                    label,
                    required,
                    options,
                } => (
                    id,
                    label,
                    *required,
                    format!("choice: {}", options.join(", ")),
                ),
                FormField::Number {
                    id,
                    label,
                    required,
                    min,
                    max,
                } => (id, label, *required, format!("number: {min}..{max}")),
                FormField::Date {
                    id,
                    label,
                    required,
                    min,
                    max,
                } => (
                    id,
                    label,
                    *required,
                    format!(
                        "date: {}..{}",
                        min.as_deref().unwrap_or("*"),
                        max.as_deref().unwrap_or("*")
                    ),
                ),
                FormField::Confirm {
                    id,
                    label,
                    required,
                } => (id, label, *required, "yes/no".to_owned()),
            };
            FormWidgetRow {
                id: id.clone(),
                caption: if required {
                    format!("{label} *")
                } else {
                    label.clone()
                },
                hint,
                required,
            }
        })
        .collect();
    let actions = form
        .actions
        .iter()
        .map(|action| FormWidgetAction {
            id: action.id.clone(),
            label: action.label.clone(),
            cancel: action.kind == FormActionKind::Cancel,
        })
        .collect();
    Some(FormWidgets {
        form_id: form.form_id.clone(),
        presentation_token: token,
        title: form.title.clone(),
        rows,
        actions,
    })
}

/// Build the `form_result` frame for one action of a trusted form.
///
/// Mirrors the web presenter: the token must be in force, the action
/// must belong to the form, and a cancel carries no values.
///
/// # Errors
///
/// Returns the reason the frame cannot be built: no token, an unknown
/// action, or an oversized frame.
pub fn form_result_frame(
    form: &FormSurface,
    action_id: &str,
    values: &Map<String, Value>,
) -> Result<Value, String> {
    let token = form
        .presentation_token
        .as_deref()
        .ok_or_else(|| "form has no presentation token".to_owned())?;
    let action = form
        .actions
        .iter()
        .find(|candidate| candidate.id == action_id)
        .ok_or_else(|| format!("unknown form action `{action_id}`"))?;
    let values = if action.kind == FormActionKind::Cancel {
        Map::new()
    } else {
        values.clone()
    };
    let frame = json!({
        "type": "form_result",
        "form_id": form.form_id,
        "presentation_token": token,
        "action": action.id,
        "values": Value::Object(values),
    });
    if frame.to_string().len() > MAX_FORM_RESULT_BYTES {
        return Err(format!("form result exceeds {MAX_FORM_RESULT_BYTES} bytes"));
    }
    Ok(frame)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";
    const NEXT_TOKEN: &str = "fedcba9876543210fedcba9876543210";

    fn form_value(token: Option<&str>) -> Value {
        let mut value = json!({
            "kind": "form",
            "version": 1,
            "form_id": "signup",
            "title": "Sign up",
            "fields": [
                {"kind": "text", "id": "name", "label": "Name", "required": true, "max_length": 40}
            ],
            "actions": [
                {"id": "submit", "label": "Send"},
                {"id": "cancel", "label": "Cancel", "kind": "cancel"}
            ]
        });
        if let Some(token) = token {
            value["presentation_token"] = json!(token);
        }
        value
    }

    fn surface(token: Option<&str>) -> WorldSurface {
        serde_json::from_value(form_value(token)).unwrap()
    }

    fn form_of(surface: &WorldSurface) -> FormSurface {
        match surface {
            WorldSurface::Form(form) => form.clone(),
            _ => panic!("form surface"),
        }
    }

    #[test]
    fn trusted_form_renders_widget_rows_and_actions() {
        let presented = surface(Some(TOKEN));
        let mut presenter = FormPresenter::new();
        let widgets = presenter.present(Some(&presented)).expect("trusted");
        assert_eq!(widgets.form_id, "signup");
        assert_eq!(widgets.presentation_token, TOKEN);
        assert_eq!(widgets.rows.len(), 1);
        assert_eq!(widgets.rows[0].caption, "Name *");
        assert!(widgets.rows[0].required);
        assert_eq!(widgets.actions.len(), 2);
        assert!(widgets.actions[1].cancel);
        assert_eq!(presenter.token(), Some(TOKEN));
    }

    #[test]
    fn untrusted_manifest_and_missing_token_render_no_widgets() {
        // A manifest with no text entry cannot collect a text field.
        let mut presenter = FormPresenter::with_manifest(json!({
            "keys": 0, "encoder": false, "pointer": false, "text_entry": "none",
        }));
        // A choice field needs a discrete control this presenter does
        // not have.
        let choice: WorldSurface = serde_json::from_value(json!({
            "kind": "form", "version": 1, "form_id": "pick", "title": "Pick",
            "presentation_token": TOKEN,
            "fields": [{"kind": "choice", "id": "plan", "label": "Plan",
                        "required": false, "options": ["free", "pro"]}],
            "actions": [{"id": "submit", "label": "Send"}]
        }))
        .unwrap();
        assert!(presenter.present(Some(&choice)).is_none());
        assert!(presenter.active().is_none());
        // The same form is trusted by the shipped shell's manifest,
        // whose key count is a discrete control.
        let mut shipped = FormPresenter::new();
        assert!(shipped.present(Some(&choice)).is_some());
        // A form without a presentation token is never actionable.
        let mut tokenless_presenter = FormPresenter::new();
        let tokenless = surface(None);
        assert!(tokenless_presenter.present(Some(&tokenless)).is_none());
        assert!(tokenless_presenter.token().is_none());
    }

    #[test]
    fn non_form_surface_clears_the_active_form() {
        let presented = surface(Some(TOKEN));
        let mut presenter = FormPresenter::new();
        assert!(presenter.present(Some(&presented)).is_some());
        let text: WorldSurface = serde_json::from_value(
            json!({"kind": "text", "version": 1, "width": 10, "lines": ["hi"]}),
        )
        .unwrap();
        assert!(presenter.present(Some(&text)).is_none());
        assert!(presenter.active().is_none());
    }

    #[test]
    fn form_result_frame_carries_id_and_token_and_cancel_sends_no_values() {
        let form = form_of(&surface(Some(TOKEN)));
        let mut values = Map::new();
        values.insert("name".to_owned(), json!("Ada"));
        let submit = form_result_frame(&form, "submit", &values).unwrap();
        assert_eq!(
            submit,
            json!({"type": "form_result", "form_id": "signup",
                   "presentation_token": TOKEN, "action": "submit",
                   "values": {"name": "Ada"}})
        );
        let cancel = form_result_frame(&form, "cancel", &values).unwrap();
        assert_eq!(cancel["values"], json!({}));
        assert_eq!(cancel["presentation_token"], json!(TOKEN));

        let tokenless = form_of(&surface(None));
        assert!(form_result_frame(&tokenless, "submit", &values).is_err());
        assert!(form_result_frame(&form, "nope", &values).is_err());
    }

    #[test]
    fn rejection_with_a_fresh_token_re_presents_with_that_token() {
        let presented = surface(Some(TOKEN));
        let mut presenter = FormPresenter::new();
        assert!(presenter.present(Some(&presented)).is_some());

        let outcome = presenter
            .settle(&json!({"type": "form_result_status", "form_id": "signup",
                            "ok": false, "presentation_token": NEXT_TOKEN}))
            .unwrap();
        let FormStatusOutcome::RePresented(form) = outcome else {
            panic!("expected re-present, got {outcome:?}");
        };
        assert_eq!(form.presentation_token.as_deref(), Some(NEXT_TOKEN));
        assert_eq!(presenter.token(), Some(NEXT_TOKEN));

        // A redraw carrying the program's stale token keeps the rotated one.
        let stale = surface(Some(TOKEN));
        assert!(presenter.present(Some(&stale)).is_some());
        assert_eq!(presenter.token(), Some(NEXT_TOKEN));

        // A redraw carrying some other token drops the alias.
        let other = surface(Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"));
        assert!(presenter.present(Some(&other)).is_some());
        assert_eq!(presenter.token(), Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"));
    }

    #[test]
    fn acceptance_and_closure_clear_the_form() {
        let mut presenter = FormPresenter::new();
        assert!(presenter.present(Some(&surface(Some(TOKEN)))).is_some());
        assert_eq!(
            presenter.settle(&json!({"form_id": "signup", "ok": true})),
            Some(FormStatusOutcome::Accepted)
        );
        assert!(presenter.active().is_none());

        assert!(presenter.present(Some(&surface(Some(TOKEN)))).is_some());
        assert_eq!(
            presenter.settle(&json!({"form_id": "signup", "ok": false,
                                     "error": "form_expired", "reason": "stale"})),
            Some(FormStatusOutcome::Closed {
                error: "form_expired".to_owned(),
                reason: "stale".to_owned()
            })
        );
        assert!(presenter.active().is_none());
    }

    #[test]
    fn status_for_another_form_is_ignored() {
        let mut presenter = FormPresenter::new();
        assert!(presenter.present(Some(&surface(Some(TOKEN)))).is_some());
        assert_eq!(
            presenter.settle(&json!({"form_id": "other", "ok": true})),
            None
        );
        assert!(presenter.active().is_some());
    }
}
