//! The currency gateway's signed request envelope (#2319, design §4).
//!
//! A currency request is the existing [`crate::directory_auth`] envelope with an
//! op-specific body: `Signed { key, at, nonce, signature }` over
//! `id-request-v1\n{method}\n{target}\n{at}\n{nonce}\nsha256(body)`, where the
//! body is canonical CBOR domain-tagged [`REQUEST_DOMAIN`].
//!
//! The gateway pipeline is fixed (design §4.3):
//!
//! 1. verify the envelope (signature via `verify_strict` only, freshness,
//!    nonce replay) — [`verify_envelope`];
//! 2. decode the canonical body strictly — [`CurrencyRequest::from_body`];
//! 3. resolve device → account (design §4.1b) — [`DeviceResolver`];
//! 4. authorize against the registry (design §4.2) — [`CurrencyRegistry`];
//! 5. derive the deterministic idempotency ID (design §4.3) — [`idempotency_id`].
//!
//! Steps 1–5 are pure functions of the request plus a registry snapshot and are
//! testable without a ledger. Nothing here touches TigerBeetle.
//!
//! ## Design §9.3: idempotency ID vs TigerBeetle's monotonic-ID guidance
//!
//! TigerBeetle's reliable-submission pattern wants client-generated, persisted,
//! *monotonic* 128-bit IDs. This module derives the idempotency ID by hashing
//! `account_id ‖ op ‖ client_request_id`, which is stateless and stable but
//! **not** monotonic. The resolution, recorded here as design §9.3 directs:
//!
//! - The hash is the identity of the request; two submissions of the same
//!   request produce the same ID, so the cluster's dedup makes a retry a no-op.
//!   That is the property the gateway actually needs, and it needs no durable
//!   counter state, no leader, and no coordination between gateway restarts.
//! - A monotonic sequence would require durable counter state owned by the
//!   single logical writer (design §4.5). That state is a new availability and
//!   failover dependency, and it buys only insertion locality in the cluster's
//!   B-tree — an operational nicety, not a correctness property.
//! - Therefore **v1 uses the hash and leans on cluster dedup as the backstop**;
//!   a gateway monotonic sequence is *not* required. If insertion locality ever
//!   becomes a measured problem, add it as a separate, later change behind the
//!   same [`idempotency_id`] call site, keeping the hash as the identity.
//!
//! Note this ID is *different* from `user_data_128`, which carries the graph
//! object id and must stay a content-addressed hash (design §9.3).

use std::collections::BTreeMap;

use cbor_core::{DecodeOptions, Strictness, Value};
use serde::{Deserialize, Serialize};
use unicode_normalization::is_nfc;

use crate::directory_auth::{self, ReplayGuard, Signed};

/// The request protocol uses fixed-width byte strings for `u128` values.
/// CBOR's integer range ends at `u64`; its larger-integer representation uses
/// tags, while this protocol deliberately rejects tags. A 16-byte big-endian
/// byte string therefore preserves all `u128` values without tags or variable
/// encodings.
mod u128_bytes {
    use std::fmt;

    use serde::de::{Error, Visitor};
    use serde::{Deserializer, Serializer};

    pub fn serialize<S>(value: &u128, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_bytes(&value.to_be_bytes())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<u128, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct U128Visitor;

        impl Visitor<'_> for U128Visitor {
            type Value = u128;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a 16-byte big-endian u128")
            }

            fn visit_bytes<E>(self, bytes: &[u8]) -> Result<Self::Value, E>
            where
                E: Error,
            {
                let bytes: [u8; 16] = bytes
                    .try_into()
                    .map_err(|_| E::invalid_length(bytes.len(), &self))?;
                Ok(u128::from_be_bytes(bytes))
            }

            fn visit_byte_buf<E>(self, bytes: Vec<u8>) -> Result<Self::Value, E>
            where
                E: Error,
            {
                self.visit_bytes(&bytes)
            }
        }

        deserializer.deserialize_bytes(U128Visitor)
    }
}

