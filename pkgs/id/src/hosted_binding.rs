//! Hosted account registry and ID binding ceremony (spec §§1 and 3).
//!
//! Implements slice 1 of the hosted provisioning + SSO→ID binding spec
//! (`doc/2026-10-30T22-45-00Z_design_hosted_provisioning_sso_binding.md`,
//! issue #2258; tracker #2852; this slice #3808), gated by the #2046
//! synthesis ruling (2026-11-03): client-side encryption is offered at
//! binding and defaults **ON** when no explicit choice is made; the rebind
//! waiting period is 30 days (enforced in slice 2, #3809, not here).
//!
//! ## Records (§1)
//!
//! - [`Account`]: one row per Google identity, keyed by the immutable
//!   [`Account::sub`] (`sub` is the primary key; an email address is never
//!   identity and is rejected as a `sub`). An account with no binding is an
//!   _empty hosted account_: it can sign in and start a ceremony, and can
//!   claim, read, and write nothing.
//! - [`BindingRecord`]: the only artefact connecting a `sub` to an Iroh ID.
//!   Its [`BindingRecord::binding_id`] is minted once, is immutable, and
//!   stays stable across key changes, so a future store named from it never
//!   moves when the key does. The record is valid only with **both**
//!   signatures: the key's proof of possession at ceremony time and the
//!   service attestation over every other field.
//!
//! ## Ceremony (§3)
//!
//! [`HostedBindingService`] runs the ceremony for both variants:
//!
//! - **(a) In-browser** (§3.3a): the web client holds the root key locally
//!   and performs request → sign → submit over the same session. Use
//!   [`HostedBindingService::complete_browser_ceremony`]. The key never
//!   leaves the device; the service sees only the public `iroh_id` and
//!   signatures. Callers must surface [`BROWSER_KEY_EXPORT_NOTICE`] on the
//!   ceremony's final screen.
//! - **(b) From own node** (§3.3b): the node holds the key; the browser
//!   session is only the authenticated channel. The browser mints a
//!   short-lived pairing code ([`HostedBindingService::create_pairing_code`])
//!   tied to the pending ceremony, the user enters it on their node, and the
//!   node exchanges it ([`HostedBindingService::exchange_pairing_code`]),
//!   signs locally, and submits the proof. Steps 4–5 are identical.
//!
//! ## Failure semantics (§3.4)
//!
//! [`CeremonyError`] encodes the table: expired challenges die without
//! state; a bad signature consumes the challenge (retry needs a fresh one,
//! repeated failures rate-limit issuance); replays are rejected; an
//! `iroh_id` bound to another `sub`, or a `sub` that already has a binding
//! in `active` **or** `rebind_pending`, is a conflict routed to the rebind
//! protocol (slice 2), never completed here; races resolve first-completed
//! wins. No failure path creates a partial binding, a store, or any claim
//! on the key.
//!
//! ## Device linkage (§1.3)
//!
//! Devices bind nothing individually and get no binding records. A device
//! grant references the bound `iroh_id`; revoking a device is a
//! groups-layer operation and never touches the binding record. See
//! [`device_authorized_for`].
//!
//! Key material is always client-held: this module has no API that accepts,
//! stores, or returns a private key. The service attestation key is the
//! service's **own** key, not a custodial user key.

use std::collections::{HashMap, HashSet};

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use rand::RngExt as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Domain prefix for the signed ceremony transcript (§3.2).
pub const TRANSCRIPT_DOMAIN: &str = "id-binding/v1";

/// Challenge lifetime in seconds (5 minutes, §3.2).
pub const CHALLENGE_TTL_SECS: u64 = 300;

/// Maximum challenge issuances per account per hour before issuance is
/// rate-limited (§3.4, "repeated failures rate-limit challenge issuance").
pub const MAX_CHALLENGES_PER_HOUR: usize = 30;

/// Maximum pairing-code creations per account per hour (§3.3b).
pub const MAX_PAIRINGS_PER_HOUR: usize = 10;

/// Client-side encryption default at binding: **ON** when the user makes no
/// explicit choice (the #2046 ruling of 2026-11-03). Default-off is
/// acceptable only for accounts that chose it with the plain-language
/// operator-readability disclosure.
pub const CSE_DEFAULT_ON: bool = true;

/// Disclosure text for the client-side-encryption offer at binding time
/// (§7, per #2045): hosted storage is operator-readable at rest unless the
/// option is on.
pub const CSE_OFFER_DISCLOSURE: &str = "Hosted storage is operator-readable at rest unless \
    client-side encryption is on. Turning it on keeps payload and blob contents private to \
    your devices, but hosted search, previews, and web-only access from devices without \
    your keys degrade to what metadata alone allows.";

/// Final-screen notice for the in-browser variant (§3.3a): a binding whose
/// only key copy dies with a browser profile is a support incident scheduled
/// in advance.
pub const BROWSER_KEY_EXPORT_NOTICE: &str =
    "Export your Iroh root key now and keep the copy somewhere safe. If this browser \
    profile is lost without an exported copy, the binding cannot be recovered except \
    through the delayed-rebind wait.";

// ---------------------------------------------------------------------------
// Small codecs
// ---------------------------------------------------------------------------

/// Lowercase hex of bytes (keys, ids, hashes).
#[must_use]
pub fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

/// Decode exactly `expected` bytes from lowercase or uppercase hex.
fn hex_decode(text: &str, expected: usize) -> Option<Vec<u8>> {
    if text.len() != expected * 2 {
        return None;
    }
    let mut out = Vec::with_capacity(expected);
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let hi = hex_val(bytes[index])?;
        let lo = hex_val(bytes[index + 1])?;
        out.push((hi << 4) | lo);
        index += 2;
    }
    Some(out)
}

fn hex_val(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Seconds since the Unix epoch, 0 if the clock is before it.
#[must_use]
pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// Format Unix seconds as RFC 3339 UTC (`YYYY-MM-DDTHH:MM:SSZ`).
#[must_use]
pub fn unix_secs_to_rfc3339(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = (secs % 86_400) as i64;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3_600,
        (rem % 3_600) / 60,
        rem % 60
    )
}

