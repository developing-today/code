//! Scheduled announcement record and local evaluator (#13743).
//!
//! Slice 3 of the creator cross-world v2 outline
//! (`doc/2026-11-23T21-00-00Z_design_id6309_creator_cross_world_v2_outline.md`,
//! sections 1, 2, 4 and 6 item 3). An author schedules a notice about
//! one of their published program versions or one of their own worlds;
//! the schedule is local, durable data on the author's own node, and the
//! author's node evaluates it. Nothing here runs on, or reads from,
//! infrastructure we operate.
//!
//! # What the record is, and is not
//!
//! - "Scheduled" is a property of an announcement, never of a program.
//!   Scheduling, firing or cancelling an announcement does not read or
//!   write the program's draft / published fact (#13742), so a schedule
//!   cannot change what a program *is* (outline section 2, question 2).
//! - An announcement may only be scheduled about a subject that is
//!   published at scheduling time, and it is checked again when it
//!   comes due: a subject that is no longer published is recorded as
//!   skipped, never announced.
//! - The record holds the notice and its times, nothing about any
//!   audience. Scheduled announcements add no tracking (outline
//!   section 1).
//!
//! # Firing late
//!
//! The evaluator ([`AnnouncementLog::evaluate`]) fires every
//! announcement whose due time has passed. A node that was offline at
//! the due time fires it on the next evaluation, normally the next
//! start, and records how late it was (outline section 4, question 4).
//! A late announcement is never silently dropped, and an announcement
//! is never fired before its due time.
//!
//! # Delivery
//!
//! Delivery goes through the F12.5 notify path, but whether that path
//! reaches a self-hosted node without our infrastructure is still open
//! (outline section 4, question 3). Until it is answered, delivery sits
//! behind the [`AnnouncementDelivery`] interface. [`NoDelivery`] is the
//! honest default: it refuses every delivery, so a fired announcement
//! stays recorded as fired with delivery pending, and is offered to the
//! channel again on each later evaluation until one accepts it.
//!
//! # Storage
//!
//! The log is one JSON line per event under the author's directory
//! ([`ANNOUNCEMENTS_FILE`]): scheduled, cancelled, fired, skipped and
//! delivered. The log is the only source of truth; an announcement's
//! state is replayed from it, and reloading re-checks every line's
//! sequence and every transition, so a damaged log fails loudly instead
//! of yielding a guessed schedule.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::program_bundle::{ProgramIdentity, check_identity};
use crate::program_publish_state::{self, ProgramPublishState};
use crate::world::Records;
use crate::world_session::encode_hex;

/// Domain separator for an announcement's content address.
const ANNOUNCEMENT_ID_DOMAIN: &[u8] = b"developing-today/id-creator-announcement/v1\0";

/// File, under the author's directory, holding the announcement log as
/// one JSON line per event.
pub const ANNOUNCEMENTS_FILE: &str = "announcements.jsonl";

/// The longest notice text, in bytes.
pub const MAX_MESSAGE_BYTES: usize = 2000;

/// The longest world id an announcement may name, in bytes.
pub const MAX_WORLD_ID_BYTES: usize = 256;

/// How long after its due time an announcement may fire and still count
/// as on time. Anything later is recorded as late.
pub const ON_TIME_GRACE_MS: u64 = 60 * 1000;

/// What an announcement is about.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AnnouncementSubject {
    /// One of the author's own worlds.
    World {
        /// The world's id.
        world_id: String,
    },
    /// A published program version held in one of the author's worlds.
    Program {
        /// The world whose records hold the version's publish fact.
        world_id: String,
        /// The program version (D1 identity).
        program: ProgramIdentity,
    },
}

impl AnnouncementSubject {
    /// The world the subject lives in.
    #[must_use]
    pub fn world_id(&self) -> &str {
        match self {
            Self::World { world_id } | Self::Program { world_id, .. } => world_id,
        }
    }
}

impl fmt::Display for AnnouncementSubject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::World { world_id } => write!(f, "world {world_id:?}"),
            Self::Program { world_id, program } => write!(
                f,
                "program {}/{} v{} in world {world_id:?}",
                program.author, program.name, program.version
            ),
        }
    }
}

/// One scheduled notice.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Announcement {
    /// What the notice is about.
    pub subject: AnnouncementSubject,
    /// The notice text.
    pub message: String,
    /// When it should fire, Unix milliseconds.
    pub due_at_ms: u64,
    /// When it was scheduled, Unix milliseconds on the author's clock.
    pub scheduled_at_ms: u64,
}

impl Announcement {
    /// The announcement's content address: BLAKE3 over a domain
    /// separator and its JSON encoding, lower-case hex. The same notice
    /// scheduled for the same time has the same id.
    ///
    /// # Errors
    ///
    /// The announcement cannot be encoded.
    pub fn id(&self) -> Result<String, AnnouncementError> {
        let json =
            serde_json::to_vec(self).map_err(|e| AnnouncementError::Malformed(e.to_string()))?;
        let mut hasher = blake3::Hasher::new();
        hasher.update(ANNOUNCEMENT_ID_DOMAIN);
        hasher.update(&json);
        Ok(encode_hex(hasher.finalize().as_bytes()))
    }
}