/// The domain tag every currency request body carries, so a currency body
/// cannot be confused with another protocol's CBOR.
pub const REQUEST_DOMAIN: &str = "id.currency.request.v1";

/// The HTTP method the envelope signs for a currency request.
pub const REQUEST_METHOD: &str = "POST";

/// The HTTP target the envelope signs for a currency request.
pub const REQUEST_TARGET: &str = "/currency";

/// The most bytes a currency request body may hold before decoding.
pub const MAX_BODY_BYTES: usize = 16 * 1024;

/// Nesting limit for a request body's CBOR.
const RECURSION_LIMIT: u16 = 16;

/// Declared element limit for one array, map, or string.
const LENGTH_LIMIT: u64 = 512;

/// Byte budget for speculative pre-allocation.
const OOM_MITIGATION: usize = 64;

/// Why a currency request was refused. Every variant is a distinct, typed
/// outcome so a caller can tell a forged signature from a stale clock from a
/// non-minter, and never has to parse a message.
#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    /// The envelope failed verification: bad signature, bad key, or a nonce
    /// that is empty or too long. The signature is never accepted by plain
    /// `verify`; only `verify_strict` is used.
    #[error("signed request does not verify")]
    Envelope,
    /// The request's `at` is outside the 5-minute window.
    #[error("signed request is outside its time window")]
    Stale,
    /// The same key and nonce were already used inside the window.
    #[error("signed request was already used")]
    Replayed,
    /// The body was larger than [`MAX_BODY_BYTES`].
    #[error("currency request body is {0} bytes; maximum is {MAX_BODY_BYTES}")]
    Oversized(usize),
    /// The body was malformed CBOR.
    #[error("currency request CBOR decode failed: {0}")]
    Cbor(#[from] cbor_core::Error),
    /// The body was valid CBOR but not its canonical encoding.
    #[error("currency request body is not canonical CBOR")]
    NotCanonical,
    /// The body's domain tag was missing or wrong.
    #[error("currency request domain tag {0:?} is not {REQUEST_DOMAIN:?}")]
    Domain(String),
    /// The body did not match the closed schema: an unknown or missing field,
    /// or a field with the wrong shape.
    #[error("currency request body schema is invalid: {0}")]
    Schema(String),
    /// The signing device is not attached to any account, or was revoked.
    #[error("signing device is not attached to an account")]
    UnknownDevice,
    /// The request names a currency the registry does not know.
    #[error("currency is not registered")]
    UnknownCurrency,
    /// The currency is frozen or sunset.
    #[error("currency is not active")]
    InactiveCurrency,
    /// The account is not the issuer or a minter for this currency.
    #[error("account is not allowed to mint this currency")]
    NotAMinter,
    /// The account is not the currency's issuer.
    #[error("account is not the currency's issuer")]
    NotIssuer,
    /// The requested mint would exceed the currency's finite supply cap.
    #[error("currency supply cap would be exceeded")]
    CapExceeded,
    /// Amount-bearing operations cannot carry zero.
    #[error("currency amount must be positive")]
    ZeroAmount,
    /// The account does not control the account it is moving value from.
    #[error("account does not control the source account")]
    NotOwner,
}

/// The operations a currency request can carry (design §4.1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "op",
    content = "body",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum CurrencyOp {
    /// Register a new currency under the caller's account.
    RegisterCurrency(RegisterCurrency),
    /// Grant minting rights on a currency the caller issued.
    GrantMinter(GrantMinter),
    /// Mint supply of a currency.
    Issue(Issue),
    /// Move value between holders.
    Transfer(Transfer),
    /// Convert between two currencies (delegated to the rate engine, #2063).
    Convert(Convert),
    /// Read an account's balance.
    Balance(Balance),
    /// Read an account's history.
    History(History),
    /// Freeze a currency.
    Freeze(Freeze),
    /// Sunset a currency.
    Sunset(Sunset),
}

