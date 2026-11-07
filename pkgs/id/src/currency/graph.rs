//! Closed graph-native currency contracts (#5522, #2043).
//!
//! This module defines canonical-CBOR bodies and signing helpers for currency
//! definitions, issuance, transfers, and burns. It does not fold balances or
//! execute ledger-backed operations; callers must not treat these contracts as
//! settled balances until those layers are implemented.

use std::fmt;

use cbor_core::{DecodeOptions, Strictness, Value};
use p2panda_core::{Hash, SeqNum, SigningKey};
use serde::de::DeserializeOwned;
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use unicode_normalization::is_nfc;

use crate::chain::{self, ChainExt, Event};
use crate::currency::{IssuerId, issuer_id};

/// Event kind carrying a currency definition.
pub const DEFINE_KIND: &str = "token.define";
/// Event kind carrying issuance.
pub const ISSUE_KIND: &str = "token.issue";
/// Event kind carrying a holder transfer.
pub const TRANSFER_KIND: &str = "token.transfer";
/// Event kind carrying a holder burn.
pub const BURN_KIND: &str = "token.burn";

const MAX_BODY_BYTES: usize = 16 * 1024;
const RECURSION_LIMIT: u16 = 16;
const LENGTH_LIMIT: u64 = 512;
const OOM_MITIGATION: usize = 64;

mod key_bytes {
    use std::fmt;

    use serde::de::{Error, Visitor};
    use serde::{Deserializer, Serializer};

    use crate::currency::IssuerId;

    pub fn serialize<S>(key: &IssuerId, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_bytes(key)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<IssuerId, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct KeyVisitor;

        impl Visitor<'_> for KeyVisitor {
            type Value = IssuerId;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a 32-byte Ed25519 verifying key")
            }

            fn visit_bytes<E>(self, bytes: &[u8]) -> Result<Self::Value, E>
            where
                E: Error,
            {
                bytes
                    .try_into()
                    .map_err(|_| E::invalid_length(bytes.len(), &self))
            }

            fn visit_byte_buf<E>(self, bytes: Vec<u8>) -> Result<Self::Value, E>
            where
                E: Error,
            {
                self.visit_bytes(&bytes)
            }
        }

        deserializer.deserialize_bytes(KeyVisitor)
    }
}

/// A finite, immutable issuance cap. V1 deliberately has no unbounded policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupplyCap {
    /// Maximum counted supply, in minor units.
    pub capped: u128,
}

/// How a definition binds its balance authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CurrencyBinding {
    /// Offline, peer-to-peer value whose balance is a graph fold.
    Graph,
    /// Online value whose balance authority is the named TigerBeetle ledger.
    Ledger {
        /// TigerBeetle ledger identifier; operational, not semantic identity.
        ledger: u32,
    },
}

impl Serialize for CurrencyBinding {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Graph => serializer.serialize_str("graph"),
            Self::Ledger { ledger } => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("ledger", ledger)?;
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for CurrencyBinding {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Wire {
            Name(String),
            Ledger(LedgerWire),
        }

        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct LedgerWire {
            ledger: u32,
        }

        match Wire::deserialize(deserializer)? {
            Wire::Name(name) if name == "graph" => Ok(Self::Graph),
            Wire::Name(name) => Err(serde::de::Error::custom(format!(
                "unknown currency binding {name:?}"
            ))),
            Wire::Ledger(LedgerWire { ledger }) => Ok(Self::Ledger { ledger }),
        }
    }
}

/// An immutable currency definition. The canonical body is its identity
/// material; changing any field creates another currency.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CurrencyDefinition {
    /// Human-facing name; NFC and non-empty, but not an identity by itself.
    pub name: String,
    /// Human-facing ticker; it may be empty and is not an identity by itself.
    pub symbol: String,
    /// Decimal places for minor units, fixed for this definition.
    pub scale: u8,
    /// Finite v1 supply cap, encoded as `{"capped": <u128>}`.
    pub supply: SupplyCap,
    /// Selected balance binding; this module does not execute ledger bindings.
    pub binding: CurrencyBinding,
    /// Optional human description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memo: Option<String>,
}

