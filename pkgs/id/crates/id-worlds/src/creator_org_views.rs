//! Read-only organization views over a creator's own worlds, programs and
//! announcement schedule (#13745; design #6309 outline §6, step 4).
//!
//! [`OrganizationView::build`] joins three local sources and never writes:
//!
//! - the author-scoped index ([`crate::creator_index`], #13741), with
//!   where it came from (a fresh stored index, or a rebuild and why);
//! - the draft / published state each program version carries in that
//!   index ([`crate::program_publish_state`], #13742);
//! - the announcement schedule ([`crate::creator_announcements`],
//!   #13743), when the caller loaded it.
//!
//! The view renders as text ([`OrganizationView::to_text`]) and as JSON
//! ([`OrganizationView::to_json`]) with the same schema discipline as the
//! v1 owner report ([`crate::world_owner_report`]): a major
//! [`ORG_VIEW_SCHEMA_VERSION`], consumers accept added optional fields and
//! refuse any other major version ([`check_schema_version`]), and a
//! section with no source is `{"value": null, "reason": "…"}`, never an
//! empty list that reads as "none".
//!
//! Nothing here ranks other authors, reads a hosted service or adds
//! tracking: it only rearranges what the author's own node already holds.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::Serialize;

use crate::creator_announcements::{
    AnnouncementEntry, AnnouncementLog, AnnouncementState, AnnouncementSubject,
};
use crate::creator_index::{AuthorIndex, IndexQuery, IndexStatus, ProgramEntry, RebuildReason};
use crate::program_bundle::ProgramIdentity;
use crate::program_publish_state::ProgramPublishState;

/// Major version of the view's JSON shape. Consumers accept added
/// optional fields and refuse any other major version.
pub const ORG_VIEW_SCHEMA_VERSION: u32 = 1;

/// How many finished announcements the view lists by default.
pub const DEFAULT_RECENT_LIMIT: usize = 20;

/// The group a program falls under when grouped by tag and it has none.
pub const UNTAGGED_GROUP: &str = "(untagged)";

/// How the view groups the programs it lists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Grouping {
    /// One group per world, in world id order.
    #[default]
    World,
    /// One group per publish state: drafts, then published.
    State,
    /// One group per tag, in tag order; a program with several tags is
    /// listed under each, and one with none under [`UNTAGGED_GROUP`].
    Tag,
}

/// What to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewRequest {
    /// Which programs to list and in what order within a group. A world
    /// filter also narrows the world list and the schedule.
    pub query: IndexQuery,
    /// How to group the programs.
    pub grouping: Grouping,
    /// The view's clock, Unix milliseconds: what is upcoming and what is
    /// overdue is measured against it.
    pub now_ms: u64,
    /// How many finished announcements (cancelled, skipped, delivered)
    /// to list, most recent first.
    pub recent_limit: usize,
}

impl ViewRequest {
    /// Every program, grouped by world, at `now_ms`.
    #[must_use]
    pub fn new(now_ms: u64) -> Self {
        Self {
            query: IndexQuery::default(),
            grouping: Grouping::default(),
            now_ms,
            recent_limit: DEFAULT_RECENT_LIMIT,
        }
    }
}

/// Why a view section is unavailable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewUnavailable {
    /// The caller did not load the announcement schedule.
    ScheduleNotLoaded,
}

impl ViewUnavailable {
    /// The human-readable form the text view prints.
    #[must_use]
    pub const fn describe(self) -> &'static str {
        match self {
            Self::ScheduleNotLoaded => "the announcement schedule was not loaded",
        }
    }
}

/// Where the index behind the view came from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum IndexSource {
    /// A stored index that matched every journal.
    Fresh,
    /// Rebuilt from the journals for this view.
    Rebuilt {
        /// Why: `missing`, `unreadable`, `other_author` or `stale`.
        reason: &'static str,
        /// For a stale index, the worlds whose journals changed.
        #[serde(skip_serializing_if = "Vec::is_empty")]
        changed: Vec<String>,
        /// For a stale index, the worlds it did not know.
        #[serde(skip_serializing_if = "Vec::is_empty")]
        added: Vec<String>,
        /// For a stale index, the worlds that are gone.
        #[serde(skip_serializing_if = "Vec::is_empty")]
        removed: Vec<String>,
    },
}

impl IndexSource {
    fn from_status(status: &IndexStatus) -> Self {
        match status {
            IndexStatus::Fresh => Self::Fresh,
            IndexStatus::Rebuilt(reason) => {
                let (name, staleness) = match reason {
                    RebuildReason::Missing => ("missing", None),
                    RebuildReason::Unreadable => ("unreadable", None),
                    RebuildReason::OtherAuthor => ("other_author", None),
                    RebuildReason::Stale(staleness) => ("stale", Some(staleness)),
                };
                Self::Rebuilt {
                    reason: name,
                    changed: staleness.map(|s| s.changed.clone()).unwrap_or_default(),
                    added: staleness.map(|s| s.added.clone()).unwrap_or_default(),
                    removed: staleness.map(|s| s.removed.clone()).unwrap_or_default(),
                }
            }
        }
    }

    fn describe(&self) -> String {
        match self {
            Self::Fresh => "fresh".to_owned(),
            Self::Rebuilt {
                reason,
                changed,
                added,
                removed,
            } => {
                let why = match *reason {
                    "missing" => "no stored index".to_owned(),
                    "unreadable" => "the stored index was unreadable".to_owned(),
                    "other_author" => "the stored index was for another author".to_owned(),
                    _ => format!(
                        "the stored index was stale: {} changed, {} added, {} removed",
                        changed.len(),
                        added.len(),
                        removed.len()
                    ),
                };
                format!("rebuilt ({why})")
            }
        }
    }
}

