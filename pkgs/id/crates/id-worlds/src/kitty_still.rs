//! The front half of the kitty graphics stills pipeline (#13623, third
//! slice): turn a bounded PNG asset into a still that fits the cell area
//! the presenter can give it.
//!
//! Research §6.2 of `doc/2026-11-23T18-32-12Z_research_id5971_image_video_conversion_tooling/`
//! splits the stills pipeline in two. The encoder half is
//! [`crate::kitty_graphics`]. This module is the shared converter front
//! half:
//!
//! - **Bounded input.** The asset is at most [`MAX_ASSET_BYTES`], must be
//!   a PNG, and its header must describe at most [`MAX_SOURCE_PIXELS`]
//!   pixels. The header is read before anything is decoded, and the
//!   decoder runs under [`MAX_DECODE_ALLOC`], so a hostile asset cannot
//!   make the presenter allocate without bound.
//! - **Geometry.** A [`CellBudget`] is the negotiated cell area times the
//!   probed cell pixel size. A session that never reported its cell size
//!   gets [`FALLBACK_CELL_WIDTH_PX`] × [`FALLBACK_CELL_HEIGHT_PX`]
//!   (geometry is probed-with-fallback). The still keeps its aspect ratio,
//!   is never enlarged, and never exceeds the budget or
//!   [`MAX_STILL_PIXELS`].
//! - **Payload choice.** A PNG that already fits passes through untouched
//!   when the terminal accepted the PNG class. Anything else is decoded,
//!   resized, and sent as zlib RGBA, raw RGBA, or a re-encoded PNG, in that
//!   order of preference, among the classes the session's
//!   [`GraphicsGate`] accepted. A session that accepted none gets no work
//!   at all.
//!
//! - **Lookup by content ID.** A call site names the asset by its blake3
//!   content ID through a [`StillSource`]; [`lookup_still`] refuses bytes
//!   over the cap or bytes that do not hash to that ID, so a store that
//!   returns the wrong thing never reaches the decoder.
//!
//! Only the PNG codec is compiled in. Other formats, animation, and the
//! golden fixtures against icat and chafa are later slices.

use std::borrow::Cow;
use std::io::Cursor;

use image::{ExtendedColorType, ImageEncoder as _, ImageFormat, ImageReader, Limits};

use crate::kitty_graphics::{
    GraphicsGate, MAX_PNG_BYTES, MAX_STILL_PIXELS, PayloadClass, Placement, StillPayload,
};

/// Largest asset, in bytes, the front half will read: the same cap as a
/// passed-through PNG payload.
pub const MAX_ASSET_BYTES: usize = MAX_PNG_BYTES;

/// Largest source image, in pixels, the front half will decode before
/// fitting it (4096 × 4096). Bigger sources are refused from the header.
pub const MAX_SOURCE_PIXELS: u64 = 16_777_216;

/// Most the PNG decoder may allocate at once, in bytes: the RGBA buffer of
/// a [`MAX_SOURCE_PIXELS`] source plus working room.
pub const MAX_DECODE_ALLOC: u64 = 96 * 1024 * 1024;

/// Cell width, in pixels, assumed when the terminal never reported one.
pub const FALLBACK_CELL_WIDTH_PX: u16 = 10;

/// Cell height, in pixels, assumed when the terminal never reported one.
pub const FALLBACK_CELL_HEIGHT_PX: u16 = 20;

/// Why an asset could not be prepared. Every variant emits nothing.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StillPrepError {
    /// The asset was empty.
    #[error("the image asset is empty")]
    EmptyAsset,
    /// The asset is over [`MAX_ASSET_BYTES`].
    #[error("the image asset is {0} bytes, over the {MAX_ASSET_BYTES}-byte cap")]
    AssetTooLarge(usize),
    /// The asset is not a PNG.
    #[error("the image asset is not a PNG")]
    NotPng,
    /// The header describes a zero width or height.
    #[error("the image has a zero dimension")]
    ZeroDimension,
    /// The header describes more than [`MAX_SOURCE_PIXELS`] pixels.
    #[error("the image is {width} x {height}, over the {MAX_SOURCE_PIXELS}-pixel source cap")]
    SourceTooLarge {
        /// Width in pixels.
        width: u32,
        /// Height in pixels.
        height: u32,
    },
    /// The cell budget has no room: zero columns or zero rows.
    #[error("the cell budget has no room for an image")]
    EmptyBudget,
    /// The PNG decoder refused the asset.
    #[error("the image could not be decoded: {0}")]
    Decode(String),
    /// The fitted pixels could not be re-encoded as a PNG.
    #[error("the image could not be re-encoded: {0}")]
    Encode(String),
}

