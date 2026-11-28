//! The pkarr publishing path for the gallery registry (#8281, criterion 2).
//!
//! A [`BundleRecord`](crate::program_bundle::BundleRecord) cannot ride pkarr
//! directly: a manifest alone may be [`MAX_MANIFEST_BYTES`] (64 KiB) while a
//! pkarr signed DNS packet is capped at [`PKARR_MAX_PACKET_BYTES`] (1000
//! bytes). The encoding decision, recorded on #8281, is therefore
//! pointer-only: the pkarr TXT record carries the bundle's D1 identity
//! (author key, name, version, BLAKE3 content hash) and the full signed
//! record stays on a registry the client fetches from over iroh (or plain
//! transport). The client then runs the identical
//! [`BundleRecord::verify`](crate::program_bundle::BundleRecord::verify)
//! path, so a pkarr-resolved record verifies identically to an
//! iroh-fetched one.
//!
//! Key binding: the author's bundle-signing Ed25519 key doubles as the
//! pkarr publishing key, so the pointer is self-certifying under the same
//! key the fetched record verifies under. No new trust root.
//!
//! This module is the transport-free core: canonical encode/decode of the
//! pointer TXT value with the packet budget enforced at encode time. Moving
//! these bytes (pkarr publish/resolve) is a later transport slice.

use crate::program_bundle::{BundleError, BundleManifest, ProgramIdentity, check_identity};

/// The pkarr signed DNS packet ceiling every pointer encoding must fit in.
pub const PKARR_MAX_PACKET_BYTES: usize = 1000;

/// Version tag of the canonical pointer TXT encoding below.
const POINTER_ENCODING_VERSION: u32 = 1;

/// A gallery bundle's D1 identity in pkarr-publishable form.
///
/// Field-for-field this is [`ProgramIdentity`]; the distinct type marks the
/// values that have passed the pkarr suitability checks (name length,
/// TXT-safe characters) and fit the packet budget once encoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GalleryPkarrPointer {
    /// The author's Ed25519 public key, lower-case hex (32 bytes).
    pub author: String,
    /// The program's name, scoped to its author. TXT-safe, non-empty.
    pub name: String,
    /// The author's monotonic version for this name.
    pub version: u64,
    /// BLAKE3 of the payload, lower-case hex (32 bytes).
    pub content_hash: String,
}

impl GalleryPkarrPointer {
    /// Build the pointer for `manifest`'s identity.
    #[must_use]
    pub fn from_manifest(manifest: &BundleManifest) -> Self {
        let identity = manifest.identity();
        Self {
            author: identity.author,
            name: identity.name,
            version: identity.version,
            content_hash: identity.content_hash,
        }
    }

    /// The D1 identity this pointer names.
    #[must_use]
    pub fn identity(&self) -> ProgramIdentity {
        ProgramIdentity {
            author: self.author.clone(),
            name: self.name.clone(),
            version: self.version,
            content_hash: self.content_hash.clone(),
        }
    }

    /// Encode the pointer as its canonical pkarr TXT value, enforcing the
    /// packet budget.
    ///
    /// # Errors
    ///
    /// The identity fails [`check_identity`], the name carries characters
    /// the `;`-separated encoding cannot carry (`;`, `=`), or the encoded
    /// value would exceed [`PKARR_MAX_PACKET_BYTES`].
    pub fn encode(&self) -> Result<String, BundleError> {
        check_identity(&self.identity()).map_err(BundleError::Manifest)?;
        if self.name.contains([';', '=']) {
            return Err(BundleError::Manifest(
                "name carries a pkarr TXT separator (';' or '='); publish via a registry instead"
                    .to_owned(),
            ));
        }
        let encoded = format!(
            "v={POINTER_ENCODING_VERSION};author={};name={};version={};hash={}",
            self.author, self.name, self.version, self.content_hash
        );
        if encoded.len() > PKARR_MAX_PACKET_BYTES {
            return Err(BundleError::Manifest(format!(
                "pkarr pointer is {} bytes; the packet limit is {PKARR_MAX_PACKET_BYTES}",
                encoded.len()
            )));
        }
        Ok(encoded)
    }