impl CurrencyOp {
    /// The stable op name used in the idempotency derivation and the registry
    /// checks. It is the CBOR tag string, so it never drifts from the wire.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::RegisterCurrency(_) => "register_currency",
            Self::GrantMinter(_) => "grant_minter",
            Self::Issue(_) => "issue",
            Self::Transfer(_) => "transfer",
            Self::Convert(_) => "convert",
            Self::Balance(_) => "balance",
            Self::History(_) => "history",
            Self::Freeze(_) => "freeze",
            Self::Sunset(_) => "sunset",
        }
    }
}

/// `register_currency`: the caller's account becomes the issuer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisterCurrency {
    /// Human-facing name; NFC and non-empty.
    pub name: String,
    /// Human-facing ticker; may be empty.
    pub symbol: String,
    /// Decimal places for minor units.
    pub scale: u8,
    /// Finite v1 supply cap, in minor units.
    #[serde(with = "u128_bytes")]
    pub cap: u128,
}

/// `grant_minter`: add an account to a currency's minter set.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrantMinter {
    /// The currency to grant on.
    pub currency: [u8; 32],
    /// The account that may mint.
    pub minter: String,
}

/// `issue`: mint supply.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Issue {
    /// The currency to mint.
    pub currency: [u8; 32],
    /// Minor-unit amount; must be positive.
    #[serde(with = "u128_bytes")]
    pub amount: u128,
    /// The account receiving the minted value.
    pub to: String,
}

/// `transfer`: move value between holders.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transfer {
    /// The currency being moved.
    pub currency: [u8; 32],
    /// The account the value leaves; the caller must control it.
    pub from: String,
    /// The account the value arrives at.
    pub to: String,
    /// Minor-unit amount; must be positive.
    #[serde(with = "u128_bytes")]
    pub amount: u128,
}

/// `convert`: a conversion leg (rate authorization lives in #2063).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Convert {
    /// The currency being sold.
    pub source: [u8; 32],
    /// The currency being bought.
    pub dest: [u8; 32],
    /// The account the value leaves; the caller must control it.
    pub from: String,
    /// Minor-unit amount; must be positive.
    #[serde(with = "u128_bytes")]
    pub amount: u128,
}

/// `balance`: read an account's balance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Balance {
    /// The currency to read.
    pub currency: [u8; 32],
    /// The account to read.
    pub account: String,
}

/// `history`: read an account's history.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct History {
    /// The currency to read.
    pub currency: [u8; 32],
    /// The account to read.
    pub account: String,
}

/// `freeze`: stop all movement of a currency.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Freeze {
    /// The currency to freeze.
    pub currency: [u8; 32],
}

/// `sunset`: retire a currency.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sunset {
    /// The currency to sunset.
    pub currency: [u8; 32],
}

/// The canonical body a currency request signs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CurrencyRequest {
    /// A client-chosen 128-bit value (or a client monotonic counter) that makes
    /// retries of the same logical request share an idempotency ID.
    #[serde(with = "u128_bytes")]
    pub client_request_id: u128,
    /// The operation and its op-specific body.
    pub op: CurrencyOp,
}

/// The wire form: the domain tag beside the request. It is a separate struct so
/// the tag is signed with the body and a body without it is refused.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    domain: String,
    request: CurrencyRequest,
}

impl CurrencyRequest {
    /// The canonical CBOR body, domain-tagged [`REQUEST_DOMAIN`].
    ///
    /// # Errors
    ///
    /// Fails if the request does not serialize to a canonical value.
    pub fn body(&self) -> Result<Vec<u8>, GatewayError> {
        let wire = Wire {
            domain: REQUEST_DOMAIN.to_owned(),
            request: self.clone(),
        };
        let value =
            Value::serialized(&wire).map_err(|error| GatewayError::Schema(error.to_string()))?;
        Ok(value.encode())
    }

