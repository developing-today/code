//! Pairing a second device key into an identity's keyset (#2380 packet 2).
//!
//! The admin offers an enrolment as a [`PairingOffer`]: the body the
//! record will carry, fixed at the log head it was made against. The offer
//! travels as text (a QR code, or an invite) to the new device, which
//! countersigns that body with its own key and returns the signature. The
//! admin then admits the record with `KeysetFile::admit_paired_enrolment`,
//! which folds it like any other change. An offer made against a head that
//! has since moved folds nowhere, so a stale or replayed offer is refused.

use anyhow::{Context, Result, ensure};
use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};

use crate::keyset::{KeysetEntry, countersign_entry};

/// An enrolment body awaiting the device's countersignature.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingOffer {
    /// The admin key that will sign the record.
    pub author: String,
    /// The record's lamport, one past the head it extends.
    pub lamport: u64,
    /// The head the record extends; a later head makes the offer stale.
    pub prev_hash: String,
    /// The enrolment itself.
    pub entry: KeysetEntry,
}

impl PairingOffer {
    /// The offer as the text a QR code carries.
    ///
    /// # Errors
    ///
    /// Fails if the offer cannot be encoded.
    pub fn to_payload(&self) -> Result<String> {
        serde_json::to_string(self).context("could not encode the pairing offer")
    }

    /// Reads an offer from the text a QR code carries.
    ///
    /// # Errors
    ///
    /// Fails if the text is not an offer.
    pub fn from_payload(text: &str) -> Result<Self> {
        serde_json::from_str(text).context("not a pairing offer")
    }

    /// The device's countersignature over this offer, given with the device
    /// key it names. Fails for any other key.
    ///
    /// # Errors
    ///
    /// Fails if `device` is not the key the offer enrols.
    pub fn countersign(&self, device: &SigningKey) -> Result<String> {
        countersign_entry(
            device,
            &self.author,
            self.lamport,
            &self.prev_hash,
            &self.entry,
        )
        .context("the device key does not match this pairing offer")
    }

    /// Fails unless the offer is an enrolment, which is the only entry a
    /// pairing payload may carry.
    ///
    /// # Errors
    ///
    /// Fails for any other entry.
    pub fn ensure_enrolment(&self) -> Result<()> {
        ensure!(
            matches!(self.entry, KeysetEntry::DeviceEnrolled { .. }),
            "a pairing offer enrols a device key"
        );
        Ok(())
    }
}
