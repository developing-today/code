//! Phase 3 blob offload, client core (code#13318, design §8, §10, §11).
//!
//! This module holds the parts of the blob plane that need no endpoint:
//!
//! - the versioned row envelope (`kv:raw-v1` for inline bytes, `kv:ref-v1`
//!   for a [`KvBlobRefV1`]); an unknown envelope version is refused, never
//!   ignored (D015);
//! - the `KvBlobRefV1` reference format, which reuses the #9120
//!   `raw-blake3-256` profile and carries the `(jurisdiction, tier)`
//!   partition (D010, D012); central accepts `published` references only;
//! - placement: the effective threshold is a hard inline ceiling,
//!   `min(namespace knob, partition ceiling)`, and `--force-blob` only ever
//!   moves a value toward the blob path (D009);
//! - streaming BLAKE3-256 over exact bytes in 64 KiB chunks, so a value of
//!   any size is hashed and verified with a fixed buffer, and a pull whose
//!   bytes do not match the reference is refused (§8, §9).
//!
//! The object plane itself (staging, publish-CAS, capabilities and GC) is
//! the next slice.

use std::io::{self, Read, Write};

/// The #9120 content-id profile every blob reference uses.
pub const PROFILE: &str = "raw-blake3-256";
/// The envelope version of an inline row.
pub const RAW_ENVELOPE: &str = "kv:raw-v1";
/// The envelope version of a blob reference row.
pub const REF_ENVELOPE: &str = "kv:ref-v1";
/// The only reference state central accepts.
pub const PUBLISHED: &str = "published";
/// Streaming I/O moves in chunks of at most this many bytes.
pub const CHUNK: usize = 64 * 1024;
/// The resident working-set budget of a push or pull.
pub const RESIDENT_BUDGET: usize = 1 << 20;
/// Component F's conservative starting partition ceiling (design §8).
pub const PARTITION_CEILING_START: u64 = 16 * 1024;

/// Why a blob operation was refused.
#[derive(Debug)]
pub enum BlobError {
    /// Bytes above the effective ceiling were addressed to an inline row.
    InlineAboveCeiling {
        len: u64,
        ceiling: u64,
    },
    /// A row carries an envelope version this client does not know.
    UnknownEnvelope(String),
    /// A reference row does not parse as `KvBlobRefV1`.
    MalformedRef(&'static str),
    /// Streamed bytes are not as long as the reference says.
    LengthMismatch {
        expected: u64,
        actual: u64,
    },
    /// Streamed bytes do not hash to the reference's digest.
    DigestMismatch {
        expected: String,
        actual: String,
    },
    /// A reference was resolved in a partition other than its own.
    PartitionMismatch {
        reference: Partition,
        caller: Partition,
    },
    Io(io::Error),
}

impl BlobError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InlineAboveCeiling { .. } => "inline_above_ceiling",
            Self::UnknownEnvelope(_) => "unknown_envelope",
            Self::MalformedRef(_) => "malformed_blob_ref",
            Self::LengthMismatch { .. } => "blob_length_mismatch",
            Self::DigestMismatch { .. } => "blob_digest_mismatch",
            Self::PartitionMismatch { .. } => "partition_mismatch",
            Self::Io(_) => "io",
        }
    }
}

impl std::fmt::Display for BlobError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InlineAboveCeiling { len, ceiling } => write!(
                f,
                "{len} bytes is above the {ceiling}-byte inline ceiling; it must take the blob path"
            ),
            Self::UnknownEnvelope(version) => {
                write!(
                    f,
                    "envelope version `{version}` is not one this client reads"
                )
            }
            Self::MalformedRef(field) => write!(f, "the blob reference has a bad {field}"),
            Self::LengthMismatch { expected, actual } => {
                write!(f, "expected {expected} bytes, got {actual}")
            }
            Self::DigestMismatch { expected, actual } => {
                write!(f, "expected digest {expected}, the bytes hash to {actual}")
            }
            Self::PartitionMismatch { reference, caller } => write!(
                f,
                "the reference lives in {reference} and cannot resolve in {caller}"
            ),
            Self::Io(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for BlobError {}

impl From<io::Error> for BlobError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// A dedup partition (#3401, component F §4). Dedup never crosses it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Partition {
    pub jurisdiction: String,
    pub tier: String,
}