impl CurrencyDefinition {
    /// Validate the fields whose constraints are fixed by the v1 design.
    pub fn validate(&self) -> Result<(), DefinitionError> {
        if self.name.is_empty() {
            return Err(DefinitionError::EmptyName);
        }
        if !is_nfc(&self.name) {
            return Err(DefinitionError::NameNotNfc);
        }
        if self.scale > 18 {
            return Err(DefinitionError::ScaleOutOfRange(self.scale));
        }
        Ok(())
    }

    /// Canonical CBOR body signed by the issuer.
    pub fn body(&self) -> Result<Vec<u8>, TokenError> {
        self.validate()?;
        Ok(canonical_body(self)?)
    }

    /// Decode a strict canonical-CBOR definition body and validate it.
    pub fn from_body(bytes: &[u8]) -> Result<Self, TokenError> {
        let definition: Self = decode_body(bytes)?;
        definition.validate()?;
        Ok(definition)
    }
}

/// The semantic currency identity: issuer key plus canonical definition hash.
/// Registry and ledger identifiers are deliberately not part of this pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CurrencyId {
    /// Issuer verifying key that signed the definition.
    issuer: IssuerId,
    /// BLAKE3 hash of the canonical-CBOR definition body.
    definition_hash: Hash,
}

impl CurrencyId {
    /// Derive the semantic pair from the issuer and a valid definition.
    pub fn from_definition(
        issuer: IssuerId,
        definition: &CurrencyDefinition,
    ) -> Result<Self, TokenError> {
        let body = definition.body()?;
        Ok(Self {
            issuer,
            definition_hash: Hash::digest(&body),
        })
    }

    /// The issuer component of the semantic identity.
    #[must_use]
    pub const fn issuer(&self) -> IssuerId {
        self.issuer
    }

    /// The canonical definition-body hash component of the semantic identity.
    #[must_use]
    pub const fn definition_hash(&self) -> Hash {
        self.definition_hash
    }

    /// Compact lookup/reference form. The pair above remains authoritative.
    #[must_use]
    pub fn compact_hash(&self) -> Hash {
        let mut bytes = Vec::with_capacity(64);
        bytes.extend_from_slice(&self.issuer);
        bytes.extend_from_slice(self.definition_hash.as_bytes());
        Hash::digest(&bytes)
    }
}

/// Issuance signed by the currency issuer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenIssue {
    /// Compact reference to the semantic currency identity.
    pub currency: Hash,
    /// Positive minor-unit amount.
    pub amount: u128,
    /// First holder receiving the issuance.
    #[serde(with = "key_bytes")]
    pub to: IssuerId,
    /// Optional human description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memo: Option<String>,
}

impl TokenIssue {
    /// Canonical CBOR body.
    pub fn body(&self) -> Result<Vec<u8>, TokenError> {
        validate_amount(self.amount)?;
        Ok(canonical_body(self)?)
    }

    /// Decode a strict canonical-CBOR body and validate it.
    pub fn from_body(bytes: &[u8]) -> Result<Self, TokenError> {
        let issue: Self = decode_body(bytes)?;
        validate_amount(issue.amount)?;
        Ok(issue)
    }
}

/// Holder-signed transfer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenTransfer {
    /// Compact reference to the semantic currency identity.
    pub currency: Hash,
    /// Positive minor-unit amount.
    pub amount: u128,
    /// Recipient verifying key.
    #[serde(with = "key_bytes")]
    pub to: IssuerId,
    /// Sender-chain sequence position the sender asserts is covered.
    pub seq_ref: u64,
    /// Optional human description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memo: Option<String>,
}

impl TokenTransfer {
    /// Canonical CBOR body.
    pub fn body(&self) -> Result<Vec<u8>, TokenError> {
        validate_amount(self.amount)?;
        Ok(canonical_body(self)?)
    }

    /// Decode a strict canonical-CBOR body and validate it.
    pub fn from_body(bytes: &[u8]) -> Result<Self, TokenError> {
        let transfer: Self = decode_body(bytes)?;
        validate_amount(transfer.amount)?;
        Ok(transfer)
    }
}

/// Holder-signed burn; only this kind reduces counted supply.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenBurn {
    /// Compact reference to the semantic currency identity.
    pub currency: Hash,
    /// Positive minor-unit amount.
    pub amount: u128,
    /// Holder-chain sequence position the holder asserts is covered.
    pub seq_ref: u64,
}

impl TokenBurn {
    /// Canonical CBOR body.
    pub fn body(&self) -> Result<Vec<u8>, TokenError> {
        validate_amount(self.amount)?;
        Ok(canonical_body(self)?)
    }

