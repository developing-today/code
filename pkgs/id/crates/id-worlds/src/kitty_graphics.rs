//! Presenter-side kitty graphics stills encoder (#13623, first slice).
//!
//! The encoder half of the stills pipeline from the #5971 conversion
//! research (`doc/2026-11-23T18-32-12Z_research_id5971_image_video_conversion_tooling/`,
//! §2.1, §6.2 and amendment A.3): the pieces the research keeps in
//! presenter code whichever protocol library is chosen.
//!
//! - **Transmission** of one still under a caller-chosen image ID, as a PNG
//!   payload passed through untouched (`f=100`) or raw RGBA (`f=32`),
//!   optionally zlib-compressed (`o=z`, for frames of a changing
//!   sequence). Only the direct medium (`t=d`) exists here: file and
//!   shared-memory mediums are local-only and never reach an SSH or iroh
//!   session.
//! - **Chunking**: the base64 payload splits into chunks of at most 4096
//!   characters. The first chunk carries every control key; later chunks
//!   carry only `m=` (and `q=` when quiet), with `m=1` on all but the last.
//! - **Placement and delete** by image ID, so re-displaying or moving an
//!   image never retransmits it.
//! - **Gating**: detection alone is not support (research §2.1, §2.3). A
//!   [`GraphicsGate`] emits bytes for a payload class only when the
//!   session's probe saw the kitty graphics bit AND that class's own `a=q`
//!   query came back `OK` on this session. Every other case emits nothing.
//!
//! The boundary of `doc/2026-11-01_design_id786.md` holds: these functions
//! take presenter-owned pixels or PNG bytes, never guest-supplied escape
//! bytes, and nothing here is journaled.
//!
//! Not in this slice: decoding and resizing with the `image` crate (the
//! shared front half), unicode placeholders, animation frames, and sixel.

use std::fmt::Write as _;
use std::io::Write as _;
use std::num::NonZeroU32;

use base64::Engine as _;

/// Largest single chunk of a transmission, in base64 characters.
pub const CHUNK_CHARS: usize = 4096;

/// Largest still, in pixels, the presenter will transmit (2048 × 2048).
pub const MAX_STILL_PIXELS: u64 = 4_194_304;

/// Largest PNG payload, in bytes, the presenter will pass through.
pub const MAX_PNG_BYTES: usize = 8 * 1024 * 1024;

/// The eight-byte PNG signature.
const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

/// Why a transmission could not be encoded. Every variant emits nothing.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KittyGraphicsError {
    /// The payload was empty.
    #[error("empty image payload")]
    EmptyPayload,
    /// A PNG payload did not start with a PNG signature and IHDR chunk.
    #[error("payload is not a PNG")]
    NotPng,
    /// A PNG payload was larger than [`MAX_PNG_BYTES`].
    #[error("PNG payload of {0} bytes exceeds the presenter cap")]
    PngTooLarge(usize),
    /// The image had a zero width or height.
    #[error("image has a zero dimension")]
    ZeroDimension,
    /// The image had more than [`MAX_STILL_PIXELS`] pixels.
    #[error("image of {0} pixels exceeds the presenter cap")]
    TooManyPixels(u64),
    /// Raw RGBA data did not hold exactly width × height × 4 bytes.
    #[error("RGBA payload is {actual} bytes, expected {expected}")]
    SizeMismatch {
        /// Bytes the geometry calls for.
        expected: u64,
        /// Bytes supplied.
        actual: u64,
    },
    /// zlib compression failed.
    #[error("zlib compression failed: {0}")]
    Compression(String),
}

/// The payload classes a terminal may accept or refuse independently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PayloadClass {
    /// `f=100`: a PNG the terminal decodes.
    Png,
    /// `f=32`: raw RGBA, uncompressed.
    Rgba,
    /// `f=32,o=z`: raw RGBA, zlib-compressed.
    RgbaZlib,
}