impl Partition {
    pub fn new(jurisdiction: &str, tier: &str) -> Self {
        Self {
            jurisdiction: jurisdiction.to_string(),
            tier: tier.to_string(),
        }
    }
}

impl std::fmt::Display for Partition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "({}, {})", self.jurisdiction, self.tier)
    }
}

/// A published blob reference (design §10). The profile is always
/// [`PROFILE`] and the state always [`PUBLISHED`]: a reference in any other
/// state never reaches a central row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KvBlobRefV1 {
    /// Lower-case hex BLAKE3-256 over the exact bytes.
    pub digest: String,
    pub bytes_len: u64,
    pub partition: Partition,
    /// The adapter's private locator: opaque, never a stable API.
    pub locator: String,
}

const REF_FIELDS: [&str; 7] = [
    "profile",
    "digest",
    "bytes_len",
    "jurisdiction",
    "tier",
    "locator",
    "state",
];

impl KvBlobRefV1 {
    /// The canonical encoding: one `field=value` line per field, in a fixed
    /// order. Decoding accepts exactly this shape.
    pub fn encode(&self) -> Vec<u8> {
        let values = [
            PROFILE,
            &self.digest,
            &self.bytes_len.to_string(),
            &self.partition.jurisdiction,
            &self.partition.tier,
            &self.locator,
            PUBLISHED,
        ];
        let mut encoded = String::new();
        for (field, value) in REF_FIELDS.iter().zip(values) {
            encoded.push_str(field);
            encoded.push('=');
            encoded.push_str(value);
            encoded.push('\n');
        }
        encoded.into_bytes()
    }

    /// Parse a reference row's payload. Every field must be present, in
    /// order, well formed, and the state must be `published`.
    pub fn decode(bytes: &[u8]) -> Result<Self, BlobError> {
        let text = std::str::from_utf8(bytes).map_err(|_| BlobError::MalformedRef("encoding"))?;
        let body = text
            .strip_suffix('\n')
            .ok_or(BlobError::MalformedRef("encoding"))?;
        let mut values = Vec::with_capacity(REF_FIELDS.len());
        let mut lines = body.split('\n');
        for field in REF_FIELDS {
            let line = lines.next().ok_or(BlobError::MalformedRef(field))?;
            let value = line
                .strip_prefix(field)
                .and_then(|rest| rest.strip_prefix('='))
                .ok_or(BlobError::MalformedRef(field))?;
            values.push(value);
        }
        if lines.next().is_some() {
            return Err(BlobError::MalformedRef("encoding"));
        }
        let &[
            profile,
            digest,
            bytes_len,
            jurisdiction,
            tier,
            locator,
            state,
        ] = &values[..]
        else {
            return Err(BlobError::MalformedRef("encoding"));
        };
        if profile != PROFILE {
            return Err(BlobError::MalformedRef("profile"));
        }
        if !is_digest(digest) {
            return Err(BlobError::MalformedRef("digest"));
        }
        let bytes_len = bytes_len
            .parse::<u64>()
            .ok()
            .filter(|len| len.to_string() == bytes_len)
            .ok_or(BlobError::MalformedRef("bytes_len"))?;
        if !is_label(jurisdiction) {
            return Err(BlobError::MalformedRef("jurisdiction"));
        }
        if !is_label(tier) {
            return Err(BlobError::MalformedRef("tier"));
        }
        if locator.is_empty() || locator.chars().any(char::is_control) {
            return Err(BlobError::MalformedRef("locator"));
        }
        if state != PUBLISHED {
            return Err(BlobError::MalformedRef("state"));
        }
        Ok(Self {
            digest: digest.to_string(),
            bytes_len,
            partition: Partition::new(jurisdiction, tier),
            locator: locator.to_string(),
        })
    }

