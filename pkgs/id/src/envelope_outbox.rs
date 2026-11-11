//! A durable queue of what this server sends to other servers: friend envelopes
//! and signed artifacts, each to a target endpoint.
//!
//! Each change is one JSON line, synced before it counts. The file is replayed
//! on open, so an entry that was not finished survives a restart. A line cut
//! short by a crash was never acknowledged, so it is dropped on open.

use std::collections::{BTreeMap, VecDeque};
#[cfg(unix)]
use std::fs::File;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

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
/// The maximum number of newest finished entries retained by the outbox.
pub const MAX_FINISHED_ENTRIES: usize = 100;
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
///
/// It retains the newest [`MAX_FINISHED_ENTRIES`] finished entries by finish
/// order, along with every unfinished entry. When replay drops older finished
/// entries, the journal is compacted on open.
#[derive(Debug)]
pub struct Outbox {
    path: PathBuf,
    entries: BTreeMap<String, Entry>,
    finished_order: VecDeque<String>,
}

impl Outbox {
    /// Opens the queue at `path`, replaying every record already in it.
    ///
    /// # Errors
    ///
    /// Fails if the file cannot be read, a complete line is corrupt, or a
    /// journal record violates the outbox state machine.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let mut outbox = Self {
            path: path.into(),
            entries: BTreeMap::new(),
            finished_order: VecDeque::new(),
        };
        let bytes = match fs::read(&outbox.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(outbox),
            Err(error) => return Err(error).context("reading the envelope outbox"),
        };
        let mut kept = 0;
        let mut dropped_finished = false;
        for (number, line) in bytes.split_inclusive(|byte| *byte == b'\n').enumerate() {
            if !line.ends_with(b"\n") {
                // The final line has no newline: a crash cut the append
                // short. Parseable or not, it was never completely
                // written, so it is torn and dropped — the same rule
                // ArtifactLog::open and Directory::replay apply. Keeping
                // a parseable torn line would let the next append merge
                // two records onto one line and brick the queue (#1620).
                // `kept` stays at the last complete line, so the file is
                // truncated to it below.
                break;
            }
            let text = std::str::from_utf8(line)
                .with_context(|| format!("envelope outbox line {} is corrupt", number + 1))?;
            if text.trim().is_empty() {
                // Blank lines carry no records, but they are complete bytes in
                // the journal and must remain before any later torn tail.
                kept += line.len();
                continue;
            }
            match serde_json::from_str::<Record>(text.trim_end()) {
                Ok(record) => {
                    dropped_finished |= outbox.apply(record).with_context(|| {
                        format!("envelope outbox line {} is refused", number + 1)
                    })?;
                    kept += line.len();
                }
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("envelope outbox line {} is corrupt", number + 1)
                    });
                }
            }
        }
        if kept < bytes.len() {
            OpenOptions::new()
                .write(true)
                .open(&outbox.path)?
                .set_len(kept as u64)?;
        }
        if dropped_finished {
            outbox.compact_journal()?;
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
        self.apply(record)?;
        Ok(())
    }

    /// Applies a record and reports whether it evicted a finished entry.
    fn apply(&mut self, record: Record) -> Result<bool> {
        let mut dropped_finished = false;
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
                ensure!(entry.finish.is_none(), "envelope {id} already finished");
                let expected_attempts = entry.attempts + 1;
                ensure!(
                    attempts == expected_attempts,
                    "outbox record for envelope {id} expected {expected_attempts} attempts, got {attempts}"
                );
                ensure!(
                    attempts < MAX_ATTEMPTS,
                    "outbox record attempts envelope {id} at the retry limit"
                );
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
                ensure!(entry.finish.is_none(), "envelope {id} already finished");
                let expected_attempts = entry.attempts + 1;
                ensure!(
                    attempts == expected_attempts,
                    "outbox record for envelope {id} expected {expected_attempts} attempts, got {attempts}"
                );
                ensure!(
                    attempts <= MAX_ATTEMPTS,
                    "outbox record exceeds the retry limit for envelope {id}"
                );
                match outcome {
                    Finish::Delivered => ensure!(
                        detail.is_none(),
                        "delivered outbox record for envelope {id} has refusal detail"
                    ),
                    Finish::Refused => ensure!(
                        detail.is_some(),
                        "refused outbox record for envelope {id} is missing detail"
                    ),
                    Finish::GaveUp => {
                        ensure!(
                            attempts == MAX_ATTEMPTS,
                            "gave-up outbox record for envelope {id} is not at the retry limit"
                        );
                        ensure!(
                            detail.is_some(),
                            "gave-up outbox record for envelope {id} is missing detail"
                        );
                    }
                }
                entry.attempts = attempts;
                entry.finish = Some(outcome);
                entry.last_error = detail;
                self.finished_order.retain(|finished_id| finished_id != &id);
                self.finished_order.push_back(id);
                if self.finished_order.len() > MAX_FINISHED_ENTRIES {
                    if let Some(oldest) = self.finished_order.pop_front() {
                        self.entries.remove(&oldest);
                        dropped_finished = true;
                    }
                }
            }
            Record::Withdrawn { id } => {
                let entry = self
                    .entries
                    .get(&id)
                    .with_context(|| format!("outbox record withdraws unknown envelope {id}"))?;
                ensure!(
                    entry.attempts == 0 && entry.finish.is_none(),
                    "envelope {id} was already attempted"
                );
                self.finished_order.retain(|finished_id| finished_id != &id);
                self.entries.remove(&id);
            }
        }
        Ok(dropped_finished)
    }

    /// Rewrites the journal as a snapshot of the retained entries.
    fn compact_journal(&self) -> Result<()> {
        let mut temporary_path = self.path.as_os_str().to_owned();
        temporary_path.push(".compact.tmp");
        let temporary_path = PathBuf::from(temporary_path);
        let permissions = fs::metadata(&self.path)?.permissions();
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temporary_path)
            .context("creating the compacted envelope outbox")?;

        for record in self.compact_records()? {
            serde_json::to_writer(&mut file, &record)?;
            file.write_all(b"\n")?;
        }
        fs::set_permissions(&temporary_path, permissions)?;
        file.sync_all()?;
        drop(file);
        replace_journal(&temporary_path, &self.path)
            .context("replacing the envelope outbox with its compacted journal")?;

        #[cfg(unix)]
        {
            let parent = self
                .path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."));
            File::open(parent)?.sync_all()?;
        }
        Ok(())
    }

    /// Builds a replayable snapshot while preserving the finish-order queue.
    fn compact_records(&self) -> Result<Vec<Record>> {
        // NOTE: the snapshot must satisfy the same replay validation as a
        // live journal (see `apply`): every `Attempted` record increments
        // the attempt count by exactly one, and a `Finished` record carries
        // the count after its final attempt. A snapshot therefore replays
        // an entry's attempt count incrementally (`push_attempt_history`)
        // instead of jumping straight to the final count.
        let mut records = Vec::with_capacity(self.entries.len() * 2);
        for entry in self.entries.values() {
            records.push(match &entry.payload {
                Payload::Envelope(envelope) => Record::Queued {
                    target: entry.target.clone(),
                    envelope: envelope.clone(),
                },
                Payload::Artifact(artifact) => Record::QueuedArtifact {
                    target: entry.target.clone(),
                    artifact: artifact.clone(),
                },
            });
        }

        // Pending entries have no finish-order significance, so their retry
        // state can be replayed before the finished entries.
        for (id, entry) in &self.entries {
            if entry.finish.is_none()
                && (entry.attempts > 0 || entry.next_at != 0 || entry.last_error.is_some())
            {
                push_attempt_increments(&mut records, id, entry.attempts);
                if entry.attempts > 0 {
                    records.push(Record::Attempted {
                        id: id.clone(),
                        attempts: entry.attempts,
                        next_at: entry.next_at,
                        error: entry.last_error.clone().unwrap_or_default(),
                    });
                }
            }
        }

        for id in &self.finished_order {
            let entry = self
                .entries
                .get(id)
                .with_context(|| format!("finished entry {id} is missing during compaction"))?;
            // The `Finished` record itself carries the final attempt count,
            // so only the increments below it are replayed as `Attempted`.
            // A finished entry's `next_at` is inert (`due` skips finished
            // entries), so the increments use a placeholder.
            push_attempt_increments(&mut records, id, entry.attempts);
            records.push(Record::Finished {
                id: id.clone(),
                attempts: entry.attempts,
                outcome: entry
                    .finish
                    .context("finished-order entry has no finish during compaction")?,
                detail: entry.last_error.clone(),
            });
        }
        Ok(records)
    }

    fn insert_queued(&mut self, target: String, payload: Payload) -> Result<()> {
        // The live path checks the target in the enqueue verbs
        // before committing; replay must hold the same line, or a journal
        // record the live API could never produce loads as a due entry and
        // only fails at flush time as a spurious refusal (#5635).
        check_target(&target)?;
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

/// Pushes placeholder `Attempted` records for the counts `1..below`, so a
/// compacted snapshot replays an entry's attempt count one increment at a
/// time, as replay validation requires. The caller pushes the final record
/// (the real `Attempted` for a pending entry, or the `Finished` record)
/// itself; placeholders carry no retry state.
fn push_attempt_increments(records: &mut Vec<Record>, id: &str, below: u32) {
    for attempts in 1..below {
        records.push(Record::Attempted {
            id: id.to_owned(),
            attempts,
            next_at: 0,
            error: String::new(),
        });
    }
}

/// Replaces the journal at `destination` with the compacted file at
/// `temporary`, as atomically as the platform allows.
#[cfg(not(windows))]
fn replace_journal(temporary: &Path, destination: &Path) -> io::Result<()> {
    // rename(2) replaces an existing destination atomically.
    fs::rename(temporary, destination)
}

/// Replaces the journal at `destination` with the compacted file at
/// `temporary`, as atomically as the platform allows.
#[cfg(windows)]
fn replace_journal(temporary: &Path, destination: &Path) -> io::Result<()> {
    // `fs::rename` cannot replace an existing file on Windows, and the
    // journal always exists by the time compaction runs, so a plain rename
    // would fail every compaction there (Copilot review on PR #4710).
    // Remove-then-rename is the fallback: it is NOT atomic — a crash
    // between the two calls leaves the journal missing and only the
    // `.compact.tmp` file behind — but compaction only runs after a full
    // successful replay, and the alternative on Windows is that
    // compaction (and with it `Outbox::open`) never succeeds at all.
    if destination.exists() {
        fs::remove_file(destination)?;
    }
    fs::rename(temporary, destination)
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

    fn queued_line(id: &str) -> String {
        let dir = tempfile::tempdir().unwrap();
        let mut outbox = Outbox::open(path(&dir)).unwrap();
        outbox.enqueue(&target(), envelope(id)).unwrap();
        fs::read_to_string(path(&dir))
            .unwrap()
            .trim_end_matches('\n')
            .to_owned()
    }

    fn queued_record(id: &str) -> Record {
        Record::Queued {
            target: target(),
            envelope: envelope(id),
        }
    }

    fn write_records(dir: &tempfile::TempDir, records: impl IntoIterator<Item = Record>) {
        let text = records
            .into_iter()
            .map(|record| format!("{}\n", serde_json::to_string(&record).unwrap()))
            .collect::<String>();
        fs::write(path(dir), text).unwrap();
    }

    fn assert_replay_error(records: impl IntoIterator<Item = Record>, line: usize, reason: &str) {
        let dir = tempfile::tempdir().unwrap();
        write_records(&dir, records);
        let error = Outbox::open(path(&dir))
            .err()
            .expect("invalid replay should fail");
        let message = format!("{error:#}");
        assert!(message.contains(&format!("line {line}")), "{message}");
        assert!(message.contains(reason), "{message}");
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
        drop(outbox);

        let reopened = Outbox::open(path(&dir)).unwrap();
        assert_eq!(reopened.entry("e1").unwrap().finish, Some(Finish::Refused));
        assert_eq!(
            reopened.entry("e1").unwrap().last_error.as_deref(),
            Some("bad signature")
        );
    }

    #[test]
    fn a_delivery_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let mut outbox = Outbox::open(path(&dir)).unwrap();
        outbox.enqueue(&target(), envelope("e1")).unwrap();
        outbox.record("e1", NOW, Outcome::Delivered).unwrap();
        drop(outbox);

        let reopened = Outbox::open(path(&dir)).unwrap();
        assert_eq!(
            reopened.entry("e1").unwrap().finish,
            Some(Finish::Delivered)
        );
        assert!(reopened.entry("e1").unwrap().last_error.is_none());
    }

    #[test]
    fn runtime_retains_the_newest_finished_entries_in_finish_order() {
        let dir = tempfile::tempdir().unwrap();
        let mut outbox = Outbox::open(path(&dir)).unwrap();
        let total = MAX_FINISHED_ENTRIES + 5;

        for index in 0..total {
            let id = format!("e{index:03}");
            outbox.enqueue(&target(), envelope(&id)).unwrap();
        }
        for index in (0..total).step_by(2).chain((1..total).step_by(2)) {
            let id = format!("e{index:03}");
            outbox.record(&id, NOW, Outcome::Delivered).unwrap();
        }

        assert_eq!(
            outbox
                .entries()
                .filter(|entry| entry.finish.is_some())
                .count(),
            MAX_FINISHED_ENTRIES
        );
        for oldest in [0, 2, 4, 6, 8] {
            assert!(outbox.entry(&format!("e{oldest:03}")).is_none());
        }
        assert!(outbox.entry("e001").is_some());
        assert!(outbox.entry("e104").is_some());

        outbox.enqueue(&target(), envelope("pending")).unwrap();
        assert_eq!(outbox.entries().count(), MAX_FINISHED_ENTRIES + 1);
        assert_eq!(outbox.due(NOW), vec!["pending".to_owned()]);
    }

    #[test]
    fn replay_compacts_finished_history_and_preserves_state_and_finish_order() {
        let dir = tempfile::tempdir().unwrap();
        let journal = path(&dir);
        let mut outbox = Outbox::open(&journal).unwrap();
        let total = MAX_FINISHED_ENTRIES + 5;

        for index in 0..total {
            let id = format!("e{index:03}");
            outbox.enqueue(&target(), envelope(&id)).unwrap();
        }
        for index in (0..total).step_by(2).chain((1..total).step_by(2)) {
            let id = format!("e{index:03}");
            outbox.record(&id, NOW, Outcome::Delivered).unwrap();
        }
        outbox.enqueue(&target(), envelope("pending-now")).unwrap();
        outbox
            .enqueue(&target(), envelope("pending-later"))
            .unwrap();
        outbox
            .record("pending-later", NOW, Outcome::Retry("down".to_owned()))
            .unwrap();

        let expected_entries = outbox.entries.clone();
        let expected_due_now = outbox.due(NOW);
        let expected_due_later = outbox.due(NOW + MINUTE);
        drop(outbox);

        // A parseable record without its trailing newline is still a torn tail;
        // open must truncate it before compacting the completed history.
        let mut file = OpenOptions::new().append(true).open(&journal).unwrap();
        file.write_all(queued_line("torn").as_bytes()).unwrap();
        drop(file);

        let reopened = Outbox::open(&journal).unwrap();
        assert_eq!(reopened.entries, expected_entries);
        assert_eq!(reopened.due(NOW), expected_due_now);
        assert_eq!(reopened.due(NOW + MINUTE), expected_due_later);
        assert!(reopened.entry("torn").is_none());

        let compacted = fs::read_to_string(&journal).unwrap();
        assert!(compacted.ends_with('\n'));
        assert!(!compacted.contains("e000"));
        assert!(!compacted.contains("e002"));
        assert!(compacted.contains("e001"));
        assert!(compacted.contains("pending-later"));
        drop(reopened);

        let mut reopened_again = Outbox::open(&journal).unwrap();
        assert_eq!(reopened_again.entries, expected_entries);
        assert_eq!(reopened_again.due(NOW), expected_due_now);
        assert_eq!(reopened_again.due(NOW + MINUTE), expected_due_later);
        assert_eq!(fs::read_to_string(&journal).unwrap(), compacted);

        // Compaction must keep the relative finish order so the next finish
        // evicts the oldest entry that survived the first cap.
        reopened_again
            .enqueue(&target(), envelope("next-finish"))
            .unwrap();
        reopened_again
            .record("next-finish", NOW, Outcome::Delivered)
            .unwrap();
        assert!(reopened_again.entry("e010").is_none());
        assert!(reopened_again.entry("e001").is_some());
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
    fn test_blank_and_whitespace_lines_are_skipped_and_kept() {
        let dir = tempfile::tempdir().unwrap();
        let contents = format!("{}\n\n \t\r\n{}\n\n", queued_line("e1"), queued_line("e2"));
        fs::write(path(&dir), &contents).unwrap();

        let outbox = Outbox::open(path(&dir)).unwrap();
        assert!(outbox.entry("e1").is_some());
        assert!(outbox.entry("e2").is_some());
        assert_eq!(fs::read_to_string(path(&dir)).unwrap(), contents);
    }

    #[test]
    fn test_blank_lines_before_a_torn_tail_are_kept_when_the_tail_is_truncated() {
        let dir = tempfile::tempdir().unwrap();
        let kept = format!("{}\n \t\n", queued_line("e1"));
        fs::write(path(&dir), format!("{kept}{{\"op\":\"queued\"")).unwrap();

        let outbox = Outbox::open(path(&dir)).unwrap();
        assert!(outbox.entry("e1").is_some());
        assert_eq!(fs::read_to_string(path(&dir)).unwrap(), kept);
    }

    #[test]
    fn a_torn_last_line_is_dropped_and_not_mixed_into_the_next_record() {
        let dir = tempfile::tempdir().unwrap();
        let mut outbox = Outbox::open(path(&dir)).unwrap();
        outbox.enqueue(&target(), envelope("e1")).unwrap();
        drop(outbox);
        let mut file = OpenOptions::new().append(true).open(path(&dir)).unwrap();
        // The tail is cut after the first byte of a multibyte name.
        file.write_all(b"{\"name\":\"Jos\xc3").unwrap();
        drop(file);

        let mut outbox = Outbox::open(path(&dir)).unwrap();
        assert!(outbox.entry("e1").is_some());
        outbox.enqueue(&target(), envelope("e2")).unwrap();
        drop(outbox);

        let reopened = Outbox::open(path(&dir)).unwrap();
        assert!(reopened.entry("e1").is_some() && reopened.entry("e2").is_some());
    }

    #[test]
    fn a_parseable_torn_last_line_is_also_dropped() {
        // #1620: the torn tail here is a complete JSON record missing
        // only its trailing newline. Before the fix, open accepted it,
        // and the next append merged two records onto one line.
        let dir = tempfile::tempdir().unwrap();
        let mut outbox = Outbox::open(path(&dir)).unwrap();
        outbox.enqueue(&target(), envelope("e1")).unwrap();
        drop(outbox);
        // A whole record line for e2, written by a healthy outbox, with
        // its newline stripped to simulate the crash cut.
        let scratch = tempfile::tempdir().unwrap();
        let mut other = Outbox::open(path(&scratch)).unwrap();
        other.enqueue(&target(), envelope("e2")).unwrap();
        drop(other);
        let line = fs::read_to_string(path(&scratch)).unwrap();
        let mut file = OpenOptions::new().append(true).open(path(&dir)).unwrap();
        file.write_all(line.trim_end_matches('\n').as_bytes())
            .unwrap();
        drop(file);

        let mut outbox = Outbox::open(path(&dir)).unwrap();
        assert!(outbox.entry("e1").is_some());
        assert!(
            outbox.entry("e2").is_none(),
            "the parseable torn record must be dropped, not accepted"
        );
        // The file was truncated to the last complete line, so the next
        // append starts on its own line and round-trips.
        outbox.enqueue(&target(), envelope("e2")).unwrap();
        drop(outbox);

        let reopened = Outbox::open(path(&dir)).unwrap();
        assert!(reopened.entry("e1").is_some() && reopened.entry("e2").is_some());
    }

    #[test]
    fn a_corrupt_complete_line_stops_the_load() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(path(&dir), "\nnot json\n").unwrap();
        let error = Outbox::open(path(&dir)).err().unwrap();
        assert!(format!("{error:#}").contains("envelope outbox line 2 is corrupt"));
    }

    #[test]
    fn replay_rejects_attempt_after_finish() {
        assert_replay_error(
            [
                queued_record("e1"),
                Record::Finished {
                    id: "e1".to_owned(),
                    attempts: 1,
                    outcome: Finish::Delivered,
                    detail: None,
                },
                Record::Attempted {
                    id: "e1".to_owned(),
                    attempts: 2,
                    next_at: NOW + MINUTE,
                    error: "down".to_owned(),
                },
            ],
            3,
            "already finished",
        );
    }

    #[test]
    fn replay_rejects_a_second_finish() {
        assert_replay_error(
            [
                queued_record("e1"),
                Record::Finished {
                    id: "e1".to_owned(),
                    attempts: 1,
                    outcome: Finish::Delivered,
                    detail: None,
                },
                Record::Finished {
                    id: "e1".to_owned(),
                    attempts: 2,
                    outcome: Finish::Refused,
                    detail: Some("bad signature".to_owned()),
                },
            ],
            3,
            "already finished",
        );
    }

    #[test]
    fn replay_rejects_attempt_count_regression() {
        let mut records = vec![queued_record("e1")];
        for attempts in 1..=5 {
            records.push(Record::Attempted {
                id: "e1".to_owned(),
                attempts,
                next_at: NOW + u64::from(attempts) * MINUTE,
                error: "down".to_owned(),
            });
        }
        records.push(Record::Attempted {
            id: "e1".to_owned(),
            attempts: 2,
            next_at: NOW + 2 * MINUTE,
            error: "down".to_owned(),
        });

        assert_replay_error(records, 7, "expected 6 attempts, got 2");
    }

    #[test]
    fn replay_rejects_withdrawal_after_an_attempt() {
        assert_replay_error(
            [
                queued_record("e1"),
                Record::Attempted {
                    id: "e1".to_owned(),
                    attempts: 1,
                    next_at: NOW + MINUTE,
                    error: "down".to_owned(),
                },
                Record::Withdrawn {
                    id: "e1".to_owned(),
                },
            ],
            3,
            "already attempted",
        );
    }

    #[test]
    fn replay_rejects_withdrawal_after_finish() {
        assert_replay_error(
            [
                queued_record("e1"),
                Record::Finished {
                    id: "e1".to_owned(),
                    attempts: 1,
                    outcome: Finish::Delivered,
                    detail: None,
                },
                Record::Withdrawn {
                    id: "e1".to_owned(),
                },
            ],
            3,
            "already attempted",
        );
    }

    #[test]
    fn replay_rejects_a_queued_record_with_a_malformed_target() {
        assert_replay_error(
            [Record::Queued {
                target: "not a target".to_owned(),
                envelope: envelope("e1"),
            }],
            1,
            "is neither an endpoint ticket nor a node ID",
        );
    }

    #[test]
    fn replay_rejects_a_queued_artifact_record_with_a_malformed_target() {
        let artifact = SignedArtifact {
            author: "00".to_owned(),
            seq: 1,
            prev: String::new(),
            kind: "note".to_owned(),
            body: serde_json::Value::Null,
            after: Vec::new(),
            hash: "00".to_owned(),
            signature: "00".to_owned(),
        };
        assert_replay_error(
            [Record::QueuedArtifact {
                target: "not a target".to_owned(),
                artifact,
            }],
            1,
            "is neither an endpoint ticket nor a node ID",
        );
    }

    #[test]
    fn a_node_id_is_required() {
        assert!(check_target(&target()).is_ok());
        assert!(check_target("https://bo.example/envelope").is_err());
        assert!(check_target("not a node").is_err());
    }
}
