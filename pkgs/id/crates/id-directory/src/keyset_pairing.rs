//! QR/invite-style pairing for keyset enrolment and revocation
//! (#2380 packet 2, keyset ADR §7 item 2).
//!
//! `id keyset enrol` today reads the new device's 32-byte key file, so
//! the secret must travel to the admin machine. This module is the
//! offline alternative the ADR names: the QR payload carries the
//! enrolment body, and the new device only ever reveals a
//! countersignature over it.
//!
//! - The admin mints an [`EnrolOffer`] from its keyset head and shows
//!   its [`EnrolOffer::qr_payload`] as a QR code (or pasted string).
//! - The new device scans it and returns an [`EnrolAccept`] carrying
//!   its public key and a proof-of-possession countersignature over
//!   the exact enrolment body.
//! - The admin feeds the accept to
//!   [`KeysetFile::complete_enrol`](crate::keyset_file::KeysetFile::complete_enrol),
//!   which refuses stale offers (the head moved on), wrong admins,
//!   and bad countersignatures, then appends the `DeviceEnrolled`
//!   record.
//!
//! Revocation needs no countersignature, but the device to revoke
//! still has to name itself without typing hex: [`DeviceQr`] is the
//! payload a device shows, which the admin scans and passes to
//! [`KeysetFile::revoke_scanned`](crate::keyset_file::KeysetFile::revoke_scanned)
//! alongside a reason.
//!
//! Payloads reuse the `id:…?…` invite discipline (`crate::invite`):
//! lowercase canonical hex, percent-encoded labels, and the same
//! 512-character paste budget as [`crate::invites::MAX_PASTE_CHARS`],
//! so an offer or accept survives any chat box a QR scan feeds.

use anyhow::{Context, Result, bail, ensure};
use ed25519_dalek::{Signer as _, SigningKey};
use sha2::{Digest, Sha256};

use crate::directory::hex_to_array;
use crate::keyset::{KeysetEntry, KeysetRecord, body_bytes, verify_signature, verifying_key};
use id_core::hex::hex_encode;

/// The scheme of an enrolment offer and its accept.
const ENROL_SCHEME: &str = "id:keyset-enrol?";
/// The scheme of the public-key payload a device shows for revocation.
const DEVICE_SCHEME: &str = "id:keyset-device?";
/// Longest label an offer may carry, matching the invite name budget.
const MAX_LABEL_CHARS: usize = 64;
/// Longest paste form, matching the invitation paste budget.
const MAX_PASTE_CHARS: usize = 512;

/// An admin's invitation to enrol one more device.
///
/// It names the exact chain position the enrolment will take
/// (`prev_hash`, `lamport`): an accept minted for an older head is
/// stale and [`KeysetFile::complete_enrol`](crate::keyset_file::KeysetFile::complete_enrol)
/// refuses it, so a captured QR cannot enrol twice across a moved head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnrolOffer {
    /// The identity being joined (genesis admin key, hex).
    pub identity_id: String,
    /// The admin key authoring the enrolment (hex).
    pub author: String,
    /// The head the enrolment extends (hex).
    pub prev_hash: String,
    /// The chain position the enrolment takes.
    pub lamport: u64,
    /// A name for the device, shown by `id keyset show`.
    pub label: String,
}

impl EnrolOffer {
    /// Mint the offer the admin shows.
    ///
    /// # Errors
    ///
    /// Fails if any key is not canonical hex, the lamport is zero, or
    /// the label is empty, too long, or carries control/format
    /// characters (the invite name rule, #1770).
    pub fn mint(
        identity_id: &str,
        author: &str,
        prev_hash: &str,
        lamport: u64,
        label: &str,
    ) -> Result<Self> {
        ensure!(lamport > 0, "an enrolment never takes lamport zero");
        Ok(Self {
            identity_id: checked_key("identity", identity_id)?,
            author: checked_key("author", author)?,
            prev_hash: checked_hash("prev", prev_hash)?,
            lamport,
            label: checked_label(label)?,
        })
    }

