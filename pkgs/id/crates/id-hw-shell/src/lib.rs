//! ESP32-class hardware shell for `id` — more-shells v1 slice 3
//! (#10929; design `doc/2026-11-10T20-40-00Z_design_more_shells_v1`,
//! §3 "Hardware — ESP32-class, an interaction subset stated
//! plainly").
//!
//! This crate is the spike. It speaks the F13.1 shell contract
//! (`pkgs/id/contract/shell-contract.v1.json`) in exactly the
//! subset the design says survives at this device size:
//!
//! - **presence** — who/what is in the world, read at wake from
//!   the snapshot's participant list, with any system joined/left
//!   events the journal carries folded on top (the host pushes
//!   no live join notifications — presence is a wake-time read,
//!   which is what this shell's posture assumes);
//! - **journal-head state in compressed form** — the newest durable
//!   sequence the shell has folded, and nothing else about the
//!   journal;
//! - **confirm/act** — one deliberate input: answering a presented
//!   form choice with a `form_result` frame (submit/cancel actions,
//!   choice selections, yes/no confirmations).
//!
//! Rich rendering, free text entry, and world hosting do not
//! survive at this size; they are excluded by name in
//! [`capability_statement`], and the code makes the exclusions
//! real: the only client frames this crate can build are `join`
//! and `form_result`, and a form whose fields require free text is
//! refused with [`HwError::FreeTextExcluded`]. A hardware shell
//! claiming frames it cannot present is the failure mode the
//! design section exists to prevent.
//!
//! Power/attention posture: the device holds no live session open
//! to look alive. It wakes, joins, folds frames to the journal
//! head, acts if a choice is presented, and disconnects — a shell
//! that is off is absent, and the contract tolerates absent
//! shells. A wake that needs presence takes a fresh join, because
//! the snapshot is the only presence read the contract offers; a
//! wake that only needs the events since its recorded head
//! resumes with `after`. If that replay does not continue
//! contiguously from the recorded head, continuity is unprovable
//! (the journal may have been rewritten), so the shell discards
//! its folded presence and waits for a fresh snapshot — review
//! finding 3 of the design record.
//!
//! The crate is transport-free: the frame builders and the fold
//! are pure, so the same code runs over a real iroh session on a
//! device (or through the paired full node the design names) and
//! over recorded frames in tests. Every frame the shell builds is
//! checked against the embedded F13.1 contract before it is
//! returned, and every frame it folds is checked on the way in —
//! conformance on the frames it does send, per the design's Done.

use std::collections::BTreeSet;
use std::fmt;

use id::shell_contract::{Phase, ShellContract};
use id::world_presentation::{FormField, FormSurface, WorldSurface};
use serde_json::{Map, Value, json};

/// Shell-contract name this client speaks (F13.1).
pub const CONTRACT_NAME: &str = "shell";
/// Shell-contract major version implemented by this crate.
pub const CONTRACT_VERSION: u64 = 1;

/// The capability statement of this shell, in the design record's
/// terms: the client frames it sends, the server frames it
/// presents, the F13.7 providers it declares (none), its
/// power/attention posture, and its exclusions by name.
///
/// This statement is the recorded contract of the spike: the
/// crate's tests pin it to the F13.1 contract file and to the
/// crate's actual builders and fold, so it cannot drift silently
/// from the code.
#[must_use]
pub fn capability_statement() -> Value {
    json!({
        "shell": "id-hw-shell",
        "shell_class": "esp32-class hardware",
        "contract": {
            "name": CONTRACT_NAME,
            "version": CONTRACT_VERSION,
        },
        "frames_sent": ["join", "form_result"],
        "server_frames_presented": [
            "snapshot",
            "event",
            "view",
            "form_result_status",
            "error",
        ],
        "providers_declared": [],
        "posture": "wake-on-journal-head: no live session is held open; \
            the device joins, folds to the journal head, acts if a \
            choice is presented, and disconnects; a wake that needs \
            presence takes a fresh join (the snapshot is the only \
            presence read the contract offers), and a wake that only \
            needs the events since its recorded head resumes with \
            `after`, treating a non-contiguous replay as unprovable \
            continuity",
        "exclusions": [
            {
                "name": "rich_rendering",
                "detail": "screen, scene and audio surfaces are never \
                    rendered; a view is read only for its journal head \
                    and a presented form choice",
            },
            {
                "name": "free_text_entry",
                "detail": "chat, paste and input frames are never \
                    built; a form field that requires free text (text, \
                    number or date entry) makes the form unanswerable \
                    by this shell",
            },
            {
                "name": "world_hosting",
                "detail": "the shell never hosts a world; where the \
                    device class cannot hold a full iroh node it speaks \
                    to a paired full node it owns, with the contract's \
                    frames on the wire",
            },
            {
                "name": "provider_offers",
                "detail": "no F13.7 capability providers are declared \
                    or answered; provider_request frames are ignored",
            },
        ],
    })
}