/// Parse strict RFC 3339 UTC (`YYYY-MM-DDTHH:MM:SSZ`) back to Unix seconds.
#[must_use]
pub fn parse_rfc3339(text: &str) -> Option<u64> {
    let bytes = text.as_bytes();
    if bytes.len() != 20 {
        return None;
    }
    if bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'Z'
    {
        return None;
    }
    let num = |from: usize, to: usize| -> Option<i64> {
        let mut value: i64 = 0;
        for byte in &bytes[from..to] {
            if !byte.is_ascii_digit() {
                return None;
            }
            value = value * 10 + i64::from(byte - b'0');
        }
        Some(value)
    };
    let year = num(0, 4)?;
    let month = num(5, 7)?;
    let day = num(8, 10)?;
    let hour = num(11, 13)?;
    let minute = num(14, 16)?;
    let second = num(17, 19)?;
    if !(1..=12).contains(&month) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let leap = is_leap_year(year);
    let month_len: i64 = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if leap {
                29
            } else {
                28
            }
        }
    };
    if day < 1 || day > month_len {
        return None;
    }
    let days = days_from_civil(year, month, day);
    if days < 0 {
        return None;
    }
    Some(days as u64 * 86_400 + hour as u64 * 3_600 + minute as u64 * 60 + second as u64)
}

fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

// Howard Hinnant's civil-date algorithms (proleptic Gregorian calendar).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (if month <= 2 { year + 1 } else { year }, month, day)
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

// ---------------------------------------------------------------------------
// Records (§1)
// ---------------------------------------------------------------------------

/// An Iroh root public key: the 32 bytes of the account identity.
pub type IrohId = [u8; 32];

/// Account record (§1.1): one row per Google identity in service-operated
/// registry storage. The registry holds no user content.
///
/// The `sub` (map key) is the primary key. `email_contact` is contact
/// metadata for notifications only — never a key, never an identity claim.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    /// Service-minted opaque ID, assigned at first sign-in, immutable.
    pub account_id: String,
    /// First sign-in time (RFC 3339).
    pub created_at: String,
    /// Contact metadata from the Google profile, used only for
    /// notifications (notably rebind notices). Never a key.
    pub email_contact: Option<String>,
    /// Null until first binding (§3); then the stable binding record ID.
    pub binding_id: Option<String>,
    /// Tier identifier. The numbers behind each tier are a #2046 parameter
    /// and are not decided here.
    pub tier: String,
    /// Latest metering rollup from the user's store. Populated by the
    /// provisioning/metering slice (#3811); `None` until then.
    pub meter_rollup: Option<serde_json::Value>,
}

/// Binding status (§1.2). This slice creates `active` bindings; the
/// `rebind_pending` and `superseded` states belong to the delayed-rebind
/// slice (#3809) and appear here so conflict checks reserve the account
/// exactly as specified.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BindingStatus {
    /// The current binding for the account.
    Active,
    /// A delayed rebind is waiting; the account is still reserved (§1.2).
    RebindPending,
    /// Replaced by a newer key; kept for the verifiable history chain.
    Superseded,
}

/// One append-only history entry: a superseded key is never deleted (§1.2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// The previous Iroh root public key (64 hex characters).
    pub iroh_id: String,
    /// When it was superseded (RFC 3339).
    pub superseded_at: String,
    /// How it was replaced: `old-key-signature` or `delayed-rebind`.
    pub via: String,
    /// The §4.4 record for delayed rebinds; absent for fast-path rebinds.
    pub rebind_record_id: Option<String>,
}

/// The complete challenge transcript persisted in the binding record (§1.2),
/// so the key signature verifies offline with no hosted-service access.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChallengeTranscript {
    /// The ceremony challenge this binding was proven with.
    pub challenge_id: String,
    /// The Google `sub` the challenge was issued to.
    pub sub: String,
    /// The Iroh root public key (64 hex characters).
    pub iroh_id: String,
    /// The service identifier.
    pub service_id: String,
    /// The challenge's 32 random bytes, base64.
    pub nonce: String,
    /// Challenge issuance (RFC 3339).
    pub issued_at: String,
    /// Challenge expiry (RFC 3339).
    pub expires_at: String,
}

/// Binding record (§1.2): the only artefact connecting a `sub` to an Iroh
/// ID. `binding_id` is minted once and never changes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BindingRecord {
    /// Opaque ID, minted at first binding, immutable across rebinds.
    pub binding_id: String,
    /// Record version.
    pub version: u32,
    /// The Google `sub` this binding belongs to.
    pub sub: String,
    /// The account record ID.
    pub account_id: String,
    /// The current Iroh root public key (64 hex characters).
    pub iroh_id: String,
    /// Binding time (RFC 3339).
    pub bound_at: String,
    /// The ceremony challenge this binding was proven with.
    pub challenge_id: String,
    /// Full transcript, so verification needs no hosted-service access.
    pub challenge_transcript: ChallengeTranscript,
    /// Iroh root key signature over the canonical ceremony transcript.
    pub key_signature: String,
    /// Service key signature over SHA-256 of the canonical record without
    /// this field (regenerated on every mutation).
    pub service_attestation: String,
    /// Binding status.
    pub status: BindingStatus,
    /// Whether client-side encryption was chosen at binding (§7).
    pub client_side_encryption: bool,
    /// Append-only superseded-key history.
    pub history: Vec<HistoryEntry>,
}

/// Challenge minted in ceremony step 1 (§3.2): 5-minute lifetime, single-use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Challenge {
    /// Challenge identifier.
    pub challenge_id: String,
    /// The authenticated session's `sub`.
    pub sub: String,
    /// The claimed Iroh root public key.
    pub iroh_id: IrohId,
    /// The service identifier.
    pub service_id: String,
    /// 32 random bytes.
    pub nonce: [u8; 32],
    /// Issuance time (Unix seconds).
    pub issued_at: u64,
    /// Expiry time (Unix seconds).
    pub expires_at: u64,
}

/// Short-lived pairing code for the own-node variant (§3.3b): single-use,
/// expires with the challenge, rate-limited per account.
#[derive(Clone, Debug, PartialEq, Eq)]
struct PairingCode {
    /// The pending ceremony's challenge.
    challenge_id: String,
    /// Expiry (Unix seconds, equal to the challenge's).
    expires_at: u64,
}

// ---------------------------------------------------------------------------
// Errors (§3.4)
// ---------------------------------------------------------------------------