/// The room a still may take: a cell area and the cell size in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellBudget {
    /// Columns available.
    pub columns: u16,
    /// Rows available.
    pub rows: u16,
    /// Probed cell size in pixels, width then height, if the terminal
    /// reported one. A zero in either is treated as unreported.
    pub cell_px: Option<(u16, u16)>,
}

impl CellBudget {
    /// The cell size in pixels, falling back to
    /// [`FALLBACK_CELL_WIDTH_PX`] × [`FALLBACK_CELL_HEIGHT_PX`].
    #[must_use]
    pub fn cell_size(&self) -> (u32, u32) {
        match self.cell_px {
            Some((width, height)) if width > 0 && height > 0 => {
                (u32::from(width), u32::from(height))
            }
            _ => (
                u32::from(FALLBACK_CELL_WIDTH_PX),
                u32::from(FALLBACK_CELL_HEIGHT_PX),
            ),
        }
    }

    /// The pixel box the still must fit: columns × cell width by rows ×
    /// cell height.
    #[must_use]
    pub fn pixel_box(&self) -> (u32, u32) {
        let (cell_width, cell_height) = self.cell_size();
        (
            u32::from(self.columns) * cell_width,
            u32::from(self.rows) * cell_height,
        )
    }
}

/// A still ready to transmit: fitted pixels in a class the session
/// accepted, and the cell area it covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedStill<'a> {
    width: u32,
    height: u32,
    columns: u16,
    rows: u16,
    pixels: Pixels<'a>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Pixels<'a> {
    Png(Cow<'a, [u8]>),
    Rgba { data: Vec<u8>, compress: bool },
}

impl PreparedStill<'_> {
    /// Width of the fitted still, in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Height of the fitted still, in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Columns the still covers.
    #[must_use]
    pub const fn columns(&self) -> u16 {
        self.columns
    }

    /// Rows the still covers.
    #[must_use]
    pub const fn rows(&self) -> u16 {
        self.rows
    }

    /// Whether the asset's own PNG bytes are sent untouched.
    #[must_use]
    pub const fn is_passthrough(&self) -> bool {
        matches!(self.pixels, Pixels::Png(Cow::Borrowed(_)))
    }

    /// The payload to hand the encoder.
    #[must_use]
    pub fn payload(&self) -> StillPayload<'_> {
        match &self.pixels {
            Pixels::Png(bytes) => StillPayload::Png(bytes),
            Pixels::Rgba { data, compress } => StillPayload::Rgba {
                data,
                width: self.width,
                height: self.height,
                compress: *compress,
            },
        }
    }

    /// A placement over exactly the cells the still covers.
    #[must_use]
    pub const fn placement(&self) -> Placement {
        Placement {
            placement_id: None,
            columns: Some(self.columns),
            rows: Some(self.rows),
            z_index: None,
        }
    }
}