/// Why a hardware-shell operation could not be carried out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HwError {
    /// The embedded F13.1 contract could not be loaded.
    Contract(String),
    /// A frame (built or received) violates the F13.1 contract.
    OutOfContract(String),
    /// The host refused the session with an `error` frame.
    HostRefused(String),
    /// A received frame is missing a field the fold requires.
    MalformedFrame(&'static str),
    /// `confirm` was called with no form choice in force.
    NoChoicePresented,
    /// The presented form carries no presentation token, so no
    /// answer can be correlated with the presentation.
    NoPresentationToken,
    /// The named action is not one of the presented form's actions.
    UnknownAction(String),
    /// A selection names a field the form does not have.
    UnknownField(String),
    /// A choice selection is not one of the field's declared
    /// options, or a confirmation selection is not a boolean.
    BadSelection {
        /// The field the bad selection targets.
        field: String,
    },
    /// A required selectable field has no selection.
    MissingSelection {
        /// The required field left unanswered.
        field: String,
    },
    /// The form has a field that requires free text entry, which
    /// this shell excludes by name: the whole form is unanswerable
    /// here rather than silently partially answered.
    FreeTextExcluded {
        /// The free-text field that excludes the form.
        field: String,
    },
}

impl fmt::Display for HwError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contract(why) => write!(f, "shell contract unavailable: {why}"),
            Self::OutOfContract(why) => write!(f, "frame outside the F13.1 contract: {why}"),
            Self::HostRefused(message) => write!(f, "world host refused the session: {message}"),
            Self::MalformedFrame(what) => write!(f, "host frame is malformed: {what}"),
            Self::NoChoicePresented => write!(f, "no form choice is currently presented"),
            Self::NoPresentationToken => {
                write!(f, "the presented form carries no presentation token")
            }
            Self::UnknownAction(action) => {
                write!(f, "the presented form has no action `{action}`")
            }
            Self::UnknownField(field) => {
                write!(f, "the presented form has no field `{field}`")
            }
            Self::BadSelection { field } => {
                write!(
                    f,
                    "selection for field `{field}` is not one of its declared options"
                )
            }
            Self::MissingSelection { field } => {
                write!(f, "required field `{field}` has no selection")
            }
            Self::FreeTextExcluded { field } => {
                write!(
                    f,
                    "field `{field}` requires free text entry, which this shell excludes"
                )
            }
        }
    }
}

impl std::error::Error for HwError {}

/// The kind of update one folded host frame produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HwUpdate {
    /// A complete snapshot arrived: head and presence were rebuilt
    /// from it and any presented choice was cleared.
    Snapshot,
    /// A durable journal event arrived and was folded.
    Event,
    /// An ephemeral view arrived: the head advanced and the
    /// presented choice now mirrors this view (or is cleared).
    View,
    /// The host answered a `form_result` with its status.
    FormResultStatus,
    /// A contract-valid frame of a kind this shell does not
    /// present; ignored, as the contract prescribes for frame
    /// types a shell does not know.
    Ignored,
}

