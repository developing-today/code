//! Canonical CBOR body types for the four D0 did-it tracker kinds.
//!
//! A body is a canonical CBOR array tagged with its kind and format version:
//!
//! ```text
//! ["didit.entity",      version, entity, household, kind, name, aliases, retired_at]
//! ["didit.record",      version, id, subjects, act, at, tz_offset_minutes,
//!                                details, shared, measure, note, place]
//! ["didit.expectation", version, id, subjects, act, window, active,
//!                                supersedes, ends_at, note]
//! ["didit.opinion",     version, about, stance, note, at]
//! ```
//!
//! Canonicality is judged at the CBOR value level, the way
//! [`crate::chain::decode`] judges a header: the bytes must be the canonical
//! encoding of the value they hold. Every decoder applies the same shape rules
//! as its encoder, so a body that decodes is a body that would re-encode to
//! itself.
//!
//! `subjects` and `about` are explicit and non-empty. An empty subject list is
//! refused, never read as "every current member" (design open question 2), so a
//! record's meaning does not change when an entity is added or retired.

use std::collections::{BTreeMap, HashSet};

use cbor_core::{DecodeOptions, Float, Strictness, Value};
use p2panda_core::Hash;
use serde::{Deserialize, Serialize};
use unicode_normalization::is_nfc;

/// The kind of a `didit.entity` event, as the header's `kind` names it.
pub const ENTITY_KIND: &str = "didit.entity";

/// The kind of a `didit.record` event, as the header's `kind` names it.
pub const RECORD_KIND: &str = "didit.record";

/// The kind of a `didit.expectation` event, as the header's `kind` names it.
pub const EXPECTATION_KIND: &str = "didit.expectation";

/// The kind of a `didit.opinion` event, as the header's `kind` names it.
pub const OPINION_KIND: &str = "didit.opinion";

/// The version of the body format this code writes and reads.
pub const VERSION: u64 = 1;

/// The most bytes a did-it body may have. Bodies are unbounded in the header
/// (which caps text at 512 bytes), so the tracker sets its own ceiling.
pub const MAX_BODY_BYTES: usize = 1024 * 1024;

/// The longest text field, in UTF-8 bytes.
pub const MAX_TEXT_BYTES: usize = 512;

/// The most subjects one record or expectation may name.
pub const MAX_SUBJECTS: usize = 512;

/// The most per-subject detail entries one record may carry.
pub const MAX_DETAILS: usize = 512;

/// The most aliases one entity may carry.
pub const MAX_ALIASES: usize = 512;

/// The most hours spans one hours window may list.
pub const MAX_HOURS_SPANS: usize = 512;

/// Nesting limit for a body's CBOR. Bodies are shallow by construction.
const RECURSION_LIMIT: u16 = 8;

/// Declared element limit for one array, map, or string.
const LENGTH_LIMIT: u64 = 8192;

/// Byte budget for speculative pre-allocation. Matches the header decoder.
const OOM_MITIGATION: usize = 64;

/// What a record can be about (design §3.1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entity {
    /// Stable, derived entity ID:
    /// `BLAKE3(household || kind || name_at_creation)`.
    pub entity: Hash,
    /// The household group the entity belongs to.
    pub household: Hash,
    /// What kind of thing the entity is: `"dog"`, `"cat"`, `"plant"`,
    /// `"person"`, or an open string.
    pub kind: String,
    /// The entity's name at creation. Renames are new events; the ID never
    /// changes.
    pub name: String,
    /// Other names the entity answers to.
    pub aliases: Vec<String>,
    /// When the entity was retired, in unix seconds. A retired entity keeps
    /// its history.
    pub retired_at: Option<i64>,
}

impl Entity {
    /// Checks the rules every entity body meets.
    ///
    /// # Errors
    ///
    /// Returns [`DiditError::Invalid`] for an empty or over-long text field,
    /// text that is not NFC, a repeated alias, or more aliases than
    /// [`MAX_ALIASES`].
    pub fn check(&self) -> Result<(), DiditError> {
        check_text(&self.kind, "kind", true)?;
        check_text(&self.name, "name", true)?;
        if self.aliases.len() > MAX_ALIASES {
            return Err(DiditError::Invalid(format!(
                "entity aliases has {} entries, the most is {MAX_ALIASES}",
                self.aliases.len()
            )));
        }
        let mut seen = HashSet::new();
        for alias in &self.aliases {
            check_text(alias, "alias", false)?;
            if !seen.insert(alias) {
                return Err(DiditError::Invalid("an alias is repeated".to_owned()));
            }
        }
        Ok(())
    }
}

/// A per-subject detail slot (design §3.2). Every field is optional: a record
/// is allowed to say only that something happened.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RecordDetail {
    /// Free-text note.
    pub note: Option<String>,
    /// What came of the act: `"peed"`, `"did not pee"`, `"refused"`, free text.
    pub outcome: Option<String>,
    /// A per-subject measure, when the act has one for this subject.
    pub measure: Option<Measure>,
}

impl RecordDetail {
    /// Checks the rules a detail slot meets.
    ///
    /// # Errors
    ///
    /// Returns [`DiditError::Invalid`] for over-long or non-NFC text, or a
    /// non-finite measure.
    pub fn check(&self) -> Result<(), DiditError> {
        if let Some(note) = &self.note {
            check_text(note, "detail note", false)?;
        }
        if let Some(outcome) = &self.outcome {
            check_text(outcome, "detail outcome", false)?;
        }
        if let Some(measure) = &self.measure {
            measure.check()?;
        }
        Ok(())
    }
}

