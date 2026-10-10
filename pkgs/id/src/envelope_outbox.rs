//! A durable queue of what this server sends to other servers: friend envelopes
//! and signed artifacts, each to a target endpoint.
//!
//! Each change is one JSON line, synced before it counts. The file is replayed
//! on open, so an entry that was not finished survives a restart. A line cut
//! short by a crash was never acknowledged, so it is dropped on open.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow, ensure};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use iroh::{Endpoint, EndpointAddr, EndpointId};
use iroh_tickets::endpoint::EndpointTicket;

use crate::artifact::{self, SignedArtifact};
use crate::directory::Envelope;
use crate::envelope_net::deliver;

/// Deliveries attempted before an envelope is given up.
pub const MAX_ATTEMPTS: u32 = 12;
const BASE_DELAY_MS: u64 = 60_000;
const MAX_DELAY_MS: u64 = 6 * 60 * 60 * 1000;

/// How a queued envelope ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Finish {
    /// The recipient's server accepted it.
    Delivered,
    /// The recipient's server refused it, so it is not retried.
    Refused,
    /// Every attempt failed.
    GaveUp,
}

/// The result of one delivery attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The envelope was delivered or was already applied there.
    Delivered,
    /// The envelope will not be accepted, and retrying cannot change that.
    Refused(String),
    /// The attempt failed in a way a later attempt may fix.
    Retry(String),
}

/// What the outbox carries to another server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Payload {
    /// A friend envelope, for the recipient's server.
    Envelope(Envelope),
    /// A signed artifact, for a home server that holds the account's chain.
    Artifact(SignedArtifact),
}

impl Payload {
    /// The key the queue files this payload under, so one payload is queued once.
    #[must_use]
    pub fn key(&self, target: &str) -> String {
        match self {
            Self::Envelope(envelope) => envelope.id.clone(),
            Self::Artifact(artifact) => format!("artifact {target} {}", artifact.hash),
        }
    }
}

/// One queued payload and what has happened to it so far.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The recipient: an endpoint ticket or a bare node ID.
    pub target: String,
    /// What to deliver.
    pub payload: Payload,
    /// Attempts made so far.
    pub attempts: u32,
    /// The earliest time the next attempt may run, in Unix milliseconds.
    pub next_at: u64,
    /// The reason for the last failed attempt or the final outcome.
    pub last_error: Option<String>,
    /// Set once the payload is no longer queued.
    pub finish: Option<Finish>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Record {
    Queued {
        target: String,
        envelope: Envelope,
    },
    QueuedArtifact {
        target: String,
        artifact: SignedArtifact,
    },
    Attempted {
        id: String,
        attempts: u32,
        next_at: u64,
        error: String,
    },
    Finished {
        id: String,
        attempts: u32,
        outcome: Finish,
        detail: Option<String>,
    },
    Withdrawn {
        id: String,
    },
}

/// The queue, keyed by envelope ID.
#[derive(Debug)]
pub struct Outbox {
    path: PathBuf,
    entries: BTreeMap<String, Entry>,
}

