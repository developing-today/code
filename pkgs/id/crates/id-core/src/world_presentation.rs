//! Validated, transport-neutral presentations emitted by world programs.
//!
//! Legacy UTF-8 text remains valid. A program opts into a portable screen by
//! returning the versioned `kind: screen` JSON document parsed here. Guests
//! never supply terminal escapes, HTML, or CSS: adapters render this value
//! using their own trusted presentation code.
//!
//! Text is the main presentation mode (`text/1`): a program may instead
//! return a versioned `kind: text` JSON document — UTF-8 lines with no
//! escapes, no cursor addressing, and no redraw. Text is append-only: a
//! presenter renders the lines in order and later documents append further
//! lines. Every [`WorldSurface`] variant must provide a deterministic,
//! escape-free text projection via [`WorldSurface::text_projection`]; the
//! match there is exhaustive, so adding a variant without a projection is a
//! compile-time error.
//!
//! A program may instead return a versioned `kind: scene` JSON document
//! (`scene/1`, #12657): a bounded list of named nodes with a closed shape
//! set, positions and sizes — the portable rich-presentation surface a
//! native shell renders in 3D. A shell without a scene renderer uses the
//! scene's text projection; a host that does not know the kind at all
//! drops the document to the legacy text path (see [`parse_surface`]).
//!
//! A program may instead return a versioned `kind: audio` JSON document
//! (`audio/1`, #12657): a bounded list of descriptive audio cues — named
//! references with a closed format set, durations and repeat flags. Cues
//! carry no embedded bytes, URLs or scripts: a presenter resolves each
//! cue id against the world's own assets. A shell without an audio
//! player uses the audio text projection; a host that does not know the
//! kind at all drops the document to the legacy text path.
//!
//! Semantic token adapter contract (xterm-256 foreground, bold; 1-bit):
//! | token | terminal | 1-bit | text |
//! |---|---|---|---|
//! | emphasis | default, bold | on | plain |
//! | muted | 8, normal | dither | plain |
//! | heading | default, bold | on | plain |
//! | code | 14, normal | on | plain |
//! | warning | 11, bold | on | plain |
//! | success | 10, normal | on | plain |
//! | link | 12, normal | dither | plain |
//! Adapters resolve every token; unknown future tokens fall back to plain,
//! while guest documents reject unknown tokens. Tokens override foreground
//! and bold of overlapping style spans and retain their background. Text
//! strips styling. Rendering and dithering are trusted adapter operations.

use std::fmt::Write as _;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Maximum dimensions accepted from one world screen.
pub const MAX_SCREEN_WIDTH: usize = 120;
/// Maximum height accepted from one world screen.
pub const MAX_SCREEN_HEIGHT: usize = 50;
/// Maximum row-major cells accepted from one world screen.
pub const MAX_SCREEN_CELLS: usize = MAX_SCREEN_WIDTH * MAX_SCREEN_HEIGHT;
/// Maximum coalesced style ranges accepted from one world screen.
pub const MAX_SCREEN_STYLE_SPANS: usize = 4096;
/// Default wrap width for a `text/1` document that names no width.
pub const DEFAULT_TEXT_WIDTH: usize = 80;
/// Maximum lines accepted from one world text document, after wrapping.
pub const MAX_TEXT_LINES: usize = 1000;
/// Maximum serialized form document size, matching the capability args budget.
pub const MAX_FORM_BYTES: usize = 8 * 1024;
/// Maximum fields, options, or actions in one form.
pub const MAX_FORM_ITEMS: usize = 64;
/// Maximum nodes accepted in one world scene document.
pub const MAX_SCENE_NODES: usize = 256;
/// Maximum serialized scene document size in bytes.
pub const MAX_SCENE_BYTES: usize = 64 * 1024;
/// Maximum absolute value of a scene position or size component, in world
/// units. Keeps scene geometry in a range every presenter can scale
/// without overflow.
pub const MAX_SCENE_EXTENT: f64 = 1_000_000.0;
/// Maximum cues accepted in one world audio document.
pub const MAX_AUDIO_CUES: usize = 64;
/// Maximum serialized audio document size in bytes.
pub const MAX_AUDIO_BYTES: usize = 64 * 1024;
/// Maximum duration of one audio cue, in milliseconds (one hour). Keeps
/// a cue's stated length in a range every presenter can schedule
/// without overflow.
pub const MAX_AUDIO_DURATION_MS: u32 = 3_600_000;

/// A validated view understood by more than one transport presenter.
///
/// `Eq` holds for every variant: scene geometry compares bit-exactly
/// (see [`SceneNode`]'s `PartialEq`), not by `f64` value equality.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorldSurface {
    /// A fixed-size terminal cell buffer, suitable for an SSH terminal or web DOM.
    Screen(ScreenSurface),
    /// Plain UTF-8 lines (`text/1`): the main mode for agents and scripts.
    Text(TextSurface),
    /// Structured input rendered by trusted adapters.
    Form(FormSurface),
    /// A 3D scene (`scene/1`): named nodes with shapes, positions and
    /// sizes, rendered by native shells; other shells use its text
    /// projection.
    Scene(SceneSurface),
    /// An audio cue list (`audio/1`): named sound references with
    /// formats, durations and repeat flags, played by shells with an
    /// audio player; other shells use its text projection.
    Audio(AudioSurface),
}

impl WorldSurface {
    /// The deterministic, escape-free, line-oriented text projection every
    /// surface must provide. This match is deliberately exhaustive with no
    /// wildcard arm: adding a `WorldSurface` variant fails compilation here
    /// until that variant supplies its own projection.
    #[must_use]
    pub fn text_projection(&self) -> String {
        match self {
            Self::Screen(screen) => screen.text_projection(),
            Self::Text(text) => text.render_text(),
            Self::Form(form) => form.text_projection(),
            Self::Scene(scene) => scene.text_projection(),
            Self::Audio(audio) => audio.text_projection(),
        }
    }
}

/// A versioned form document. Guest input is data only: there are no HTML,
/// CSS, or executable-validation fields.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FormSurface {
    pub version: u8,
    pub form_id: String,
    /// Opaque host-generated token identifying one presentation of this form.
    /// Programs copy it from the `form.present` result into their view; hosts
    /// require it on deferred submissions to reject stale widget callbacks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presentation_token: Option<String>,
    pub title: String,
    pub fields: Vec<FormField>,
    pub actions: Vec<FormAction>,
}

/// Closed set of supported form input fields.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FormField {
    /// Free text; requires a device manifest with `input.text_entry` other
    /// than `"none"`.
    Text {
        id: String,
        label: String,
        required: bool,
        max_length: u16,
    },
    /// A closed list of labels; requires at least one trusted selectable
    /// control (keys, encoder, or pointer).
    Choice {
        id: String,
        label: String,
        required: bool,
        options: Vec<String>,
    },
    /// An integer range; requires text entry or a trusted selectable control.
    Number {
        id: String,
        label: String,
        required: bool,
        min: i64,
        max: i64,
    },
    /// An ISO-8601 calendar date; requires text entry in this v1 surface.
    Date {
        id: String,
        label: String,
        required: bool,
        min: Option<String>,
        max: Option<String>,
    },
    /// A yes/no confirmation; requires a trusted selectable control.
    Confirm {
        id: String,
        label: String,
        required: bool,
    },
}

/// A submit or cancel action offered by a form.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FormAction {
    pub id: String,
    pub label: String,
    /// What submitting this action means. Missing values default to `submit`.
    #[serde(default)]
    pub kind: FormActionKind,
}

/// Whether an action submits field values or abandons the form.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FormActionKind {
    /// Validate all required fields and deliver their values.
    #[default]
    Submit,
    /// Abandon the form without requiring any field values.
    Cancel,
}

/// A deferred answer to a presented form. The host correlates this value by
/// `form_id`; it is delivered independently of the request that presented it.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FormResult {
    pub form_id: String,
    /// Host-generated token returned by the matching `form.present` request.
    pub presentation_token: String,
    pub action: String,
    pub values: serde_json::Map<String, Value>,
}

impl FormSurface {
    /// Validate the closed field/action model, identifiers, safe text and size.
    ///
    /// # Errors
    ///
    /// Rejects unsupported versions, empty or oversized field/action sets,
    /// duplicate ids, control characters, inverted ranges and oversize forms.
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1,
            "unsupported world form version {}",
            self.version
        );
        ensure!(
            safe_form_id(&self.form_id) && safe_form_text(&self.title),
            "invalid form id or title"
        );
        ensure!(
            !self.fields.is_empty() && self.fields.len() <= MAX_FORM_ITEMS,
            "invalid form field count"
        );
        ensure!(
            !self.actions.is_empty() && self.actions.len() <= MAX_FORM_ITEMS,
            "invalid form action count"
        );
        let mut ids = std::collections::HashSet::new();
        for field in &self.fields {
            let (id, label) = match field {
                FormField::Text {
                    id,
                    label,
                    max_length,
                    ..
                } => {
                    ensure!(*max_length > 0, "text field max_length must be positive");
                    (id, label)
                }
                FormField::Choice {
                    id, label, options, ..
                } => {
                    ensure!(
                        !options.is_empty() && options.len() <= MAX_FORM_ITEMS,
                        "invalid choice option count"
                    );
                    ensure!(
                        options.iter().all(|value| safe_form_text(value)),
                        "invalid choice option"
                    );
                    (id, label)
                }
                FormField::Number {
                    id,
                    label,
                    min,
                    max,
                    ..
                } => {
                    ensure!(min <= max, "number field min exceeds max");
                    (id, label)
                }
                FormField::Date {
                    id,
                    label,
                    min,
                    max,
                    ..
                } => {
                    ensure!(
                        min.as_deref().is_none_or(valid_form_date)
                            && max.as_deref().is_none_or(valid_form_date),
                        "invalid date bound"
                    );
                    ensure!(
                        min.as_deref()
                            .zip(max.as_deref())
                            .is_none_or(|(min, max)| min <= max),
                        "date field min exceeds max"
                    );
                    (id, label)
                }
                FormField::Confirm { id, label, .. } => (id, label),
            };
            ensure!(
                safe_form_id(id) && safe_form_text(label),
                "invalid form field id or label"
            );
            ensure!(ids.insert(id.as_str()), "duplicate form id");
        }
        for action in &self.actions {
            ensure!(
                safe_form_id(&action.id) && safe_form_text(&action.label),
                "invalid form action id or label"
            );
            ensure!(ids.insert(action.id.as_str()), "duplicate form id");
        }
        ensure!(
            self.presentation_token
                .as_deref()
                .is_none_or(safe_form_token),
            "invalid form presentation token"
        );
        ensure!(
            serde_json::to_vec(self)?.len() <= MAX_FORM_BYTES,
            "form exceeds {MAX_FORM_BYTES} bytes"
        );
        Ok(())
    }

    /// Check that one trusted presenter can collect all fields in this form.
    /// The caller passes that presenter's `DeviceManifest.input` object, not
    /// the hosting server's manifest: a headless server may serve browser or
    /// terminal presenters that have different input affordances.
    ///
    /// # Errors
    ///
    /// Returns an error when the input object cannot satisfy any field's
    /// minimum requirements.
    pub fn validate_device_input(&self, input: &Value) -> Result<()> {
        self.validate()?;
        let can_enter_text = input
            .get("text_entry")
            .and_then(Value::as_str)
            .is_some_and(|mode| mode != "none");
        let has_discrete_control = input
            .get("keys")
            .and_then(Value::as_u64)
            .is_some_and(|count| count > 0)
            || input
                .get("encoder")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            || input
                .get("pointer")
                .and_then(Value::as_bool)
                .unwrap_or(false);
        for field in &self.fields {
            match field {
                FormField::Text { .. } | FormField::Date { .. } => ensure!(
                    can_enter_text,
                    "text entry unavailable for form field {:?}",
                    field.id()
                ),
                FormField::Choice { .. } | FormField::Confirm { .. } => ensure!(
                    has_discrete_control,
                    "selectable input unavailable for form field {:?}",
                    field.id()
                ),
                FormField::Number { .. } => ensure!(
                    can_enter_text || has_discrete_control,
                    "numeric input unavailable for form field {:?}",
                    field.id()
                ),
            }
        }
        Ok(())
    }

    /// Deterministic escape-free projection: the title, numbered fields,
    /// then numbered actions.
    #[must_use]
    pub fn text_projection(&self) -> String {
        let mut lines = vec![self.title.clone()];
        for (index, field) in self.fields.iter().enumerate() {
            let (label, detail) = match field {
                FormField::Text {
                    label, max_length, ..
                } => (label, format!("text, max {max_length}")),
                FormField::Choice { label, options, .. } => {
                    (label, format!("choice: {}", options.join(", ")))
                }
                FormField::Number {
                    label, min, max, ..
                } => (label, format!("number: {min}..{max}")),
                FormField::Date {
                    label, min, max, ..
                } => (
                    label,
                    format!(
                        "date: {}..{}",
                        min.as_deref().unwrap_or("*"),
                        max.as_deref().unwrap_or("*")
                    ),
                ),
                FormField::Confirm { label, .. } => (label, "confirm".to_owned()),
            };
            lines.push(format!("{}. {} ({detail})", index + 1, label));
        }
        lines.extend(
            self.actions
                .iter()
                .enumerate()
                .map(|(index, action)| format!("{}. {}", index + 1, action.label)),
        );
        lines.join("\n")
    }

    /// Validate a submitted result against this form's closed field model.
    ///
    /// # Errors
    ///
    /// Rejects an unknown action or field, missing required value, or value
    /// whose JSON type or declared bounds do not match the field.
    pub fn validate_result(&self, result: &FormResult) -> Result<()> {
        self.validate()?;
        ensure!(
            safe_form_id(&result.form_id) && result.form_id == self.form_id,
            "form result id does not match the presented form"
        );
        ensure!(
            safe_form_token(&result.presentation_token),
            "invalid form result presentation token"
        );
        ensure!(
            self.presentation_token
                .as_deref()
                .is_none_or(|token| token == result.presentation_token),
            "form result is for a stale presentation"
        );
        let action = self
            .actions
            .iter()
            .find(|action| action.id == result.action)
            .context("form result action is not declared")?;
        if action.kind == FormActionKind::Cancel {
            ensure!(
                result.values.is_empty(),
                "cancel action must not include values"
            );
            return Ok(());
        }
        for key in result.values.keys() {
            ensure!(
                self.fields.iter().any(|field| field.id() == key),
                "form result contains an unknown field"
            );
        }
        for field in &self.fields {
            let Some(value) = result.values.get(field.id()) else {
                ensure!(!field.required(), "required form value is missing");
                continue;
            };
            ensure!(!value.is_null(), "form value must not be null");
            match field {
                FormField::Text { max_length, .. } => {
                    let text = value.as_str().context("text form value must be a string")?;
                    ensure!(
                        !field.required() || !text.trim().is_empty(),
                        "required text form value is empty"
                    );
                    ensure!(
                        text.graphemes(true).count() <= usize::from(*max_length),
                        "text form value exceeds max_length"
                    );
                }
                FormField::Choice { options, .. } => {
                    let choice = value
                        .as_str()
                        .context("choice form value must be a string")?;
                    ensure!(
                        options.iter().any(|option| option == choice),
                        "invalid choice value"
                    );
                }
                FormField::Number { min, max, .. } => {
                    let number = value
                        .as_i64()
                        .context("number form value must be an integer")?;
                    ensure!(
                        (*min..=*max).contains(&number),
                        "number form value is out of range"
                    );
                }
                FormField::Date { min, max, .. } => {
                    let date = value.as_str().context("date form value must be a string")?;
                    ensure!(valid_form_date(date), "date form value must be YYYY-MM-DD");
                    ensure!(
                        min.as_deref().is_none_or(|bound| date >= bound)
                            && max.as_deref().is_none_or(|bound| date <= bound),
                        "date form value is out of range"
                    );
                }
                FormField::Confirm { .. } => {
                    value
                        .as_bool()
                        .context("confirm form value must be a boolean")?;
                }
            }
        }
        Ok(())
    }
}

