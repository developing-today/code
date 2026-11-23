//! Versioned MessagePack encoding for the transport-neutral F12.11 screen patch.
//!
//! Version 1 is the plain MessagePack frame. Version 2 wraps the same
//! version-1 frame in an envelope naming its compression, so a compressed
//! frame is self-describing: `{version: 2, compression, payload}`. A
//! compressed frame is emitted only when the complete compressed frame is
//! smaller than its uncompressed version-1 form, so compression never grows
//! a frame on the wire (F12.11 phase 4, #805).

use std::io::{Cursor, Read, Write};

use serde::{Deserialize, Serialize};
use serde_bytes::ByteBuf;

use crate::world_presentation::ScreenPatch;

/// Version of the MessagePack screen-patch wire representation.
pub const SCREEN_CODEC_VERSION: u8 = 1;
/// Version of the compressed screen-patch envelope: the payload is the
/// version-1 MessagePack frame, compressed as named by `compression`.
pub const SCREEN_CODEC_VERSION_COMPRESSED: u8 = 2;
/// No compression: the version-2 payload is the version-1 frame as-is.
pub const COMPRESSION_NONE: u8 = 0;
/// Raw DEFLATE (RFC 1951) of the version-1 frame. Raw DEFLATE is what the
/// browser's `DecompressionStream("deflate")` decodes, so the same bytes
/// serve the Rust transports and the browser with no extra dependency.
pub const COMPRESSION_DEFLATE: u8 = 1;
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
    /// The frame names a compression this implementation does not support.
    #[error("unsupported screen compression {0}")]
    UnsupportedCompression(u8),
    /// A compression or decompression I/O error occurred.
    #[error("screen frame compression failed: {0}")]
    Compression(#[from] std::io::Error),
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

/// Decodes one bounded, versioned MessagePack view frame, plain
/// (version 1) or compressed (version 2).
pub fn decode_view_patch(bytes: &[u8]) -> Result<ViewPatchFrame, ScreenCodecError> {
    if bytes.len() > MAX_SCREEN_FRAME_BYTES {
        return Err(ScreenCodecError::TooLarge);
    }
    let mut deserializer = rmp_serde::Deserializer::new(Cursor::new(bytes));
    let probe = <VersionProbe as Deserialize>::deserialize(&mut deserializer)?;
    match probe.version {
        SCREEN_CODEC_VERSION => decode_v1_view_patch(bytes),
        SCREEN_CODEC_VERSION_COMPRESSED => decode_compressed_view_patch(bytes),
        other => Err(ScreenCodecError::UnsupportedVersion(other)),
    }
}

/// The one field every envelope version carries, read before dispatching
/// to the version's own shape. Unknown fields are ignored here; the
/// version-specific decoder below still enforces its exact shape.
#[derive(Deserialize)]
struct VersionProbe {
    version: u8,
}

/// Decodes one bounded, versioned MessagePack view frame in the plain
/// version-1 shape.
fn decode_v1_view_patch(bytes: &[u8]) -> Result<ViewPatchFrame, ScreenCodecError> {
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

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct VersionedCompressedViewPatch {
    version: u8,
    compression: u8,
    payload: ByteBuf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OwnedVersionedCompressedViewPatch {
    version: u8,
    compression: u8,
    payload: ByteBuf,
}

/// Encodes a view frame, compressing it only when the complete compressed
/// frame is smaller than the uncompressed version-1 frame (F12.11 phase 4,
/// #805; design `doc/2026-11-01_design_id805.md` §2.3). The returned bytes
/// are either the version-1 frame or a version-2 envelope
/// `{version: 2, compression: 1, payload}` whose payload is the raw-DEFLATE
/// (RFC 1951) of the version-1 frame. Small frames come back byte-identical
/// to [`encode_view_patch`], so compression never grows a frame on the wire.
pub fn encode_view_patch_compressed(
    frame: &ViewPatchFrame,
) -> Result<Vec<u8>, ScreenCodecError> {
    let raw = encode_view_patch(frame)?;
    let mut encoder =
        flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&raw)?;
    let deflated = encoder.finish()?;
    let envelope = rmp_serde::to_vec_named(&VersionedCompressedViewPatch {
        version: SCREEN_CODEC_VERSION_COMPRESSED,
        compression: COMPRESSION_DEFLATE,
        payload: ByteBuf::from(deflated),
    })?;
    if envelope.len() < raw.len() {
        Ok(envelope)
    } else {
        Ok(raw)
    }
}

/// Decodes one version-2 compressed view frame: validates the envelope,
/// inflates the payload with its output bounded by the frame limit, then
/// decodes the inner version-1 frame exactly as a plain frame.
fn decode_compressed_view_patch(bytes: &[u8]) -> Result<ViewPatchFrame, ScreenCodecError> {
    let mut deserializer = rmp_serde::Deserializer::new(Cursor::new(bytes));
    let frame =
        <OwnedVersionedCompressedViewPatch as Deserialize>::deserialize(&mut deserializer)?;
    if usize::try_from(deserializer.into_inner().position()).ok() != Some(bytes.len()) {
        return Err(ScreenCodecError::TrailingBytes);
    }
    if frame.version != SCREEN_CODEC_VERSION_COMPRESSED {
        return Err(ScreenCodecError::UnsupportedVersion(frame.version));
    }
    let raw = match frame.compression {
        COMPRESSION_NONE => frame.payload.into_vec(),
        COMPRESSION_DEFLATE => inflate_bounded(&frame.payload)?,
        other => return Err(ScreenCodecError::UnsupportedCompression(other)),
    };
    decode_v1_view_patch(&raw)
}

/// Inflates raw-DEFLATE bytes, refusing output past the shared frame limit
/// instead of allocating on an untrusted claimed size.
fn inflate_bounded(payload: &[u8]) -> Result<Vec<u8>, ScreenCodecError> {
    let mut decoder = flate2::read::DeflateDecoder::new(payload);
    let mut out = Vec::new();
    decoder
        .take(u64::try_from(MAX_SCREEN_FRAME_BYTES).unwrap_or(u64::MAX) + 1)
        .read_to_end(&mut out)?;
    if out.len() > MAX_SCREEN_FRAME_BYTES {
        return Err(ScreenCodecError::TooLarge);
    }
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use proptest::prelude::*;

    use super::{
        COMPRESSION_DEFLATE, COMPRESSION_NONE, MAX_SCREEN_FRAME_BYTES,
        SCREEN_CODEC_VERSION_COMPRESSED, ScreenCodecError, ViewPatchFrame, decode_patch,
        decode_view_patch, encode_patch, encode_view_patch, encode_view_patch_compressed,
    };
    use crate::world_presentation::{CellRun, ScreenCell, ScreenPatch, ScreenSurface, StyleToken};

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

        #[test]
        fn view_patch_frames_round_trip_through_compression(
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
            let encoded = encode_view_patch_compressed(&frame).unwrap();
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
        // Version 3 names no envelope this implementation knows.
        let mut unsupported = encode_view_patch(&sample_view_patch()).unwrap();
        unsupported[9] = 3;
        assert!(matches!(
            decode_view_patch(&unsupported),
            Err(ScreenCodecError::UnsupportedVersion(3))
        ));
        // Version 2 with a version-1 body is a malformed compressed
        // envelope, not a plain frame: the compression tag is missing.
        let mut mistyped = encode_view_patch(&sample_view_patch()).unwrap();
        mistyped[9] = 2;
        assert!(matches!(
            decode_view_patch(&mistyped),
            Err(ScreenCodecError::Invalid(_))
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

    /// Mirror of the version-2 envelope shape, for building tampered
    /// envelopes the encoder never emits.
    #[derive(serde::Serialize, serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct TestCompressedEnvelope {
        version: u8,
        compression: u8,
        payload: serde_bytes::ByteBuf,
    }

    fn deflate(bytes: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let mut encoder =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }

    /// A view frame big enough for DEFLATE to win: a 120x50 screen of
    /// varied cells, patched in full.
    fn large_view_patch() -> ViewPatchFrame {
        const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz";
        let mut runs = Vec::new();
        for row in 0..50_u16 {
            let cells = (0..120_u16)
                .map(|col| {
                    let symbol = ALPHABET
                        [(usize::from(row) * 7 + usize::from(col) * 13) % ALPHABET.len()]
                        as char;
                    ScreenCell {
                        symbol: symbol.to_string(),
                        fg: u8::try_from((usize::from(row) + usize::from(col)) % 8)
                            .unwrap_or_default(),
                        bg: 0,
                        bold: (row + col) % 11 == 0,
                        columns: 1,
                        token: None,
                    }
                })
                .collect::<Vec<_>>();
            runs.push(CellRun {
                start: u32::from(row) * 120,
                cells,
            });
        }
        ViewPatchFrame {
            sequence: 1,
            view_revision: 2,
            base_view_revision: Some(1),
            resume_token: None,
            patch: ScreenPatch {
                version: 1,
                width: 120,
                height: 50,
                runs,
            },
        }
    }

    #[test]
    fn large_view_patches_round_trip_through_compression_and_stay_smaller() {
        let frame = large_view_patch();
        let raw = encode_view_patch(&frame).unwrap();
        let compressed = encode_view_patch_compressed(&frame).unwrap();
        assert!(
            compressed.len() < raw.len(),
            "a 120x50 varied patch must shrink: {} vs {} raw",
            compressed.len(),
            raw.len()
        );
        assert_ne!(compressed, raw);
        assert_eq!(decode_view_patch(&compressed).unwrap(), frame);
        assert_eq!(
            encode_view_patch_compressed(&frame).unwrap(),
            compressed,
            "compression is deterministic"
        );
    }

    #[test]
    fn small_view_patches_are_never_grown_by_compression() {
        let frame = sample_view_patch();
        let raw = encode_view_patch(&frame).unwrap();
        assert_eq!(encode_view_patch_compressed(&frame).unwrap(), raw);
        let one_cell = ViewPatchFrame {
            sequence: 1,
            view_revision: 2,
            base_view_revision: Some(1),
            resume_token: None,
            patch: ScreenPatch {
                version: 1,
                width: 80,
                height: 24,
                runs: vec![CellRun {
                    start: 0,
                    cells: vec![ScreenCell {
                        symbol: "x".to_owned(),
                        fg: 1,
                        bg: 0,
                        bold: false,
                        columns: 1,
                        token: None,
                    }],
                }],
            },
        };
        let one_raw = encode_view_patch(&one_cell).unwrap();
        let one_compressed = encode_view_patch_compressed(&one_cell).unwrap();
        assert!(one_compressed.len() <= one_raw.len());
        assert_eq!(decode_view_patch(&one_compressed).unwrap(), one_cell);
    }

    #[test]
    fn compressed_frames_carry_a_version_two_deflate_envelope() {
        let frame = large_view_patch();
        let compressed = encode_view_patch_compressed(&frame).unwrap();
        let envelope: TestCompressedEnvelope = rmp_serde::from_slice(&compressed).unwrap();
        assert_eq!(envelope.version, SCREEN_CODEC_VERSION_COMPRESSED);
        assert_eq!(envelope.compression, COMPRESSION_DEFLATE);
        assert!(!envelope.payload.is_empty());
        assert_eq!(decode_view_patch(&compressed).unwrap(), frame);
    }

    #[test]
    fn compressed_frames_reject_unknown_compression_truncated_and_oversized_payloads() {
        let frame = large_view_patch();
        let raw = encode_view_patch(&frame).unwrap();
        let deflated = deflate(&raw);

        let unknown = rmp_serde::to_vec_named(&TestCompressedEnvelope {
            version: SCREEN_CODEC_VERSION_COMPRESSED,
            compression: 9,
            payload: serde_bytes::ByteBuf::from(deflated.clone()),
        })
        .unwrap();
        assert!(matches!(
            decode_view_patch(&unknown),
            Err(ScreenCodecError::UnsupportedCompression(9))
        ));

        let truncated = rmp_serde::to_vec_named(&TestCompressedEnvelope {
            version: SCREEN_CODEC_VERSION_COMPRESSED,
            compression: COMPRESSION_DEFLATE,
            payload: serde_bytes::ByteBuf::from(deflated[..deflated.len() / 2].to_vec()),
        })
        .unwrap();
        assert!(matches!(
            decode_view_patch(&truncated),
            Err(ScreenCodecError::Compression(_))
        ));

        let bomb = rmp_serde::to_vec_named(&TestCompressedEnvelope {
            version: SCREEN_CODEC_VERSION_COMPRESSED,
            compression: COMPRESSION_DEFLATE,
            payload: serde_bytes::ByteBuf::from(deflate(&vec![
                0_u8;
                MAX_SCREEN_FRAME_BYTES + 1
            ])),
        })
        .unwrap();
        assert!(matches!(
            decode_view_patch(&bomb),
            Err(ScreenCodecError::TooLarge)
        ));

        let plain = rmp_serde::to_vec_named(&TestCompressedEnvelope {
            version: SCREEN_CODEC_VERSION_COMPRESSED,
            compression: COMPRESSION_NONE,
            payload: serde_bytes::ByteBuf::from(raw),
        })
        .unwrap();
        assert_eq!(decode_view_patch(&plain).unwrap(), frame);

        let mut trailing = encode_view_patch_compressed(&frame).unwrap();
        trailing.push(0xc0);
        assert!(matches!(
            decode_view_patch(&trailing),
            Err(ScreenCodecError::TrailingBytes)
        ));
    }

    fn blank_cell() -> ScreenCell {
        ScreenCell {
            symbol: " ".to_owned(),
            fg: 0,
            bg: 0,
            bold: false,
            columns: 1,
            token: None,
        }
    }

    /// Deterministic document-like screen: rows of varied words with
    /// cycling colours, so the full frame carries realistic entropy
    /// instead of one repeated cell.
    fn document_screen(width: usize, height: usize) -> ScreenSurface {
        const WORDS: &[&str] = &[
            "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel",
        ];
        let mut cells = Vec::with_capacity(width * height);
        for row in 0..height {
            let mut col = 0;
            let mut word_index = row % WORDS.len();
            while col < width {
                let word = WORDS[word_index % WORDS.len()];
                word_index += 1;
                for ch in word.chars() {
                    if col >= width {
                        break;
                    }
                    cells.push(ScreenCell {
                        symbol: ch.to_string(),
                        fg: u8::try_from((row + col) % 8).unwrap_or_default(),
                        bg: 0,
                        bold: (row + col) % 11 == 0,
                        columns: 1,
                        token: if col == 0 && row % 5 == 0 {
                            Some(StyleToken::Heading)
                        } else {
                            None
                        },
                    });
                    col += 1;
                }
                if col < width {
                    cells.push(blank_cell());
                    col += 1;
                }
            }
        }
        ScreenSurface::from_cells(width, height, cells).unwrap()
    }

    /// A typical sparse redraw of a document screen: a status count
    /// tick (two cells on the last row) plus two scattered single-cell
    /// edits.
    ///
    /// The count tick — not a full status-line rewrite — is the
    /// nominated typical case. Restyling all 29 cells of the last row
    /// is a dense change, and its minimum encoding already exceeds
    /// the 10% bar at 80x24: the full-line restyle measures 279/2487
    /// bytes (11.2%), and even the 29-cell run alone is ~244 bytes
    /// against a 248-byte bar, so no encoder could meet the bar for
    /// it. A real status line redraw changes the count, not the
    /// whole line.
    fn sparse_redraw(screen: &ScreenSurface) -> ScreenSurface {
        let width = usize::from(screen.width);
        let height = usize::from(screen.height);
        let mut cells = screen.cells.clone();
        // Status count ticks 12 -> 13: two cells on the last row.
        let row_start = (height - 1) * width;
        for (offset, ch) in [(11_usize, '1'), (12_usize, '3')] {
            cells[row_start + offset] = ScreenCell {
                symbol: ch.to_string(),
                fg: 7,
                bg: 0,
                bold: true,
                columns: 1,
                token: None,
            };
        }
        for (row, col) in [(3_usize, 5_usize), (10_usize, 40_usize)] {
            if row < height && col < width {
                cells[row * width + col] = ScreenCell {
                    symbol: "#".to_owned(),
                    fg: 1,
                    bg: 0,
                    bold: false,
                    columns: 1,
                    token: Some(StyleToken::Warning),
                };
            }
        }
        ScreenSurface::from_cells(width, height, cells).unwrap()
    }

    fn full_snapshot_patch(screen: &ScreenSurface) -> ScreenPatch {
        ScreenPatch {
            version: screen.version,
            width: screen.width,
            height: screen.height,
            runs: vec![CellRun {
                start: 0,
                cells: screen.cells.clone(),
            }],
        }
    }

    #[test]
    fn sparse_redraws_measure_under_ten_percent_of_full_frame() {
        // Oracle: the #805 acceptance as decided (design §7, Q2 option B):
        // complete logical View payload bytes, full snapshot and patch
        // encoded with the same negotiated version and codec, at 80x24
        // and at the largest valid screen, 120x50 (the screen bounds
        // are 120x50, so the design's 200x60 cannot be constructed).
        // The F11.9 raw guest-frame sizes are a different layer and
        // stay out of the ratio.
        for (width, height) in [(80_usize, 24_usize), (120_usize, 50_usize)] {
            let full = document_screen(width, height);
            let changed = sparse_redraw(&full);
            let blank = ScreenSurface::from_cells(width, height, vec![
                blank_cell();
                width * height
            ])
            .unwrap();
            let full_frame = ViewPatchFrame {
                sequence: 1,
                view_revision: 2,
                base_view_revision: Some(1),
                resume_token: None,
                patch: full_snapshot_patch(&full),
            };
            let sparse_patch = ScreenPatch::diff(&full, &changed).unwrap();
            assert!(
                !sparse_patch.is_empty(),
                "the fixture redraw must change cells at {width}x{height}"
            );
            let sparse_frame = ViewPatchFrame {
                sequence: 1,
                view_revision: 2,
                base_view_revision: Some(1),
                resume_token: None,
                patch: sparse_patch.clone(),
            };
            let full_bytes = encode_view_patch_compressed(&full_frame).unwrap();
            let sparse_bytes = encode_view_patch_compressed(&sparse_frame).unwrap();
            // Parity: the compressed sparse patch applies to the full
            // screen and yields exactly the redrawn screen, and the
            // full snapshot patch rebuilds the screen from blank.
            assert_eq!(decode_view_patch(&sparse_bytes).unwrap(), sparse_frame);
            assert_eq!(decode_view_patch(&full_bytes).unwrap(), full_frame);
            assert_eq!(sparse_patch.apply(&full).unwrap(), changed);
            assert_eq!(full_snapshot_patch(&full).apply(&blank).unwrap(), full);
            assert!(
                sparse_bytes.len() * 10 < full_bytes.len(),
                "sparse redraw is {} bytes, over 10% of the {}-byte full frame at {width}x{height}",
                sparse_bytes.len(),
                full_bytes.len(),
            );
        }
    }
}