impl Outbox {
    /// Opens the queue at `path`, replaying every record already in it.
    ///
    /// # Errors
    ///
    /// Fails if the file cannot be read, or a complete line is corrupt.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let mut outbox = Self {
            path: path.into(),
            entries: BTreeMap::new(),
        };
        let text = match fs::read_to_string(&outbox.path) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(outbox),
            Err(error) => return Err(error).context("reading the envelope outbox"),
        };
        let mut kept = 0;
        for (number, line) in text.split_inclusive('\n').enumerate() {
            let complete = line.ends_with('\n');
            match serde_json::from_str::<Record>(line.trim_end()) {
                Ok(record) => {
                    outbox.apply(record)?;
                    kept += line.len();
                }
                Err(_) if !complete => break,
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("envelope outbox line {} is corrupt", number + 1)
                    });
                }
            }
        }
        if kept < text.len() {
            OpenOptions::new()
                .write(true)
                .open(&outbox.path)?
                .set_len(kept as u64)?;
        }
        Ok(outbox)
    }

    /// Queues `envelope` for delivery to the node `target`, due immediately.
    ///
    /// # Errors
    ///
    /// Fails if the node ID is malformed, the envelope is already queued, or the
    /// record cannot be written.
    pub fn enqueue(&mut self, target: &str, envelope: Envelope) -> Result<()> {
        check_target(target)?;
        self.enqueue_payload(&Payload::Envelope(envelope), target)
    }

    /// Queues `artifact` for a home server at `target`, due immediately.
    ///
    /// # Errors
    ///
    /// Fails if the target is malformed, the artifact is already queued for it,
    /// or the record cannot be written.
    pub fn enqueue_artifact(&mut self, target: &str, artifact: SignedArtifact) -> Result<()> {
        check_target(target)?;
        self.enqueue_payload(&Payload::Artifact(artifact), target)
    }

    fn enqueue_payload(&mut self, payload: &Payload, target: &str) -> Result<()> {
        let key = payload.key(target);
        ensure!(!self.entries.contains_key(&key), "{key} is already queued");
        let record = match payload {
            Payload::Envelope(envelope) => Record::Queued {
                target: target.to_owned(),
                envelope: envelope.clone(),
            },
            Payload::Artifact(artifact) => Record::QueuedArtifact {
                target: target.to_owned(),
                artifact: artifact.clone(),
            },
        };
        self.commit(record)
    }

    /// The IDs of envelopes that are queued and due at `now`.
    #[must_use]
    pub fn due(&self, now: u64) -> Vec<String> {
        self.entries
            .iter()
            .filter(|(_, entry)| entry.finish.is_none() && entry.next_at <= now)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// The queued envelope with `id`, finished or not.
    #[must_use]
    pub fn entry(&self, id: &str) -> Option<&Entry> {
        self.entries.get(id)
    }

    /// Removes an envelope that was queued but never attempted, as if it had
    /// never been queued.
    ///
    /// # Errors
    ///
    /// Fails if `id` is not queued, has been attempted, or the record cannot be
    /// written.
    pub fn withdraw(&mut self, id: &str) -> Result<()> {
        let entry = self.entries.get(id).context("envelope is not queued")?;
        ensure!(
            entry.attempts == 0 && entry.finish.is_none(),
            "envelope {id} was already attempted"
        );
        self.commit(Record::Withdrawn { id: id.to_owned() })
    }

    /// Records the outcome of an attempt made at `now`.
    ///
    /// # Errors
    ///
    /// Fails if `id` is not queued, or the record cannot be written.
    pub fn record(&mut self, id: &str, now: u64, outcome: Outcome) -> Result<()> {
        let entry = self.entries.get(id).context("envelope is not queued")?;
        ensure!(entry.finish.is_none(), "envelope {id} already finished");
        let attempts = entry.attempts + 1;
        let record = match outcome {
            Outcome::Delivered => Record::Finished {
                id: id.to_owned(),
                attempts,
                outcome: Finish::Delivered,
                detail: None,
            },
            Outcome::Refused(error) => Record::Finished {
                id: id.to_owned(),
                attempts,
                outcome: Finish::Refused,
                detail: Some(error),
            },
            Outcome::Retry(error) if attempts >= MAX_ATTEMPTS => Record::Finished {
                id: id.to_owned(),
                attempts,
                outcome: Finish::GaveUp,
                detail: Some(error),
            },
            Outcome::Retry(error) => Record::Attempted {
                id: id.to_owned(),
                attempts,
                next_at: now.saturating_add(backoff_ms(attempts)),
                error,
            },
        };
        self.commit(record)
    }

    /// Every queued envelope, finished or not, in ID order.
    pub fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.entries.values()
    }

    fn commit(&mut self, record: Record) -> Result<()> {
        let mut line = serde_json::to_string(&record)?;
        line.push('\n');
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        file.write_all(line.as_bytes())?;
        file.sync_data()?;
        self.apply(record)
    }

    fn apply(&mut self, record: Record) -> Result<()> {
        match record {
            Record::Queued { target, envelope } => {
                self.insert_queued(target, Payload::Envelope(envelope))?;
            }
            Record::QueuedArtifact { target, artifact } => {
                self.insert_queued(target, Payload::Artifact(artifact))?;
            }
            Record::Attempted {
                id,
                attempts,
                next_at,
                error,
            } => {
                let entry = self.entry_mut(&id)?;
                entry.attempts = attempts;
                entry.next_at = next_at;
                entry.last_error = Some(error);
            }
            Record::Finished {
                id,
                attempts,
                outcome,
                detail,
            } => {
                let entry = self.entry_mut(&id)?;
                entry.attempts = attempts;
                entry.finish = Some(outcome);
                entry.last_error = detail;
            }
            Record::Withdrawn { id } => {
                ensure!(
                    self.entries.remove(&id).is_some(),
                    "outbox record withdraws unknown envelope {id}"
                );
            }
        }
        Ok(())
    }

    fn insert_queued(&mut self, target: String, payload: Payload) -> Result<()> {
        let key = payload.key(&target);
        ensure!(!self.entries.contains_key(&key), "{key} is queued twice");
        self.entries.insert(
            key,
            Entry {
                target,
                payload,
                attempts: 0,
                next_at: 0,
                last_error: None,
                finish: None,
            },
        );
        Ok(())
    }

    fn entry_mut(&mut self, id: &str) -> Result<&mut Entry> {
        self.entries
            .get_mut(id)
            .with_context(|| format!("outbox record names unknown envelope {id}"))
    }
}

