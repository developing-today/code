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

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{decode_patch, encode_patch};
    use crate::world_presentation::{CellRun, ScreenCell, ScreenPatch, StyleToken};

    fn screen_cell() -> impl Strategy<Value = ScreenCell> {
        (
            prop_oneof![
                Just(String::new()),
                Just("a".to_owned()),
                Just("Ω".to_owned()),
                Just("水".to_owned()),
                Just("🧭".to_owned()),
                Just("e\u{301}".to_owned()),
            ],
            any::<u8>(),
            any::<u8>(),
            any::<bool>(),
            0_u8..=2,
            prop_oneof![
                Just(None),
                Just(Some(StyleToken::Emphasis)),
                Just(Some(StyleToken::Muted)),
                Just(Some(StyleToken::Heading)),
                Just(Some(StyleToken::Code)),
                Just(Some(StyleToken::Warning)),
                Just(Some(StyleToken::Success)),
                Just(Some(StyleToken::Link)),
            ],
        )
            .prop_map(|(symbol, fg, bg, bold, columns, token)| ScreenCell {
                symbol,
                fg,
                bg,
                bold,
                columns,
                token,
            })
    }

    fn populated_patch() -> impl Strategy<Value = ScreenPatch> {
        (
            any::<u8>(),
            36_u16..=120,
            1_u16..=50,
            prop::collection::vec(prop::collection::vec(screen_cell(), 1..=8), 1..=4),
        )
            .prop_map(|(version, width, height, cell_runs)| {
                let mut next_start = 0_u32;
                let runs = cell_runs
                    .into_iter()
                    .map(|cells| {
                        let run = CellRun {
                            start: next_start,
                            cells,
                        };
                        // Leave one unchanged cell between runs. The strategy's
                        // bounds keep every run inside the minimum width of 36.
                        next_start += u32::try_from(run.cells.len()).unwrap() + 1;
                        run
                    })
                    .collect();
                ScreenPatch {
                    version,
                    width,
                    height,
                    runs,
                }
            })
    }

    proptest! {
        #[test]
        fn populated_screen_patches_round_trip_through_messagepack(patch in populated_patch()) {
            let encoded = encode_patch(&patch).unwrap();
            prop_assert_eq!(decode_patch(&encoded).unwrap(), patch);
        }
    }
}