/// Ceremony and registry failures. The variants follow the §3.4 table: no
/// failure path creates a partial binding, a store, or any claim on a key.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CeremonyError {
    /// The `sub` is empty or shaped like an email address. Email is contact
    /// metadata, never identity (§1.1).
    #[error("invalid sub: must be a non-empty opaque Google subject, never an email address")]
    InvalidSub,
    /// The tier label is empty.
    #[error("invalid tier: must be a non-empty tier identifier")]
    InvalidTier,
    /// No live challenge with that ID (unknown or already expired away).
    #[error("unknown challenge: request a fresh one")]
    UnknownChallenge,
    /// The challenge expired. State change: none; the challenge is dead.
    /// Retry with a fresh challenge.
    #[error("challenge expired: request a fresh challenge")]
    ChallengeExpired,
    /// The challenge was already presented (success or failure). Replays
    /// are rejected and logged; retry with a fresh challenge.
    #[error("challenge already consumed: request a fresh challenge")]
    ChallengeReplay,
    /// The presenting session's `sub` is not the challenge's `sub`.
    #[error("session mismatch: the presenting session is not the challenged account")]
    SessionMismatch,
    /// Bad signature. The challenge is consumed; retry with a fresh
    /// challenge. Repeated failures rate-limit issuance per account.
    #[error("bad signature: the proof does not verify against the claimed key")]
    BadSignature,
    /// This `iroh_id` is already bound to another `sub`. Not retryable as
    /// a fresh binding; the rebind path (slice 2) applies.
    #[error("key already bound to another account: use the rebind protocol")]
    IrohIdBoundElsewhere,
    /// This `sub` already has an active binding for a different key. Not
    /// completable here; the rebind protocol (slice 2) applies.
    #[error("account already bound: use the rebind protocol")]
    SubAlreadyBound,
    /// This `sub` has a binding in `rebind_pending`. Only old-key
    /// cancel/approve or waiting out the delay (slice 2) applies — a fresh
    /// ceremony never bypasses it.
    #[error("rebind already pending: resolve it before any fresh ceremony")]
    RebindPending,
    /// Too many challenge or pairing requests; back off and retry later.
    #[error("rate limited: too many ceremony requests for this account")]
    RateLimited,
    /// Malformed signature bytes on submit.
    #[error("malformed signature: expected 64 bytes")]
    MalformedSignature,
    /// The service could not encode the canonical record. No binding is stored.
    #[error("could not encode the canonical binding record")]
    RecordEncoding,
}

// ---------------------------------------------------------------------------
// Service
// ---------------------------------------------------------------------------

/// Runs the account registry and the binding ceremony (§§1, 3).
///
/// Holds service-operated registry state only — accounts, bindings,
/// challenges — and the service's **own** attestation key. It never sees a
/// user private key: callers pass public keys and signatures.
///
/// `SigningKey` intentionally has no `Debug`, so this service does not
/// implement it either: registry state must never be dumped into logs.
#[allow(missing_debug_implementations)]
pub struct HostedBindingService {
    service_id: String,
    service_key: SigningKey,
    accounts: HashMap<String, Account>,
    bindings: HashMap<String, BindingRecord>,
    binding_by_sub: HashMap<String, String>,
    binding_by_iroh: HashMap<String, String>,
    challenges: HashMap<String, Challenge>,
    consumed_challenges: HashSet<String>,
    pairing_codes: HashMap<String, PairingCode>,
    challenge_issued_at: HashMap<String, Vec<u64>>,
    pairing_issued_at: HashMap<String, Vec<u64>>,
    now_override: Option<u64>,
}

impl HostedBindingService {
    /// Create a service with a fresh random attestation key.
    #[must_use]
    pub fn new(service_id: impl Into<String>) -> Self {
        let mut rng = rand::rng();
        let mut seed = [0u8; 32];
        rng.fill(&mut seed);
        Self::with_seed(service_id, seed)
    }

    /// Create a service with a fixed attestation key (tests, fixtures).
    #[must_use]
    pub fn with_seed(service_id: impl Into<String>, seed: [u8; 32]) -> Self {
        Self {
            service_id: service_id.into(),
            service_key: SigningKey::from_bytes(&seed),
            accounts: HashMap::new(),
            bindings: HashMap::new(),
            binding_by_sub: HashMap::new(),
            binding_by_iroh: HashMap::new(),
            challenges: HashMap::new(),
            consumed_challenges: HashSet::new(),
            pairing_codes: HashMap::new(),
            challenge_issued_at: HashMap::new(),
            pairing_issued_at: HashMap::new(),
            now_override: None,
        }
    }

    /// Override the clock (tests). `None` restores the system clock.
    pub fn set_now_override(&mut self, now: Option<u64>) {
        self.now_override = now;
    }

    fn now(&self) -> u64 {
        self.now_override.unwrap_or_else(now_secs)
    }

    /// The service identifier challenges and transcripts name.
    #[must_use]
    pub fn service_id(&self) -> &str {
        &self.service_id
    }

    /// The service attestation public key (32 bytes).
    #[must_use]
    pub fn service_public_key(&self) -> [u8; 32] {
        self.service_key.verifying_key().to_bytes()
    }

    // -- accounts (§1.1) ----------------------------------------------------

    /// First sign-in: create the account row, or return the existing row
    /// (refreshing contact metadata) on repeat sign-in. Creates nothing
    /// else: an account with no binding claims, reads, and writes nothing.
    ///
    /// # Errors
    ///
    /// [`CeremonyError::InvalidSub`] if `sub` is empty or shaped like an
    /// email address; [`CeremonyError::InvalidTier`] if `tier` is empty.
    pub fn signup_account(
        &mut self,
        sub: &str,
        email_contact: Option<&str>,
        tier: &str,
    ) -> Result<Account, CeremonyError> {
        if sub.is_empty() || sub.contains('@') {
            return Err(CeremonyError::InvalidSub);
        }
        if tier.is_empty() {
            return Err(CeremonyError::InvalidTier);
        }
        if let Some(existing) = self.accounts.get_mut(sub) {
            existing.email_contact = email_contact.map(str::to_owned);
            return Ok(existing.clone());
        }
        let account = Account {
            account_id: random_hex_id(),
            created_at: unix_secs_to_rfc3339(self.now()),
            email_contact: email_contact.map(str::to_owned),
            binding_id: None,
            tier: tier.to_owned(),
            meter_rollup: None,
        };
        self.accounts.insert(sub.to_owned(), account.clone());
        Ok(account)
    }

    /// Look up an account by `sub`. There is deliberately no lookup by
    /// email: email is contact metadata, never a key (§1.1).
    #[must_use]
    pub fn account(&self, sub: &str) -> Option<&Account> {
        self.accounts.get(sub)
    }

    // -- ceremony (§3.2) ----------------------------------------------------