/// Delivers every envelope due at `now`, recording each outcome as it happens.
///
/// The lock is released while a delivery is in flight, so enqueueing is not
/// held up by a slow recipient. Returns how many attempts were made.
///
/// # Errors
///
/// Fails if an outcome cannot be written. Envelopes already recorded stay
/// recorded.
pub async fn flush(outbox: &Mutex<Outbox>, endpoint: &Endpoint, now: u64) -> Result<usize> {
    let due: Vec<(String, String, Payload)> = {
        let queue = outbox.lock().await;
        queue
            .due(now)
            .into_iter()
            .filter_map(|id| {
                let entry = queue.entry(&id)?;
                Some((id.clone(), entry.target.clone(), entry.payload.clone()))
            })
            .collect()
    };
    for (id, target, payload) in &due {
        let outcome = match check_target(target) {
            Ok(addr) => match payload {
                Payload::Envelope(envelope) => deliver(endpoint, addr, envelope).await,
                Payload::Artifact(artifact) => artifact::push(endpoint, addr, artifact).await,
            },
            Err(error) => Outcome::Refused(format!("{error:#}")),
        };
        outbox.lock().await.record(id, now, outcome)?;
    }
    Ok(due.len())
}

/// Reads a recipient: an endpoint ticket, or a bare node ID that resolves
/// through the discovery services the endpoint uses.
///
/// # Errors
///
/// Fails if `target` is neither, so nothing is queued for it.
pub fn check_target(target: &str) -> Result<EndpointAddr> {
    if let Ok(ticket) = target.parse::<EndpointTicket>() {
        return Ok(ticket.endpoint_addr().clone());
    }
    target
        .parse::<EndpointId>()
        .map(EndpointAddr::from)
        .map_err(|error| anyhow!("{target} is neither an endpoint ticket nor a node ID: {error}"))
}