    /// Decode a strict, canonical, domain-tagged body.
    ///
    /// # Errors
    ///
    /// Fails if the body is oversized, malformed, non-canonical, carries the
    /// wrong domain tag, or does not match the closed schema.
    pub fn from_body(bytes: &[u8]) -> Result<Self, GatewayError> {
        let value = canonical_value(bytes)?;
        let wire: Wire = value
            .deserialized()
            .map_err(|error| GatewayError::Schema(error.to_string()))?;
        if wire.domain != REQUEST_DOMAIN {
            return Err(GatewayError::Domain(wire.domain));
        }
        Ok(wire.request)
    }
}

/// Decode strictly, and accept the bytes only if they are the canonical
/// encoding of the value they hold. Mirrors `chain.rs`'s decoder.
fn canonical_value(bytes: &[u8]) -> Result<Value<'static>, GatewayError> {
    if bytes.len() > MAX_BODY_BYTES {
        return Err(GatewayError::Oversized(bytes.len()));
    }
    let value = DecodeOptions::new()
        .strictness(Strictness::STRICT)
        .recursion_limit(RECURSION_LIMIT)
        .length_limit(LENGTH_LIMIT)
        .oom_mitigation(OOM_MITIGATION)
        .decode_owned(bytes)?;
    if value.encode() != bytes {
        return Err(GatewayError::NotCanonical);
    }
    check_value(&value)?;
    Ok(value)
}

/// The value rules: text is NFC, and floats and tags are refused.
fn check_value(value: &Value<'_>) -> Result<(), GatewayError> {
    match value {
        Value::Unsigned(_) | Value::Negative(_) | Value::ByteString(_) => Ok(()),
        Value::TextString(text) if is_nfc(text) => Ok(()),
        Value::TextString(_) => Err(GatewayError::Schema("text is not NFC".to_owned())),
        Value::Array(items) => items.iter().try_for_each(check_value),
        Value::Map(map) => map.iter().try_for_each(|(key, val)| {
            check_value(key)?;
            check_value(val)
        }),
        Value::SimpleValue(simple) => match simple.to_u8() {
            20..=22 => Ok(()),
            other => Err(GatewayError::Schema(format!("simple value {other}"))),
        },
        Value::Float(_) => Err(GatewayError::Schema("a float".to_owned())),
        Value::Tag(number, _) => Err(GatewayError::Schema(format!("tag {number}"))),
    }
}

/// Sign a currency request as a client holding `key` would (design §4.1).
#[must_use]
pub fn sign_request(
    key: &ed25519_dalek::SigningKey,
    request: &CurrencyRequest,
    at: u64,
    nonce: &str,
) -> Result<(Signed, Vec<u8>), GatewayError> {
    let body = request.body()?;
    let signed =
        directory_auth::sign_request(key, REQUEST_METHOD, REQUEST_TARGET, &body, at, nonce);
    Ok((signed, body))
}

/// The verified request: who signed it, and what it asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verified {
    /// The signing device's public key, as lower-case hex.
    pub device: String,
    /// The decoded request.
    pub request: CurrencyRequest,
}

/// Verify a currency request's envelope and decode its body (steps 1–2).
///
/// The signature is checked with `verify_strict` only (via
/// [`directory_auth::verify_signed`]); the 5-minute window and the replay guard
/// are the same ones the rest of the ID server uses.
///
/// # Errors
///
/// Fails with [`GatewayError::Stale`] outside the window,
/// [`GatewayError::Replayed`] for a reused nonce, [`GatewayError::Envelope`] for
/// a bad signature, and the body errors from [`CurrencyRequest::from_body`].
pub fn verify_envelope(
    replays: &mut ReplayGuard,
    signed: &Signed,
    body: &[u8],
    now: u64,
) -> Result<Verified, GatewayError> {
    let device =
        directory_auth::verify_signed(replays, signed, REQUEST_METHOD, REQUEST_TARGET, body, now)
            .map_err(classify_verify_error)?;
    let request = CurrencyRequest::from_body(body)?;
    Ok(Verified { device, request })
}

/// Map the generic `directory_auth` refusal onto the gateway's typed surface so
/// a caller can tell a stale clock from a replay from a bad signature.
fn classify_verify_error(error: anyhow::Error) -> GatewayError {
    let text = error.to_string();
    if text.contains("outside its time window") {
        GatewayError::Stale
    } else if text.contains("already used") {
        GatewayError::Replayed
    } else {
        GatewayError::Envelope
    }
}