/// The index behind the view.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct IndexSummary {
    /// When the index was built, Unix milliseconds.
    pub built_at_ms: u64,
    /// Where it came from.
    pub source: IndexSource,
}

/// Counts over what the view lists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Totals {
    /// Worlds listed.
    pub worlds: u64,
    /// Program versions listed.
    pub programs: u64,
    /// Of those, drafts.
    pub drafts: u64,
    /// Of those, published.
    pub published: u64,
    /// Of those, installed in their world.
    pub installed: u64,
    /// Of those, the active module of their world.
    pub active: u64,
}

impl Totals {
    const fn count(&mut self, program: &ProgramEntry) {
        self.programs += 1;
        match program.state {
            ProgramPublishState::Draft => self.drafts += 1,
            ProgramPublishState::Published => self.published += 1,
        }
        if program.installed {
            self.installed += 1;
        }
        if program.active {
            self.active += 1;
        }
    }
}

/// One world, as the view lists it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WorldRow {
    /// The world's id.
    pub world_id: String,
    /// Program versions the world holds that the query kept.
    pub programs: u64,
    /// Of those, drafts.
    pub drafts: u64,
    /// Of those, published.
    pub published: u64,
    /// Install events the journal records.
    pub installs: u64,
    /// The active module's hash, if any.
    pub active_module: Option<String>,
    /// Last journal activity, Unix milliseconds; `null` when the journal
    /// records no time.
    pub last_activity_ms: Option<u64>,
    /// Whether the journal ends in an unterminated line.
    pub torn_tail: bool,
    /// Announcements about this world or its programs still waiting to
    /// fire (upcoming or overdue).
    pub pending_announcements: u64,
}

/// One program version, as the view lists it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProgramRow {
    /// The world holding it.
    pub world_id: String,
    /// The program's author key, lower-case hex.
    pub author: String,
    /// The program's name.
    pub name: String,
    /// The version.
    pub version: u64,
    /// BLAKE3 of the payload, lower-case hex.
    pub content_hash: String,
    /// Draft or published.
    pub state: ProgramPublishState,
    /// The bundle's description, if any.
    pub description: Option<String>,
    /// The bundle's tags.
    pub tags: Vec<String>,
    /// Whether the world installed it.
    pub installed: bool,
    /// Whether it is the world's active module.
    pub active: bool,
    /// Announcements about this version still waiting to fire.
    pub pending_announcements: u64,
}

/// A group of programs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProgramGroup {
    /// The group's key: a world id, a state name or a tag.
    pub key: String,
    /// The programs in it, in the query's order.
    pub programs: Vec<ProgramRow>,
}

/// What an announcement is about, flattened for display.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SubjectRow {
    /// `world` or `program`.
    pub kind: &'static str,
    /// The world.
    pub world_id: String,
    /// The program's name, for a program subject.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub program: Option<String>,
    /// The program's version, for a program subject.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<u64>,
}

/// One announcement, as the view lists it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AnnouncementRow {
    /// The announcement's id.
    pub id: String,
    /// What it is about.
    pub subject: SubjectRow,
    /// The notice text.
    pub message: String,
    /// When it is or was due, Unix milliseconds.
    pub due_at_ms: u64,
    /// Where it stands: `scheduled`, `cancelled`, `skipped`, `fired` or
    /// `delivered`.
    pub state: &'static str,
    /// When it was cancelled, skipped or fired.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at_ms: Option<u64>,
    /// How long after its due time it fired.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub late_by_ms: Option<u64>,
    /// Whether it fired outside the on-time grace.
    pub late: bool,
    /// When the channel accepted it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivered_at_ms: Option<u64>,
    /// Why it was skipped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl AnnouncementRow {
    fn from_entry(entry: &AnnouncementEntry) -> Self {
        let subject = match &entry.announcement.subject {
            AnnouncementSubject::World { world_id } => SubjectRow {
                kind: "world",
                world_id: world_id.clone(),
                program: None,
                version: None,
            },
            AnnouncementSubject::Program { world_id, program } => SubjectRow {
                kind: "program",
                world_id: world_id.clone(),
                program: Some(program.name.clone()),
                version: Some(program.version),
            },
        };
        let mut row = Self {
            id: entry.id.clone(),
            subject,
            message: entry.announcement.message.clone(),
            due_at_ms: entry.announcement.due_at_ms,
            state: "scheduled",
            at_ms: None,
            late_by_ms: None,
            late: entry.state.is_late(),
            delivered_at_ms: None,
            reason: None,
        };
        match &entry.state {
            AnnouncementState::Scheduled => {}
            AnnouncementState::Cancelled { at_ms } => {
                row.state = "cancelled";
                row.at_ms = Some(*at_ms);
            }
            AnnouncementState::Skipped { at_ms, reason } => {
                row.state = "skipped";
                row.at_ms = Some(*at_ms);
                row.reason = Some(reason.clone());
            }
            AnnouncementState::Fired {
                fired_at_ms,
                late_by_ms,
                delivered_at_ms,
            } => {
                row.state = if delivered_at_ms.is_some() {
                    "delivered"
                } else {
                    "fired"
                };
                row.at_ms = Some(*fired_at_ms);
                row.late_by_ms = Some(*late_by_ms);
                row.delivered_at_ms = *delivered_at_ms;
            }
        }
        row
    }

    /// The time it last moved, for ordering finished announcements.
    fn last_moved_ms(&self) -> u64 {
        self.delivered_at_ms
            .or(self.at_ms)
            .unwrap_or(self.due_at_ms)
    }

    fn describe_subject(&self) -> String {
        match (&self.subject.program, self.subject.version) {
            (Some(name), Some(version)) => {
                format!("{name} v{version} in {}", self.subject.world_id)
            }
            _ => format!("world {}", self.subject.world_id),
        }
    }
}

