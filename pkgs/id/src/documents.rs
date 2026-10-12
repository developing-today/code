//! Documents as signed events over content hashes (Phase 5, P5.4; design §9.3).
//!
//! A document is a `document.put` event. Its header names the content by BLAKE3 hash and size,
//! and the media type, in a [`DocumentRef`]. The header carries it as the tag operation does
//! (#182), so the event is signed with its document and sync moves the event alone. The bytes
//! are not in the event. They are a blob, which iroh-blobs moves on request through the gate in
//! [`crate::blob_gate`].
//!
//! A replica that stores fetched bytes checks them with [`DocumentRef::verify_content`] first. Bytes
//! whose length or hash differ from the document's are refused.
//!
//! Tags on a document are ordinary `tag.set` and `tag.clear` events whose subject is the document
//! event's ID, so they follow the tag model in [`crate::tag_claims`]. A clear removes a tag claim,
//! never the document.
//!
//! Inline content (design §9.3, `json` and `bytes`) is not built. A body does not travel with
//! sync, and a header holds byte strings of at most 512 bytes, so content is always a blob here.

use p2panda_core::Hash;
use serde::{Deserialize, Serialize};
use unicode_normalization::is_nfc;

use crate::waiting::AuthorKey;

/// The kind of a `document.put` event, as the header's `kind` names it.
pub const DOCUMENT_KIND: &str = "document.put";

/// The most bytes a media type may have. A media type is at least one byte.
pub const MAX_MEDIA_TYPE_BYTES: usize = 128;

/// A document's content, as its `document.put` event names it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentRef {
    /// The media type of the content, for example `text/markdown`. NFC, 1 to 128 bytes.
    pub media_type: String,
    /// The BLAKE3 hash of the content. It is also the blob's hash.
    pub content: Hash,
    /// The content's length in bytes.
    pub size: u64,
}

/// Why a document was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DocumentError {
    /// The media type is empty, longer than [`MAX_MEDIA_TYPE_BYTES`], or not NFC.
    #[error("{0}")]
    MediaType(String),
    /// The content is not the length the document names.
    #[error("the content is {actual} bytes, the document names {named}")]
    Size {
        /// The length the document names.
        named: u64,
        /// The length of the content.
        actual: u64,
    },
    /// The content does not hash to the document's content hash.
    #[error("the content does not hash to the document's content hash")]
    Hash,
}

/// Whether `kind` is the document kind, so its event carries a document.
#[must_use]
pub fn is_document_kind(kind: &str) -> bool {
    kind == DOCUMENT_KIND
}

impl DocumentRef {
    /// The reference to `bytes`, as a document of `media_type`.
    ///
    /// # Errors
    ///
    /// Returns [`DocumentError::MediaType`] for a media type that [`DocumentRef::check`] refuses.
    pub fn for_bytes(media_type: &str, bytes: &[u8]) -> Result<Self, DocumentError> {
        let doc = Self {
            media_type: media_type.to_owned(),
            content: Hash::digest(bytes),
            size: len_of(bytes),
        };
        doc.check()?;
        Ok(doc)
    }

    /// Checks the rules every document reference meets. The builder and the decoder both apply
    /// them, so a signed document always passes them on receipt.
    ///
    /// # Errors
    ///
    /// Returns [`DocumentError::MediaType`] for a media type that is empty, longer than
    /// [`MAX_MEDIA_TYPE_BYTES`], or not NFC.
    pub fn check(&self) -> Result<(), DocumentError> {
        let length = self.media_type.len();
        if !(1..=MAX_MEDIA_TYPE_BYTES).contains(&length) {
            return Err(DocumentError::MediaType(format!(
                "the media type is {length} bytes, it must be 1 to {MAX_MEDIA_TYPE_BYTES}"
            )));
        }
        if !is_nfc(&self.media_type) {
            return Err(DocumentError::MediaType(
                "the media type is not NFC".to_owned(),
            ));
        }
        Ok(())
    }

    /// Checks `bytes` against the reference: the length first, then the hash.
    ///
    /// # Errors
    ///
    /// Returns [`DocumentError::Size`] when the length differs, and [`DocumentError::Hash`] when
    /// the bytes do not hash to the content hash.
    pub fn verify_content(&self, bytes: &[u8]) -> Result<(), DocumentError> {
        let actual = len_of(bytes);
        if actual != self.size {
            return Err(DocumentError::Size {
                named: self.size,
                actual,
            });
        }
        if Hash::digest(bytes) != self.content {
            return Err(DocumentError::Hash);
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

/// A document in the store: its `document.put` event, the key that signed it, and its content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentRecord {
    /// The document's ID: the `document.put` event's ID. Tags name it as their subject.
    pub id: Hash,
    /// The key that signed the document event.
    pub author: AuthorKey,
    /// The content the event names.
    pub document: DocumentRef,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_the_content_is_checked_by_its_length_and_its_hash() {
        let body = b"quarterly figures";
        let doc = DocumentRef::for_bytes("text/plain", body).unwrap();
        assert!(doc.verify_content(body).is_ok());
        // Same length, one byte changed: the hash refuses it.
        assert_eq!(
            doc.verify_content(b"quarterly figurez"),
            Err(DocumentError::Hash)
        );
        // A prefix: the length refuses it before any hash is taken.
        assert!(matches!(
            doc.verify_content(&body[..body.len() - 1]),
            Err(DocumentError::Size { .. })
        ));
    }

    #[test]
    fn test_a_reference_that_names_the_wrong_length_is_refused() {
        let body = b"quarterly figures";
        let doc = DocumentRef {
            size: 3,
            ..DocumentRef::for_bytes("text/plain", body).unwrap()
        };
        assert!(matches!(
            doc.verify_content(body),
            Err(DocumentError::Size { named: 3, .. })
        ));
    }

    #[test]
    fn test_the_media_type_is_one_to_128_bytes_in_nfc() {
        assert!(DocumentRef::for_bytes("", b"x").is_err());
        let longest = "m".repeat(MAX_MEDIA_TYPE_BYTES);
        assert!(DocumentRef::for_bytes(&longest, b"x").is_ok());
        let too_long = "m".repeat(MAX_MEDIA_TYPE_BYTES + 1);
        assert!(DocumentRef::for_bytes(&too_long, b"x").is_err());
        // "e" and a combining acute accent is NFD, not NFC.
        assert!(DocumentRef::for_bytes("e\u{301}", b"x").is_err());
    }

    #[test]
    fn test_the_blob_hash_is_the_content_hash_of_the_bytes() {
        let body = b"the same bytes, hashed two ways";
        let doc = DocumentRef::for_bytes("text/plain", body).unwrap();
        assert_eq!(doc.blob_hash(), iroh_blobs::Hash::new(body));
    }
}
