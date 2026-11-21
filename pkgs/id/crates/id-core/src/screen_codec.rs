//! Versioned MessagePack encoding for the transport-neutral F12.11 screen patch.

use std::io::Cursor;

use serde::{Deserialize, Serialize};

use crate::world_presentation::ScreenPatch;

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
