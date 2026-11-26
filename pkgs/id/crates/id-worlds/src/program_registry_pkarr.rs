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
//! The core is the canonical encode/decode of the pointer TXT value, with
//! the packet budget enforced at encode time. The transport rides the same
//! pkarr machinery world announcements use (`id_sync::announcement_pkarr`
//! and `announcement_relay`), under its own record label:
//!
//! - [`sign_pointers`] puts an author's pointers in one signed packet, one
//!   `TXT` record named `_idg.<z32 author key>` per program. A pkarr key
//!   holds one packet, so the packet is the author's whole current pkarr
//!   listing and a publish replaces it.
//! - [`publish_pointers`] sends that packet to the operator's relay sinks.
//! - [`resolve_pointers`] checks one relay payload; [`fetch_pointers`]
//!   asks every relay and keeps the newest packet that checks out.
//! - [`check_fetched`] closes the loop: the bundle fetched from a registry
//!   by the pointer's content hash (decoded, and so verified, exactly as a
//!   sideloaded file is) must carry the pointer's identity.

use ed25519_dalek::SigningKey;
use id_sync::announcement_pkarr::{PkarrItem, txt_packet, txt_records};
use id_sync::announcement_relay::{PublishReport, RelaySinks};
use id_sync::bridge_http::BridgeHttp;

use crate::program_bundle::{
    BundleError, BundleManifest, ProgramBundle, ProgramIdentity, check_identity,
};

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
            let (key, value) = part
                .split_once('=')
                .ok_or_else(|| BundleError::Manifest("pkarr pointer part has no '='".to_owned()))?;
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
            author: author.ok_or_else(|| {
                BundleError::Manifest("pkarr pointer is missing 'author'".to_owned())
            })?,
            name: name.ok_or_else(|| {
                BundleError::Manifest("pkarr pointer is missing 'name'".to_owned())
            })?,
            version: version.ok_or_else(|| {
                BundleError::Manifest("pkarr pointer is missing 'version'".to_owned())
            })?,
            content_hash: hash.ok_or_else(|| {
                BundleError::Manifest("pkarr pointer is missing 'hash'".to_owned())
            })?,
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

/// The DNS label gallery pointers live under, beside the world
/// announcements' `_ida`.
pub const GALLERY_RECORD_LABEL: &str = "_idg";

fn pkarr_refusal(message: impl std::fmt::Display) -> BundleError {
    BundleError::Manifest(format!("pkarr pointer packet refused: {message}"))
}

fn author_hex(key: &SigningKey) -> String {
    crate::world_session::encode_hex(key.verifying_key().as_bytes())
}

fn check_unique_names(pointers: &[GalleryPkarrPointer]) -> Result<(), BundleError> {
    let mut names = std::collections::BTreeSet::new();
    for pointer in pointers {
        if !names.insert(pointer.name.as_str()) {
            return Err(pkarr_refusal(format!(
                "program {:?} appears more than once",
                pointer.name
            )));
        }
    }
    Ok(())
}

/// Sign `pointers` into one pkarr packet under the author `key` at
/// `timestamp_us` (the BEP44 `seq`, microseconds since the Unix epoch).
///
/// # Errors
///
/// A pointer is not `key`'s, does not encode, or names a program twice, or
/// the packet would exceed the pkarr budget (publish the rest via a
/// registry instead). An empty list is refused too: it would publish an
/// empty listing that hides nothing and says nothing.
pub fn sign_pointers(
    key: &SigningKey,
    pointers: &[GalleryPkarrPointer],
    timestamp_us: u64,
) -> Result<PkarrItem, BundleError> {
    if pointers.is_empty() {
        return Err(pkarr_refusal("there is no pointer to publish"));
    }
    let author = author_hex(key);
    for pointer in pointers {
        if pointer.author != author {
            return Err(pkarr_refusal(format!(
                "program {:?} is by {}, not the publishing key {author}",
                pointer.name, pointer.author
            )));
        }
    }
    check_unique_names(pointers)?;
    let encoded = pointers
        .iter()
        .map(GalleryPkarrPointer::encode)
        .collect::<Result<Vec<_>, _>>()?;
    let records: Vec<&[u8]> = encoded.iter().map(String::as_bytes).collect();
    let dns = txt_packet(
        &key.verifying_key().to_bytes(),
        GALLERY_RECORD_LABEL,
        &records,
    )
    .map_err(|e| pkarr_refusal(format!("{e:#}")))?;
    PkarrItem::sign_packet(key, timestamp_us, dns).map_err(|e| pkarr_refusal(format!("{e:#}")))
}