/// Why an announcement operation was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AnnouncementError {
    /// The announcement or a log line is malformed or out of bounds.
    #[error("announcement: {0}")]
    Malformed(String),
    /// The subject is not one of the author's published programs or
    /// worlds.
    #[error("{0} is not published by this author; only a published subject can be announced")]
    NotPublished(String),
    /// No announcement with this id is in the log.
    #[error("no announcement {0} in the log")]
    Unknown(String),
    /// The announcement already fired, was skipped or was cancelled, so
    /// it cannot be cancelled.
    #[error("announcement {id} is {state}; only a scheduled announcement can be cancelled")]
    NotScheduled {
        /// The announcement's id.
        id: String,
        /// The state it is in.
        state: &'static str,
    },
    /// Reading or writing the log failed.
    #[error("announcement storage: {0}")]
    Storage(String),
}

/// Which subjects the author may announce: their own worlds, and the
/// program versions recorded as published in them.
pub trait PublishedSubjects {
    /// Whether `subject` is the author's and published.
    fn is_published(&self, subject: &AnnouncementSubject) -> bool;
}

impl<S: PublishedSubjects + ?Sized> PublishedSubjects for &S {
    fn is_published(&self, subject: &AnnouncementSubject) -> bool {
        (**self).is_published(subject)
    }
}

/// The author's own worlds, with the program versions each has recorded
/// as published, read from the worlds' own records (#13742 facts).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AuthorWorlds {
    /// World id to the record keys of its published program versions.
    worlds: BTreeMap<String, BTreeSet<String>>,
}

impl AuthorWorlds {
    /// No worlds: nothing can be announced.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add one of the author's worlds, taking its published program
    /// versions from its records. A version with no fact is a draft and
    /// cannot be announced.
    ///
    /// # Errors
    ///
    /// The world id is out of bounds, or a publish-state record in the
    /// world is malformed (never read as a guessed state).
    pub fn hold(&mut self, world_id: &str, records: &Records) -> Result<(), AnnouncementError> {
        check_world_id(world_id)?;
        let facts = crate::world_records::program_publish_facts(records)
            .map_err(|e| AnnouncementError::Malformed(format!("world {world_id:?}: {e:#}")))?;
        let published = facts
            .into_iter()
            .filter(|fact| fact.state == ProgramPublishState::Published)
            .map(|fact| program_publish_state::record_key(&fact.program))
            .collect();
        self.worlds.insert(world_id.to_owned(), published);
        Ok(())
    }

    /// Stop treating a world as the author's.
    pub fn release(&mut self, world_id: &str) {
        self.worlds.remove(world_id);
    }
}

impl PublishedSubjects for AuthorWorlds {
    fn is_published(&self, subject: &AnnouncementSubject) -> bool {
        match subject {
            AnnouncementSubject::World { world_id } => self.worlds.contains_key(world_id),
            AnnouncementSubject::Program { world_id, program } => {
                self.worlds.get(world_id).is_some_and(|published| {
                    published.contains(&program_publish_state::record_key(program))
                })
            }
        }
    }
}

/// The delivery channel for fired announcements: F12.5 notify, once
/// outline question 3 is answered.
pub trait AnnouncementDelivery {
    /// Deliver one fired announcement.
    ///
    /// # Errors
    ///
    /// The channel could not take it now; the reason is reported and the
    /// announcement is offered again on the next evaluation.
    fn deliver(&mut self, id: &str, announcement: &Announcement) -> Result<(), String>;
}

/// No delivery channel is configured. Every delivery is refused, so
/// fired announcements stay recorded with delivery pending.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NoDelivery;

/// The reason [`NoDelivery`] gives.
pub const NO_DELIVERY_REASON: &str =
    "no delivery channel is configured (F12.5 notify; creator v2 outline open question 3)";

impl AnnouncementDelivery for NoDelivery {
    fn deliver(&mut self, _id: &str, _announcement: &Announcement) -> Result<(), String> {
        Err(NO_DELIVERY_REASON.to_owned())
    }
}

/// One event in the log.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
pub enum AnnouncementEvent {
    /// The author scheduled a notice.
    Scheduled {
        /// The announcement's id ([`Announcement::id`]).
        id: String,
        /// The notice.
        announcement: Announcement,
    },
    /// The author cancelled it before it fired.
    Cancelled {
        /// The announcement's id.
        id: String,
        /// When, Unix milliseconds.
        at_ms: u64,
    },
    /// The evaluator found it due and fired it.
    Fired {
        /// The announcement's id.
        id: String,
        /// When the evaluator fired it, Unix milliseconds.
        fired_at_ms: u64,
        /// How long after its due time it fired.
        late_by_ms: u64,
    },
    /// The evaluator found it due, but its subject was no longer
    /// published, so it was not announced.
    Skipped {
        /// The announcement's id.
        id: String,
        /// When, Unix milliseconds.
        at_ms: u64,
        /// Why.
        reason: String,
    },
    /// The delivery channel accepted it.
    Delivered {
        /// The announcement's id.
        id: String,
        /// When, Unix milliseconds.
        at_ms: u64,
    },
}

impl AnnouncementEvent {
    fn id(&self) -> &str {
        match self {
            Self::Scheduled { id, .. }
            | Self::Cancelled { id, .. }
            | Self::Fired { id, .. }
            | Self::Skipped { id, .. }
            | Self::Delivered { id, .. } => id,
        }
    }
}

/// One line of the log.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnnouncementLine {
    /// Position in the log, from 1.
    pub sequence: u64,
    /// The event.
    pub event: AnnouncementEvent,
}

