//! Media artifacts as signed events over content hashes (issue #2342; taxonomy §9.3
//! decision accepted 2026-11-16, pending-decisions ledger item artifact-storage-2342:
//! the proposal's defaults on all four open questions).
//!
//! A media artifact is a `media.put` world event, a sibling of `document.put`
//! ([`crate::documents`]), not an extension of `records` and not of the signed-statement
//! chains in `artifact.rs`. Its header carries a [`MediaRef`]: the media type, the
//! BLAKE3 hash of the content (decision 1: BLAKE3 only, no stored sha256), its length,
//! its dimensions or duration, and its creation time. The event's ID is the artifact's
//! ID. The bytes are never in the event (events are capped at 64 KiB); they are a blob
//! in the same iroh-blobs store documents use, moved on request through the blob gate,
//! and a replica that stores fetched bytes checks them with
//! [`MediaRef::verify_content`] first. A surface renders the reference only; bytes
//! never travel in a surface.
//!
//! Dedupe is content addressing: identical bytes are one blob, stored once per node,
//! however many `media.put` events name their hash. The refcount is derived, never
//! stored: the number of live references in folded world state (the blob gate's
//! reference index counts media and document references together). A `media.remove`
//! event (author-only effect, mirroring `document.remove`) drops the reference and
//! never deletes bytes. A blob whose derived refcount reaches zero becomes
//! GC-eligible after [`GC_GRACE_MS`] (decision 3: 30 days) and is deleted only by the
//! node's own store maintenance — GC is local store state, not an event, so replay
//! never resurrects bytes.
//!
//! Oversize is refused at the header, before any bytes move: a reference naming more
//! than [`MAX_MEDIA_BYTES`] (decision 2: 256 MiB, a code constant beside the event
//! cap) is refused as `bad_args` + `oversize`. A device or link with a lower ceiling
//! refuses earlier with its own ceiling (`outside_ceiling`).

use p2panda_core::Hash;
use serde::{Deserialize, Serialize};
use unicode_normalization::is_nfc;

use id_core::waiting::AuthorKey;

use crate::documents::MAX_MEDIA_TYPE_BYTES;

/// The kind of a `media.put` event, as the header's `kind` names it.
pub const MEDIA_KIND: &str = "media.put";

/// The kind of a `media.remove` event, which names the artifact it removes in its header.
pub const MEDIA_REMOVAL_KIND: &str = "media.remove";

/// The most bytes a media artifact may name: 256 MiB, a code constant beside the
/// 64 KiB event cap. Content over it is refused with `bad_args` + `oversize`.
pub const MAX_MEDIA_BYTES: u64 = 256 * 1024 * 1024;

/// The grace period between a blob's derived refcount reaching zero and the blob
/// becoming GC-eligible: 30 days, in milliseconds.
pub const GC_GRACE_MS: u64 = 30 * 24 * 60 * 60 * 1000;

/// The most bytes in one `media.stream` frame: the 64 KiB event cap
/// (`MAX_EVENT_BYTES` in `id-caps`).
pub const MAX_STREAM_FRAME_BYTES: usize = 64 * 1024;

/// Which typed media capability produced an artifact.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaSort {
    /// `media.image`
    Image,
    /// `media.audio`
    Audio,
    /// `media.video`
    Video,
}

impl MediaSort {
    /// The capability name that produces this sort.
    #[must_use]
    pub fn cap_name(self) -> &'static str {
        match self {
            MediaSort::Image => "media.image",
            MediaSort::Audio => "media.audio",
            MediaSort::Video => "media.video",
        }
    }

    /// The media-type prefix this sort's content must carry.
    #[must_use]
    pub fn media_type_prefix(self) -> &'static str {
        match self {
            MediaSort::Image => "image/",
            MediaSort::Audio => "audio/",
            MediaSort::Video => "video/",
        }
    }
}