    /// Step 1: mint a challenge for `{iroh_id}` over the authenticated
    /// session for `session_sub`. The client never asserts a `sub`: the
    /// service binds the challenge to the session it issued it on.
    ///
    /// # Errors
    ///
    /// [`CeremonyError::RateLimited`] after too many issuances for the
    /// account in the past hour.
    pub fn request_challenge(
        &mut self,
        session_sub: &str,
        iroh_id: &IrohId,
    ) -> Result<Challenge, CeremonyError> {
        let now = self.now();
        let log = self
            .challenge_issued_at
            .entry(session_sub.to_owned())
            .or_default();
        prune_before(log, now.saturating_sub(3_600));
        if log.len() >= MAX_CHALLENGES_PER_HOUR {
            return Err(CeremonyError::RateLimited);
        }
        log.push(now);

        let mut rng = rand::rng();
        let mut nonce = [0u8; 32];
        rng.fill(&mut nonce);
        let challenge = Challenge {
            challenge_id: random_hex_id(),
            sub: session_sub.to_owned(),
            iroh_id: *iroh_id,
            service_id: self.service_id.clone(),
            nonce,
            issued_at: now,
            expires_at: now + CHALLENGE_TTL_SECS,
        };
        self.challenges
            .insert(challenge.challenge_id.clone(), challenge.clone());
        Ok(challenge)
    }

    /// The canonical transcript bytes the Iroh root key signs (step 2):
    /// `id-binding/v1 || challenge_id || sub || iroh_id || nonce ||
    /// expires_at`, each field length-prefixed (u32 big-endian), UTF-8.
    /// Signing this binds the proof to this account, this key, and this
    /// challenge — a replay against another account or challenge does not
    /// verify.
    #[must_use]
    pub fn canonical_transcript(challenge: &Challenge) -> Vec<u8> {
        let fields = [
            TRANSCRIPT_DOMAIN,
            challenge.challenge_id.as_str(),
            challenge.sub.as_str(),
            &hex_encode(&challenge.iroh_id),
            &B64.encode(challenge.nonce),
            &unix_secs_to_rfc3339(challenge.expires_at),
        ];
        let mut out = Vec::new();
        for field in fields {
            out.extend_from_slice(&(field.len() as u32).to_be_bytes());
            out.extend_from_slice(field.as_bytes());
        }
        out
    }

    /// Steps 4–5: verify the proof and, on success, write the binding
    /// record with the service attestation, set `account.binding_id`, and
    /// return the completed record. Checks run in the specified order:
    /// existence, expiry, single-use, session match, signature, conflicts.
    /// The challenge is consumed on first presentation, success or failure.
    ///
    /// `cse_choice` is the user's client-side-encryption choice at
    /// binding (§7): `Some(true/false)` records it; `None` takes the
    /// default, which is ON per the #2046 ruling.
    ///
    /// # Errors
    ///
    /// See [`CeremonyError`]. Conflict variants route to the rebind
    /// protocol (slice 2) and are never completed here.
    pub fn submit_proof(
        &mut self,
        session_sub: &str,
        challenge_id: &str,
        key_signature: &[u8],
        cse_choice: Option<bool>,
    ) -> Result<BindingRecord, CeremonyError> {
        if self.consumed_challenges.contains(challenge_id) {
            return Err(CeremonyError::ChallengeReplay);
        }
        let challenge = self
            .challenges
            .get(challenge_id)
            .cloned()
            .ok_or(CeremonyError::UnknownChallenge)?;
        let now = self.now();
        if now >= challenge.expires_at {
            self.challenges.remove(challenge_id);
            self.consumed_challenges.insert(challenge_id.to_owned());
            return Err(CeremonyError::ChallengeExpired);
        }
        if session_sub != challenge.sub {
            self.consume(challenge_id);
            return Err(CeremonyError::SessionMismatch);
        }
        // Consumed on first presentation, success or failure (§3.2).
        self.consume(challenge_id);
        let signature =
            Signature::from_slice(key_signature).map_err(|_| CeremonyError::MalformedSignature)?;
        let verifying = VerifyingKey::from_bytes(&challenge.iroh_id)
            .map_err(|_| CeremonyError::BadSignature)?;
        if verifying
            .verify_strict(&Self::canonical_transcript(&challenge), &signature)
            .is_err()
        {
            return Err(CeremonyError::BadSignature);
        }
        // Conflict checks (§1.2 rules): an `active` or `rebind_pending`
        // binding reserves the account; one `sub` per `iroh_id` at a time.
        if let Some(existing_id) = self.binding_by_sub.get(session_sub) {
            let Some(existing) = self.bindings.get(existing_id) else {
                // An index entry without a record is corrupt registry state,
                // not a bindable account: refuse rather than overwrite.
                return Err(CeremonyError::SubAlreadyBound);
            };
            match existing.status {
                BindingStatus::Active => return Err(CeremonyError::SubAlreadyBound),
                BindingStatus::RebindPending => return Err(CeremonyError::RebindPending),
                BindingStatus::Superseded => {
                    // Unreachable through this slice's API (a superseded
                    // record never stays behind the `binding_by_sub` index),
                    // but a fresh ceremony still may not overwrite it: route
                    // to the rebind protocol rather than minting a second
                    // record.
                    return Err(CeremonyError::SubAlreadyBound);
                }
            }
        }
        if self
            .binding_by_iroh
            .contains_key(&hex_encode(&challenge.iroh_id))
        {
            return Err(CeremonyError::IrohIdBoundElsewhere);
        }

        let account = self
            .accounts
            .get(session_sub)
            .cloned()
            .ok_or(CeremonyError::InvalidSub)?;
        let transcript = ChallengeTranscript {
            challenge_id: challenge.challenge_id.clone(),
            sub: challenge.sub.clone(),
            iroh_id: hex_encode(&challenge.iroh_id),
            service_id: challenge.service_id.clone(),
            nonce: B64.encode(challenge.nonce),
            issued_at: unix_secs_to_rfc3339(challenge.issued_at),
            expires_at: unix_secs_to_rfc3339(challenge.expires_at),
        };
        let mut record = BindingRecord {
            binding_id: random_hex_id(),
            version: 1,
            sub: session_sub.to_owned(),
            account_id: account.account_id.clone(),
            iroh_id: hex_encode(&challenge.iroh_id),
            bound_at: unix_secs_to_rfc3339(now),
            challenge_id: challenge.challenge_id.clone(),
            challenge_transcript: transcript,
            key_signature: hex_encode(key_signature),
            service_attestation: String::new(),
            status: BindingStatus::Active,
            client_side_encryption: resolve_cse_choice(cse_choice),
            history: Vec::new(),
        };
        record.service_attestation = hex_encode(
            &self
                .attest_bytes(&record)
                .ok_or(CeremonyError::RecordEncoding)?,
        );
        self.bindings
            .insert(record.binding_id.clone(), record.clone());
        self.binding_by_sub
            .insert(session_sub.to_owned(), record.binding_id.clone());
        self.binding_by_iroh
            .insert(record.iroh_id.clone(), record.binding_id.clone());
        if let Some(account) = self.accounts.get_mut(session_sub) {
            account.binding_id = Some(record.binding_id.clone());
        }
        Ok(record)
    }