/// A number with a unit (design §3.2). Absent for non-metric acts: "drank
/// water" carries no measure rather than a `0` sentinel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Measure {
    /// The value.
    pub value: f64,
    /// The unit: `"g"`, `"ml"`, `"min"`, `"km"`, or an open string.
    pub unit: String,
}

impl Measure {
    /// Checks the rules a measure meets: a finite value and a non-empty unit.
    ///
    /// # Errors
    ///
    /// Returns [`DiditError::Invalid`] for a non-finite value or bad unit.
    pub fn check(&self) -> Result<(), DiditError> {
        if !self.value.is_finite() {
            return Err(DiditError::Invalid(
                "a measure value is not finite".to_owned(),
            ));
        }
        check_text(&self.unit, "measure unit", true)
    }
}

/// Where an act happened (design §3.2, `geo?`). The design leaves the shape
/// open; the tracker stores WGS-84 decimal degrees and no more.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Place {
    /// Latitude in decimal degrees.
    pub latitude: f64,
    /// Longitude in decimal degrees.
    pub longitude: f64,
}

impl Place {
    /// Checks the rules a place meets: finite coordinates in range.
    ///
    /// # Errors
    ///
    /// Returns [`DiditError::Invalid`] for a non-finite or out-of-range
    /// coordinate.
    pub fn check(&self) -> Result<(), DiditError> {
        if !self.latitude.is_finite() || !(-90.0..=90.0).contains(&self.latitude) {
            return Err(DiditError::Invalid("a latitude is out of range".to_owned()));
        }
        if !self.longitude.is_finite() || !(-180.0..=180.0).contains(&self.longitude) {
            return Err(DiditError::Invalid(
                "a longitude is out of range".to_owned(),
            ));
        }
        Ok(())
    }
}

/// What happened (design §3.2, decision D4).
///
/// One record is one act: one author, one time, one causal position, one
/// subject set. The per-subject `details` map carries the parts that diverge.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Record {
    /// The carrying event's ID, carried for convenience in projections.
    pub id: Hash,
    /// The entity IDs the act covers, explicit and non-empty.
    pub subjects: Vec<Hash>,
    /// What happened: `"fed"`, `"walked"`, `"outside"`, `"water"`, or an open
    /// string.
    pub act: String,
    /// The author's clock, in unix seconds.
    pub at: i64,
    /// The author's UTC offset at `at`, in minutes. Evaluation uses the
    /// expectation's IANA zone instead; this field keeps the author's
    /// disagreement visible rather than silently coerced.
    pub tz_offset_minutes: i32,
    /// Per-subject detail, keyed by entity ID and sorted by key bytes.
    pub details: Vec<(Hash, RecordDetail)>,
    /// Detail that applies to every subject.
    pub shared: Option<RecordDetail>,
    /// The act's measure, absent for non-metric acts.
    pub measure: Option<Measure>,
    /// A free-text note about the whole act.
    pub note: Option<String>,
    /// Where the act happened.
    pub place: Option<Place>,
}

/// The default cross-app view of a record (design §7, decision D7).
///
/// Stable event and subject IDs are retained so related opinions and entity
/// references still resolve, while structural details and free text are
/// omitted. This is a derived view; it is never encoded as a didit body.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecordSummary {
    /// The carrying event's stable ID.
    pub id: Hash,
    /// The explicit entity IDs covered by the act.
    pub subjects: Vec<Hash>,
    /// The number of subjects, retained as a convenient summary field.
    pub subject_count: usize,
    /// What happened.
    pub act: String,
    /// The author's clock, in unix seconds.
    pub at: i64,
    /// The author's UTC offset at `at`, in minutes.
    pub tz_offset_minutes: i32,
}

impl Record {
    /// Returns the summary projection, omitting details, shared detail,
    /// measure, note, and place while retaining the stable record ID.
    #[must_use]
    pub fn summary_projection(&self) -> RecordSummary {
        RecordSummary {
            id: self.id,
            subjects: self.subjects.clone(),
            subject_count: self.subjects.len(),
            act: self.act.clone(),
            at: self.at,
            tz_offset_minutes: self.tz_offset_minutes,
        }
    }

    /// Returns the full projection without dropping any record fields.
    #[must_use]
    pub fn full_projection(&self) -> Self {
        self.clone()
    }

    /// The detail slot for `subject`, when the record carries one.
    #[must_use]
    pub fn detail_for(&self, subject: &Hash) -> Option<&RecordDetail> {
        self.details
            .binary_search_by(|(key, _)| key.as_bytes().cmp(subject.as_bytes()))
            .ok()
            .map(|index| &self.details[index].1)
    }

    /// Checks the rules every record body meets: non-empty distinct subjects,
    /// known detail keys, and valid text, measure, and offset fields.
    ///
    /// # Errors
    ///
    /// Returns [`DiditError::Invalid`] naming the rule that failed.
    pub fn check(&self) -> Result<(), DiditError> {
        check_subjects(&self.subjects, "record subjects")?;
        check_text(&self.act, "act", true)?;
        if !(-1440..=1440).contains(&self.tz_offset_minutes) {
            return Err(DiditError::Invalid(
                "tz_offset_minutes is outside -1440..=1440".to_owned(),
            ));
        }
        if self.details.len() > MAX_DETAILS {
            return Err(DiditError::Invalid(format!(
                "{} details, the most is {MAX_DETAILS}",
                self.details.len()
            )));
        }
        let subjects: HashSet<Hash> = self.subjects.iter().copied().collect();
        let mut previous: Option<[u8; 32]> = None;
        for (subject, detail) in &self.details {
            if !subjects.contains(subject) {
                return Err(DiditError::Invalid(
                    "a detail names a subject the record does not list".to_owned(),
                ));
            }
            let bytes = *subject.as_bytes();
            if let Some(prev) = previous {
                if prev >= bytes {
                    return Err(DiditError::Invalid(
                        "details are not strictly sorted by subject".to_owned(),
                    ));
                }
            }
            previous = Some(bytes);
            detail.check()?;
        }
        if let Some(shared) = &self.shared {
            shared.check()?;
        }
        if let Some(measure) = &self.measure {
            measure.check()?;
        }
        if let Some(note) = &self.note {
            check_text(note, "note", false)?;
        }
        if let Some(place) = &self.place {
            place.check()?;
        }
        Ok(())
    }
}