/// A media artifact's content, as its `media.put` event names it. The taxonomy §3.4
/// reference shape is `{artifact_id, mime, bytes, digest, width/height or
/// duration_ms, created_at}`: the artifact_id is the event's ID, and the digest is
/// the BLAKE3 `content` hash (accepted decision 1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaRef {
    /// Which typed capability produced the artifact.
    pub sort: MediaSort,
    /// The media type of the content, for example `image/png`. NFC, 1 to 128 bytes,
    /// under the sort's prefix.
    pub media_type: String,
    /// The BLAKE3 hash of the content. It is also the blob's hash.
    pub content: Hash,
    /// The content's length in bytes, at most [`MAX_MEDIA_BYTES`].
    pub size: u64,
    /// The width in pixels, for image and video content.
    pub width: Option<u32>,
    /// The height in pixels, for image and video content.
    pub height: Option<u32>,
    /// The duration in milliseconds, for audio and video content.
    pub duration_ms: Option<u64>,
    /// When the artifact was created, in milliseconds since the Unix epoch.
    pub created_at: u64,
}

/// Why a media artifact was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum MediaError {
    /// The media type is empty, longer than the documents media-type cap, or not NFC.
    #[error("{0}")]
    MediaType(String),
    /// The content is over [`MAX_MEDIA_BYTES`]: the `bad_args` + `oversize` refusal,
    /// raised at the header before any bytes move.
    #[error(
        "the content is {named} bytes, the most a media artifact may name is {max} (bad_args + oversize)"
    )]
    Oversize {
        /// The length the reference names.
        named: u64,
        /// The bound it exceeds.
        max: u64,
    },
    /// The reference is inconsistent: the media type is outside the sort's prefix,
    /// dimensions are half-given or on audio, or a duration is on an image.
    #[error("{0}")]
    BadArgs(String),
    /// The content is not the length the artifact names.
    #[error("the content is {actual} bytes, the artifact names {named}")]
    Size {
        /// The length the artifact names.
        named: u64,
        /// The length of the content.
        actual: u64,
    },
    /// The content does not hash to the artifact's content hash.
    #[error("the content does not hash to the artifact's content hash")]
    Hash,
}

/// Whether `kind` is the media kind, so its event carries a media artifact.
#[must_use]
pub fn is_media_kind(kind: &str) -> bool {
    kind == MEDIA_KIND
}

/// Whether `kind` is the media removal kind, so its event carries the ID of the
/// artifact it removes.
#[must_use]
pub fn is_media_removal_kind(kind: &str) -> bool {
    kind == MEDIA_REMOVAL_KIND
}

impl MediaRef {
    /// The reference to `bytes`, as an artifact of `sort` and `media_type`.
    ///
    /// # Errors
    ///
    /// Returns [`MediaError::Oversize`] when `bytes` is over [`MAX_MEDIA_BYTES`],
    /// and the [`MediaRef::check`] errors for an inconsistent reference.
    #[allow(clippy::too_many_arguments)]
    pub fn for_bytes(
        sort: MediaSort,
        media_type: &str,
        bytes: &[u8],
        width: Option<u32>,
        height: Option<u32>,
        duration_ms: Option<u64>,
        created_at: u64,
    ) -> Result<Self, MediaError> {
        let media = Self {
            sort,
            media_type: media_type.to_owned(),
            content: Hash::digest(bytes),
            size: len_of(bytes),
            width,
            height,
            duration_ms,
            created_at,
        };
        media.check()?;
        Ok(media)
    }

