//! Checkpoint part splitting and reassembly for world snapshots.
//!
//! A checkpoint whose encoded fields exceed the journal part limit is
//! written as a sequence of parts; this module splits a snapshot into
//! parts on write and accumulates parts back into the whole on restore.
//! Extracted from `core.rs` for the production-line guard (#5358).

use super::super::*;

use anyhow::{Context, Result, ensure};

/// Which variable-size checkpoint field a sequence of parts carries.
#[derive(Clone, Copy)]
pub(super) enum CheckpointEventPartKind {
    Retained,
    Admin,
}

pub(super) fn checkpoint_part_json_len(entry: &JournalEntry) -> Result<usize> {
    Ok(serde_json::to_vec(entry)
        .context("serialize checkpoint part")?
        .len())
}

fn checkpoint_event_part(
    kind: CheckpointEventPartKind,
    sequence: u64,
    part_index: u32,
    part_count: u32,
    events: Vec<WorldEvent>,
) -> JournalEntry {
    match kind {
        CheckpointEventPartKind::Retained => JournalEntry::CheckpointEventsPart {
            sequence,
            part_index,
            part_count,
            events,
        },
        CheckpointEventPartKind::Admin => JournalEntry::CheckpointAdminPart {
            sequence,
            part_index,
            part_count,
            events,
        },
    }
}

fn checkpoint_part_count(count: usize) -> Result<u32> {
    u32::try_from(count).context("checkpoint has too many parts")
}

pub(super) fn split_checkpoint_snapshot(
    sequence: u64,
    snapshot: &str,
) -> Result<Vec<JournalEntry>> {
    let mut chunks = Vec::new();
    if snapshot.is_empty() {
        chunks.push(String::new());
    } else {
        let mut start = 0;
        while start < snapshot.len() {
            let max_end = floor_char_boundary(
                snapshot,
                start
                    .saturating_add(MAX_CHECKPOINT_PART_BYTES)
                    .min(snapshot.len()),
            );
            let mut low = start + 1;
            let mut high = max_end;
            let mut best_end = start;
            while low <= high {
                let middle = low + (high - low) / 2;
                let end = floor_char_boundary(snapshot, middle);
                if end <= start {
                    // The midpoint fell inside the first character, so the
                    // floored prefix is empty: this says nothing about the
                    // size limit, and the search must advance to a larger
                    // endpoint instead of excluding the character's end.
                    low = middle.saturating_add(1);
                    continue;
                }
                let fits = checkpoint_part_json_len(&JournalEntry::CheckpointSnapshotPart {
                    sequence,
                    // Maximal-width placeholders ensure the final indices
                    // and count can only make the serialized entry smaller.
                    part_index: u32::MAX,
                    part_count: u32::MAX,
                    bytes: snapshot[start..end].to_owned(),
                })? <= MAX_CHECKPOINT_PART_BYTES;
                if fits {
                    best_end = end;
                    low = middle.saturating_add(1);
                } else {
                    high = middle.saturating_sub(1);
                }
            }
            ensure!(
                best_end > start,
                "one UTF-8 character cannot fit in a checkpoint snapshot part"
            );
            chunks.push(snapshot[start..best_end].to_owned());
            start = best_end;
        }
    }

    let part_count = checkpoint_part_count(chunks.len())?;
    chunks
        .into_iter()
        .enumerate()
        .map(|(part_index, bytes)| {
            let entry = JournalEntry::CheckpointSnapshotPart {
                sequence,
                part_index: u32::try_from(part_index).context("checkpoint part index overflow")?,
                part_count,
                bytes,
            };
            ensure!(
                checkpoint_part_json_len(&entry)? <= MAX_CHECKPOINT_PART_BYTES,
                "checkpoint snapshot part exceeds the encoded size limit"
            );
            Ok(entry)
        })
        .collect()
}

pub(super) fn split_checkpoint_events(
    sequence: u64,
    events: &[WorldEvent],
    kind: CheckpointEventPartKind,
) -> Result<Vec<JournalEntry>> {
    // Size each event once and accumulate: re-serializing the growing
    // candidate chunk per event would make splitting quadratic in the
    // window size. The accumulated length is conservative -- maximal-width
    // index placeholders for the entry overhead and one comma byte per
    // event -- and every finished part is re-checked exactly below.
    let overhead = checkpoint_part_json_len(&checkpoint_event_part(
        kind,
        sequence,
        u32::MAX,
        u32::MAX,
        Vec::new(),
    ))?;
    let mut chunks: Vec<Vec<WorldEvent>> = Vec::new();
    let mut current: Vec<WorldEvent> = Vec::new();
    let mut current_len = overhead;
    for event in events {
        let event_len = serde_json::to_vec(event)
            .context("serialize checkpoint event")?
            .len()
            + 1;
        ensure!(
            overhead + event_len <= MAX_CHECKPOINT_PART_BYTES,
            "one event cannot fit in a checkpoint part"
        );
        if current_len + event_len > MAX_CHECKPOINT_PART_BYTES {
            chunks.push(std::mem::take(&mut current));
            current_len = overhead;
        }
        current.push(event.clone());
        current_len += event_len;
    }
    if !current.is_empty() || chunks.is_empty() {
        chunks.push(current);
    }

    let part_count = checkpoint_part_count(chunks.len())?;
    chunks
        .into_iter()
        .enumerate()
        .map(|(part_index, events)| {
            let entry = checkpoint_event_part(
                kind,
                sequence,
                u32::try_from(part_index).context("checkpoint part index overflow")?,
                part_count,
                events,
            );
            ensure!(
                checkpoint_part_json_len(&entry)? <= MAX_CHECKPOINT_PART_BYTES,
                "checkpoint event part exceeds the encoded size limit"
            );
            Ok(entry)
        })
        .collect()
}

