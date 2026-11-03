//! Welcome chunking above the frame cap (P4.12).
//!
//! A welcome is the envelope that adds a member, and it grows with the epochs it carries: about 40
//! bytes an epoch with `serde_bytes`. A full welcome passes one world frame of
//! [`MAX_FRAME_BYTES`] at about 26,000 rotations (round 2 `welcome-recent`, extrapolated). Past
//! that, the welcome goes as chunks.
//!
//! [`split`] cuts the encoded envelope into chunks that each fit a frame. Every chunk names its
//! position, the total, the digest of the whole welcome, and a check on its own bytes. The
//! [`Reassembly`] on the joiner's side refuses an altered chunk as it arrives, refuses a chunk out
//! of sequence, and refuses the whole when its digest does not match. The envelope is decoded only
//! after the whole checks out.
//!
//! The receive side is bounded, so a forged chunk stream cannot make a joiner buffer without
//! limit: a chunk carrying more than [`CHUNK_PAYLOAD_BYTES`] is refused before it is buffered,
//! a first chunk declaring more chunks than [`MAX_WELCOME_BYTES`] allows is refused outright,
//! and any chunk that would push the buffered payload over [`MAX_WELCOME_BYTES`] is refused.
//! The buffered-payload bound is what actually caps memory however small the sender's frames
//! are; the declared-total refusal just fails absurd totals fast.

use std::num::TryFromIntError;

use p2panda_core::Hash;
use p2panda_core::cbor::{decode_cbor, encode_cbor};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::ordering::Envelope;
use crate::world_session::MAX_FRAME_BYTES;

/// Bytes a chunk spends on everything but its payload: its position, the total, the two digests
/// and their CBOR framing.
///
/// Generous, so a chunk of payload `frame_cap - CHUNK_HEADER_BYTES` fits a frame of `frame_cap`
/// bytes.
pub const CHUNK_HEADER_BYTES: usize = 256;

/// The payload one chunk carries in a world frame: the frame less its header budget.
pub const CHUNK_PAYLOAD_BYTES: usize = MAX_FRAME_BYTES - CHUNK_HEADER_BYTES;

/// The largest welcome a [`Reassembly`] will buffer, in bytes.
///
/// A welcome grows by about 40 bytes an epoch, so one frame already carries some 26,000
/// rotations; 64 frames' worth is far beyond any plausible epoch history. The bound exists so
/// a forged chunk stream — a first chunk declaring `u32::MAX` chunks, say — cannot make a
/// joiner accumulate payload without limit while the whole-digest check waits for a completion
/// that never comes.
pub const MAX_WELCOME_BYTES: usize = 64 * MAX_FRAME_BYTES;

/// One part of a welcome.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chunk {
    /// This chunk's position, from 0.
    pub(super) index: u32,
    /// How many chunks the welcome has. Never zero.
    pub(super) total: u32,
    /// Digest of the whole encoded envelope.
    pub(super) digest: Hash,
    /// Digest of this chunk's bytes, so an altered chunk is refused as it arrives.
    pub(super) check: Hash,
    #[serde(with = "byte_string")]
    pub(super) bytes: Vec<u8>,
}

/// The payload as one CBOR byte string. A `Vec<u8>` would encode as an array of integers, nearly
/// twice the size.
mod byte_string {
    use std::fmt;

    use serde::de::Visitor;
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(bytes)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        struct Bytes;

        impl Visitor<'_> for Bytes {
            type Value = Vec<u8>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a byte string")
            }

            fn visit_bytes<E: serde::de::Error>(self, value: &[u8]) -> Result<Vec<u8>, E> {
                Ok(value.to_vec())
            }
        }

        deserializer.deserialize_bytes(Bytes)
    }
}

/// A welcome that cannot be split, or that does not reassemble.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum WelcomeError {
    /// The frame is too small to carry a chunk's header and any payload.
    #[error("a frame of {0} bytes cannot carry a welcome chunk")]
    FrameTooSmall(usize),
    /// The welcome has more chunks than a `u32` can count.
    #[error("the welcome has more chunks than a u32 can count: {0}")]
    TooManyChunks(#[source] TryFromIntError),
    /// The envelope could not be encoded.
    #[error("cannot encode the welcome: {0}")]
    Encode(String),
    /// The reassembled bytes are not an envelope.
    #[error("cannot decode the reassembled welcome: {0}")]
    Decode(String),
    /// A chunk's bytes do not match its check.
    #[error("chunk {index} was altered in transit")]
    AlteredChunk {
        /// The position of the altered chunk.
        index: u32,
    },
    /// A chunk arrived where another was due, so a chunk is missing or out of order.
    #[error("chunk {got} arrived where chunk {expected} was due")]
    OutOfSequence {
        /// The position the reassembly waits for.
        expected: u32,
        /// The position of the chunk that arrived.
        got: u32,
    },
    /// A chunk claims no chunks at all.
    #[error("a chunk claims a welcome of no chunks")]
    NoChunks,
    /// A chunk carries more payload than one frame's payload budget, so it could not have come
    /// from [`split`] and is refused before it is buffered.
    #[error("chunk {index} carries {size} payload bytes, over the chunk payload cap")]
    ChunkTooLarge {
        /// The position of the oversized chunk.
        index: u32,
        /// The payload size the chunk carries.
        size: usize,
    },
    /// The welcome is over [`MAX_WELCOME_BYTES`]: either the first chunk declares more
    /// chunks than the byte bound allows, or this chunk would push the buffered payload over it.
    #[error("a welcome of {total} chunks is over the welcome size bound")]
    WelcomeTooLarge {
        /// The total the first chunk declares.
        total: u32,
    },
    /// A chunk names a different welcome from the one being reassembled.
    #[error("the chunk belongs to a different welcome")]
    OtherWelcome,
    /// Every chunk checks, but the whole does not match its digest.
    #[error("the reassembled welcome does not match its digest")]
    AlteredWelcome,
    /// The reassembly ended before the last chunk.
    #[error("the welcome is incomplete: {received} of {total} chunks arrived")]
    Incomplete {
        /// The chunks that arrived.
        received: u32,
        /// The chunks the welcome has, or zero when none arrived.
        total: u32,
    },
}

