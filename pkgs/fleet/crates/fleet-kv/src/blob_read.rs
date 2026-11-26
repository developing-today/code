//! Reading a central value under its row envelope (code#13318, design §9,
//! §11, D011, D015).
//!
//! A `kv:raw-v1` head's bytes are the value. A `kv:ref-v1` head holds a
//! published blob reference: the reader asks the trusted door for a
//! short-lived read capability minted from the key's live reference and
//! streams the bytes it names, which this client verifies against the
//! reference's length and digest before it reports success. An envelope
//! this client does not know is refused, never read as bytes.

use std::io::{Read, Write};

use crate::blob::{self, BlobError, Envelope, KvBlobRefV1};
use crate::central::{self, Address, CentralError};

/// The read side of the blob door: opens the published bytes of the blob
/// `reference` names, where `reference` is `address`'s live head. The
/// stream is checked by the caller; an implementation only fetches.
pub trait BlobReader {
    fn open(&self, address: &Address, reference: &KvBlobRefV1) -> central::Result<Box<dyn Read>>;
}

/// Why a central value could not be read.
#[derive(Debug)]
pub enum ValueError {
    Central(CentralError),
    Blob(BlobError),
}

impl From<CentralError> for ValueError {
    fn from(error: CentralError) -> Self {
        Self::Central(error)
    }
}

impl From<BlobError> for ValueError {
    fn from(error: BlobError) -> Self {
        Self::Blob(error)
    }
}

/// Writes the value a stored head stands for to `out` and returns its
/// length: inline bytes as they are, a blob reference through `blobs`,
/// verified. With no blob reader a reference is refused as
/// `central_not_configured`. On a verification failure some bytes may
/// already be in `out`; the caller reports the refusal and the reader
/// discards them.
pub fn write_value(
    envelope: &str,
    stored: &[u8],
    address: &Address,
    blobs: Option<&dyn BlobReader>,
    out: &mut dyn Write,
) -> Result<u64, ValueError> {
    match Envelope::from_row(envelope, stored)? {
        Envelope::Raw(bytes) => {
            out.write_all(&bytes).map_err(BlobError::Io)?;
            Ok(bytes.len() as u64)
        }
        Envelope::Ref(reference) => {
            let blobs = blobs.ok_or_else(|| {
                CentralError::NotConfigured(
                    "this value is a blob; reading it needs the blob door beside FLEET_KV_URL"
                        .into(),
                )
            })?;
            let mut stream = blobs.open(address, &reference)?;
            Ok(blob::verify_copy(&reference, &mut stream, out)?)
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::blob::{Partition, RAW_ENVELOPE, REF_ENVELOPE};

    struct Fixed(Vec<u8>);

    impl BlobReader for Fixed {
        fn open(
            &self,
            _address: &Address,
            _reference: &KvBlobRefV1,
        ) -> central::Result<Box<dyn Read>> {
            Ok(Box::new(std::io::Cursor::new(self.0.clone())))
        }
    }

    fn reference(bytes: &[u8]) -> KvBlobRefV1 {
        KvBlobRefV1 {
            digest: blake3::hash(bytes).to_hex().to_string(),
            bytes_len: bytes.len() as u64,
            partition: Partition::new("eu", "standard"),
            locator: "r2:kv/blobs/eu/standard/x".to_string(),
        }
    }

    fn address() -> Address {
        Address::new("shared:team", "status", "big")
    }

    #[test]
    fn raw_values_are_their_bytes_and_references_are_read_and_verified() {
        let mut out = Vec::new();
        let len = write_value(RAW_ENVELOPE, b"inline", &address(), None, &mut out).unwrap();
        assert_eq!((len, out.as_slice()), (6, &b"inline"[..]));

        let bytes = b"the blob's own bytes".to_vec();
        let encoded = reference(&bytes).encode();
        let door = Fixed(bytes.clone());
        let mut out = Vec::new();
        let len = write_value(REF_ENVELOPE, &encoded, &address(), Some(&door), &mut out).unwrap();
        assert_eq!(len, bytes.len() as u64);
        assert_eq!(out, bytes);
    }

    #[test]
    fn a_reference_is_never_read_as_bytes() {
        let encoded = reference(b"x").encode();
        // No blob door: refused, nothing written.
        let mut out = Vec::new();
        let error = write_value(REF_ENVELOPE, &encoded, &address(), None, &mut out).unwrap_err();
        assert!(
            matches!(error, ValueError::Central(CentralError::NotConfigured(_))),
            "{error:?}"
        );
        assert!(out.is_empty());
        // Bytes that differ from the reference are refused.
        let liar = Fixed(b"y".to_vec());
        let error = write_value(
            REF_ENVELOPE,
            &encoded,
            &address(),
            Some(&liar),
            &mut Vec::new(),
        )
        .unwrap_err();
        assert!(
            matches!(error, ValueError::Blob(BlobError::DigestMismatch { .. })),
            "{error:?}"
        );
        let short = Fixed(Vec::new());
        let error = write_value(
            REF_ENVELOPE,
            &encoded,
            &address(),
            Some(&short),
            &mut Vec::new(),
        )
        .unwrap_err();
        assert!(
            matches!(error, ValueError::Blob(BlobError::LengthMismatch { .. })),
            "{error:?}"
        );
        // An envelope this client does not know.
        let error = write_value("kv:list-v9", b"x", &address(), None, &mut Vec::new()).unwrap_err();
        assert!(
            matches!(error, ValueError::Blob(BlobError::UnknownEnvelope(_))),
            "{error:?}"
        );
    }
}
