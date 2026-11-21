//! Versioned MessagePack encoding for the transport-neutral F12.11 screen patch.

use std::io::Cursor;

use serde::{Deserialize, Serialize};

use crate::world_presentation::ScreenPatch;

/// The logical contents of a binary `view` patch frame.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct ScreenPatchFrame {
    /// Existing world-event cursor; independent of screen revisions.
    pub sequence: u64,
    /// Revision produced by applying this patch.
    pub view_revision: u64,
    /// Exact screen revision this patch applies to.
    pub base_view_revision: u64,
    /// Optional token for resuming from this target revision.
    pub resume_token: Option<String>,
    /// Canonical transport-neutral screen delta.
    pub patch: ScreenPatch,
}

/// Version of the MessagePack screen-patch wire representation.
pub const SCREEN_CODEC_VERSION: u8 = 1;
/// Maximum complete logical frame size, matching the session frame limit.
pub const MAX_SCREEN_FRAME_BYTES: usize = 1024 * 1024;

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct VersionedPatch<'a> {
    version: u8,
    #[serde(borrow)]
    patch: &'a ScreenPatch,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OwnedVersionedPatch {
    version: u8,
    patch: ScreenPatch,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct VersionedViewPatch<'a> {
    version: u8,
    #[serde(rename = "type")]
    frame_type: &'static str,
    sequence: u64,
    view_revision: u64,
    base_view_revision: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    resume_token: Option<&'a str>,
    patch: &'a ScreenPatch,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OwnedVersionedViewPatch {
    version: u8,
    #[serde(rename = "type")]
    frame_type: String,
    sequence: u64,
    view_revision: u64,
    base_view_revision: u64,
    #[serde(default)]
    resume_token: Option<String>,
    patch: ScreenPatch,
}

/// Errors produced while encoding or decoding a versioned screen patch.
#[derive(Debug, thiserror::Error)]
pub enum ScreenCodecError {
    /// The encoded frame exceeds the shared session frame limit.
    #[error("screen frame exceeds the 1 MiB limit")]
    TooLarge,
    /// The payload is not exactly one valid MessagePack value.
    #[error("invalid MessagePack screen frame: {0}")]
    Invalid(#[from] rmp_serde::decode::Error),
    /// A serialization error occurred while encoding a screen patch.
    #[error("could not encode MessagePack screen frame: {0}")]
    Encode(#[from] rmp_serde::encode::Error),
    /// The frame uses a codec version this implementation does not support.
    #[error("unsupported screen codec version {0}")]
    UnsupportedVersion(u8),
    /// The MessagePack value has trailing bytes after its one complete value.
    #[error("trailing bytes after screen frame")]
    TrailingBytes,
    /// The binary frame is not the supported `view` frame kind.
    #[error("invalid binary screen frame type")]
    InvalidFrameType,
}

/// Encodes a patch as one versioned MessagePack value.
pub fn encode_patch(patch: &ScreenPatch) -> Result<Vec<u8>, ScreenCodecError> {
    let bytes = rmp_serde::to_vec_named(&VersionedPatch {
        version: SCREEN_CODEC_VERSION,
        patch,
    })?;
    if bytes.len() > MAX_SCREEN_FRAME_BYTES {
        return Err(ScreenCodecError::TooLarge);
    }
    Ok(bytes)
}

/// Decodes one bounded, versioned MessagePack patch value.
pub fn decode_patch(bytes: &[u8]) -> Result<ScreenPatch, ScreenCodecError> {
    if bytes.len() > MAX_SCREEN_FRAME_BYTES {
        return Err(ScreenCodecError::TooLarge);
    }
    let mut deserializer = rmp_serde::Deserializer::new(Cursor::new(bytes));
    let frame = <OwnedVersionedPatch as Deserialize>::deserialize(&mut deserializer)?;
    if usize::try_from(deserializer.into_inner().position()).ok() != Some(bytes.len()) {
        return Err(ScreenCodecError::TrailingBytes);
    }
    if frame.version != SCREEN_CODEC_VERSION {
        return Err(ScreenCodecError::UnsupportedVersion(frame.version));
    }
    Ok(frame.patch)
}

/// Encodes a complete binary session view-patch frame, including the event
/// cursor and screen revision metadata needed to safely apply the patch.
pub fn encode_view_patch(
    sequence: u64,
    view_revision: u64,
    base_view_revision: u64,
    resume_token: Option<&str>,
    patch: &ScreenPatch,
) -> Result<Vec<u8>, ScreenCodecError> {
    let bytes = rmp_serde::to_vec_named(&VersionedViewPatch {
        version: SCREEN_CODEC_VERSION,
        frame_type: "view",
        sequence,
        view_revision,
        base_view_revision,
        resume_token,
        patch,
    })?;
    if bytes.len() > MAX_SCREEN_FRAME_BYTES {
        return Err(ScreenCodecError::TooLarge);
    }
    Ok(bytes)
}

/// Decodes one bounded binary session view-patch frame.
pub fn decode_view_patch(bytes: &[u8]) -> Result<ScreenPatchFrame, ScreenCodecError> {
    if bytes.len() > MAX_SCREEN_FRAME_BYTES {
        return Err(ScreenCodecError::TooLarge);
    }
    let mut deserializer = rmp_serde::Deserializer::new(Cursor::new(bytes));
    let frame = <OwnedVersionedViewPatch as Deserialize>::deserialize(&mut deserializer)?;
    if usize::try_from(deserializer.into_inner().position()).ok() != Some(bytes.len()) {
        return Err(ScreenCodecError::TrailingBytes);
    }
    if frame.version != SCREEN_CODEC_VERSION {
        return Err(ScreenCodecError::UnsupportedVersion(frame.version));
    }
    if frame.frame_type != "view" {
        return Err(ScreenCodecError::InvalidFrameType);
    }
    Ok(ScreenPatchFrame {
        sequence: frame.sequence,
        view_revision: frame.view_revision,
        base_view_revision: frame.base_view_revision,
        resume_token: frame.resume_token,
        patch: frame.patch,
    })
}