    /// The QR/paste payload the new device scans.
    ///
    /// # Errors
    ///
    /// Fails if the payload would exceed the paste budget (pick a
    /// shorter label).
    pub fn qr_payload(&self) -> Result<String> {
        let payload = format!(
            "{ENROL_SCHEME}identity={}&author={}&prev={}&lamport={}&label={}",
            self.identity_id,
            self.author,
            self.prev_hash,
            self.lamport,
            encode(&self.label),
        );
        ensure!(
            payload.chars().count() <= MAX_PASTE_CHARS,
            "the enrolment offer is over the {MAX_PASTE_CHARS}-character budget; use a shorter label"
        );
        Ok(payload)
    }

    /// Read an offer back from its payload.
    ///
    /// # Errors
    ///
    /// Fails if the scheme is wrong, a field is missing, duplicated,
    /// or invalid, or device/accept fields are present (that is an
    /// accept, not an offer).
    pub fn parse(text: &str) -> Result<Self> {
        let query = text
            .trim()
            .strip_prefix(ENROL_SCHEME)
            .context("a keyset enrolment offer starts with id:keyset-enrol?")?;
        let fields = split_query(query)?;
        ensure!(
            !fields.contains_key("device") && !fields.contains_key("countersignature"),
            "this is an enrolment accept, not an offer"
        );
        Self::mint(
            &field(&fields, "identity")?,
            &field(&fields, "author")?,
            &field(&fields, "prev")?,
            field(&fields, "lamport")?
                .parse::<u64>()
                .context("lamport must be a number")?,
            &decode(&field(&fields, "label")?)?,
        )
    }

    /// Countersign this offer with the new device's key, producing the
    /// accept the device shows back.
    ///
    /// # Errors
    ///
    /// Fails if the device key is not a valid Ed25519 public key.
    pub fn accept(&self, device: &SigningKey) -> Result<EnrolAccept> {
        let device_key = hex_encode(&device.verifying_key().to_bytes());
        let entry = KeysetEntry::DeviceEnrolled {
            device_key: device_key.clone(),
            label: self.label.clone(),
        };
        let body = body_bytes(&self.author, self.lamport, &self.prev_hash, &entry)?;
        EnrolAccept::checked(
            self.clone(),
            device_key,
            hex_encode(&device.sign(&body).to_bytes()),
        )
    }
}

/// The new device's answer: its public key plus a proof that it holds
/// the secret, over the exact body the admin will file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnrolAccept {
    /// The offer this answers.
    pub offer: EnrolOffer,
    /// The new device's public key (hex).
    pub device_key: String,
    /// Its signature over the enrolment body (hex).
    pub countersignature: String,
}

impl EnrolAccept {
    /// Check a reassembled accept (used by [`EnrolOffer::accept`] and
    /// [`EnrolAccept::parse`]).
    fn checked(offer: EnrolOffer, device_key: String, countersignature: String) -> Result<Self> {
        let device_key = checked_key("device", &device_key)?;
        let bytes = hex_to_bytes::<64>("countersignature", &countersignature)?;
        ensure!(
            hex_encode(&bytes) == countersignature,
            "countersignature is not canonically encoded"
        );
        let accept = Self {
            offer,
            device_key,
            countersignature,
        };
        accept.verify()?;
        Ok(accept)
    }

    /// The QR/paste payload the admin scans.
    ///
    /// # Errors
    ///
    /// Fails if the payload would exceed the paste budget (enrol with
    /// a shorter label instead).
    pub fn qr_payload(&self) -> Result<String> {
        let payload = format!(
            "{}&device={}&countersignature={}",
            self.offer
                .qr_payload()?
                .strip_prefix(ENROL_SCHEME)
                .unwrap_or_default(),
            self.device_key,
            self.countersignature,
        );
        let payload = format!("{ENROL_SCHEME}{payload}");
        ensure!(
            payload.chars().count() <= MAX_PASTE_CHARS,
            "the enrolment accept is over the {MAX_PASTE_CHARS}-character budget; enrol with a shorter label"
        );
        Ok(payload)
    }

    /// Read an accept back from its payload.
    ///
    /// # Errors
    ///
    /// Fails if the scheme is wrong, a field is missing, duplicated,
    /// or invalid, or the countersignature does not verify.
    pub fn parse(text: &str) -> Result<Self> {
        let query = text
            .trim()
            .strip_prefix(ENROL_SCHEME)
            .context("a keyset enrolment accept starts with id:keyset-enrol?")?;
        let fields = split_query(query)?;
        let offer = EnrolOffer::mint(
            &field(&fields, "identity")?,
            &field(&fields, "author")?,
            &field(&fields, "prev")?,
            field(&fields, "lamport")?
                .parse::<u64>()
                .context("lamport must be a number")?,
            &decode(&field(&fields, "label")?)?,
        )?;
        Self::checked(
            offer,
            field(&fields, "device")?,
            field(&fields, "countersignature")?,
        )
    }