impl FormField {
    fn id(&self) -> &str {
        match self {
            Self::Text { id, .. }
            | Self::Choice { id, .. }
            | Self::Number { id, .. }
            | Self::Date { id, .. }
            | Self::Confirm { id, .. } => id,
        }
    }

    fn required(&self) -> bool {
        match self {
            Self::Text { required, .. }
            | Self::Choice { required, .. }
            | Self::Number { required, .. }
            | Self::Date { required, .. }
            | Self::Confirm { required, .. } => *required,
        }
    }
}

fn safe_form_text(value: &str) -> bool {
    !value.trim().is_empty() && !value.chars().any(char::is_control)
}

fn safe_form_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn safe_form_token(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_form_date(value: &str) -> bool {
    let Some((year, rest)) = value.split_once('-') else {
        return false;
    };
    let Some((month, day)) = rest.split_once('-') else {
        return false;
    };
    if year.len() != 4
        || month.len() != 2
        || day.len() != 2
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'-')
    {
        return false;
    }
    let (Ok(year), Ok(month), Ok(day)) =
        (year.parse::<u16>(), month.parse::<u8>(), day.parse::<u8>())
    else {
        return false;
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&day)
}

/// A validated `text/1` document: UTF-8 lines, wrapped to `width` columns
/// except for the one-grapheme overhang documented on [`TextSurface::with_width`]
/// (a single grapheme can be wider than `width` and is never split).
/// Serializes in exactly the guest document shape
/// (`kind: text, version, width, lines`), so a parsed text surface
/// round-trips through [`parse_surface`].
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TextSurface {
    /// Text format version. `text/1` defines version 1.
    pub version: u8,
    /// Wrap width in terminal columns: the requested width capped at
    /// [`MAX_SCREEN_WIDTH`], defaulting to [`DEFAULT_TEXT_WIDTH`].
    pub width: u16,
    /// The wrapped UTF-8 lines, in append order. No line contains a control
    /// character, so the lines are escape-free by construction.
    pub lines: Vec<String>,
}

impl TextSurface {
    /// Build a validated text surface at the default width.
    ///
    /// # Errors
    ///
    /// Rejects control characters (escapes included) in any line and
    /// documents that exceed [`MAX_TEXT_LINES`] after wrapping.
    pub fn new(lines: Vec<String>) -> Result<Self> {
        Self::with_width(DEFAULT_TEXT_WIDTH, lines)
    }

    /// Build a validated text surface, wrapping each line to `width`
    /// columns (capped at [`MAX_SCREEN_WIDTH`]) on grapheme boundaries.
    ///
    /// Wrapping keeps every stored line within `width` columns with one
    /// documented exception: a single grapheme that is itself wider than
    /// `width` is placed whole on its own line, because splitting it would
    /// corrupt the character. Such a line exceeds `width` by at most the
    /// width of that one grapheme (one column for a CJK or emoji grapheme at
    /// `width` 1). Presenters that must hard-fit a viewport of `width` 1-2
    /// should therefore clamp or blank the overhanging grapheme themselves
    /// ([`ScreenSurface`] is the cell-precise alternative), rather than
    /// assuming the stored lines are never wider than `width`.
    ///
    /// # Errors
    ///
    /// Rejects a zero width, control characters in any line, and documents
    /// that exceed [`MAX_TEXT_LINES`] after wrapping.
    pub fn with_width(width: usize, lines: Vec<String>) -> Result<Self> {
        ensure!(width >= 1, "world text width must be at least 1");
        let width = width.min(MAX_SCREEN_WIDTH);
        let mut wrapped = Vec::with_capacity(lines.len());
        for line in &lines {
            ensure!(
                !line.chars().any(char::is_control),
                "world text line contains a control character"
            );
            wrapped.extend(wrap_text_line(line, width));
        }
        ensure!(
            wrapped.len() <= MAX_TEXT_LINES,
            "world text has too many lines"
        );
        Ok(Self {
            version: 1,
            width: u16::try_from(width).context("text width is not representable")?,
            lines: wrapped,
        })
    }

    /// Render the document as plain text: its lines joined by `\n`, with no
    /// trailing newline and no escape sequences.
    #[must_use]
    pub fn render_text(&self) -> String {
        self.lines.join("\n")
    }
}

/// Wrap one logical line to `width` display columns. Breaks prefer the last
/// space that fits; a word longer than `width` is hard-wrapped on grapheme
/// boundaries, so no output line exceeds `width` columns except when a single
/// grapheme is itself wider than `width`: that grapheme is never split, so it
/// occupies a line of its own that is wider than `width` by the grapheme's
/// width (zero-width graphemes attach to the preceding column). A line that
/// already fits is returned unchanged, so conforming documents round-trip
/// exactly.
fn wrap_text_line(line: &str, width: usize) -> Vec<String> {
    if UnicodeWidthStr::width(line) <= width {
        return vec![line.to_owned()];
    }
    let mut out = Vec::new();
    let mut current = String::new();
    let mut current_width = 0_usize;
    // Byte index and width of the last space placed in `current`, so a soft
    // break can end the line there instead of splitting a word.
    let mut last_space: Option<(usize, usize)> = None;
    for grapheme in line.graphemes(true) {
        let grapheme_width = UnicodeWidthStr::width(grapheme);
        if grapheme == " " && current.is_empty() {
            continue; // a break consumes the space it breaks on
        }
        if current_width + grapheme_width > width && !current.is_empty() {
            if grapheme != " " {
                if let Some((space_at, width_at_space)) = last_space.take() {
                    let tail = current[space_at + 1..].to_owned();
                    let tail_width = current_width - width_at_space - 1;
                    current.truncate(space_at);
                    out.push(std::mem::take(&mut current));
                    current = tail;
                    current_width = tail_width;
                } else {
                    out.push(std::mem::take(&mut current));
                    current_width = 0;
                }
            } else {
                out.push(std::mem::take(&mut current));
                current_width = 0;
                last_space = None;
                continue;
            }
            last_space = None;
            if current_width + grapheme_width > width && !current.is_empty() {
                out.push(std::mem::take(&mut current));
                current_width = 0;
            }
        }
        if grapheme == " " {
            last_space = Some((current.len(), current_width));
        }
        current.push_str(grapheme);
        current_width += grapheme_width;
    }
    out.push(current);
    out
}

/// Normalized screen buffer. `cells` is row-major and has exactly width × height entries.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScreenSurface {
    /// Screen format version. P9.1 defines version 1.
    pub version: u8,
    /// Number of terminal columns.
    pub width: u16,
    /// Number of terminal rows.
    pub height: u16,
    /// Normalized row-major cells.
    pub cells: Vec<ScreenCell>,
}

impl ScreenSurface {
    /// Build a validated portable screen from a terminal buffer's row-major
    /// cells. Integrations should translate their native colors and modifiers
    /// to the DTO's xterm-256 indices and bold flag before calling this.
    ///
    /// # Errors
    ///
    /// Rejects empty or oversized dimensions, a mismatched cell count, unsafe
    /// glyphs, invalid wide-glyph continuations, and invalid style values.
    pub fn from_cells(width: usize, height: usize, cells: Vec<ScreenCell>) -> Result<Self> {
        ensure!(
            (1..=MAX_SCREEN_WIDTH).contains(&width),
            "screen width must be 1..={MAX_SCREEN_WIDTH}"
        );
        ensure!(
            (1..=MAX_SCREEN_HEIGHT).contains(&height),
            "screen height must be 1..={MAX_SCREEN_HEIGHT}"
        );
        ensure!(
            cells.len() == width * height,
            "screen cell count does not match its dimensions"
        );
        let screen = Self {
            version: 1,
            width: u16::try_from(width)?,
            height: u16::try_from(height)?,
            cells,
        };
        ensure!(
            screen.render_ansi().is_some(),
            "screen cells do not form a valid portable screen"
        );
        Ok(screen)
    }

    /// Crop a screen to a connection-local viewport without changing world state.
    /// Cropping through the second column of a wide glyph replaces that partial
    /// glyph with a blank cell so the returned grid remains valid.
    /// A zero viewport dimension is treated as 1, so the returned grid
    /// always satisfies the same 1..=MAX dimension invariant as
    /// `from_cells` (a detached or resizing terminal can report 0).
    pub fn fit(&self, width: usize, height: usize) -> Self {
        let width = width.min(usize::from(self.width)).max(1);
        let height = height.min(usize::from(self.height)).max(1);
        let mut cells = Vec::with_capacity(width * height);
        for y in 0..height {
            let start = y * usize::from(self.width);
            let mut row = self.cells[start..start + width].to_vec();
            if row.last().is_some_and(|cell| cell.columns == 2) {
                if let Some(last) = row.last_mut() {
                    *last = ScreenCell {
                        symbol: " ".to_owned(),
                        fg: 0,
                        bg: 0,
                        bold: false,
                        columns: 1,
                        token: None,
                    };
                }
            }
            cells.extend(row);
        }
        Self {
            version: self.version,
            width: u16::try_from(width).unwrap_or(self.width),
            height: u16::try_from(height).unwrap_or(self.height),
            cells,
        }
    }
}

/// A versioned `scene/1` document: a bounded, flat list of named nodes
/// with a closed shape set, positions and sizes. Guest input is data
/// only: there are no mesh, material, script or URL fields — a renderer
/// derives everything visual from the shape, position and size. The
/// document shape is the struct shape, so a parsed scene round-trips
/// through [`parse_surface`].
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SceneSurface {
    /// Scene format version. `scene/1` defines version 1.
    pub version: u8,
    /// Optional caption naming the scene; the text projection's first
    /// line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The scene's nodes, in document order. Ids are unique.
    pub nodes: Vec<SceneNode>,
}

/// One named object in a [`SceneSurface`].
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SceneNode {
    /// Stable identifier, unique within the scene.
    pub id: String,
    /// Human-readable name, used by the text projection and by
    /// renderers that label nodes.
    pub label: String,
    /// The node's geometry from the closed shape set.
    pub shape: SceneShape,
    /// Position in world units (x, y, z).
    pub position: [f64; 3],
    /// Extents in world units (x, y, z); every component is positive.
    /// For a sphere the components are its diameters; for a point, its
    /// nominal diameter.
    pub size: [f64; 3],
}