/// The schedule, as the view lists it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ScheduleSection {
    /// Scheduled and not yet due, soonest first.
    pub upcoming: Vec<AnnouncementRow>,
    /// Scheduled and past due: the author's node has not evaluated the
    /// schedule since, and fires these late on its next evaluation.
    pub overdue: Vec<AnnouncementRow>,
    /// Fired, waiting for the delivery channel to accept them.
    pub delivery_pending: Vec<AnnouncementRow>,
    /// Cancelled, skipped or delivered, most recent first, up to the
    /// request's limit.
    pub recent: Vec<AnnouncementRow>,
    /// Finished announcements left out by the limit.
    pub recent_omitted: u64,
    /// How many fired outside the on-time grace, across the whole
    /// schedule the query kept.
    pub fired_late: u64,
}

/// A section that is either present or unavailable for a stated reason.
/// Serializes as `{"value": v}` or `{"value": null, "reason": "…"}`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Section<T> {
    /// The section.
    Known(T),
    /// No section, and why.
    Unavailable(ViewUnavailable),
}

impl<T> Section<T> {
    /// The section when present.
    #[must_use]
    pub const fn value(&self) -> Option<&T> {
        match self {
            Self::Known(value) => Some(value),
            Self::Unavailable(_) => None,
        }
    }
}

impl<T: Serialize> Serialize for Section<T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        match self {
            Self::Known(value) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("value", value)?;
                map.end()
            }
            Self::Unavailable(reason) => {
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("value", &None::<()>)?;
                map.serialize_entry("reason", reason)?;
                map.end()
            }
        }
    }
}

/// The organization view.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OrganizationView {
    /// [`ORG_VIEW_SCHEMA_VERSION`].
    pub schema_version: u32,
    /// The author, lower-case hex Ed25519 key.
    pub author: String,
    /// The view's clock, Unix milliseconds.
    pub generated_at_ms: u64,
    /// How the programs are grouped.
    pub grouping: Grouping,
    /// The index behind the view.
    pub index: IndexSummary,
    /// Counts over what is listed.
    pub totals: Totals,
    /// The author's worlds, in world id order.
    pub worlds: Vec<WorldRow>,
    /// The program versions, grouped.
    pub groups: Vec<ProgramGroup>,
    /// The announcement schedule.
    pub announcements: Section<ScheduleSection>,
}

impl OrganizationView {
    /// Build the view over `index` (and where it came from, `status`) and
    /// the author's announcement schedule when loaded.
    #[must_use]
    pub fn build(
        index: &AuthorIndex,
        status: &IndexStatus,
        schedule: Option<&AnnouncementLog>,
        request: &ViewRequest,
    ) -> Self {
        let world_filter = request.query.world_id.as_deref();
        let in_scope = |world_id: &str| world_filter.is_none_or(|w| w == world_id);
        let programs = index.search(&request.query);

        let entries: Vec<&AnnouncementEntry> = schedule
            .map(|log| {
                log.entries()
                    .into_iter()
                    .filter(|e| in_scope(e.announcement.subject.world_id()))
                    .collect()
            })
            .unwrap_or_default();
        let mut pending_by_world: BTreeMap<&str, u64> = BTreeMap::new();
        let mut pending_by_program: BTreeMap<ProgramKey, u64> = BTreeMap::new();
        for entry in entries
            .iter()
            .filter(|e| matches!(e.state, AnnouncementState::Scheduled))
        {
            let subject = &entry.announcement.subject;
            *pending_by_world.entry(subject.world_id()).or_default() += 1;
            if let AnnouncementSubject::Program { world_id, program } = subject {
                *pending_by_program
                    .entry(program_key(world_id, program))
                    .or_default() += 1;
            }
        }

        let row = |p: &ProgramEntry| ProgramRow {
            world_id: p.world_id.clone(),
            author: p.program.author.clone(),
            name: p.program.name.clone(),
            version: p.program.version,
            content_hash: p.program.content_hash.clone(),
            state: p.state,
            description: p.description.clone(),
            tags: p.tags.clone(),
            installed: p.installed,
            active: p.active,
            pending_announcements: pending_by_program
                .get(&program_key(&p.world_id, &p.program))
                .copied()
                .unwrap_or(0),
        };

        let mut totals = Totals::default();
        let mut per_world: BTreeMap<&str, Totals> = BTreeMap::new();
        for program in &programs {
            totals.count(program);
            per_world
                .entry(program.world_id.as_str())
                .or_default()
                .count(program);
        }

        let worlds: Vec<WorldRow> = index
            .worlds
            .iter()
            .filter(|w| in_scope(&w.world_id))
            .map(|w| {
                let counts = per_world
                    .get(w.world_id.as_str())
                    .copied()
                    .unwrap_or_default();
                WorldRow {
                    world_id: w.world_id.clone(),
                    programs: counts.programs,
                    drafts: counts.drafts,
                    published: counts.published,
                    installs: w.installs,
                    active_module: w.active_module.clone(),
                    last_activity_ms: w.last_activity_ms,
                    torn_tail: w.torn_tail,
                    pending_announcements: pending_by_world
                        .get(w.world_id.as_str())
                        .copied()
                        .unwrap_or(0),
                }
            })
            .collect();
        totals.worlds = u64::try_from(worlds.len()).unwrap_or(u64::MAX);

        let groups = group_programs(&programs, request.grouping, &row);

        let announcements = schedule.map_or(
            Section::Unavailable(ViewUnavailable::ScheduleNotLoaded),
            |_| Section::Known(schedule_section(&entries, request)),
        );

        Self {
            schema_version: ORG_VIEW_SCHEMA_VERSION,
            author: index.author.clone(),
            generated_at_ms: request.now_ms,
            grouping: request.grouping,
            index: IndexSummary {
                built_at_ms: index.built_at_ms,
                source: IndexSource::from_status(status),
            },
            totals,
            worlds,
            groups,
            announcements,
        }
    }

