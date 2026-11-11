//! Vocabulary types of the event store: its error, its ingest outcomes, its
//! operator block list, and the row shapes its tables hold.
//!
//! Step 2a of the pkgs/id crate split (#4405): these types are store
//! vocabulary, so they live in `id-store` with the store traits that name
//! them ([`StoreError`] is the traits' error, [`Ingest`] their outcome).
//! The `EventStore` implementation that produces them stays in the facade
//! until step 2b extracts `id-events`; it re-exports these types at their
//! historical `id::event_store` paths, so no call site changed.

use std::io;

use p2panda_core::{Hash, SeqNum};
use redb::{CommitError, DatabaseError, StorageError, TableError, TransactionError};

use id_core::rejection::Reason;
use id_core::waiting::AuthorKey;

pub use id_core::block_list::{BlockKind, BlockList};

/// The store's file name in its directory.
pub const FILE_NAME: &str = "events.redb";

/// Bytes in a chain row: the next sequence number, the head hash, the frozen flag.
const CHAIN_LEN: usize = 8 + 32 + 1;

/// Why the event store failed. A failure is never a refusal of an event: refused
/// events are reported as outcomes, not errors.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The directory could not be read or written.
    #[error("event store file I/O: {0}")]
    Io(#[from] io::Error),
    /// redb could not open or create the database.
    #[error("event store database: {0}")]
    Database(#[from] DatabaseError),
    /// A redb transaction could not begin.
    #[error("event store transaction: {0}")]
    Transaction(#[from] redb::TransactionError),
    /// A table could not be opened, or it does not exist.
    #[error("event store table: {0}")]
    Table(#[from] redb::TableError),
    /// A read or write failed inside a transaction.
    #[error("event store storage: {0}")]
    Storage(#[from] StorageError),
    /// A commit failed.
    #[error("event store commit: {0}")]
    Commit(#[from] redb::CommitError),
    /// Another process holds the store open. redb allows one opener at a time.
    #[error("{FILE_NAME} is open in another process")]
    InUse,
    /// The file is there, but it is not an event store of this format.
    #[error("{FILE_NAME} is not an event store: {0}")]
    NotAStore(String),
    /// The store's rows contradict each other. The file was changed outside the store.
    #[error("event store is corrupt: {0}")]
    Corrupt(String),
    /// The position already holds an event, so a second one cannot be stored there.
    #[error("sequence {seq} of this author is already stored")]
    Occupied {
        /// The sequence number that is taken.
        seq: SeqNum,
    },
    /// A write failed, and the waiting events could not be rebuilt from disk. The store
    /// refuses further calls. Reopening it rebuilds them.
    #[error("event store is unusable after a failed write: reopen it")]
    Unusable,
    /// The reader is not a member of the group whose chat they asked to read (#3950).
    #[error("the reader is not a member of this group")]
    NotMember,
}

pub fn corrupt(message: impl Into<String>) -> StoreError {
    StoreError::Corrupt(message.into())
}

/// The applied prefix of one author's chain, as the `chains` table holds it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Chain {
    /// The sequence number the next applied event takes: the number of applied
    /// events. Zero for an author with none.
    pub next_seq: SeqNum,
    /// The hash of the last applied event, or `None` when none applies.
    pub head: Option<Hash>,
    /// Whether the author has forked. A frozen chain applies nothing from its
    /// fork point on.
    pub frozen: bool,
}

impl Chain {
    pub fn encode(self) -> [u8; CHAIN_LEN] {
        let mut row = [0u8; CHAIN_LEN];
        row[..8].copy_from_slice(&u64::from(self.next_seq).to_be_bytes());
        if let Some(head) = self.head {
            row[8..40].copy_from_slice(head.as_bytes());
        }
        row[40] = u8::from(self.frozen);
        row
    }

    pub fn decode(row: &[u8]) -> Result<Self, StoreError> {
        let row: &[u8; CHAIN_LEN] = row
            .try_into()
            .map_err(|_err| corrupt("a chain row has the wrong length"))?;
        let next_seq = seq_num(be_u64(&row[..8])?)?;
        let head = if next_seq == 0 {
            None
        } else {
            Some(hash_of(&row[8..40])?)
        };
        let frozen = match row[40] {
            0 => false,
            1 => true,
            _ => return Err(corrupt("a chain row has an unknown frozen flag")),
        };
        Ok(Self {
            next_seq,
            head,
            frozen,
        })
    }
}

/// One operator block-list row: the blocked value, when it was added, why,
/// and when it stops being enforced. An `expires_at_ms` of `None` is a
/// permanent block. An expired block stays on the list (the operator's
/// record) but is never enforced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    /// What the block names.
    pub kind: BlockKind,
    /// The blocked value: a 32-byte endpoint key, an IP address, or a topic
    /// tag.
    pub value: Vec<u8>,
    /// When the block was added, in milliseconds since the Unix epoch.
    pub added_at_ms: u64,
    /// The operator's reason for the block.
    pub reason: String,
    /// When the block stops being enforced, in milliseconds since the Unix
    /// epoch, or `None` for a permanent block.
    pub expires_at_ms: Option<u64>,
}

impl Block {
    /// Whether the block is enforced at `now_ms`. Expiry is exclusive: a
    /// block that expires at `t` is no longer enforced at `t`.
    pub fn is_active(&self, now_ms: u64) -> bool {
        self.expires_at_ms.map_or(true, |expiry| expiry > now_ms)
    }

    pub fn key(&self) -> Vec<u8> {
        let mut key = Vec::with_capacity(1 + self.value.len());
        key.push(self.kind.tag());
        key.extend_from_slice(&self.value);
        key
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(8 + 1 + 8 + 4 + self.reason.len());
        out.extend_from_slice(&self.added_at_ms.to_be_bytes());
        match self.expires_at_ms {
            Some(expiry) => {
                out.push(1);
                out.extend_from_slice(&expiry.to_be_bytes());
            }
            None => {
                out.push(0);
                out.extend_from_slice(&0u64.to_be_bytes());
            }
        }
        out.extend_from_slice(&(self.reason.len() as u32).to_be_bytes());
        out.extend_from_slice(self.reason.as_bytes());
        out
    }

    pub fn decode(key: &[u8], value: &[u8]) -> Result<Self, StoreError> {
        let Some((&tag, blocked)) = key.split_first() else {
            return Err(corrupt("a block row has an empty key"));
        };
        let kind = BlockKind::from_tag(tag)
            .ok_or_else(|| corrupt("a block row has an unknown kind tag"))?;
        if value.len() < 8 + 1 + 8 + 4 {
            return Err(corrupt("a block row is too short"));
        }
        let added_at_ms = u64::from_be_bytes(
            value[0..8]
                .try_into()
                .map_err(|_| corrupt("a block row has a truncated timestamp"))?,
        );
        let expires_at_ms = match value[8] {
            0 => None,
            1 => {
                Some(u64::from_be_bytes(value[9..17].try_into().map_err(
                    |_| corrupt("a block row has a truncated expiry"),
                )?))
            }
            _ => return Err(corrupt("a block row has a bad expiry flag")),
        };
        let reason_len = u32::from_be_bytes(
            value[17..21]
                .try_into()
                .map_err(|_| corrupt("a block row has a truncated reason length"))?,
        ) as usize;
        if value.len() != 21 + reason_len {
            return Err(corrupt("a block row has the wrong reason length"));
        }
        let reason = String::from_utf8(value[21..].to_vec())
            .map_err(|_err| corrupt("a block row's reason is not UTF-8"))?;
        Ok(Block {
            kind,
            value: blocked.to_vec(),
            added_at_ms,
            reason,
            expires_at_ms,
        })
    }
}


/// Why an event was refused. A refused event changes nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The bytes are not a valid event. The message names the rule that failed: a
    /// non-canonical encoding, a bad signature, or an extension that breaks its rules.
    Malformed(String),
    /// The event is above its author's fork point, so it never applies.
    Frozen,
    /// The author already holds as many waiting events as the caps allow.
    WaitingFull,
}

impl Refusal {
    /// The reason this refusal gives. A malformed event has none: its bytes do not verify, so it is
    /// not an event the store judged.
    pub const fn reason(&self) -> Option<Reason> {
        match self {
            Self::Malformed(_) => None,
            Self::Frozen => Some(Reason::Frozen),
            Self::WaitingFull => Some(Reason::Waiting),
        }
    }
}

/// What [`EventStore::ingest`] did with one event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ingest {
    /// The event is stored and applied. `released` lists the waiting events that
    /// became applied because of it, in the order they applied.
    Accepted {
        /// The event's ID.
        id: Hash,
        /// The IDs of the waiting events it released, in application order.
        released: Vec<Hash>,
    },
    /// The event is already stored, or it is one of the author's fork proof headers.
    /// Nothing changed.
    Duplicate {
        /// The event's ID.
        id: Hash,
    },
    /// The author signed two different events at one sequence number. The chain is
    /// frozen there, and both headers are stored as the proof. A further event at the
    /// fork point that changes the proof is reported the same way. The headers are present
    /// now, so events of other authors that waited for them may apply. Those are not listed
    /// here. Read them from the store.
    Fork {
        /// The author.
        author: AuthorKey,
        /// The fork point: the lowest sequence number where the author conflicts.
        seq: SeqNum,
    },
    /// The event is held, and not stored. It waits for a dependency, or it is parked because its
    /// backlink can never be the applied head. The hold is on disk, so a restart keeps it. Sync
    /// sends it again if the caps or the age limit drop it.
    Waiting {
        /// The event's ID.
        id: Hash,
    },
    /// The event was refused. Nothing changed.
    Refused(Refusal),
    /// The event is stored but does not apply. It is a group operation the membership layer
    /// voids, or content whose author lost authority at a voided event it names. It still counts
    /// as present, so events that name it and events after it in its author's chain are not held.
    /// `released` lists the waiting events that became applied because of it.
    Voided {
        /// The event's ID.
        id: Hash,
        /// The IDs of the waiting events it released that applied, in application order.
        released: Vec<Hash>,
    },
}

impl Ingest {
    /// The reason this result gives, when it is a refusal or a hold. A stored, applied, or duplicate
    /// event, and a fork that proves a conflict, give none.
    pub const fn reason(&self) -> Option<Reason> {
        match self {
            Self::Refused(refusal) => refusal.reason(),
            Self::Waiting { .. } => Some(Reason::Waiting),
            Self::Voided { .. } => Some(Reason::Voided),
            Self::Accepted { .. } | Self::Duplicate { .. } | Self::Fork { .. } => None,
        }
    }
}

pub fn be_u64(bytes: &[u8]) -> Result<u64, StoreError> {
    let bytes: [u8; 8] = bytes
        .try_into()
        .map_err(|_err| corrupt("a number is not 8 bytes"))?;
    Ok(u64::from_be_bytes(bytes))
}

/// A stored sequence number. Stored as eight bytes, and checked to fit `SeqNum`.
pub fn seq_num(value: u64) -> Result<SeqNum, StoreError> {
    SeqNum::try_from(value).map_err(|_err| corrupt("a sequence number is out of range"))
}

pub fn hash_of(bytes: &[u8]) -> Result<Hash, StoreError> {
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_err| corrupt("a hash is not 32 bytes"))?;
    Ok(Hash::from_bytes(bytes))
}