/// Where an announcement stands, replayed from the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AnnouncementState {
    /// Waiting for its due time.
    Scheduled,
    /// Cancelled before it fired.
    Cancelled {
        /// When.
        at_ms: u64,
    },
    /// Its subject was no longer published when it came due.
    Skipped {
        /// When.
        at_ms: u64,
        /// Why.
        reason: String,
    },
    /// Fired at or after its due time.
    Fired {
        /// When it fired.
        fired_at_ms: u64,
        /// How long after its due time.
        late_by_ms: u64,
        /// When the channel accepted it, or `None` while delivery is
        /// pending.
        delivered_at_ms: Option<u64>,
    },
}

impl AnnouncementState {
    /// A short name for the state.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Scheduled => "scheduled",
            Self::Cancelled { .. } => "cancelled",
            Self::Skipped { .. } => "skipped",
            Self::Fired {
                delivered_at_ms: None,
                ..
            } => "fired, delivery pending",
            Self::Fired { .. } => "fired and delivered",
        }
    }

    /// Whether it fired more than [`ON_TIME_GRACE_MS`] after its due
    /// time.
    #[must_use]
    pub const fn is_late(&self) -> bool {
        matches!(self, Self::Fired { late_by_ms, .. } if *late_by_ms > ON_TIME_GRACE_MS)
    }
}

/// One announcement and where it stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnnouncementEntry {
    /// The announcement's id.
    pub id: String,
    /// The notice.
    pub announcement: Announcement,
    /// Where it stands.
    pub state: AnnouncementState,
}

/// What one evaluation did to one announcement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EvaluationOutcome {
    /// It fired now and the channel accepted it.
    FiredAndDelivered {
        /// The announcement's id.
        id: String,
        /// How long after its due time it fired.
        late_by_ms: u64,
    },
    /// It fired now; the channel refused it, so delivery is pending.
    FiredDeliveryPending {
        /// The announcement's id.
        id: String,
        /// How long after its due time it fired.
        late_by_ms: u64,
        /// The channel's reason.
        reason: String,
    },
    /// It had fired earlier and the channel accepted it now.
    Delivered {
        /// The announcement's id.
        id: String,
    },
    /// It had fired earlier and the channel still refused it.
    StillPending {
        /// The announcement's id.
        id: String,
        /// The channel's reason.
        reason: String,
    },
    /// Its subject was no longer published, so it was not announced.
    Skipped {
        /// The announcement's id.
        id: String,
    },
}

/// The author's announcement log: every event, and each announcement's
/// state replayed from them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AnnouncementLog {
    lines: Vec<AnnouncementLine>,
    entries: BTreeMap<String, AnnouncementEntry>,
}

impl AnnouncementLog {
    /// An empty log in memory.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Load the log under `author_dir`; a missing file is an empty log.
    /// Every line is replayed and checked.
    ///
    /// # Errors
    ///
    /// The file cannot be read, a line is malformed, the sequence is
    /// not 1, 2, 3 ..., or an event is not a valid transition.
    pub fn load(author_dir: &Path) -> Result<Self, AnnouncementError> {
        let path = author_dir.join(ANNOUNCEMENTS_FILE);
        let mut log = Self::default();
        if !path.is_file() {
            return Ok(log);
        }
        let text = std::fs::read_to_string(&path).map_err(|e| storage(&path, &e))?;
        for line in text.lines().filter(|line| !line.trim().is_empty()) {
            let line: AnnouncementLine = id_core::json_unique::from_slice_without_duplicate_keys(
                line.as_bytes(),
            )
            .map_err(|e| AnnouncementError::Malformed(format!("{}: {e}", path.display())))?;
            if line.sequence != log.next_sequence() {
                return Err(AnnouncementError::Malformed(format!(
                    "announcement log sequence {} where {} was expected",
                    line.sequence,
                    log.next_sequence()
                )));
            }
            log.apply(&line.event)?;
            log.lines.push(line);
        }
        Ok(log)
    }

    /// Every event, in sequence order.
    #[must_use]
    pub fn lines(&self) -> &[AnnouncementLine] {
        &self.lines
    }