    /// The view as pretty-printed JSON.
    ///
    /// # Errors
    ///
    /// The view cannot be encoded.
    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string_pretty(self)
    }

    /// The view as plain text.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        let t = &self.totals;
        let _ = writeln!(
            out,
            "Organization view for author {} at {} (schema {})",
            short(&self.author),
            self.generated_at_ms,
            self.schema_version
        );
        let _ = writeln!(
            out,
            "Index built at {}: {}",
            self.index.built_at_ms,
            self.index.source.describe()
        );
        let _ = writeln!(
            out,
            "{} {}, {} program {} ({} draft, {} published; {} installed, {} active)",
            t.worlds,
            plural(t.worlds, "world", "worlds"),
            t.programs,
            plural(t.programs, "version", "versions"),
            t.drafts,
            t.published,
            t.installed,
            t.active
        );

        out.push_str("\nWorlds\n");
        if self.worlds.is_empty() {
            out.push_str("  none\n");
        }
        for w in &self.worlds {
            let activity = w.last_activity_ms.map_or_else(
                || "no recorded activity".to_owned(),
                |t| format!("last activity {t}"),
            );
            let _ = write!(
                out,
                "  {}: {} {} ({} draft, {} published), {} {}, {activity}",
                w.world_id,
                w.programs,
                plural(w.programs, "program", "programs"),
                w.drafts,
                w.published,
                w.installs,
                plural(w.installs, "install", "installs"),
            );
            if w.pending_announcements > 0 {
                let _ = write!(out, ", {} pending announcement(s)", w.pending_announcements);
            }
            if w.torn_tail {
                out.push_str(", journal ends in a torn line");
            }
            out.push('\n');
        }

        let heading = match self.grouping {
            Grouping::World => "Programs by world",
            Grouping::State => "Programs by state",
            Grouping::Tag => "Programs by tag",
        };
        let _ = writeln!(out, "\n{heading}");
        if self.groups.is_empty() {
            out.push_str("  none\n");
        }
        for group in &self.groups {
            let _ = writeln!(out, "  {}", group.key);
            for p in &group.programs {
                let _ = write!(
                    out,
                    "    {} v{} [{}]",
                    p.name,
                    p.version,
                    state_name(p.state)
                );
                if self.grouping != Grouping::World {
                    let _ = write!(out, " in {}", p.world_id);
                }
                if p.active {
                    out.push_str(", active");
                } else if p.installed {
                    out.push_str(", installed");
                }
                if !p.tags.is_empty() {
                    let _ = write!(out, ", tags: {}", p.tags.join(", "));
                }
                if p.pending_announcements > 0 {
                    let _ = write!(out, ", {} pending announcement(s)", p.pending_announcements);
                }
                out.push('\n');
            }
        }

        out.push_str("\nAnnouncements\n");
        match &self.announcements {
            Section::Unavailable(reason) => {
                let _ = writeln!(out, "  unavailable: {}", reason.describe());
            }
            Section::Known(schedule) => write_schedule(&mut out, schedule),
        }
        out
    }
}

/// Refuse a view JSON whose major schema version this reader does not
/// know. Added fields are fine; a missing or other version is not.
///
/// # Errors
///
/// The version found (or `None` when absent or not a number).
pub fn check_schema_version(view: &serde_json::Value) -> Result<u32, Option<u64>> {
    match view
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
    {
        Some(v) if v == u64::from(ORG_VIEW_SCHEMA_VERSION) => Ok(ORG_VIEW_SCHEMA_VERSION),
        other => Err(other),
    }
}

/// A program version in a world: world id, name, version, content hash.
type ProgramKey = (String, String, u64, String);

fn program_key(world_id: &str, program: &ProgramIdentity) -> ProgramKey {
    (
        world_id.to_owned(),
        program.name.clone(),
        program.version,
        program.content_hash.clone(),
    )
}