/// A period a count window repeats over (design §3.3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Period {
    /// One local calendar day in the expectation's time zone.
    Day,
    /// One local week, starting Monday, in the expectation's time zone.
    Week,
    /// A fixed-length cycle anchored at an absolute instant.
    Custom(PeriodCustom),
}

/// A custom cycle: `length_seconds` starting at `anchor` (design §3.3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeriodCustom {
    /// The cycle's anchor, in unix seconds.
    pub anchor: i64,
    /// The cycle's length, in seconds.
    pub length_seconds: u64,
}

/// What a `didit.expectation` expects (design §3.3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Window {
    /// At least `count` matching records per period.
    CountPerPeriod {
        /// The least number of records the period needs.
        count: u64,
        /// The period the count repeats over.
        period: Period,
    },
    /// Consecutive matching records no further apart than `max_seconds` and
    /// no closer together than `min_seconds`, inside the active spans.
    Interval {
        /// The least gap between two records, in seconds.
        min_seconds: u64,
        /// The most gap the tracker tolerates, in seconds.
        max_seconds: u64,
    },
    /// At least one matching record per period.
    AtLeastOncePer {
        /// The period the requirement repeats over.
        period: Period,
    },
}

impl Window {
    /// Checks the rules a window meets.
    ///
    /// # Errors
    ///
    /// Returns [`DiditError::Invalid`] for a zero count, an interval whose
    /// minimum exceeds its maximum, or a zero-length custom period.
    pub fn check(&self) -> Result<(), DiditError> {
        match self {
            Window::CountPerPeriod { count, period } => {
                if *count == 0 {
                    return Err(DiditError::Invalid(
                        "a count window asks for at least one record".to_owned(),
                    ));
                }
                period.check()
            }
            Window::Interval {
                min_seconds,
                max_seconds,
            } => {
                if min_seconds > max_seconds {
                    return Err(DiditError::Invalid(
                        "an interval's minimum exceeds its maximum".to_owned(),
                    ));
                }
                Ok(())
            }
            Window::AtLeastOncePer { period } => period.check(),
        }
    }
}

impl Period {
    fn check(&self) -> Result<(), DiditError> {
        if let Period::Custom(custom) = self {
            if custom.length_seconds == 0 {
                return Err(DiditError::Invalid(
                    "a custom period has zero length".to_owned(),
                ));
            }
        }
        Ok(())
    }
}

/// One active hours span, in minutes since local midnight (design §3.3).
/// `from_minute` greater than `to_minute` wraps past midnight.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HoursSpan {
    /// The span's start, in minutes since local midnight (`0..=1440`).
    pub from_minute: u32,
    /// The span's end, in minutes since local midnight (`0..=1440`).
    pub to_minute: u32,
    /// Days the span applies to, `0 = Monday .. 6 = Sunday`. `None` means every
    /// day.
    pub days: Option<Vec<u8>>,
}

impl HoursSpan {
    fn check(&self) -> Result<(), DiditError> {
        check_minute(self.from_minute, "from_minute")?;
        check_minute(self.to_minute, "to_minute")?;
        if self.from_minute == self.to_minute {
            return Err(DiditError::Invalid(
                "an hours span starts and ends at the same minute".to_owned(),
            ));
        }
        if let Some(days) = &self.days {
            if days.is_empty() {
                return Err(DiditError::Invalid(
                    "an hours span's day list is empty".to_owned(),
                ));
            }
            let mut seen = HashSet::new();
            for day in days {
                if *day > 6 {
                    return Err(DiditError::Invalid(
                        "a day is outside 0..=6 (0 = Monday)".to_owned(),
                    ));
                }
                if !seen.insert(*day) {
                    return Err(DiditError::Invalid("a day is repeated".to_owned()));
                }
            }
        }
        Ok(())
    }
}

/// When an expectation applies at all (design §3.3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActiveWindow {
    /// Around the clock.
    Always,
    /// Inside the listed spans, in the expectation's IANA time zone.
    Hours {
        /// The spans the expectation applies in.
        hours: Vec<HoursSpan>,
        /// The IANA zone the spans are expressed in, e.g. `"America/Chicago"`.
        tz: String,
    },
    /// Outside the household's sleep window, in the expectation's IANA zone.
    SleepWindow {
        /// When sleep starts, in minutes since local midnight.
        from_minute: u32,
        /// When sleep ends, in minutes since local midnight.
        to_minute: u32,
        /// The IANA zone the window is expressed in.
        tz: String,
    },
}

impl ActiveWindow {
    /// The IANA time zone this window evaluates in.
    #[must_use]
    pub fn tz(&self) -> Option<&str> {
        match self {
            ActiveWindow::Always => None,
            ActiveWindow::Hours { tz, .. } | ActiveWindow::SleepWindow { tz, .. } => Some(tz),
        }
    }