impl PayloadClass {
    /// Every class, in gate order.
    pub const ALL: [Self; 3] = [Self::Png, Self::Rgba, Self::RgbaZlib];

    const fn index(self) -> usize {
        match self {
            Self::Png => 0,
            Self::Rgba => 1,
            Self::RgbaZlib => 2,
        }
    }

    /// The class whose support query uses image ID `id`.
    #[must_use]
    pub const fn from_query_id(id: u32) -> Option<Self> {
        match id {
            41 => Some(Self::Png),
            42 => Some(Self::Rgba),
            43 => Some(Self::RgbaZlib),
            _ => None,
        }
    }

    /// The image ID this class's support query uses. Distinct from the
    /// startup probe's ID 31 and from each other, so a reply names exactly
    /// one outstanding query.
    #[must_use]
    pub const fn query_id(self) -> u32 {
        match self {
            Self::Png => 41,
            Self::Rgba => 42,
            Self::RgbaZlib => 43,
        }
    }
}

/// The pixels of one still.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StillPayload<'a> {
    /// PNG bytes, passed through untouched.
    Png(&'a [u8]),
    /// Row-major RGBA, four bytes per pixel.
    Rgba {
        /// The pixels.
        data: &'a [u8],
        /// Width in pixels.
        width: u32,
        /// Height in pixels.
        height: u32,
        /// Whether to zlib-compress the data (`o=z`).
        compress: bool,
    },
}

impl StillPayload<'_> {
    /// The class a terminal has to accept for this payload.
    #[must_use]
    pub const fn class(&self) -> PayloadClass {
        match self {
            Self::Png(_) => PayloadClass::Png,
            Self::Rgba {
                compress: false, ..
            } => PayloadClass::Rgba,
            Self::Rgba { compress: true, .. } => PayloadClass::RgbaZlib,
        }
    }
}

/// Where and how to show a transmitted image, in terminal cells.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Placement {
    /// The placement's ID (`p=`), so it can be moved or deleted alone.
    pub placement_id: Option<NonZeroU32>,
    /// Columns to scale the image into (`c=`).
    pub columns: Option<u16>,
    /// Rows to scale the image into (`r=`).
    pub rows: Option<u16>,
    /// Stacking order against text and other images (`z=`).
    pub z_index: Option<i32>,
}

impl Placement {
    fn write_keys(&self, out: &mut String) {
        if let Some(id) = self.placement_id {
            let _ = write!(out, ",p={id}");
        }
        if let Some(columns) = self.columns {
            let _ = write!(out, ",c={columns}");
        }
        if let Some(rows) = self.rows {
            let _ = write!(out, ",r={rows}");
        }
        if let Some(z) = self.z_index {
            let _ = write!(out, ",z={z}");
        }
    }
}

/// What a transmission asks the terminal to do with the image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Store the image only (`a=t`); place it later with [`encode_placement`].
    Store,
    /// Store the image and place it at the cursor (`a=T`).
    Place(Placement),
}

/// Width and height read from a PNG's IHDR chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PngInfo {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

impl PngInfo {
    /// Read the dimensions of a PNG and check the presenter caps.
    ///
    /// # Errors
    ///
    /// [`KittyGraphicsError`] when the bytes are empty, not a PNG, over
    /// [`MAX_PNG_BYTES`], or describe a zero or over-cap geometry.
    pub fn parse(bytes: &[u8]) -> Result<Self, KittyGraphicsError> {
        if bytes.is_empty() {
            return Err(KittyGraphicsError::EmptyPayload);
        }
        if bytes.len() > MAX_PNG_BYTES {
            return Err(KittyGraphicsError::PngTooLarge(bytes.len()));
        }
        // Signature, then the IHDR chunk: length 13, type, width, height.
        if bytes.len() < 24
            || &bytes[..8] != PNG_SIGNATURE
            || bytes[8..12] != [0, 0, 0, 13]
            || &bytes[12..16] != b"IHDR"
        {
            return Err(KittyGraphicsError::NotPng);
        }
        let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
        check_geometry(width, height)?;
        Ok(Self { width, height })
    }