    fn consume(&mut self, challenge_id: &str) {
        self.challenges.remove(challenge_id);
        self.consumed_challenges.insert(challenge_id.to_owned());
    }

    // -- variants (§3.3) -----------------------------------------------------

    /// Variant (a), in-browser: request, sign with the locally held root
    /// key, and submit over the same session. The key never leaves the
    /// caller — only the public key and signature cross into the service.
    /// Callers must surface [`BROWSER_KEY_EXPORT_NOTICE`] on the final
    /// screen.
    ///
    /// # Errors
    ///
    /// As [`HostedBindingService::submit_proof`].
    pub fn complete_browser_ceremony(
        &mut self,
        session_sub: &str,
        root_key: &SigningKey,
        cse_choice: Option<bool>,
    ) -> Result<BindingRecord, CeremonyError> {
        let iroh_id = root_key.verifying_key().to_bytes();
        let challenge = self.request_challenge(session_sub, &iroh_id)?;
        let signature = root_key.sign(&Self::canonical_transcript(&challenge));
        self.submit_proof(
            session_sub,
            &challenge.challenge_id,
            &signature.to_bytes(),
            cse_choice,
        )
    }

    /// Variant (b), own-node, browser side: mint a short-lived pairing code
    /// tied to the pending ceremony. The browser displays it; the user
    /// enters it on their node. Single-use, expires with the challenge,
    /// rate-limited per account.
    ///
    /// # Errors
    ///
    /// [`CeremonyError::UnknownChallenge`] / [`CeremonyError::SessionMismatch`]
    /// for a foreign or missing challenge; [`CeremonyError::ChallengeExpired`]
    /// / [`CeremonyError::ChallengeReplay`] for a dead one;
    /// [`CeremonyError::RateLimited`] after too many pairing codes.
    pub fn create_pairing_code(
        &mut self,
        session_sub: &str,
        challenge_id: &str,
    ) -> Result<String, CeremonyError> {
        let challenge = self
            .challenges
            .get(challenge_id)
            .cloned()
            .ok_or(CeremonyError::UnknownChallenge)?;
        if self.consumed_challenges.contains(challenge_id) {
            return Err(CeremonyError::ChallengeReplay);
        }
        let now = self.now();
        if now >= challenge.expires_at {
            return Err(CeremonyError::ChallengeExpired);
        }
        if session_sub != challenge.sub {
            return Err(CeremonyError::SessionMismatch);
        }
        let log = self
            .pairing_issued_at
            .entry(session_sub.to_owned())
            .or_default();
        prune_before(log, now.saturating_sub(3_600));
        if log.len() >= MAX_PAIRINGS_PER_HOUR {
            return Err(CeremonyError::RateLimited);
        }
        log.push(now);

        let mut rng = rand::rng();
        let code = loop {
            let candidate: u32 = rng.random_range(0..1_000_000);
            let candidate = format!("{candidate:06}");
            if !self.pairing_codes.contains_key(&candidate) {
                break candidate;
            }
        };
        self.pairing_codes.insert(
            code.clone(),
            PairingCode {
                challenge_id: challenge.challenge_id.clone(),
                expires_at: challenge.expires_at,
            },
        );
        Ok(code)
    }

    /// Variant (b), own-node, node side: exchange the pairing code for the
    /// challenge. The node signs locally (the key never leaves the node)
    /// and submits the proof against the same `challenge_id` via
    /// [`HostedBindingService::submit_proof`]. The code is single-use.
    ///
    /// # Errors
    ///
    /// [`CeremonyError::UnknownChallenge`] for a missing, used, or
    /// challenge-dead code; [`CeremonyError::ChallengeExpired`] past expiry.
    pub fn exchange_pairing_code(&mut self, code: &str) -> Result<Challenge, CeremonyError> {
        let pairing = self
            .pairing_codes
            .remove(code)
            .ok_or(CeremonyError::UnknownChallenge)?;
        let challenge = self
            .challenges
            .get(&pairing.challenge_id)
            .cloned()
            .ok_or(CeremonyError::UnknownChallenge)?;
        if self.now() >= pairing.expires_at {
            return Err(CeremonyError::ChallengeExpired);
        }
        Ok(challenge)
    }

    // -- records -------------------------------------------------------------

    /// The binding record by ID.
    #[must_use]
    pub fn binding(&self, binding_id: &str) -> Option<&BindingRecord> {
        self.bindings.get(binding_id)
    }

    /// The account's current binding, if the account has completed one.
    #[must_use]
    pub fn binding_for_sub(&self, sub: &str) -> Option<&BindingRecord> {
        self.binding_by_sub
            .get(sub)
            .and_then(|id| self.bindings.get(id))
    }

    /// Verify the key signature from the record's own stored transcript
    /// (§1.2): reconstruct the signed message from `challenge_id`, `sub`,
    /// `iroh_id`, `nonce`, and `expires_at` with no hosted-service access.
    /// Returns `false` on any malformed field or mismatch.
    #[must_use]
    pub fn verify_key_signature(record: &BindingRecord) -> bool {
        let transcript = &record.challenge_transcript;
        let Some(iroh_bytes) = hex_decode(&transcript.iroh_id, 32) else {
            return false;
        };
        let Ok(iroh_id) = <[u8; 32]>::try_from(iroh_bytes) else {
            return false;
        };
        let Ok(nonce_bytes) = B64.decode(&transcript.nonce) else {
            return false;
        };
        let Ok(nonce) = <[u8; 32]>::try_from(nonce_bytes) else {
            return false;
        };
        if transcript.challenge_id != record.challenge_id
            || transcript.sub != record.sub
            || transcript.iroh_id != record.iroh_id
        {
            return false;
        }
        let challenge = Challenge {
            challenge_id: transcript.challenge_id.clone(),
            sub: transcript.sub.clone(),
            iroh_id,
            service_id: transcript.service_id.clone(),
            nonce,
            issued_at: parse_rfc3339(&transcript.issued_at).unwrap_or(0),
            expires_at: parse_rfc3339(&transcript.expires_at).unwrap_or(0),
        };
        // A transcript whose expiry does not parse cannot have been signed
        // by [`canonical_transcript`], which always emits valid RFC 3339.
        if parse_rfc3339(&transcript.expires_at).is_none()
            || parse_rfc3339(&transcript.issued_at).is_none()
        {
            return false;
        }
        let Some(signature_bytes) = hex_decode(&record.key_signature, 64) else {
            return false;
        };
        // `hex_decode` already enforces the length; copy into the array.
        let mut signature_array = [0u8; 64];
        signature_array.copy_from_slice(&signature_bytes);
        let Ok(signature) = Signature::from_slice(&signature_array) else {
            return false;
        };
        let Ok(verifying) = VerifyingKey::from_bytes(&challenge.iroh_id) else {
            return false;
        };
        verifying
            .verify_strict(&Self::canonical_transcript(&challenge), &signature)
            .is_ok()
    }