    /// Checks the rules a window meets, including that its time zone is a
    /// known IANA name.
    ///
    /// # Errors
    ///
    /// Returns [`DiditError::Invalid`] naming the rule that failed.
    pub fn check(&self) -> Result<(), DiditError> {
        match self {
            ActiveWindow::Always => Ok(()),
            ActiveWindow::Hours { hours, tz } => {
                check_tz(tz)?;
                if hours.is_empty() {
                    return Err(DiditError::Invalid(
                        "an hours window lists no spans".to_owned(),
                    ));
                }
                if hours.len() > MAX_HOURS_SPANS {
                    return Err(DiditError::Invalid(format!(
                        "hours spans has {} entries, the most is {MAX_HOURS_SPANS}",
                        hours.len()
                    )));
                }
                for span in hours {
                    span.check()?;
                }
                Ok(())
            }
            ActiveWindow::SleepWindow {
                from_minute,
                to_minute,
                tz,
            } => {
                check_tz(tz)?;
                check_minute(*from_minute, "from_minute")?;
                check_minute(*to_minute, "to_minute")?;
                if from_minute == to_minute {
                    return Err(DiditError::Invalid(
                        "a sleep window starts and ends at the same minute".to_owned(),
                    ));
                }
                Ok(())
            }
        }
    }
}

/// What should happen, as an append-only declaration (design §3.3, D1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Expectation {
    /// The carrying event's ID.
    pub id: Hash,
    /// The entities the expectation covers, explicit and non-empty.
    pub subjects: Vec<Hash>,
    /// The act that satisfies it; must equal [`Record::act`].
    pub act: String,
    /// What is expected.
    pub window: Window,
    /// When the expectation applies.
    pub active: ActiveWindow,
    /// The prior expectation this one replaces, if any.
    pub supersedes: Option<Hash>,
    /// An optional stop date, in unix seconds.
    pub ends_at: Option<i64>,
    /// A free-text note.
    pub note: Option<String>,
}

impl Expectation {
    /// Checks the rules every expectation body meets: non-empty distinct
    /// subjects, a valid window and active window, valid text, and no
    /// self-supersede.
    ///
    /// # Errors
    ///
    /// Returns [`DiditError::Invalid`] naming the rule that failed.
    pub fn check(&self) -> Result<(), DiditError> {
        if self.supersedes == Some(self.id) {
            return Err(DiditError::Invalid(
                "an expectation cannot supersede itself".to_owned(),
            ));
        }
        check_subjects(&self.subjects, "expectation subjects")?;
        check_text(&self.act, "act", true)?;
        self.window.check()?;
        self.active.check()?;
        if let Some(note) = &self.note {
            check_text(note, "note", false)?;
        }
        Ok(())
    }
}

/// What someone thinks about a record or an expectation (design §3.4, D3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Opinion {
    /// The record and/or expectation event IDs the opinion is about, explicit
    /// and non-empty.
    pub about: Vec<Hash>,
    /// `"good"`, `"bad"`, `"unsure"`, or an open string.
    pub stance: String,
    /// A free-text note.
    pub note: Option<String>,
    /// The author's clock, in unix seconds.
    pub at: i64,
}

impl Opinion {
    /// Checks the rules every opinion body meets.
    ///
    /// # Errors
    ///
    /// Returns [`DiditError::Invalid`] naming the rule that failed.
    pub fn check(&self) -> Result<(), DiditError> {
        check_subjects(&self.about, "opinion about")?;
        check_text(&self.stance, "stance", true)?;
        if let Some(note) = &self.note {
            check_text(note, "note", false)?;
        }
        Ok(())
    }
}

/// Why a did-it body was refused, or why one could not be encoded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DiditError {
    /// The body is longer than [`MAX_BODY_BYTES`].
    #[error("a did-it body is {0} bytes, the most allowed is {max}", max = MAX_BODY_BYTES)]
    Oversized(usize),
    /// The bytes are CBOR that strict decoding refuses.
    #[error("the did-it body is not CBOR: {0}")]
    Cbor(String),
    /// The bytes decode, but they are not the canonical encoding of their value.
    #[error("the did-it body is not canonical CBOR")]
    NotCanonical,
    /// The body's tag names another kind.
    #[error("the did-it body is for kind {0:?}, not {1:?}")]
    Kind(String, &'static str),
    /// The format version is not [`VERSION`].
    #[error("the did-it body has version {0}, this code reads version {v}", v = VERSION)]
    Version(u64),
    /// The body has the wrong shape for its kind.
    #[error("the did-it body has the wrong shape: {0}")]
    Shape(String),
    /// The body has the right shape but breaks a model rule.
    #[error("the did-it body is invalid: {0}")]
    Invalid(String),
}

/// Encodes an entity as an event body.
///
/// # Errors
///
/// Returns the rule [`Entity::check`] fails, or [`DiditError::Oversized`].
pub fn encode_entity_body(entity: &Entity) -> Result<Vec<u8>, DiditError> {
    entity.check()?;
    check_size(
        Value::Array(vec![
            tag(ENTITY_KIND),
            Value::Unsigned(VERSION),
            hash_value(&entity.entity),
            hash_value(&entity.household),
            text_value(&entity.kind),
            text_value(&entity.name),
            Value::Array(
                entity
                    .aliases
                    .iter()
                    .map(|alias| text_value(alias))
                    .collect(),
            ),
            optional_int(entity.retired_at),
        ])
        .encode(),
    )
}