fn group_programs(
    programs: &[&ProgramEntry],
    grouping: Grouping,
    row: &impl Fn(&ProgramEntry) -> ProgramRow,
) -> Vec<ProgramGroup> {
    let mut groups: BTreeMap<String, Vec<ProgramRow>> = BTreeMap::new();
    for program in programs {
        match grouping {
            Grouping::World => groups
                .entry(program.world_id.clone())
                .or_default()
                .push(row(program)),
            Grouping::State => groups
                .entry(state_name(program.state).to_owned())
                .or_default()
                .push(row(program)),
            Grouping::Tag if program.tags.is_empty() => groups
                .entry(UNTAGGED_GROUP.to_owned())
                .or_default()
                .push(row(program)),
            Grouping::Tag => {
                for tag in &program.tags {
                    groups.entry(tag.clone()).or_default().push(row(program));
                }
            }
        }
    }
    let mut groups: Vec<ProgramGroup> = groups
        .into_iter()
        .map(|(key, programs)| ProgramGroup { key, programs })
        .collect();
    if grouping == Grouping::Tag
        && let Some(at) = groups.iter().position(|g| g.key == UNTAGGED_GROUP)
    {
        let untagged = groups.remove(at);
        groups.push(untagged);
    }
    groups
}

fn schedule_section(entries: &[&AnnouncementEntry], request: &ViewRequest) -> ScheduleSection {
    let mut section = ScheduleSection::default();
    let mut finished = Vec::new();
    for entry in entries {
        let row = AnnouncementRow::from_entry(entry);
        if row.late {
            section.fired_late += 1;
        }
        match (&entry.state, row.delivered_at_ms) {
            (AnnouncementState::Scheduled, _) if row.due_at_ms >= request.now_ms => {
                section.upcoming.push(row);
            }
            (AnnouncementState::Scheduled, _) => section.overdue.push(row),
            (AnnouncementState::Fired { .. }, None) => section.delivery_pending.push(row),
            _ => finished.push(row),
        }
    }
    // `entries` arrive in due-time order, which is the order upcoming,
    // overdue and pending want; finished ones go most recent first.
    finished.sort_by(|a, b| {
        b.last_moved_ms()
            .cmp(&a.last_moved_ms())
            .then_with(|| a.id.cmp(&b.id))
    });
    let omitted = finished.len().saturating_sub(request.recent_limit);
    finished.truncate(request.recent_limit);
    section.recent = finished;
    section.recent_omitted = u64::try_from(omitted).unwrap_or(u64::MAX);
    section
}

fn write_schedule(out: &mut String, schedule: &ScheduleSection) {
    let parts: [(&str, &[AnnouncementRow]); 4] = [
        ("Upcoming", &schedule.upcoming),
        (
            "Overdue (fires late on the next evaluation)",
            &schedule.overdue,
        ),
        ("Fired, delivery pending", &schedule.delivery_pending),
        ("Recent", &schedule.recent),
    ];
    let mut any = false;
    for (heading, rows) in parts {
        if rows.is_empty() {
            continue;
        }
        any = true;
        let _ = writeln!(out, "  {heading}");
        for row in rows {
            let _ = write!(
                out,
                "    due {} {}: {:?} [{}",
                row.due_at_ms,
                row.describe_subject(),
                row.message,
                row.state
            );
            if let Some(late_by) = row.late_by_ms.filter(|_| row.late) {
                let _ = write!(out, ", {late_by} ms late");
            }
            if let Some(reason) = &row.reason {
                let _ = write!(out, ": {reason}");
            }
            out.push_str("]\n");
        }
    }
    if schedule.recent_omitted > 0 {
        let _ = writeln!(
            out,
            "  {} older finished announcement(s) not shown",
            schedule.recent_omitted
        );
    }
    if !any {
        out.push_str("  none\n");
    }
}

const fn state_name(state: ProgramPublishState) -> &'static str {
    match state {
        ProgramPublishState::Draft => "draft",
        ProgramPublishState::Published => "published",
    }
}

const fn plural<'a>(n: u64, one: &'a str, many: &'a str) -> &'a str {
    if n == 1 { one } else { many }
}