/// Splits `envelope` into chunks that each fit a world frame of `frame_cap` bytes. An envelope that
/// fits one frame is one chunk, so the joiner takes one path for every welcome (P4.12).
pub fn split(envelope: &Envelope, frame_cap: usize) -> Result<Vec<Chunk>, WelcomeError> {
    let payload_bytes = frame_cap
        .checked_sub(CHUNK_HEADER_BYTES)
        .filter(|bytes| *bytes > 0)
        .ok_or(WelcomeError::FrameTooSmall(frame_cap))?;
    let payload = encode_cbor(envelope).map_err(|err| WelcomeError::Encode(err.to_string()))?;
    let digest = Hash::digest(&payload);
    let parts: Vec<&[u8]> = payload.chunks(payload_bytes).collect();
    let total = u32::try_from(parts.len()).map_err(WelcomeError::TooManyChunks)?;
    let mut chunks = Vec::with_capacity(parts.len());
    for (index, bytes) in parts.into_iter().enumerate() {
        chunks.push(Chunk {
            index: u32::try_from(index).map_err(WelcomeError::TooManyChunks)?,
            total,
            digest,
            check: Hash::digest(bytes),
            bytes: bytes.to_vec(),
        });
    }
    Ok(chunks)
}

/// The chunks of one welcome, as they arrive. Chunks come in order, and each one is checked before
/// it is kept.
#[derive(Debug, Default)]
pub struct Reassembly {
    /// The total and the digest, once the first chunk has been accepted.
    whole: Option<(u32, Hash)>,
    /// How many chunks have been accepted, and so the index the next must have.
    next: u32,
    payload: Vec<u8>,
}

impl Reassembly {
    /// Takes the next chunk. Returns the envelope when the last chunk arrives and the whole matches
    /// its digest. Until then, and on every error, nothing is decoded.
    pub fn push(&mut self, chunk: &Chunk) -> Result<Option<Envelope>, WelcomeError> {
        if chunk.bytes.len() > CHUNK_PAYLOAD_BYTES {
            return Err(WelcomeError::ChunkTooLarge {
                index: chunk.index,
                size: chunk.bytes.len(),
            });
        }
        if Hash::digest(&chunk.bytes) != chunk.check {
            return Err(WelcomeError::AlteredChunk { index: chunk.index });
        }
        if chunk.total == 0 {
            return Err(WelcomeError::NoChunks);
        }
        // A first chunk declaring more chunks than the byte bound allows is refused outright:
        // every chunk from [`split`] carries at least one payload byte, so such a total is over
        // [`MAX_WELCOME_BYTES`] whatever frame size the sender used.
        if self.whole.is_none() && u64::from(chunk.total) > MAX_WELCOME_BYTES as u64 {
            return Err(WelcomeError::WelcomeTooLarge { total: chunk.total });
        }
        if let Some((total, digest)) = self.whole
            && (chunk.total != total || chunk.digest != digest)
        {
            return Err(WelcomeError::OtherWelcome);
        }
        let expected = self.next;
        if chunk.index != expected || expected >= chunk.total {
            return Err(WelcomeError::OutOfSequence {
                expected,
                got: chunk.index,
            });
        }
        // The tight bound, checked on every chunk right before buffering: whatever the declared
        // total, the joiner never holds more than [`MAX_WELCOME_BYTES`] of payload.
        if self.payload.len() + chunk.bytes.len() > MAX_WELCOME_BYTES {
            return Err(WelcomeError::WelcomeTooLarge { total: chunk.total });
        }

        self.whole = Some((chunk.total, chunk.digest));
        self.payload.extend_from_slice(&chunk.bytes);
        self.next += 1;
        if self.next < chunk.total {
            return Ok(None);
        }

        if Hash::digest(&self.payload) != chunk.digest {
            return Err(WelcomeError::AlteredWelcome);
        }
        decode_cbor(self.payload.as_slice())
            .map(Some)
            .map_err(|err| WelcomeError::Decode(err.to_string()))
    }

    /// Ends the reassembly, and fails if the welcome is not complete.
    pub fn finish(self) -> Result<(), WelcomeError> {
        let total = self.whole.map_or(0, |(total, _)| total);
        if self.next == 0 || self.next < total {
            return Err(WelcomeError::Incomplete {
                received: self.next,
                total,
            });
        }
        Ok(())
    }
}