    /// The announcement with this id, if scheduled.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&AnnouncementEntry> {
        self.entries.get(id)
    }

    /// Every announcement, ordered by due time, then id.
    #[must_use]
    pub fn entries(&self) -> Vec<&AnnouncementEntry> {
        let mut entries: Vec<_> = self.entries.values().collect();
        entries.sort_by(|a, b| {
            (a.announcement.due_at_ms, &a.id).cmp(&(b.announcement.due_at_ms, &b.id))
        });
        entries
    }

    /// How many announcements the log holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the log holds none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Schedule a notice about `subject` to fire at `due_at_ms`,
    /// writing the line under `author_dir` when given. Scheduling the
    /// same notice for the same time again returns the existing id and
    /// appends nothing.
    ///
    /// # Errors
    ///
    /// The notice is empty or out of bounds, the due time is before
    /// `now_ms`, the subject is not one the author has published, or
    /// the line cannot be written (the log in memory is then
    /// unchanged).
    pub fn schedule(
        &mut self,
        subject: AnnouncementSubject,
        message: &str,
        due_at_ms: u64,
        subjects: &impl PublishedSubjects,
        now_ms: u64,
        author_dir: Option<&Path>,
    ) -> Result<String, AnnouncementError> {
        let announcement = Announcement {
            subject,
            message: message.to_owned(),
            due_at_ms,
            scheduled_at_ms: now_ms,
        };
        check_announcement(&announcement)?;
        if !subjects.is_published(&announcement.subject) {
            return Err(AnnouncementError::NotPublished(
                announcement.subject.to_string(),
            ));
        }
        let id = announcement.id()?;
        if self.entries.contains_key(&id) {
            return Ok(id);
        }
        self.append(
            AnnouncementEvent::Scheduled {
                id: id.clone(),
                announcement,
            },
            author_dir,
        )?;
        Ok(id)
    }

    /// Cancel a scheduled announcement.
    ///
    /// # Errors
    ///
    /// No such announcement, it is no longer scheduled, or the line
    /// cannot be written.
    pub fn cancel(
        &mut self,
        id: &str,
        now_ms: u64,
        author_dir: Option<&Path>,
    ) -> Result<(), AnnouncementError> {
        self.append(
            AnnouncementEvent::Cancelled {
                id: id.to_owned(),
                at_ms: now_ms,
            },
            author_dir,
        )
    }

    /// Evaluate the log at `now_ms`: fire every scheduled announcement
    /// whose due time has passed, in due-time order, recording how late
    /// it fired; skip any whose subject is no longer published; and
    /// offer every fired, undelivered announcement to `delivery`. Run it
    /// on start, so a node that was offline fires what it missed, and
    /// then as often as the author's node likes.
    ///
    /// # Errors
    ///
    /// A line cannot be written. Events already written stay written,
    /// and the log in memory matches the file.
    pub fn evaluate(
        &mut self,
        now_ms: u64,
        subjects: &impl PublishedSubjects,
        delivery: &mut impl AnnouncementDelivery,
        author_dir: Option<&Path>,
    ) -> Result<Vec<EvaluationOutcome>, AnnouncementError> {
        let mut outcomes = Vec::new();
        let work: Vec<(String, Announcement, AnnouncementState)> = self
            .entries()
            .into_iter()
            .map(|entry| {
                (
                    entry.id.clone(),
                    entry.announcement.clone(),
                    entry.state.clone(),
                )
            })
            .collect();
        for (id, announcement, state) in work {
            match state {
                AnnouncementState::Scheduled if announcement.due_at_ms <= now_ms => {
                    if !subjects.is_published(&announcement.subject) {
                        self.append(
                            AnnouncementEvent::Skipped {
                                id: id.clone(),
                                at_ms: now_ms,
                                reason: format!("{} is no longer published", announcement.subject),
                            },
                            author_dir,
                        )?;
                        outcomes.push(EvaluationOutcome::Skipped { id });
                        continue;
                    }
                    let late_by_ms = now_ms.saturating_sub(announcement.due_at_ms);
                    self.append(
                        AnnouncementEvent::Fired {
                            id: id.clone(),
                            fired_at_ms: now_ms,
                            late_by_ms,
                        },
                        author_dir,
                    )?;
                    match delivery.deliver(&id, &announcement) {
                        Ok(()) => {
                            self.append(
                                AnnouncementEvent::Delivered {
                                    id: id.clone(),
                                    at_ms: now_ms,
                                },
                                author_dir,
                            )?;
                            outcomes.push(EvaluationOutcome::FiredAndDelivered { id, late_by_ms });
                        }
                        Err(reason) => outcomes.push(EvaluationOutcome::FiredDeliveryPending {
                            id,
                            late_by_ms,
                            reason,
                        }),
                    }
                }
                AnnouncementState::Fired {
                    delivered_at_ms: None,
                    ..
                } => match delivery.deliver(&id, &announcement) {
                    Ok(()) => {
                        self.append(
                            AnnouncementEvent::Delivered {
                                id: id.clone(),
                                at_ms: now_ms,
                            },
                            author_dir,
                        )?;
                        outcomes.push(EvaluationOutcome::Delivered { id });
                    }
                    Err(reason) => outcomes.push(EvaluationOutcome::StillPending { id, reason }),
                },
                _ => {}
            }
        }
        Ok(outcomes)
    }

    fn next_sequence(&self) -> u64 {
        self.lines
            .last()
            .map_or(1, |line| line.sequence.saturating_add(1))
    }

    /// Check `event` against the current state and write it: to the
    /// file first, when given, then to memory.
    fn append(
        &mut self,
        event: AnnouncementEvent,
        author_dir: Option<&Path>,
    ) -> Result<(), AnnouncementError> {
        let mut next = self.entries.get(event.id()).cloned();
        transition(&mut next, &event)?;
        let line = AnnouncementLine {
            sequence: self.next_sequence(),
            event,
        };
        if let Some(dir) = author_dir {
            append_line(&dir.join(ANNOUNCEMENTS_FILE), &line)?;
        }
        if let Some(entry) = next {
            self.entries.insert(entry.id.clone(), entry);
        }
        self.lines.push(line);
        Ok(())
    }

    fn apply(&mut self, event: &AnnouncementEvent) -> Result<(), AnnouncementError> {
        let mut next = self.entries.get(event.id()).cloned();
        transition(&mut next, event)?;
        if let Some(entry) = next {
            self.entries.insert(entry.id.clone(), entry);
        }
        Ok(())
    }
}