/// Resolve a signing device to the account it acts for (design §4.1b).
///
/// Issuance authority binds to the account key, not the device key, so the
/// gateway resolves before it authorizes. The account factory (#2318) is not
/// landed; this trait is the seam it will plug into, and fixture resolvers test
/// the pipeline without it.
pub trait DeviceResolver {
    /// The account `device` (lower-case hex) acts for, or `None` if the device
    /// is unknown or revoked.
    fn resolve_account(&self, device: &str) -> Option<String>;
}

/// A fixed device → account map, for tests and fixtures.
#[derive(Clone, Debug, Default)]
pub struct FixedDevices {
    accounts: BTreeMap<String, String>,
}

impl FixedDevices {
    /// An empty resolver.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Attach `device` to `account`.
    pub fn attach(&mut self, device: &str, account: &str) {
        self.accounts.insert(device.to_owned(), account.to_owned());
    }
}

impl DeviceResolver for FixedDevices {
    fn resolve_account(&self, device: &str) -> Option<String> {
        self.accounts.get(device).cloned()
    }
}

/// What the gateway needs to know about a currency to authorize a request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CurrencyRecord {
    /// The account that issued the currency.
    pub issuer_id: String,
    /// Whether the currency still accepts movement.
    pub active: bool,
    /// The finite v1 supply cap, in minor units.
    pub cap: u128,
    /// The current supply, in minor units.
    pub issued: u128,
}

/// The registry view the authorization step reads (design §4.2). The registry
/// itself is #2317; this trait is the seam, and fixture registries test the
/// pipeline without it.
pub trait CurrencyRegistry {
    /// The record for `currency`, or `None` if it is not registered.
    fn currency(&self, currency: &[u8; 32]) -> Option<CurrencyRecord>;
    /// Whether `account` may mint `currency`.
    fn is_minter(&self, currency: &[u8; 32], account: &str) -> bool;
    /// Whether `account` controls `holder` for this currency (ownership or a
    /// valid delegation). The default permits only an account reading or
    /// moving its own account; registries with delegation support override it.
    fn controls_account(&self, _currency: &[u8; 32], account: &str, holder: &str) -> bool {
        account == holder
    }
}

/// The outcome of the authorization step: the account that acted, and the
/// idempotency ID for the request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Authorized {
    /// The account the device acted for.
    pub account: String,
    /// The deterministic idempotency ID (design §4.3).
    pub idempotency: u128,
}

/// Resolve, authorize, and derive the idempotency ID (steps 3–5).
///
/// The registry, not the signature, decides authority: a valid signature from a
/// non-minter is refused (design §4.2).
///
/// # Errors
///
/// Fails with [`GatewayError::UnknownDevice`] when the device is unattached, and
/// with the authorization errors from [`authorize`].
pub fn admit(
    devices: &impl DeviceResolver,
    registry: &impl CurrencyRegistry,
    verified: &Verified,
) -> Result<Authorized, GatewayError> {
    let account = devices
        .resolve_account(&verified.device)
        .ok_or(GatewayError::UnknownDevice)?;
    authorize(registry, &account, &verified.request.op)?;
    let idempotency = idempotency_id(&account, &verified.request);
    Ok(Authorized {
        account,
        idempotency,
    })
}