    /// Checks the rules every media reference meets. The builder and the decoder
    /// both apply them, so a signed artifact always passes them on receipt. The
    /// size bound is checked first: oversize is refused at the header, before any
    /// bytes move.
    ///
    /// # Errors
    ///
    /// Returns [`MediaError::Oversize`] for a size over [`MAX_MEDIA_BYTES`],
    /// [`MediaError::MediaType`] for a media type that is empty, too long, or not
    /// NFC, and [`MediaError::BadArgs`] for a sort/media-type mismatch or
    /// dimensions/duration on the wrong sort.
    pub fn check(&self) -> Result<(), MediaError> {
        if self.size > MAX_MEDIA_BYTES {
            return Err(MediaError::Oversize {
                named: self.size,
                max: MAX_MEDIA_BYTES,
            });
        }
        let length = self.media_type.len();
        if !(1..=MAX_MEDIA_TYPE_BYTES).contains(&length) {
            return Err(MediaError::MediaType(format!(
                "the media type is {length} bytes, it must be 1 to {MAX_MEDIA_TYPE_BYTES}"
            )));
        }
        if !is_nfc(&self.media_type) {
            return Err(MediaError::MediaType(
                "the media type is not NFC".to_owned(),
            ));
        }
        if !self.media_type.starts_with(self.sort.media_type_prefix()) {
            return Err(MediaError::BadArgs(format!(
                "the media type {} is not {} content",
                self.media_type,
                self.sort.media_type_prefix()
            )));
        }
        match (self.width, self.height) {
            (Some(0), _) | (_, Some(0)) => {
                return Err(MediaError::BadArgs(
                    "a dimension of zero is not a dimension".to_owned(),
                ));
            }
            (Some(_), None) | (None, Some(_)) => {
                return Err(MediaError::BadArgs(
                    "width and height come together or not at all".to_owned(),
                ));
            }
            (Some(_), Some(_)) if self.sort == MediaSort::Audio => {
                return Err(MediaError::BadArgs(
                    "audio content has no dimensions".to_owned(),
                ));
            }
            _ => {}
        }
        if self.duration_ms.is_some() && self.sort == MediaSort::Image {
            return Err(MediaError::BadArgs(
                "image content has no duration".to_owned(),
            ));
        }
        Ok(())
    }

    /// Checks `bytes` against the reference: the length first, then the hash.
    ///
    /// # Errors
    ///
    /// Returns [`MediaError::Size`] when the length differs, and
    /// [`MediaError::Hash`] when the bytes do not hash to the content hash.
    pub fn verify_content(&self, bytes: &[u8]) -> Result<(), MediaError> {
        let actual = len_of(bytes);
        if actual != self.size {
            return Err(MediaError::Size {
                named: self.size,
                actual,
            });
        }
        if Hash::digest(bytes) != self.content {
            return Err(MediaError::Hash);
        }
        Ok(())
    }

    /// The content hash as an iroh-blobs hash. It names the blob that holds the content.
    #[must_use]
    pub fn blob_hash(&self) -> iroh_blobs::Hash {
        iroh_blobs::Hash::from_bytes(*self.content.as_bytes())
    }
}

/// The length of `bytes`. A slice is never longer than `u64::MAX` bytes.
fn len_of(bytes: &[u8]) -> u64 {
    u64::try_from(bytes.len()).unwrap_or(u64::MAX)
}

/// A media artifact in the store: its `media.put` event, the key that signed it,
/// its content reference, and the removal the author made of it, if any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaRecord {
    /// The artifact's ID: the `media.put` event's ID.
    pub id: Hash,
    /// The key that signed the media event.
    pub author: AuthorKey,
    /// The content the event names. Removal keeps it, as with documents.
    pub media: MediaRef,
    /// The ID of the `media.remove` event the author signed for this artifact, if
    /// the author has removed it.
    pub removal: Option<Hash>,
}

/// The derived refcount of the blob `content`: the number of live (not removed)
/// media references to it in folded world state. The count is computed from the
/// records, never stored; the blob gate's reference index adds document
/// references to the same count, so GC is safe across both.
#[must_use]
pub fn derived_refcount<'a>(
    records: impl IntoIterator<Item = &'a MediaRecord>,
    content: Hash,
) -> usize {
    records
        .into_iter()
        .filter(|record| record.media.content == content && record.removal.is_none())
        .count()
}

/// Whether a blob whose derived refcount reached zero at `zero_ref_since_ms`
/// (milliseconds since the Unix epoch) is GC-eligible at `now_ms`: the full
/// [`GC_GRACE_MS`] grace period has elapsed. GC itself is local store
/// maintenance, never an event.
#[must_use]
pub fn gc_eligible(zero_ref_since_ms: u64, now_ms: u64) -> bool {
    now_ms >= zero_ref_since_ms.saturating_add(GC_GRACE_MS)
}