/// Decodes an event body as an entity.
///
/// # Errors
///
/// Returns the reason the body was refused. See [`DiditError`].
pub fn decode_entity_body(body: &[u8]) -> Result<Entity, DiditError> {
    let items = top_level(body, ENTITY_KIND, 8)?;
    let entity = Entity {
        entity: hash_field(&items[2], "entity")?,
        household: hash_field(&items[3], "household")?,
        kind: text_field(&items[4], "kind")?,
        name: text_field(&items[5], "name")?,
        aliases: text_array(&items[6], "aliases")?,
        retired_at: optional_integer(&items[7], "retired_at")?,
    };
    entity.check()?;
    Ok(entity)
}

/// Encodes a record as an event body.
///
/// # Errors
///
/// Returns the rule [`Record::check`] fails, or [`DiditError::Oversized`].
pub fn encode_record_body(record: &Record) -> Result<Vec<u8>, DiditError> {
    record.check()?;
    let mut details = BTreeMap::new();
    for (subject, detail) in &record.details {
        details.insert(hash_value(subject), detail_value(detail));
    }
    check_size(
        Value::Array(vec![
            tag(RECORD_KIND),
            Value::Unsigned(VERSION),
            hash_value(&record.id),
            hash_array(&record.subjects),
            text_value(&record.act),
            int_value(record.at),
            int_value(i64::from(record.tz_offset_minutes)),
            Value::Map(details),
            match &record.shared {
                Some(detail) => detail_value(detail),
                None => Value::null(),
            },
            match &record.measure {
                Some(measure) => measure_value(measure),
                None => Value::null(),
            },
            optional_text(record.note.as_deref()),
            match &record.place {
                Some(place) => Value::Array(vec![
                    Value::Float(Float::from(place.latitude)),
                    Value::Float(Float::from(place.longitude)),
                ]),
                None => Value::null(),
            },
        ])
        .encode(),
    )
}

/// Decodes an event body as a record.
///
/// # Errors
///
/// Returns the reason the body was refused. See [`DiditError`].
pub fn decode_record_body(body: &[u8]) -> Result<Record, DiditError> {
    let items = top_level(body, RECORD_KIND, 12)?;
    let offset = integer(&items[6], "tz_offset_minutes")?;
    let tz_offset_minutes = i32::try_from(offset)
        .map_err(|_| DiditError::Invalid("tz_offset_minutes is out of range".to_owned()))?;
    let mut details = Vec::new();
    let Value::Map(map) = &items[7] else {
        return Err(DiditError::Shape("details is not a map".to_owned()));
    };
    for (key, value) in map {
        details.push((hash_field(key, "a detail subject")?, detail_field(value)?));
    }
    details.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let record = Record {
        id: hash_field(&items[2], "id")?,
        subjects: hash_array_field(&items[3], "subjects")?,
        act: text_field(&items[4], "act")?,
        at: integer(&items[5], "at")?,
        tz_offset_minutes,
        details,
        shared: optional_detail(&items[8])?,
        measure: optional_measure(&items[9])?,
        note: optional_text_field(&items[10], "note")?,
        place: optional_place(&items[11])?,
    };
    record.check()?;
    Ok(record)
}

/// Encodes an expectation as an event body.
///
/// # Errors
///
/// Returns the rule [`Expectation::check`] fails, or [`DiditError::Oversized`].
pub fn encode_expectation_body(expectation: &Expectation) -> Result<Vec<u8>, DiditError> {
    expectation.check()?;
    check_size(
        Value::Array(vec![
            tag(EXPECTATION_KIND),
            Value::Unsigned(VERSION),
            hash_value(&expectation.id),
            hash_array(&expectation.subjects),
            text_value(&expectation.act),
            window_value(&expectation.window),
            active_value(&expectation.active),
            match &expectation.supersedes {
                Some(hash) => hash_value(hash),
                None => Value::null(),
            },
            optional_int(expectation.ends_at),
            optional_text(expectation.note.as_deref()),
        ])
        .encode(),
    )
}

/// Decodes an event body as an expectation.
///
/// # Errors
///
/// Returns the reason the body was refused. See [`DiditError`].
pub fn decode_expectation_body(body: &[u8]) -> Result<Expectation, DiditError> {
    let items = top_level(body, EXPECTATION_KIND, 10)?;
    let expectation = Expectation {
        id: hash_field(&items[2], "id")?,
        subjects: hash_array_field(&items[3], "subjects")?,
        act: text_field(&items[4], "act")?,
        window: window_field(&items[5])?,
        active: active_field(&items[6])?,
        supersedes: match &items[7] {
            Value::SimpleValue(simple) if simple.to_u8() == 22 => None,
            value => Some(hash_field(value, "supersedes")?),
        },
        ends_at: optional_integer(&items[8], "ends_at")?,
        note: optional_text_field(&items[9], "note")?,
    };
    expectation.check()?;
    Ok(expectation)
}

/// Encodes an opinion as an event body.
///
/// # Errors
///
/// Returns the rule [`Opinion::check`] fails, or [`DiditError::Oversized`].
pub fn encode_opinion_body(opinion: &Opinion) -> Result<Vec<u8>, DiditError> {
    opinion.check()?;
    check_size(
        Value::Array(vec![
            tag(OPINION_KIND),
            Value::Unsigned(VERSION),
            hash_array(&opinion.about),
            text_value(&opinion.stance),
            optional_text(opinion.note.as_deref()),
            int_value(opinion.at),
        ])
        .encode(),
    )
}