    /// Decode a canonical pointer TXT value, validating every field.
    ///
    /// # Errors
    ///
    /// The value is not the canonical encoding (wrong version, missing or
    /// extra fields, malformed version) or the decoded identity fails
    /// [`check_identity`].
    pub fn decode(txt: &str) -> Result<Self, BundleError> {
        let mut author = None;
        let mut name = None;
        let mut version = None;
        let mut hash = None;
        let mut seen_version = false;
        for part in txt.split(';') {
            let (key, value) = part.split_once('=').ok_or_else(|| {
                BundleError::Manifest("pkarr pointer part has no '='".to_owned())
            })?;
            match key {
                "v" => {
                    if value != POINTER_ENCODING_VERSION.to_string() {
                        return Err(BundleError::Manifest(format!(
                            "pkarr pointer version is {value}; this build reads v{POINTER_ENCODING_VERSION}"
                        )));
                    }
                    seen_version = true;
                }
                "author" => author = Some(value.to_owned()),
                "name" => name = Some(value.to_owned()),
                "version" => {
                    version = Some(value.parse::<u64>().map_err(|_| {
                        BundleError::Manifest("pkarr pointer version is not a u64".to_owned())
                    })?);
                }
                "hash" => hash = Some(value.to_owned()),
                _ => {
                    return Err(BundleError::Manifest(format!(
                        "pkarr pointer carries unknown field '{key}'"
                    )));
                }
            }
        }
        let pointer = Self {
            author: author
                .ok_or_else(|| BundleError::Manifest("pkarr pointer is missing 'author'".to_owned()))?,
            name: name
                .ok_or_else(|| BundleError::Manifest("pkarr pointer is missing 'name'".to_owned()))?,
            version: version.ok_or_else(|| {
                BundleError::Manifest("pkarr pointer is missing 'version'".to_owned())
            })?,
            content_hash: hash
                .ok_or_else(|| BundleError::Manifest("pkarr pointer is missing 'hash'".to_owned()))?,
        };
        if !seen_version {
            return Err(BundleError::Manifest(
                "pkarr pointer is missing its version tag".to_owned(),
            ));
        }
        check_identity(&pointer.identity()).map_err(BundleError::Manifest)?;
        Ok(pointer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;

    fn author_hex() -> String {
        let verifying = SigningKey::from_bytes(&[1; 32]).verifying_key();
        crate::world_session::encode_hex(verifying.as_bytes())
    }

    fn sample_pointer() -> GalleryPkarrPointer {
        GalleryPkarrPointer {
            author: author_hex(),
            name: "gallery-demo".to_owned(),
            version: 7,
            content_hash: "cd".repeat(32),
        }
    }

    #[test]
    fn pointer_round_trips_through_its_canonical_encoding() {
        let pointer = sample_pointer();
        let decoded = GalleryPkarrPointer::decode(&pointer.encode().unwrap()).unwrap();
        assert_eq!(pointer, decoded);
    }

    #[test]
    fn max_size_pointer_still_fits_the_packet_budget() {
        let pointer = GalleryPkarrPointer {
            name: "n".repeat(crate::program_bundle::MAX_NAME_BYTES),
            version: u64::MAX,
            ..sample_pointer()
        };
        let encoded = pointer.encode().unwrap();
        assert!(
            encoded.len() <= PKARR_MAX_PACKET_BYTES,
            "max-size pointer is {} bytes; the packet limit is {PKARR_MAX_PACKET_BYTES}",
            encoded.len()
        );
    }

    #[test]
    fn full_record_scale_does_not_fit_the_packet_budget() {
        // The constraint that forces pointer-only: a manifest alone may be
        // 64 KiB, two orders of magnitude over the 1000-byte packet cap.
        assert!(crate::program_bundle::MAX_MANIFEST_BYTES > PKARR_MAX_PACKET_BYTES);
    }

    #[test]
    fn decode_refuses_unknown_fields_and_bad_versions() {
        let encoded = sample_pointer().encode().unwrap();
        assert!(GalleryPkarrPointer::decode(&format!("{encoded};extra=1")).is_err());
        assert!(GalleryPkarrPointer::decode(&encoded.replace("v=1", "v=2")).is_err());
        assert!(GalleryPkarrPointer::decode("author=ab").is_err());
    }

    #[test]
    fn encode_refuses_names_the_txt_encoding_cannot_carry() {
        let mut pointer = sample_pointer();
        pointer.name = "a;b".to_owned();
        assert!(pointer.encode().is_err());
        pointer.name = "a=b".to_owned();
        assert!(pointer.encode().is_err());
    }

    #[test]
    fn decode_refuses_identities_that_fail_the_identity_check() {
        // Corrupt the encoded value after the fact and confirm decode
        // still refuses it: the author field is no longer 32 bytes of hex.
        let pointer = sample_pointer();
        let encoded = pointer.encode().unwrap();
        let corrupted = encoded.replacen(&pointer.author, "not-hex", 1);
        assert_ne!(encoded, corrupted);
        assert!(GalleryPkarrPointer::decode(&corrupted).is_err());
    }
}