    /// Verify the service attestation against the record as it currently
    /// stands. Verifiers never accept a record whose attestation does not
    /// match its current fields.
    #[must_use]
    pub fn verify_attestation(&self, record: &BindingRecord) -> bool {
        Self::verify_service_attestation(&self.service_public_key(), record)
    }

    /// Verify the service signature using a pinned service public key,
    /// without access to the hosted service (§1.2, importer requirement).
    #[must_use]
    pub fn verify_service_attestation(public_key: &[u8; 32], record: &BindingRecord) -> bool {
        let Some(digest) = record_digest(record) else {
            return false;
        };
        let Some(signature_bytes) = hex_decode(&record.service_attestation, 64) else {
            return false;
        };
        let mut signature_array = [0u8; 64];
        signature_array.copy_from_slice(&signature_bytes);
        let Ok(signature) = Signature::from_slice(&signature_array) else {
            return false;
        };
        let Ok(verifying_key) = VerifyingKey::from_bytes(public_key) else {
            return false;
        };
        verifying_key.verify_strict(&digest, &signature).is_ok()
    }

    fn attest_bytes(&self, record: &BindingRecord) -> Option<[u8; 64]> {
        let digest = record_digest(record)?;
        Some(self.service_key.sign(&digest).to_bytes())
    }
}

/// Resolve the client-side-encryption choice at binding (§7): an explicit
/// choice is recorded as-is; absent a choice the default is ON per the
/// #2046 ruling.
#[must_use]
pub fn resolve_cse_choice(choice: Option<bool>) -> bool {
    choice.unwrap_or(CSE_DEFAULT_ON)
}

/// Whether `device` may act for the bound account: devices attach through
/// the platform's existing groups-layer device grants (§1.3), exactly as in
/// the self-hosted model. A device grant references the bound `iroh_id`;
/// revoking a device never touches the binding record.
///
/// `account` must be the binding's `iroh_id`; the projection supplies the
/// grants in force.
#[must_use]
pub fn device_authorized_for(
    projection: &crate::groups::Projection,
    group: &p2panda_core::Hash,
    account: &crate::groups::Key,
    device: &crate::groups::Key,
) -> bool {
    projection.active_devices(group, account).contains(device)
}

// -- canonical record digest -------------------------------------------------

/// The canonical record bytes the service attests: every field of the
/// record except `service_attestation` itself, in declaration order.
#[derive(Serialize)]
struct AttestedRecord<'a> {
    binding_id: &'a str,
    version: u32,
    sub: &'a str,
    account_id: &'a str,
    iroh_id: &'a str,
    bound_at: &'a str,
    challenge_id: &'a str,
    challenge_transcript: &'a ChallengeTranscript,
    key_signature: &'a str,
    status: &'a BindingStatus,
    client_side_encryption: bool,
    history: &'a [HistoryEntry],
}

fn record_digest(record: &BindingRecord) -> Option<[u8; 32]> {
    let view = AttestedRecord {
        binding_id: &record.binding_id,
        version: record.version,
        sub: &record.sub,
        account_id: &record.account_id,
        iroh_id: &record.iroh_id,
        bound_at: &record.bound_at,
        challenge_id: &record.challenge_id,
        challenge_transcript: &record.challenge_transcript,
        key_signature: &record.key_signature,
        status: &record.status,
        client_side_encryption: record.client_side_encryption,
        history: &record.history,
    };
    let bytes = serde_json::to_vec(&view).ok()?;
    let digest = Sha256::digest(&bytes);
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    Some(out)
}

fn random_hex_id() -> String {
    let mut rng = rand::rng();
    let mut bytes = [0u8; 16];
    rng.fill(&mut bytes);
    hex_encode(&bytes)
}