/// The one state machine both appends and reloads go through.
fn transition(
    entry: &mut Option<AnnouncementEntry>,
    event: &AnnouncementEvent,
) -> Result<(), AnnouncementError> {
    if let AnnouncementEvent::Scheduled { id, announcement } = event {
        if entry.is_some() {
            return Err(AnnouncementError::Malformed(format!(
                "announcement {id} is scheduled twice"
            )));
        }
        check_announcement(announcement)?;
        if announcement.id()? != *id {
            return Err(AnnouncementError::Malformed(format!(
                "announcement {id} does not match its content address"
            )));
        }
        *entry = Some(AnnouncementEntry {
            id: id.clone(),
            announcement: announcement.clone(),
            state: AnnouncementState::Scheduled,
        });
        return Ok(());
    }
    let Some(current) = entry.as_mut() else {
        return Err(AnnouncementError::Unknown(event.id().to_owned()));
    };
    let prior = current.state.clone();
    let state = &mut current.state;
    match (event, &prior) {
        (AnnouncementEvent::Cancelled { at_ms, .. }, AnnouncementState::Scheduled) => {
            *state = AnnouncementState::Cancelled { at_ms: *at_ms };
        }
        (AnnouncementEvent::Skipped { at_ms, reason, .. }, AnnouncementState::Scheduled) => {
            if *at_ms < current.announcement.due_at_ms {
                return Err(AnnouncementError::Malformed(format!(
                    "announcement {} skipped before its due time",
                    current.id
                )));
            }
            *state = AnnouncementState::Skipped {
                at_ms: *at_ms,
                reason: reason.clone(),
            };
        }
        (
            AnnouncementEvent::Fired {
                fired_at_ms,
                late_by_ms,
                ..
            },
            AnnouncementState::Scheduled,
        ) => {
            if fired_at_ms.checked_sub(current.announcement.due_at_ms) != Some(*late_by_ms) {
                return Err(AnnouncementError::Malformed(format!(
                    "announcement {} fired before its due time or with a wrong lateness",
                    current.id
                )));
            }
            *state = AnnouncementState::Fired {
                fired_at_ms: *fired_at_ms,
                late_by_ms: *late_by_ms,
                delivered_at_ms: None,
            };
        }
        (
            AnnouncementEvent::Delivered { at_ms, .. },
            AnnouncementState::Fired {
                fired_at_ms,
                late_by_ms,
                delivered_at_ms: None,
            },
        ) => {
            *state = AnnouncementState::Fired {
                fired_at_ms: *fired_at_ms,
                late_by_ms: *late_by_ms,
                delivered_at_ms: Some(*at_ms),
            };
        }
        (AnnouncementEvent::Cancelled { .. }, other) => {
            return Err(AnnouncementError::NotScheduled {
                id: current.id.clone(),
                state: other.name(),
            });
        }
        (_, other) => {
            return Err(AnnouncementError::Malformed(format!(
                "announcement {} cannot take that event while {}",
                current.id,
                other.name()
            )));
        }
    }
    Ok(())
}

fn check_world_id(world_id: &str) -> Result<(), AnnouncementError> {
    if world_id.is_empty() || world_id.len() > MAX_WORLD_ID_BYTES {
        return Err(AnnouncementError::Malformed(format!(
            "a world id must be 1 to {MAX_WORLD_ID_BYTES} bytes"
        )));
    }
    Ok(())
}

fn check_announcement(announcement: &Announcement) -> Result<(), AnnouncementError> {
    check_world_id(announcement.subject.world_id())?;
    if let AnnouncementSubject::Program { program, .. } = &announcement.subject {
        check_identity(program).map_err(AnnouncementError::Malformed)?;
    }
    if announcement.message.trim().is_empty() {
        return Err(AnnouncementError::Malformed(
            "the notice text is empty".to_owned(),
        ));
    }
    if announcement.message.len() > MAX_MESSAGE_BYTES {
        return Err(AnnouncementError::Malformed(format!(
            "the notice text is {} bytes; the limit is {MAX_MESSAGE_BYTES}",
            announcement.message.len()
        )));
    }
    if announcement.due_at_ms < announcement.scheduled_at_ms {
        return Err(AnnouncementError::Malformed(format!(
            "due at {} ms is before it was scheduled ({} ms)",
            announcement.due_at_ms, announcement.scheduled_at_ms
        )));
    }
    Ok(())
}

fn append_line(path: &Path, line: &AnnouncementLine) -> Result<(), AnnouncementError> {
    let dir = path
        .parent()
        .ok_or_else(|| AnnouncementError::Storage(format!("{} has no parent", path.display())))?;
    std::fs::create_dir_all(dir).map_err(|e| storage(dir, &e))?;
    let mut bytes =
        serde_json::to_vec(line).map_err(|e| AnnouncementError::Storage(e.to_string()))?;
    bytes.push(b'\n');
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| storage(path, &e))?;
    std::io::Write::write_all(&mut file, &bytes).map_err(|e| storage(path, &e))?;
    file.sync_all().map_err(|e| storage(path, &e))
}