    /// This reference, for a caller in `partition`: a reference never
    /// resolves across partitions (design §10).
    pub fn resolve_in(&self, partition: &Partition) -> Result<&Self, BlobError> {
        if &self.partition == partition {
            Ok(self)
        } else {
            Err(BlobError::PartitionMismatch {
                reference: self.partition.clone(),
                caller: partition.clone(),
            })
        }
    }
}

fn is_digest(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Whether `text` is a content digest: 64 lower-case hex digits.
pub fn valid_digest(text: &str) -> bool {
    is_digest(text)
}

/// Whether `text` is a partition label: 1 to 63 of `a-z`, `0-9` and `-`,
/// the shape the upload door accepts.
pub fn valid_label(text: &str) -> bool {
    is_label(text)
}

fn is_label(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 63
        && text
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

/// A row's value with its envelope version (design §11).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Envelope {
    /// Inline bytes.
    Raw(Vec<u8>),
    /// A published blob reference.
    Ref(KvBlobRefV1),
}

impl Envelope {
    /// Read a row from its envelope version and stored bytes. A version this
    /// client does not know is refused.
    pub fn from_row(version: &str, bytes: &[u8]) -> Result<Self, BlobError> {
        match version {
            RAW_ENVELOPE => Ok(Self::Raw(bytes.to_vec())),
            REF_ENVELOPE => Ok(Self::Ref(KvBlobRefV1::decode(bytes)?)),
            other => Err(BlobError::UnknownEnvelope(other.to_string())),
        }
    }

    /// The envelope version and the bytes to store.
    pub fn to_row(&self) -> (&'static str, Vec<u8>) {
        match self {
            Self::Raw(bytes) => (RAW_ENVELOPE, bytes.clone()),
            Self::Ref(reference) => (REF_ENVELOPE, reference.encode()),
        }
    }

    /// The length of the value the row stands for.
    pub fn value_len(&self) -> u64 {
        match self {
            Self::Raw(bytes) => bytes.len() as u64,
            Self::Ref(reference) => reference.bytes_len,
        }
    }
}

/// The effective inline ceiling: the namespace knob, narrowed by the
/// partition's ceiling where component E governs one (D009). A partition
/// ceiling can only narrow, never widen.
pub fn effective_threshold(namespace_knob: u64, partition_ceiling: Option<u64>) -> u64 {
    partition_ceiling.map_or(namespace_knob, |ceiling| ceiling.min(namespace_knob))
}

/// Where a value goes in a central row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    Inline,
    Blob,
}

/// Placement is a pure function of the length, the effective threshold and
/// `--force-blob`: above the threshold is always a blob, and `--force-blob`
/// moves a value at or below it to the blob path. Nothing moves a value
/// the other way.
pub fn place(len: u64, effective: u64, force_blob: bool) -> Placement {
    if force_blob || len > effective {
        Placement::Blob
    } else {
        Placement::Inline
    }
}

/// Refuse an inline row above the effective ceiling.
pub fn check_inline(len: u64, effective: u64) -> Result<(), BlobError> {
    if len > effective {
        Err(BlobError::InlineAboveCeiling {
            len,
            ceiling: effective,
        })
    } else {
        Ok(())
    }
}

/// The length and BLAKE3-256 of a stream of exact bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Digest {
    pub bytes_len: u64,
    /// Lower-case hex BLAKE3-256.
    pub digest: String,
}

/// Hash `reader` to its end with one [`CHUNK`]-sized buffer, passing every
/// byte on to `out`.
pub fn digest_copy(reader: &mut dyn Read, out: &mut dyn Write) -> io::Result<Digest> {
    let mut hasher = blake3::Hasher::new();
    let mut chunk = vec![0; CHUNK];
    let mut bytes_len = 0u64;
    loop {
        let read = match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        hasher.update(&chunk[..read]);
        out.write_all(&chunk[..read])?;
        bytes_len += read as u64;
    }
    Ok(Digest {
        bytes_len,
        digest: hasher.finalize().to_hex().to_string(),
    })
}

/// Hash `reader` to its end without keeping the bytes.
pub fn digest(reader: &mut dyn Read) -> io::Result<Digest> {
    digest_copy(reader, &mut io::sink())
}