    /// Whether the PNG already fits a box of `max_width` × `max_height`
    /// pixels, so it can pass through with no re-encode (research §6.2).
    #[must_use]
    pub const fn fits_within(self, max_width: u32, max_height: u32) -> bool {
        self.width <= max_width && self.height <= max_height
    }
}

fn check_geometry(width: u32, height: u32) -> Result<u64, KittyGraphicsError> {
    if width == 0 || height == 0 {
        return Err(KittyGraphicsError::ZeroDimension);
    }
    let pixels = u64::from(width) * u64::from(height);
    if pixels > MAX_STILL_PIXELS {
        return Err(KittyGraphicsError::TooManyPixels(pixels));
    }
    Ok(pixels)
}

fn zlib(data: &[u8]) -> Result<Vec<u8>, KittyGraphicsError> {
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(data)
        .map_err(|err| KittyGraphicsError::Compression(err.to_string()))?;
    encoder
        .finish()
        .map_err(|err| KittyGraphicsError::Compression(err.to_string()))
}

/// Split a base64 payload into APC chunks: `first_keys` on the first chunk,
/// only `m=` (and `q=2` when quiet) on the rest.
fn chunked(first_keys: &str, payload: &str, quiet: bool) -> String {
    let quiet_key = if quiet { ",q=2" } else { "" };
    let mut out = String::with_capacity(payload.len() + 64);
    // The payload is base64, so every byte is ASCII and any split is a
    // character boundary.
    let mut chunks = payload.as_bytes().chunks(CHUNK_CHARS).peekable();
    let mut first = true;
    while let Some(chunk) = chunks.next() {
        let more = u8::from(chunks.peek().is_some());
        if first {
            let _ = write!(out, "\x1b_G{first_keys}{quiet_key},m={more};");
            first = false;
        } else if quiet {
            let _ = write!(out, "\x1b_Gm={more},q=2;");
        } else {
            let _ = write!(out, "\x1b_Gm={more};");
        }
        out.extend(chunk.iter().copied().map(char::from));
        out.push_str("\x1b\\");
    }
    out
}

/// Encode one still as a direct (`t=d`) transmission under `image_id`.
///
/// The transmission is quiet (`q=2`): the terminal sends no reply that
/// could land in the session's input stream.
///
/// # Errors
///
/// [`KittyGraphicsError`] for an empty, malformed, mis-sized or over-cap
/// payload, or a failed compression. Nothing is emitted on error.
pub fn encode_still(
    image_id: NonZeroU32,
    payload: StillPayload<'_>,
    action: Action,
) -> Result<String, KittyGraphicsError> {
    let verb = match action {
        Action::Store => 't',
        Action::Place(_) => 'T',
    };
    let mut keys = format!("a={verb}");
    let data = match payload {
        StillPayload::Png(bytes) => {
            PngInfo::parse(bytes)?;
            keys.push_str(",f=100,t=d");
            base64::engine::general_purpose::STANDARD.encode(bytes)
        }
        StillPayload::Rgba {
            data,
            width,
            height,
            compress,
        } => {
            let pixels = check_geometry(width, height)?;
            let expected = pixels * 4;
            let actual = data.len() as u64;
            if actual != expected {
                return Err(KittyGraphicsError::SizeMismatch { expected, actual });
            }
            let _ = write!(keys, ",f=32,t=d,s={width},v={height}");
            if compress {
                keys.push_str(",o=z");
                base64::engine::general_purpose::STANDARD.encode(zlib(data)?)
            } else {
                base64::engine::general_purpose::STANDARD.encode(data)
            }
        }
    };
    let _ = write!(keys, ",i={image_id}");
    if let Action::Place(placement) = action {
        placement.write_keys(&mut keys);
    }
    Ok(chunked(&keys, &data, true))
}