/// Decodes an event body as an opinion.
///
/// # Errors
///
/// Returns the reason the body was refused. See [`DiditError`].
pub fn decode_opinion_body(body: &[u8]) -> Result<Opinion, DiditError> {
    let items = top_level(body, OPINION_KIND, 6)?;
    let opinion = Opinion {
        about: hash_array_field(&items[2], "about")?,
        stance: text_field(&items[3], "stance")?,
        note: optional_text_field(&items[4], "note")?,
        at: integer(&items[5], "at")?,
    };
    opinion.check()?;
    Ok(opinion)
}

fn check_size(body: Vec<u8>) -> Result<Vec<u8>, DiditError> {
    if body.len() > MAX_BODY_BYTES {
        return Err(DiditError::Oversized(body.len()));
    }
    Ok(body)
}

fn check_text(value: &str, what: &str, non_empty: bool) -> Result<(), DiditError> {
    if non_empty && value.is_empty() {
        return Err(DiditError::Invalid(format!("{what} is empty")));
    }
    if value.len() > MAX_TEXT_BYTES {
        return Err(DiditError::Invalid(format!(
            "{what} is {} bytes, the most is {MAX_TEXT_BYTES}",
            value.len()
        )));
    }
    if !is_nfc(value) {
        return Err(DiditError::Invalid(format!("{what} is not in Unicode NFC")));
    }
    Ok(())
}

fn check_subjects(subjects: &[Hash], what: &str) -> Result<(), DiditError> {
    if subjects.is_empty() {
        return Err(DiditError::Invalid(format!(
            "{what} is empty; subjects are explicit entity IDs"
        )));
    }
    if subjects.len() > MAX_SUBJECTS {
        return Err(DiditError::Invalid(format!(
            "{what} has {} entries, the most is {MAX_SUBJECTS}",
            subjects.len()
        )));
    }
    let mut seen = HashSet::new();
    for subject in subjects {
        if !seen.insert(*subject) {
            return Err(DiditError::Invalid(format!("{what} repeats an entity")));
        }
    }
    Ok(())
}

fn check_minute(minute: u32, what: &str) -> Result<(), DiditError> {
    if minute > 1440 {
        return Err(DiditError::Invalid(format!("{what} is outside 0..=1440")));
    }
    Ok(())
}

fn check_tz(tz: &str) -> Result<(), DiditError> {
    tz.parse::<chrono_tz::Tz>()
        .map(|_| ())
        .map_err(|_| DiditError::Invalid(format!("{tz:?} is not a known IANA time zone")))
}

fn top_level(
    body: &[u8],
    kind: &'static str,
    count: usize,
) -> Result<Vec<Value<'static>>, DiditError> {
    if body.len() > MAX_BODY_BYTES {
        return Err(DiditError::Oversized(body.len()));
    }
    let value = DecodeOptions::new()
        .strictness(Strictness::STRICT)
        .recursion_limit(RECURSION_LIMIT)
        .length_limit(LENGTH_LIMIT)
        .oom_mitigation(OOM_MITIGATION)
        .decode_owned(body)
        .map_err(|err| DiditError::Cbor(err.to_string()))?;
    if value.encode().as_slice() != body {
        return Err(DiditError::NotCanonical);
    }
    let Value::Array(items) = value else {
        return Err(DiditError::Shape("the body is not an array".to_owned()));
    };
    let Some(Value::TextString(tag)) = items.first() else {
        return Err(DiditError::Shape("the body's tag is not text".to_owned()));
    };
    if tag.as_ref() != kind {
        return Err(DiditError::Kind(tag.to_string(), kind));
    }
    if items.len() != count {
        return Err(DiditError::Shape(format!(
            "a {kind} body has {count} items, this one has {}",
            items.len()
        )));
    }
    let version = unsigned(&items[1], "version")?;
    if version != VERSION {
        return Err(DiditError::Version(version));
    }
    Ok(items)
}

fn tag(kind: &str) -> Value<'static> {
    Value::TextString(kind.to_owned().into())
}

fn hash_value(hash: &Hash) -> Value<'static> {
    Value::ByteString(hash.as_bytes().to_vec().into())
}

fn hash_array(hashes: &[Hash]) -> Value<'static> {
    Value::Array(hashes.iter().map(hash_value).collect())
}

fn text_value(text: &str) -> Value<'static> {
    Value::TextString(text.to_owned().into())
}

fn optional_text(text: Option<&str>) -> Value<'static> {
    match text {
        Some(text) => text_value(text),
        None => Value::null(),
    }
}

fn int_value(number: i64) -> Value<'static> {
    if number >= 0 {
        Value::Unsigned(number.unsigned_abs())
    } else {
        Value::Negative(number.unsigned_abs() - 1)
    }
}

fn optional_int(number: Option<i64>) -> Value<'static> {
    match number {
        Some(number) => int_value(number),
        None => Value::null(),
    }
}

fn measure_value(measure: &Measure) -> Value<'static> {
    Value::Array(vec![
        Value::Float(Float::from(measure.value)),
        text_value(&measure.unit),
    ])
}

fn detail_value(detail: &RecordDetail) -> Value<'static> {
    Value::Array(vec![
        optional_text(detail.note.as_deref()),
        optional_text(detail.outcome.as_deref()),
        match &detail.measure {
            Some(measure) => measure_value(measure),
            None => Value::null(),
        },
    ])
}

fn window_value(window: &Window) -> Value<'static> {
    match window {
        Window::CountPerPeriod { count, period } => Value::Array(vec![
            text_value("count_per_period"),
            Value::Unsigned(*count),
            period_value(period),
        ]),
        Window::Interval {
            min_seconds,
            max_seconds,
        } => Value::Array(vec![
            text_value("interval"),
            Value::Unsigned(*min_seconds),
            Value::Unsigned(*max_seconds),
        ]),
        Window::AtLeastOncePer { period } => {
            Value::Array(vec![text_value("at_least_once_per"), period_value(period)])
        }
    }
}