/// One selectable field of a presented choice, in the compressed
/// form a device with a few controls can offer: a closed list of
/// options, or a yes/no confirmation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectableField {
    /// Field id, as named in the form.
    pub id: String,
    /// Human-readable label.
    pub label: String,
    /// Whether an answer is required before submit.
    pub required: bool,
    /// Declared options for a choice field; empty for a yes/no
    /// confirmation field (answer with a boolean selection).
    pub options: Vec<String>,
    /// Whether this field is a yes/no confirmation.
    pub confirm: bool,
}

/// The choice currently presented to the device, compressed from
/// the host's form surface: what a few buttons or an encoder can
/// act on, and nothing more.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresentedChoice {
    /// The form's id, echoed in the answer frame.
    pub form_id: String,
    /// The form's title.
    pub title: String,
    /// Action ids the device may take, in presented order.
    pub actions: Vec<String>,
    /// The fields the device can select among.
    pub selectable: Vec<SelectableField>,
}

/// The hardware shell's whole world state, deliberately
/// compressed: the journal head, the presence set, and the choice
/// currently presented. Nothing else is retained — no journal, no
/// rendered surface, no history.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HwShell {
    head: u64,
    presence: BTreeSet<String>,
    choice: Option<FormSurface>,
    last_status: Option<(bool, Option<String>)>,
    resume_expect: Option<u64>,
    diverged: bool,
}

impl HwShell {
    /// A shell with no folded state: head 0, nobody present, no
    /// choice in force.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The journal head: the newest durable sequence folded so
    /// far. This is the compressed journal-head state the design
    /// names — the device wakes on its change.
    #[must_use]
    pub const fn head(&self) -> u64 {
        self.head
    }

    /// Who/what is currently in the world, by participant name:
    /// seeded from the snapshot's participant list, with any
    /// system joined/left events the journal carries folded on
    /// top.
    #[must_use]
    pub const fn presence(&self) -> &BTreeSet<String> {
        &self.presence
    }

    /// Whether continuity with the recorded head proved
    /// unprovable on the last resume: the journal may have been
    /// rewritten, the folded presence was discarded, and only a
    /// fresh snapshot re-establishes state.
    #[must_use]
    pub const fn diverged(&self) -> bool {
        self.diverged
    }

    /// The outcome of the most recent `form_result_status` reply:
    /// whether the host accepted the answer, and its stated reason
    /// when it did not.
    #[must_use]
    pub fn last_status(&self) -> Option<(bool, Option<&str>)> {
        self.last_status
            .as_ref()
            .map(|(ok, reason)| (*ok, reason.as_deref()))
    }

    /// The choice currently presented, compressed for the device's
    /// controls; `None` when the latest view presents no form.
    #[must_use]
    pub fn choice(&self) -> Option<PresentedChoice> {
        self.choice.as_ref().map(|form| PresentedChoice {
            form_id: form.form_id.clone(),
            title: form.title.clone(),
            actions: form
                .actions
                .iter()
                .map(|action| action.id.clone())
                .collect(),
            selectable: form
                .fields
                .iter()
                .filter_map(|field| match field {
                    FormField::Choice {
                        id,
                        label,
                        required,
                        options,
                    } => Some(SelectableField {
                        id: id.clone(),
                        label: label.clone(),
                        required: *required,
                        options: options.clone(),
                        confirm: false,
                    }),
                    FormField::Confirm {
                        id,
                        label,
                        required,
                    } => Some(SelectableField {
                        id: id.clone(),
                        label: label.clone(),
                        required: *required,
                        options: Vec::new(),
                        confirm: true,
                    }),
                    _ => None,
                })
                .collect(),
        })
    }

    /// Build the opening `join` frame for a fresh session (no
    /// recorded head to resume from).
    ///
    /// # Errors
    ///
    /// [`HwError::OutOfContract`] if the built frame fails the
    /// embedded contract check — a programming error, surfaced
    /// rather than sent.
    pub fn join_frame(&self, capability: &str) -> Result<Value, HwError> {
        let frame = json!({
            "type": "join",
            "capability": capability,
            "after": Value::Null,
        });
        check_client(&frame, Phase::Opening)?;
        Ok(frame)
    }

