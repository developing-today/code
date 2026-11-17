//! #8938 native-shell half: a presented form is completable and
//! cancellable through this client, and a rejection re-presents with the
//! host-issued token.
//!
//! These tests drive the same frames the session loop sees (a `view`
//! carrying a `FormSurface`, then a `form_result_status` reply) and
//! assert the contract-visible outcome: the `form_result` frame carries
//! the view's `form_id` and current `presentation_token`, and an
//! untrusted-manifest form never produces an actionable widget. The
//! oracle is the SSH presenter in `id-worlds` (`world_ssh.rs`) plus the
//! machine-readable contract (`contract/shell-contract.v1.json`).

use id_native_shell::form::{
    FormPresenter, FormStatusOutcome, form_result_frame, native_form_input_manifest, widgets_for,
};
use id_native_shell::{ServerUpdate, SessionState};
use serde_json::{Map, Value, json};

const TOKEN: &str = "0123456789abcdef0123456789abcdef";
const NEXT_TOKEN: &str = "fedcba9876543210fedcba9876543210";

fn form_view(surface: Value) -> Value {
    json!({
        "type": "view",
        "sequence": 3,
        "data": "fallback text",
        "surface": surface,
        "view_revision": 0,
    })
}

fn signup_form(token: Option<&str>) -> Value {
    let mut surface = json!({
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
        ],
    });
    if let Some(token) = token {
        surface["presentation_token"] = json!(token);
    }
    surface
}

/// The view's surface as the session decodes it.
fn view_surface(state: &SessionState) -> id::world_presentation::WorldSurface {
    let view = state.view().expect("view applied");
    view.surface.clone().expect("surface decoded")
}

#[test]
fn submitted_form_result_carries_the_form_id_and_presentation_token() {
    let mut state = SessionState::default();
    assert_eq!(
        state
            .apply_server_frame(form_view(signup_form(Some(TOKEN))))
            .unwrap(),
        ServerUpdate::View
    );

    let mut presenter = FormPresenter::new();
    let surface = view_surface(&state);
    let widgets = presenter.present(Some(&surface)).expect("trusted form");
    assert_eq!(widgets.form_id, "signup");
    assert_eq!(widgets.presentation_token, TOKEN);

    let mut values = Map::new();
    values.insert("name".to_owned(), json!("Ada"));
    let frame = presenter.answer("submit", &values).expect("frame");
    assert_eq!(frame["type"], "form_result");
    assert_eq!(frame["form_id"], "signup");
    assert_eq!(frame["presentation_token"], TOKEN);
    assert_eq!(frame["action"], "submit");
    assert_eq!(frame["values"], json!({"name": "Ada"}));
}

#[test]
fn cancelled_form_result_carries_the_token_and_no_values() {
    let mut state = SessionState::default();
    state
        .apply_server_frame(form_view(signup_form(Some(TOKEN))))
        .unwrap();
    let surface = view_surface(&state);
    let mut presenter = FormPresenter::new();
    presenter.present(Some(&surface)).expect("trusted form");

    let mut values = Map::new();
    values.insert("name".to_owned(), json!("Ada"));
    let frame = presenter.answer("cancel", &values).expect("frame");
    assert_eq!(frame["action"], "cancel");
    assert_eq!(frame["presentation_token"], TOKEN);
    assert_eq!(frame["values"], json!({}));
}

#[test]
fn rejection_re_presents_with_the_returned_token_and_answers_with_it() {
    let mut state = SessionState::default();
    state
        .apply_server_frame(form_view(signup_form(Some(TOKEN))))
        .unwrap();
    let surface = view_surface(&state);
    let mut presenter = FormPresenter::new();
    presenter.present(Some(&surface)).expect("trusted form");

    // The host replies through the same session state the shell uses.
    assert_eq!(
        state
            .apply_server_frame(json!({
                "type": "form_result_status",
                "form_id": "signup",
                "ok": false,
                "presentation_token": NEXT_TOKEN,
            }))
            .unwrap(),
        ServerUpdate::FormResultStatus
    );
    let status = state.take_form_result_status().expect("retained reply");
    let outcome = presenter.settle(&status).expect("status for our form");
    let FormStatusOutcome::RePresented(re_presented) = outcome else {
        panic!("expected a re-present, got {outcome:?}");
    };
    assert_eq!(re_presented.presentation_token.as_deref(), Some(NEXT_TOKEN));

    // The next answer uses the rotated token, not the view's stale one.
    let frame = presenter.answer("submit", &Map::new()).expect("frame");
    assert_eq!(frame["presentation_token"], NEXT_TOKEN);
    assert!(state.take_form_result_status().is_none(), "consumed once");
}

#[test]
fn untrusted_manifest_renders_no_actionable_widget() {
    // A presenter whose manifest has no text entry cannot collect a text
    // field — the reference rule from `FormSurface::validate_device_input`
    // refuses it, so the same form must yield no widget and no answer.
    let mut state = SessionState::default();
    state
        .apply_server_frame(form_view(signup_form(Some(TOKEN))))
        .unwrap();
    let surface = view_surface(&state);

    let mut presenter = FormPresenter::with_manifest(json!({
        "keys": 0, "encoder": false, "pointer": false, "text_entry": "none",
    }));
    assert!(presenter.present(Some(&surface)).is_none());
    assert!(presenter.active().is_none());
    assert!(presenter.answer("submit", &Map::new()).is_err());

    // The shipped shell's manifest does trust it.
    let mut shipped = FormPresenter::new();
    assert!(shipped.present(Some(&surface)).is_some());

    // A form without a presentation token is never actionable either,
    // whatever the manifest.
    let mut presenters = FormPresenter::new();
    let mut tokenless_state = SessionState::default();
    tokenless_state
        .apply_server_frame(form_view(signup_form(None)))
        .unwrap();
    let tokenless = view_surface(&tokenless_state);
    assert!(presenters.present(Some(&tokenless)).is_none());
}

#[test]
fn widgets_and_manifest_match_the_reference_field_model() {
    let surface: id::world_presentation::WorldSurface =
        serde_json::from_value(signup_form(Some(TOKEN))).expect("surface");
    let id::world_presentation::WorldSurface::Form(form) = &surface else {
        panic!("form surface");
    };
    let widgets = widgets_for(form).expect("widgets");
    assert_eq!(widgets.rows.len(), 1);
    assert_eq!(widgets.rows[0].id, "name");
    assert!(widgets.rows[0].required);
    assert_eq!(widgets.actions.len(), 2);
    assert!(!widgets.actions[0].cancel);
    assert!(widgets.actions[1].cancel);

    // The manifest is this presenter's own: no pointer, keyboard text.
    let manifest = native_form_input_manifest();
    assert_eq!(manifest["text_entry"], "keyboard");
    assert_eq!(manifest["pointer"], false);
    assert_eq!(manifest["encoder"], false);
}

#[test]
fn frame_builder_rejects_an_unknown_action_and_a_tokenless_form() {
    let surface: id::world_presentation::WorldSurface =
        serde_json::from_value(signup_form(Some(TOKEN))).expect("surface");
    let id::world_presentation::WorldSurface::Form(form) = &surface else {
        panic!("form surface");
    };
    assert!(form_result_frame(form, "nope", &Map::new()).is_err());

    let tokenless: id::world_presentation::WorldSurface =
        serde_json::from_value(signup_form(None)).expect("surface");
    let id::world_presentation::WorldSurface::Form(form) = &tokenless else {
        panic!("form surface");
    };
    assert!(form_result_frame(form, "submit", &Map::new()).is_err());
}