/// Place an already-transmitted image at the cursor (`a=p`), quietly.
#[must_use]
pub fn encode_placement(image_id: NonZeroU32, placement: Placement) -> String {
    let mut keys = format!("a=p,i={image_id}");
    placement.write_keys(&mut keys);
    format!("\x1b_G{keys},q=2\x1b\\")
}

/// Delete an image and every placement of it, freeing its stored data
/// (`a=d,d=I`), quietly.
#[must_use]
pub fn encode_delete(image_id: NonZeroU32) -> String {
    format!("\x1b_Ga=d,d=I,i={image_id},q=2\x1b\\")
}

/// The PNG class's query: a 1 × 1 PNG under query ID 41.
pub const PNG_QUERY: &str = "\x1b_Ga=q,t=d,f=100,i=41;iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==\x1b\\";

/// The raw RGBA class's query: one transparent pixel under query ID 42.
pub const RGBA_QUERY: &str = "\x1b_Ga=q,t=d,f=32,s=1,v=1,i=42;AAAAAA==\x1b\\";

/// The compressed RGBA class's query: one transparent pixel, zlib, under
/// query ID 43.
pub const RGBA_ZLIB_QUERY: &str = "\x1b_Ga=q,t=d,f=32,s=1,v=1,o=z,i=43;eJxjYGBgAAAABAAB\x1b\\";

/// The `a=q` query asking whether this terminal accepts `class`.
///
/// It carries a 1 × 1 payload of exactly that class, under the class's
/// query ID. The query is not quiet, because its reply is the whole
/// point. The startup probe sends all three in its one burst
/// (`world_terminal::PROBE_QUERIES`).
#[must_use]
pub const fn encode_query(class: PayloadClass) -> &'static str {
    match class {
        PayloadClass::Png => PNG_QUERY,
        PayloadClass::Rgba => RGBA_QUERY,
        PayloadClass::RgbaZlib => RGBA_ZLIB_QUERY,
    }
}

/// Per-session record of which payload classes this terminal accepted.
///
/// Built from the startup probe's kitty graphics bit. A class opens only
/// after [`GraphicsGate::query`] sent its query and
/// [`GraphicsGate::observe_reply`] saw the matching `OK`; an error reply,
/// a reply to a query never sent, or a missing kitty bit keeps it shut.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GraphicsGate {
    kitty_graphics: bool,
    outstanding: [bool; 3],
    accepted: [Option<bool>; 3],
}

impl GraphicsGate {
    /// A gate for a session whose probe did (or did not) see kitty graphics.
    #[must_use]
    pub const fn new(kitty_graphics: bool) -> Self {
        Self {
            kitty_graphics,
            outstanding: [false; 3],
            accepted: [None; 3],
        }
    }

    /// The query bytes for `class`, marking its reply as due. `None` when
    /// the probe never saw kitty graphics: then nothing is sent at all.
    pub fn query(&mut self, class: PayloadClass) -> Option<String> {
        if !self.kitty_graphics {
            return None;
        }
        self.outstanding[class.index()] = true;
        Some(encode_query(class).to_owned())
    }

    /// Record an answer that another reply parser read for this session,
    /// such as the startup probe, which sends every class query in its
    /// burst and matches only replies to queries it sent.
    pub const fn record_answer(&mut self, class: PayloadClass, accepted: bool) {
        self.outstanding[class.index()] = false;
        self.accepted[class.index()] = Some(accepted);
    }

    /// Offer one complete APC body (the bytes between `ESC _` and `ESC \`).
    /// Returns `true` when it was the reply to an outstanding query of
    /// this gate and was consumed; every other body is left to the caller
    /// as input.
    pub fn observe_reply(&mut self, apc_body: &[u8]) -> bool {
        for class in PayloadClass::ALL {
            let index = class.index();
            if !self.outstanding[index] {
                continue;
            }
            let prefix = format!("Gi={};", class.query_id());
            if let Some(status) = apc_body.strip_prefix(prefix.as_bytes()) {
                self.outstanding[index] = false;
                self.accepted[index] = Some(status == b"OK");
                return true;
            }
        }
        false
    }