    /// Check the countersignature against the offer's body.
    ///
    /// # Errors
    ///
    /// Fails if the device key is invalid or the countersignature does
    /// not sign this offer's enrolment body.
    pub fn verify(&self) -> Result<()> {
        let entry = KeysetEntry::DeviceEnrolled {
            device_key: self.device_key.clone(),
            label: self.offer.label.clone(),
        };
        let body = body_bytes(
            &self.offer.author,
            self.offer.lamport,
            &self.offer.prev_hash,
            &entry,
        )?;
        let key = verifying_key(&self.device_key).context("malformed device key")?;
        verify_signature(&key, &body, &self.countersignature).context("countersignature")?;
        Ok(())
    }

    /// Sign the author's side, producing the fileable record.
    ///
    /// # Errors
    ///
    /// Fails if `admin` is not the offer's author or a signature does
    /// not verify.
    pub fn record(&self, admin: &SigningKey) -> Result<KeysetRecord> {
        let author = hex_encode(&admin.verifying_key().to_bytes());
        ensure!(
            author == self.offer.author,
            "this accept is addressed to another admin key"
        );
        self.verify()?;
        let entry = KeysetEntry::DeviceEnrolled {
            device_key: self.device_key.clone(),
            label: self.offer.label.clone(),
        };
        let body = body_bytes(&author, self.offer.lamport, &self.offer.prev_hash, &entry)?;
        let hash = hex_encode(&Sha256::digest(&body));
        let record = KeysetRecord {
            author,
            lamport: self.offer.lamport,
            prev_hash: self.offer.prev_hash.clone(),
            hash,
            entry,
            signature: hex_encode(&admin.sign(&body).to_bytes()),
            countersignature: Some(self.countersignature.clone()),
        };
        record.verify()?;
        Ok(record)
    }
}

/// The public-key payload a device shows so the admin can revoke it
/// without typing hex: `id:keyset-device?device=<hex>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceQr {
    /// The device's public key (hex).
    pub device_key: String,
}

impl DeviceQr {
    /// The payload for `key` to show.
    #[must_use]
    pub fn for_key(key: &SigningKey) -> Self {
        Self {
            device_key: hex_encode(&key.verifying_key().to_bytes()),
        }
    }

    /// The QR/paste payload.
    #[must_use]
    pub fn qr_payload(&self) -> String {
        format!("{DEVICE_SCHEME}device={}", self.device_key)
    }

    /// Read the payload back after scanning.
    ///
    /// # Errors
    ///
    /// Fails if the scheme is wrong or the device key is missing,
    /// duplicated, or invalid.
    pub fn parse(text: &str) -> Result<Self> {
        let query = text
            .trim()
            .strip_prefix(DEVICE_SCHEME)
            .context("a keyset device payload starts with id:keyset-device?")?;
        let fields = split_query(query)?;
        Ok(Self {
            device_key: checked_key("device", &field(&fields, "device")?)?,
        })
    }
}

fn checked_key(field: &str, text: &str) -> Result<String> {
    // No case folding: payloads are machine-generated lowercase, and an
    // uppercase spelling is a second encoding of the same key (#1770
    // discipline, as for invite queries).
    let key = text.trim().to_owned();
    let bytes = hex_to_bytes::<32>(field, &key)?;
    ensure!(
        hex_encode(&bytes) == key,
        "{field} is not canonically encoded"
    );
    verifying_key(&key).with_context(|| format!("invalid {field}"))?;
    Ok(key)
}

fn checked_hash(field: &str, text: &str) -> Result<String> {
    let hash = text.trim().to_owned();
    let bytes = hex_to_bytes::<32>(field, &hash)?;
    ensure!(
        hex_encode(&bytes) == hash,
        "{field} is not canonically encoded"
    );
    Ok(hash)
}

fn checked_label(label: &str) -> Result<String> {
    ensure!(
        !label.chars().any(char::is_control),
        "a device label cannot contain control characters"
    );
    ensure!(
        !label.chars().any(id_core::is_format_char),
        "a device label cannot contain format characters"
    );
    let label = label.trim().to_owned();
    ensure!(!label.is_empty(), "a device needs a label");
    ensure!(
        label.chars().count() <= MAX_LABEL_CHARS,
        "a device label is at most {MAX_LABEL_CHARS} characters"
    );
    Ok(label)
}