    /// Build the `join` frame for a head-only wake: resume after
    /// the recorded journal head. The host replays the durable
    /// events after the cursor — it sends no snapshot on a resume
    /// join, so presence is NOT refreshed by this frame; a wake
    /// that needs presence uses [`HwShell::join_frame`] and reads
    /// the snapshot instead. The next durable event the host
    /// replays must continue contiguously from the recorded head;
    /// if it does not, [`HwShell::apply_server_frame`] marks the
    /// shell diverged and discards the folded presence (review
    /// finding 3). With no recorded head this is a fresh join.
    ///
    /// # Errors
    ///
    /// Same as [`HwShell::join_frame`].
    pub fn resume_frame(&mut self, capability: &str) -> Result<Value, HwError> {
        if self.head == 0 {
            return self.join_frame(capability);
        }
        self.resume_expect = Some(self.head + 1);
        let frame = json!({
            "type": "join",
            "capability": capability,
            "after": self.head,
        });
        check_client(&frame, Phase::Opening)?;
        Ok(frame)
    }

    /// Fold one host frame into the compressed state. The frame is
    /// validated against the embedded F13.1 contract first; frame
    /// kinds this shell does not present are ignored, as the
    /// contract prescribes.
    ///
    /// # Errors
    ///
    /// [`HwError::OutOfContract`] for a frame the contract
    /// rejects, [`HwError::HostRefused`] for an `error` frame, and
    /// [`HwError::MalformedFrame`] for a contract-valid frame
    /// missing a field the fold requires.
    pub fn apply_server_frame(&mut self, frame: &Value) -> Result<HwUpdate, HwError> {
        let contract = ShellContract::embedded().map_err(HwError::Contract)?;
        let kind = contract
            .check_server_frame(frame)
            .map_err(HwError::OutOfContract)?;
        match kind {
            "snapshot" => {
                let snapshot = &frame["snapshot"];
                self.head = snapshot["current_sequence"].as_u64().unwrap_or(0);
                // The snapshot's participant list is the host's
                // authoritative presence at this head; the events
                // it carries fold on top of it.
                self.presence.clear();
                if let Some(participants) = snapshot["participants"].as_array() {
                    for participant in participants {
                        if let Some(name) = participant["display_name"]
                            .as_str()
                            .or_else(|| participant["name"].as_str())
                        {
                            self.presence.insert(name.to_owned());
                        }
                    }
                }
                self.choice = None;
                self.resume_expect = None;
                self.diverged = false;
                if let Some(events) = snapshot["events"].as_array() {
                    for event in events {
                        self.fold_event(event);
                    }
                }
                Ok(HwUpdate::Snapshot)
            }
            "event" => {
                let event = &frame["event"];
                let sequence = event["sequence"].as_u64().ok_or(HwError::MalformedFrame(
                    "event frame has no unsigned sequence",
                ))?;
                if let Some(expect) = self.resume_expect.take()
                    && sequence != expect
                {
                    // The replay does not continue from the
                    // recorded head: the journal may have been
                    // rewritten, so continuity is unprovable and
                    // the folded presence is discarded. The gapped
                    // event itself is not folded either — until a
                    // fresh snapshot re-establishes state, only
                    // the head advances.
                    self.diverged = true;
                    self.presence.clear();
                    self.choice = None;
                    self.head = self.head.max(sequence);
                    return Ok(HwUpdate::Event);
                }
                self.fold_event(event);
                Ok(HwUpdate::Event)
            }
            "view" => {
                let sequence = frame["sequence"].as_u64().ok_or(HwError::MalformedFrame(
                    "view frame has no unsigned sequence",
                ))?;
                self.head = self.head.max(sequence);
                self.choice = frame
                    .get("surface")
                    .and_then(|surface| {
                        serde_json::from_value::<WorldSurface>(surface.clone()).ok()
                    })
                    .and_then(|surface| match surface {
                        WorldSurface::Form(form) => Some(form),
                        _ => None,
                    });
                Ok(HwUpdate::View)
            }
            "form_result_status" => {
                let ok = frame["ok"].as_bool().unwrap_or(false);
                let reason = frame["reason"]
                    .as_str()
                    .or_else(|| frame["error"].as_str())
                    .map(str::to_owned);
                self.last_status = Some((ok, reason));
                if ok {
                    self.choice = None;
                }
                Ok(HwUpdate::FormResultStatus)
            }
            "error" => Err(HwError::HostRefused(
                frame["message"]
                    .as_str()
                    .unwrap_or("unspecified error")
                    .to_owned(),
            )),
            _ => Ok(HwUpdate::Ignored),
        }
    }