fn backoff_ms(attempts: u32) -> u64 {
    let shift = attempts.saturating_sub(1).min(20);
    (BASE_DELAY_MS << shift).min(MAX_DELAY_MS)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::directory::EnvelopeKind;

    const NOW: u64 = 1_000_000_000;
    const MINUTE: u64 = 60_000;

    fn envelope(id: &str) -> Envelope {
        Envelope {
            kind: EnvelopeKind::FriendRequest,
            from: "ann".to_owned(),
            to: "bo".to_owned(),
            audience: "club".to_owned(),
            id: id.to_owned(),
            request: None,
            at: NOW,
            signature: "00".to_owned(),
        }
    }

    fn target() -> String {
        iroh::SecretKey::from_bytes(&[7; 32]).public().to_string()
    }

    fn path(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join("outbox.jsonl")
    }

    #[test]
    fn a_target_is_a_ticket_or_a_bare_node_id() {
        let id = iroh::SecretKey::from_bytes(&[7; 32]).public();
        let addr = EndpointAddr::from(id);
        let ticket = EndpointTicket::new(addr.clone()).to_string();
        assert_eq!(check_target(&ticket).unwrap(), addr);
        assert_eq!(check_target(&id.to_string()).unwrap(), addr);
        assert!(check_target("not a target").is_err());
    }

    #[test]
    fn queued_envelopes_survive_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let mut outbox = Outbox::open(path(&dir)).unwrap();
        outbox.enqueue(&target(), envelope("e1")).unwrap();
        drop(outbox);

        let reopened = Outbox::open(path(&dir)).unwrap();
        assert_eq!(reopened.due(NOW), vec!["e1".to_owned()]);
        assert_eq!(
            reopened.entry("e1").unwrap().payload,
            Payload::Envelope(envelope("e1"))
        );
    }

    #[test]
    fn a_retry_waits_for_its_backoff_and_then_gives_up() {
        let dir = tempfile::tempdir().unwrap();
        let mut outbox = Outbox::open(path(&dir)).unwrap();
        outbox.enqueue(&target(), envelope("e1")).unwrap();

        outbox
            .record("e1", NOW, Outcome::Retry("down".to_owned()))
            .unwrap();
        let entry = outbox.entry("e1").unwrap();
        assert_eq!((entry.attempts, entry.next_at), (1, NOW + MINUTE));
        assert!(outbox.due(NOW + MINUTE - 1).is_empty());
        assert_eq!(outbox.due(NOW + MINUTE), vec!["e1".to_owned()]);

        for attempt in 2..=MAX_ATTEMPTS {
            outbox
                .record("e1", NOW, Outcome::Retry("down".to_owned()))
                .unwrap();
            assert_eq!(outbox.entry("e1").unwrap().attempts, attempt);
        }
        let entry = outbox.entry("e1").unwrap();
        assert_eq!(entry.finish, Some(Finish::GaveUp));
        assert!(outbox.due(u64::MAX).is_empty());

        let reopened = Outbox::open(path(&dir)).unwrap();
        assert_eq!(reopened.entry("e1").unwrap().finish, Some(Finish::GaveUp));
    }

    #[test]
    fn a_refusal_is_final() {
        let dir = tempfile::tempdir().unwrap();
        let mut outbox = Outbox::open(path(&dir)).unwrap();
        outbox.enqueue(&target(), envelope("e1")).unwrap();
        outbox
            .record("e1", NOW, Outcome::Refused("bad signature".to_owned()))
            .unwrap();
        let entry = outbox.entry("e1").unwrap();
        assert_eq!(entry.finish, Some(Finish::Refused));
        assert_eq!(entry.last_error.as_deref(), Some("bad signature"));
        assert!(outbox.due(u64::MAX).is_empty());
        assert!(outbox.record("e1", NOW, Outcome::Delivered).is_err());
    }

    #[test]
    fn a_withdrawn_envelope_is_gone_and_stays_gone() {
        let dir = tempfile::tempdir().unwrap();
        let mut outbox = Outbox::open(path(&dir)).unwrap();
        outbox.enqueue(&target(), envelope("e1")).unwrap();
        outbox.withdraw("e1").unwrap();
        assert!(outbox.entry("e1").is_none());
        assert!(outbox.due(u64::MAX).is_empty());
        assert!(outbox.withdraw("e1").is_err());
        drop(outbox);

        let reopened = Outbox::open(path(&dir)).unwrap();
        assert!(reopened.entry("e1").is_none());
    }

    #[test]
    fn a_torn_last_line_is_dropped_and_not_mixed_into_the_next_record() {
        let dir = tempfile::tempdir().unwrap();
        let mut outbox = Outbox::open(path(&dir)).unwrap();
        outbox.enqueue(&target(), envelope("e1")).unwrap();
        drop(outbox);
        let mut file = OpenOptions::new().append(true).open(path(&dir)).unwrap();
        file.write_all(b"{\"op\":\"queued\",\"target\"").unwrap();
        drop(file);

        let mut outbox = Outbox::open(path(&dir)).unwrap();
        assert!(outbox.entry("e1").is_some());
        outbox.enqueue(&target(), envelope("e2")).unwrap();
        drop(outbox);

        let reopened = Outbox::open(path(&dir)).unwrap();
        assert!(reopened.entry("e1").is_some() && reopened.entry("e2").is_some());
    }

    #[test]
    fn a_corrupt_complete_line_stops_the_load() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(path(&dir), "not json\n").unwrap();
        assert!(Outbox::open(path(&dir)).is_err());
    }

    #[test]
    fn a_node_id_is_required() {
        assert!(check_target(&target()).is_ok());
        assert!(check_target("https://bo.example/envelope").is_err());
        assert!(check_target("not a node").is_err());
    }
}