    /// Decode a strict canonical-CBOR body and validate it.
    pub fn from_body(bytes: &[u8]) -> Result<Self, TokenError> {
        let burn: Self = decode_body(bytes)?;
        validate_amount(burn.amount)?;
        Ok(burn)
    }
}

/// Why a definition is invalid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DefinitionError {
    /// Display name must contain at least one character.
    #[error("currency name is empty")]
    EmptyName,
    /// Display name must already be Unicode NFC.
    #[error("currency name is not Unicode NFC")]
    NameNotNfc,
    /// V1 supports scales from 0 through 18.
    #[error("currency scale {0} is outside 0..=18")]
    ScaleOutOfRange(u8),
}

/// A body was oversized, malformed, non-canonical, or had the wrong schema.
#[derive(Debug, thiserror::Error)]
pub enum BodyError {
    /// Payloads are bounded before CBOR decoding.
    #[error("currency body is {0} bytes; maximum is {MAX_BODY_BYTES}")]
    Oversized(usize),
    /// Decoder rejected the CBOR representation.
    #[error("currency CBOR decode failed: {0}")]
    Cbor(#[from] cbor_core::Error),
    /// Parsed value did not round-trip byte-for-byte through canonical encoding.
    #[error("currency body is not canonical CBOR")]
    NotCanonical,
    /// Canonical CBOR did not match the closed Rust schema.
    #[error("currency body schema is invalid: {0}")]
    Schema(String),
}

/// Failure while validating or signing a currency object.
#[derive(Debug, thiserror::Error)]
pub enum TokenError {
    /// Invalid currency definition.
    #[error(transparent)]
    Definition(#[from] DefinitionError),
    /// Invalid or non-canonical body.
    #[error(transparent)]
    Body(#[from] BodyError),
    /// Amount-bearing objects cannot carry zero.
    #[error("currency amount must be positive")]
    ZeroAmount,
    /// Issue signer must be the issuer in the semantic currency identity.
    #[error("issue signer does not match the currency issuer")]
    IssuerMismatch,
    /// Issue body must refer to the supplied semantic currency identity.
    #[error("issue body names a different currency")]
    CurrencyMismatch,
    /// Existing chained-event validation refused the event.
    #[error(transparent)]
    Chain(#[from] chain::ChainError),
}

/// Sign a definition and return its semantic identity with the event.
pub fn sign_definition(
    key: &SigningKey,
    seq_num: SeqNum,
    backlink: Option<Hash>,
    definition: &CurrencyDefinition,
    after: Vec<Hash>,
) -> Result<(CurrencyId, Event), TokenError> {
    let body = definition.body()?;
    let id = CurrencyId {
        issuer: issuer_id(key),
        definition_hash: Hash::digest(&body),
    };
    let event = sign_body(key, seq_num, backlink, &body, DEFINE_KIND, after)?;
    Ok((id, event))
}

/// Sign issuance, refusing an issuer or currency mismatch before emitting an event.
pub fn sign_issue(
    key: &SigningKey,
    seq_num: SeqNum,
    backlink: Option<Hash>,
    currency: &CurrencyId,
    issue: &TokenIssue,
    after: Vec<Hash>,
) -> Result<Event, TokenError> {
    if issuer_id(key) != currency.issuer {
        return Err(TokenError::IssuerMismatch);
    }
    if issue.currency != currency.compact_hash() {
        return Err(TokenError::CurrencyMismatch);
    }
    let body = issue.body()?;
    sign_body(key, seq_num, backlink, &body, ISSUE_KIND, after)
}

/// Sign a holder transfer. This only signs the assertion; it does not prove funds.
pub fn sign_transfer(
    key: &SigningKey,
    seq_num: SeqNum,
    backlink: Option<Hash>,
    transfer: &TokenTransfer,
    after: Vec<Hash>,
) -> Result<Event, TokenError> {
    let body = transfer.body()?;
    sign_body(key, seq_num, backlink, &body, TRANSFER_KIND, after)
}

/// Sign a holder burn. This only signs the assertion; supply effects require a fold.
pub fn sign_burn(
    key: &SigningKey,
    seq_num: SeqNum,
    backlink: Option<Hash>,
    burn: &TokenBurn,
    after: Vec<Hash>,
) -> Result<Event, TokenError> {
    let body = burn.body()?;
    sign_body(key, seq_num, backlink, &body, BURN_KIND, after)
}

fn sign_body(
    key: &SigningKey,
    seq_num: SeqNum,
    backlink: Option<Hash>,
    body: &[u8],
    kind: &str,
    after: Vec<Hash>,
) -> Result<Event, TokenError> {
    Ok(chain::sign(
        key,
        seq_num,
        backlink,
        Some(body),
        ChainExt {
            kind: kind.to_owned(),
            group: None,
            after,
            tag: None,
            document: None,
            removes: None,
            note: None,
            link: None,
            checkpoint: None,
            rotation: None,
        },
    )?)
}

fn validate_amount(amount: u128) -> Result<(), TokenError> {
    if amount == 0 {
        return Err(TokenError::ZeroAmount);
    }
    Ok(())
}

fn canonical_body<T: Serialize>(value: &T) -> Result<Vec<u8>, BodyError> {
    let value = Value::serialized(value).map_err(|error| BodyError::Schema(error.to_string()))?;
    Ok(value.encode())
}

fn decode_body<T>(bytes: &[u8]) -> Result<T, BodyError>
where
    T: DeserializeOwned + Serialize,
{
    if bytes.len() > MAX_BODY_BYTES {
        return Err(BodyError::Oversized(bytes.len()));
    }
    let value = DecodeOptions::new()
        .strictness(Strictness::STRICT)
        .recursion_limit(RECURSION_LIMIT)
        .length_limit(LENGTH_LIMIT)
        .oom_mitigation(OOM_MITIGATION)
        .decode_owned(bytes)?;
    if value.encode().as_slice() != bytes {
        return Err(BodyError::NotCanonical);
    }
    let decoded = value
        .deserialized()
        .map_err(|error| BodyError::Schema(error.to_string()))?;
    if canonical_body(&decoded)?.as_slice() != bytes {
        return Err(BodyError::NotCanonical);
    }
    Ok(decoded)
}

impl fmt::Display for CurrencyBinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Graph => f.write_str("graph"),
            Self::Ledger { ledger } => write!(f, "ledger:{ledger}"),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn definition(name: &str, scale: u8, cap: u128) -> CurrencyDefinition {
        CurrencyDefinition {
            name: name.to_owned(),
            symbol: "TST".to_owned(),
            scale,
            supply: SupplyCap { capped: cap },
            binding: CurrencyBinding::Graph,
            memo: None,
        }
    }

    fn issuer_key() -> SigningKey {
        SigningKey::from_bytes(&[7; 32])
    }

    fn other_key() -> SigningKey {
        SigningKey::from_bytes(&[9; 32])
    }

    #[test]
    fn definition_identity_is_issuer_and_canonical_body_hash() {
        let def = definition("Tokens", 2, 1_000);
        let issuer = issuer_id(&issuer_key());
        let first = CurrencyId::from_definition(issuer, &def).unwrap();
        let same = CurrencyId::from_definition(issuer, &def).unwrap();
        let other_issuer = CurrencyId::from_definition(issuer_id(&other_key()), &def).unwrap();
        let changed = CurrencyId::from_definition(issuer, &definition("Tokens", 3, 1_000)).unwrap();

        assert_eq!(first, same);
        assert_eq!(first.definition_hash(), Hash::digest(&def.body().unwrap()));
        assert_ne!(first, other_issuer);
        assert_ne!(first, changed);
        assert_ne!(first.compact_hash(), other_issuer.compact_hash());
    }

    #[test]
    fn definition_bodies_are_canonical_and_closed() {
        let def = definition("Tokens", 2, 1_000);
        let body = def.body().unwrap();
        assert_eq!(CurrencyDefinition::from_body(&body).unwrap(), def);

        let zero_cap = definition("Non-issuable", 0, 0);
        assert_eq!(
            CurrencyDefinition::from_body(&zero_cap.body().unwrap()).unwrap(),
            zero_cap
        );

        let mut value: serde_json::Value =
            serde_json::from_slice(&serde_json::to_vec(&def).unwrap()).unwrap();
        value["unexpected"] = serde_json::json!(true);
        let unknown_field = canonical_body(&value).unwrap();
        assert!(matches!(
            CurrencyDefinition::from_body(&unknown_field),
            Err(TokenError::Body(BodyError::Schema(_)))
        ));
    }

    #[test]
    fn definition_validation_enforces_nfc_scale_and_finite_cap() {
        assert_eq!(
            definition("", 0, 1).validate(),
            Err(DefinitionError::EmptyName)
        );
        assert_eq!(
            definition("e\u{301}", 0, 1).validate(),
            Err(DefinitionError::NameNotNfc)
        );
        assert_eq!(
            definition("Tokens", 19, 1).validate(),
            Err(DefinitionError::ScaleOutOfRange(19))
        );
        assert_eq!(definition("Tokens", 0, 0).validate(), Ok(()));
    }

    #[test]
    fn both_binding_shapes_round_trip() {
        for binding in [
            CurrencyBinding::Graph,
            CurrencyBinding::Ledger { ledger: 42 },
        ] {
            let mut def = definition("Tokens", 2, 1_000);
            def.binding = binding;
            let decoded = CurrencyDefinition::from_body(&def.body().unwrap()).unwrap();
            assert_eq!(decoded.binding, binding);
        }
    }

    #[test]
    fn issue_signing_binds_the_issuer_and_currency_pair() {
        let key = issuer_key();
        let (currency, event) =
            sign_definition(&key, 0, None, &definition("Tokens", 2, 1_000), vec![]).unwrap();
        let issue = TokenIssue {
            currency: currency.compact_hash(),
            amount: 25,
            to: issuer_id(&other_key()),
            memo: None,
        };
        let signed = sign_issue(&key, 1, Some(event.id), &currency, &issue, vec![]).unwrap();
        let decoded = chain::decode(&signed.bytes).unwrap();
        assert_eq!(decoded.header.extensions.kind, ISSUE_KIND);
        assert_eq!(issuer_id(&key), *decoded.header.verifying_key.as_bytes());

        assert!(matches!(
            sign_issue(&other_key(), 0, None, &currency, &issue, vec![]),
            Err(TokenError::IssuerMismatch)
        ));
        let mut wrong_currency = issue;
        wrong_currency.currency = Hash::digest(b"another currency");
        assert!(matches!(
            sign_issue(&key, 1, Some(event.id), &currency, &wrong_currency, vec![]),
            Err(TokenError::CurrencyMismatch)
        ));
    }

    #[test]
    fn all_value_objects_reject_zero_amount() {
        let currency = Hash::digest(b"currency");
        let issue = TokenIssue {
            currency,
            amount: 0,
            to: [1; 32],
            memo: None,
        };
        let transfer = TokenTransfer {
            currency,
            amount: 0,
            to: [1; 32],
            seq_ref: 0,
            memo: None,
        };
        let burn = TokenBurn {
            currency,
            amount: 0,
            seq_ref: 0,
        };
        assert!(matches!(issue.body(), Err(TokenError::ZeroAmount)));
        assert!(matches!(transfer.body(), Err(TokenError::ZeroAmount)));
        assert!(matches!(burn.body(), Err(TokenError::ZeroAmount)));
    }

    #[test]
    fn value_object_bodies_round_trip_with_byte_string_keys() {
        let currency = Hash::digest(b"currency");
        let recipient = issuer_id(&other_key());
        let issue = TokenIssue {
            currency,
            amount: 25,
            to: recipient,
            memo: Some("award".to_owned()),
        };
        let transfer = TokenTransfer {
            currency,
            amount: 10,
            to: recipient,
            seq_ref: 4,
            memo: None,
        };
        let burn = TokenBurn {
            currency,
            amount: 5,
            seq_ref: 5,
        };

        assert_eq!(
            TokenIssue::from_body(&issue.body().unwrap()).unwrap(),
            issue
        );
        assert_eq!(
            TokenTransfer::from_body(&transfer.body().unwrap()).unwrap(),
            transfer
        );
        assert_eq!(TokenBurn::from_body(&burn.body().unwrap()).unwrap(), burn);
    }

    proptest! {
        #[test]
        fn valid_definitions_round_trip_canonically(
            name in "[A-Za-z][A-Za-z0-9 ]{0,31}",
            scale in 0u8..=18,
            capped in 1u128..=u128::MAX,
        ) {
            let definition = definition(&name, scale, capped);
            let body = definition.body().expect("generated definition validates");
            let decoded = CurrencyDefinition::from_body(&body).expect("canonical body decodes");
            prop_assert_eq!(decoded, definition);
        }
    }
}