/// The closed set of node geometries in a `scene/1` document. Guest
/// documents reject unknown shapes.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SceneShape {
    /// An axis-aligned box.
    Box,
    /// A sphere.
    Sphere,
    /// A flat rectangular plane.
    Plane,
    /// A point marker.
    Point,
}

impl SceneShape {
    /// The document spelling of the shape, used by the text projection.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Box => "box",
            Self::Sphere => "sphere",
            Self::Plane => "plane",
            Self::Point => "point",
        }
    }
}

/// Bit-exact comparison of 3-component scene geometry.
fn scene_floats_bit_eq(left: &[f64; 3], right: &[f64; 3]) -> bool {
    left.iter()
        .zip(right.iter())
        .all(|(a, b)| a.to_bits() == b.to_bits())
}

impl PartialEq for SceneNode {
    /// Bit-exact equality: two nodes are equal only when every geometry
    /// component has identical bits. Unlike `f64` value equality, bit
    /// equality is a true equivalence (reflexive even for NaN, which
    /// [`SceneSurface::validate`] refuses before a scene is presented),
    /// so `Eq` is honest and `WorldSurface` keeps the `Eq` bound
    /// session-state structs rely on.
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.label == other.label
            && self.shape == other.shape
            && scene_floats_bit_eq(&self.position, &other.position)
            && scene_floats_bit_eq(&self.size, &other.size)
    }
}

impl Eq for SceneNode {}

impl PartialEq for SceneSurface {
    fn eq(&self, other: &Self) -> bool {
        self.version == other.version && self.title == other.title && self.nodes == other.nodes
    }
}

impl Eq for SceneSurface {}

impl SceneSurface {
    /// Validate the version, node set, identifiers, safe text, geometry
    /// bounds and total size. Hosts run this before any presenter sees
    /// the scene (via [`parse_surface`]).
    ///
    /// # Errors
    ///
    /// Rejects unsupported versions, unsafe titles, empty or oversized
    /// node sets, duplicate ids, unsafe labels, non-finite or
    /// out-of-extent geometry, non-positive sizes, and oversize
    /// documents.
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1,
            "unsupported world scene version {}",
            self.version
        );
        ensure!(
            self.title.as_deref().is_none_or(safe_form_text),
            "invalid scene title"
        );
        ensure!(
            !self.nodes.is_empty() && self.nodes.len() <= MAX_SCENE_NODES,
            "invalid scene node count"
        );
        let mut ids = std::collections::HashSet::new();
        for node in &self.nodes {
            ensure!(
                safe_form_id(&node.id) && safe_form_text(&node.label),
                "invalid scene node id or label"
            );
            ensure!(ids.insert(node.id.as_str()), "duplicate scene node id");
            ensure!(
                node.position
                    .iter()
                    .chain(node.size.iter())
                    .all(|component| component.is_finite()
                        && component.abs() <= MAX_SCENE_EXTENT),
                "scene geometry out of bounds"
            );
            ensure!(
                node.size.iter().all(|component| *component > 0.0),
                "scene node size must be positive"
            );
        }
        ensure!(
            serde_json::to_vec(self)?.len() <= MAX_SCENE_BYTES,
            "scene exceeds {MAX_SCENE_BYTES} bytes"
        );
        Ok(())
    }

    /// Deterministic escape-free projection: the title (or `Scene`),
    /// then one numbered line per node naming its label, shape,
    /// position and size. This is what a shell without a scene
    /// renderer shows, so it is designed here, with the kind, not
    /// discovered by the renderer slice.
    #[must_use]
    pub fn text_projection(&self) -> String {
        let mut lines = vec![self.title.clone().unwrap_or_else(|| "Scene".to_owned())];
        for (index, node) in self.nodes.iter().enumerate() {
            lines.push(format!(
                "{}. {} ({} at {}, {}, {}; size {}, {}, {})",
                index + 1,
                node.label,
                node.shape.as_str(),
                node.position[0],
                node.position[1],
                node.position[2],
                node.size[0],
                node.size[1],
                node.size[2],
            ));
        }
        lines.join("\n")
    }
}

/// A versioned `audio/1` document: a bounded list of descriptive audio
/// cues. Guest input is data only: a cue names a sound and describes it
/// (format, duration, repeat) — there are no embedded bytes, URL,
/// script or gain fields, so a presenter resolves each cue id against
/// the world's own assets and derives everything audible from the
/// description. The document shape is the struct shape, so a parsed
/// audio document round-trips through [`parse_surface`].
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AudioSurface {
    /// Audio format version. `audio/1` defines version 1.
    pub version: u8,
    /// Optional caption naming the cue list; the text projection's
    /// first line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The document's cues, in document order. Ids are unique.
    pub cues: Vec<AudioCue>,
}

/// One named sound in an [`AudioSurface`].
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AudioCue {
    /// Stable identifier, unique within the document. A presenter
    /// resolves it against the world's own assets.
    pub id: String,
    /// Human-readable name, used by the text projection and by
    /// presenters that label cues.
    pub label: String,
    /// The referenced sound's encoding, from the closed format set.
    pub format: AudioFormat,
    /// The sound's length in milliseconds.
    pub duration_ms: u32,
    /// Whether the sound repeats until the view changes.
    pub repeat: bool,
}

/// The closed set of sound encodings an `audio/1` cue may name. Guest
/// documents reject unknown formats.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioFormat {
    /// Waveform audio (PCM WAV).
    Wav,
    /// Ogg container (Vorbis/Opus).
    Ogg,
    /// MP3 audio.
    Mp3,
}

impl AudioFormat {
    /// The document spelling of the format, used by the text projection.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Wav => "wav",
            Self::Ogg => "ogg",
            Self::Mp3 => "mp3",
        }
    }
}

impl AudioSurface {
    /// Validate the version, cue set, identifiers, safe text, duration
    /// bounds and total size. Hosts run this before any presenter sees
    /// the document (via [`parse_surface`]).
    ///
    /// # Errors
    ///
    /// Rejects unsupported versions, unsafe titles, empty or oversized
    /// cue sets, duplicate ids, unsafe labels, out-of-bound durations,
    /// and oversize documents.
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1,
            "unsupported world audio version {}",
            self.version
        );
        ensure!(
            self.title.as_deref().is_none_or(safe_form_text),
            "invalid audio title"
        );
        ensure!(
            !self.cues.is_empty() && self.cues.len() <= MAX_AUDIO_CUES,
            "invalid audio cue count"
        );
        let mut ids = std::collections::HashSet::new();
        for cue in &self.cues {
            ensure!(
                safe_form_id(&cue.id) && safe_form_text(&cue.label),
                "invalid audio cue id or label"
            );
            ensure!(ids.insert(cue.id.as_str()), "duplicate audio cue id");
            ensure!(
                (1..=MAX_AUDIO_DURATION_MS).contains(&cue.duration_ms),
                "audio cue duration out of bounds"
            );
        }
        ensure!(
            serde_json::to_vec(self)?.len() <= MAX_AUDIO_BYTES,
            "audio exceeds {MAX_AUDIO_BYTES} bytes"
        );
        Ok(())
    }

    /// Deterministic escape-free projection: the title (or `Audio`),
    /// then one numbered line per cue naming its label, format and
    /// duration, with a `repeats` marker for looping cues. This is what
    /// a shell without an audio player shows, so it is designed here,
    /// with the kind, not discovered by the renderer slice.
    #[must_use]
    pub fn text_projection(&self) -> String {
        let mut lines = vec![self.title.clone().unwrap_or_else(|| "Audio".to_owned())];
        for (index, cue) in self.cues.iter().enumerate() {
            let repeat = if cue.repeat { "; repeats" } else { "" };
            lines.push(format!(
                "{}. {} ({}; {} ms{})",
                index + 1,
                cue.label,
                cue.format.as_str(),
                cue.duration_ms,
                repeat,
            ));
        }
        lines.join("\n")
    }
}

/// A cell-run delta between two consecutive screens (F12.11, first slice).
///
/// A patch carries only the cells that changed, grouped into maximal runs of
/// consecutive row-major indices. It is transport-neutral: the same patch can
/// ride any shell transport, and a client that applies it to the previous
/// screen holds exactly the next full frame. Binary encoding and compression
/// are later F12.11 slices; this type is the delta they will encode.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScreenPatch {
    /// Screen format version; matches the screens it patches.
    pub version: u8,
    /// Number of terminal columns; matches the screens it patches.
    pub width: u16,
    /// Number of terminal rows; matches the screens it patches.
    pub height: u16,
    /// The changed cell runs, in row-major order, non-overlapping.
    pub runs: Vec<CellRun>,
}

/// One maximal run of consecutive row-major cells that changed.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CellRun {
    /// Row-major index of the run's first cell.
    pub start: u32,
    /// The replacement cells, in row-major order.
    pub cells: Vec<ScreenCell>,
}

impl ScreenPatch {
    /// The patch that turns `prev` into `next`: the maximal runs of cells
    /// that differ. Returns `None` when the screens do not share a version
    /// and dimensions — the caller sends a full frame instead.
    #[must_use]
    pub fn diff(prev: &ScreenSurface, next: &ScreenSurface) -> Option<Self> {
        if prev.version != next.version || prev.width != next.width || prev.height != next.height {
            return None;
        }
        let mut runs = Vec::new();
        let mut current: Option<(usize, Vec<ScreenCell>)> = None;
        for (index, (before, after)) in prev.cells.iter().zip(&next.cells).enumerate() {
            if before == after {
                if let Some((start, cells)) = current.take() {
                    runs.push(CellRun {
                        start: u32::try_from(start).unwrap_or(u32::MAX),
                        cells,
                    });
                }
                continue;
            }
            match &mut current {
                Some((_, cells)) => cells.push(after.clone()),
                None => current = Some((index, vec![after.clone()])),
            }
        }
        if let Some((start, cells)) = current.take() {
            runs.push(CellRun {
                start: u32::try_from(start).unwrap_or(u32::MAX),
                cells,
            });
        }
        Some(Self {
            version: next.version,
            width: next.width,
            height: next.height,
            runs,
        })
    }

    /// The screen this patch produces from `prev`. Returns `None` when the
    /// patch does not belong to `prev` (version or dimensions differ) or a
    /// run is out of order, overlaps an earlier run, or runs past the last
    /// cell — the caller falls back to a full frame.
    #[must_use]
    pub fn apply(&self, prev: &ScreenSurface) -> Option<ScreenSurface> {
        if self.version != prev.version || self.width != prev.width || self.height != prev.height {
            return None;
        }
        let mut cells = prev.cells.clone();
        let mut covered_until = 0usize;
        for run in &self.runs {
            let start = usize::try_from(run.start).ok()?;
            let end = start.checked_add(run.cells.len())?;
            if start < covered_until || end > cells.len() {
                return None;
            }
            cells[start..end].clone_from_slice(&run.cells);
            covered_until = end;
        }
        ScreenSurface::from_cells(usize::from(prev.width), usize::from(prev.height), cells).ok()
    }

    /// Whether the patch changes nothing (the two screens are identical).
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }
}

/// Semantic intent retained for adapters, not guest-supplied markup.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StyleToken {
    Emphasis,
    Muted,
    Heading,
    Code,
    Warning,
    Success,
    Link,
}

impl StyleToken {
    /// Xterm-256 foreground (zero is default) and bold projection.
    #[must_use]
    pub const fn ansi_style(self) -> (u8, bool) {
        match self {
            Self::Emphasis | Self::Heading => (0, true),
            Self::Muted => (8, false),
            Self::Code => (14, false),
            Self::Warning => (11, true),
            Self::Success => (10, false),
            Self::Link => (12, false),
        }
    }
}