/// How a `media.stream` ended: exactly one end frame, either the assembled bytes
/// ingested as an artifact or the stream discarded. The stream itself is never
/// the permanent record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamEnd {
    /// The stream terminated into this artifact: the assembled bytes went
    /// through ingest (hash computed and verified, `media.put` event written).
    Artifact(MediaRef),
    /// The stream was discarded with this reason. A disconnect mid-stream is an
    /// implicit discard, and no partial artifact exists.
    Discarded(String),
}

/// Why a `media.stream` operation was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum StreamError {
    /// A frame is over [`MAX_STREAM_FRAME_BYTES`]: `bad_args` + `oversize` at the
    /// frame ceiling.
    #[error("a stream frame is {size} bytes, the most allowed is {max} (bad_args + oversize)")]
    FrameTooLarge {
        /// The frame's length.
        size: usize,
        /// The frame ceiling.
        max: usize,
    },
    /// The subscriber's credit window is closed: the publisher pauses. A
    /// subscriber that stays closed is terminated as `outside_ceiling`, never
    /// buffered without bound.
    #[error("the subscriber's credit window is closed; the publisher must pause")]
    Backpressure,
    /// The stream already ended; it takes exactly one end frame.
    #[error("the stream has already ended")]
    AlreadyEnded,
    /// The assembled content failed artifact ingest.
    #[error("{0}")]
    Media(#[from] MediaError),
}

/// One `media.stream` subscription, subscriber side: frames of at most
/// [`MAX_STREAM_FRAME_BYTES`] arrive under a credit window, and the stream ends
/// with exactly one explicit end — into an artifact or discarded.
#[derive(Clone, Debug)]
pub struct MediaStream {
    sort: MediaSort,
    media_type: String,
    width: Option<u32>,
    height: Option<u32>,
    duration_ms: Option<u64>,
    created_at: u64,
    credit: usize,
    buffered: Vec<u8>,
    end: Option<StreamEnd>,
}