fn prune_before(log: &mut Vec<u64>, cutoff: u64) {
    log.retain(|timestamp| *timestamp >= cutoff);
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::groups::policy::{Policy, Template};
    use crate::groups::{fold, Body, Create, DeviceOp, Event, Key, Role};

    const SUB: &str = "108204481453987112345678";
    const OTHER_SUB: &str = "209305592564098223456789";
    const TIER: &str = "free";

    fn service() -> HostedBindingService {
        let mut service = HostedBindingService::with_seed("test-service", [9u8; 32]);
        service.set_now_override(Some(1_700_000_000));
        service
    }

    fn root_key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn signup(service: &mut HostedBindingService, sub: &str) {
        service.signup_account(sub, None, TIER).unwrap();
    }

    #[test]
    fn account_registry_keys_on_sub_and_stores_contact_only() {
        let mut service = service();
        let account = service
            .signup_account(SUB, Some("user@example.com"), TIER)
            .unwrap();
        assert_eq!(account.binding_id, None);
        assert_eq!(account.email_contact.as_deref(), Some("user@example.com"));

        // Repeat sign-in returns the same account; nothing else is created.
        let again = service
            .signup_account(SUB, Some("new@example.com"), TIER)
            .unwrap();
        assert_eq!(again.account_id, account.account_id);
        assert_eq!(again.email_contact.as_deref(), Some("new@example.com"));
        assert_eq!(again.binding_id, None);
    }

    #[test]
    fn email_is_never_identity() {
        let mut service = service();
        assert_eq!(
            service.signup_account("user@example.com", None, TIER),
            Err(CeremonyError::InvalidSub)
        );
        assert_eq!(
            service.signup_account("", None, TIER),
            Err(CeremonyError::InvalidSub)
        );
        // And there is no lookup by email: the only accessor is by `sub`.
        assert!(service.account("user@example.com").is_none());
    }

    #[test]
    fn browser_variant_binds_and_both_signatures_verify() {
        let mut service = service();
        signup(&mut service, SUB);
        let key = root_key(1);

        let record = service.complete_browser_ceremony(SUB, &key, None).unwrap();
        assert_eq!(record.sub, SUB);
        assert_eq!(record.status, BindingStatus::Active);
        assert_eq!(record.version, 1);
        assert!(record.history.is_empty());
        // CSE default is ON absent an explicit choice (#2046 ruling).
        assert!(record.client_side_encryption);

        assert!(HostedBindingService::verify_key_signature(&record));
        assert!(service.verify_attestation(&record));

        let stored = service.binding(&record.binding_id).unwrap();
        assert_eq!(stored, &record);
        assert_eq!(
            service
                .binding_for_sub(SUB)
                .map(|binding| &binding.binding_id),
            Some(&record.binding_id)
        );
        assert_eq!(
            service.account(SUB).unwrap().binding_id.as_deref(),
            Some(record.binding_id.as_str())
        );
    }

    #[test]
    fn own_node_variant_binds_through_a_pairing_code() {
        let mut service = service();
        signup(&mut service, SUB);
        let key = root_key(2);
        let iroh_id = key.verifying_key().to_bytes();

        // Browser side: request + display code over the same session.
        let challenge = service.request_challenge(SUB, &iroh_id).unwrap();
        let code = service
            .create_pairing_code(SUB, &challenge.challenge_id)
            .unwrap();
        assert_eq!(code.len(), 6);

        // Node side: exchange, sign locally, submit against the challenge.
        let node_challenge = service.exchange_pairing_code(&code).unwrap();
        assert_eq!(node_challenge.challenge_id, challenge.challenge_id);
        let signature = key.sign(&HostedBindingService::canonical_transcript(&node_challenge));
        let record = service
            .submit_proof(
                SUB,
                &node_challenge.challenge_id,
                &signature.to_bytes(),
                Some(false),
            )
            .unwrap();
        assert_eq!(record.iroh_id, hex_encode(&iroh_id));
        assert!(
            !record.client_side_encryption,
            "explicit opt-out is recorded"
        );
        assert!(HostedBindingService::verify_key_signature(&record));
        assert!(service.verify_attestation(&record));

        // The code is single-use.
        assert_eq!(
            service.exchange_pairing_code(&code),
            Err(CeremonyError::UnknownChallenge)
        );
    }

    #[test]
    fn wrong_key_consumes_and_retry_needs_a_fresh_challenge() {
        let mut service = service();
        signup(&mut service, SUB);
        let key = root_key(3);
        let wrong = root_key(4);
        let iroh_id = key.verifying_key().to_bytes();

        let challenge = service.request_challenge(SUB, &iroh_id).unwrap();
        let bad = wrong.sign(&HostedBindingService::canonical_transcript(&challenge));
        assert_eq!(
            service.submit_proof(SUB, &challenge.challenge_id, &bad.to_bytes(), None),
            Err(CeremonyError::BadSignature)
        );
        // Same challenge is now consumed even for a correct retry.
        let good = key.sign(&HostedBindingService::canonical_transcript(&challenge));
        assert_eq!(
            service.submit_proof(SUB, &challenge.challenge_id, &good.to_bytes(), None),
            Err(CeremonyError::ChallengeReplay)
        );
        // A fresh challenge succeeds.
        let fresh = service.request_challenge(SUB, &iroh_id).unwrap();
        let good = key.sign(&HostedBindingService::canonical_transcript(&fresh));
        let record = service
            .submit_proof(SUB, &fresh.challenge_id, &good.to_bytes(), None)
            .unwrap();
        assert!(HostedBindingService::verify_key_signature(&record));
    }

    #[test]
    fn replay_of_a_completed_proof_is_rejected() {
        let mut service = service();
        signup(&mut service, SUB);
        let key = root_key(5);
        let iroh_id = key.verifying_key().to_bytes();

        let challenge = service.request_challenge(SUB, &iroh_id).unwrap();
        let signature = key.sign(&HostedBindingService::canonical_transcript(&challenge));
        service
            .submit_proof(SUB, &challenge.challenge_id, &signature.to_bytes(), None)
            .unwrap();
        assert_eq!(
            service.submit_proof(SUB, &challenge.challenge_id, &signature.to_bytes(), None),
            Err(CeremonyError::ChallengeReplay)
        );
    }

    #[test]
    fn expired_challenge_dies_and_a_fresh_one_works() {
        let mut service = service();
        signup(&mut service, SUB);
        let key = root_key(6);
        let iroh_id = key.verifying_key().to_bytes();

        let challenge = service.request_challenge(SUB, &iroh_id).unwrap();
        service.set_now_override(Some(1_700_000_000 + CHALLENGE_TTL_SECS + 1));
        let signature = key.sign(&HostedBindingService::canonical_transcript(&challenge));
        assert_eq!(
            service.submit_proof(SUB, &challenge.challenge_id, &signature.to_bytes(), None),
            Err(CeremonyError::ChallengeExpired)
        );
        // Expiry alone creates no binding and no account claim.
        assert!(service.binding_for_sub(SUB).is_none());
        assert_eq!(service.account(SUB).unwrap().binding_id, None);

        let fresh = service.request_challenge(SUB, &iroh_id).unwrap();
        let good = key.sign(&HostedBindingService::canonical_transcript(&fresh));
        service
            .submit_proof(SUB, &fresh.challenge_id, &good.to_bytes(), None)
            .unwrap();
        assert!(service.binding_for_sub(SUB).is_some());
    }

    #[test]
    fn account_and_key_uniqueness_conflict_route_to_rebind() {
        let mut service = service();
        signup(&mut service, SUB);
        signup(&mut service, OTHER_SUB);
        let key = root_key(7);
        let other_key = root_key(8);
        let iroh_id = key.verifying_key().to_bytes();
        let other_iroh = other_key.verifying_key().to_bytes();

        service.complete_browser_ceremony(SUB, &key, None).unwrap();

        // Same `sub`, different key: conflict, never an overwrite.
        let challenge = service.request_challenge(SUB, &other_iroh).unwrap();
        let signature = other_key.sign(&HostedBindingService::canonical_transcript(&challenge));
        assert_eq!(
            service.submit_proof(SUB, &challenge.challenge_id, &signature.to_bytes(), None),
            Err(CeremonyError::SubAlreadyBound)
        );

        // Same `iroh_id`, different `sub`: conflict the other way.
        let challenge = service.request_challenge(OTHER_SUB, &iroh_id).unwrap();
        let signature = key.sign(&HostedBindingService::canonical_transcript(&challenge));
        assert_eq!(
            service.submit_proof(
                OTHER_SUB,
                &challenge.challenge_id,
                &signature.to_bytes(),
                None
            ),
            Err(CeremonyError::IrohIdBoundElsewhere)
        );

        // Neither conflict created anything for the other account.
        assert!(service.binding_for_sub(OTHER_SUB).is_none());
    }

    #[test]
    fn first_completed_ceremony_wins_the_race() {
        let mut service = service();
        signup(&mut service, SUB);
        let first = root_key(9);
        let second = root_key(10);
        let first_id = first.verifying_key().to_bytes();
        let second_id = second.verifying_key().to_bytes();

        // Two ceremonies race for one `sub`.
        let first_challenge = service.request_challenge(SUB, &first_id).unwrap();
        let second_challenge = service.request_challenge(SUB, &second_id).unwrap();
        let first_sig = first.sign(&HostedBindingService::canonical_transcript(
            &first_challenge,
        ));
        service
            .submit_proof(
                SUB,
                &first_challenge.challenge_id,
                &first_sig.to_bytes(),
                None,
            )
            .unwrap();
        let second_sig = second.sign(&HostedBindingService::canonical_transcript(
            &second_challenge,
        ));
        assert_eq!(
            service.submit_proof(
                SUB,
                &second_challenge.challenge_id,
                &second_sig.to_bytes(),
                None
            ),
            Err(CeremonyError::SubAlreadyBound)
        );
        assert_eq!(
            service.binding_for_sub(SUB).unwrap().iroh_id,
            hex_encode(&first_id)
        );
    }

    #[test]
    fn binding_id_is_stable_and_round_trips_through_json() {
        let mut service = service();
        signup(&mut service, SUB);
        let record = service
            .complete_browser_ceremony(SUB, &root_key(11), None)
            .unwrap();
        let binding_id = record.binding_id.clone();

        // Export-shaped JSON round-trip (slice 4 consumes this shape).
        let json = serde_json::to_string(&record).unwrap();
        let restored: BindingRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.binding_id, binding_id);
        assert!(HostedBindingService::verify_key_signature(&restored));
        assert!(service.verify_attestation(&restored));
    }

    #[test]
    fn tampered_records_fail_verification() {
        let mut service = service();
        signup(&mut service, SUB);
        let record = service
            .complete_browser_ceremony(SUB, &root_key(12), None)
            .unwrap();

        let mut tampered_key = record.clone();
        tampered_key.iroh_id = hex_encode(&root_key(13).verifying_key().to_bytes());
        assert!(!HostedBindingService::verify_key_signature(&tampered_key));

        let mut tampered_status = record.clone();
        tampered_status.status = BindingStatus::Superseded;
        assert!(
            !service.verify_attestation(&tampered_status),
            "attestation covers status: a mutated copy must not verify"
        );

        let mut tampered_cse = record.clone();
        tampered_cse.client_side_encryption = !record.client_side_encryption;
        assert!(!service.verify_attestation(&tampered_cse));
    }

    #[test]
    fn devices_attach_through_groups_grants_not_the_binding() {
        let mut service = service();
        signup(&mut service, SUB);
        let account_key = root_key(14).verifying_key().to_bytes();
        let record = service
            .complete_browser_ceremony(SUB, &root_key(15), None)
            .unwrap();
        let bound: Key = hex_decode(&record.iroh_id, 32).unwrap().try_into().unwrap();
        assert_eq!(bound, root_key(15).verifying_key().to_bytes());

        // The binding names the root/person key; devices join through the
        // existing groups device-grant mechanism.
        let group = p2panda_core::Hash::digest(b"device-group");
        let create = Event {
            id: group,
            author: bound,
            after: Vec::new(),
            group: None,
            body: Body::Create(Create {
                template: Template::Custom,
                policy: Policy::default(),
                members: Vec::new(),
                admins_on_create: false,
            }),
        };
        let device: Key = account_key;
        let grant = Event {
            id: p2panda_core::Hash::digest(b"device-grant"),
            author: bound,
            after: vec![group],
            group: Some(group),
            body: Body::Device(DeviceOp::Add {
                device,
                scope: Role::Read,
            }),
        };
        let projection = fold(&[create, grant]).unwrap();
        assert!(device_authorized_for(&projection, &group, &bound, &device));
        assert!(!device_authorized_for(
            &projection,
            &group,
            &bound,
            &[0u8; 32]
        ));

        // Granting a device never touches the binding record.
        assert_eq!(service.binding(&record.binding_id).unwrap(), &record);
    }

    #[test]
    fn pairing_codes_expire_with_the_challenge_and_rate_limit() {
        let mut service = service();
        signup(&mut service, SUB);
        let iroh_id = root_key(16).verifying_key().to_bytes();
        let challenge = service.request_challenge(SUB, &iroh_id).unwrap();
        let code = service
            .create_pairing_code(SUB, &challenge.challenge_id)
            .unwrap();
        service.set_now_override(Some(1_700_000_000 + CHALLENGE_TTL_SECS + 1));
        assert_eq!(
            service.exchange_pairing_code(&code),
            Err(CeremonyError::ChallengeExpired)
        );

        service.set_now_override(Some(1_700_000_000));
        for _ in 0..(MAX_PAIRINGS_PER_HOUR - 1) {
            let challenge = service.request_challenge(SUB, &iroh_id).unwrap();
            service
                .create_pairing_code(SUB, &challenge.challenge_id)
                .unwrap();
        }
        let challenge = service.request_challenge(SUB, &iroh_id).unwrap();
        assert_eq!(
            service.create_pairing_code(SUB, &challenge.challenge_id),
            Err(CeremonyError::RateLimited)
        );
    }

    #[test]
    fn rfc3339_round_trips() {
        assert_eq!(unix_secs_to_rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(unix_secs_to_rfc3339(1_700_000_000), "2023-11-14T22:13:20Z");
        for secs in [0u64, 1, 86_399, 86_400, 1_700_000_000, 4_102_444_800] {
            assert_eq!(
                parse_rfc3339(&unix_secs_to_rfc3339(secs)),
                Some(secs),
                "secs={secs}"
            );
        }
        assert_eq!(parse_rfc3339("2023-13-01T00:00:00Z"), None);
        assert_eq!(parse_rfc3339("2023-02-29T00:00:00Z"), None);
        assert_eq!(parse_rfc3339("2024-02-29T00:00:00Z"), Some(1_709_164_800));
        assert_eq!(parse_rfc3339("not-a-time"), None);
    }

    #[test]
    fn transcript_binds_account_key_and_challenge() {
        let mut service = service();
        signup(&mut service, SUB);
        let key = root_key(17);
        let iroh_id = key.verifying_key().to_bytes();
        let challenge = service.request_challenge(SUB, &iroh_id).unwrap();
        let signature = key.sign(&HostedBindingService::canonical_transcript(&challenge));

        // Replayed against another account or challenge, the proof fails.
        signup(&mut service, OTHER_SUB);
        assert_eq!(
            service.submit_proof(
                OTHER_SUB,
                &challenge.challenge_id,
                &signature.to_bytes(),
                None
            ),
            Err(CeremonyError::SessionMismatch)
        );
    }
}