fn floor_char_boundary(text: &str, mut index: usize) -> usize {
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

#[derive(Default)]
struct CheckpointPartAccumulator<T> {
    part_count: Option<u32>,
    next_index: u32,
    parts: Vec<T>,
}

impl<T> CheckpointPartAccumulator<T> {
    fn push(&mut self, part_index: u32, part_count: u32, part: T, field: &str) -> Result<()> {
        ensure!(
            part_count > 0,
            "checkpoint {field} part_count must be positive"
        );
        ensure!(
            part_index < part_count,
            "checkpoint {field} part index is outside part_count"
        );
        if let Some(expected) = self.part_count {
            ensure!(
                part_count == expected,
                "checkpoint {field} part_count does not match"
            );
        } else {
            self.part_count = Some(part_count);
        }
        ensure!(
            part_index == self.next_index,
            "checkpoint {field} parts are duplicate or out of order"
        );
        self.next_index = self
            .next_index
            .checked_add(1)
            .context("checkpoint part index overflow")?;
        self.parts.push(part);
        Ok(())
    }

    fn is_complete(&self) -> bool {
        self.part_count == Some(self.next_index)
    }
}

#[derive(Default)]
pub(super) struct PendingCheckpointParts {
    sequence: Option<u64>,
    phase: u8,
    snapshot: CheckpointPartAccumulator<String>,
    events: CheckpointPartAccumulator<Vec<WorldEvent>>,
    admin: CheckpointPartAccumulator<Vec<WorldEvent>>,
}

impl PendingCheckpointParts {
    pub(super) fn has_parts(&self) -> bool {
        self.sequence.is_some()
    }

    pub(super) fn record_sequence(&mut self, sequence: u64) -> Result<()> {
        if let Some(expected) = self.sequence {
            ensure!(
                sequence == expected,
                "checkpoint part sequence does not match"
            );
        } else {
            self.sequence = Some(sequence);
        }
        Ok(())
    }

    pub(super) fn push_snapshot(
        &mut self,
        sequence: u64,
        part_index: u32,
        part_count: u32,
        bytes: String,
    ) -> Result<()> {
        ensure!(self.phase <= 1, "checkpoint parts are out of order");
        self.phase = 1;
        self.record_sequence(sequence)?;
        self.snapshot
            .push(part_index, part_count, bytes, "snapshot")
    }

    pub(super) fn push_events(
        &mut self,
        sequence: u64,
        part_index: u32,
        part_count: u32,
        events: Vec<WorldEvent>,
    ) -> Result<()> {
        if self.phase == 1 {
            ensure!(
                self.snapshot.is_complete(),
                "checkpoint snapshot parts are missing"
            );
            self.phase = 2;
        }
        ensure!(self.phase == 2, "checkpoint parts are out of order");
        self.record_sequence(sequence)?;
        self.events.push(part_index, part_count, events, "events")
    }

    pub(super) fn push_admin(
        &mut self,
        sequence: u64,
        part_index: u32,
        part_count: u32,
        events: Vec<WorldEvent>,
    ) -> Result<()> {
        if self.phase == 2 {
            ensure!(
                self.events.is_complete(),
                "checkpoint retained-event parts are missing"
            );
            self.phase = 3;
        }
        ensure!(self.phase == 3, "checkpoint parts are out of order");
        self.record_sequence(sequence)?;
        self.admin.push(part_index, part_count, events, "admin")
    }

    pub(super) fn assemble(
        self,
        sequence: u64,
    ) -> Result<(String, Vec<WorldEvent>, Vec<WorldEvent>)> {
        ensure!(
            self.sequence == Some(sequence),
            "checkpoint part sequence does not match checkpoint"
        );
        ensure!(
            self.phase == 3,
            "checkpoint is missing one or more part groups"
        );
        ensure!(
            self.snapshot.is_complete(),
            "checkpoint snapshot parts are missing"
        );
        ensure!(
            self.events.is_complete(),
            "checkpoint retained-event parts are missing"
        );
        ensure!(
            self.admin.is_complete(),
            "checkpoint admin-history parts are missing"
        );
        let snapshot = self.snapshot.parts.join("");
        let events = self.events.parts.into_iter().flatten().collect();
        let admin = self.admin.parts.into_iter().flatten().collect();
        Ok((snapshot, events, admin))
    }
}