/// Stream a pulled blob into `out`, refusing it unless it is exactly
/// `reference.bytes_len` bytes that hash to `reference.digest`. Reading
/// stops at the first byte past the expected length. On a refusal some
/// bytes may already be in `out`: the caller discards them and serves
/// nothing.
pub fn verify_copy(
    reference: &KvBlobRefV1,
    reader: &mut dyn Read,
    out: &mut dyn Write,
) -> Result<u64, BlobError> {
    // One byte past the expected length shows an over-long stream without
    // reading the rest of it.
    let mut limited = reader.take(reference.bytes_len.saturating_add(1));
    let got = digest_copy(&mut limited, out)?;
    if got.bytes_len != reference.bytes_len {
        return Err(BlobError::LengthMismatch {
            expected: reference.bytes_len,
            actual: got.bytes_len,
        });
    }
    if got.digest != reference.digest {
        return Err(BlobError::DigestMismatch {
            expected: reference.digest.clone(),
            actual: got.digest,
        });
    }
    Ok(got.bytes_len)
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn reference(bytes: &[u8]) -> KvBlobRefV1 {
        KvBlobRefV1 {
            digest: blake3::hash(bytes).to_hex().to_string(),
            bytes_len: bytes.len() as u64,
            partition: Partition::new("eu", "standard"),
            locator: "r2:objects/abc".to_string(),
        }
    }

    /// Deterministic pseudo-random bytes, generated as they are read so a
    /// large stream is never held in memory. It records the largest buffer
    /// any read asked it to fill.
    struct Generated {
        remaining: u64,
        state: u64,
        largest_read: usize,
    }

    impl Generated {
        fn new(len: u64, seed: u64) -> Self {
            Self {
                remaining: len,
                state: seed | 1,
                largest_read: 0,
            }
        }
    }

    impl Read for Generated {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.largest_read = self.largest_read.max(buf.len());
            let take = buf
                .len()
                .min(usize::try_from(self.remaining).unwrap_or(usize::MAX));
            for chunk in buf[..take].chunks_mut(8) {
                self.state ^= self.state << 13;
                self.state ^= self.state >> 7;
                self.state ^= self.state << 17;
                let word = self.state.to_le_bytes();
                chunk.copy_from_slice(&word[..chunk.len()]);
            }
            self.remaining -= take as u64;
            Ok(take)
        }
    }

    /// A sink that keeps only a running hash and the largest single write.
    #[derive(Default)]
    struct Hashing {
        hasher: blake3::Hasher,
        written: u64,
        largest_write: usize,
    }

    impl Write for Hashing {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.largest_write = self.largest_write.max(buf.len());
            self.hasher.update(buf);
            self.written += buf.len() as u64;
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn blob_effective_threshold_only_narrows() {
        assert_eq!(effective_threshold(256 * 1024, None), 256 * 1024);
        assert_eq!(
            effective_threshold(256 * 1024, Some(PARTITION_CEILING_START)),
            PARTITION_CEILING_START
        );
        // A partition ceiling above the knob never widens it.
        assert_eq!(effective_threshold(4096, Some(1 << 20)), 4096);
    }

    #[test]
    fn blob_threshold_boundary_cases() {
        let effective = effective_threshold(256 * 1024, Some(PARTITION_CEILING_START));
        assert_eq!(place(effective - 1, effective, false), Placement::Inline);
        assert_eq!(place(effective, effective, false), Placement::Inline);
        assert_eq!(place(effective + 1, effective, false), Placement::Blob);
        assert_eq!(place(0, effective, true), Placement::Blob);
        assert_eq!(place(effective, effective, true), Placement::Blob);
        assert!(check_inline(effective, effective).is_ok());
        let refused = check_inline(effective + 1, effective).expect_err("above the ceiling");
        assert_eq!(refused.code(), "inline_above_ceiling");
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// Placement is a pure function of (length, effective threshold,
        /// --force-blob), and --force-blob only moves toward the blob path.
        #[test]
        fn blob_placement_is_pure_and_force_only_moves_toward_blob(
            knob in 1u64..(1 << 20),
            ceiling in proptest::option::of(1u64..(1 << 20)),
            offset in -2i64..=2,
        ) {
            let effective = effective_threshold(knob, ceiling);
            let len = effective.saturating_add_signed(offset);
            let plain = place(len, effective, false);
            prop_assert_eq!(plain, place(len, effective, false));
            prop_assert_eq!(plain == Placement::Blob, len > effective);
            prop_assert_eq!(place(len, effective, true), Placement::Blob);
            prop_assert_eq!(check_inline(len, effective).is_ok(), plain == Placement::Inline);
        }

        /// Any truncation, extension or flipped byte is refused; the exact
        /// bytes pass and re-hash to the reference digest.
        #[test]
        fn blob_corruption_is_refused(
            bytes in proptest::collection::vec(any::<u8>(), 1..(3 * CHUNK)),
            flip in any::<prop::sample::Index>(),
            cut in any::<prop::sample::Index>(),
        ) {
            let wanted = reference(&bytes);
            let mut out = Vec::new();
            prop_assert_eq!(
                verify_copy(&wanted, &mut &bytes[..], &mut out).expect("exact bytes"),
                bytes.len() as u64
            );
            prop_assert_eq!(&out, &bytes);

            let mut flipped = bytes.clone();
            flipped[flip.index(bytes.len())] ^= 0x01;
            let refused = verify_copy(&wanted, &mut &flipped[..], &mut Vec::new())
                .expect_err("a flipped byte");
            prop_assert_eq!(refused.code(), "blob_digest_mismatch");

            let short = &bytes[..cut.index(bytes.len())];
            let refused = verify_copy(&wanted, &mut &short[..], &mut Vec::new())
                .expect_err("truncated");
            prop_assert_eq!(refused.code(), "blob_length_mismatch");

            let mut long = bytes.clone();
            long.push(0);
            let refused = verify_copy(&wanted, &mut &long[..], &mut Vec::new())
                .expect_err("extended");
            prop_assert_eq!(refused.code(), "blob_length_mismatch");
        }

        /// Reads of any chunking hash to the same digest.
        #[test]
        fn blob_digest_ignores_how_reads_are_split(
            bytes in proptest::collection::vec(any::<u8>(), 0..(2 * CHUNK)),
            step in 1usize..5000,
        ) {
            struct Dribble<'a> { bytes: &'a [u8], step: usize }
            impl Read for Dribble<'_> {
                fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                    let take = buf.len().min(self.step).min(self.bytes.len());
                    buf[..take].copy_from_slice(&self.bytes[..take]);
                    self.bytes = &self.bytes[take..];
                    Ok(take)
                }
            }
            let whole = digest(&mut &bytes[..]).expect("digest");
            let dribbled = digest(&mut Dribble { bytes: &bytes, step }).expect("digest");
            prop_assert_eq!(&whole, &dribbled);
            prop_assert_eq!(whole.digest, blake3::hash(&bytes).to_hex().to_string());
        }
    }

    /// A value far above any threshold and larger than the streaming
    /// buffer hashes, then verifies, with every read and write at most one
    /// chunk: the resident working set stays inside the 1 MiB budget.
    #[test]
    fn blob_streams_64_mib_with_a_bounded_buffer() {
        let len = 64 * 1024 * 1024 + 17;
        let mut source = Generated::new(len, 0x9e37_79b9_7f4a_7c15);
        let mut sink = Hashing::default();
        let pushed = digest_copy(&mut source, &mut sink).expect("push");
        assert_eq!(pushed.bytes_len, len);
        assert_eq!(sink.written, len);
        assert_eq!(pushed.digest, sink.hasher.finalize().to_hex().to_string());
        assert!(source.largest_read <= CHUNK, "{}", source.largest_read);
        assert!(sink.largest_write <= CHUNK, "{}", sink.largest_write);
        const { assert!(CHUNK <= RESIDENT_BUDGET) };

        let wanted = KvBlobRefV1 {
            digest: pushed.digest.clone(),
            bytes_len: pushed.bytes_len,
            partition: Partition::new("eu", "standard"),
            locator: "r2:objects/big".to_string(),
        };
        let mut pulled = Generated::new(len, 0x9e37_79b9_7f4a_7c15);
        let mut received = Hashing::default();
        assert_eq!(
            verify_copy(&wanted, &mut pulled, &mut received).expect("pull"),
            len
        );
        assert_eq!(
            received.hasher.finalize().to_hex().to_string(),
            pushed.digest
        );
        assert!(pulled.largest_read <= CHUNK, "{}", pulled.largest_read);

        // Different bytes of the same length are refused.
        let mut other = Generated::new(len, 7);
        let refused = verify_copy(&wanted, &mut other, &mut io::sink()).expect_err("other bytes");
        assert_eq!(refused.code(), "blob_digest_mismatch");
    }

    #[test]
    fn blob_envelope_round_trips_and_unknown_versions_refuse() {
        let inline = Envelope::Raw(b"hello".to_vec());
        let (version, bytes) = inline.to_row();
        assert_eq!(version, RAW_ENVELOPE);
        assert_eq!(Envelope::from_row(version, &bytes).expect("raw"), inline);

        let blob = Envelope::Ref(reference(b"a large value"));
        let (version, bytes) = blob.to_row();
        assert_eq!(version, REF_ENVELOPE);
        assert_eq!(Envelope::from_row(version, &bytes).expect("ref"), blob);
        assert_eq!(blob.value_len(), 13);

        for version in ["kv:raw-v2", "kv:list-v1", "", "KV:RAW-V1"] {
            let refused = Envelope::from_row(version, b"x").expect_err("unknown");
            assert_eq!(refused.code(), "unknown_envelope", "{version}");
        }
    }

    #[test]
    fn blob_reference_encoding_is_canonical_and_strict() {
        let wanted = reference(b"bytes");
        let encoded = String::from_utf8(wanted.encode()).expect("utf8");
        assert_eq!(
            encoded,
            format!(
                "profile=raw-blake3-256\ndigest={}\nbytes_len=5\njurisdiction=eu\ntier=standard\nlocator=r2:objects/abc\nstate=published\n",
                wanted.digest
            )
        );
        let refusals = [
            (encoded.replace("raw-blake3-256", "raw-sha256"), "profile"),
            (
                encoded.replace(&wanted.digest, &wanted.digest.to_uppercase()),
                "digest",
            ),
            (
                encoded.replace(&wanted.digest, &wanted.digest[..63]),
                "digest",
            ),
            (encoded.replace("bytes_len=5", "bytes_len=05"), "bytes_len"),
            (encoded.replace("bytes_len=5", "bytes_len=-5"), "bytes_len"),
            (
                encoded.replace("jurisdiction=eu", "jurisdiction=EU"),
                "jurisdiction",
            ),
            (encoded.replace("tier=standard", "tier="), "tier"),
            (
                encoded.replace("locator=r2:objects/abc", "locator="),
                "locator",
            ),
            (encoded.replace("state=published", "state=staged"), "state"),
            (
                encoded.replace("state=published", "state=verified"),
                "state",
            ),
            (encoded.replace("tier=standard\n", ""), "tier"),
            (format!("{encoded}extra=1\n"), "encoding"),
            (encoded.trim_end().to_string(), "encoding"),
        ];
        for (text, field) in refusals {
            match KvBlobRefV1::decode(text.as_bytes()) {
                Err(BlobError::MalformedRef(got)) => assert_eq!(got, field, "{text}"),
                other => panic!("{text}: expected a malformed {field}, got {other:?}"),
            }
        }
    }

    #[test]
    fn blob_references_never_resolve_across_partitions() {
        let wanted = reference(b"bytes");
        assert!(wanted.resolve_in(&Partition::new("eu", "standard")).is_ok());
        for other in [
            Partition::new("us", "standard"),
            Partition::new("eu", "sensitive"),
        ] {
            let refused = wanted.resolve_in(&other).expect_err("other partition");
            assert_eq!(refused.code(), "partition_mismatch");
        }
    }
}