/// Prepare `asset` for a session whose accepted classes are in `gate`, to
/// fit `budget`.
///
/// Returns `Ok(None)` when the gate accepts no payload class, before the
/// asset is read at all: there is nothing this session could display.
///
/// # Errors
///
/// [`StillPrepError`] when the asset is empty, too large, not a PNG, over
/// the source cap, undecodable, or the budget has no room.
pub fn prepare_still<'a>(
    asset: &'a [u8],
    budget: CellBudget,
    gate: &GraphicsGate,
) -> Result<Option<PreparedStill<'a>>, StillPrepError> {
    let accepts_png = gate.accepts(PayloadClass::Png);
    let accepts_rgba = gate.accepts(PayloadClass::Rgba);
    let accepts_zlib = gate.accepts(PayloadClass::RgbaZlib);
    if !(accepts_png || accepts_rgba || accepts_zlib) {
        return Ok(None);
    }
    if budget.columns == 0 || budget.rows == 0 {
        return Err(StillPrepError::EmptyBudget);
    }
    let (source_width, source_height) = read_header(asset)?;
    let (width, height) = fit(source_width, source_height, budget.pixel_box());
    let (cell_width, cell_height) = budget.cell_size();
    let columns = cells(width, cell_width, budget.columns);
    let rows = cells(height, cell_height, budget.rows);

    let unchanged = width == source_width && height == source_height;
    if unchanged && accepts_png {
        return Ok(Some(PreparedStill {
            width,
            height,
            columns,
            rows,
            pixels: Pixels::Png(Cow::Borrowed(asset)),
        }));
    }

    let rgba = decode_fitted(asset, width, height)?;
    let pixels = if accepts_zlib || accepts_rgba {
        Pixels::Rgba {
            data: rgba,
            compress: accepts_zlib,
        }
    } else {
        Pixels::Png(Cow::Owned(encode_png(&rgba, width, height)?))
    };
    Ok(Some(PreparedStill {
        width,
        height,
        columns,
        rows,
        pixels,
    }))
}

/// Read and check the PNG header without decoding any pixels.
fn read_header(asset: &[u8]) -> Result<(u32, u32), StillPrepError> {
    if asset.is_empty() {
        return Err(StillPrepError::EmptyAsset);
    }
    if asset.len() > MAX_ASSET_BYTES {
        return Err(StillPrepError::AssetTooLarge(asset.len()));
    }
    if image::guess_format(asset).ok() != Some(ImageFormat::Png) {
        return Err(StillPrepError::NotPng);
    }
    let (width, height) = ImageReader::with_format(Cursor::new(asset), ImageFormat::Png)
        .into_dimensions()
        .map_err(|error| StillPrepError::Decode(error.to_string()))?;
    if width == 0 || height == 0 {
        return Err(StillPrepError::ZeroDimension);
    }
    if u64::from(width) * u64::from(height) > MAX_SOURCE_PIXELS {
        return Err(StillPrepError::SourceTooLarge { width, height });
    }
    Ok((width, height))
}

/// The largest size with the source's aspect ratio that fits the pixel
/// box and [`MAX_STILL_PIXELS`], never larger than the source and never
/// below 1 × 1.
fn fit(width: u32, height: u32, (box_width, box_height): (u32, u32)) -> (u32, u32) {
    let width_f = f64::from(width);
    let height_f = f64::from(height);
    let mut scale = 1.0_f64
        .min(f64::from(box_width) / width_f)
        .min(f64::from(box_height) / height_f);
    let area = width_f * height_f * scale * scale;
    #[allow(clippy::cast_precision_loss)]
    let cap = MAX_STILL_PIXELS as f64;
    if area > cap {
        scale *= (cap / area).sqrt();
    }
    if scale >= 1.0 {
        return (width, height);
    }
    (scaled(width_f, scale), scaled(height_f, scale))
}

/// `length × scale`, floored, at least 1. The result is never above the
/// source length, which is a `u32`.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn scaled(length: f64, scale: f64) -> u32 {
    ((length * scale).floor() as u32).max(1)
}

/// How many cells `pixels` covers at `cell` pixels a cell, at least 1 and
/// at most `limit`.
fn cells(pixels: u32, cell: u32, limit: u16) -> u16 {
    let covered = pixels.div_ceil(cell).max(1);
    u16::try_from(covered).map_or(limit, |covered| covered.min(limit))
}

/// Decode the PNG under the allocation cap and resize it to `width` ×
/// `height`, as row-major RGBA.
fn decode_fitted(asset: &[u8], width: u32, height: u32) -> Result<Vec<u8>, StillPrepError> {
    let mut reader = ImageReader::with_format(Cursor::new(asset), ImageFormat::Png);
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_DECODE_ALLOC);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|error| StillPrepError::Decode(error.to_string()))?
        .into_rgba8();
    let fitted = if decoded.width() == width && decoded.height() == height {
        decoded
    } else {
        image::imageops::resize(
            &decoded,
            width,
            height,
            image::imageops::FilterType::Triangle,
        )
    };
    Ok(fitted.into_raw())
}

