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
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == ErrorKind::NotFound => String::new(),
            Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
        };
        let mut events = Vec::new();
        let mut kept = 0;
        for (number, line) in text.split_inclusive('\n').enumerate() {
            if line.trim().is_empty() {
                kept += line.len();
                continue;
            }
            let complete = line.ends_with('\n');
            match serde_json::from_str::<Event>(line.trim_end()) {
                Ok(event) => {
                    events.push(event);
                    kept += line.len();
                }
                Err(_) if !complete => {
                    tracing::warn!(
                        "group log {}: dropping torn final entry at byte {kept}",
                        path.display()
                    );
                    break;
                }
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("{}: line {} is not an event", path.display(), number + 1)
                    });
                }
            }
        }
        if kept < text.len() {
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

/// Serde adapters. A key or a hash is written as 64 hex characters, so a log line reads as text.
/// Each module is named for the type it adapts, and is used as `#[serde(with = "...")]`.
pub(crate) mod wire {
    use p2panda_core::Hash;
    use serde::de;

    use crate::groups::Key;

    fn key_text(key: &Key) -> String {
        Hash::from_bytes(*key).to_hex()
    }

    fn hash_text(hash: &Hash) -> String {
        hash.to_hex()
    }

    fn text_to_hash<E: de::Error>(text: &str) -> Result<Hash, E> {
        text.parse()
            .map_err(|error| E::custom(format!("`{text}` is not 64 hex characters: {error}")))
    }

    fn text_to_key<E: de::Error>(text: &str) -> Result<Key, E> {
        text_to_hash::<E>(text).map(|hash| *hash.as_bytes())
    }

    /// A key, as hex.
    pub mod key {
        use serde::{Deserialize, Deserializer, Serializer};

        use super::{Key, key_text, text_to_key};

        pub fn serialize<S: Serializer>(key: &Key, serializer: S) -> Result<S::Ok, S::Error> {
            serializer.serialize_str(&key_text(key))
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Key, D::Error> {
            text_to_key(&String::deserialize(deserializer)?)
        }
    }

    /// An optional key, as hex.
    pub mod opt_key {
        use serde::{Deserialize, Deserializer, Serialize, Serializer};

        use super::{Key, key_text, text_to_key};

        #[allow(clippy::ref_option)] // serde passes the field by reference.
        pub fn serialize<S: Serializer>(
            key: &Option<Key>,
            serializer: S,
        ) -> Result<S::Ok, S::Error> {
            key.as_ref().map(key_text).serialize(serializer)
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(
            deserializer: D,
        ) -> Result<Option<Key>, D::Error> {
            Option::<String>::deserialize(deserializer)?
                .as_deref()
                .map(text_to_key::<D::Error>)
                .transpose()
        }
    }

    /// A hash, as hex.
    pub mod hash {
        use p2panda_core::Hash;
        use serde::{Deserialize, Deserializer, Serializer};

        use super::{hash_text, text_to_hash};

        pub fn serialize<S: Serializer>(hash: &Hash, serializer: S) -> Result<S::Ok, S::Error> {
            serializer.serialize_str(&hash_text(hash))
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Hash, D::Error> {
            text_to_hash(&String::deserialize(deserializer)?)
        }
    }

    /// An optional hash, as hex.
    pub mod opt_hash {
        use p2panda_core::Hash;
        use serde::{Deserialize, Deserializer, Serialize, Serializer};

        use super::{hash_text, text_to_hash};

        #[allow(clippy::ref_option)] // serde passes the field by reference.
        pub fn serialize<S: Serializer>(
            hash: &Option<Hash>,
            serializer: S,
        ) -> Result<S::Ok, S::Error> {
            hash.as_ref().map(hash_text).serialize(serializer)
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(
            deserializer: D,
        ) -> Result<Option<Hash>, D::Error> {
            Option::<String>::deserialize(deserializer)?
                .as_deref()
                .map(text_to_hash::<D::Error>)
                .transpose()
        }
    }

    /// A list of hashes, as hex.
    pub mod hashes {
        use p2panda_core::Hash;
        use serde::{Deserialize, Deserializer, Serialize, Serializer};

        use super::{hash_text, text_to_hash};

        pub fn serialize<S: Serializer>(hashes: &[Hash], serializer: S) -> Result<S::Ok, S::Error> {
            hashes
                .iter()
                .map(hash_text)
                .collect::<Vec<_>>()
                .serialize(serializer)
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(
            deserializer: D,
        ) -> Result<Vec<Hash>, D::Error> {
            Vec::<String>::deserialize(deserializer)?
                .iter()
                .map(|text| text_to_hash::<D::Error>(text))
                .collect()
        }
    }

    /// A create's members, each with its role, as hex.
    pub mod members {
        use serde::{Deserialize, Deserializer, Serialize, Serializer};

        use crate::groups::Role;

        use super::{Key, key_text, text_to_key};

        pub fn serialize<S: Serializer>(
            members: &[(Key, Role)],
            serializer: S,
        ) -> Result<S::Ok, S::Error> {
            members
                .iter()
                .map(|(key, role)| (key_text(key), *role))
                .collect::<Vec<_>>()
                .serialize(serializer)
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(
            deserializer: D,
        ) -> Result<Vec<(Key, Role)>, D::Error> {
            Vec::<(String, Role)>::deserialize(deserializer)?
                .iter()
                .map(|(text, role)| Ok((text_to_key::<D::Error>(text)?, *role)))
                .collect()
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::groups::policy::{Policy, Template};
    use crate::groups::{Create, MemberOp, Role};
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
        let good = std::fs::read_to_string(&path).unwrap();
        // A crash mid-append leaves a partial event, no trailing newline.
        let mut torn = good.clone();
        torn.push_str("{\"id\":\"f00");
        std::fs::write(&path, torn).unwrap();

        let reopened = GroupLog::open(&path).unwrap();
        assert_eq!(reopened.events().len(), 1);
        let projection = reopened.projection().unwrap();
        assert!(projection.group(&created.event).is_some());
        // The torn bytes are truncated, so the next append is not glued
        // onto them and replays cleanly on the next open.
        assert_eq!(std::fs::read_to_string(&path).unwrap(), good);
    }
}