    /// Whether `class` may be emitted on this session.
    #[must_use]
    pub const fn accepts(&self, class: PayloadClass) -> bool {
        self.kitty_graphics && matches!(self.accepted[class.index()], Some(true))
    }

    /// Encode `payload` only if this session accepts its class.
    ///
    /// `Ok(None)` means the gate is shut and no bytes may be written.
    ///
    /// # Errors
    ///
    /// As [`encode_still`], checked only once the gate is open.
    pub fn encode_still(
        &self,
        image_id: NonZeroU32,
        payload: StillPayload<'_>,
        action: Action,
    ) -> Result<Option<String>, KittyGraphicsError> {
        if !self.accepts(payload.class()) {
            return Ok(None);
        }
        encode_still(image_id, payload, action).map(Some)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::io::Read as _;

    /// A valid 1 × 1 PNG, base64, sent as the PNG class's query payload.
    const PNG_QUERY_PAYLOAD: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

    /// One transparent RGBA pixel, base64, sent as the raw RGBA query payload.
    const RGBA_QUERY_PAYLOAD: &str = "AAAAAA==";

    /// One transparent RGBA pixel, zlib-compressed then base64, sent as the
    /// compressed RGBA query payload.
    const RGBA_ZLIB_QUERY_PAYLOAD: &str = "eJxjYGBgAAAABAAB";

    use super::*;

    fn id(n: u32) -> NonZeroU32 {
        NonZeroU32::new(n).unwrap()
    }

    fn b64(bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    fn unb64(text: &str) -> Vec<u8> {
        base64::engine::general_purpose::STANDARD
            .decode(text)
            .unwrap()
    }

    fn png_1x1() -> Vec<u8> {
        unb64(PNG_QUERY_PAYLOAD)
    }

    /// A syntactically valid PNG header of the given geometry; the body
    /// after IHDR is never inspected by the encoder.
    fn png_header(width: u32, height: u32, extra: usize) -> Vec<u8> {
        let mut bytes = PNG_SIGNATURE.to_vec();
        bytes.extend_from_slice(&13_u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        bytes.resize(bytes.len() + extra, 0xab);
        bytes
    }

    /// Split an encoded stream into (control keys, payload) per APC.
    fn apcs(stream: &str) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let mut rest = stream;
        while !rest.is_empty() {
            let body = rest.strip_prefix("\x1b_G").expect("APC start");
            let end = body.find("\x1b\\").expect("APC end");
            let (keys, payload) = body[..end].split_once(';').unwrap_or((&body[..end], ""));
            out.push((keys.to_owned(), payload.to_owned()));
            rest = &body[end + 2..];
        }
        out
    }

    #[test]
    fn a_small_png_passes_through_in_one_chunk() {
        let png = png_1x1();
        let out = encode_still(id(7), StillPayload::Png(&png), Action::Store).unwrap();
        assert_eq!(
            out,
            format!("\x1b_Ga=t,f=100,t=d,i=7,q=2,m=0;{PNG_QUERY_PAYLOAD}\x1b\\")
        );
        let chunks = apcs(&out);
        assert_eq!(unb64(&chunks[0].1), png);
    }

    #[test]
    fn placement_keys_follow_the_image_id() {
        let png = png_1x1();
        let placement = Placement {
            placement_id: Some(id(3)),
            columns: Some(20),
            rows: Some(10),
            z_index: Some(-1),
        };
        let out = encode_still(id(9), StillPayload::Png(&png), Action::Place(placement)).unwrap();
        assert!(out.starts_with("\x1b_Ga=T,f=100,t=d,i=9,p=3,c=20,r=10,z=-1,q=2,m=0;"));
        assert_eq!(
            encode_placement(id(9), placement),
            "\x1b_Ga=p,i=9,p=3,c=20,r=10,z=-1,q=2\x1b\\"
        );
        assert_eq!(
            encode_placement(id(9), Placement::default()),
            "\x1b_Ga=p,i=9,q=2\x1b\\"
        );
        assert_eq!(encode_delete(id(9)), "\x1b_Ga=d,d=I,i=9,q=2\x1b\\");
    }

    #[test]
    fn a_payload_of_exactly_one_chunk_has_no_continuation() {
        // 3072 bytes encode to exactly 4096 base64 characters.
        let png = png_header(1, 1, 3072 - 33);
        assert_eq!(b64(&png).len(), CHUNK_CHARS);
        let chunks = apcs(&encode_still(id(1), StillPayload::Png(&png), Action::Store).unwrap());
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].0.ends_with(",m=0"));
    }