fn period_value(period: &Period) -> Value<'static> {
    match period {
        Period::Day => text_value("day"),
        Period::Week => text_value("week"),
        Period::Custom(custom) => Value::Array(vec![
            text_value("custom"),
            int_value(custom.anchor),
            Value::Unsigned(custom.length_seconds),
        ]),
    }
}

fn active_value(active: &ActiveWindow) -> Value<'static> {
    match active {
        ActiveWindow::Always => Value::Array(vec![text_value("always")]),
        ActiveWindow::Hours { hours, tz } => Value::Array(vec![
            text_value("hours"),
            text_value(tz),
            Value::Array(
                hours
                    .iter()
                    .map(|span| {
                        Value::Array(vec![
                            Value::Unsigned(u64::from(span.from_minute)),
                            Value::Unsigned(u64::from(span.to_minute)),
                            match &span.days {
                                Some(days) => Value::Array(
                                    days.iter()
                                        .map(|day| Value::Unsigned(u64::from(*day)))
                                        .collect(),
                                ),
                                None => Value::null(),
                            },
                        ])
                    })
                    .collect(),
            ),
        ]),
        ActiveWindow::SleepWindow {
            from_minute,
            to_minute,
            tz,
        } => Value::Array(vec![
            text_value("sleep_window"),
            Value::Unsigned(u64::from(*from_minute)),
            Value::Unsigned(u64::from(*to_minute)),
            text_value(tz),
        ]),
    }
}

fn window_field(value: &Value<'_>) -> Result<Window, DiditError> {
    let Value::Array(items) = value else {
        return Err(DiditError::Shape("a window is not an array".to_owned()));
    };
    let tag = text_field(
        items
            .first()
            .ok_or_else(|| DiditError::Shape("a window is empty".to_owned()))?,
        "window tag",
    )?;
    match tag.as_str() {
        "count_per_period" if items.len() == 3 => Ok(Window::CountPerPeriod {
            count: unsigned(&items[1], "count")?,
            period: period_field(&items[2])?,
        }),
        "interval" if items.len() == 3 => Ok(Window::Interval {
            min_seconds: unsigned(&items[1], "min_seconds")?,
            max_seconds: unsigned(&items[2], "max_seconds")?,
        }),
        "at_least_once_per" if items.len() == 2 => Ok(Window::AtLeastOncePer {
            period: period_field(&items[1])?,
        }),
        other => Err(DiditError::Shape(format!("{other:?} is not a window"))),
    }
}

fn period_field(value: &Value<'_>) -> Result<Period, DiditError> {
    match value {
        Value::TextString(text) if text.as_ref() == "day" => Ok(Period::Day),
        Value::TextString(text) if text.as_ref() == "week" => Ok(Period::Week),
        Value::Array(items) if items.len() == 3 => {
            let tag = text_field(&items[0], "period tag")?;
            if tag != "custom" {
                return Err(DiditError::Shape(format!("{tag:?} is not a period")));
            }
            Ok(Period::Custom(PeriodCustom {
                anchor: integer(&items[1], "anchor")?,
                length_seconds: unsigned(&items[2], "length_seconds")?,
            }))
        }
        _ => Err(DiditError::Shape(
            "a period is not day, week, or custom".to_owned(),
        )),
    }
}

fn active_field(value: &Value<'_>) -> Result<ActiveWindow, DiditError> {
    let Value::Array(items) = value else {
        return Err(DiditError::Shape(
            "an active window is not an array".to_owned(),
        ));
    };
    let tag = text_field(
        items
            .first()
            .ok_or_else(|| DiditError::Shape("an active window is empty".to_owned()))?,
        "active window tag",
    )?;
    match tag.as_str() {
        "always" if items.len() == 1 => Ok(ActiveWindow::Always),
        "hours" if items.len() == 3 => {
            let tz = text_field(&items[1], "tz")?;
            let Value::Array(spans) = &items[2] else {
                return Err(DiditError::Shape("hours is not an array".to_owned()));
            };
            let mut hours = Vec::with_capacity(spans.len());
            for span in spans {
                let Value::Array(parts) = span else {
                    return Err(DiditError::Shape(
                        "an hours span is not an array".to_owned(),
                    ));
                };
                if parts.len() != 3 {
                    return Err(DiditError::Shape(
                        "an hours span is not [from, to, days]".to_owned(),
                    ));
                }
                hours.push(HoursSpan {
                    from_minute: minute_field(&parts[0], "from_minute")?,
                    to_minute: minute_field(&parts[1], "to_minute")?,
                    days: day_list(&parts[2])?,
                });
            }
            Ok(ActiveWindow::Hours { hours, tz })
        }
        "sleep_window" if items.len() == 4 => Ok(ActiveWindow::SleepWindow {
            from_minute: minute_field(&items[1], "from_minute")?,
            to_minute: minute_field(&items[2], "to_minute")?,
            tz: text_field(&items[3], "tz")?,
        }),
        other => Err(DiditError::Shape(format!(
            "{other:?} is not an active window"
        ))),
    }
}