/// The registry checks of design §4.2, for the ops this slice covers.
///
/// # Errors
///
/// Fails with [`GatewayError::UnknownCurrency`], [`GatewayError::InactiveCurrency`],
/// [`GatewayError::NotIssuer`], [`GatewayError::NotAMinter`], or
/// [`GatewayError::NotOwner`] as the operation requires.
pub fn authorize(
    registry: &impl CurrencyRegistry,
    account: &str,
    op: &CurrencyOp,
) -> Result<(), GatewayError> {
    match op {
        CurrencyOp::RegisterCurrency(_) => Ok(()),
        CurrencyOp::GrantMinter(grant) => {
            let record = active_currency(registry, &grant.currency)?;
            if record.issuer_id != account {
                return Err(GatewayError::NotIssuer);
            }
            Ok(())
        }
        CurrencyOp::Issue(issue) => {
            let record = active_currency(registry, &issue.currency)?;
            if issue.amount == 0 {
                return Err(GatewayError::ZeroAmount);
            }
            if record.issuer_id != account && !registry.is_minter(&issue.currency, account) {
                return Err(GatewayError::NotAMinter);
            }
            if record
                .issued
                .checked_add(issue.amount)
                .is_none_or(|projected| projected > record.cap)
            {
                return Err(GatewayError::CapExceeded);
            }
            Ok(())
        }
        CurrencyOp::Transfer(transfer) => {
            active_currency(registry, &transfer.currency)?;
            if transfer.amount == 0 {
                return Err(GatewayError::ZeroAmount);
            }
            if !registry.controls_account(&transfer.currency, account, &transfer.from) {
                return Err(GatewayError::NotOwner);
            }
            Ok(())
        }
        CurrencyOp::Convert(convert) => {
            active_currency(registry, &convert.source)?;
            if convert.amount == 0 {
                return Err(GatewayError::ZeroAmount);
            }
            if !registry.controls_account(&convert.source, account, &convert.from) {
                return Err(GatewayError::NotOwner);
            }
            Ok(())
        }
        CurrencyOp::Balance(balance) => {
            read_check(registry, &balance.currency, &balance.account, account)
        }
        CurrencyOp::History(history) => {
            read_check(registry, &history.currency, &history.account, account)
        }
        CurrencyOp::Freeze(freeze) => {
            let record = active_currency(registry, &freeze.currency)?;
            if record.issuer_id != account {
                return Err(GatewayError::NotIssuer);
            }
            Ok(())
        }
        CurrencyOp::Sunset(sunset) => {
            let record = active_currency(registry, &sunset.currency)?;
            if record.issuer_id != account {
                return Err(GatewayError::NotIssuer);
            }
            Ok(())
        }
    }
}

/// A registered, active currency.
fn active_currency(
    registry: &impl CurrencyRegistry,
    currency: &[u8; 32],
) -> Result<CurrencyRecord, GatewayError> {
    let record = registry
        .currency(currency)
        .ok_or(GatewayError::UnknownCurrency)?;
    if !record.active {
        return Err(GatewayError::InactiveCurrency);
    }
    Ok(record)
}

/// Reads are allowed on the caller's own account, and to the issuer.
fn read_check(
    registry: &impl CurrencyRegistry,
    currency: &[u8; 32],
    subject: &str,
    account: &str,
) -> Result<(), GatewayError> {
    let record = active_currency(registry, currency)?;
    if registry.controls_account(currency, account, subject) || record.issuer_id == account {
        Ok(())
    } else {
        Err(GatewayError::NotOwner)
    }
}