/// Check one relay payload fetched for `author` and return its timestamp
/// and pointers.
///
/// # Errors
///
/// The pkarr signature does not verify under `author`, the packet is
/// malformed or carries no pointer, a pointer does not decode or names
/// another author, or a program appears twice.
pub fn resolve_pointers(
    author: &[u8; 32],
    payload: &[u8],
) -> Result<(u64, Vec<GalleryPkarrPointer>), BundleError> {
    let item =
        PkarrItem::from_relay_payload(payload).map_err(|e| pkarr_refusal(format!("{e:#}")))?;
    item.verify(author)
        .map_err(|e| pkarr_refusal(format!("{e:#}")))?;
    let records = txt_records(&item.dns_packet, author, GALLERY_RECORD_LABEL)
        .map_err(|e| pkarr_refusal(format!("{e:#}")))?;
    if records.is_empty() {
        return Err(pkarr_refusal("the packet carries no gallery pointer"));
    }
    let expected = crate::world_session::encode_hex(author);
    let mut pointers = Vec::with_capacity(records.len());
    for record in records {
        let text = String::from_utf8(record)
            .map_err(|_| pkarr_refusal("a pointer record is not UTF-8"))?;
        let pointer = GalleryPkarrPointer::decode(&text)?;
        if pointer.author != expected {
            return Err(pkarr_refusal(format!(
                "program {:?} names author {}, not the queried key {expected}",
                pointer.name, pointer.author
            )));
        }
        pointers.push(pointer);
    }
    check_unique_names(&pointers)?;
    Ok((item.timestamp_us, pointers))
}

/// Sign `pointers` (see [`sign_pointers`]) and send the packet to every
/// relay in `sinks`. The report says which relays accepted it.
///
/// # Errors
///
/// The pointers cannot be signed into a packet. Relay outcomes are in the
/// report, not errors.
pub async fn publish_pointers<H: BridgeHttp>(
    sinks: &RelaySinks,
    http: &mut H,
    key: &SigningKey,
    pointers: &[GalleryPkarrPointer],
    timestamp_us: u64,
) -> Result<PublishReport, BundleError> {
    let item = sign_pointers(key, pointers, timestamp_us)?;
    Ok(sinks
        .publish(http, &key.verifying_key().to_bytes(), &item)
        .await)
}

/// Fetch `author`'s pointer packet from every relay in `sinks` and return
/// the pointers of the newest one that resolves. A relay that is
/// unreachable, has nothing, or serves a packet that does not resolve is
/// skipped.
///
/// # Errors
///
/// No relay served a packet that resolves; the reasons are listed.
pub async fn fetch_pointers<H: BridgeHttp>(
    sinks: &RelaySinks,
    http: &mut H,
    author: &[u8; 32],
) -> Result<Vec<GalleryPkarrPointer>, BundleError> {
    let mut best: Option<(u64, Vec<GalleryPkarrPointer>)> = None;
    let mut reasons = Vec::new();
    for (base, payload) in sinks.fetch_payloads(http, author).await {
        let resolved = payload
            .map_err(|e| pkarr_refusal(format!("{e:#}")))
            .and_then(|body| resolve_pointers(author, &body));
        match resolved {
            Ok((timestamp, pointers)) => {
                if best
                    .as_ref()
                    .is_none_or(|(current, _)| timestamp > *current)
                {
                    best = Some((timestamp, pointers));
                }
            }
            Err(error) => reasons.push(format!("{base}: {error}")),
        }
    }
    match best {
        Some((_, pointers)) => Ok(pointers),
        None if reasons.is_empty() => Err(pkarr_refusal("no pkarr relay is configured")),
        None => Err(pkarr_refusal(format!(
            "no relay served a packet that resolves: {}",
            reasons.join("; ")
        ))),
    }
}