fn day_list(value: &Value<'_>) -> Result<Option<Vec<u8>>, DiditError> {
    match value {
        Value::SimpleValue(simple) if simple.to_u8() == 22 => Ok(None),
        Value::Array(days) => {
            let mut out = Vec::with_capacity(days.len());
            for day in days {
                let day = unsigned(day, "a day")?;
                out.push(
                    u8::try_from(day)
                        .map_err(|_| DiditError::Invalid("a day is out of range".to_owned()))?,
                );
            }
            Ok(Some(out))
        }
        _ => Err(DiditError::Shape("days is not an array or null".to_owned())),
    }
}

fn detail_field(value: &Value<'_>) -> Result<RecordDetail, DiditError> {
    let Value::Array(items) = value else {
        return Err(DiditError::Shape("a detail is not an array".to_owned()));
    };
    if items.len() != 3 {
        return Err(DiditError::Shape(
            "a detail is not [note, outcome, measure]".to_owned(),
        ));
    }
    Ok(RecordDetail {
        note: optional_text_field(&items[0], "detail note")?,
        outcome: optional_text_field(&items[1], "detail outcome")?,
        measure: optional_measure(&items[2])?,
    })
}

fn optional_detail(value: &Value<'_>) -> Result<Option<RecordDetail>, DiditError> {
    match value {
        Value::SimpleValue(simple) if simple.to_u8() == 22 => Ok(None),
        _ => Ok(Some(detail_field(value)?)),
    }
}

fn optional_measure(value: &Value<'_>) -> Result<Option<Measure>, DiditError> {
    match value {
        Value::SimpleValue(simple) if simple.to_u8() == 22 => Ok(None),
        Value::Array(items) if items.len() == 2 => Ok(Some(Measure {
            value: float_field(&items[0], "measure value")?,
            unit: text_field(&items[1], "measure unit")?,
        })),
        _ => Err(DiditError::Shape(
            "a measure is not [value, unit]".to_owned(),
        )),
    }
}

fn optional_place(value: &Value<'_>) -> Result<Option<Place>, DiditError> {
    match value {
        Value::SimpleValue(simple) if simple.to_u8() == 22 => Ok(None),
        Value::Array(items) if items.len() == 2 => Ok(Some(Place {
            latitude: float_field(&items[0], "latitude")?,
            longitude: float_field(&items[1], "longitude")?,
        })),
        _ => Err(DiditError::Shape(
            "a place is not [latitude, longitude]".to_owned(),
        )),
    }
}

fn unsigned(value: &Value<'_>, what: &str) -> Result<u64, DiditError> {
    match value {
        Value::Unsigned(number) => Ok(*number),
        _ => Err(DiditError::Shape(format!(
            "{what} is not an unsigned integer"
        ))),
    }
}

fn integer(value: &Value<'_>, what: &str) -> Result<i64, DiditError> {
    match value {
        Value::Unsigned(number) => i64::try_from(*number)
            .map_err(|_| DiditError::Invalid(format!("{what} is out of range"))),
        Value::Negative(number) => {
            let number = -1i128 - i128::from(*number);
            i64::try_from(number)
                .map_err(|_| DiditError::Invalid(format!("{what} is out of range")))
        }
        _ => Err(DiditError::Shape(format!("{what} is not an integer"))),
    }
}

fn optional_integer(value: &Value<'_>, what: &str) -> Result<Option<i64>, DiditError> {
    match value {
        Value::SimpleValue(simple) if simple.to_u8() == 22 => Ok(None),
        _ => Ok(Some(integer(value, what)?)),
    }
}

fn float_field(value: &Value<'_>, what: &str) -> Result<f64, DiditError> {
    match value {
        Value::Float(float) => Ok(float.to_f64()),
        _ => Err(DiditError::Shape(format!("{what} is not a number"))),
    }
}

fn text_field(value: &Value<'_>, what: &str) -> Result<String, DiditError> {
    match value {
        Value::TextString(text) => Ok(text.to_string()),
        _ => Err(DiditError::Shape(format!("{what} is not text"))),
    }
}

fn optional_text_field(value: &Value<'_>, what: &str) -> Result<Option<String>, DiditError> {
    match value {
        Value::SimpleValue(simple) if simple.to_u8() == 22 => Ok(None),
        _ => Ok(Some(text_field(value, what)?)),
    }
}

fn text_array(value: &Value<'_>, what: &str) -> Result<Vec<String>, DiditError> {
    let Value::Array(items) = value else {
        return Err(DiditError::Shape(format!("{what} is not an array")));
    };
    items
        .iter()
        .map(|item| text_field(item, what))
        .collect::<Result<_, _>>()
}

fn minute_field(value: &Value<'_>, what: &str) -> Result<u32, DiditError> {
    let minute = unsigned(value, what)?;
    u32::try_from(minute).map_err(|_| DiditError::Invalid(format!("{what} is out of range")))
}

fn bytes32(value: &Value<'_>, what: &str) -> Result<[u8; 32], DiditError> {
    let Value::ByteString(bytes) = value else {
        return Err(DiditError::Shape(format!("{what} is not a byte string")));
    };
    <[u8; 32]>::try_from(bytes.as_ref())
        .map_err(|_| DiditError::Shape(format!("{what} is {} bytes, it must be 32", bytes.len())))
}

fn hash_field(value: &Value<'_>, what: &str) -> Result<Hash, DiditError> {
    Ok(Hash::from_bytes(bytes32(value, what)?))
}

fn hash_array_field(value: &Value<'_>, what: &str) -> Result<Vec<Hash>, DiditError> {
    let Value::Array(items) = value else {
        return Err(DiditError::Shape(format!("{what} is not an array")));
    };
    items
        .iter()
        .map(|item| hash_field(item, what))
        .collect::<Result<_, _>>()
}