/// The deterministic idempotency ID (design §4.3):
/// `BLAKE3(account_id ‖ op_name ‖ client_request_id)[0..16]`.
///
/// Identical requests by the same account yield the same ID; a different
/// account, op, or client request id yields a different one. See the module
/// docs for why a hash rather than a monotonic sequence (design §9.3).
#[must_use]
pub fn idempotency_id(account: &str, request: &CurrencyRequest) -> u128 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(account.as_bytes());
    hasher.update(request.op.name().as_bytes());
    hasher.update(&request.client_request_id.to_le_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest.as_bytes()[..16]);
    u128::from_le_bytes(bytes)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::Duration;

    use ed25519_dalek::{Verifier as _, VerifyingKey};

    use super::*;
    use crate::test_forgeries::{IDENTITY_KEY, identity_forgery};

    fn key(byte: u8) -> ed25519_dalek::SigningKey {
        ed25519_dalek::SigningKey::from_bytes(&[byte; 32])
    }

    fn encode_hex(bytes: &[u8]) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut encoded = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
        encoded
    }

    fn request(op: CurrencyOp) -> CurrencyRequest {
        CurrencyRequest {
            client_request_id: 41,
            op,
        }
    }

    fn issue() -> CurrencyOp {
        CurrencyOp::Issue(Issue {
            currency: [9; 32],
            amount: 17,
            to: "holder-account".to_owned(),
        })
    }

    fn devices(device: &str, account: &str) -> FixedDevices {
        let mut devices = FixedDevices::new();
        devices.attach(device, account);
        devices
    }

    #[derive(Default)]
    struct Registry {
        currencies: BTreeMap<[u8; 32], CurrencyRecord>,
        minters: BTreeMap<([u8; 32], String), bool>,
    }

    impl CurrencyRegistry for Registry {
        fn currency(&self, id: &[u8; 32]) -> Option<CurrencyRecord> {
            self.currencies.get(id).cloned()
        }

        fn is_minter(&self, id: &[u8; 32], account: &str) -> bool {
            self.minters
                .get(&(*id, account.to_owned()))
                .copied()
                .unwrap_or(false)
        }
    }

    fn registry(issuer: &str, minters: &[&str]) -> Registry {
        let mut registry = Registry::default();
        registry.currencies.insert(
            [9; 32],
            CurrencyRecord {
                issuer_id: issuer.to_owned(),
                active: true,
                cap: 1_000,
                issued: 0,
            },
        );
        for minter in minters {
            registry
                .minters
                .insert(([9; 32], (*minter).to_owned()), true);
        }
        registry
    }

    #[test]
    fn signed_request_round_trips_and_resolves_device_before_account_authorization() {
        let client = key(7);
        let req = request(issue());
        let (signed, body) = sign_request(&client, &req, 10_000, "nonce-1").unwrap();
        let mut replays = ReplayGuard::after_boot(0, Duration::ZERO);
        let verified = verify_envelope(&mut replays, &signed, &body, 10_000).unwrap();
        assert_eq!(verified.request, req);
        let device = verified.device.clone();
        let admitted = admit(
            &devices(&device, "minter-account"),
            &registry("issuer-account", &["minter-account"]),
            &verified,
        )
        .unwrap();
        assert_eq!(admitted.account, "minter-account");
        assert_eq!(admitted.idempotency, idempotency_id("minter-account", &req));
    }

    #[test]
    fn tampered_body_is_rejected() {
        let client = key(7);
        let (signed, body) = sign_request(&client, &request(issue()), 10_000, "nonce-1").unwrap();
        let mut tampered = body.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 1;
        let mut replays = ReplayGuard::after_boot(0, Duration::ZERO);
        assert!(matches!(
            verify_envelope(&mut replays, &signed, &tampered, 10_000),
            Err(GatewayError::Envelope)
        ));
    }

    #[test]
    fn stale_request_is_rejected_with_a_typed_error() {
        let client = key(7);
        let (signed, body) = sign_request(&client, &request(issue()), 1, "nonce-1").unwrap();
        let mut replays = ReplayGuard::after_boot(0, Duration::ZERO);
        assert!(matches!(
            verify_envelope(&mut replays, &signed, &body, 1 + 300_001),
            Err(GatewayError::Stale)
        ));
    }

    #[test]
    fn replayed_nonce_is_rejected_with_a_typed_error() {
        let client = key(7);
        let (signed, body) = sign_request(&client, &request(issue()), 10_000, "nonce-1").unwrap();
        let mut replays = ReplayGuard::after_boot(0, Duration::ZERO);
        verify_envelope(&mut replays, &signed, &body, 10_000).unwrap();
        assert!(matches!(
            verify_envelope(&mut replays, &signed, &body, 10_000),
            Err(GatewayError::Replayed)
        ));
    }

    #[test]
    fn rejects_identity_forgery_that_plain_verify_accepts() {
        let forged = Signed {
            key: encode_hex(&IDENTITY_KEY),
            at: 10_000,
            nonce: "nonce-1".to_owned(),
            signature: encode_hex(&identity_forgery()),
        };
        let req = request(issue());
        let body = req.body().unwrap();
        let public = VerifyingKey::from_bytes(&IDENTITY_KEY).unwrap();
        let signature = ed25519_dalek::Signature::from_bytes(&identity_forgery());
        let message = directory_auth::signed_message(
            REQUEST_METHOD,
            REQUEST_TARGET,
            forged.at,
            &forged.nonce,
            &body,
        );
        assert!(public.verify(message.as_bytes(), &signature).is_ok());

        let mut replays = ReplayGuard::after_boot(0, Duration::ZERO);
        assert!(matches!(
            verify_envelope(&mut replays, &forged, &body, 10_000),
            Err(GatewayError::Envelope)
        ));
    }

    #[test]
    fn signature_from_non_minter_does_not_authorize() {
        let client = key(7);
        let (signed, body) = sign_request(&client, &request(issue()), 10_000, "nonce-1").unwrap();
        let mut replays = ReplayGuard::after_boot(0, Duration::ZERO);
        let verified = verify_envelope(&mut replays, &signed, &body, 10_000).unwrap();
        assert!(matches!(
            admit(
                &devices(&verified.device, "not-a-minter"),
                &registry("issuer-account", &[]),
                &verified,
            ),
            Err(GatewayError::NotAMinter)
        ));
    }

    #[test]
    fn request_body_is_canonical_domain_tagged_and_closed() {
        let req = request(issue());
        let body = req.body().unwrap();
        assert_eq!(CurrencyRequest::from_body(&body).unwrap(), req);
        assert!(matches!(
            CurrencyRequest::from_body(&[body.as_slice(), &[0]].concat()),
            Err(GatewayError::Cbor(_) | GatewayError::NotCanonical)
        ));

        #[derive(Serialize)]
        struct UnknownField<'a> {
            domain: &'a str,
            request: CurrencyRequest,
            extra: bool,
        }

        let unknown = Value::serialized(&UnknownField {
            domain: REQUEST_DOMAIN,
            request: req,
            extra: true,
        })
        .unwrap()
        .encode();
        assert!(matches!(
            CurrencyRequest::from_body(&unknown),
            Err(GatewayError::Schema(_))
        ));
    }

    #[test]
    fn all_u128_values_round_trip_as_fixed_width_cbor_bytes() {
        let req = CurrencyRequest {
            client_request_id: u128::MAX,
            op: CurrencyOp::Issue(Issue {
                currency: [9; 32],
                amount: u128::MAX,
                to: "holder-account".to_owned(),
            }),
        };
        let body = req.body().unwrap();
        assert_eq!(CurrencyRequest::from_body(&body).unwrap(), req);

        // The registration cap is a u128 on the same protocol: above
        // u64::MAX it round-trips only through the fixed-width adapter.
        let register = CurrencyRequest {
            client_request_id: 7,
            op: CurrencyOp::RegisterCurrency(RegisterCurrency {
                name: "Test Coin".to_owned(),
                symbol: "TST".to_owned(),
                scale: 2,
                cap: u128::MAX,
            }),
        };
        let body = register.body().unwrap();
        assert_eq!(CurrencyRequest::from_body(&body).unwrap(), register);
    }

    #[test]
    fn idempotency_ids_are_stable_and_request_sensitive() {
        let first = request(issue());
        let same = request(issue());
        let mut other_id = request(issue());
        other_id.client_request_id += 1;
        let other_op = request(CurrencyOp::Balance(Balance {
            currency: [9; 32],
            account: "holder-account".to_owned(),
        }));
        assert_eq!(
            idempotency_id("account", &first),
            idempotency_id("account", &same)
        );
        assert_ne!(
            idempotency_id("account", &first),
            idempotency_id("account", &other_id)
        );
        assert_ne!(
            idempotency_id("account", &first),
            idempotency_id("account", &other_op)
        );
        assert_ne!(
            idempotency_id("account", &first),
            idempotency_id("other-account", &first)
        );
    }
}