    #[test]
    fn one_character_over_a_chunk_starts_a_second_chunk() {
        // 3073 bytes encode to 4100 characters: 4096 + 4.
        let png = png_header(1, 1, 3073 - 33);
        let out = encode_still(id(1), StillPayload::Png(&png), Action::Store).unwrap();
        let chunks = apcs(&out);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].0, "a=t,f=100,t=d,i=1,q=2,m=1");
        assert_eq!(chunks[0].1.len(), CHUNK_CHARS);
        assert_eq!(chunks[1].0, "m=0,q=2");
        assert_eq!(chunks[1].1.len(), 4);
        let joined: String = chunks.iter().map(|(_, p)| p.as_str()).collect();
        assert_eq!(unb64(&joined), png);
    }

    #[test]
    fn every_chunk_but_the_last_is_full_and_marked_more() {
        let png = png_header(1, 1, 20_000);
        let chunks = apcs(&encode_still(id(1), StillPayload::Png(&png), Action::Store).unwrap());
        let total = b64(&png).len();
        assert_eq!(chunks.len(), total.div_ceil(CHUNK_CHARS));
        for (keys, payload) in &chunks[..chunks.len() - 1] {
            assert_eq!(payload.len(), CHUNK_CHARS);
            assert!(keys.contains("m=1"), "{keys}");
        }
        let (last_keys, last) = chunks.last().unwrap();
        assert_eq!(last_keys, "m=0,q=2");
        assert!(!last.is_empty() && last.len() <= CHUNK_CHARS);
        // Only the first chunk carries the image's control keys.
        assert!(chunks[1..].iter().all(|(keys, _)| !keys.contains("i=")));
    }

    #[test]
    fn raw_rgba_carries_its_geometry() {
        let pixels = [255_u8, 0, 0, 255, 0, 255, 0, 255];
        let out = encode_still(
            id(5),
            StillPayload::Rgba {
                data: &pixels,
                width: 2,
                height: 1,
                compress: false,
            },
            Action::Store,
        )
        .unwrap();
        assert_eq!(
            out,
            format!(
                "\x1b_Ga=t,f=32,t=d,s=2,v=1,i=5,q=2,m=0;{}\x1b\\",
                b64(&pixels)
            )
        );
    }

    #[test]
    fn compressed_rgba_inflates_back_to_the_pixels() {
        let pixels: Vec<u8> = (0..64 * 64 * 4_u32)
            .map(|i| u8::try_from(i % 7).unwrap())
            .collect();
        let out = encode_still(
            id(5),
            StillPayload::Rgba {
                data: &pixels,
                width: 64,
                height: 64,
                compress: true,
            },
            Action::Store,
        )
        .unwrap();
        let chunks = apcs(&out);
        assert!(chunks[0].0.starts_with("a=t,f=32,t=d,s=64,v=64,o=z,i=5"));
        let joined: String = chunks.iter().map(|(_, p)| p.as_str()).collect();
        let mut inflated = Vec::new();
        flate2::read::ZlibDecoder::new(unb64(&joined).as_slice())
            .read_to_end(&mut inflated)
            .unwrap();
        assert_eq!(inflated, pixels);
    }

    #[test]
    fn malformed_payloads_emit_nothing() {
        let rgba = |data: &'static [u8], width, height| StillPayload::Rgba {
            data,
            width,
            height,
            compress: false,
        };
        let cases: Vec<(StillPayload<'_>, KittyGraphicsError)> = vec![
            (StillPayload::Png(&[]), KittyGraphicsError::EmptyPayload),
            (
                StillPayload::Png(b"GIF89a....................."),
                KittyGraphicsError::NotPng,
            ),
            (rgba(&[0; 4], 0, 1), KittyGraphicsError::ZeroDimension),
            (
                rgba(&[0; 4], 2, 1),
                KittyGraphicsError::SizeMismatch {
                    expected: 8,
                    actual: 4,
                },
            ),
            (
                rgba(&[0; 4], 4096, 4096),
                KittyGraphicsError::TooManyPixels(16_777_216),
            ),
        ];
        for (payload, expected) in cases {
            assert_eq!(encode_still(id(1), payload, Action::Store), Err(expected));
        }
        let zero_png = png_header(0, 5, 0);
        assert_eq!(
            PngInfo::parse(&zero_png),
            Err(KittyGraphicsError::ZeroDimension)
        );
        let big = vec![0_u8; MAX_PNG_BYTES + 1];
        assert_eq!(
            PngInfo::parse(&big),
            Err(KittyGraphicsError::PngTooLarge(MAX_PNG_BYTES + 1))
        );
    }

    #[test]
    fn png_geometry_decides_passthrough() {
        let info = PngInfo::parse(&png_header(640, 480, 0)).unwrap();
        assert_eq!(
            info,
            PngInfo {
                width: 640,
                height: 480
            }
        );
        assert!(info.fits_within(640, 480));
        assert!(info.fits_within(800, 600));
        assert!(!info.fits_within(639, 480));
        assert!(!info.fits_within(640, 479));
        assert_eq!(
            PngInfo::parse(&png_1x1()).unwrap(),
            PngInfo {
                width: 1,
                height: 1
            }
        );
    }

    #[test]
    fn query_payloads_are_valid_for_their_class() {
        assert_eq!(PngInfo::parse(&png_1x1()).unwrap().width, 1);
        assert_eq!(unb64(RGBA_QUERY_PAYLOAD), vec![0; 4]);
        let mut inflated = Vec::new();
        flate2::read::ZlibDecoder::new(unb64(RGBA_ZLIB_QUERY_PAYLOAD).as_slice())
            .read_to_end(&mut inflated)
            .unwrap();
        assert_eq!(inflated, vec![0; 4]);
        assert_eq!(
            encode_query(PayloadClass::RgbaZlib),
            "\x1b_Ga=q,t=d,f=32,s=1,v=1,o=z,i=43;eJxjYGBgAAAABAAB\x1b\\"
        );
        assert!(encode_query(PayloadClass::Png).starts_with("\x1b_Ga=q,t=d,f=100,i=41;iVBOR"));
        // Query IDs never collide with each other or the startup probe's 31.
        let ids: Vec<u32> = PayloadClass::ALL.iter().map(|c| c.query_id()).collect();
        assert_eq!(ids, [41, 42, 43]);
    }

    #[test]
    fn the_query_constants_carry_the_class_payloads() {
        assert_eq!(
            PNG_QUERY,
            format!("\x1b_Ga=q,t=d,f=100,i=41;{PNG_QUERY_PAYLOAD}\x1b\\")
        );
        assert_eq!(
            RGBA_QUERY,
            format!("\x1b_Ga=q,t=d,f=32,s=1,v=1,i=42;{RGBA_QUERY_PAYLOAD}\x1b\\")
        );
        assert_eq!(
            RGBA_ZLIB_QUERY,
            format!("\x1b_Ga=q,t=d,f=32,s=1,v=1,o=z,i=43;{RGBA_ZLIB_QUERY_PAYLOAD}\x1b\\")
        );
        for class in PayloadClass::ALL {
            assert_eq!(PayloadClass::from_query_id(class.query_id()), Some(class));
        }
        assert_eq!(PayloadClass::from_query_id(31), None);
    }

    #[test]
    fn a_recorded_answer_needs_the_kitty_bit_too() {
        let mut gate = GraphicsGate::new(true);
        gate.record_answer(PayloadClass::Png, true);
        gate.record_answer(PayloadClass::Rgba, false);
        assert!(gate.accepts(PayloadClass::Png));
        assert!(!gate.accepts(PayloadClass::Rgba));
        assert!(!gate.accepts(PayloadClass::RgbaZlib));
        let mut shut = GraphicsGate::new(false);
        shut.record_answer(PayloadClass::Png, true);
        assert!(!shut.accepts(PayloadClass::Png));
    }

    #[test]
    fn no_kitty_bit_means_no_query_and_no_bytes() {
        let mut gate = GraphicsGate::new(false);
        assert_eq!(gate.query(PayloadClass::Png), None);
        assert!(!gate.observe_reply(b"Gi=41;OK"));
        assert!(!gate.accepts(PayloadClass::Png));
        let png = png_1x1();
        assert_eq!(
            gate.encode_still(id(1), StillPayload::Png(&png), Action::Store),
            Ok(None)
        );
    }

    #[test]
    fn detection_alone_is_not_support() {
        let gate = GraphicsGate::new(true);
        let png = png_1x1();
        for class in PayloadClass::ALL {
            assert!(!gate.accepts(class));
        }
        assert_eq!(
            gate.encode_still(id(1), StillPayload::Png(&png), Action::Store),
            Ok(None)
        );
    }

    #[test]
    fn an_ok_reply_opens_only_its_own_class() {
        let mut gate = GraphicsGate::new(true);
        assert_eq!(
            gate.query(PayloadClass::Png).unwrap(),
            encode_query(PayloadClass::Png)
        );
        assert!(gate.observe_reply(b"Gi=41;OK"));
        assert!(gate.accepts(PayloadClass::Png));
        assert!(!gate.accepts(PayloadClass::Rgba));
        let png = png_1x1();
        let out = gate
            .encode_still(id(1), StillPayload::Png(&png), Action::Store)
            .unwrap()
            .unwrap();
        assert!(out.starts_with("\x1b_Ga=t,f=100"));
        let pixels = [0_u8; 4];
        assert_eq!(
            gate.encode_still(
                id(2),
                StillPayload::Rgba {
                    data: &pixels,
                    width: 1,
                    height: 1,
                    compress: true
                },
                Action::Store
            ),
            Ok(None)
        );
    }

    #[test]
    fn an_error_reply_keeps_the_class_shut() {
        let mut gate = GraphicsGate::new(true);
        gate.query(PayloadClass::RgbaZlib).unwrap();
        assert!(gate.observe_reply(b"Gi=43;ENOTSUPPORTED:compression"));
        assert!(!gate.accepts(PayloadClass::RgbaZlib));
    }

    #[test]
    fn only_replies_to_outstanding_queries_are_consumed() {
        let mut gate = GraphicsGate::new(true);
        // No query sent yet: an unsolicited OK is input, not support.
        assert!(!gate.observe_reply(b"Gi=41;OK"));
        assert!(!gate.accepts(PayloadClass::Png));
        gate.query(PayloadClass::Rgba).unwrap();
        // A reply for another class, or the startup probe's ID, is not ours.
        assert!(!gate.observe_reply(b"Gi=41;OK"));
        assert!(!gate.observe_reply(b"Gi=31;OK"));
        assert!(gate.observe_reply(b"Gi=42;OK"));
        assert!(gate.accepts(PayloadClass::Rgba));
        // The answer was consumed once; a repeat is input again.
        assert!(!gate.observe_reply(b"Gi=42;OK"));
    }
}