fn storage(path: &Path, error: &std::io::Error) -> AnnouncementError {
    AnnouncementError::Storage(format!("{}: {error}", path.display()))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use ed25519_dalek::SigningKey;

    use super::*;
    use crate::program_bundle::{ProgramBundle, ProgramSpec, export};
    use crate::program_publish_state::{ProgramPublishFact, record_fact};

    const NOW: u64 = 1_795_000_000_000;
    const HOUR: u64 = 60 * 60 * 1000;

    fn author() -> SigningKey {
        SigningKey::from_bytes(&[7_u8; 32])
    }

    fn bundle(name: &str, version: u64, payload: &[u8]) -> ProgramBundle {
        export(
            payload.to_vec(),
            ProgramSpec {
                name,
                version,
                licence: None,
                description: None,
                categories: &[],
            },
            &author(),
        )
        .unwrap()
    }

    /// A world "quiz-hall" with v1 published and v2 a draft.
    fn quiz_hall() -> (Records, ProgramBundle, ProgramBundle) {
        let published = bundle("quiz", 1, b"quiz payload v1");
        let draft = bundle("quiz", 2, b"quiz payload v2");
        let mut records = Records::new();
        record_fact(
            &mut records,
            &ProgramPublishFact::published(published.identity(), published.record()).unwrap(),
        )
        .unwrap();
        record_fact(&mut records, &ProgramPublishFact::draft(draft.identity())).unwrap();
        (records, published, draft)
    }

    fn author_worlds(records: &Records) -> AuthorWorlds {
        let mut worlds = AuthorWorlds::new();
        worlds.hold("quiz-hall", records).unwrap();
        worlds
    }

    fn program_subject(bundle: &ProgramBundle) -> AnnouncementSubject {
        AnnouncementSubject::Program {
            world_id: "quiz-hall".to_owned(),
            program: bundle.identity(),
        }
    }

    /// A delivery channel that records what it took, and can be told to
    /// refuse.
    #[derive(Default)]
    struct Recorder {
        refuse: bool,
        delivered: Vec<String>,
    }

    impl AnnouncementDelivery for Recorder {
        fn deliver(&mut self, id: &str, _announcement: &Announcement) -> Result<(), String> {
            if self.refuse {
                return Err("channel down".to_owned());
            }
            self.delivered.push(id.to_owned());
            Ok(())
        }
    }

    #[test]
    fn only_published_programs_and_the_authors_own_worlds_can_be_announced() {
        let (records, published, draft) = quiz_hall();
        let worlds = author_worlds(&records);
        let mut log = AnnouncementLog::new();

        log.schedule(
            program_subject(&published),
            "quiz v1 is out",
            NOW + HOUR,
            &worlds,
            NOW,
            None,
        )
        .unwrap();
        log.schedule(
            AnnouncementSubject::World {
                world_id: "quiz-hall".to_owned(),
            },
            "quiz night on Friday",
            NOW + HOUR,
            &worlds,
            NOW,
            None,
        )
        .unwrap();

        // A draft is not published.
        assert!(matches!(
            log.schedule(
                program_subject(&draft),
                "quiz v2 is out",
                NOW + HOUR,
                &worlds,
                NOW,
                None
            ),
            Err(AnnouncementError::NotPublished(_))
        ));
        // A world that is not the author's.
        assert!(matches!(
            log.schedule(
                AnnouncementSubject::World {
                    world_id: "someone-elses".to_owned()
                },
                "hello",
                NOW + HOUR,
                &worlds,
                NOW,
                None
            ),
            Err(AnnouncementError::NotPublished(_))
        ));
        assert_eq!(log.len(), 2);
    }

    #[test]
    fn scheduling_never_touches_the_program_publish_state() {
        let (records, published, draft) = quiz_hall();
        let before = records.clone();
        let worlds = author_worlds(&records);
        let mut log = AnnouncementLog::new();
        let id = log
            .schedule(
                program_subject(&published),
                "quiz v1 is out",
                NOW + HOUR,
                &worlds,
                NOW,
                None,
            )
            .unwrap();
        log.evaluate(NOW + 2 * HOUR, &worlds, &mut NoDelivery, None)
            .unwrap();
        log.cancel(&id, NOW + 3 * HOUR, None).unwrap_err();
        assert_eq!(records, before);
        assert_eq!(
            program_publish_state::state_in(&records, &published.identity()).unwrap(),
            ProgramPublishState::Published
        );
        assert_eq!(
            program_publish_state::state_in(&records, &draft.identity()).unwrap(),
            ProgramPublishState::Draft
        );
    }

    #[test]
    fn bad_notices_are_refused() {
        let (records, published, _draft) = quiz_hall();
        let worlds = author_worlds(&records);
        let mut log = AnnouncementLog::new();
        let oversized = "x".repeat(MAX_MESSAGE_BYTES + 1);
        for (message, due) in [
            ("", NOW + HOUR),
            ("   ", NOW + HOUR),
            (oversized.as_str(), NOW + HOUR),
            ("in the past", NOW - 1),
        ] {
            assert!(matches!(
                log.schedule(
                    program_subject(&published),
                    message,
                    due,
                    &worlds,
                    NOW,
                    None
                ),
                Err(AnnouncementError::Malformed(_))
            ));
        }
        assert!(log.is_empty());
    }

    #[test]
    fn an_announcement_never_fires_before_its_due_time() {
        let (records, published, _draft) = quiz_hall();
        let worlds = author_worlds(&records);
        let mut log = AnnouncementLog::new();
        let id = log
            .schedule(
                program_subject(&published),
                "quiz v1 is out",
                NOW + HOUR,
                &worlds,
                NOW,
                None,
            )
            .unwrap();
        let mut channel = Recorder::default();
        let outcomes = log
            .evaluate(NOW + HOUR - 1, &worlds, &mut channel, None)
            .unwrap();
        assert!(outcomes.is_empty());
        assert_eq!(log.get(&id).unwrap().state, AnnouncementState::Scheduled);

        let outcomes = log
            .evaluate(NOW + HOUR, &worlds, &mut channel, None)
            .unwrap();
        assert_eq!(
            outcomes,
            vec![EvaluationOutcome::FiredAndDelivered {
                id: id.clone(),
                late_by_ms: 0
            }]
        );
        assert_eq!(channel.delivered, vec![id.clone()]);
        let state = &log.get(&id).unwrap().state;
        assert!(!state.is_late());
        assert_eq!(
            *state,
            AnnouncementState::Fired {
                fired_at_ms: NOW + HOUR,
                late_by_ms: 0,
                delivered_at_ms: Some(NOW + HOUR)
            }
        );

        // A second evaluation does nothing more.
        assert!(
            log.evaluate(NOW + 2 * HOUR, &worlds, &mut channel, None)
                .unwrap()
                .is_empty()
        );
        assert_eq!(channel.delivered.len(), 1);
    }

    #[test]
    fn a_node_offline_at_the_due_time_fires_on_next_start_and_records_lateness() {
        let dir = tempfile::tempdir().unwrap();
        let (records, published, _draft) = quiz_hall();
        let worlds = author_worlds(&records);
        let id = {
            let mut log = AnnouncementLog::load(dir.path()).unwrap();
            log.schedule(
                program_subject(&published),
                "quiz v1 is out",
                NOW + HOUR,
                &worlds,
                NOW,
                Some(dir.path()),
            )
            .unwrap()
        };

        // The node was off for a day; it starts again, loads and
        // evaluates.
        let restart = NOW + HOUR + 24 * HOUR;
        let mut log = AnnouncementLog::load(dir.path()).unwrap();
        assert_eq!(log.get(&id).unwrap().state, AnnouncementState::Scheduled);
        let mut channel = Recorder::default();
        let outcomes = log
            .evaluate(restart, &worlds, &mut channel, Some(dir.path()))
            .unwrap();
        assert_eq!(
            outcomes,
            vec![EvaluationOutcome::FiredAndDelivered {
                id: id.clone(),
                late_by_ms: 24 * HOUR
            }]
        );
        let state = log.get(&id).unwrap().state.clone();
        assert!(state.is_late());

        // The lateness is durable.
        let reloaded = AnnouncementLog::load(dir.path()).unwrap();
        assert_eq!(reloaded.get(&id).unwrap().state, state);
        assert_eq!(reloaded, log);
    }

    #[test]
    fn with_no_channel_a_fired_announcement_stays_pending_and_is_retried() {
        let dir = tempfile::tempdir().unwrap();
        let (records, published, _draft) = quiz_hall();
        let worlds = author_worlds(&records);
        let mut log = AnnouncementLog::new();
        let id = log
            .schedule(
                program_subject(&published),
                "quiz v1 is out",
                NOW + HOUR,
                &worlds,
                NOW,
                Some(dir.path()),
            )
            .unwrap();

        let outcomes = log
            .evaluate(NOW + HOUR + 5, &worlds, &mut NoDelivery, Some(dir.path()))
            .unwrap();
        assert_eq!(
            outcomes,
            vec![EvaluationOutcome::FiredDeliveryPending {
                id: id.clone(),
                late_by_ms: 5,
                reason: NO_DELIVERY_REASON.to_owned()
            }]
        );
        assert_eq!(
            log.get(&id).unwrap().state.name(),
            "fired, delivery pending"
        );

        // Still no channel: it stays fired, not fired twice.
        let outcomes = log
            .evaluate(NOW + 2 * HOUR, &worlds, &mut NoDelivery, Some(dir.path()))
            .unwrap();
        assert_eq!(
            outcomes,
            vec![EvaluationOutcome::StillPending {
                id: id.clone(),
                reason: NO_DELIVERY_REASON.to_owned()
            }]
        );

        // A channel arrives later and takes it; the original fire time
        // and lateness are kept.
        let mut log = AnnouncementLog::load(dir.path()).unwrap();
        let mut channel = Recorder::default();
        let outcomes = log
            .evaluate(NOW + 3 * HOUR, &worlds, &mut channel, Some(dir.path()))
            .unwrap();
        assert_eq!(
            outcomes,
            vec![EvaluationOutcome::Delivered { id: id.clone() }]
        );
        assert_eq!(
            log.get(&id).unwrap().state,
            AnnouncementState::Fired {
                fired_at_ms: NOW + HOUR + 5,
                late_by_ms: 5,
                delivered_at_ms: Some(NOW + 3 * HOUR)
            }
        );
        assert_eq!(AnnouncementLog::load(dir.path()).unwrap(), log);
    }

    #[test]
    fn a_subject_unpublished_before_the_due_time_is_skipped_not_announced() {
        let (records, published, _draft) = quiz_hall();
        let worlds = author_worlds(&records);
        let mut log = AnnouncementLog::new();
        let id = log
            .schedule(
                program_subject(&published),
                "quiz v1 is out",
                NOW + HOUR,
                &worlds,
                NOW,
                None,
            )
            .unwrap();

        // The author moves v1 back to draft before the due time.
        let mut records = records;
        record_fact(
            &mut records,
            &ProgramPublishFact::draft(published.identity()),
        )
        .unwrap();
        let worlds = author_worlds(&records);

        let mut channel = Recorder::default();
        let outcomes = log
            .evaluate(NOW + 2 * HOUR, &worlds, &mut channel, None)
            .unwrap();
        assert_eq!(
            outcomes,
            vec![EvaluationOutcome::Skipped { id: id.clone() }]
        );
        assert!(channel.delivered.is_empty());
        assert_eq!(log.get(&id).unwrap().state.name(), "skipped");
    }

    #[test]
    fn only_a_scheduled_announcement_can_be_cancelled() {
        let (records, published, _draft) = quiz_hall();
        let worlds = author_worlds(&records);
        let mut log = AnnouncementLog::new();
        let early = log
            .schedule(
                program_subject(&published),
                "quiz v1 is out",
                NOW + HOUR,
                &worlds,
                NOW,
                None,
            )
            .unwrap();
        let later = log
            .schedule(
                program_subject(&published),
                "quiz v1 reminder",
                NOW + 3 * HOUR,
                &worlds,
                NOW,
                None,
            )
            .unwrap();

        log.cancel(&later, NOW + 10, None).unwrap();
        log.evaluate(NOW + 4 * HOUR, &worlds, &mut NoDelivery, None)
            .unwrap();
        assert_eq!(log.get(&later).unwrap().state.name(), "cancelled");
        assert!(matches!(
            log.cancel(&early, NOW + 5 * HOUR, None),
            Err(AnnouncementError::NotScheduled { .. })
        ));
        assert!(matches!(
            log.cancel(&later, NOW + 5 * HOUR, None),
            Err(AnnouncementError::NotScheduled { .. })
        ));
        assert!(matches!(
            log.cancel("nope", NOW, None),
            Err(AnnouncementError::Unknown(_))
        ));
    }

    #[test]
    fn evaluation_fires_in_due_order_and_scheduling_twice_is_idempotent() {
        let (records, published, _draft) = quiz_hall();
        let worlds = author_worlds(&records);
        let mut log = AnnouncementLog::new();
        let second = log
            .schedule(
                program_subject(&published),
                "second",
                NOW + 2 * HOUR,
                &worlds,
                NOW,
                None,
            )
            .unwrap();
        let first = log
            .schedule(
                program_subject(&published),
                "first",
                NOW + HOUR,
                &worlds,
                NOW,
                None,
            )
            .unwrap();
        let again = log
            .schedule(
                program_subject(&published),
                "first",
                NOW + HOUR,
                &worlds,
                NOW,
                None,
            )
            .unwrap();
        assert_eq!(again, first);
        assert_eq!(log.len(), 2);
        assert_eq!(log.lines().len(), 2);

        let mut channel = Recorder::default();
        log.evaluate(NOW + 3 * HOUR, &worlds, &mut channel, None)
            .unwrap();
        assert_eq!(channel.delivered, vec![first, second]);
    }

    #[test]
    fn a_damaged_log_fails_to_load() {
        let dir = tempfile::tempdir().unwrap();
        let (records, published, _draft) = quiz_hall();
        let worlds = author_worlds(&records);
        let mut log = AnnouncementLog::new();
        log.schedule(
            program_subject(&published),
            "quiz v1 is out",
            NOW + HOUR,
            &worlds,
            NOW,
            Some(dir.path()),
        )
        .unwrap();
        log.evaluate(NOW + HOUR, &worlds, &mut NoDelivery, Some(dir.path()))
            .unwrap();
        let path = dir.path().join(ANNOUNCEMENTS_FILE);
        let good = std::fs::read_to_string(&path).unwrap();

        // An edited notice no longer matches its id.
        std::fs::write(&path, good.replace("quiz v1 is out", "quiz v9 is out")).unwrap();
        assert!(matches!(
            AnnouncementLog::load(dir.path()),
            Err(AnnouncementError::Malformed(_))
        ));

        // A fire time moved before the due time.
        let fired_early = good.replace(
            &format!("\"fired_at_ms\":{}", NOW + HOUR),
            &format!("\"fired_at_ms\":{}", NOW + HOUR - 1),
        );
        assert_ne!(fired_early, good);
        std::fs::write(&path, fired_early).unwrap();
        assert!(matches!(
            AnnouncementLog::load(dir.path()),
            Err(AnnouncementError::Malformed(_))
        ));

        // A dropped line breaks the sequence.
        let first_line_dropped: String = good.lines().skip(1).fold(String::new(), |mut out, l| {
            out.push_str(l);
            out.push('\n');
            out
        });
        std::fs::write(&path, first_line_dropped).unwrap();
        assert!(AnnouncementLog::load(dir.path()).is_err());

        std::fs::write(&path, good).unwrap();
        assert_eq!(AnnouncementLog::load(dir.path()).unwrap(), log);
    }

    #[test]
    fn the_record_holds_no_audience_data() {
        let (records, published, _draft) = quiz_hall();
        let worlds = author_worlds(&records);
        let mut log = AnnouncementLog::new();
        log.schedule(
            program_subject(&published),
            "quiz v1 is out",
            NOW + HOUR,
            &worlds,
            NOW,
            None,
        )
        .unwrap();
        log.evaluate(NOW + HOUR, &worlds, &mut Recorder::default(), None)
            .unwrap();
        for line in log.lines() {
            let value = serde_json::to_value(line).unwrap();
            let event = serde_json::to_value(&line.event).unwrap();
            let keys: BTreeSet<&str> = value
                .as_object()
                .unwrap()
                .keys()
                .chain(event.as_object().unwrap().keys())
                .map(String::as_str)
                .collect();
            for forbidden in ["audience", "recipients", "views", "opens", "clicks"] {
                assert!(!keys.contains(forbidden));
            }
        }
    }
}
