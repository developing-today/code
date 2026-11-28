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

/// One negotiated binary view frame (F12.11 phase 3, #12242): the
/// `ScreenPatch` a `view` server frame carries, plus the envelope
/// fields that frame carries alongside the patch — the world-event
/// `sequence`, the target `view_revision`, the `base_view_revision`
/// the patch applies to, and the resume token naming the target
/// revision. A session sends this instead of the JSON `view` frame
/// only after the client negotiated binary patches at join; the
/// world-event cursor and view-revision semantics are unchanged.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewPatchFrame {
    /// World-event cursor this view follows.
    pub sequence: u64,
    /// The session view revision this patch produces.
    pub view_revision: u64,
    /// The view revision the patch applies to.
    pub base_view_revision: Option<u64>,
    /// Resume token naming `view_revision`, when one is issued.
    pub resume_token: Option<String>,
    /// The screen delta itself.
    pub patch: ScreenPatch,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct VersionedViewPatch<'a> {
    version: u8,
    #[serde(borrow)]
    frame: &'a ViewPatchFrame,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OwnedVersionedViewPatch {
    version: u8,
    frame: ViewPatchFrame,
}

/// Encodes a view frame as one versioned MessagePack value.
pub fn encode_view_patch(frame: &ViewPatchFrame) -> Result<Vec<u8>, ScreenCodecError> {
    let bytes = rmp_serde::to_vec_named(&VersionedViewPatch {
        version: SCREEN_CODEC_VERSION,
        frame,
    })?;
    if bytes.len() > MAX_SCREEN_FRAME_BYTES {
        return Err(ScreenCodecError::TooLarge);
    }
    Ok(bytes)
}

/// Decodes one bounded, versioned MessagePack view frame.
pub fn decode_view_patch(bytes: &[u8]) -> Result<ViewPatchFrame, ScreenCodecError> {
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
    Ok(frame.frame)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{
        MAX_SCREEN_FRAME_BYTES, ScreenCodecError, ViewPatchFrame, decode_patch, decode_view_patch,
        encode_patch, encode_view_patch,
    };
    use crate::world_presentation::{CellRun, ScreenCell, ScreenPatch, StyleToken};

    const GOLDEN_VIEW_PATCH: &[u8] = &[
        0x82, 0xa7, b'v', b'e', b'r', b's', b'i', b'o', b'n', 0x01, 0xa5, b'f', b'r', b'a', b'm',
        b'e', 0x85, 0xa8, b's', b'e', b'q', b'u', b'e', b'n', b'c', b'e', 0x07, 0xad, b'v', b'i',
        b'e', b'w', b'_', b'r', b'e', b'v', b'i', b's', b'i', b'o', b'n', 0x03, 0xb2, b'b', b'a',
        b's', b'e', b'_', b'v', b'i', b'e', b'w', b'_', b'r', b'e', b'v', b'i', b's', b'i', b'o',
        b'n', 0x02, 0xac, b'r', b'e', b's', b'u', b'm', b'e', b'_', b't', b'o', b'k', b'e', b'n',
        0xc0, 0xa5, b'p', b'a', b't', b'c', b'h', 0x84, 0xa7, b'v', b'e', b'r', b's', b'i', b'o',
        b'n', 0x01, 0xa5, b'w', b'i', b'd', b't', b'h', 0x02, 0xa6, b'h', b'e', b'i', b'g', b'h',
        b't', 0x01, 0xa4, b'r', b'u', b'n', b's', 0x90,
    ];

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

        #[test]
        fn view_patch_frames_round_trip_through_messagepack(
            patch in populated_patch(),
            sequence in any::<u64>(),
            view_revision in any::<u64>(),
            base_view_revision in prop::option::of(any::<u64>()),
            resume_token in prop::option::of("[0-9a-f]{1,64}"),
        ) {
            let frame = ViewPatchFrame {
                sequence,
                view_revision,
                base_view_revision,
                resume_token,
                patch,
            };
            let encoded = encode_view_patch(&frame).unwrap();
            prop_assert_eq!(decode_view_patch(&encoded).unwrap(), frame);
        }
    }

    fn sample_view_patch() -> ViewPatchFrame {
        ViewPatchFrame {
            sequence: 7,
            view_revision: 3,
            base_view_revision: Some(2),
            resume_token: None,
            patch: ScreenPatch {
                version: 1,
                width: 2,
                height: 1,
                runs: Vec::new(),
            },
        }
    }

    #[test]
    fn view_patch_matches_versioned_messagepack_golden_vector() {
        let encoded = encode_view_patch(&sample_view_patch()).unwrap();
        assert_eq!(encoded, GOLDEN_VIEW_PATCH);
        assert_eq!(
            decode_view_patch(GOLDEN_VIEW_PATCH).unwrap(),
            sample_view_patch()
        );
    }

    #[test]
    fn view_patch_rejects_unsupported_version_and_malformed_frames() {
        let mut unsupported = encode_view_patch(&sample_view_patch()).unwrap();
        unsupported[9] = 2;
        assert!(matches!(
            decode_view_patch(&unsupported),
            Err(ScreenCodecError::UnsupportedVersion(2))
        ));
        assert!(matches!(
            decode_view_patch(&[0xc1]),
            Err(ScreenCodecError::Invalid(_))
        ));

        let mut trailing = encode_view_patch(&sample_view_patch()).unwrap();
        trailing.push(0xc0);
        assert!(matches!(
            decode_view_patch(&trailing),
            Err(ScreenCodecError::TrailingBytes)
        ));

        let oversized = vec![0; MAX_SCREEN_FRAME_BYTES + 1];
        assert!(matches!(
            decode_view_patch(&oversized),
            Err(ScreenCodecError::TooLarge)
        ));
    }
}
