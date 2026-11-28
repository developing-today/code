//! The custom-GUI table, as data.
//!
//! Transcribed from decision record #5853
//! (`doc/2026-11-08_decision_id5853_custom_gui_rendering_overrides.md`):
//! the per-kind interaction state sets of §2.3 — the oracle the
//! state-completeness check runs against. Per #5855 invariant 6
//! ("tables are data"), a change to a kind's state set is a change
//! to this table, never to the lowering code in
//! [`crate::lower_gui`].
//!
//! The state names are the record's (§2.3: default, hover, focus,
//! active, disabled, and error where applicable), aligned with
//! the #5850 §5 state vocabulary the parent design fixes.

/// The state set of a kind with no interaction states of its own:
/// only the default state exists to cover.
pub const DEFAULT_STATES: &[&str] = &["default"];

/// #5853 §2.3 state sets for the interactive kinds. `field`
/// additionally carries `error` (its validation state is part of
/// its interaction contract); the remaining interactive kinds
/// share the five-state set the record lists.
pub const STATE_SETS: &[(&str, &[&str])] = &[
    (
        "button",
        &["default", "hover", "focus", "active", "disabled"],
    ),
    (
        "field",
        &["default", "hover", "focus", "active", "disabled", "error"],
    ),
    ("link", &["default", "hover", "focus", "active", "disabled"]),
    ("tab", &["default", "hover", "focus", "active", "disabled"]),
    (
        "menu-item",
        &["default", "hover", "focus", "active", "disabled"],
    ),
];

/// The full interaction state set an override for `kind` must
/// cover (§2.3): the kind's row, or [`DEFAULT_STATES`] for kinds
/// with no interaction states.
#[must_use]
pub fn state_set(kind: &str) -> &'static [&'static str] {
    STATE_SETS
        .iter()
        .find(|(k, _)| *k == kind)
        .map_or(DEFAULT_STATES, |(_, states)| *states)
}
