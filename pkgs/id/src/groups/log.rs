//! The group log: the events one node has recorded for its groups, kept in a file, one JSON object
//! per line (P6.7a and P6.7b).
//!
//! The log stands in for the events sync will carry. Every recorded event is kept, including the
//! ones the fold refuses, so the fold decides what took effect and the file is the author's record.
//! Each event names as its `after` the IDs that were the log's heads when it was recorded, so it
//! comes after every event before it. The fold needs that: it refuses an `after` it cannot place.
//!
//! Events are not signed here. The caller names the author, and the log is trusted as the file the
//! node keeps. Signature checks belong to the event store (see [`crate::chain`]).

use std::collections::BTreeSet;
use std::fs::OpenOptions;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result};
use p2panda_core::Hash;

use super::fold::{FoldError, Outcome, Projection, fold};

/// The serde adapters for the group model types, re-exported from
/// `id-events` (step 2b of the crate split, #4405): the model they
/// serialize moved down with the fold, and the adapters moved with it.
pub(crate) use id_events::groups::wire;
use super::{Body, Event, Key};

/// The file a node keeps its group log in, under its data directory.
pub const FILE_NAME: &str = "groups.jsonl";

/// What one recorded event did: its ID, and what the fold made of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recorded {
    /// The event's ID.
    pub event: Hash,
    /// Whether it took effect, or why not.
    pub outcome: Outcome,
}

/// The events one node has recorded, read from and appended to a file.
#[derive(Debug)]
pub struct GroupLog {
    path: PathBuf,
    events: Vec<Event>,
}

impl GroupLog {
    /// Opens the log at `path`. A file that does not exist is an empty log.
    ///
    /// A torn final line, left by a crash mid-append, is dropped and
    /// truncated away: that event was never acknowledged, and keeping it
    /// would glue it onto the next append. A newline-terminated line that
    /// does not parse was fully written, so it is corruption and stays
    /// fatal, mirroring `Outbox::open` and the world journal.
    ///
    /// # Errors
    ///
    /// Fails if the file cannot be read, or a complete line is not an event.
    pub fn open(path: &Path) -> Result<Self> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
        };
        let mut events = Vec::new();
        let mut kept = 0;
        for (number, line) in bytes.split_inclusive(|byte| *byte == b'\n').enumerate() {
            if !line.ends_with(b"\n") {
                tracing::warn!(
                    "group log {}: dropping torn final entry at byte {kept}",
                    path.display()
                );
                break;
            }
            let text = std::str::from_utf8(line).with_context(|| {
                format!("{}: line {} is not an event", path.display(), number + 1)
            })?;
            if text.trim().is_empty() {
                kept += line.len();
                continue;
            }
            match serde_json::from_str::<Event>(text.trim_end()) {
                Ok(event) => {
                    events.push(event);
                    kept += line.len();
                }
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("{}: line {} is not an event", path.display(), number + 1)
                    });
                }
            }
        }
        if kept < bytes.len() {
            OpenOptions::new()
                .write(true)
                .open(path)
                .with_context(|| format!("open {} to truncate its torn tail", path.display()))?
                .set_len(kept as u64)
                .with_context(|| format!("truncate torn tail of {}", path.display()))?;
        }
        Ok(Self {
            path: path.to_path_buf(),
            events,
        })
    }

    /// Every recorded event, in the order it was recorded.
    pub fn events(&self) -> &[Event] {
        &self.events
    }

    /// The groups the events give, and what became of each event.
    ///
    /// # Errors
    ///
    /// Fails if the events cannot be folded.
    pub fn projection(&self) -> Result<Projection, FoldError> {
        fold(&self.events)
    }

    /// Records an event by `author`, in `group` if it names one, after every event in the log. The
    /// fold decides its effect, and the event is written whether or not it took effect.
    ///
    /// # Errors
    ///
    /// Fails if the events cannot be folded, or the file cannot be written. Nothing is written when
    /// the fold fails.
    pub fn record(&mut self, author: Key, group: Option<Hash>, body: Body) -> Result<Recorded> {
        let mut event = Event {
            id: Hash::from_bytes([0; 32]),
            author,
            after: self.heads(),
            group,
            body,
        };
        // The ID is the hash of the event with a placeholder ID. Each event names every event before
        // it, so two events never serialize alike, and the ID is unique.
        event.id = Hash::digest(serde_json::to_vec(&event)?);
        let id = event.id;
        let mut next = self.events.clone();
        next.push(event);
        let outcome = fold(&next)?
            .outcome(&id)
            .cloned()
            .context("the fold did not decide the event")?;
        if let Some(added) = next.last() {
            self.append(added)?;
        }
        self.events = next;
        Ok(Recorded { event: id, outcome })
    }

    /// Records an event that was already formed elsewhere, keeping its
    /// ID and its place in its own chain: the seeding path for events a
    /// config provisioned (#5119 slice 3). An event whose ID the log
    /// already holds is skipped and changes nothing, so seeding the
    /// same events on every start is a no-op after the first.
    ///
    /// # Errors
    ///
    /// Fails if the events cannot be folded, or the file cannot be
    /// written. Nothing is written when the fold fails.
    pub fn record_preformed(&mut self, event: &Event) -> Result<Option<Recorded>> {
        if self.events.iter().any(|held| held.id == event.id) {
            return Ok(None);
        }
        let mut next = self.events.clone();
        next.push(event.clone());
        let outcome = fold(&next)?
            .outcome(&event.id)
            .cloned()
            .context("the fold did not decide the event")?;
        self.append(event)?;
        self.events = next;
        Ok(Some(Recorded {
            event: event.id,
            outcome,
        }))
    }

    /// The IDs no event names in its `after`. A new event names these, so it comes after all of them.
    fn heads(&self) -> Vec<Hash> {
        let named: BTreeSet<Hash> = self
            .events
            .iter()
            .flat_map(|event| event.after.iter().copied())
            .collect();
        self.events
            .iter()
            .map(|event| event.id)
            .filter(|id| !named.contains(id))
            .collect()
    }

    /// Appends one event as a line, and syncs it to disk.
    fn append(&self, event: &Event) -> Result<()> {
        let mut line = serde_json::to_string(event)?;
        line.push('\n');
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .with_context(|| format!("open {}", self.path.display()))?;
        file.write_all(line.as_bytes())
            .with_context(|| format!("write {}", self.path.display()))?;
        file.sync_data()
            .with_context(|| format!("sync {}", self.path.display()))
    }
}