    /// Build the `form_result` frame answering the presented
    /// choice with one deliberate act: `action_id` (a submit or
    /// cancel action of the form) plus `selections` naming a
    /// declared option per choice field and a boolean per
    /// confirmation field. A cancel answer carries no values, as
    /// the contract's presenters do.
    ///
    /// # Errors
    ///
    /// [`HwError::NoChoicePresented`] with no form in force;
    /// [`HwError::FreeTextExcluded`] when the form has a field
    /// requiring free text — the whole form is unanswerable by
    /// this shell, never silently partially answered;
    /// [`HwError::UnknownAction`], [`HwError::UnknownField`],
    /// [`HwError::BadSelection`] and [`HwError::MissingSelection`]
    /// for answers outside the presented choice.
    pub fn confirm(
        &self,
        action_id: &str,
        selections: &Map<String, Value>,
    ) -> Result<Value, HwError> {
        let form = self.choice.as_ref().ok_or(HwError::NoChoicePresented)?;
        let token = form
            .presentation_token
            .as_deref()
            .ok_or(HwError::NoPresentationToken)?;
        let action = form
            .actions
            .iter()
            .find(|candidate| candidate.id == action_id)
            .ok_or_else(|| HwError::UnknownAction(action_id.to_owned()))?;
        let values = if action.kind == id::world_presentation::FormActionKind::Cancel {
            Map::new()
        } else {
            validate_selections(form, selections)?
        };
        let frame = json!({
            "type": "form_result",
            "form_id": form.form_id,
            "presentation_token": token,
            "action": action.id,
            "values": Value::Object(values),
        });
        check_client(&frame, Phase::Joined)?;
        Ok(frame)
    }

    /// Fold one durable journal event: advance the head, and track
    /// presence from the system joined/left events.
    fn fold_event(&mut self, event: &Value) {
        if let Some(sequence) = event["sequence"].as_u64() {
            self.head = self.head.max(sequence);
        }
        if event["kind"]["kind"].as_str() != Some("system") {
            return;
        }
        let Some(raw) = event["kind"]["data"]["event"].as_str() else {
            return;
        };
        let Ok(system) = serde_json::from_str::<Value>(raw) else {
            return;
        };
        let Some(name) = system["participant"]["name"]
            .as_str()
            .or_else(|| system["participant"]["display_name"].as_str())
        else {
            return;
        };
        match system["event"].as_str() {
            Some("joined") => {
                self.presence.insert(name.to_owned());
            }
            Some("left") => {
                self.presence.remove(name);
            }
            _ => {}
        }
    }
}

/// Check one client frame this shell built against the embedded
/// contract, in the phase it will be sent.
fn check_client(frame: &Value, phase: Phase) -> Result<(), HwError> {
    let contract = ShellContract::embedded().map_err(HwError::Contract)?;
    contract
        .check_client_frame(frame, phase)
        .map_err(HwError::OutOfContract)?;
    Ok(())
}

/// The id of any form field, whatever its kind.
fn field_id(field: &FormField) -> &str {
    match field {
        FormField::Text { id, .. }
        | FormField::Choice { id, .. }
        | FormField::Number { id, .. }
        | FormField::Date { id, .. }
        | FormField::Confirm { id, .. } => id,
    }
}