/// One normalized cell. `columns == 0` marks the continuation of a two-column glyph.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScreenCell {
    /// One safe Unicode grapheme, or empty for a wide glyph's continuation cell.
    pub symbol: String,
    /// Xterm-256 foreground index; zero selects the presenter default.
    pub fg: u8,
    /// Xterm-256 background index; zero selects the presenter default.
    pub bg: u8,
    /// Whether the cell is bold.
    pub bold: bool,
    /// Terminal columns occupied by this grapheme (zero for continuation cells).
    pub columns: u8,
    /// Semantic intent, if supplied by the guest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<StyleToken>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TextPayload {
    kind: String,
    version: u8,
    #[serde(default)]
    width: Option<usize>,
    lines: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScreenPayload {
    kind: String,
    version: u8,
    width: usize,
    height: usize,
    rows: Vec<String>,
    #[serde(default)]
    styles: Vec<Vec<StyleSpan>>,
    #[serde(default)]
    tokens: Vec<Vec<TokenSpan>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StyleSpan {
    x: usize,
    length: usize,
    fg: u8,
    bg: u8,
    bold: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TokenSpan {
    x: usize,
    length: usize,
    token: StyleToken,
}

/// Whether the document's top-level object repeats the `kind` key.
///
/// Only a top-level duplicate makes the surface dispatch ambiguous: a
/// `kind` repeated inside a nested object (for example a legacy
/// `{"payload":{"kind":"a","kind":"b"}}`) cannot affect dispatch, which
/// reads the top-level `kind` alone, so it must not turn a plain-text
/// document into an error. Nested values are skipped with
/// `IgnoredAny`, which does not itself reject duplicates.
fn top_level_kind_is_duplicated(data: &str) -> bool {
    use serde::Deserializer as _;
    use serde::de::{IgnoredAny, MapAccess, Visitor};
    use std::fmt;

    struct TopLevelKindVisitor;

    impl<'de> Visitor<'de> for TopLevelKindVisitor {
        type Value = bool;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a JSON object")
        }

        fn visit_map<A>(self, mut map: A) -> Result<bool, A::Error>
        where
            A: MapAccess<'de>,
        {
            // No early return on the duplicate: serde_json calls
            // end_map() after the visitor returns, so leaving the map
            // unconsumed turns the remaining entries into a trailing
            // error that the caller maps to a negative verdict (Codex
            // P2 on PR #8424). Record the flag and consume the rest.
            let mut seen_kind = false;
            let mut duplicated = false;
            while let Some(key) = map.next_key::<String>()? {
                let _ignored = map.next_value::<IgnoredAny>()?;
                if key == "kind" {
                    if seen_kind {
                        duplicated = true;
                    }
                    seen_kind = true;
                }
            }
            Ok(duplicated)
        }
    }

    serde_json::Deserializer::from_str(data)
        .deserialize_map(TopLevelKindVisitor)
        .unwrap_or(false)
}

/// Parse the opt-in screen contract; untagged program views remain legacy text.
///
/// Malformed values that identify themselves as a screen are rejected. Other
/// JSON and ordinary strings remain plain text for compatibility.
pub fn parse_surface(data: &str) -> Result<Option<WorldSurface>> {
    // A duplicate object key is a parse error for a surface document
    // (#6491, the #6520 rule): the checks below must see the document as
    // written, never a last-wins collapse of it — a forbidden version,
    // width, lines, or rows hidden behind a duplicate was accepted
    // before. Parse through the shared duplicate-rejecting helper;
    // anything that is not a surface document keeps the legacy
    // plain-text path (Ok(None)).
    let value: Value = match crate::json_unique::parse_json_without_duplicate_keys(data) {
        Ok(value) => value,
        Err(err) => {
            if err.to_string().contains("duplicate JSON object key") {
                // The collapsed parse still names the document's kind: a
                // duplicate anywhere in a text/screen document — or a
                // duplicated `kind` key itself, which makes the dispatch
                // ambiguous — rejects the document instead of letting
                // the surviving values pass the checks.
                let lenient: Option<Value> = serde_json::from_str(data).ok();
                let kind = lenient
                    .as_ref()
                    .and_then(|value| value.get("kind"))
                    .and_then(Value::as_str);
                if matches!(kind, Some("screen" | "text" | "form" | "scene" | "audio"))
                    || top_level_kind_is_duplicated(data)
                {
                    return Err(err).context("duplicate JSON object key in world presentation");
                }
            }
            return Ok(None);
        }
    };
    match value.get("kind").and_then(Value::as_str) {
        Some("screen") => {}
        Some("form") => {
            let mut value = value;
            if let Some(object) = value.as_object_mut() {
                object.remove("kind");
            }
            let form: FormSurface =
                serde_json::from_value(value).context("decode world form presentation")?;
            form.validate()?;
            return Ok(Some(WorldSurface::Form(form)));
        }
        Some("scene") => {
            let mut value = value;
            if let Some(object) = value.as_object_mut() {
                object.remove("kind");
            }
            let scene: SceneSurface =
                serde_json::from_value(value).context("decode world scene presentation")?;
            scene.validate()?;
            return Ok(Some(WorldSurface::Scene(scene)));
        }
        Some("audio") => {
            let mut value = value;
            if let Some(object) = value.as_object_mut() {
                object.remove("kind");
            }
            let audio: AudioSurface =
                serde_json::from_value(value).context("decode world audio presentation")?;
            audio.validate()?;
            return Ok(Some(WorldSurface::Audio(audio)));
        }
        Some("text") => {
            let payload: TextPayload =
                serde_json::from_value(value).context("decode world text presentation")?;
            ensure!(payload.kind == "text", "invalid world text kind");
            ensure!(
                payload.version == 1,
                "unsupported world text version {}",
                payload.version
            );
            let text = TextSurface::with_width(
                payload.width.unwrap_or(DEFAULT_TEXT_WIDTH),
                payload.lines,
            )?;
            return Ok(Some(WorldSurface::Text(text)));
        }
        _ => return Ok(None),
    }

    let payload: ScreenPayload =
        serde_json::from_value(value).context("decode world screen presentation")?;
    ensure!(payload.kind == "screen", "invalid world screen kind");
    ensure!(
        payload.version == 1,
        "unsupported world screen version {}",
        payload.version
    );
    ensure!(
        (1..=MAX_SCREEN_WIDTH).contains(&payload.width),
        "world screen width must be 1..={MAX_SCREEN_WIDTH}"
    );
    ensure!(
        (1..=MAX_SCREEN_HEIGHT).contains(&payload.height),
        "world screen height must be 1..={MAX_SCREEN_HEIGHT}"
    );
    let cell_count = payload
        .width
        .checked_mul(payload.height)
        .context("world screen dimensions overflow")?;
    ensure!(
        cell_count <= MAX_SCREEN_CELLS,
        "world screen has too many cells"
    );
    ensure!(
        payload.rows.len() == payload.height,
        "world screen row count does not match height"
    );
    ensure!(
        payload.styles.is_empty() || payload.styles.len() == payload.height,
        "world screen style rows do not match height"
    );
    ensure!(
        payload.tokens.is_empty() || payload.tokens.len() == payload.height,
        "world screen token rows do not match height"
    );
    ensure!(
        payload.styles.iter().map(Vec::len).sum::<usize>()
            + payload.tokens.iter().map(Vec::len).sum::<usize>()
            <= MAX_SCREEN_STYLE_SPANS,
        "world screen has too many style spans"
    );

    let default_cell = ScreenCell {
        symbol: " ".to_owned(),
        fg: 0,
        bg: 0,
        bold: false,
        columns: 1,
        token: None,
    };
    let mut cells = vec![default_cell; cell_count];

    for (row_index, row) in payload.rows.iter().enumerate() {
        let row_start = row_index * payload.width;
        let mut column = 0_usize;
        let mut boundaries = vec![false; payload.width + 1];
        boundaries[0] = true;
        for grapheme in row.graphemes(true) {
            ensure!(
                !grapheme.chars().any(char::is_control),
                "world screen row {row_index} contains a control character"
            );
            let columns = UnicodeWidthStr::width(grapheme);
            ensure!(
                (1..=2).contains(&columns),
                "world screen glyph must occupy one or two columns"
            );
            let end = column
                .checked_add(columns)
                .context("world screen row width overflow")?;
            ensure!(
                end <= payload.width,
                "world screen row {row_index} exceeds its width"
            );
            boundaries[column] = true;
            boundaries[end] = true;
            cells[row_start + column] = ScreenCell {
                symbol: grapheme.to_owned(),
                fg: 0,
                bg: 0,
                bold: false,
                columns: u8::try_from(columns)
                    .context("screen glyph width is not representable")?,
                token: None,
            };
            if columns == 2 {
                cells[row_start + column + 1] = ScreenCell {
                    symbol: String::new(),
                    fg: 0,
                    bg: 0,
                    bold: false,
                    columns: 0,
                    token: None,
                };
            }
            column = end;
        }
        ensure!(
            column == payload.width,
            "world screen row {row_index} does not fill its width"
        );

        if let Some(style_spans) = payload.styles.get(row_index) {
            let mut styled = vec![false; payload.width];
            let mut previous_end = 0;
            for span in style_spans {
                ensure!(span.length > 0, "world screen style span must not be empty");
                let end = span
                    .x
                    .checked_add(span.length)
                    .context("world screen style span overflows")?;
                ensure!(
                    end <= payload.width,
                    "world screen style span exceeds its row"
                );
                ensure!(
                    span.x >= previous_end,
                    "world screen style spans overlap or are unsorted"
                );
                ensure!(
                    boundaries[span.x] && boundaries[end],
                    "world screen style span splits a wide glyph"
                );
                for column in span.x..end {
                    ensure!(!styled[column], "world screen style spans overlap");
                    styled[column] = true;
                    let cell = &mut cells[row_start + column];
                    cell.fg = span.fg;
                    cell.bg = span.bg;
                    cell.bold = span.bold;
                }
                previous_end = end;
            }
        }
        if let Some(spans) = payload.tokens.get(row_index) {
            let mut previous_end = 0;
            for span in spans {
                ensure!(span.length > 0, "world screen token span must not be empty");
                let end = span
                    .x
                    .checked_add(span.length)
                    .context("world screen token span overflows")?;
                ensure!(
                    end <= payload.width,
                    "world screen token span exceeds its row"
                );
                ensure!(
                    span.x >= previous_end,
                    "world screen token spans overlap or are unsorted"
                );
                ensure!(
                    boundaries[span.x] && boundaries[end],
                    "world screen token span splits a wide glyph"
                );
                let (fg, bold) = span.token.ansi_style();
                for cell in &mut cells[row_start + span.x..row_start + end] {
                    cell.token = Some(span.token);
                    cell.fg = fg;
                    cell.bold = bold;
                }
                previous_end = end;
            }
        }
    }

    for row in 0..payload.height {
        let start = row * payload.width;
        for column in 0..payload.width {
            let cell = &cells[start + column];
            if cell.columns == 0 {
                ensure!(column > 0, "world screen continuation has no leading glyph");
                let leading = &cells[start + column - 1];
                ensure!(
                    leading.columns == 2
                        && leading.fg == cell.fg
                        && leading.bg == cell.bg
                        && leading.bold == cell.bold
                        && leading.token == cell.token,
                    "world screen continuation does not match its wide glyph"
                );
            }
        }
    }

    Ok(Some(WorldSurface::Screen(ScreenSurface {
        version: 1,
        width: u16::try_from(payload.width).context("screen width is not representable")?,
        height: u16::try_from(payload.height).context("screen height is not representable")?,
        cells,
    })))
}

impl ScreenSurface {
    /// The screen's text projection: one line per row, cell symbols in
    /// order (wide-glyph continuation cells contribute nothing), trailing
    /// blanks trimmed, rows joined by `\n`. Styles are dropped, so the
    /// projection is escape-free and line-oriented.
    #[must_use]
    pub fn text_projection(&self) -> String {
        let width = usize::from(self.width);
        if width == 0 {
            return String::new();
        }
        let mut rows = Vec::with_capacity(usize::from(self.height));
        for row in self.cells.chunks(width) {
            let mut line = String::new();
            for cell in row {
                if cell.columns != 0 {
                    line.push_str(&cell.symbol);
                }
            }
            rows.push(line.trim_end().to_owned());
        }
        rows.join("\n")
    }

    /// Render a safe terminal frame. Escape sequences are generated only from
    /// validated numeric style fields; guest-provided glyphs contain no controls.
    #[must_use]
    pub fn render_ansi(&self) -> Option<String> {
        if !self.is_valid() {
            return None;
        }
        let width = usize::from(self.width);
        let height = usize::from(self.height);
        let mut output = String::from("\x1b[H\x1b[2J");
        for row in 0..height {
            let mut previous_style: Option<(u8, u8, bool)> = None;
            for cell in &self.cells[row * width..(row + 1) * width] {
                if cell.columns == 0 {
                    continue;
                }
                let style = (cell.fg, cell.bg, cell.bold);
                if previous_style != Some(style) {
                    output.push_str(&style_sequence(style.0, style.1, style.2));
                    previous_style = Some(style);
                }
                output.push_str(&cell.symbol);
            }
            output.push_str("\x1b[0m");
            if row + 1 < height {
                output.push_str("\r\n");
            }
        }
        Some(output)
    }

    /// Render only rows that differ from the preceding screen. A changed row
    /// is rewritten from column one so wide-to-narrow glyph transitions cannot
    /// leave a stale continuation cell behind. `None` means the caller must
    /// use a full repaint (invalid screen or changed dimensions/version).
    #[must_use]
    pub fn render_ansi_diff(&self, previous: &Self) -> Option<String> {
        if !self.is_valid()
            || !previous.is_valid()
            || self.version != previous.version
            || self.width != previous.width
            || self.height != previous.height
        {
            return None;
        }

        let width = usize::from(self.width);
        let height = usize::from(self.height);
        let mut output = String::new();
        for row in 0..height {
            let start = row * width;
            let end = start + width;
            if self.cells[start..end] == previous.cells[start..end] {
                continue;
            }

            // Repaint a complete changed row. This also replaces both cells
            // when a glyph changes between narrow and wide forms.
            write!(&mut output, "\x1b[{};1H", row + 1).ok()?;
            let mut previous_style: Option<(u8, u8, bool)> = None;
            for cell in &self.cells[start..end] {
                if cell.columns == 0 {
                    continue;
                }
                let style = (cell.fg, cell.bg, cell.bold);
                if previous_style != Some(style) {
                    output.push_str(&style_sequence(style.0, style.1, style.2));
                    previous_style = Some(style);
                }
                output.push_str(&cell.symbol);
            }
            output.push_str("\x1b[0m");
        }
        if !output.is_empty() {
            // An absolute move clears the terminal's pending-wrap state after
            // writing a row through its last column, without changing cells.
            write!(&mut output, "\x1b[{};{}H", height, width).ok()?;
        }
        Some(output)
    }

    fn is_valid(&self) -> bool {
        let width = usize::from(self.width);
        let height = usize::from(self.height);
        if width == 0
            || width > MAX_SCREEN_WIDTH
            || height == 0
            || height > MAX_SCREEN_HEIGHT
            || width.checked_mul(height) != Some(self.cells.len())
        {
            return false;
        }
        for (index, cell) in self.cells.iter().enumerate() {
            let x = index % width;
            if cell.columns == 0 {
                if x == 0 {
                    return false;
                }
                let Some(previous) = index
                    .checked_sub(1)
                    .and_then(|previous| self.cells.get(previous))
                else {
                    return false;
                };
                if previous.columns != 2
                    || previous.fg != cell.fg
                    || previous.bg != cell.bg
                    || previous.bold != cell.bold
                    || previous.token != cell.token
                    || !cell.symbol.is_empty()
                {
                    return false;
                }
            } else if cell.columns == 1 || cell.columns == 2 {
                if cell.symbol.graphemes(true).count() != 1
                    || cell.symbol.chars().any(char::is_control)
                    || UnicodeWidthStr::width(cell.symbol.as_str()) != usize::from(cell.columns)
                    || (cell.columns == 2 && x + 1 == width)
                {
                    return false;
                }
                if cell.columns == 2 {
                    let Some(continuation) = index
                        .checked_add(1)
                        .and_then(|continuation| self.cells.get(continuation))
                    else {
                        return false;
                    };
                    if continuation.columns != 0
                        || continuation.fg != cell.fg
                        || continuation.bg != cell.bg
                        || continuation.bold != cell.bold
                        || continuation.token != cell.token
                        || !continuation.symbol.is_empty()
                    {
                        return false;
                    }
                }
            } else {
                return false;
            }
        }
        true
    }
}

fn style_sequence(fg: u8, bg: u8, bold: bool) -> String {
    let mut codes = vec!["0".to_owned()];
    if bold {
        codes.push("1".to_owned());
    }
    if fg != 0 {
        codes.push(format!("38;5;{fg}"));
    }
    if bg != 0 {
        codes.push(format!("48;5;{bg}"));
    }
    format!("\x1b[{}m", codes.join(";"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use serde_json::json;

    #[test]
    fn semantic_tokens_validate_and_project() {
        let mut doc = json!({"kind":"screen", "version":1, "width":1, "height":1,
            "rows":["x"], "tokens":[[{"x":0,"length":1,"token":"warning"}]]});
        let screen = parsed_screen(doc.clone());
        assert_eq!(screen.cells[0].token, Some(StyleToken::Warning));
        assert_eq!(
            screen.render_ansi().unwrap(),
            "\x1b[H\x1b[2J\x1b[0;1;38;5;11mx\x1b[0m"
        );
        assert_eq!(screen.text_projection(), "x");
        doc["tokens"][0][0]["token"] = json!("<script>");
        assert!(parse_surface(&doc.to_string()).is_err());
    }

    #[derive(Deserialize)]
    struct ScreenCellParityCorpus {
        cases: Vec<ScreenCellParityCase>,
    }

    #[derive(Deserialize)]
    struct ScreenCellParityCase {
        name: String,
        width: u16,
        height: u16,
        expected_valid: bool,
        cells: Vec<ScreenCell>,
    }

    fn screen(value: Value) -> Result<Option<WorldSurface>> {
        parse_surface(&value.to_string())
    }

    fn parsed_screen(value: Value) -> ScreenSurface {
        match screen(value).unwrap() {
            Some(WorldSurface::Screen(screen)) => screen,
            _ => panic!("expected a screen surface"),
        }
    }

    fn cell(symbol: &str) -> ScreenCell {
        ScreenCell {
            symbol: symbol.to_owned(),
            fg: 0,
            bg: 0,
            bold: false,
            columns: 1,
            token: None,
        }
    }

    fn grid(width: usize, height: usize, fill: &str) -> ScreenSurface {
        ScreenSurface::from_cells(width, height, vec![cell(fill); width * height]).unwrap()
    }

    fn codec_patch() -> ScreenPatch {
        ScreenPatch {
            version: 1,
            width: 2,
            height: 1,
            runs: Vec::new(),
        }
    }

    #[test]
    fn screen_codec_matches_versioned_messagepack_golden_vector() {
        let golden = [
            0x82, 0xa7, b'v', b'e', b'r', b's', b'i', b'o', b'n', 0x01, 0xa5, b'p', b'a', b't',
            b'c', b'h', 0x84, 0xa7, b'v', b'e', b'r', b's', b'i', b'o', b'n', 0x01, 0xa5, b'w',
            b'i', b'd', b't', b'h', 0x02, 0xa6, b'h', b'e', b'i', b'g', b'h', b't', 0x01, 0xa4,
            b'r', b'u', b'n', b's', 0x90,
        ];

        let encoded = crate::screen_codec::encode_patch(&codec_patch()).unwrap();
        assert_eq!(encoded, golden);
        assert_eq!(
            crate::screen_codec::decode_patch(&golden).unwrap(),
            codec_patch()
        );
    }

    #[test]
    fn screen_codec_rejects_unsupported_version_and_malformed_frames() {
        use crate::screen_codec::{ScreenCodecError, decode_patch};

        let mut unsupported = crate::screen_codec::encode_patch(&codec_patch()).unwrap();
        unsupported[9] = 2;
        assert!(matches!(
            decode_patch(&unsupported),
            Err(ScreenCodecError::UnsupportedVersion(2))
        ));
        assert!(matches!(
            decode_patch(&[0xc1]),
            Err(ScreenCodecError::Invalid(_))
        ));

        let mut trailing = crate::screen_codec::encode_patch(&codec_patch()).unwrap();
        trailing.push(0xc0);
        assert!(matches!(
            decode_patch(&trailing),
            Err(ScreenCodecError::TrailingBytes)
        ));
    }

    #[test]
    fn screen_codec_rejects_frames_over_one_mibibyte() {
        use crate::screen_codec::{MAX_SCREEN_FRAME_BYTES, ScreenCodecError, decode_patch};

        let oversized = vec![0; MAX_SCREEN_FRAME_BYTES + 1];
        assert!(matches!(
            decode_patch(&oversized),
            Err(ScreenCodecError::TooLarge)
        ));
    }

    #[test]
    fn test_rust_screen_presenter_matches_the_shared_web_parity_corpus() {
        let corpus: ScreenCellParityCorpus = serde_json::from_str(include_str!(
            "../../../web/src/test-fixtures/screen-cell-parity.json"
        ))
        .unwrap();

        let case_names: Vec<_> = corpus.cases.iter().map(|case| case.name.as_str()).collect();
        for required_case in [
            "CJK wide character",
            "astral-plane emoji",
            "ZWJ emoji sequence",
            "warning sign with emoji variation selector",
            "non-empty continuation cell",
            "wide character declared with the wrong width",
        ] {
            assert!(
                case_names.contains(&required_case),
                "missing {required_case:?}"
            );
        }
        for case in corpus.cases {
            let surface = ScreenSurface {
                version: 1,
                width: case.width,
                height: case.height,
                cells: case.cells,
            };
            assert_eq!(
                surface.render_ansi().is_some(),
                case.expected_valid,
                "Rust screen presenter verdict for {:?}",
                case.name
            );
        }
    }

    #[test]
    fn patch_parity_patched_client_equals_full_frame_client() {
        let prev = grid(4, 2, " ");
        let mut cells = prev.cells.clone();
        cells[0] = cell("a");
        cells[3] = ScreenCell { fg: 9, ..cell(" ") };
        cells[5] = cell("x");
        cells[6] = cell("y");
        let next = ScreenSurface::from_cells(4, 2, cells).unwrap();

        let patch = ScreenPatch::diff(&prev, &next).unwrap();
        assert_eq!(patch.runs.len(), 3, "single cell, style-only, and a pair");
        assert_eq!(patch.apply(&prev), Some(next));

        let identical = ScreenPatch::diff(&prev, &prev).unwrap();
        assert!(identical.is_empty());
        assert_eq!(identical.apply(&prev), Some(prev));
    }

    #[test]
    fn patch_for_a_small_change_is_under_a_tenth_of_the_full_frame() {
        let prev = grid(80, 24, " ");
        let mut cells = prev.cells.clone();
        cells[1000] = cell("x");
        let next = ScreenSurface::from_cells(80, 24, cells).unwrap();

        let patch = ScreenPatch::diff(&prev, &next).unwrap();
        let patch_bytes = serde_json::to_string(&patch).unwrap().len();
        let full_bytes = serde_json::to_string(&next).unwrap().len();
        assert!(
            patch_bytes * 10 < full_bytes,
            "patch {patch_bytes} bytes vs full frame {full_bytes} bytes"
        );
        assert_eq!(patch.apply(&prev), Some(next));
    }

    #[test]
    fn patch_refuses_mismatched_screens_and_bad_runs() {
        let small = grid(4, 2, " ");
        let other = grid(2, 2, " ");
        assert_eq!(ScreenPatch::diff(&small, &other), None);

        let patch = ScreenPatch::diff(&small, &small).unwrap();
        assert_eq!(patch.apply(&other), None);

        let bad = ScreenPatch {
            version: 1,
            width: 4,
            height: 2,
            runs: vec![CellRun {
                start: 7,
                cells: vec![cell("x"), cell("y")],
            }],
        };
        assert_eq!(bad.apply(&small), None, "run past the last cell");
    }

    #[test]
    fn patch_refuses_a_result_that_is_not_a_valid_screen() {
        let previous = grid(2, 1, " ");
        let invalid = ScreenPatch {
            version: 1,
            width: 2,
            height: 1,
            runs: vec![CellRun {
                start: 0,
                cells: vec![
                    ScreenCell {
                        symbol: "界".to_owned(),
                        columns: 2,
                        token: None,
                        ..cell("界")
                    },
                    cell("x"),
                ],
            }],
        };

        assert_eq!(invalid.apply(&previous), None);
    }

    #[test]
    fn patch_refuses_a_control_character_replacement() {
        let previous = grid(2, 1, " ");
        let invalid = ScreenPatch {
            version: 1,
            width: 2,
            height: 1,
            runs: vec![CellRun {
                start: 0,
                cells: vec![cell("\x1b")],
            }],
        };

        assert_eq!(invalid.apply(&previous), None);
    }

    #[test]
    fn patch_refuses_a_narrow_replacement_that_leaves_an_orphaned_continuation() {
        let previous = ScreenSurface::from_cells(
            2,
            1,
            vec![
                ScreenCell {
                    symbol: "界".to_owned(),
                    columns: 2,
                    token: None,
                    ..cell("界")
                },
                ScreenCell {
                    symbol: String::new(),
                    columns: 0,
                    token: None,
                    ..cell("")
                },
            ],
        )
        .unwrap();
        let invalid = ScreenPatch {
            version: 1,
            width: 2,
            height: 1,
            runs: vec![CellRun {
                start: 0,
                cells: vec![cell("x")],
            }],
        };

        assert_eq!(invalid.apply(&previous), None);
    }

    #[test]
    fn patch_refuses_a_cell_with_an_invalid_column_count() {
        let previous = grid(2, 1, " ");
        let invalid = ScreenPatch {
            version: 1,
            width: 2,
            height: 1,
            runs: vec![CellRun {
                start: 0,
                cells: vec![ScreenCell {
                    columns: 3,
                    token: None,
                    ..cell("x")
                }],
            }],
        };

        assert_eq!(invalid.apply(&previous), None);
    }

    #[test]
    fn patch_refuses_a_wide_glyph_in_the_last_column_of_a_row() {
        let previous = grid(2, 1, " ");
        let invalid = ScreenPatch {
            version: 1,
            width: 2,
            height: 1,
            runs: vec![CellRun {
                start: 1,
                cells: vec![ScreenCell {
                    symbol: "界".to_owned(),
                    columns: 2,
                    token: None,
                    ..cell("界")
                }],
            }],
        };

        assert_eq!(invalid.apply(&previous), None);
    }

    #[test]
    fn ansi_diff_repaints_changed_rows_with_terminal_parity_and_byte_savings() {
        let previous = grid(80, 24, " ");
        let mut cells = previous.cells.clone();
        cells[10 * 80 + 30] = cell("x");
        let next = ScreenSurface::from_cells(80, 24, cells).unwrap();

        let diff = next.render_ansi_diff(&previous).unwrap();
        let full = next.render_ansi().unwrap();
        assert!(diff.contains("\x1b[11;1H"));
        assert!(!diff.contains("\x1b[10;1H"));
        assert!(!diff.contains("\x1b[2J"));
        assert!(
            diff.len() * 10 < full.len(),
            "diff {diff:?} vs full repaint"
        );

        let mut patched = vt100::Parser::default();
        patched.process(previous.render_ansi().unwrap().as_bytes());
        patched.process(diff.as_bytes());
        let mut repainted = vt100::Parser::default();
        repainted.process(full.as_bytes());
        assert_eq!(patched.screen().contents(), repainted.screen().contents());
    }

    #[test]
    fn ansi_diff_handles_wide_glyph_transitions_and_style_changes() {
        let previous = parsed_screen(json!({
            "kind": "screen",
            "version": 1,
            "width": 6,
            "height": 1,
            "rows": ["界ab  "],
            "styles": [[{"x": 0, "length": 2, "fg": 1, "bg": 0, "bold": false}]]
        }));
        let next = parsed_screen(json!({
            "kind": "screen",
            "version": 1,
            "width": 6,
            "height": 1,
            "rows": ["x界b  "],
            "styles": [[{"x": 1, "length": 2, "fg": 2, "bg": 0, "bold": true}]]
        }));

        let diff = next.render_ansi_diff(&previous).unwrap();
        assert!(diff.contains("\x1b[0;1;38;5;2m"));
        let mut patched = vt100::Parser::new(
            std::num::NonZeroU16::MIN,
            std::num::NonZeroU16::new(6).unwrap(),
            0,
        );
        patched.process(previous.render_ansi().unwrap().as_bytes());
        patched.process(diff.as_bytes());
        let mut repainted = vt100::Parser::new(
            std::num::NonZeroU16::MIN,
            std::num::NonZeroU16::new(6).unwrap(),
            0,
        );
        repainted.process(next.render_ansi().unwrap().as_bytes());
        assert_eq!(patched.screen().contents(), repainted.screen().contents());
        assert_eq!(patched.screen().cell(0, 1), repainted.screen().cell(0, 1));
    }

    #[test]
    fn ansi_diff_is_empty_for_identical_screens_and_requests_full_fallback_on_resize() {
        let screen = grid(4, 2, " ");
        assert_eq!(screen.render_ansi_diff(&screen), Some(String::new()));
        assert_eq!(screen.render_ansi_diff(&grid(2, 2, " ")), None);
    }

    #[test]
    fn text_parses_and_round_trips() {
        let parsed =
            parse_surface(r#"{"kind":"text","version":1,"lines":["hello","wide 界 line"]}"#)
                .unwrap()
                .unwrap();
        let WorldSurface::Text(text) = &parsed else {
            panic!("expected a text surface");
        };
        assert_eq!(text.version, 1);
        assert_eq!(usize::from(text.width), DEFAULT_TEXT_WIDTH);
        assert_eq!(text.lines, ["hello", "wide 界 line"]);

        // The serialized form is the guest document shape, so it re-parses
        // to the identical surface.
        let serialized = serde_json::to_string(&parsed).unwrap();
        assert_eq!(parse_surface(&serialized).unwrap(), Some(parsed));
    }

    #[test]
    fn text_wrapping_is_width_aware_bounded_and_grapheme_safe() {
        let text = TextSurface::with_width(10, vec!["hello world again".to_owned()]).unwrap();
        assert_eq!(text.lines, ["hello", "world", "again"]);

        // A single over-long word hard-wraps on grapheme boundaries, and a
        // wide grapheme is never split across lines.
        let text = TextSurface::with_width(4, vec!["abcdefghij".to_owned()]).unwrap();
        assert_eq!(text.lines, ["abcd", "efgh", "ij"]);
        let text = TextSurface::with_width(3, vec!["ab界cd".to_owned()]).unwrap();
        assert!(
            text.lines
                .iter()
                .all(|line| UnicodeWidthStr::width(line.as_str()) <= 3)
        );
        assert_eq!(text.lines.concat(), "ab界cd");

        // The default is 80 and any requested width is capped like a screen.
        let text = TextSurface::with_width(10_000, vec!["x".repeat(200)]).unwrap();
        assert_eq!(usize::from(text.width), MAX_SCREEN_WIDTH);
        assert!(
            text.lines
                .iter()
                .all(|line| UnicodeWidthStr::width(line.as_str()) <= MAX_SCREEN_WIDTH)
        );
        assert!(TextSurface::with_width(0, vec!["x".to_owned()]).is_err());
    }

    #[test]
    fn text_wrapping_keeps_an_unsplittable_grapheme_whole() {
        // A grapheme wider than the requested width is never split: it takes
        // its own line, and that line is wider than the width by exactly the
        // grapheme's width (the documented one-grapheme exception).
        let text = TextSurface::with_width(1, vec!["界".to_owned()]).unwrap();
        assert_eq!(usize::from(text.width), 1);
        assert_eq!(text.lines, ["界"]);
        assert_eq!(UnicodeWidthStr::width(text.lines[0].as_str()), 2);

        // The same grapheme at the width it needs fits exactly, with no
        // exception: the line is within the width.
        let text = TextSurface::with_width(2, vec!["界".to_owned()]).unwrap();
        assert_eq!(text.lines, ["界"]);
        let widths: Vec<usize> = text
            .lines
            .iter()
            .map(|line| UnicodeWidthStr::width(line.as_str()))
            .collect();
        assert!(widths.iter().all(|width| *width <= 2));

        // Text around the wide grapheme still wraps normally, and only the
        // lone over-wide grapheme may exceed the width.
        let text = TextSurface::with_width(1, vec!["a界b".to_owned()]).unwrap();
        assert_eq!(text.lines, ["a", "界", "b"]);
        let widths: Vec<usize> = text
            .lines
            .iter()
            .map(|line| UnicodeWidthStr::width(line.as_str()))
            .collect();
        assert_eq!(widths, [1, 2, 1]);
    }

    #[test]
    fn text_rejects_escapes_controls_and_bad_versions() {
        let escape = json!({"kind": "text", "version": 1, "lines": ["\u{001b}[2J"]});
        assert!(parse_surface(&escape.to_string()).is_err());
        let tab = json!({"kind": "text", "version": 1, "lines": ["a\tb"]});
        assert!(parse_surface(&tab.to_string()).is_err());
        let unsupported = json!({"kind": "text", "version": 2, "lines": ["x"]});
        assert!(parse_surface(&unsupported.to_string()).is_err());
    }

    #[test]
    fn form_surface_parses_projects_and_rejects_unsafe_or_oversize_documents() {
        let document = serde_json::json!({
            "kind": "form", "version": 1, "form_id": "join-1", "title": "Join",
            "fields": [
                {"kind": "text", "id": "name", "label": "Name", "required": true, "max_length": 40},
                {"kind": "choice", "id": "role", "label": "Role", "required": true, "options": ["Builder", "Guide"]},
                {"kind": "number", "id": "age", "label": "Age", "required": true, "min": 0, "max": 120},
                {"kind": "date", "id": "start", "label": "Start date", "required": true, "min": "2026-01-01", "max": "2027-12-31"},
                {"kind": "confirm", "id": "accept", "label": "Accept?", "required": true}
            ],
            "actions": [{"id": "submit", "label": "Continue"}]
        });
        let parsed = parse_surface(&document.to_string()).unwrap().unwrap();
        let WorldSurface::Form(form) = parsed else {
            panic!("expected form surface")
        };
        assert_eq!(
            form.text_projection(),
            "Join\n1. Name (text, max 40)\n2. Role (choice: Builder, Guide)\n3. Age (number: 0..120)\n4. Start date (date: 2026-01-01..2027-12-31)\n5. Accept? (confirm)\n1. Continue"
        );
        let valid_result = FormResult {
            form_id: "join-1".into(),
            presentation_token: "0123456789abcdef0123456789abcdef".into(),
            action: "submit".into(),
            values: serde_json::json!({
                "name": "Ada", "role": "Guide", "age": 42,
                "start": "2026-05-12", "accept": true
            })
            .as_object()
            .unwrap()
            .clone(),
        };
        assert!(form.validate_result(&valid_result).is_ok());
        let invalid_result = FormResult {
            values: serde_json::json!({"name": "Ada", "role": "Admin", "age": 42, "start": "2026-05-12", "accept": true})
                .as_object()
                .unwrap()
                .clone(),
            ..valid_result.clone()
        };
        assert!(form.validate_result(&invalid_result).is_err());
        let with_html = serde_json::json!({
            "kind": "form", "version": 1, "form_id": "join-2", "title": "Join",
            "html": "<b>unsafe</b>", "css": ".unsafe{}", "validation": "eval('x')",
            "fields": [{"kind": "confirm", "id": "yes", "label": "Proceed?", "required": true}],
            "actions": [{"id": "ok", "label": "OK"}]
        });
        assert!(parse_surface(&with_html.to_string()).is_err());
        let escape = serde_json::json!({
            "kind": "form", "version": 1, "form_id": "join-3", "title": "Join\u{001b}[2J",
            "fields": [{"kind": "confirm", "id": "yes", "label": "Proceed?", "required": true}],
            "actions": [{"id": "ok", "label": "OK"}]
        });
        assert!(parse_surface(&escape.to_string()).is_err());
        let oversized = FormSurface {
            version: 1,
            form_id: "join-4".into(),
            presentation_token: None,
            title: "Join".into(),
            fields: vec![FormField::Text {
                id: "name".into(),
                label: "x".repeat(MAX_FORM_BYTES),
                required: false,
                max_length: 1,
            }],
            actions: vec![FormAction {
                id: "cancel".into(),
                label: "Cancel".into(),
                kind: FormActionKind::Cancel,
            }],
        };
        assert!(oversized.validate().is_err());
    }

    #[test]
    fn scene_surface_parses_projects_and_round_trips() {
        // Oracle: #12657 slice 1a — a versioned scene document is accepted
        // by host validation, projects deterministically to text (the
        // old-shell fallback), and round-trips through serialization.
        let document = serde_json::json!({
            "kind": "scene", "version": 1, "title": "Harbor",
            "nodes": [
                {"id": "boat", "label": "Boat", "shape": "box",
                 "position": [0, 0.5, -2], "size": [2, 1, 1]},
                {"id": "sun", "label": "Sun", "shape": "sphere",
                 "position": [10, 8, -20], "size": [3, 3, 3]}
            ]
        });
        let parsed = parse_surface(&document.to_string()).unwrap().unwrap();
        assert_eq!(
            parsed.text_projection(),
            "Harbor\n1. Boat (box at 0, 0.5, -2; size 2, 1, 1)\n2. Sun (sphere at 10, 8, -20; size 3, 3, 3)"
        );
        let serialized = serde_json::to_string(&parsed).unwrap();
        assert_eq!(parse_surface(&serialized).unwrap(), Some(parsed));

        // Without a title the projection still names what the view is.
        let untitled = parse_surface(
            r#"{"kind":"scene","version":1,"nodes":[{"id":"a","label":"A","shape":"point","position":[1,2,3],"size":[1,1,1]}]}"#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            untitled.text_projection(),
            "Scene\n1. A (point at 1, 2, 3; size 1, 1, 1)"
        );
    }

    #[test]
    fn scene_surface_rejects_unsafe_or_oversize_documents() {
        // Oracle: #12657 criterion — malformed or oversized rich surfaces
        // are refused by host validation (an Err from parse_surface, which
        // the host turns into an error frame), never silently dropped to
        // the legacy text path and never accepted.
        let node = |id: &str| {
            serde_json::json!({"id": id, "label": "A", "shape": "box",
                "position": [0, 0, 0], "size": [1, 1, 1]})
        };
        let scene = |nodes: Value| {
            serde_json::json!({"kind": "scene", "version": 1, "nodes": nodes}).to_string()
        };
        // Bad versions.
        assert!(parse_surface(
            &serde_json::json!({"kind": "scene", "version": 2, "nodes": [node("a")]})
                .to_string()
        )
        .is_err());
        assert!(parse_surface(
            &serde_json::json!({"kind": "scene", "version": 0, "nodes": [node("a")]})
                .to_string()
        )
        .is_err());
        // Unknown fields, top level and per node (deny_unknown_fields).
        assert!(parse_surface(
            &serde_json::json!({"kind": "scene", "version": 1, "html": "<b>x</b>", "nodes": [node("a")]})
                .to_string()
        )
        .is_err());
        assert!(parse_surface(&scene(serde_json::json!([
            {"id": "a", "label": "A", "shape": "box", "position": [0, 0, 0],
             "size": [1, 1, 1], "onclick": "run()"}
        ])))
        .is_err());
        // Unknown shape: the shape set is closed.
        assert!(parse_surface(&scene(serde_json::json!([
            {"id": "a", "label": "A", "shape": "mesh", "position": [0, 0, 0], "size": [1, 1, 1]}
        ])))
        .is_err());
        // Empty and duplicate-id node sets.
        assert!(parse_surface(&scene(serde_json::json!([]))).is_err());
        assert!(parse_surface(&scene(serde_json::json!([node("a"), node("a")]))).is_err());
        // Too many nodes: the bound is 256 (MAX_SCENE_NODES).
        let many: Vec<Value> = (0..257)
            .map(|index| node(&format!("n{index}")))
            .collect();
        assert!(parse_surface(&scene(Value::Array(many))).is_err());
        // Unsafe label text (a JSON-escaped ESC).
        assert!(parse_surface(&scene(serde_json::json!([
            {"id": "a", "label": "A\u{001b}[2J", "shape": "box", "position": [0, 0, 0], "size": [1, 1, 1]}
        ])))
        .is_err());
        // Geometry bounds: size must be positive, position and size
        // components stay within 1_000_000 (MAX_SCENE_EXTENT).
        assert!(parse_surface(&scene(serde_json::json!([
            {"id": "a", "label": "A", "shape": "box", "position": [0, 0, 0], "size": [0, 1, 1]}
        ])))
        .is_err());
        assert!(parse_surface(&scene(serde_json::json!([
            {"id": "a", "label": "A", "shape": "box", "position": [0, 0, 0], "size": [-1, 1, 1]}
        ])))
        .is_err());
        assert!(parse_surface(&scene(serde_json::json!([
            {"id": "a", "label": "A", "shape": "box", "position": [1000001, 0, 0], "size": [1, 1, 1]}
        ])))
        .is_err());
        // Oversize document: the serialized cap is 65536 bytes
        // (MAX_SCENE_BYTES); a single huge label trips it.
        assert!(parse_surface(&scene(serde_json::json!([
            {"id": "a", "label": "x".repeat(70_000), "shape": "box", "position": [0, 0, 0], "size": [1, 1, 1]}
        ])))
        .is_err());
    }

    #[test]
    fn scene_validate_rejects_non_finite_geometry() {
        // A scene built in Rust (not parsed from JSON, where non-finite
        // numbers cannot be represented) must still refuse NaN and
        // infinite geometry: presenters scale positions and sizes
        // without re-checking them.
        let node = |position: [f64; 3]| SceneNode {
            id: "a".into(),
            label: "A".into(),
            shape: SceneShape::Box,
            position,
            size: [1.0, 1.0, 1.0],
        };
        let scene = |position: [f64; 3]| SceneSurface {
            version: 1,
            title: None,
            nodes: vec![node(position)],
        };
        assert!(scene([0.0, 0.0, 0.0]).validate().is_ok());
        assert!(scene([f64::NAN, 0.0, 0.0]).validate().is_err());
        assert!(scene([0.0, f64::INFINITY, 0.0]).validate().is_err());
        assert!(scene([0.0, 0.0, f64::NEG_INFINITY]).validate().is_err());
    }

    #[test]
    fn scene_equality_is_bit_exact() {
        // WorldSurface keeps its Eq derive (session-state structs rely
        // on it), so scene equality is bit-exact: 0.0 and -0.0 differ.
        let scene = |x: f64| SceneSurface {
            version: 1,
            title: None,
            nodes: vec![SceneNode {
                id: "a".into(),
                label: "A".into(),
                shape: SceneShape::Point,
                position: [x, 0.0, 0.0],
                size: [1.0, 1.0, 1.0],
            }],
        };
        assert_eq!(scene(0.0), scene(0.0));
        assert_ne!(scene(0.0), scene(-0.0));
    }

    #[test]
    fn audio_surface_parses_projects_and_round_trips() {
        // Oracle: #12657 slice 1b — a versioned audio document is accepted
        // by host validation, projects deterministically to text (the
        // old-shell fallback), and round-trips through serialization.
        let document = serde_json::json!({
            "kind": "audio", "version": 1, "title": "Harbor Sounds",
            "cues": [
                {"id": "rain", "label": "Rain", "format": "ogg",
                 "duration_ms": 90000, "repeat": true},
                {"id": "bell", "label": "Bell", "format": "wav",
                 "duration_ms": 2500, "repeat": false}
            ]
        });
        let parsed = parse_surface(&document.to_string()).unwrap().unwrap();
        assert_eq!(
            parsed.text_projection(),
            "Harbor Sounds\n1. Rain (ogg; 90000 ms; repeats)\n2. Bell (wav; 2500 ms)"
        );
        let serialized = serde_json::to_string(&parsed).unwrap();
        assert_eq!(parse_surface(&serialized).unwrap(), Some(parsed));

        // Without a title the projection still names what the view is.
        let untitled = parse_surface(
            r#"{"kind":"audio","version":1,"cues":[{"id":"a","label":"A","format":"mp3","duration_ms":1000,"repeat":false}]}"#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(untitled.text_projection(), "Audio\n1. A (mp3; 1000 ms)");
    }

    #[test]
    fn audio_surface_rejects_unsafe_or_oversize_documents() {
        // Oracle: #12657 criterion — malformed or oversized rich surfaces
        // are refused by host validation (an Err from parse_surface, which
        // the host turns into an error frame), never silently dropped to
        // the legacy text path and never accepted.
        let cue = |id: &str| {
            serde_json::json!({"id": id, "label": "A", "format": "ogg",
                "duration_ms": 1000, "repeat": false})
        };
        let audio = |cues: Value| {
            serde_json::json!({"kind": "audio", "version": 1, "cues": cues}).to_string()
        };
        // Bad versions.
        assert!(parse_surface(
            &serde_json::json!({"kind": "audio", "version": 2, "cues": [cue("a")]})
                .to_string()
        )
        .is_err());
        assert!(parse_surface(
            &serde_json::json!({"kind": "audio", "version": 0, "cues": [cue("a")]})
                .to_string()
        )
        .is_err());
        // Unknown fields, top level and per cue (deny_unknown_fields) —
        // in particular a cue carries no embedded bytes or URL.
        assert!(parse_surface(
            &serde_json::json!({"kind": "audio", "version": 1, "src": "x.ogg", "cues": [cue("a")]})
                .to_string()
        )
        .is_err());
        assert!(parse_surface(&audio(serde_json::json!([
            {"id": "a", "label": "A", "format": "ogg", "duration_ms": 1000,
             "repeat": false, "data": "AAAB"}
        ])))
        .is_err());
        // Unknown format: the format set is closed.
        assert!(parse_surface(&audio(serde_json::json!([
            {"id": "a", "label": "A", "format": "flac", "duration_ms": 1000, "repeat": false}
        ])))
        .is_err());
        // Empty and duplicate-id cue sets.
        assert!(parse_surface(&audio(serde_json::json!([]))).is_err());
        assert!(parse_surface(&audio(serde_json::json!([cue("a"), cue("a")]))).is_err());
        // Too many cues: the bound is 64 (MAX_AUDIO_CUES).
        let many: Vec<Value> = (0..65).map(|index| cue(&format!("c{index}"))).collect();
        assert!(parse_surface(&audio(Value::Array(many))).is_err());
        // Unsafe label text (a JSON-escaped ESC).
        assert!(parse_surface(&audio(serde_json::json!([
            {"id": "a", "label": "A\u{001b}[2J", "format": "ogg", "duration_ms": 1000, "repeat": false}
        ])))
        .is_err());
        // Duration bounds: a cue lasts 1..=3_600_000 ms
        // (MAX_AUDIO_DURATION_MS, one hour).
        assert!(parse_surface(&audio(serde_json::json!([
            {"id": "a", "label": "A", "format": "ogg", "duration_ms": 0, "repeat": false}
        ])))
        .is_err());
        assert!(parse_surface(&audio(serde_json::json!([
            {"id": "a", "label": "A", "format": "ogg", "duration_ms": 3600001, "repeat": false}
        ])))
        .is_err());
        // Oversize document: the serialized cap is 65536 bytes
        // (MAX_AUDIO_BYTES); a single huge label trips it.
        assert!(parse_surface(&audio(serde_json::json!([
            {"id": "a", "label": "x".repeat(70_000), "format": "ogg", "duration_ms": 1000, "repeat": false}
        ])))
        .is_err());
    }

    #[test]
    fn unknown_rich_surface_kinds_stay_on_the_legacy_path() {
        // Forward-compat seam (#12657 triage): a rich kind this host does
        // not know is not an error — the document drops to the legacy
        // plain-text path, so a new-kind document is safe on an old host.
        // (The widget kind lands in slice 1c; today it is unknown here,
        // as is any kind no slice has named.)
        assert_eq!(
            parse_surface(r#"{"kind":"widget","version":1}"#).unwrap(),
            None
        );
        assert_eq!(
            parse_surface(r#"{"kind":"hologram","version":1}"#).unwrap(),
            None
        );
    }

    #[test]
    fn form_date_bounds_are_canonical_and_cancel_skips_required_fields() {
        let invalid_bound = serde_json::json!({
            "kind": "form", "version": 1, "form_id": "date-1", "title": "Date",
            "fields": [{"kind": "date", "id": "date", "label": "Date", "required": true, "min": "zzz", "max": null}],
            "actions": [{"id": "submit", "label": "Submit"}]
        });
        assert!(parse_surface(&invalid_bound.to_string()).is_err());
        let inverted_range = serde_json::json!({
            "kind": "form", "version": 1, "form_id": "date-2", "title": "Date",
            "fields": [{"kind": "date", "id": "date", "label": "Date", "required": true, "min": "2027-01-01", "max": "2026-12-31"}],
            "actions": [{"id": "submit", "label": "Submit"}]
        });
        assert!(parse_surface(&inverted_range.to_string()).is_err());

        let form = FormSurface {
            version: 1,
            form_id: "cancel-form".into(),
            presentation_token: None,
            title: "Required form".into(),
            fields: vec![FormField::Text {
                id: "name".into(),
                label: "Name".into(),
                required: true,
                max_length: 30,
            }],
            actions: vec![
                FormAction {
                    id: "submit".into(),
                    label: "Submit".into(),
                    kind: FormActionKind::Submit,
                },
                FormAction {
                    id: "cancel".into(),
                    label: "Cancel".into(),
                    kind: FormActionKind::Cancel,
                },
            ],
        };
        let cancel = FormResult {
            form_id: "cancel-form".into(),
            presentation_token: "fedcba9876543210fedcba9876543210".into(),
            action: "cancel".into(),
            values: serde_json::Map::new(),
        };
        assert!(form.validate_result(&cancel).is_ok());
        let malformed_date = FormSurface {
            fields: vec![FormField::Date {
                id: "date".into(),
                label: "Date".into(),
                required: true,
                min: None,
                max: None,
            }],
            ..form
        };
        let invalid_date = FormResult {
            form_id: "cancel-form".into(),
            presentation_token: cancel.presentation_token,
            action: "submit".into(),
            values: serde_json::json!({"date": "2026-+1-01"})
                .as_object()
                .unwrap()
                .clone(),
        };
        assert!(malformed_date.validate_result(&invalid_date).is_err());
    }

    #[test]
    fn form_device_gating_uses_the_presenting_device_input_manifest() {
        let encoder_form = FormSurface {
            version: 1,
            form_id: "encoder-form".into(),
            presentation_token: None,
            title: "Choose".into(),
            fields: vec![FormField::Choice {
                id: "mode".into(),
                label: "Mode".into(),
                required: true,
                options: vec!["quiet".into(), "loud".into()],
            }],
            actions: vec![FormAction {
                id: "submit".into(),
                label: "Submit".into(),
                kind: FormActionKind::Submit,
            }],
        };
        let headless_input = serde_json::json!({
            "keys": 0, "encoder": false, "pointer": false, "text_entry": "none"
        });
        assert!(encoder_form.validate_device_input(&headless_input).is_err());
        let rotary_input = serde_json::json!({
            "keys": 0, "encoder": true, "pointer": false, "text_entry": "none"
        });
        assert!(encoder_form.validate_device_input(&rotary_input).is_ok());

        let text_form = FormSurface {
            fields: vec![FormField::Text {
                id: "name".into(),
                label: "Name".into(),
                required: true,
                max_length: 24,
            }],
            ..encoder_form
        };
        assert!(text_form.validate_device_input(&rotary_input).is_err());
        assert!(
            text_form
                .validate_device_input(&serde_json::json!({"text_entry": "keyboard"}))
                .is_ok()
        );
    }

    #[test]
    fn screen_and_text_projections_are_line_oriented_and_escape_free() {
        // One program, two documents: its screen and its text.
        let screen = parse_surface(
            r#"{"kind":"screen","version":1,"width":4,"height":2,"rows":["界ab","wxyz"]}"#,
        )
        .unwrap()
        .unwrap();
        let text = parse_surface(r#"{"kind":"text","version":1,"lines":["界ab","wxyz"]}"#)
            .unwrap()
            .unwrap();

        assert_eq!(screen.text_projection(), "界ab\nwxyz");
        assert_eq!(text.text_projection(), "界ab\nwxyz");
        for surface in [screen, text] {
            let projection = surface.text_projection();
            assert!(!projection.contains('\u{001b}'));
            assert!(!projection.chars().any(|c| c.is_control() && c != '\n'));
            assert_eq!(projection.lines().count(), 2);
        }
    }

    #[test]
    fn plain_text_stays_on_the_legacy_path() {
        assert_eq!(parse_surface("hello\nworld").unwrap(), None);
        assert_eq!(
            parse_surface(r#"{"kind":"other","version":1}"#).unwrap(),
            None
        );
    }

    #[test]
    fn duplicate_keys_in_surface_documents_are_rejected() {
        // #6491: raw string literals — serde_json::json! values cannot
        // represent duplicate keys, so the in-tree tests could not express
        // this defect. Every document below was accepted (Ok(Some(..)))
        // before the fix: the forbidden value hides behind a benign
        // duplicate and only the survivor was ever checked.
        for doc in [
            // Duplicate version, text: unsupported version first.
            r#"{"kind":"text","version":99,"version":1,"lines":["hi"]}"#,
            // Duplicate version, screen.
            r#"{"kind":"screen","version":99,"version":1,"width":2,"height":1,"rows":["hi"]}"#,
            // Duplicate width: a zero width hidden behind a benign one.
            r#"{"kind":"text","version":1,"width":0,"width":80,"lines":["hi"]}"#,
            // Duplicate lines: a control character (JSON-escaped ESC)
            // hidden behind clean lines.
            r#"{"kind":"text","version":1,"lines":["\u001b[31mred"],"lines":["clean"]}"#,
            // Duplicate rows: an over-wide row hidden behind a fitting one.
            r#"{"kind":"screen","version":1,"width":2,"height":1,"rows":["toolong"],"rows":["hi"]}"#,
            // Duplicate version, scene: unsupported version first.
            r#"{"kind":"scene","version":99,"version":1,"nodes":[]}"#,
            // Duplicate kind naming scene: dispatch must not be last-wins.
            r#"{"kind":"scene","kind":"text","version":1,"lines":["hi"]}"#,
            // Duplicate kind: the dispatch itself must not be last-wins.
            r#"{"kind":"screen","kind":"text","version":1,"lines":["hi"]}"#,
            // Duplicate kind whose survivor is NOT a surface kind, with
            // further fields after the duplicate: the top-level scan
            // must consume the whole map or its verdict is lost to a
            // trailing-content error (Codex P2 on PR #8424).
            r#"{"kind":"screen","kind":"other","version":1}"#,
            // Duplicate kind preceded by an unrelated duplicate key: the
            // unique-key parser stops at the first duplicate, so the
            // verdict must come from the top-level scan, not from which
            // duplicate the parser hit first (Codex P2 on PR #8424).
            r#"{"x":1,"x":2,"kind":"screen","kind":"other"}"#,
            // Duplicate key at a nested level the parser inspects (a
            // style span object).
            r#"{"kind":"screen","version":1,"width":2,"height":1,"rows":["hi"],"styles":[[{"x":0,"x":0,"length":1,"fg":0,"bg":0,"bold":false}]]}"#,
        ] {
            assert!(
                parse_surface(doc).is_err(),
                "accepted duplicate-key document: {doc}"
            );
        }
        // Controls: single-key documents behave exactly as before, and a
        // duplicate in a document that is not a surface at all stays on
        // the legacy plain-text path.
        assert!(
            parse_surface(r#"{"kind":"text","version":1,"lines":["hi"]}"#)
                .unwrap()
                .is_some()
        );
        assert_eq!(
            parse_surface(r#"{"kind":"other","version":1,"version":1}"#).unwrap(),
            None
        );
    }

    #[test]
    fn viewport_crops_rows_and_does_not_split_wide_graphemes() {
        let parsed = parse_surface(
            r#"{"kind":"screen","version":1,"width":4,"height":2,"rows":["界ab","wxyz"]}"#,
        )
        .unwrap()
        .unwrap();
        let WorldSurface::Screen(screen) = parsed else {
            panic!("expected a screen surface");
        };

        let cropped = screen.fit(1, 1);
        assert_eq!(cropped.width, 1);
        assert_eq!(cropped.height, 1);
        assert_eq!(cropped.cells[0].symbol, " ");
        assert_eq!(cropped.cells[0].columns, 1);
        assert!(cropped.render_ansi().is_some());
    }

    #[test]
    fn test_fit_treats_a_zero_viewport_dimension_as_one() {
        let screen = grid(4, 2, " ");
        for (requested, expected) in [((0, 2), (1, 2)), ((4, 0), (4, 1)), ((0, 0), (1, 1))] {
            let fitted = screen.fit(requested.0, requested.1);
            assert_eq!(
                (usize::from(fitted.width), usize::from(fitted.height)),
                expected,
                "fit{requested:?}"
            );
            assert_eq!(fitted.cells.len(), expected.0 * expected.1);
            assert!(fitted.render_ansi().is_some(), "fit{requested:?}");
        }
    }

    #[test]
    fn test_fit_to_the_source_dimensions_is_the_identity() {
        let screen = grid(4, 2, "x");
        assert_eq!(screen.fit(4, 2), screen);
    }

    #[test]
    fn buffer_capture_constructor_enforces_the_portable_cell_policy() {
        let cells = vec![
            ScreenCell {
                symbol: "界".to_owned(),
                fg: 196,
                bg: 0,
                bold: true,
                columns: 2,
                token: None,
            },
            ScreenCell {
                symbol: String::new(),
                fg: 196,
                bg: 0,
                bold: true,
                columns: 0,
                token: None,
            },
        ];
        let captured = ScreenSurface::from_cells(2, 1, cells).unwrap();
        assert_eq!(captured.width, 2);
        assert_eq!(captured.cells[0].fg, 196);
        assert!(captured.render_ansi().is_some());

        assert!(ScreenSurface::from_cells(2, 1, vec![]).is_err());
        assert!(
            ScreenSurface::from_cells(
                121,
                1,
                vec![
                    ScreenCell {
                        symbol: " ".to_owned(),
                        fg: 0,
                        bg: 0,
                        bold: false,
                        columns: 1,
                        token: None,
                    };
                    121
                ]
            )
            .is_err()
        );
    }

    #[test]
    fn validates_and_normalizes_a_styled_unicode_grid() {
        let parsed = screen(json!({
            "kind": "screen",
            "version": 1,
            "width": 4,
            "height": 1,
            "rows": ["界ab"],
            "styles": [[{"x": 0, "length": 2, "fg": 196, "bg": 0, "bold": true}]],
        }))
        .unwrap()
        .unwrap();
        let WorldSurface::Screen(buffer) = parsed else {
            panic!("expected a screen surface");
        };
        assert_eq!(buffer.cells.len(), 4);
        assert_eq!(buffer.cells[0].symbol, "界");
        assert_eq!(buffer.cells[0].columns, 2);
        assert_eq!(buffer.cells[1].symbol, "");
        assert_eq!(buffer.cells[1].columns, 0);
        assert_eq!(buffer.cells[1].fg, 196);
        assert!(buffer.cells[0].bold);
        assert_eq!(buffer.cells[2].symbol, "a");
    }

    #[test]
    fn terminal_renderer_emits_only_presenter_owned_escape_codes() {
        let parsed = screen(json!({
            "kind": "screen",
            "version": 1,
            "width": 2,
            "height": 1,
            "rows": ["ok"],
            "styles": [[{"x": 0, "length": 2, "fg": 196, "bg": 0, "bold": true}]],
        }))
        .unwrap()
        .unwrap();
        let WorldSurface::Screen(buffer) = parsed else {
            panic!("expected a screen surface");
        };
        assert_eq!(
            buffer.render_ansi().unwrap(),
            "\x1b[H\x1b[2J\x1b[0;1;38;5;196mok\x1b[0m"
        );
    }

    #[test]
    fn terminal_renderer_refuses_unvalidated_escape_sequences() {
        let screen = ScreenSurface {
            version: 1,
            width: 1,
            height: 1,
            cells: vec![ScreenCell {
                symbol: "\u{001b}".to_owned(),
                fg: 0,
                bg: 0,
                bold: false,
                columns: 1,
                token: None,
            }],
        };
        assert_eq!(screen.render_ansi(), None);
    }

    #[test]
    fn rejects_guest_control_sequences_and_malformed_grid_shapes() {
        let mut control = json!({
            "kind": "screen", "version": 1, "width": 1, "height": 1,
            "rows": ["x"], "styles": [[]],
        });
        control["rows"][0] = json!("\u{001b}[2J");
        assert!(screen(control).is_err());

        let too_wide = json!({
            "kind": "screen", "version": 1, "width": 2, "height": 1,
            "rows": ["x"], "styles": [[]],
        });
        assert!(screen(too_wide).is_err());

        let split_wide_glyph = json!({
            "kind": "screen", "version": 1, "width": 3, "height": 1,
            "rows": ["界x"],
            "styles": [[{"x": 0, "length": 1, "fg": 2, "bg": 0, "bold": false}]],
        });
        assert!(screen(split_wide_glyph).is_err());
    }

    #[test]
    fn rejects_overlapping_unsorted_spans_and_unsupported_versions() {
        let overlap = json!({
            "kind": "screen", "version": 1, "width": 4, "height": 1,
            "rows": ["abcd"],
            "styles": [[
                {"x": 0, "length": 3, "fg": 2, "bg": 0, "bold": false},
                {"x": 2, "length": 2, "fg": 3, "bg": 0, "bold": false}
            ]],
        });
        assert!(screen(overlap).is_err());

        let unsupported = json!({
            "kind": "screen", "version": 2, "width": 1, "height": 1,
            "rows": ["x"], "styles": [[]],
        });
        assert!(screen(unsupported).is_err());
    }

    #[test]
    fn enforces_screen_dimension_and_style_span_caps() {
        let too_large = json!({
            "kind": "screen", "version": 1, "width": MAX_SCREEN_WIDTH + 1,
            "height": 1, "rows": ["x"], "styles": [[]],
        });
        assert!(screen(too_large).is_err());
    }

    #[test]
    fn enforces_screen_height_and_zero_dimension_caps() {
        // Height one past the cap is rejected, exactly like width.
        let too_tall = json!({
            "kind": "screen", "version": 1, "width": 1,
            "height": MAX_SCREEN_HEIGHT + 1, "rows": ["x"], "styles": [[]],
        });
        assert!(screen(too_tall).is_err());

        // Zero is below the 1..=MAX range on both axes.
        let zero_width = json!({
            "kind": "screen", "version": 1, "width": 0,
            "height": 1, "rows": ["x"], "styles": [[]],
        });
        assert!(screen(zero_width).is_err());
        let zero_height = json!({
            "kind": "screen", "version": 1, "width": 1,
            "height": 0, "rows": [], "styles": [],
        });
        assert!(screen(zero_height).is_err());
    }

    #[test]
    fn rejects_style_rows_that_do_not_match_height_and_empty_spans() {
        // A non-empty styles array carries exactly one row per screen row.
        let mismatched = json!({
            "kind": "screen", "version": 1, "width": 2, "height": 2,
            "rows": ["ab", "cd"], "styles": [[]],
        });
        assert!(screen(mismatched).is_err());

        // A span must cover at least one column.
        let empty_span = json!({
            "kind": "screen", "version": 1, "width": 2, "height": 1,
            "rows": ["ab"],
            "styles": [[{"x": 0, "length": 0, "fg": 1, "bg": 0, "bold": false}]],
        });
        assert!(screen(empty_span).is_err());
    }

    #[test]
    fn rejects_style_span_totals_past_the_cap() {
        // Every row filled with single-column spans crosses
        // MAX_SCREEN_STYLE_SPANS in total (120 x 50 = 6000 > 4096).
        let rows: Vec<String> = (0..MAX_SCREEN_HEIGHT)
            .map(|_| "x".repeat(MAX_SCREEN_WIDTH))
            .collect();
        let span_row: Vec<Value> = (0..MAX_SCREEN_WIDTH)
            .map(|x| json!({"x": x, "length": 1, "fg": 1, "bg": 0, "bold": false}))
            .collect();
        let styles: Vec<Value> = (0..MAX_SCREEN_HEIGHT)
            .map(|_| Value::Array(span_row.clone()))
            .collect();
        let flooded = json!({
            "kind": "screen", "version": 1,
            "width": MAX_SCREEN_WIDTH, "height": MAX_SCREEN_HEIGHT,
            "rows": rows, "styles": styles,
        });
        assert!(screen(flooded).is_err());
    }

    #[test]
    fn screen_serialization_is_deterministic() {
        let payload = json!({
            "kind": "screen", "version": 1, "width": 4, "height": 2,
            "rows": ["abcd", "wxyz"],
            "styles": [[{"x": 0, "length": 2, "fg": 3, "bg": 1, "bold": true}], []],
        });
        let first = screen(payload.clone()).unwrap().unwrap();
        let second = screen(payload).unwrap().unwrap();
        assert_eq!(first, second);
        let once = serde_json::to_string(&first).unwrap();
        assert_eq!(serde_json::to_string(&second).unwrap(), once);
        assert_eq!(serde_json::to_string(&first).unwrap(), once);
    }
}