/// Parses an account key from its 64 hex characters.
///
/// # Errors
///
/// Fails if the text is not 64 hex characters.
pub fn key_from_hex(text: &str) -> Result<Key> {
    Ok(*hash_from_hex(text)?.as_bytes())
}

/// Parses an event or group ID from its 64 hex characters.
///
/// # Errors
///
/// Fails if the text is not 64 hex characters.
pub fn hash_from_hex(text: &str) -> Result<Hash> {
    Hash::from_str(text.trim()).with_context(|| format!("`{text}` is not 64 hex characters"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::groups::actions::NewGroup;
    use crate::groups::policy::{Policy, RemoveMode, RemoveModeKind, RemovePolicy, Template};
    use crate::groups::{Create, MemberOp, Outcome, Refusal, Role};
    use tempfile::TempDir;

    const OWNER: Key = [1; 32];
    const ANN: Key = [2; 32];

    fn create_body() -> Body {
        Body::Create(Create {
            template: Template::Custom,
            policy: Policy::default(),
            members: Vec::new(),
            admins_on_create: false,
        })
    }

    #[test]
    fn test_record_preformed_seeds_once_and_then_skips() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(FILE_NAME);
        // Form the event in a scratch log, as provisioning forms it
        // away from the log it will seed.
        let scratch = dir.path().join("scratch.jsonl");
        let mut formed = GroupLog::open(&scratch).unwrap();
        let recorded = formed.record(OWNER, None, create_body()).unwrap();
        let event = formed.events()[0].clone();

        let mut log = GroupLog::open(&path).unwrap();
        let seeded = log.record_preformed(&event).unwrap().unwrap();
        assert_eq!(seeded.event, recorded.event);
        assert_eq!(seeded.outcome, Outcome::Applied);
        // Seeding the same event again changes nothing.
        assert!(log.record_preformed(&event).unwrap().is_none());
        assert_eq!(log.events().len(), 1);
        // And the seeded event is what a reopen reads back.
        let reopened = GroupLog::open(&path).unwrap();
        assert_eq!(reopened.events(), log.events());
        assert!(
            reopened
                .projection()
                .unwrap()
                .group(&recorded.event)
                .is_some()
        );
    }

    #[test]
    fn test_a_recorded_event_is_read_back_and_folds_the_same() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(FILE_NAME);
        let mut log = GroupLog::open(&path).unwrap();
        let created = log.record(OWNER, None, create_body()).unwrap();
        assert_eq!(created.outcome, Outcome::Applied);

        let reopened = GroupLog::open(&path).unwrap();
        assert_eq!(reopened.events(), log.events());
        let projection = reopened.projection().unwrap();
        assert!(projection.group(&created.event).is_some());
    }

    #[test]
    fn test_a_refused_event_is_kept_with_its_refusal() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(FILE_NAME);
        let mut log = GroupLog::open(&path).unwrap();
        let group = log.record(OWNER, None, create_body()).unwrap().event;
        let stranger = Body::Member(MemberOp::Add {
            member: ANN,
            role: Role::Read,
        });
        let refused = log.record(ANN, Some(group), stranger).unwrap();

        assert!(matches!(refused.outcome, Outcome::Refused(_)));
        assert_eq!(GroupLog::open(&path).unwrap().events().len(), 2);
    }

    #[test]
    fn test_remove_resolves_server_default_and_validates_before_recording() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(FILE_NAME);
        let mut log = GroupLog::open(&path).unwrap();
        let group = log
            .create(
                OWNER,
                NewGroup::Normal {
                    max: None,
                    admins_on_create: false,
                },
            )
            .unwrap()
            .group;
        log.add(OWNER, group, ANN, Role::Read).unwrap();
        let policy = Policy {
            remove_policy: RemovePolicy {
                allowed_modes: vec![RemoveModeKind::Revoke],
                allowed_successors: Vec::new(),
                default_mode: RemoveMode::Revoke,
            },
            ..Policy::default()
        };
        log.record(OWNER, Some(group), Body::Policy(policy))
            .unwrap();

        let action = log
            .remove(OWNER, group, ANN, RemoveMode::ServerDefault)
            .unwrap();
        assert_eq!(action.outcome, Outcome::Applied);
        assert!(matches!(
            &log.events().last().unwrap().body,
            Body::Member(MemberOp::Remove {
                mode: RemoveMode::Revoke,
                ..
            })
        ));

        let before = log.events().len();
        assert!(
            log.remove(OWNER, group, OWNER, RemoveMode::KeepPrior)
                .is_err()
        );
        assert_eq!(
            log.events().len(),
            before,
            "a disallowed mode is not recorded"
        );

        let refused = log
            .record(
                OWNER,
                Some(group),
                Body::Member(MemberOp::Remove {
                    member: OWNER,
                    mode: RemoveMode::KeepPrior,
                }),
            )
            .unwrap();
        assert_eq!(
            refused.outcome,
            Outcome::Refused(Refusal::RemoveModeNotAllowed(RemoveModeKind::KeepPrior))
        );
    }

    #[test]
    fn test_legacy_remove_events_map_successor_to_a_deterministic_mode() {
        let member = Hash::from_bytes(ANN).to_hex();
        let successor = Hash::from_bytes([3; 32]).to_hex();
        let legacy_revoke = serde_json::json!({
            "Remove": { "member": member, "successor": null }
        });
        let legacy_transfer = serde_json::json!({
            "Remove": { "member": member, "successor": successor }
        });

        assert_eq!(
            serde_json::from_value::<MemberOp>(legacy_revoke).unwrap(),
            MemberOp::Remove {
                member: ANN,
                mode: RemoveMode::Revoke,
            }
        );
        assert_eq!(
            serde_json::from_value::<MemberOp>(legacy_transfer).unwrap(),
            MemberOp::Remove {
                member: ANN,
                mode: RemoveMode::Transfer(crate::groups::TransferOpts {
                    successor: crate::groups::RemoveSuccessor::Member([3; 32]),
                }),
            }
        );

        let encoded = serde_json::to_value(MemberOp::Remove {
            member: ANN,
            mode: RemoveMode::Revoke,
        })
        .unwrap();
        let remove = &encoded["Remove"];
        assert_eq!(remove["mode"], serde_json::json!("Revoke"));
        assert!(remove.get("successor").is_none());
    }

    #[test]
    fn test_each_event_names_every_head_before_it() {
        let dir = TempDir::new().unwrap();
        let mut log = GroupLog::open(&dir.path().join(FILE_NAME)).unwrap();
        let first = log.record(OWNER, None, create_body()).unwrap().event;
        let second = log.record(OWNER, None, create_body()).unwrap().event;
        assert_eq!(log.events()[1].after, vec![first]);
        assert_ne!(first, second);
    }

    #[test]
    fn test_a_line_that_is_not_an_event_is_an_error() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(FILE_NAME);
        std::fs::write(&path, "not json\n").unwrap();
        assert!(GroupLog::open(&path).is_err());
    }

    #[test]
    fn test_a_torn_final_fragment_is_dropped_and_truncated() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(FILE_NAME);
        // A log whose only content is a fragment: the writer crashed
        // mid-append, before the event was ever acknowledged.
        std::fs::write(&path, "{\"id\":\"abc").unwrap();
        let log = GroupLog::open(&path).unwrap();
        assert!(log.events().is_empty());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "");
    }

    #[test]
    fn test_a_torn_fragment_after_valid_events_keeps_them_and_folds() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(FILE_NAME);
        let mut log = GroupLog::open(&path).unwrap();
        let created = log.record(OWNER, None, create_body()).unwrap();
        drop(log);
        let good = std::fs::read(&path).unwrap();
        // A crash after the first byte of a multibyte name leaves invalid
        // UTF-8 in the unterminated final event.
        let mut torn = good.clone();
        torn.extend_from_slice(b"{\"name\":\"Jos");
        torn.push(0xc3); // First byte of "é".
        std::fs::write(&path, torn).unwrap();

        let mut reopened = GroupLog::open(&path).unwrap();
        assert_eq!(reopened.events().len(), 1);
        let projection = reopened.projection().unwrap();
        assert!(projection.group(&created.event).is_some());
        // The torn bytes are truncated, so the next append is not glued
        // onto them and replays cleanly on the next open.
        assert_eq!(std::fs::read(&path).unwrap(), good);
        reopened.record(OWNER, None, create_body()).unwrap();
        drop(reopened);
        assert_eq!(GroupLog::open(&path).unwrap().events().len(), 2);
    }

    #[test]
    fn test_a_parseable_torn_final_line_is_dropped_before_append() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(FILE_NAME);
        let mut log = GroupLog::open(&path).unwrap();
        let first = log.record(OWNER, None, create_body()).unwrap().event;
        log.record(OWNER, None, create_body()).unwrap();
        drop(log);

        let complete = std::fs::read_to_string(&path).unwrap();
        let last_complete_line_end = complete[..complete.len() - 1].rfind('\n').unwrap() + 1;
        let kept = &complete[..last_complete_line_end];
        std::fs::write(&path, &complete[..complete.len() - 1]).unwrap();

        let mut reopened = GroupLog::open(&path).unwrap();
        assert_eq!(reopened.events().len(), 1);
        assert!(reopened.projection().unwrap().group(&first).is_some());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), kept);

        reopened.record(OWNER, None, create_body()).unwrap();
        drop(reopened);
        let roundtrip = GroupLog::open(&path).unwrap();
        assert_eq!(roundtrip.events().len(), 2);
        assert!(roundtrip.projection().unwrap().group(&first).is_some());
    }
}