fn short(key: &str) -> &str {
    key.get(..12).unwrap_or(key)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::creator_announcements::{
        Announcement, AnnouncementDelivery, NoDelivery, PublishedSubjects,
    };
    use crate::creator_index::{IndexOrder, ProgramEntry, Staleness, WorldEntry};

    const AUTHOR: &str = "aa11bb22cc33dd44ee55ff66aa11bb22cc33dd44ee55ff66aa11bb22cc33dd44";
    const T0: u64 = 1_800_000_000_000;
    const MINUTE: u64 = 60_000;

    fn world(id: &str, last_activity_ms: Option<u64>, installs: u64) -> WorldEntry {
        WorldEntry {
            world_id: id.to_owned(),
            journal_bytes: 100,
            journal_entries: 4,
            journal_version: 12,
            torn_tail: false,
            installs,
            active_module: None,
            last_activity_ms,
        }
    }

    fn identity(name: &str, version: u64) -> ProgramIdentity {
        ProgramIdentity {
            author: AUTHOR.to_owned(),
            name: name.to_owned(),
            version,
            content_hash: format!("{:064x}", version * 31 + u64::try_from(name.len()).unwrap()),
        }
    }

    fn program(
        world_id: &str,
        name: &str,
        version: u64,
        state: ProgramPublishState,
        tags: &[&str],
    ) -> ProgramEntry {
        ProgramEntry {
            world_id: world_id.to_owned(),
            program: identity(name, version),
            state,
            description: Some(format!("{name} for {world_id}")),
            tags: tags.iter().map(|t| (*t).to_owned()).collect(),
            installed: version == 2,
            active: version == 2 && name == "trivia",
        }
    }

    fn index() -> AuthorIndex {
        AuthorIndex {
            schema_version: crate::creator_index::AUTHOR_INDEX_SCHEMA_VERSION,
            author: AUTHOR.to_owned(),
            built_at_ms: T0 - MINUTE,
            worlds: vec![
                world("gallery", None, 0),
                world("quiz-hall", Some(T0 - 5 * MINUTE), 2),
            ],
            programs: vec![
                program(
                    "quiz-hall",
                    "trivia",
                    1,
                    ProgramPublishState::Published,
                    &["games", "trivia"],
                ),
                program(
                    "quiz-hall",
                    "trivia",
                    2,
                    ProgramPublishState::Draft,
                    &["games", "trivia"],
                ),
                program("gallery", "frames", 1, ProgramPublishState::Published, &[]),
            ],
        }
    }

    struct Everything;

    impl PublishedSubjects for Everything {
        fn is_published(&self, _subject: &AnnouncementSubject) -> bool {
            true
        }
    }

    struct Accept(u64);

    impl AnnouncementDelivery for Accept {
        fn deliver(&mut self, _id: &str, _announcement: &Announcement) -> Result<(), String> {
            self.0 += 1;
            Ok(())
        }
    }

    fn about_trivia_v1() -> AnnouncementSubject {
        AnnouncementSubject::Program {
            world_id: "quiz-hall".to_owned(),
            program: identity("trivia", 1),
        }
    }

    fn about_gallery() -> AnnouncementSubject {
        AnnouncementSubject::World {
            world_id: "gallery".to_owned(),
        }
    }

    /// A schedule with one announcement in every state, evaluated by a
    /// node that went offline for an hour.
    fn schedule() -> AnnouncementLog {
        let mut log = AnnouncementLog::new();
        let s = &Everything;
        log.schedule(about_trivia_v1(), "trivia night", T0 + MINUTE, s, T0, None)
            .unwrap();
        log.schedule(about_gallery(), "new frames", T0 + 2 * MINUTE, s, T0, None)
            .unwrap();
        let cancelled = log
            .schedule(about_gallery(), "never mind", T0 + 3 * MINUTE, s, T0, None)
            .unwrap();
        log.cancel(&cancelled, T0 + 10, None).unwrap();
        log.schedule(about_trivia_v1(), "finals", T0 + 600 * MINUTE, s, T0, None)
            .unwrap();
        log.schedule(about_gallery(), "late show", T0 + 90 * MINUTE, s, T0, None)
            .unwrap();
        // Offline until an hour past the first two: both fire late, the
        // channel is down, then it comes back for the next evaluation.
        log.evaluate(T0 + 61 * MINUTE, s, &mut NoDelivery, None)
            .unwrap();
        let mut channel = Accept(0);
        log.evaluate(T0 + 62 * MINUTE, s, &mut channel, None)
            .unwrap();
        assert_eq!(channel.0, 2);
        log
    }

    #[test]
    fn the_view_groups_by_world_and_counts_what_it_lists() {
        let view =
            OrganizationView::build(&index(), &IndexStatus::Fresh, None, &ViewRequest::new(T0));
        assert_eq!(view.schema_version, ORG_VIEW_SCHEMA_VERSION);
        assert_eq!(view.author, AUTHOR);
        assert_eq!(
            view.totals,
            Totals {
                worlds: 2,
                programs: 3,
                drafts: 1,
                published: 2,
                installed: 1,
                active: 1,
            }
        );
        let keys: Vec<&str> = view.groups.iter().map(|g| g.key.as_str()).collect();
        assert_eq!(keys, ["gallery", "quiz-hall"]);
        let quiz = &view.groups[1].programs;
        assert_eq!(
            quiz.iter().map(|p| p.version).collect::<Vec<_>>(),
            [1, 2],
            "within a group the query's order holds"
        );
        let hall = view
            .worlds
            .iter()
            .find(|w| w.world_id == "quiz-hall")
            .unwrap();
        assert_eq!((hall.programs, hall.drafts, hall.published), (2, 1, 1));
        assert_eq!(hall.last_activity_ms, Some(T0 - 5 * MINUTE));
        let gallery = view
            .worlds
            .iter()
            .find(|w| w.world_id == "gallery")
            .unwrap();
        assert_eq!(gallery.last_activity_ms, None);
    }

    #[test]
    fn grouping_by_state_and_by_tag() {
        let mut request = ViewRequest::new(T0);
        request.grouping = Grouping::State;
        let view = OrganizationView::build(&index(), &IndexStatus::Fresh, None, &request);
        let keys: Vec<&str> = view.groups.iter().map(|g| g.key.as_str()).collect();
        assert_eq!(keys, ["draft", "published"]);
        assert_eq!(view.groups[0].programs.len(), 1);
        assert_eq!(view.groups[1].programs.len(), 2);

        request.grouping = Grouping::Tag;
        let view = OrganizationView::build(&index(), &IndexStatus::Fresh, None, &request);
        let keys: Vec<&str> = view.groups.iter().map(|g| g.key.as_str()).collect();
        assert_eq!(
            keys,
            ["games", "trivia", UNTAGGED_GROUP],
            "a program is listed under each tag, untagged last"
        );
        assert_eq!(view.groups[0].programs.len(), 2);
        assert_eq!(view.groups[2].programs[0].name, "frames");
        assert_eq!(view.totals.programs, 3, "tags do not double-count");
    }

    #[test]
    fn a_query_narrows_programs_worlds_and_schedule() {
        let log = schedule();
        let mut request = ViewRequest::new(T0 + 62 * MINUTE);
        request.query = IndexQuery {
            world_id: Some("gallery".to_owned()),
            ..IndexQuery::default()
        };
        let view = OrganizationView::build(&index(), &IndexStatus::Fresh, Some(&log), &request);
        assert_eq!(view.worlds.len(), 1);
        assert_eq!(view.totals.worlds, 1);
        assert_eq!(view.totals.programs, 1);
        let schedule = view.announcements.value().unwrap();
        let all: Vec<&AnnouncementRow> = schedule
            .upcoming
            .iter()
            .chain(&schedule.overdue)
            .chain(&schedule.delivery_pending)
            .chain(&schedule.recent)
            .collect();
        assert!(all.iter().all(|r| r.subject.world_id == "gallery"));
        assert_eq!(all.len(), 3);

        request.query = IndexQuery {
            state: Some(ProgramPublishState::Draft),
            order: IndexOrder::Name,
            ..IndexQuery::default()
        };
        let view = OrganizationView::build(&index(), &IndexStatus::Fresh, None, &request);
        assert_eq!(view.totals.programs, 1);
        assert_eq!(view.totals.worlds, 2, "a state filter keeps every world");
        let gallery = view
            .worlds
            .iter()
            .find(|w| w.world_id == "gallery")
            .unwrap();
        assert_eq!(gallery.programs, 0);
    }

    #[test]
    fn the_schedule_splits_into_upcoming_overdue_pending_and_recent() {
        let log = schedule();
        let view = OrganizationView::build(
            &index(),
            &IndexStatus::Fresh,
            Some(&log),
            &ViewRequest::new(T0 + 62 * MINUTE),
        );
        let s = view.announcements.value().unwrap();
        let messages = |rows: &[AnnouncementRow]| -> Vec<String> {
            rows.iter().map(|r| r.message.clone()).collect()
        };
        assert_eq!(messages(&s.upcoming), ["late show", "finals"]);
        assert_eq!(s.overdue, Vec::new());
        assert_eq!(s.delivery_pending, Vec::new());
        // Most recently moved first: both deliveries happened in the same
        // evaluation, then the early cancel.
        assert_eq!(s.recent.len(), 3);
        let mut delivered_first = messages(&s.recent[..2]);
        delivered_first.sort();
        assert_eq!(delivered_first, ["new frames", "trivia night"]);
        assert_eq!(s.recent[2].message, "never mind");
        assert_eq!(s.fired_late, 2);
        let delivered: Vec<&AnnouncementRow> =
            s.recent.iter().filter(|r| r.state == "delivered").collect();
        assert_eq!(delivered.len(), 2);
        assert!(delivered.iter().all(|r| r.late && r.late_by_ms.is_some()));
        let cancelled = s.recent.iter().find(|r| r.state == "cancelled").unwrap();
        assert_eq!(cancelled.at_ms, Some(T0 + 10));
        assert_eq!(s.recent.last().unwrap().state, "cancelled");

        // Pending counts reach the world and the program rows.
        let hall = view
            .worlds
            .iter()
            .find(|w| w.world_id == "quiz-hall")
            .unwrap();
        assert_eq!(hall.pending_announcements, 1);
        let v1 = &view
            .groups
            .iter()
            .find(|g| g.key == "quiz-hall")
            .unwrap()
            .programs[0];
        assert_eq!((v1.version, v1.pending_announcements), (1, 1));
    }

    #[test]
    fn an_unevaluated_schedule_shows_overdue_and_pending_delivery() {
        let mut log = AnnouncementLog::new();
        let s = &Everything;
        log.schedule(about_gallery(), "missed", T0 + MINUTE, s, T0, None)
            .unwrap();
        log.schedule(about_trivia_v1(), "queued", T0 + 2 * MINUTE, s, T0, None)
            .unwrap();
        log.evaluate(T0 + 2 * MINUTE, s, &mut NoDelivery, None)
            .unwrap();
        log.schedule(
            about_gallery(),
            "asleep",
            T0 + 3 * MINUTE,
            s,
            T0 + 2 * MINUTE,
            None,
        )
        .unwrap();
        // Viewed after "asleep" came due, without an evaluation since.
        let view = OrganizationView::build(
            &index(),
            &IndexStatus::Fresh,
            Some(&log),
            &ViewRequest::new(T0 + 10 * MINUTE),
        );
        let s = view.announcements.value().unwrap();
        assert_eq!(s.upcoming, Vec::new());
        assert_eq!(s.overdue.len(), 1);
        assert_eq!(s.overdue[0].message, "asleep");
        assert_eq!(s.delivery_pending.len(), 2);
        assert!(s.delivery_pending.iter().all(|r| r.state == "fired"));
        let text = view.to_text();
        assert!(text.contains("Overdue (fires late on the next evaluation)"));
        assert!(text.contains("Fired, delivery pending"));
    }

    #[test]
    fn the_recent_list_is_bounded_and_says_how_many_it_left_out() {
        let log = schedule();
        let mut request = ViewRequest::new(T0 + 62 * MINUTE);
        request.recent_limit = 1;
        let view = OrganizationView::build(&index(), &IndexStatus::Fresh, Some(&log), &request);
        let s = view.announcements.value().unwrap();
        assert_eq!(s.recent.len(), 1);
        assert_eq!(s.recent_omitted, 2);
        assert!(
            view.to_text()
                .contains("2 older finished announcement(s) not shown")
        );
    }

    #[test]
    fn an_unloaded_schedule_is_unavailable_not_empty() {
        let view =
            OrganizationView::build(&index(), &IndexStatus::Fresh, None, &ViewRequest::new(T0));
        let json: serde_json::Value = serde_json::from_str(&view.to_json().unwrap()).unwrap();
        assert_eq!(
            json["announcements"],
            serde_json::json!({"value": null, "reason": "schedule_not_loaded"})
        );
        assert!(
            view.to_text()
                .contains("unavailable: the announcement schedule was not loaded")
        );

        let empty = AnnouncementLog::new();
        let view = OrganizationView::build(
            &index(),
            &IndexStatus::Fresh,
            Some(&empty),
            &ViewRequest::new(T0),
        );
        let json: serde_json::Value = serde_json::from_str(&view.to_json().unwrap()).unwrap();
        assert_eq!(
            json["announcements"]["value"]["upcoming"],
            serde_json::json!([])
        );
        assert!(json["announcements"].get("reason").is_none());
    }

    #[test]
    fn the_index_source_is_labelled() {
        let stale = IndexStatus::Rebuilt(RebuildReason::Stale(Staleness {
            changed: vec!["quiz-hall".to_owned()],
            added: vec!["new-world".to_owned()],
            removed: Vec::new(),
        }));
        let view = OrganizationView::build(&index(), &stale, None, &ViewRequest::new(T0));
        let json: serde_json::Value = serde_json::from_str(&view.to_json().unwrap()).unwrap();
        assert_eq!(
            json["index"],
            serde_json::json!({
                "built_at_ms": T0 - MINUTE,
                "source": {
                    "status": "rebuilt",
                    "reason": "stale",
                    "changed": ["quiz-hall"],
                    "added": ["new-world"]
                }
            })
        );
        assert!(
            view.to_text()
                .contains("rebuilt (the stored index was stale: 1 changed, 1 added, 0 removed)")
        );

        for (reason, name) in [
            (RebuildReason::Missing, "missing"),
            (RebuildReason::Unreadable, "unreadable"),
            (RebuildReason::OtherAuthor, "other_author"),
        ] {
            let view = OrganizationView::build(
                &index(),
                &IndexStatus::Rebuilt(reason),
                None,
                &ViewRequest::new(T0),
            );
            let json: serde_json::Value = serde_json::from_str(&view.to_json().unwrap()).unwrap();
            assert_eq!(
                json["index"]["source"],
                serde_json::json!({"status": "rebuilt", "reason": name})
            );
        }
        let fresh =
            OrganizationView::build(&index(), &IndexStatus::Fresh, None, &ViewRequest::new(T0));
        assert!(fresh.to_text().contains(": fresh\n"));
    }

    #[test]
    fn the_json_carries_its_schema_version_and_readers_check_it() {
        let view = OrganizationView::build(
            &index(),
            &IndexStatus::Fresh,
            Some(&schedule()),
            &ViewRequest::new(T0 + 62 * MINUTE),
        );
        let mut json: serde_json::Value = serde_json::from_str(&view.to_json().unwrap()).unwrap();
        assert_eq!(check_schema_version(&json), Ok(ORG_VIEW_SCHEMA_VERSION));
        assert_eq!(json["grouping"], "world");
        assert_eq!(json["groups"][1]["programs"][0]["state"], "published");
        assert_eq!(json["groups"][1]["programs"][1]["state"], "draft");

        json["added_later"] = serde_json::json!(true);
        assert_eq!(
            check_schema_version(&json),
            Ok(ORG_VIEW_SCHEMA_VERSION),
            "added fields are fine"
        );
        json["schema_version"] = serde_json::json!(2);
        assert_eq!(check_schema_version(&json), Err(Some(2)));
        json.as_object_mut().unwrap().remove("schema_version");
        assert_eq!(check_schema_version(&json), Err(None));
    }

    #[test]
    fn the_text_view_reads_as_a_report() {
        let view = OrganizationView::build(
            &index(),
            &IndexStatus::Fresh,
            Some(&schedule()),
            &ViewRequest::new(T0 + 62 * MINUTE),
        );
        let text = view.to_text();
        assert!(text.starts_with("Organization view for author aa11bb22cc33 at "));
        assert!(text.contains(
            "2 worlds, 3 program versions (1 draft, 2 published; 1 installed, 1 active)"
        ));
        assert!(text.contains(
            "  gallery: 1 program (0 draft, 1 published), 0 installs, no recorded activity"
        ));
        assert!(text.contains("    trivia v2 [draft], active, tags: games, trivia\n"));
        assert!(text.contains("    frames v1 [published]\n"));
        assert!(text.contains("  Upcoming\n"));
        assert!(text.contains("trivia v1 in quiz-hall: \"trivia night\" [delivered, "));
        assert!(text.contains("ms late]"));
    }

    #[test]
    fn an_empty_index_still_renders() {
        let empty = AuthorIndex {
            schema_version: crate::creator_index::AUTHOR_INDEX_SCHEMA_VERSION,
            author: AUTHOR.to_owned(),
            built_at_ms: T0,
            worlds: Vec::new(),
            programs: Vec::new(),
        };
        let view = OrganizationView::build(
            &empty,
            &IndexStatus::Rebuilt(RebuildReason::Missing),
            Some(&AnnouncementLog::new()),
            &ViewRequest::new(T0),
        );
        assert_eq!(view.totals, Totals::default());
        assert_eq!(view.groups, [] as [ProgramGroup; 0]);
        let text = view.to_text();
        assert!(text.contains("0 worlds, 0 program versions"));
        assert!(text.contains("Worlds\n  none\n"));
        assert!(text.contains("Announcements\n  none\n"));
    }
}