/// Check that `bundle`, fetched from a registry by `pointer`'s content hash
/// and decoded as a sideloaded file is, is the program the pointer names.
///
/// # Errors
///
/// The bundle's identity (author, name, version or content hash) differs
/// from the pointer's.
pub fn check_fetched(
    pointer: &GalleryPkarrPointer,
    bundle: &ProgramBundle,
) -> Result<(), BundleError> {
    let stated = pointer.identity();
    let actual = bundle.identity();
    if stated != actual {
        return Err(pkarr_refusal(format!(
            "the pointer names {stated:?} but the fetched bundle is {actual:?}"
        )));
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use id_sync::bridge_http::{HttpRequest, HttpResponse};

    use crate::program_bundle::{ProgramSpec, export};
    use crate::program_registry::{AdmitAll, Registry};

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

    // ---- transport (#8281 criterion 2, slice 2) ----

    const NOW_US: u64 = 1_764_000_000_000_000;

    fn alice() -> SigningKey {
        SigningKey::from_bytes(&[1; 32])
    }

    fn bob() -> SigningKey {
        SigningKey::from_bytes(&[2; 32])
    }

    fn key_bytes(key: &SigningKey) -> [u8; 32] {
        key.verifying_key().to_bytes()
    }

    fn bundle(payload: &[u8], name: &str, version: u64, author: &SigningKey) -> ProgramBundle {
        export(
            payload.to_vec(),
            ProgramSpec {
                name,
                version,
                licence: None,
                description: None,
                categories: &[],
            },
            author,
        )
        .unwrap()
    }

    fn pointer_for(bundle: &ProgramBundle) -> GalleryPkarrPointer {
        GalleryPkarrPointer::from_manifest(bundle.manifest())
    }

    /// An in-memory pkarr relay with the pkarr-worker's rules: a PUT is
    /// stored when strictly newer (409 otherwise), a GET serves the stored
    /// payload verbatim, and listed bases are down.
    #[derive(Default)]
    struct FakeRelays {
        stored: HashMap<String, (u64, Vec<u8>)>,
        down: Vec<String>,
    }

    impl BridgeHttp for FakeRelays {
        async fn request(&mut self, request: HttpRequest) -> anyhow::Result<HttpResponse> {
            let base = request.url.rsplit_once('/').unwrap().0.to_owned();
            if self.down.contains(&base) {
                anyhow::bail!("connection refused");
            }
            let respond = |status: u16, body: Vec<u8>| HttpResponse {
                status,
                headers: Vec::new(),
                body,
            };
            match request.method.as_str() {
                "PUT" => {
                    let item = PkarrItem::from_relay_payload(&request.body)?;
                    match self.stored.get(&request.url) {
                        Some((current, _)) if item.timestamp_us <= *current => {
                            Ok(respond(409, Vec::new()))
                        }
                        _ => {
                            self.stored
                                .insert(request.url, (item.timestamp_us, request.body));
                            Ok(respond(204, Vec::new()))
                        }
                    }
                }
                "GET" => Ok(match self.stored.get(&request.url) {
                    Some((_, body)) => respond(200, body.clone()),
                    None => respond(404, Vec::new()),
                }),
                other => panic!("unexpected method {other}"),
            }
        }
    }

    const RELAY_A: &str = "https://relay-a.example/pkarr";
    const RELAY_B: &str = "https://relay-b.example/pkarr";

    fn sinks() -> RelaySinks {
        RelaySinks::new([RELAY_A, RELAY_B]).unwrap()
    }

    #[test]
    fn pointers_sign_and_resolve_under_the_author_key() {
        let chess = pointer_for(&bundle(b"chess", "chess", 1, &alice()));
        let go = pointer_for(&bundle(b"go", "go", 3, &alice()));
        let item = sign_pointers(&alice(), &[chess.clone(), go.clone()], NOW_US).unwrap();
        let (timestamp, pointers) =
            resolve_pointers(&key_bytes(&alice()), &item.to_relay_payload()).unwrap();
        assert_eq!(timestamp, NOW_US);
        assert_eq!(pointers, vec![chess, go]);
    }

    #[tokio::test]
    async fn a_pkarr_published_pointer_fetches_and_verifies_like_the_registry_record() {
        // The acceptance check: the record a self-hosted registry lists
        // resolves from pkarr and verifies identically.
        let chess = bundle(b"chess v1", "chess", 1, &alice());
        let go = bundle(b"go v2", "go", 2, &alice());
        let mut registry = Registry::in_memory(AdmitAll);
        registry.publish(&chess.to_bytes()).unwrap();
        registry.publish(&go.to_bytes()).unwrap();

        let mut relays = FakeRelays {
            down: vec![RELAY_B.to_owned()],
            ..FakeRelays::default()
        };
        let report = publish_pointers(
            &sinks(),
            &mut relays,
            &alice(),
            &[pointer_for(&chess), pointer_for(&go)],
            NOW_US,
        )
        .await
        .unwrap();
        assert!(report.succeeded());

        relays.down.clear();
        let pointers = fetch_pointers(&sinks(), &mut relays, &key_bytes(&alice()))
            .await
            .unwrap();
        assert_eq!(pointers.len(), 2);
        for pointer in &pointers {
            let bytes = registry.fetch(&pointer.content_hash).unwrap();
            let fetched = ProgramBundle::decode(&bytes).unwrap();
            check_fetched(pointer, &fetched).unwrap();
            let (record, identity) = registry.get(&pointer.content_hash).unwrap();
            assert_eq!(record.verify().unwrap().identity(), pointer.identity());
            assert_eq!(identity, pointer.identity());
            assert_eq!(fetched.record(), record);
        }
    }

    #[tokio::test]
    async fn the_newest_packet_across_relays_wins() {
        let v1 = pointer_for(&bundle(b"chess v1", "chess", 1, &alice()));
        let v2 = pointer_for(&bundle(b"chess v2", "chess", 2, &alice()));
        let mut relays = FakeRelays::default();
        let old = sign_pointers(&alice(), &[v1], NOW_US).unwrap();
        let new = sign_pointers(&alice(), &[v2.clone()], NOW_US + 1).unwrap();
        let z32 = id_sync::announcement_pkarr::z32_encode(&key_bytes(&alice()));
        relays
            .stored
            .insert(format!("{RELAY_A}/{z32}"), (NOW_US, old.to_relay_payload()));
        relays.stored.insert(
            format!("{RELAY_B}/{z32}"),
            (NOW_US + 1, new.to_relay_payload()),
        );
        let pointers = fetch_pointers(&sinks(), &mut relays, &key_bytes(&alice()))
            .await
            .unwrap();
        assert_eq!(pointers, vec![v2]);
    }

    #[tokio::test]
    async fn a_forged_packet_is_skipped_and_nothing_valid_is_an_error() {
        let mut relays = FakeRelays::default();
        let z32 = id_sync::announcement_pkarr::z32_encode(&key_bytes(&alice()));
        // Bob signs a packet and a relay serves it under Alice's key.
        let bobs = sign_pointers(
            &bob(),
            &[pointer_for(&bundle(b"x", "x", 1, &bob()))],
            NOW_US,
        )
        .unwrap();
        relays.stored.insert(
            format!("{RELAY_A}/{z32}"),
            (NOW_US, bobs.to_relay_payload()),
        );
        let error = fetch_pointers(&sinks(), &mut relays, &key_bytes(&alice()))
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("does not verify"), "{error}");
        assert!(error.contains("HTTP 404"), "{error}");

        let empty = RelaySinks::new(Vec::<String>::new()).unwrap();
        let error = fetch_pointers(&empty, &mut relays, &key_bytes(&alice()))
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("no pkarr relay is configured"), "{error}");
    }

    #[test]
    fn signing_refuses_foreign_duplicate_empty_and_oversized_listings() {
        let mine = pointer_for(&bundle(b"chess", "chess", 1, &alice()));
        let theirs = pointer_for(&bundle(b"go", "go", 1, &bob()));
        let error = sign_pointers(&alice(), &[theirs], NOW_US)
            .unwrap_err()
            .to_string();
        assert!(error.contains("not the publishing key"), "{error}");
        let error = sign_pointers(&alice(), &[mine.clone(), mine.clone()], NOW_US)
            .unwrap_err()
            .to_string();
        assert!(error.contains("more than once"), "{error}");
        assert!(sign_pointers(&alice(), &[], NOW_US).is_err());
        let many: Vec<_> = (0..5)
            .map(|i| GalleryPkarrPointer {
                name: format!(
                    "{i}{}",
                    "n".repeat(crate::program_bundle::MAX_NAME_BYTES - 1)
                ),
                ..mine.clone()
            })
            .collect();
        let error = sign_pointers(&alice(), &many, NOW_US)
            .unwrap_err()
            .to_string();
        assert!(error.contains("limit"), "{error}");
    }

    #[test]
    fn resolving_refuses_tampering_foreign_pointers_and_empty_packets() {
        let mine = pointer_for(&bundle(b"chess", "chess", 1, &alice()));
        let item = sign_pointers(&alice(), &[mine], NOW_US).unwrap();
        let mut payload = item.to_relay_payload();
        let last = payload.len() - 1;
        payload[last] ^= 1;
        assert!(resolve_pointers(&key_bytes(&alice()), &payload).is_err());

        // A pointer naming Bob, signed into a packet under Alice's key.
        let theirs = pointer_for(&bundle(b"go", "go", 1, &bob()))
            .encode()
            .unwrap();
        let dns = txt_packet(
            &key_bytes(&alice()),
            GALLERY_RECORD_LABEL,
            &[theirs.as_bytes()],
        )
        .unwrap();
        let forged = PkarrItem::sign_packet(&alice(), NOW_US, dns).unwrap();
        let error = resolve_pointers(&key_bytes(&alice()), &forged.to_relay_payload())
            .unwrap_err()
            .to_string();
        assert!(error.contains("not the queried key"), "{error}");

        // A packet with only another family's records carries no pointer.
        let dns = txt_packet(&key_bytes(&alice()), "_ida", &[b"x".as_slice()]).unwrap();
        let other = PkarrItem::sign_packet(&alice(), NOW_US, dns).unwrap();
        let error = resolve_pointers(&key_bytes(&alice()), &other.to_relay_payload())
            .unwrap_err()
            .to_string();
        assert!(error.contains("no gallery pointer"), "{error}");
    }

    #[test]
    fn a_fetched_bundle_must_be_the_program_the_pointer_names() {
        let v1 = bundle(b"chess v1", "chess", 1, &alice());
        let v2 = bundle(b"chess v2", "chess", 2, &alice());
        check_fetched(&pointer_for(&v1), &v1).unwrap();
        let error = check_fetched(&pointer_for(&v1), &v2)
            .unwrap_err()
            .to_string();
        assert!(error.contains("the fetched bundle is"), "{error}");
    }
}