/// Validate a submit answer against the presented form's fields:
/// every field must be selectable by this shell, every selection
/// must name a real field and a declared value, and every
/// required field must be answered.
fn validate_selections(
    form: &FormSurface,
    selections: &Map<String, Value>,
) -> Result<Map<String, Value>, HwError> {
    for field in &form.fields {
        match field {
            FormField::Choice { .. } | FormField::Confirm { .. } => {}
            other => {
                return Err(HwError::FreeTextExcluded {
                    field: field_id(other).to_owned(),
                });
            }
        }
    }
    for key in selections.keys() {
        if !form.fields.iter().any(|field| field_id(field) == key) {
            return Err(HwError::UnknownField(key.clone()));
        }
    }
    let mut values = Map::new();
    for field in &form.fields {
        let (id, required) = match field {
            FormField::Choice {
                id,
                required,
                options,
                ..
            } => {
                if let Some(selection) = selections.get(id) {
                    let chosen = selection
                        .as_str()
                        .ok_or_else(|| HwError::BadSelection { field: id.clone() })?;
                    if !options.iter().any(|option| option == chosen) {
                        return Err(HwError::BadSelection { field: id.clone() });
                    }
                    values.insert(id.clone(), Value::String(chosen.to_owned()));
                } else if *required {
                    return Err(HwError::MissingSelection { field: id.clone() });
                }
                continue;
            }
            FormField::Confirm { id, required, .. } => (id, required),
            _ => continue,
        };
        if let Some(selection) = selections.get(id) {
            let confirmed = selection
                .as_bool()
                .ok_or_else(|| HwError::BadSelection { field: id.clone() })?;
            values.insert(id.clone(), Value::Bool(confirmed));
        } else if *required {
            return Err(HwError::MissingSelection { field: id.clone() });
        }
    }
    Ok(values)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";

    fn contract() -> Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../contract/shell-contract.v1.json");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
        serde_json::from_str(&text).expect("contract parses")
    }

    fn system_event(sequence: u64, what: &str, name: &str) -> Value {
        let system = json!({"event": what, "participant": {"name": name}}).to_string();
        json!({
            "sequence": sequence,
            "kind": {"kind": "system", "data": {"event": system}},
        })
    }

    fn choice_form_surface() -> Value {
        json!({
            "kind": "form",
            "version": 1,
            "form_id": "hw-choice",
            "presentation_token": TOKEN,
            "title": "Ship the update?",
            "fields": [{
                "kind": "choice",
                "id": "when",
                "label": "When",
                "required": true,
                "options": ["now", "later"],
            }],
            "actions": [
                {"id": "submit", "label": "Confirm"},
                {"id": "cancel", "label": "Cancel", "kind": "cancel"},
            ],
        })
    }

    fn shell_with_choice() -> HwShell {
        let mut shell = HwShell::new();
        let view = json!({
            "type": "view",
            "sequence": 5,
            "data": "Ship the update?",
            "view_revision": 5,
            "surface": choice_form_surface(),
        });
        assert_eq!(shell.apply_server_frame(&view).unwrap(), HwUpdate::View);
        shell
    }

    fn selections(pairs: &[(&str, Value)]) -> Map<String, Value> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect()
    }

    /// Oracle: the F13.1 contract file itself — every frame kind
    /// the statement claims must exist in the contract, on the
    /// side the statement claims it for.
    #[test]
    fn statement_frames_exist_in_the_contract() {
        let contract = contract();
        let statement = capability_statement();
        for frame in statement["frames_sent"].as_array().unwrap() {
            let name = frame.as_str().unwrap();
            assert!(
                contract["client_frames"].get(name).is_some(),
                "statement claims client frame `{name}`, absent from the contract"
            );
        }
        for frame in statement["server_frames_presented"].as_array().unwrap() {
            let name = frame.as_str().unwrap();
            assert!(
                contract["server_frames"].get(name).is_some(),
                "statement claims server frame `{name}`, absent from the contract"
            );
        }
        assert_eq!(statement["providers_declared"], json!([]));
    }

    /// Oracle: the crate's own builders — the statement's
    /// frames_sent must be exactly the set of client frame types
    /// the builders can produce, no more and no fewer.
    #[test]
    fn statement_matches_the_builders() {
        let shell = shell_with_choice();
        let mut built = BTreeSet::new();
        built.insert(
            shell.join_frame("cap").unwrap()["type"]
                .as_str()
                .unwrap()
                .to_owned(),
        );
        let answer = shell
            .confirm("submit", &selections(&[("when", json!("now"))]))
            .unwrap();
        built.insert(answer["type"].as_str().unwrap().to_owned());
        let claimed: BTreeSet<String> = capability_statement()["frames_sent"]
            .as_array()
            .unwrap()
            .iter()
            .map(|frame| frame.as_str().unwrap().to_owned())
            .collect();
        assert_eq!(built, claimed);
    }

    #[test]
    fn snapshot_fold_rebuilds_presence_and_head() {
        let mut shell = HwShell::new();
        let snapshot = json!({
            "type": "snapshot",
            "snapshot": {
                "current_sequence": 3,
                "participants": [{"display_name": "ann", "id": 1}],
                "events": [
                    system_event(2, "joined", "bo"),
                    system_event(3, "left", "bo"),
                ],
            },
        });
        assert_eq!(
            shell.apply_server_frame(&snapshot).unwrap(),
            HwUpdate::Snapshot
        );
        assert_eq!(shell.head(), 3);
        assert_eq!(shell.presence(), &BTreeSet::from(["ann".to_owned()]));
        // A live joined event folds the same way.
        let event = json!({"type": "event", "event": system_event(4, "joined", "bo")});
        assert_eq!(shell.apply_server_frame(&event).unwrap(), HwUpdate::Event);
        assert_eq!(shell.head(), 4);
        assert!(shell.presence().contains("bo"));
    }

    #[test]
    fn resume_with_contiguous_replay_keeps_state() {
        let mut shell = HwShell::new();
        let snapshot = json!({
            "type": "snapshot",
            "snapshot": {
                "current_sequence": 3,
                "participants": [{"display_name": "ann", "id": 1}],
                "events": [],
            },
        });
        shell.apply_server_frame(&snapshot).unwrap();
        let frame = shell.resume_frame("cap").unwrap();
        assert_eq!(frame["after"], json!(3));
        let event = json!({"type": "event", "event": system_event(4, "joined", "bo")});
        shell.apply_server_frame(&event).unwrap();
        assert!(!shell.diverged());
        assert!(shell.presence().contains("ann"));
        assert!(shell.presence().contains("bo"));
    }

    /// Oracle: review finding 3 of the design record — a replay
    /// that does not continue from the recorded head leaves
    /// continuity unprovable, so folded presence is discarded.
    #[test]
    fn resume_with_a_gap_marks_divergence_and_drops_presence() {
        let mut shell = HwShell::new();
        let snapshot = json!({
            "type": "snapshot",
            "snapshot": {
                "current_sequence": 3,
                "participants": [{"display_name": "ann", "id": 1}],
                "events": [],
            },
        });
        shell.apply_server_frame(&snapshot).unwrap();
        shell.resume_frame("cap").unwrap();
        let event = json!({"type": "event", "event": system_event(7, "joined", "bo")});
        shell.apply_server_frame(&event).unwrap();
        assert!(shell.diverged());
        assert!(shell.presence().is_empty());
        assert_eq!(shell.head(), 7);
    }

    #[test]
    fn view_presents_the_choice_in_compressed_form() {
        let shell = shell_with_choice();
        assert_eq!(shell.head(), 5);
        let choice = shell.choice().expect("choice presented");
        assert_eq!(choice.form_id, "hw-choice");
        assert_eq!(
            choice.actions,
            vec!["submit".to_owned(), "cancel".to_owned()]
        );
        assert_eq!(choice.selectable.len(), 1);
        assert_eq!(choice.selectable[0].id, "when");
        assert_eq!(
            choice.selectable[0].options,
            vec!["now".to_owned(), "later".to_owned()]
        );
        // A non-form view clears the presented choice.
        let mut shell = shell;
        let text_view = json!({
            "type": "view",
            "sequence": 6,
            "data": "all quiet",
            "view_revision": 6,
        });
        shell.apply_server_frame(&text_view).unwrap();
        assert!(shell.choice().is_none());
    }

    #[test]
    fn confirm_builds_the_contract_form_result() {
        let shell = shell_with_choice();
        let frame = shell
            .confirm("submit", &selections(&[("when", json!("now"))]))
            .unwrap();
        assert_eq!(
            frame,
            json!({
                "type": "form_result",
                "form_id": "hw-choice",
                "presentation_token": TOKEN,
                "action": "submit",
                "values": {"when": "now"},
            })
        );
        // Cancel carries no values.
        let frame = shell.confirm("cancel", &Map::new()).unwrap();
        assert_eq!(frame["values"], json!({}));
    }

    #[test]
    fn confirm_refuses_answers_outside_the_choice() {
        let shell = shell_with_choice();
        assert_eq!(
            shell.confirm("bogus", &selections(&[("when", json!("now"))])),
            Err(HwError::UnknownAction("bogus".to_owned()))
        );
        assert_eq!(
            shell.confirm("submit", &selections(&[("when", json!("eventually"))])),
            Err(HwError::BadSelection {
                field: "when".to_owned()
            })
        );
        assert_eq!(
            shell.confirm("submit", &Map::new()),
            Err(HwError::MissingSelection {
                field: "when".to_owned()
            })
        );
        assert_eq!(
            shell.confirm("submit", &selections(&[("where", json!("now"))])),
            Err(HwError::UnknownField("where".to_owned()))
        );
        assert_eq!(
            HwShell::new().confirm("submit", &Map::new()),
            Err(HwError::NoChoicePresented)
        );
    }

    /// Oracle: the statement's free_text_entry exclusion — a form
    /// with a text field is unanswerable by this shell, in whole,
    /// never silently in part.
    #[test]
    fn confirm_refuses_a_form_requiring_free_text() {
        let mut shell = HwShell::new();
        let mut surface = choice_form_surface();
        surface["fields"] = json!([
            {"kind": "text", "id": "name", "label": "Name", "required": true, "max_length": 32},
            {"kind": "choice", "id": "when", "label": "When", "required": true, "options": ["now", "later"]},
        ]);
        let view = json!({
            "type": "view",
            "sequence": 5,
            "data": "Profile",
            "view_revision": 5,
            "surface": surface,
        });
        shell.apply_server_frame(&view).unwrap();
        assert_eq!(
            shell.confirm("submit", &selections(&[("when", json!("now"))])),
            Err(HwError::FreeTextExcluded {
                field: "name".to_owned()
            })
        );
    }

    #[test]
    fn status_reply_settles_the_choice() {
        let mut shell = shell_with_choice();
        let status = json!({
            "type": "form_result_status",
            "form_id": "hw-choice",
            "ok": true,
            "presentation_token": TOKEN,
        });
        assert_eq!(
            shell.apply_server_frame(&status).unwrap(),
            HwUpdate::FormResultStatus
        );
        assert_eq!(shell.last_status(), Some((true, None)));
        assert!(shell.choice().is_none());
    }

    #[test]
    fn unpresented_frame_kinds_are_ignored_not_fatal() {
        let mut shell = HwShell::new();
        let metrics = json!({"type": "metrics", "metrics": {}});
        assert_eq!(
            shell.apply_server_frame(&metrics).unwrap(),
            HwUpdate::Ignored
        );
        let refused = json!({"type": "error", "message": "admin denied"});
        assert_eq!(
            shell.apply_server_frame(&refused),
            Err(HwError::HostRefused("admin denied".to_owned()))
        );
    }
}