/// Re-encode fitted RGBA pixels as a PNG, for a session that accepts only
/// the PNG class.
fn encode_png(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, StillPrepError> {
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(rgba, width, height, ExtendedColorType::Rgba8)
        .map_err(|error| StillPrepError::Encode(error.to_string()))?;
    Ok(out)
}

/// A content-addressed store of still assets, keyed by the blake3 hash of
/// the asset's bytes (the same content ID iroh-blobs uses).
///
/// This is the presenter's only way in to an asset: a call site names the
/// content it wants, never a path or a guest-supplied buffer, and
/// [`lookup_still`] checks that what the source hands back really is that
/// content before anything decodes it.
pub trait StillSource {
    /// The bytes stored under `content`, or `None` when the source does not
    /// hold them.
    fn still_asset(&self, content: &blake3::Hash) -> Option<Cow<'_, [u8]>>;
}

/// Why a content ID did not yield an asset. Every variant emits nothing.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StillLookupError {
    /// The source does not hold the content.
    #[error("no still asset is stored under content {0}")]
    Missing(blake3::Hash),
    /// The source returned more than [`MAX_ASSET_BYTES`]; refused before
    /// hashing.
    #[error(
        "the still asset under content {content} is {len} bytes, over the {MAX_ASSET_BYTES}-byte cap"
    )]
    TooLarge {
        /// The content ID asked for.
        content: blake3::Hash,
        /// Bytes the source returned.
        len: usize,
    },
    /// The source returned bytes whose hash is not the content ID asked
    /// for.
    #[error("the still asset under content {expected} hashes to {actual}")]
    HashMismatch {
        /// The content ID asked for.
        expected: blake3::Hash,
        /// The hash of the bytes the source returned.
        actual: blake3::Hash,
    },
}