impl MediaStream {
    /// A stream for content of `sort` and `media_type`, opening with a credit
    /// window of `credit_window` frames.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        sort: MediaSort,
        media_type: &str,
        width: Option<u32>,
        height: Option<u32>,
        duration_ms: Option<u64>,
        created_at: u64,
        credit_window: usize,
    ) -> Self {
        Self {
            sort,
            media_type: media_type.to_owned(),
            width,
            height,
            duration_ms,
            created_at,
            credit: credit_window,
            buffered: Vec::new(),
            end: None,
        }
    }

    /// Grants `frames` more frames of credit, reopening the window.
    pub fn grant_credit(&mut self, frames: usize) {
        self.credit = self.credit.saturating_add(frames);
    }

    /// The frames of credit remaining before the publisher must pause.
    #[must_use]
    pub fn credit(&self) -> usize {
        self.credit
    }

    /// Accepts one frame. The frame is checked against the ceiling before the
    /// window, so an oversize frame is refused even by a closed window.
    ///
    /// # Errors
    ///
    /// Returns [`StreamError::AlreadyEnded`] after the end frame,
    /// [`StreamError::FrameTooLarge`] for a frame over
    /// [`MAX_STREAM_FRAME_BYTES`], and [`StreamError::Backpressure`] when the
    /// credit window is closed.
    pub fn push_frame(&mut self, frame: &[u8]) -> Result<(), StreamError> {
        if self.end.is_some() {
            return Err(StreamError::AlreadyEnded);
        }
        if frame.len() > MAX_STREAM_FRAME_BYTES {
            return Err(StreamError::FrameTooLarge {
                size: frame.len(),
                max: MAX_STREAM_FRAME_BYTES,
            });
        }
        if self.credit == 0 {
            return Err(StreamError::Backpressure);
        }
        self.credit -= 1;
        self.buffered.extend_from_slice(frame);
        Ok(())
    }

    /// The bytes assembled so far. They are not an artifact and are not a
    /// record: only [`MediaStream::finish_artifact`] makes them one.
    #[must_use]
    pub fn buffered_bytes(&self) -> &[u8] {
        &self.buffered
    }

    /// The end frame, once the stream has ended.
    #[must_use]
    pub fn end(&self) -> Option<&StreamEnd> {
        self.end.as_ref()
    }

    /// Whether the stream has ended.
    #[must_use]
    pub fn is_terminated(&self) -> bool {
        self.end.is_some()
    }

    /// Ends the stream into an artifact: the assembled bytes go through ingest
    /// (hash computed, oversize refused) and the resulting reference is the end
    /// frame.
    ///
    /// # Errors
    ///
    /// Returns [`StreamError::AlreadyEnded`] after an end frame, and
    /// [`StreamError::Media`] when the assembled content fails ingest.
    pub fn finish_artifact(&mut self) -> Result<MediaRef, StreamError> {
        if self.end.is_some() {
            return Err(StreamError::AlreadyEnded);
        }
        let media = MediaRef::for_bytes(
            self.sort,
            &self.media_type,
            &self.buffered,
            self.width,
            self.height,
            self.duration_ms,
            self.created_at,
        )?;
        self.end = Some(StreamEnd::Artifact(media.clone()));
        Ok(media)
    }

    /// Ends the stream as discarded with `reason`, dropping the assembled
    /// bytes: no partial artifact exists. A subscriber that stays closed is
    /// terminated this way with reason `outside_ceiling`.
    ///
    /// # Errors
    ///
    /// Returns [`StreamError::AlreadyEnded`] after an end frame.
    pub fn finish_discarded(&mut self, reason: &str) -> Result<(), StreamError> {
        if self.end.is_some() {
            return Err(StreamError::AlreadyEnded);
        }
        self.buffered.clear();
        self.end = Some(StreamEnd::Discarded(reason.to_owned()));
        Ok(())
    }

    /// A disconnect mid-stream: an implicit discard, if the stream has not
    /// already ended.
    pub fn disconnect(&mut self) {
        if self.end.is_none() {
            self.buffered.clear();
            self.end = Some(StreamEnd::Discarded("disconnect".to_owned()));
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn image_ref(bytes: &[u8]) -> MediaRef {
        MediaRef::for_bytes(
            MediaSort::Image,
            "image/png",
            bytes,
            Some(2),
            Some(3),
            None,
            1_700_000_000_000,
        )
        .unwrap()
    }

    #[test]
    fn test_a_typed_artifact_is_read_back_by_its_digest() {
        let body = b"fake png bytes";
        let media = image_ref(body);
        assert_eq!(media.sort.cap_name(), "media.image");
        assert!(media.verify_content(body).is_ok());
        // Same length, one byte changed: the hash refuses it.
        assert_eq!(
            media.verify_content(b"fake png bytez"),
            Err(MediaError::Hash)
        );
        // A prefix: the length refuses it before any hash is taken.
        assert!(matches!(
            media.verify_content(&body[..body.len() - 1]),
            Err(MediaError::Size { .. })
        ));
        assert_eq!(media.blob_hash(), iroh_blobs::Hash::new(body));
    }

    #[test]
    fn test_oversize_is_refused_at_the_header_before_bytes_move() {
        // A reference naming over MAX_MEDIA_BYTES is refused by check() alone:
        // no bytes are ever transferred for it.
        let media = MediaRef {
            size: MAX_MEDIA_BYTES + 1,
            ..image_ref(b"x")
        };
        assert_eq!(
            media.check(),
            Err(MediaError::Oversize {
                named: MAX_MEDIA_BYTES + 1,
                max: MAX_MEDIA_BYTES,
            })
        );
        // Exactly at the bound passes the size check.
        let at_bound = MediaRef {
            size: MAX_MEDIA_BYTES,
            ..image_ref(b"x")
        };
        assert!(at_bound.check().is_ok());
    }

    #[test]
    fn test_the_sort_and_the_media_type_must_agree() {
        assert!(matches!(
            MediaRef::for_bytes(MediaSort::Image, "audio/ogg", b"x", None, None, None, 0),
            Err(MediaError::BadArgs(_))
        ));
        assert!(matches!(
            MediaRef::for_bytes(
                MediaSort::Audio,
                "audio/ogg",
                b"x",
                Some(2),
                Some(3),
                Some(10),
                0
            ),
            Err(MediaError::BadArgs(_))
        ));
        assert!(matches!(
            MediaRef::for_bytes(
                MediaSort::Image,
                "image/png",
                b"x",
                Some(2),
                Some(3),
                Some(10),
                0
            ),
            Err(MediaError::BadArgs(_))
        ));
        assert!(
            MediaRef::for_bytes(MediaSort::Audio, "audio/ogg", b"x", None, None, Some(10), 0)
                .is_ok()
        );
        assert!(
            MediaRef::for_bytes(
                MediaSort::Video,
                "video/mp4",
                b"x",
                Some(2),
                Some(3),
                Some(10),
                0
            )
            .is_ok()
        );
    }

    #[test]
    fn test_the_refcount_is_derived_and_removal_drops_it() {
        let body = b"shared bytes";
        let media = image_ref(body);
        let record = |id: u8, removal: Option<u8>| MediaRecord {
            id: Hash::digest(&[id]),
            author: [7u8; 32],
            media: media.clone(),
            removal: removal.map(|r| Hash::digest(&[r])),
        };
        let records = vec![record(1, None), record(2, None), record(3, Some(9))];
        assert_eq!(derived_refcount(&records, media.content), 2);
        assert_eq!(derived_refcount(&records, Hash::digest(b"other bytes")), 0);
    }

    #[test]
    fn test_gc_waits_the_full_grace_period() {
        let zero_at = 1_000_000u64;
        assert!(!gc_eligible(zero_at, zero_at + GC_GRACE_MS - 1));
        assert!(gc_eligible(zero_at, zero_at + GC_GRACE_MS));
    }

    #[test]
    fn test_a_stream_frame_is_capped_at_the_event_ceiling() {
        let mut stream = MediaStream::new(MediaSort::Video, "video/mp4", None, None, None, 0, 4);
        let frame = vec![0u8; MAX_STREAM_FRAME_BYTES];
        assert!(stream.push_frame(&frame).is_ok());
        let over = vec![0u8; MAX_STREAM_FRAME_BYTES + 1];
        assert_eq!(
            stream.push_frame(&over),
            Err(StreamError::FrameTooLarge {
                size: MAX_STREAM_FRAME_BYTES + 1,
                max: MAX_STREAM_FRAME_BYTES,
            })
        );
    }

    #[test]
    fn test_a_closed_credit_window_backpressures_the_publisher() {
        let mut stream = MediaStream::new(MediaSort::Audio, "audio/ogg", None, None, None, 0, 1);
        assert!(stream.push_frame(b"one").is_ok());
        assert_eq!(stream.push_frame(b"two"), Err(StreamError::Backpressure));
        stream.grant_credit(1);
        assert!(stream.push_frame(b"two").is_ok());
        assert_eq!(stream.buffered_bytes(), b"onetwo");
    }

    #[test]
    fn test_a_stream_terminates_into_an_artifact_or_a_discard_never_a_record() {
        let mut stream =
            MediaStream::new(MediaSort::Image, "image/png", Some(2), Some(3), None, 42, 4);
        stream.push_frame(b"fake ").unwrap();
        stream.push_frame(b"png bytes").unwrap();
        // Before the end frame there is no artifact: the stream is not the record.
        assert!(stream.end().is_none());
        let media = stream.finish_artifact().unwrap();
        assert!(media.verify_content(b"fake png bytes").is_ok());
        assert_eq!(stream.end(), Some(&StreamEnd::Artifact(media)));
        // Exactly one end frame.
        assert_eq!(
            stream.finish_discarded("late"),
            Err(StreamError::AlreadyEnded)
        );
        assert_eq!(stream.push_frame(b"x"), Err(StreamError::AlreadyEnded));

        // A disconnect mid-stream is an implicit discard with no partial artifact.
        let mut dropped = MediaStream::new(MediaSort::Image, "image/png", None, None, None, 0, 4);
        dropped.push_frame(b"partial").unwrap();
        dropped.disconnect();
        assert_eq!(
            dropped.end(),
            Some(&StreamEnd::Discarded("disconnect".to_owned()))
        );
        assert!(dropped.buffered_bytes().is_empty());
    }
}