fn hex_to_bytes<const N: usize>(field: &str, text: &str) -> Result<[u8; N]> {
    hex_to_array::<N>(text).with_context(|| format!("malformed {field}"))
}

fn split_query(query: &str) -> Result<std::collections::BTreeMap<String, String>> {
    let mut fields = std::collections::BTreeMap::new();
    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (name, value) = pair.split_once('=').context("a query pair needs =")?;
        ensure!(
            matches!(
                name,
                "identity"
                    | "author"
                    | "prev"
                    | "lamport"
                    | "label"
                    | "device"
                    | "countersignature"
            ),
            "unexpected field {name:?} in keyset pairing payload"
        );
        ensure!(
            fields.insert(name.to_owned(), value.to_owned()).is_none(),
            "duplicate {name} field in keyset pairing payload"
        );
    }
    Ok(fields)
}

fn field(fields: &std::collections::BTreeMap<String, String>, name: &str) -> Result<String> {
    fields
        .get(name)
        .cloned()
        .with_context(|| format!("a keyset pairing payload has no {name}"))
}

const HEX: &[u8; 16] = b"0123456789ABCDEF";

fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push('%');
            out.push(char::from(HEX[usize::from(byte >> 4)]));
            out.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
    }
    out
}

fn decode(text: &str) -> Result<String> {
    let raw = text.as_bytes();
    let mut bytes = Vec::with_capacity(raw.len());
    let mut index = 0;
    while let Some(&byte) = raw.get(index) {
        match byte {
            b'+' => {
                bytes.push(b' ');
                index += 1;
            }
            b'%' => {
                let digits = raw
                    .get(index + 1..index + 3)
                    .context("a % needs two hex digits")?;
                if !digits.iter().all(u8::is_ascii_hexdigit) {
                    bail!("a % needs two hex digits");
                }
                let value = std::str::from_utf8(digits)?;
                bytes.push(u8::from_str_radix(value, 16)?);
                index += 3;
            }
            other => {
                bytes.push(other);
                index += 1;
            }
        }
    }
    String::from_utf8(bytes).context("query value is not UTF-8")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::keyset::KeysetState;
    use crate::keyset_file::KeysetFile;

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn public(seed: u8) -> String {
        hex_encode(&key(seed).verifying_key().to_bytes())
    }

    fn migrated() -> (tempfile::TempDir, KeysetFile) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keyset.jsonl");
        let (file, _) = KeysetFile::open_or_migrate(&path, &key(1)).unwrap();
        (dir, file)
    }

    #[test]
    fn an_offer_round_trips_through_its_payload() {
        let (_dir, file) = migrated();
        let state = file.state().unwrap();
        let offer = EnrolOffer::mint(
            &state.identity_id,
            &public(1),
            &state.head_hash,
            1,
            "Ann's tablet & co.",
        )
        .unwrap();
        let payload = offer.qr_payload().unwrap();
        assert!(payload.starts_with("id:keyset-enrol?"));
        assert!(payload.chars().count() <= MAX_PASTE_CHARS);
        assert_eq!(EnrolOffer::parse(&payload).unwrap(), offer);
    }

    #[test]
    fn an_accept_round_trips_and_verifies() {
        let (_dir, file) = migrated();
        let state = file.state().unwrap();
        let offer =
            EnrolOffer::mint(&state.identity_id, &public(1), &state.head_hash, 1, "phone").unwrap();
        let accept = offer.accept(&key(2)).unwrap();
        let payload = accept.qr_payload().unwrap();
        assert!(payload.contains("&device="));
        assert!(payload.contains("&countersignature="));
        let back = EnrolAccept::parse(&payload).unwrap();
        assert_eq!(back, accept);
        back.verify().unwrap();
        // An offer string is not an accept, and an accept is not an offer.
        assert!(EnrolAccept::parse(&offer.qr_payload().unwrap()).is_err());
        assert!(EnrolOffer::parse(&payload).is_err());
    }

    #[test]
    fn pairing_enrols_a_second_device_through_the_file() {
        let (_dir, mut file) = migrated();
        let before = file.state().unwrap();
        let offer = file.enrol_offer(&key(1), "tablet").unwrap();
        assert_eq!(offer.prev_hash, before.head_hash);
        let accept = EnrolOffer::parse(&offer.qr_payload().unwrap())
            .unwrap()
            .accept(&key(2))
            .unwrap();
        let record = file
            .complete_enrol(
                &key(1),
                &EnrolAccept::parse(&accept.qr_payload().unwrap()).unwrap(),
            )
            .unwrap();
        assert_eq!(record.prev_hash, before.head_hash);
        let state = file.state().unwrap();
        assert!(state.is_device(&public(2)));
        assert_eq!(state.devices[&public(2)].label, "tablet");
        assert_eq!(state.records_folded, before.records_folded + 1);
    }

    #[test]
    fn a_stale_offer_cannot_enrol_twice() {
        let (_dir, mut file) = migrated();
        let offer = file.enrol_offer(&key(1), "tablet").unwrap();
        let accept = offer.accept(&key(2)).unwrap();
        file.complete_enrol(&key(1), &accept).unwrap();
        // Replaying the same accept targets a head that is gone.
        let error = file.complete_enrol(&key(1), &accept).unwrap_err();
        assert!(
            error.to_string().contains("stale"),
            "unexpected error: {error:#}"
        );
        // So does an offer minted before an intervening record.
        let early = file.enrol_offer(&key(1), "phone").unwrap();
        file.append(
            &key(1),
            KeysetEntry::IdpBindingBound {
                binding_id: "b".to_owned(),
            },
            None,
        )
        .unwrap();
        let late = early.accept(&key(3)).unwrap();
        assert!(file.complete_enrol(&key(1), &late).is_err());
    }

    #[test]
    fn a_tampered_accept_is_refused() {
        let (_dir, mut file) = migrated();
        let offer = file.enrol_offer(&key(1), "tablet").unwrap();
        let mut accept = offer.accept(&key(2)).unwrap();
        accept.offer.label = "renamed".to_owned();
        assert!(accept.verify().is_err());
        assert!(file.complete_enrol(&key(1), &accept).is_err());
        // Addressed to another admin: another key cannot complete it.
        let offer = file.enrol_offer(&key(1), "tablet").unwrap();
        let accept = offer.accept(&key(2)).unwrap();
        assert!(file.complete_enrol(&key(2), &accept).is_err());
    }

    #[test]
    fn revocation_by_scan_removes_the_device() {
        let (_dir, mut file) = migrated();
        let offer = file.enrol_offer(&key(1), "tablet").unwrap();
        file.complete_enrol(&key(1), &offer.accept(&key(2)).unwrap())
            .unwrap();
        assert!(file.state().unwrap().is_device(&public(2)));
        // The tablet shows its key; the admin scans it and revokes.
        let shown = DeviceQr::for_key(&key(2)).qr_payload();
        let scanned = DeviceQr::parse(&shown).unwrap();
        file.revoke_scanned(&key(1), &scanned, "lost").unwrap();
        let state = file.state().unwrap();
        assert!(!state.is_device(&public(2)));
        assert!(state.devices[&public(2)].revoked);
    }

    #[test]
    fn a_revoked_device_authorizes_nothing_afterwards() {
        let (_dir, mut file) = migrated();
        let offer = file.enrol_offer(&key(1), "tablet").unwrap();
        file.complete_enrol(&key(1), &offer.accept(&key(2)).unwrap())
            .unwrap();
        let head_before = file.state().unwrap().head_hash.clone();
        file.revoke_scanned(
            &key(1),
            &DeviceQr::parse(&DeviceQr::for_key(&key(2)).qr_payload()).unwrap(),
            "lost",
        )
        .unwrap();
        let revoked_head = file.state().unwrap().head_hash.clone();
        // The revoked tablet signs a binding against the revoked head.
        // The signature verifies, so the log holds it as evidence, but
        // the fold never advances onto it: it was not an admin's record.
        let rogue = KeysetRecord::sign(
            &key(2),
            revoked_head_records_plus_one(&file),
            revoked_head.clone(),
            KeysetEntry::IdpBindingBound {
                binding_id: "rogue".to_owned(),
            },
            None,
        )
        .unwrap();
        file.merge_records([rogue]).unwrap();
        let state: KeysetState = file.state().unwrap();
        assert_eq!(state.head_hash, revoked_head);
        assert_ne!(state.head_hash, head_before);
        assert!(!state.idp_bindings.contains_key("rogue"));
        // And the revoked admin-adjacent key cannot enrol either: the
        // fold judges authorisation at the record's position.
        let rogue_enrol = KeysetRecord::sign(
            &key(2),
            revoked_head_records_plus_one(&file),
            revoked_head.clone(),
            KeysetEntry::DeviceEnrolled {
                device_key: public(9),
                label: "rogue".to_owned(),
            },
            Some(&key(9)),
        )
        .unwrap();
        file.merge_records([rogue_enrol]).unwrap();
        let state = file.state().unwrap();
        assert!(!state.devices.contains_key(&public(9)));
    }

    fn revoked_head_records_plus_one(file: &KeysetFile) -> u64 {
        file.records()
            .map(|record| record.lamport)
            .max()
            .unwrap_or(0)
            + 1
    }

    #[test]
    fn revoking_an_unknown_device_is_refused_and_writes_nothing() {
        let (_dir, mut file) = migrated();
        let count = file.records().count();
        let error = file
            .revoke_scanned(
                &key(1),
                &DeviceQr::parse(&DeviceQr::for_key(&key(9)).qr_payload()).unwrap(),
                "unknown",
            )
            .unwrap_err();
        assert!(error.to_string().contains("refused"), "{error}");
        assert_eq!(file.records().count(), count);
    }

    #[test]
    fn only_an_admin_completes_or_revokes() {
        let (_dir, mut file) = migrated();
        let offer = file.enrol_offer(&key(1), "tablet").unwrap();
        let accept = offer.accept(&key(2)).unwrap();
        // Key 2 is not an admin of this identity.
        assert!(file.complete_enrol(&key(2), &accept).is_err());
        assert!(file.enrol_offer(&key(2), "rogue").is_err());
        file.complete_enrol(&key(1), &accept).unwrap();
        assert!(
            file.revoke_scanned(
                &key(2),
                &DeviceQr::parse(&DeviceQr::for_key(&key(2)).qr_payload()).unwrap(),
                "self",
            )
            .is_err()
        );
    }

    #[test]
    fn bad_payloads_are_refused() {
        let (_dir, file) = migrated();
        let state = file.state().unwrap();
        assert!(EnrolOffer::parse("id:invite?key=aa&node=bb&name=x").is_err());
        assert!(EnrolOffer::parse("id:keyset-enrol?identity=zz").is_err());
        assert!(
            EnrolOffer::mint(&state.identity_id, &public(1), &state.head_hash, 0, "x").is_err()
        );
        assert!(EnrolOffer::mint(&state.identity_id, "zz", &state.head_hash, 1, "x").is_err());
        assert!(EnrolOffer::mint(&state.identity_id, &public(1), &state.head_hash, 1, "").is_err());
        assert!(
            EnrolOffer::mint(&state.identity_id, &public(1), &state.head_hash, 1, "a\tb").is_err()
        );
        assert!(DeviceQr::parse("id:keyset-device?device=zz").is_err());
        assert!(DeviceQr::parse("id:keyset-device?").is_err());
        // A 64-character label is legal as a record but blows the paste
        // budget as a QR payload: the caller shortens it instead.
        let long = EnrolOffer::mint(
            &state.identity_id,
            &public(1),
            &state.head_hash,
            1,
            &"l".repeat(64),
        )
        .unwrap();
        assert!(long.qr_payload().is_ok());
        assert!(long.accept(&key(2)).unwrap().qr_payload().is_err());
    }

    #[test]
    fn re_enrolling_an_active_device_keeps_it_active() {
        let (_dir, mut file) = migrated();
        for label in ["tablet", "tablet renamed"] {
            let offer = file.enrol_offer(&key(1), label).unwrap();
            file.complete_enrol(&key(1), &offer.accept(&key(2)).unwrap())
                .unwrap();
        }
        let state = file.state().unwrap();
        assert!(state.is_device(&public(2)));
        assert_eq!(state.devices[&public(2)].label, "tablet renamed");
        // Merging the same records twice changes nothing.
        let records: Vec<KeysetRecord> = file.records().cloned().collect();
        assert_eq!(file.merge_records(records).unwrap(), 0);
        assert_eq!(file.state().unwrap(), state);
    }
}