/// Look up the asset stored under `content`, checking the size cap and that
/// the bytes hash to `content`.
///
/// # Errors
///
/// [`StillLookupError`] when the source does not hold the content, returns
/// more than [`MAX_ASSET_BYTES`], or returns bytes that hash to something
/// else.
pub fn lookup_still<'s, S: StillSource + ?Sized>(
    source: &'s S,
    content: &blake3::Hash,
) -> Result<Cow<'s, [u8]>, StillLookupError> {
    let asset = source
        .still_asset(content)
        .ok_or(StillLookupError::Missing(*content))?;
    if asset.len() > MAX_ASSET_BYTES {
        return Err(StillLookupError::TooLarge {
            content: *content,
            len: asset.len(),
        });
    }
    let actual = blake3::hash(&asset);
    if actual != *content {
        return Err(StillLookupError::HashMismatch {
            expected: *content,
            actual,
        });
    }
    Ok(asset)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::num::NonZeroU32;

    use super::*;
    use crate::kitty_graphics::{Action, PngInfo};

    /// A gate that accepted exactly `classes`.
    fn gate(classes: &[PayloadClass]) -> GraphicsGate {
        let mut gate = GraphicsGate::new(true);
        for class in PayloadClass::ALL {
            gate.record_answer(class, classes.contains(&class));
        }
        gate
    }

    /// A test-owned PNG: a horizontal gradient, red rising left to right.
    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut rgba = Vec::new();
        for _ in 0..height {
            for x in 0..width {
                let red = u8::try_from(x * 255 / (width - 1).max(1)).unwrap();
                rgba.extend_from_slice(&[red, 0, 128, 255]);
            }
        }
        encode_png(&rgba, width, height).unwrap()
    }

    fn budget(columns: u16, rows: u16, cell_px: Option<(u16, u16)>) -> CellBudget {
        CellBudget {
            columns,
            rows,
            cell_px,
        }
    }

    #[test]
    fn a_fitting_png_passes_through_untouched() {
        let asset = png(40, 30);
        let still = prepare_still(
            &asset,
            budget(10, 5, Some((8, 16))),
            &gate(&PayloadClass::ALL),
        )
        .unwrap()
        .unwrap();
        assert!(still.is_passthrough());
        assert_eq!(still.payload(), StillPayload::Png(&asset));
        assert_eq!((still.width(), still.height()), (40, 30));
        // 40 px / 8 px = 5 columns; 30 px / 16 px rounds up to 2 rows.
        assert_eq!((still.columns(), still.rows()), (5, 2));
        assert_eq!(still.placement().columns, Some(5));
        assert_eq!(still.placement().rows, Some(2));
    }

    #[test]
    fn an_oversized_png_is_shrunk_keeping_its_aspect_ratio() {
        let asset = png(400, 100);
        // The box is 20 × 10 px cells = 200 × 100 px; width binds.
        let still = prepare_still(
            &asset,
            budget(20, 10, Some((10, 10))),
            &gate(&PayloadClass::ALL),
        )
        .unwrap()
        .unwrap();
        assert!(!still.is_passthrough());
        assert_eq!((still.width(), still.height()), (200, 50));
        assert_eq!((still.columns(), still.rows()), (20, 5));
        match still.payload() {
            StillPayload::Rgba {
                data,
                width,
                height,
                compress,
            } => {
                assert!(compress, "zlib RGBA is preferred when accepted");
                assert_eq!((width, height), (200, 50));
                assert_eq!(data.len(), 200 * 50 * 4);
                // The gradient survives the resize: left dark, right bright.
                assert!(data[0] < 20);
                assert!(data[(200 - 1) * 4] > 235);
            }
            other @ StillPayload::Png(_) => panic!("expected RGBA, got {other:?}"),
        }
    }

    #[test]
    fn a_small_png_is_never_enlarged() {
        let asset = png(4, 4);
        let still = prepare_still(&asset, budget(80, 24, None), &gate(&PayloadClass::ALL))
            .unwrap()
            .unwrap();
        assert!(still.is_passthrough());
        assert_eq!((still.width(), still.height()), (4, 4));
        assert_eq!((still.columns(), still.rows()), (1, 1));
    }

    #[test]
    fn an_unreported_cell_size_falls_back() {
        let unreported = budget(3, 2, None);
        let zeroed = budget(3, 2, Some((0, 18)));
        assert_eq!(unreported.cell_size(), (10, 20));
        assert_eq!(zeroed.cell_size(), (10, 20));
        assert_eq!(unreported.pixel_box(), (30, 40));
    }

    #[test]
    fn the_payload_class_follows_what_the_session_accepted() {
        let asset = png(100, 100);
        let tight = budget(5, 5, Some((10, 10)));

        let raw = prepare_still(&asset, tight, &gate(&[PayloadClass::Rgba]))
            .unwrap()
            .unwrap();
        assert_eq!(raw.payload().class(), PayloadClass::Rgba);

        let png_only = prepare_still(&asset, tight, &gate(&[PayloadClass::Png]))
            .unwrap()
            .unwrap();
        assert_eq!(png_only.payload().class(), PayloadClass::Png);
        assert!(!png_only.is_passthrough(), "a resized still is re-encoded");
        let StillPayload::Png(bytes) = png_only.payload() else {
            panic!("expected PNG");
        };
        let info = PngInfo::parse(bytes).unwrap();
        assert_eq!((info.width, info.height), (50, 50));

        // A fitting PNG on a session without the PNG class is decoded.
        let small = png(8, 8);
        let decoded = prepare_still(&small, tight, &gate(&[PayloadClass::RgbaZlib]))
            .unwrap()
            .unwrap();
        assert_eq!(decoded.payload().class(), PayloadClass::RgbaZlib);
        assert_eq!((decoded.width(), decoded.height()), (8, 8));
    }

    #[test]
    fn a_session_that_accepted_nothing_gets_no_work() {
        // Not even a malformed asset is looked at.
        let none = gate(&[]);
        assert_eq!(
            prepare_still(b"not a png", budget(10, 10, None), &none),
            Ok(None)
        );
        let undetected = GraphicsGate::new(false);
        assert_eq!(
            prepare_still(&png(4, 4), budget(10, 10, None), &undetected),
            Ok(None)
        );
    }

    #[test]
    fn bad_assets_and_budgets_are_refused() {
        let all = gate(&PayloadClass::ALL);
        let room = budget(10, 10, None);
        assert_eq!(
            prepare_still(&[], room, &all),
            Err(StillPrepError::EmptyAsset)
        );
        assert_eq!(
            prepare_still(b"GIF89a....", room, &all),
            Err(StillPrepError::NotPng)
        );
        let huge = vec![0_u8; MAX_ASSET_BYTES + 1];
        assert_eq!(
            prepare_still(&huge, room, &all),
            Err(StillPrepError::AssetTooLarge(MAX_ASSET_BYTES + 1))
        );
        assert_eq!(
            prepare_still(&png(4, 4), budget(0, 10, None), &all),
            Err(StillPrepError::EmptyBudget)
        );
        // A header claiming a giant image is refused before decoding.
        let mut giant = png(4, 4);
        giant[16..20].copy_from_slice(&5000_u32.to_be_bytes());
        giant[20..24].copy_from_slice(&5000_u32.to_be_bytes());
        assert!(matches!(
            prepare_still(&giant, room, &all),
            Err(StillPrepError::SourceTooLarge { .. } | StillPrepError::Decode(_))
        ));
        // A truncated PNG fails in the decoder, not with a panic.
        let full = png(64, 64);
        let truncated = &full[..full.len() / 2];
        assert!(matches!(
            prepare_still(truncated, budget(2, 2, Some((8, 8))), &all),
            Err(StillPrepError::Decode(_))
        ));
    }

    #[test]
    fn the_fitted_still_is_capped_at_the_presenter_maximum() {
        // A budget far bigger than MAX_STILL_PIXELS still yields a capped
        // still.
        let (width, height) = fit(4096, 4096, (u32::MAX, u32::MAX));
        assert!(u64::from(width) * u64::from(height) <= MAX_STILL_PIXELS);
        assert_eq!((width, height), (2048, 2048));
        assert_eq!(fit(1, 10_000, (100, 1)), (1, 1));
    }

    #[test]
    fn a_prepared_still_encodes_through_the_gate() {
        let session = gate(&PayloadClass::ALL);
        let asset = png(300, 300);
        let still = prepare_still(&asset, budget(10, 5, Some((10, 20))), &session)
            .unwrap()
            .unwrap();
        // 10 × 5 cells of 10 × 20 px = 100 × 100 px.
        assert_eq!((still.width(), still.height()), (100, 100));
        assert_eq!((still.columns(), still.rows()), (10, 5));
        let encoded = session
            .encode_still(
                NonZeroU32::new(9).unwrap(),
                still.payload(),
                Action::Place(still.placement()),
            )
            .unwrap()
            .unwrap();
        assert!(encoded.starts_with("\x1b_Ga=T,f=32,t=d,s=100,v=100,o=z,i=9,c=10,r=5,q=2"));
        assert!(encoded.ends_with("\x1b\\"));
    }

    /// A test source: a map of content IDs to whatever bytes it was told
    /// to return, so a lying store can be modelled.
    struct MapSource(std::collections::HashMap<blake3::Hash, Vec<u8>>);

    impl StillSource for MapSource {
        fn still_asset(&self, content: &blake3::Hash) -> Option<Cow<'_, [u8]>> {
            self.0
                .get(content)
                .map(|bytes| Cow::Borrowed(bytes.as_slice()))
        }
    }

    #[test]
    fn lookup_returns_only_bytes_that_hash_to_the_content_id() {
        let asset = png(4, 4);
        let content = blake3::hash(&asset);
        let other = blake3::hash(b"something else");
        let source = MapSource([(content, asset.clone()), (other, asset.clone())].into());

        assert_eq!(
            lookup_still(&source, &content).unwrap().as_ref(),
            asset.as_slice()
        );
        assert_eq!(
            lookup_still(&source, &other),
            Err(StillLookupError::HashMismatch {
                expected: other,
                actual: content,
            })
        );
        let absent = blake3::hash(b"absent");
        assert_eq!(
            lookup_still(&source, &absent),
            Err(StillLookupError::Missing(absent))
        );
    }

    #[test]
    fn lookup_refuses_an_over_cap_asset_before_hashing() {
        let huge = vec![0_u8; MAX_ASSET_BYTES + 1];
        let content = blake3::hash(&huge);
        let source = MapSource([(content, huge)].into());
        assert_eq!(
            lookup_still(&source, &content),
            Err(StillLookupError::TooLarge {
                content,
                len: MAX_ASSET_BYTES + 1,
            })
        );
    }
}
