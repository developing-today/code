//! Hosted account registry and ID binding ceremony (spec §§1, 3, and 4).
//!
//! Implements slices 1 and 2 of the hosted provisioning + SSO→ID binding
//! spec (`doc/2026-10-30T22-45-00Z_design_hosted_provisioning_sso_binding.md`,
//! issue #2258; tracker #2852; slice 1 #3808, slice 2 #3809), gated by the
//! #2046 synthesis ruling (2026-11-03): client-side encryption is offered
//! at binding and defaults **ON** when no explicit choice is made; the
//! rebind waiting period is 30 days ([`REBIND_WAITING_PERIOD_SECS`]).
//!
//! ## Delayed rebind (§4)
//!
//! [`HostedBindingService::rebind_with_old_key_authorization`] is the fast
//! path (old key available, immediate) and also converts a pending
//! delayed request early (§4.3 approval);
//! [`HostedBindingService::request_delayed_rebind`] opens the delayed
//! path, [`HostedBindingService::cancel_rebind`] is the old key's cancel,
//! and [`HostedBindingService::effect_due_rebind`] publishes the signed
//! [`RebindRecord`] once the wait elapses. A pending rebind reserves the
//! account exactly as an active binding does — no fresh ceremony can
//! bypass the wait (§4.2).
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
//! ## Lifecycle transition table (#5048, closing audit of #5047)
//!
//! Every name the ceremony mints is claimed once, by one holder, for its
//! stated lifetime. Each row names the live-state transitions of one public
//! [`HostedBindingService`] method. Disposition: **conforming** (matches §3.4
//! / §4 as written), **known gap** (behavior differs from the contract; a
//! named follow-up owns the fix), or **out of scope** (not part of this
//! slice).
//!
//! | Method | Live state touched | Transitions | Disposition |
//! | --- | --- | --- | --- |
//! | `new`, `with_seed`, `set_now_override`, `service_id`, `service_public_key` | none | none | conforming |
//! | `signup_account`, `account` | account row | creates or refreshes the row; claims no key | conforming |
//! | `request_challenge` | challenge (live); one-time maps | runs the #5644 bounded sweep of expired one-time state, then inserts a live challenge, expiring after `CHALLENGE_TTL_SECS`; 30/account/hour | conforming; the challenge is not required to belong to an existing account (see `submit_proof`) |
//! | `canonical_transcript` | none | pure | conforming |
//! | `submit_proof` | challenge; binding indexes | expiry: no state change (§3.4). Session mismatch, bad or malformed signature, and conflict consume the challenge. Success consumes it and claims the `sub` and `iroh_id` in the indexes | conforming; an unknown account is detected only after the challenge is consumed |
//! | `complete_browser_ceremony` | as `request_challenge` plus `submit_proof` | composition only | conforming |
//! | `create_pairing_code` | pairing code (live); one-time maps | consumed check precedes live-map lookup (#4868); the #5644 bounded sweep runs before the collision loop, so expired codes no longer reserve their value; minting supersedes every earlier unexchanged code for the challenge (#4871); 10/account/hour | conforming |
//! | `exchange_pairing_code` | pairing code; challenge (read only) | removes the code before any check, so it is single-use even on failure; does not consume the challenge | conforming; see (a) |
//! | `request_delayed_rebind` | challenge; binding; rebind request | expiry: no state change (§3.4, #5048). Session mismatch and bad signature consume the challenge. `active` becomes `rebind_pending`; a pending request reserves the target key | conforming; `SameKey` and `NewKeyBoundElsewhere` after a valid proof leave the challenge consumed, as a successful proof already spent it |
//! | `cancel_rebind` | rebind request; binding | `pending` becomes `cancelled`; binding `rebind_pending` becomes `active` (re-attested) | known gap tracked by #5643: the request is marked `cancelled` before binding lookup and re-attestation, so `NoBinding` or `RecordEncoding` can fail after that mutation |
//! | `rebind_with_old_key_authorization` | challenge; binding; rebind request | fast path, or early approval of a pending request (`pending` becomes `effected`). Expiry: no state change. Once proof verification starts, session mismatch or signature failure consumes the challenge; success swaps the key index and appends history | conforming; pre-proof request/binding errors do not consume the challenge; failures after a valid proof leave it consumed |
//! | `effect_due_rebind` | rebind request; binding | `pending` becomes `effected` once `effective_at` has passed; the target reservation is rechecked before any mutation; `NewKeyBoundElsewhere` leaves the request `pending` | conforming |
//! | `apply_rebind` (private) | binding; key index; history; rebind records | refuses a foreign key-index owner; appends history; releases the old key from the index; claims the new key | conforming |
//! | `rebind_request`, `pending_rebind_for_binding`, `rebind_records`, `binding`, `binding_for_sub`, `verify_rebind_record`, `verify_key_signature`, `verify_attestation`, `verify_service_attestation` | none | read only | conforming |
//!
//! `verify_challenge_proof` (private, shared by the two rebind entry
//! points) follows the same §3.4 rule as `submit_proof`: expiry returns
//! [`RebindError::ChallengeExpired`] with no state change (#5048 correction).
//! Its doc names the same semantics, so one challenge object has one expiry
//! meaning on both error surfaces.
//!
//! Dispositions for the four audit items:
//!
//! - **(a) Expired pairing codes.** Fixed by #5644. `create_pairing_code`
//!   runs the bounded sweep (`prune_expired_one_time_state`) before its
//!   collision loop, so an expired, unexchanged code no longer reserves
//!   its six-digit value; a code also still leaves `pairing_codes` when
//!   exchanged or superseded for the same challenge.
//! - **(b) `consumed_challenges` growth.** Fixed by #5644. A consumed
//!   marker now carries the consumed challenge's original `expires_at`:
//!   replay is rejected through the whole validity window, the marker is
//!   pruned once that window ends (on access via `replay_blocked`, or by
//!   the sweep), and a later presentation then reports
//!   `UnknownChallenge` rather than `ChallengeReplay` — the documented
//!   post-expiry error-semantics change. The same sweep removes expired
//!   unconsumed challenges from `challenges`, so none of the three
//!   one-time maps grows for the process lifetime anymore.
//! - **(c) Reservation scan cost.** `pending_rebind_reserves_iroh` scans
//!   every retained rebind request. Requests are kept in all statuses, so
//!   the scan grows with history. A target-key index is the named
//!   alternative. It is not taken: reservation is a predicate over `Pending`
//!   requests, so there is one source of truth, and a second index would be
//!   a second place for state to disagree (the #4887 failure family). The
//!   cost is accepted at this volume; revisit if request history is pruned
//!   or grows large.
//! - **(d) Old-key-side symmetry.** Confirmed without a new reservation. The
//!   old key stays the binding's current key in `binding_by_iroh`, and the
//!   binding is `rebind_pending`, for the whole wait. `apply_rebind` releases
//!   it only when the rebind takes effect, after which it survives only in
//!   history. This matches the contract: the old key is no longer the
//!   account's once the rebind is effective.
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

use std::collections::HashMap;

use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
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
pub const BROWSER_KEY_EXPORT_NOTICE: &str = "Export your Iroh root key now and keep the copy somewhere safe. If this browser \
    profile is lost without an exported copy, the binding cannot be recovered except \
    through the delayed-rebind wait.";

/// Delayed-rebind waiting period (§4.2): **30 days**, the #2046 ruling of
/// 2026-11-03. The length is the security property of the delayed path —
/// it is a settled parameter, never chosen locally.
pub const REBIND_WAITING_PERIOD_SECS: u64 = 30 * 24 * 3_600;

/// Domain prefix for the fast-path rebind authorization (§4.1): the old
/// root key signs over `{binding_id, new_iroh_id, challenge_id}`.
pub const REBIND_AUTHORIZATION_DOMAIN: &str = "id-rebind/v1";

/// Domain prefix for the old key's cancellation signature (§4.3): the old
/// root key signs `id-rebind-cancel/v1 || rebind_id`.
pub const REBIND_CANCEL_DOMAIN: &str = "id-rebind-cancel/v1";

/// Domain prefix for the published rebind record (§4.4).
pub const REBIND_RECORD_DOMAIN: &str = "id-rebind-record/v1";

/// `via` value in history and rebind records for the fast path (§4.1).
pub const REBIND_VIA_OLD_KEY: &str = "old-key-signature";

/// `via` value in history and rebind records for the delayed path (§4.2).
pub const REBIND_VIA_DELAYED: &str = "delayed-rebind";

/// Cancellation method named in every rebind notice (§4.3).
pub const REBIND_CANCELLATION_METHOD: &str =
    "sign id-rebind-cancel/v1 || rebind_id with the old root key";

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
    if !(1..=12).contains(&month) || hour > 23 || minute > 59 || second > 59 {
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

/// Binding status (§1.2). The ceremony creates `active` bindings; the
/// delayed-rebind protocol (§4, #3809) drives `rebind_pending` and back;
/// `superseded` names replaced records in the verifiable history chain.
/// Conflict checks reserve the account in `active` and `rebind_pending`
/// alike, exactly as specified.
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
/// expires with the challenge, rate-limited per account. At most one live
/// code exists per challenge: minting a new code supersedes every earlier
/// unexchanged code for the same challenge (#4871).
#[derive(Clone, Debug, PartialEq, Eq)]
struct PairingCode {
    /// The pending ceremony's challenge.
    challenge_id: String,
    /// Expiry (Unix seconds, equal to the challenge's).
    expires_at: u64,
}

/// Status of a delayed-rebind request (§4.2). Cancelled and effected
/// requests are retained, never deleted: the request log is the auditable
/// history of the rebind protocol.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RebindRequestStatus {
    /// Waiting out the delay; the old key keeps full control.
    Pending,
    /// Cancelled by the old key (§4.3). Final for this request.
    Cancelled,
    /// The rebind took effect (at `effective_at`, or early via §4.3
    /// old-key approval converting it to the fast path).
    Effected,
}

/// A delayed-rebind request (§4.2): created from an authenticated session
/// for the binding's `sub` plus a fresh §3 challenge proving possession of
/// the **new** key. Sign-in alone never creates one silently — the new
/// key's challenge proof is required.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebindRequest {
    /// Opaque request ID, minted at creation.
    pub rebind_id: String,
    /// The binding being rebound (stable across the rebind).
    pub binding_id: String,
    /// The binding's Google `sub`.
    pub sub: String,
    /// The current Iroh root public key (64 hex characters).
    pub old_iroh_id: String,
    /// The replacement Iroh root public key (64 hex characters).
    pub new_iroh_id: String,
    /// Request time (RFC 3339).
    pub requested_at: String,
    /// `requested_at + REBIND_WAITING_PERIOD_SECS` (RFC 3339).
    pub effective_at: String,
    /// Request status.
    pub status: RebindRequestStatus,
}

/// The §4.3 notification artefact for a delayed-rebind request: what the
/// service delivers to every recorded contact point (the account's
/// `email_contact`, the old key's known endpoints/worlds, and old-key
/// session banners). It names the new key in fingerprint form, the
/// effective time, and the cancellation method.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebindNotice {
    /// The request this notice is about.
    pub rebind_id: String,
    /// The binding being rebound.
    pub binding_id: String,
    /// The binding's Google `sub`.
    pub sub: String,
    /// The account's recorded contact point, if any (§1.1).
    pub email_contact: Option<String>,
    /// Fingerprint of the new key: first 16 hex characters of the
    /// SHA-256 of the new root public key.
    pub new_iroh_id_fingerprint: String,
    /// When the rebind takes effect unless cancelled (RFC 3339).
    pub effective_at: String,
    /// How the old key cancels: [`REBIND_CANCELLATION_METHOD`].
    pub cancellation_method: String,
}

/// A published rebind record (§4.4): the verifiable artefact peers and
/// importers learn the new key from. Signed by the service over every
/// field; countersigned by the new key over the transfer claim
/// (`binding_id`, old key, new key, `via`) — see
/// [`RebindRecord::countersign_bytes`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebindRecord {
    /// The request or fast-path operation this record publishes.
    pub rebind_id: String,
    /// The binding that was rebound.
    pub binding_id: String,
    /// The previous Iroh root public key (64 hex characters).
    pub old_iroh_id: String,
    /// The new Iroh root public key (64 hex characters).
    pub new_iroh_id: String,
    /// When the rebind took effect (RFC 3339).
    pub effective_at: String,
    /// `old-key-signature` (fast path / early approval) or
    /// `delayed-rebind` (waited out).
    pub via: String,
    /// Service signature (hex) over [`RebindRecord::canonical_bytes`].
    pub service_signature: String,
    /// New key's countersignature (hex) over
    /// [`RebindRecord::countersign_bytes`].
    pub new_key_countersignature: String,
}

impl RebindRecord {
    /// The canonical bytes the service signs: every field of the record
    /// except the two signatures, length-prefixed under
    /// [`REBIND_RECORD_DOMAIN`].
    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        canonical_fields(
            REBIND_RECORD_DOMAIN,
            &[
                &self.rebind_id,
                &self.binding_id,
                &self.old_iroh_id,
                &self.new_iroh_id,
                &self.effective_at,
                &self.via,
            ],
        )
    }

    /// The canonical bytes the new key countersigns: the transfer claim
    /// itself — this binding, from this old key, to this new key, by this
    /// path. Timestamps and the request ID are the service's attestation
    /// (covered by the service signature), so the new key can sign its
    /// acceptance before the service fixes them.
    #[must_use]
    pub fn countersign_bytes(&self) -> Vec<u8> {
        canonical_fields(
            REBIND_RECORD_DOMAIN,
            &[
                &self.binding_id,
                &self.old_iroh_id,
                &self.new_iroh_id,
                &self.via,
            ],
        )
    }
}

/// Build length-prefixed canonical bytes: the domain, then each field,
/// each preceded by its u32 big-endian length (the §3.2 transcript style).
fn canonical_fields(domain: &str, fields: &[&str]) -> Vec<u8> {
    let mut out = Vec::new();
    for field in std::iter::once(&domain).chain(fields.iter()) {
        out.extend_from_slice(&(field.len() as u32).to_be_bytes());
        out.extend_from_slice(field.as_bytes());
    }
    out
}

/// The canonical bytes of the §4.1 fast-path authorization: the old root
/// key signs over `{binding_id, new_iroh_id, challenge_id}` from a fresh
/// §3 challenge.
#[must_use]
pub fn rebind_authorization_bytes(
    binding_id: &str,
    new_iroh_id: &IrohId,
    challenge_id: &str,
) -> Vec<u8> {
    canonical_fields(
        REBIND_AUTHORIZATION_DOMAIN,
        &[binding_id, &hex_encode(new_iroh_id), challenge_id],
    )
}

/// The bytes of the §4.3 cancellation: the old root key signs
/// `id-rebind-cancel/v1 || rebind_id` (the spec's literal construction).
#[must_use]
pub fn rebind_cancel_bytes(rebind_id: &str) -> Vec<u8> {
    let mut out = REBIND_CANCEL_DOMAIN.as_bytes().to_vec();
    out.extend_from_slice(rebind_id.as_bytes());
    out
}

/// Fingerprint form of a key for notices (§4.3): first 16 hex characters
/// of the SHA-256 of the key bytes named by `iroh_id_hex`.
fn key_fingerprint(iroh_id_hex: &str) -> String {
    let Some(bytes) = hex_decode(iroh_id_hex, 32) else {
        return String::new();
    };
    let digest = Sha256::digest(&bytes);
    hex_encode(&digest[..8])
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
    /// This `iroh_id` is bound to another `sub` or reserved by a pending
    /// delayed rebind. Not retryable as a fresh binding; use the rebind path.
    #[error("key is bound or reserved elsewhere: use the rebind protocol")]
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

/// Delayed-rebind failures (§4). No failure path mutates the binding, the
/// pending request, or any index: a failed rebind operation leaves the
/// registry exactly as it found it.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum RebindError {
    /// The `sub` has no binding record at all.
    #[error("no binding for this account: complete a binding ceremony first")]
    NoBinding,
    /// The binding exists but is not `active` (e.g. superseded).
    #[error("binding is not active")]
    BindingNotActive,
    /// A delayed rebind is already pending for this binding. Resolve it
    /// (old-key cancel/approve, or wait out the delay) first.
    #[error("a delayed rebind is already pending for this binding")]
    RebindAlreadyPending,
    /// No rebind request with that ID.
    #[error("unknown rebind request")]
    UnknownRebind,
    /// The request exists but is no longer pending.
    #[error("rebind request is not pending")]
    RebindNotPending,
    /// The waiting period has not elapsed yet.
    #[error("rebind is not yet effective: the waiting period has not elapsed")]
    RebindNotYetEffective,
    /// The replacement key is the currently bound key.
    #[error("new key is the currently bound key")]
    SameKey,
    /// The replacement key is bound elsewhere or reserved by another
    /// pending delayed rebind.
    #[error("new key is bound or reserved elsewhere")]
    NewKeyBoundElsewhere,
    /// No live challenge with that ID.
    #[error("unknown challenge: request a fresh one")]
    UnknownChallenge,
    /// The challenge expired. State change: none; the challenge is dead.
    /// Retry with a fresh challenge.
    #[error("challenge expired: request a fresh challenge")]
    ChallengeExpired,
    /// The challenge was already presented.
    #[error("challenge already consumed: request a fresh challenge")]
    ChallengeReplay,
    /// The presenting session's `sub` is not the challenge's `sub`.
    #[error("session mismatch: the presenting session is not the challenged account")]
    SessionMismatch,
    /// The new key's challenge proof does not verify.
    #[error("bad signature: the new key proof does not verify")]
    BadSignature,
    /// The old key's authorization or cancellation signature does not
    /// verify against the currently bound key.
    #[error("bad old-key signature: the authorization does not verify against the bound key")]
    BadOldKeySignature,
    /// The new key's rebind-record countersignature does not verify.
    #[error("bad countersignature: the new key countersignature does not verify")]
    BadCountersignature,
    /// Malformed signature bytes.
    #[error("malformed signature: expected 64 bytes")]
    MalformedSignature,
    /// The service could not encode the canonical binding record.
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
    /// Consumed challenge IDs mapped to the consumed challenge's original
    /// `expires_at` (#5644): a marker blocks replay only until that expiry,
    /// after which it is pruned and a late presentation reports
    /// `UnknownChallenge`, not `ChallengeReplay`.
    consumed_challenges: HashMap<String, u64>,
    rebind_requests: HashMap<String, RebindRequest>,
    rebind_records: Vec<RebindRecord>,
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
            consumed_challenges: HashMap::new(),
            rebind_requests: HashMap::new(),
            rebind_records: Vec::new(),
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
    /// Sign bytes with the service key for artefacts the service
    /// itself must authenticate — the unkeyed-export manifest of
    /// §6.1, produced when the user's key is unavailable.
    pub fn sign_service_bytes(&self, bytes: &[u8]) -> [u8; 64] {
        self.service_key.sign(bytes).to_bytes()
    }

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
        // Sweep only after the rate-limit check (#5644, Codex P2 on
        // PR #6319): a caller who is already rate-limited must not be
        // able to trigger repeated full-map cleanup on the rejection
        // path.
        self.prune_expired_one_time_state(now);

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
        if self.replay_blocked(challenge_id) {
            return Err(CeremonyError::ChallengeReplay);
        }
        let challenge = self
            .challenges
            .get(challenge_id)
            .cloned()
            .ok_or(CeremonyError::UnknownChallenge)?;
        let now = self.now();
        if now >= challenge.expires_at {
            // §3.4: expiry is state-free — the challenge is neither
            // removed nor recorded as consumed, so a repeat presentation
            // returns ChallengeExpired again, never ChallengeReplay (#4868).
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
        // A pending delayed rebind also reserves its target key.
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
        let iroh_id = hex_encode(&challenge.iroh_id);
        if self.binding_by_iroh.contains_key(&iroh_id)
            || self.pending_rebind_reserves_iroh(&iroh_id, None)
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
            iroh_id,
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
        // The replay marker keeps the challenge's original expiry (#5644):
        // replay is blocked through the whole validity window, and no
        // longer — the marker is prunable from `expires_at` on.
        let expires_at = self
            .challenges
            .get(challenge_id)
            .map_or_else(|| self.now() + CHALLENGE_TTL_SECS, |c| c.expires_at);
        self.challenges.remove(challenge_id);
        self.consumed_challenges
            .insert(challenge_id.to_owned(), expires_at);
    }

    /// Whether `challenge_id` was consumed and its original validity
    /// window has not yet ended (#5644). An expired marker is removed on
    /// access and no longer blocks: the challenge is long gone from
    /// `challenges`, so the caller falls through to `UnknownChallenge`.
    fn replay_blocked(&mut self, challenge_id: &str) -> bool {
        match self.consumed_challenges.get(challenge_id) {
            Some(&expires_at) if self.now() < expires_at => true,
            Some(_) => {
                self.consumed_challenges.remove(challenge_id);
                false
            }
            None => false,
        }
    }

    /// Bounded cleanup sweep for the one-time ceremony maps (#5644):
    /// expired unconsumed challenges, expired unexchanged pairing codes,
    /// and consumed markers past the original challenge expiry are
    /// removed. Called from the rate-limited minting paths
    /// (`request_challenge`, `create_pairing_code`), so the sweep itself
    /// is bounded by the issuance rate limits.
    fn prune_expired_one_time_state(&mut self, now: u64) {
        self.challenges.retain(|_, c| now < c.expires_at);
        self.pairing_codes.retain(|_, p| now < p.expires_at);
        self.consumed_challenges.retain(|_, &mut exp| now < exp);
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
    /// rate-limited per account. Minting supersedes every previously
    /// minted, unexchanged code for the same challenge (#4871), so asking
    /// for a fresh code revokes the old one.
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
        // The consumed check comes first: consume() removes a challenge
        // from the live map, so checking the map first would make this
        // ChallengeReplay branch unreachable (#4868).
        if self.replay_blocked(challenge_id) {
            return Err(CeremonyError::ChallengeReplay);
        }
        let challenge = self
            .challenges
            .get(challenge_id)
            .cloned()
            .ok_or(CeremonyError::UnknownChallenge)?;
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

        // Sweep before the collision loop (#5644): an expired,
        // unexchanged code no longer reserves its six-digit value.
        self.prune_expired_one_time_state(now);
        let mut rng = rand::rng();
        let code = loop {
            let candidate: u32 = rng.random_range(0..1_000_000);
            let candidate = format!("{candidate:06}");
            if !self.pairing_codes.contains_key(&candidate) {
                break candidate;
            }
        };
        // Minting supersedes every earlier unexchanged code for this
        // challenge (#4871): at most one live pairing code per ceremony,
        // so "that code went to the wrong place, give me a new one"
        // actually revokes the old one. The supersede runs after the new
        // code is generated, while the old codes are still in the map, so
        // the collision loop above can never re-issue a just-revoked
        // string as its replacement.
        self.pairing_codes
            .retain(|_, existing| existing.challenge_id != challenge.challenge_id);
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

    // -- delayed rebind (§4) -------------------------------------------------

    /// Verify a presented challenge proof the way §3.2 specifies —
    /// existence, expiry, single-use, session match, signature — and
    /// consume it on the first non-expired presentation. Expiry is
    /// state-free (§3.4). Shared by the rebind paths, which need the same
    /// proof of the **new** key's possession without §3's conflict outcomes.
    fn verify_challenge_proof(
        &mut self,
        session_sub: &str,
        challenge_id: &str,
        key_signature: &[u8],
    ) -> Result<Challenge, RebindError> {
        if self.replay_blocked(challenge_id) {
            return Err(RebindError::ChallengeReplay);
        }
        let challenge = self
            .challenges
            .get(challenge_id)
            .cloned()
            .ok_or(RebindError::UnknownChallenge)?;
        if self.now() >= challenge.expires_at {
            // §3.4, as in `submit_proof`: expiry is state-free (#4868).
            return Err(RebindError::ChallengeExpired);
        }
        if session_sub != challenge.sub {
            self.consume(challenge_id);
            return Err(RebindError::SessionMismatch);
        }
        self.consume(challenge_id);
        let signature =
            Signature::from_slice(key_signature).map_err(|_| RebindError::MalformedSignature)?;
        let verifying =
            VerifyingKey::from_bytes(&challenge.iroh_id).map_err(|_| RebindError::BadSignature)?;
        if verifying
            .verify_strict(&Self::canonical_transcript(&challenge), &signature)
            .is_err()
        {
            return Err(RebindError::BadSignature);
        }
        Ok(challenge)
    }

    /// Verify `signature` (hex) by the key named `iroh_id_hex` over
    /// `message`, mapping failure to `error`.
    fn verify_hex_signature(
        iroh_id_hex: &str,
        message: &[u8],
        signature_hex: &[u8],
        error: RebindError,
    ) -> Result<(), RebindError> {
        let signature =
            Signature::from_slice(signature_hex).map_err(|_| RebindError::MalformedSignature)?;
        let Some(key_bytes) = hex_decode(iroh_id_hex, 32) else {
            return Err(error);
        };
        let Ok(key_array) = <[u8; 32]>::try_from(key_bytes) else {
            return Err(error);
        };
        let verifying = VerifyingKey::from_bytes(&key_array).map_err(|_| error.clone())?;
        verifying
            .verify_strict(message, &signature)
            .map_err(|_| error)
    }

    /// The binding for `sub`, requiring an `active` status for paths that
    /// start a rebind.
    fn active_binding_for_sub(&self, sub: &str) -> Result<BindingRecord, RebindError> {
        let record = self.binding_for_sub(sub).ok_or(RebindError::NoBinding)?;
        match record.status {
            BindingStatus::Active => Ok(record.clone()),
            BindingStatus::RebindPending => Err(RebindError::RebindAlreadyPending),
            BindingStatus::Superseded => Err(RebindError::BindingNotActive),
        }
    }

    /// §4.2: request a delayed rebind. Requires the authenticated session
    /// for the binding's `sub` and a fresh §3 challenge proving possession
    /// of the **new** key (the caller signs the challenge transcript with
    /// the new root key and presents it here). Creates the request, sets
    /// the binding to `rebind_pending` (re-attesting the mutated record),
    /// and returns the request plus the §4.3 notice artefact.
    ///
    /// During the wait the old key keeps full control, the new key has
    /// none, and the pending binding reserves both the account and target
    /// key: fresh ceremonies fail with [`CeremonyError::RebindPending`]
    /// for that account and with [`CeremonyError::IrohIdBoundElsewhere`]
    /// for another account attempting the reserved key.
    ///
    /// # Errors
    ///
    /// See [`RebindError`]: no binding, one already pending, the new key
    /// is the old key, bound elsewhere, or reserved by another pending
    /// request, or the challenge proof fails.
    pub fn request_delayed_rebind(
        &mut self,
        session_sub: &str,
        challenge_id: &str,
        new_key_signature: &[u8],
    ) -> Result<(RebindRequest, RebindNotice), RebindError> {
        let binding = self.active_binding_for_sub(session_sub)?;
        let challenge =
            self.verify_challenge_proof(session_sub, challenge_id, new_key_signature)?;
        let new_iroh_id = hex_encode(&challenge.iroh_id);
        if new_iroh_id == binding.iroh_id {
            return Err(RebindError::SameKey);
        }
        if self.binding_by_iroh.contains_key(&new_iroh_id)
            || self.pending_rebind_reserves_iroh(&new_iroh_id, None)
        {
            return Err(RebindError::NewKeyBoundElsewhere);
        }
        let now = self.now();
        let request = RebindRequest {
            rebind_id: random_hex_id(),
            binding_id: binding.binding_id.clone(),
            sub: session_sub.to_owned(),
            old_iroh_id: binding.iroh_id.clone(),
            new_iroh_id: new_iroh_id.clone(),
            requested_at: unix_secs_to_rfc3339(now),
            effective_at: unix_secs_to_rfc3339(now + REBIND_WAITING_PERIOD_SECS),
            status: RebindRequestStatus::Pending,
        };
        // Set `rebind_pending` and re-attest the mutated record (§4.2).
        let mut mutated = binding.clone();
        mutated.status = BindingStatus::RebindPending;
        mutated.service_attestation = hex_encode(
            &self
                .attest_bytes(&mutated)
                .ok_or(RebindError::RecordEncoding)?,
        );
        self.bindings.insert(mutated.binding_id.clone(), mutated);
        let notice = RebindNotice {
            rebind_id: request.rebind_id.clone(),
            binding_id: request.binding_id.clone(),
            sub: session_sub.to_owned(),
            email_contact: self
                .accounts
                .get(session_sub)
                .and_then(|account| account.email_contact.clone()),
            new_iroh_id_fingerprint: key_fingerprint(&new_iroh_id),
            effective_at: request.effective_at.clone(),
            cancellation_method: REBIND_CANCELLATION_METHOD.to_owned(),
        };
        self.rebind_requests
            .insert(request.rebind_id.clone(), request.clone());
        Ok((request, notice))
    }

    /// §4.3: cancel a pending delayed rebind with the old root key's
    /// signature over [`rebind_cancel_bytes`]. Cancellation is final for
    /// that request; the binding returns to `active` (re-attested) and a
    /// later request restarts the full waiting period.
    ///
    /// # Errors
    ///
    /// [`RebindError::UnknownRebind`] / [`RebindError::RebindNotPending`]
    /// for a dead request; [`RebindError::BadOldKeySignature`] when the
    /// signature does not verify against the currently bound key.
    pub fn cancel_rebind(
        &mut self,
        rebind_id: &str,
        old_key_signature: &[u8],
    ) -> Result<BindingRecord, RebindError> {
        let request = self
            .rebind_requests
            .get(rebind_id)
            .cloned()
            .ok_or(RebindError::UnknownRebind)?;
        if request.status != RebindRequestStatus::Pending {
            return Err(RebindError::RebindNotPending);
        }
        Self::verify_hex_signature(
            &request.old_iroh_id,
            &rebind_cancel_bytes(rebind_id),
            old_key_signature,
            RebindError::BadOldKeySignature,
        )?;
        if let Some(stored) = self.rebind_requests.get_mut(rebind_id) {
            stored.status = RebindRequestStatus::Cancelled;
        }
        let mut binding = self
            .bindings
            .get(&request.binding_id)
            .cloned()
            .ok_or(RebindError::NoBinding)?;
        binding.status = BindingStatus::Active;
        binding.service_attestation = hex_encode(
            &self
                .attest_bytes(&binding)
                .ok_or(RebindError::RecordEncoding)?,
        );
        self.bindings
            .insert(binding.binding_id.clone(), binding.clone());
        Ok(binding)
    }

    /// §4.1 fast path, also the §4.3 approve-early conversion when
    /// `pending_request` names a pending delayed request: the old root
    /// key authorizes the rebind over
    /// [`rebind_authorization_bytes`] from a fresh §3 challenge, the new
    /// key proves possession of itself by signing that challenge's
    /// transcript, and the new key countersigns the §4.4 record. Effect
    /// is immediate on verification: history is appended, the binding's
    /// key is replaced and re-attested, and the signed rebind record is
    /// published (returned and retained in [`Self::rebind_records`]).
    ///
    /// # Errors
    ///
    /// See [`RebindError`]. Without `pending_request` the binding must be
    /// `active`; with one, the request must be pending and name this
    /// binding and new key.
    pub fn rebind_with_old_key_authorization(
        &mut self,
        session_sub: &str,
        pending_request: Option<&str>,
        challenge_id: &str,
        new_key_signature: &[u8],
        old_key_authorization: &[u8],
        new_key_countersignature: &[u8],
    ) -> Result<(BindingRecord, RebindRecord), RebindError> {
        let (binding, request) = match pending_request {
            Some(rebind_id) => {
                let request = self
                    .rebind_requests
                    .get(rebind_id)
                    .cloned()
                    .ok_or(RebindError::UnknownRebind)?;
                if request.status != RebindRequestStatus::Pending {
                    return Err(RebindError::RebindNotPending);
                }
                let binding = self
                    .bindings
                    .get(&request.binding_id)
                    .cloned()
                    .ok_or(RebindError::NoBinding)?;
                if binding.sub != session_sub {
                    return Err(RebindError::SessionMismatch);
                }
                (binding, Some(request))
            }
            None => (self.active_binding_for_sub(session_sub)?, None),
        };
        let challenge =
            self.verify_challenge_proof(session_sub, challenge_id, new_key_signature)?;
        let new_iroh_id = hex_encode(&challenge.iroh_id);
        if let Some(request) = &request {
            if new_iroh_id != request.new_iroh_id {
                return Err(RebindError::BadSignature);
            }
        }
        if new_iroh_id == binding.iroh_id {
            return Err(RebindError::SameKey);
        }
        if self.binding_by_iroh.contains_key(&new_iroh_id)
            || self.pending_rebind_reserves_iroh(
                &new_iroh_id,
                request.as_ref().map(|request| request.rebind_id.as_str()),
            )
        {
            return Err(RebindError::NewKeyBoundElsewhere);
        }
        Self::verify_hex_signature(
            &binding.iroh_id,
            &rebind_authorization_bytes(&binding.binding_id, &challenge.iroh_id, challenge_id),
            old_key_authorization,
            RebindError::BadOldKeySignature,
        )?;
        let effective_at = unix_secs_to_rfc3339(self.now());
        let rebind_id = request
            .as_ref()
            .map_or_else(random_hex_id, |req| req.rebind_id.clone());
        let record = RebindRecord {
            rebind_id,
            binding_id: binding.binding_id.clone(),
            old_iroh_id: binding.iroh_id.clone(),
            new_iroh_id: new_iroh_id.clone(),
            effective_at,
            via: REBIND_VIA_OLD_KEY.to_owned(),
            service_signature: String::new(),
            new_key_countersignature: hex_encode(new_key_countersignature),
        };
        Self::verify_hex_signature(
            &new_iroh_id,
            &record.countersign_bytes(),
            new_key_countersignature,
            RebindError::BadCountersignature,
        )?;
        let (updated, record) = self.apply_rebind(&record)?;
        if let Some(request) = &request {
            if let Some(stored) = self.rebind_requests.get_mut(&request.rebind_id) {
                stored.status = RebindRequestStatus::Effected;
            }
        }
        Ok((updated, record))
    }

    /// §4.4: effect a pending delayed rebind whose waiting period has
    /// elapsed. The new key countersigns the rebind record (its holder
    /// claims control at effect time); the service then appends history,
    /// replaces the binding's key, re-attests, and publishes the signed
    /// record. Before `effective_at` this fails with
    /// [`RebindError::RebindNotYetEffective`] and changes nothing.
    ///
    /// # Errors
    ///
    /// See [`RebindError`].
    pub fn effect_due_rebind(
        &mut self,
        rebind_id: &str,
        new_key_countersignature: &[u8],
    ) -> Result<(BindingRecord, RebindRecord), RebindError> {
        let request = self
            .rebind_requests
            .get(rebind_id)
            .cloned()
            .ok_or(RebindError::UnknownRebind)?;
        if request.status != RebindRequestStatus::Pending {
            return Err(RebindError::RebindNotPending);
        }
        let effective_secs = parse_rfc3339(&request.effective_at).unwrap_or(u64::MAX);
        if self.now() < effective_secs {
            return Err(RebindError::RebindNotYetEffective);
        }
        if self
            .binding_by_iroh
            .get(&request.new_iroh_id)
            .is_some_and(|binding_id| binding_id != &request.binding_id)
            || self.pending_rebind_reserves_iroh(&request.new_iroh_id, Some(rebind_id))
        {
            return Err(RebindError::NewKeyBoundElsewhere);
        }
        let record = RebindRecord {
            rebind_id: request.rebind_id.clone(),
            binding_id: request.binding_id.clone(),
            old_iroh_id: request.old_iroh_id.clone(),
            new_iroh_id: request.new_iroh_id.clone(),
            effective_at: request.effective_at.clone(),
            via: REBIND_VIA_DELAYED.to_owned(),
            service_signature: String::new(),
            new_key_countersignature: hex_encode(new_key_countersignature),
        };
        Self::verify_hex_signature(
            &request.new_iroh_id,
            &record.countersign_bytes(),
            new_key_countersignature,
            RebindError::BadCountersignature,
        )?;
        let (updated, record) = self.apply_rebind(&record)?;
        if let Some(stored) = self.rebind_requests.get_mut(rebind_id) {
            stored.status = RebindRequestStatus::Effected;
        }
        Ok((updated, record))
    }

    /// Apply a verified rebind (§4.4 steps 1–3): reject a foreign key-index
    /// owner, sign the record, append the old key to history, replace the
    /// binding's key, set it back to `active`, re-attest, re-index, and
    /// retain the published record.
    fn apply_rebind(
        &mut self,
        record: &RebindRecord,
    ) -> Result<(BindingRecord, RebindRecord), RebindError> {
        if self
            .binding_by_iroh
            .get(&record.new_iroh_id)
            .is_some_and(|binding_id| binding_id != &record.binding_id)
        {
            return Err(RebindError::NewKeyBoundElsewhere);
        }
        let mut record = record.clone();
        record.service_signature =
            hex_encode(&self.service_key.sign(&record.canonical_bytes()).to_bytes());
        let mut binding = self
            .bindings
            .get(&record.binding_id)
            .cloned()
            .ok_or(RebindError::NoBinding)?;
        binding.history.push(HistoryEntry {
            iroh_id: record.old_iroh_id.clone(),
            superseded_at: record.effective_at.clone(),
            via: record.via.clone(),
            rebind_record_id: if record.via == REBIND_VIA_DELAYED {
                Some(record.rebind_id.clone())
            } else {
                None
            },
        });
        binding.iroh_id = record.new_iroh_id.clone();
        binding.status = BindingStatus::Active;
        binding.service_attestation = hex_encode(
            &self
                .attest_bytes(&binding)
                .ok_or(RebindError::RecordEncoding)?,
        );
        self.binding_by_iroh.remove(&record.old_iroh_id);
        self.binding_by_iroh
            .insert(record.new_iroh_id.clone(), record.binding_id.clone());
        self.bindings
            .insert(binding.binding_id.clone(), binding.clone());
        self.rebind_records.push(record.clone());
        Ok((binding, record))
    }

    /// A rebind request by ID (pending, cancelled, and effected requests
    /// are all retained as the auditable history).
    #[must_use]
    pub fn rebind_request(&self, rebind_id: &str) -> Option<&RebindRequest> {
        self.rebind_requests.get(rebind_id)
    }

    /// The pending delayed-rebind request for a binding, if any.
    #[must_use]
    pub fn pending_rebind_for_binding(&self, binding_id: &str) -> Option<&RebindRequest> {
        self.rebind_requests.values().find(|request| {
            request.binding_id == binding_id && request.status == RebindRequestStatus::Pending
        })
    }

    /// Whether a pending delayed rebind reserves `iroh_id`, optionally
    /// ignoring the request currently being effected or approved.
    fn pending_rebind_reserves_iroh(&self, iroh_id: &str, except_rebind_id: Option<&str>) -> bool {
        self.rebind_requests.values().any(|request| {
            request.status == RebindRequestStatus::Pending
                && request.new_iroh_id == iroh_id
                && Some(request.rebind_id.as_str()) != except_rebind_id
        })
    }

    /// Every rebind record this service has published (§4.4), in
    /// publication order.
    #[must_use]
    pub fn rebind_records(&self) -> &[RebindRecord] {
        &self.rebind_records
    }

    /// Verify a published rebind record offline (§4.4): the service
    /// signature over the full record and the new key's countersignature
    /// over the transfer claim, against a pinned service public key.
    #[must_use]
    pub fn verify_rebind_record(public_key: &[u8; 32], record: &RebindRecord) -> bool {
        let Some(service_signature) = hex_decode(&record.service_signature, 64) else {
            return false;
        };
        let mut service_array = [0u8; 64];
        service_array.copy_from_slice(&service_signature);
        let Ok(service_signature) = Signature::from_slice(&service_array) else {
            return false;
        };
        let Ok(service_key) = VerifyingKey::from_bytes(public_key) else {
            return false;
        };
        if service_key
            .verify_strict(&record.canonical_bytes(), &service_signature)
            .is_err()
        {
            return false;
        }
        let Some(counter) = hex_decode(&record.new_key_countersignature, 64) else {
            return false;
        };
        let mut counter_array = [0u8; 64];
        counter_array.copy_from_slice(&counter);
        let Ok(counter) = Signature::from_slice(&counter_array) else {
            return false;
        };
        let Some(new_key_bytes) = hex_decode(&record.new_iroh_id, 32) else {
            return false;
        };
        let Ok(new_key_array) = <[u8; 32]>::try_from(new_key_bytes) else {
            return false;
        };
        let Ok(new_key) = VerifyingKey::from_bytes(&new_key_array) else {
            return false;
        };
        new_key
            .verify_strict(&record.countersign_bytes(), &counter)
            .is_ok()
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
    use crate::groups::{Body, Create, DeviceOp, Event, Key, Role, fold};

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
    fn minting_a_pairing_code_supersedes_earlier_codes_for_the_challenge() {
        let mut service = service();
        signup(&mut service, SUB);
        let iroh_id = root_key(17).verifying_key().to_bytes();
        let challenge = service.request_challenge(SUB, &iroh_id).unwrap();
        let other = service.request_challenge(SUB, &iroh_id).unwrap();

        let code_a = service
            .create_pairing_code(SUB, &challenge.challenge_id)
            .unwrap();
        let other_code = service
            .create_pairing_code(SUB, &other.challenge_id)
            .unwrap();
        let code_b = service
            .create_pairing_code(SUB, &challenge.challenge_id)
            .unwrap();

        // The superseded code no longer exchanges. The replacement is
        // generated while the old code is still in the map, so the two
        // strings are always distinct.
        assert_ne!(code_a, code_b);
        assert_eq!(
            service.exchange_pairing_code(&code_a),
            Err(CeremonyError::UnknownChallenge)
        );
        // The surviving code exchanges and returns the challenge, once.
        let exchanged = service.exchange_pairing_code(&code_b).unwrap();
        assert_eq!(exchanged.challenge_id, challenge.challenge_id);
        assert_eq!(
            service.exchange_pairing_code(&code_b),
            Err(CeremonyError::UnknownChallenge)
        );
        // Supersede is per-challenge: the other ceremony's code is untouched.
        let other_exchanged = service.exchange_pairing_code(&other_code).unwrap();
        assert_eq!(other_exchanged.challenge_id, other.challenge_id);
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
    fn expired_challenge_is_not_recorded_as_consumed() {
        // #4868: expiry is state-free (§3.4), so presenting the same
        // expired challenge again returns ChallengeExpired, not
        // ChallengeReplay.
        let mut service = service();
        signup(&mut service, SUB);
        let key = root_key(21);
        let iroh_id = key.verifying_key().to_bytes();
        let challenge = service.request_challenge(SUB, &iroh_id).unwrap();
        service.set_now_override(Some(1_700_000_000 + CHALLENGE_TTL_SECS + 1));
        let signature = key.sign(&HostedBindingService::canonical_transcript(&challenge));
        for _ in 0..2 {
            assert_eq!(
                service.submit_proof(SUB, &challenge.challenge_id, &signature.to_bytes(), None),
                Err(CeremonyError::ChallengeExpired)
            );
        }
    }

    #[test]
    fn create_pairing_code_reports_replay_for_a_consumed_challenge() {
        // #4868: a challenge consumed by a failed submit (bad signature)
        // is dead; create_pairing_code must return ChallengeReplay, not
        // UnknownChallenge. Unknown and live challenges are unchanged.
        let mut service = service();
        signup(&mut service, SUB);
        let key = root_key(22);
        let iroh_id = key.verifying_key().to_bytes();
        assert_eq!(
            service.create_pairing_code(SUB, "no-such-challenge"),
            Err(CeremonyError::UnknownChallenge)
        );
        let challenge = service.request_challenge(SUB, &iroh_id).unwrap();
        let bad = root_key(23).sign(&HostedBindingService::canonical_transcript(&challenge));
        assert_eq!(
            service.submit_proof(SUB, &challenge.challenge_id, &bad.to_bytes(), None),
            Err(CeremonyError::BadSignature)
        );
        assert_eq!(
            service.create_pairing_code(SUB, &challenge.challenge_id),
            Err(CeremonyError::ChallengeReplay)
        );
        let live = service.request_challenge(SUB, &iroh_id).unwrap();
        assert!(service.create_pairing_code(SUB, &live.challenge_id).is_ok());
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
    fn consumed_marker_blocks_replay_only_until_original_expiry() {
        // #5644: the replay marker carries the consumed challenge's
        // original expiry. Inside the validity window a replay is
        // ChallengeReplay; past it the marker is pruned on access and
        // the presentation reports UnknownChallenge.
        let mut service = service();
        signup(&mut service, SUB);
        let key = root_key(31);
        let iroh_id = key.verifying_key().to_bytes();
        let challenge = service.request_challenge(SUB, &iroh_id).unwrap();
        let signature = key.sign(&HostedBindingService::canonical_transcript(&challenge));
        service
            .submit_proof(SUB, &challenge.challenge_id, &signature.to_bytes(), None)
            .unwrap();
        // One second before the original expiry: still blocked.
        service.set_now_override(Some(1_700_000_000 + CHALLENGE_TTL_SECS - 1));
        assert_eq!(
            service.submit_proof(SUB, &challenge.challenge_id, &signature.to_bytes(), None),
            Err(CeremonyError::ChallengeReplay)
        );
        assert_eq!(service.consumed_challenges.len(), 1);
        // At the expiry the marker is gone (boundary: now >= expires_at).
        service.set_now_override(Some(1_700_000_000 + CHALLENGE_TTL_SECS));
        assert_eq!(
            service.submit_proof(SUB, &challenge.challenge_id, &signature.to_bytes(), None),
            Err(CeremonyError::UnknownChallenge)
        );
        assert!(service.consumed_challenges.is_empty());
    }

    #[test]
    fn expired_unconsumed_challenge_is_swept_by_the_next_request() {
        // #5644: request_challenge runs the bounded sweep, so an expired
        // unconsumed challenge does not stay in `challenges` for the
        // process lifetime; presenting it afterwards is UnknownChallenge.
        let mut service = service();
        signup(&mut service, SUB);
        let key = root_key(32);
        let iroh_id = key.verifying_key().to_bytes();
        let stale = service.request_challenge(SUB, &iroh_id).unwrap();
        service.set_now_override(Some(1_700_000_000 + CHALLENGE_TTL_SECS));
        let fresh = service.request_challenge(SUB, &iroh_id).unwrap();
        assert_eq!(service.challenges.len(), 1);
        assert!(service.challenges.contains_key(&fresh.challenge_id));
        let signature = key.sign(&HostedBindingService::canonical_transcript(&stale));
        assert_eq!(
            service.submit_proof(SUB, &stale.challenge_id, &signature.to_bytes(), None),
            Err(CeremonyError::UnknownChallenge)
        );
    }

    #[test]
    fn expired_pairing_code_is_swept_and_stops_reserving_its_value() {
        // #5644: create_pairing_code sweeps before its collision loop,
        // so an expired unexchanged code leaves `pairing_codes` and no
        // longer reserves its six-digit value; exchanging it afterwards
        // is UnknownChallenge (it was removed, not merely expired).
        let mut service = service();
        signup(&mut service, SUB);
        let iroh_id = root_key(33).verifying_key().to_bytes();
        let challenge = service.request_challenge(SUB, &iroh_id).unwrap();
        let stale_code = service
            .create_pairing_code(SUB, &challenge.challenge_id)
            .unwrap();
        assert_eq!(service.pairing_codes.len(), 1);
        service.set_now_override(Some(1_700_000_000 + CHALLENGE_TTL_SECS));
        let fresh = service.request_challenge(SUB, &iroh_id).unwrap();
        service
            .create_pairing_code(SUB, &fresh.challenge_id)
            .unwrap();
        assert_eq!(service.pairing_codes.len(), 1);
        assert!(!service.pairing_codes.contains_key(&stale_code));
        assert_eq!(
            service.exchange_pairing_code(&stale_code),
            Err(CeremonyError::UnknownChallenge)
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
        // A leap second is not modelled (#756, the same ruling
        // parse_iso_timestamp follows): second 60 is refused on every
        // date instead of aliasing to the next minute (#5416).
        assert_eq!(parse_rfc3339("2016-12-31T23:59:60Z"), None);
        assert_eq!(parse_rfc3339("2023-06-15T12:30:60Z"), None);
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

    // -- delayed rebind (§4) --------------------------------------------------

    const T0: u64 = 1_700_000_000;

    fn bind(service: &mut HostedBindingService, sub: &str, seed: u8) -> BindingRecord {
        signup(service, sub);
        service
            .complete_browser_ceremony(sub, &root_key(seed), None)
            .unwrap()
    }

    /// A fresh challenge for `key` on `sub`, signed by `key`: returns the
    /// challenge ID and the proof signature bytes.
    fn signed_challenge(
        service: &mut HostedBindingService,
        sub: &str,
        key: &SigningKey,
    ) -> (String, [u8; 64]) {
        let iroh_id = key.verifying_key().to_bytes();
        let challenge = service.request_challenge(sub, &iroh_id).unwrap();
        let signature = key.sign(&HostedBindingService::canonical_transcript(&challenge));
        (challenge.challenge_id, signature.to_bytes())
    }

    fn countersign(
        record_binding_id: &str,
        old_hex: &str,
        new_key: &SigningKey,
        via: &str,
    ) -> [u8; 64] {
        let probe = RebindRecord {
            rebind_id: String::new(),
            binding_id: record_binding_id.to_owned(),
            old_iroh_id: old_hex.to_owned(),
            new_iroh_id: hex_encode(&new_key.verifying_key().to_bytes()),
            effective_at: String::new(),
            via: via.to_owned(),
            service_signature: String::new(),
            new_key_countersignature: String::new(),
        };
        new_key.sign(&probe.countersign_bytes()).to_bytes()
    }

    #[test]
    fn delayed_rebind_request_pends_reserves_and_notifies() {
        let mut service = service();
        let binding = bind(&mut service, SUB, 21);
        let new_key = root_key(22);
        let (challenge_id, proof) = signed_challenge(&mut service, SUB, &new_key);
        let (request, notice) = service
            .request_delayed_rebind(SUB, &challenge_id, &proof)
            .unwrap();

        assert_eq!(request.status, RebindRequestStatus::Pending);
        assert_eq!(request.old_iroh_id, binding.iroh_id);
        assert_eq!(
            request.new_iroh_id,
            hex_encode(&new_key.verifying_key().to_bytes())
        );
        assert_eq!(
            parse_rfc3339(&request.effective_at),
            Some(T0 + REBIND_WAITING_PERIOD_SECS)
        );
        // The binding is pending, still keyed by the OLD key, re-attested.
        let stored = service.binding_for_sub(SUB).unwrap();
        assert_eq!(stored.status, BindingStatus::RebindPending);
        assert_eq!(stored.iroh_id, binding.iroh_id);
        assert!(service.verify_attestation(stored));
        // The notice names the new key fingerprint, the effective time,
        // and the cancellation method (§4.3).
        assert_eq!(notice.rebind_id, request.rebind_id);
        assert_eq!(notice.effective_at, request.effective_at);
        assert_eq!(
            notice.new_iroh_id_fingerprint,
            key_fingerprint(&request.new_iroh_id)
        );
        assert_eq!(notice.cancellation_method, REBIND_CANCELLATION_METHOD);
        // Bypass attempt: a fresh ceremony during the wait is a conflict,
        // never a shortcut around the delay (§4.2/§3.4).
        assert_eq!(
            service.complete_browser_ceremony(SUB, &root_key(23), None),
            Err(CeremonyError::RebindPending)
        );
        // A second delayed request is refused while one is pending.
        let (challenge_id, proof) = signed_challenge(&mut service, SUB, &new_key);
        assert_eq!(
            service.request_delayed_rebind(SUB, &challenge_id, &proof),
            Err(RebindError::RebindAlreadyPending)
        );
    }

    #[test]
    fn pending_rebind_target_cannot_be_bound_by_ordinary_ceremony() {
        let mut service = service();
        let binding = bind(&mut service, SUB, 21);
        signup(&mut service, OTHER_SUB);
        let new_key = root_key(22);
        let (challenge_id, proof) = signed_challenge(&mut service, SUB, &new_key);
        let (request, _) = service
            .request_delayed_rebind(SUB, &challenge_id, &proof)
            .unwrap();

        let (challenge_id, proof) = signed_challenge(&mut service, OTHER_SUB, &new_key);
        assert_eq!(
            service.submit_proof(OTHER_SUB, &challenge_id, &proof, None),
            Err(CeremonyError::IrohIdBoundElsewhere)
        );
        assert!(service.binding_for_sub(OTHER_SUB).is_none());
        assert_eq!(
            service.binding_by_iroh.get(&binding.iroh_id),
            Some(&binding.binding_id)
        );
        assert!(!service.binding_by_iroh.contains_key(&request.new_iroh_id));
        assert_eq!(
            service.rebind_request(&request.rebind_id).unwrap().status,
            RebindRequestStatus::Pending
        );
    }

    #[test]
    fn another_account_cannot_request_a_reserved_rebind_target() {
        let mut service = service();
        let first_binding = bind(&mut service, SUB, 21);
        let other_binding = bind(&mut service, OTHER_SUB, 23);
        let reserved_key = root_key(22);
        let (challenge_id, proof) = signed_challenge(&mut service, SUB, &reserved_key);
        let (first_request, _) = service
            .request_delayed_rebind(SUB, &challenge_id, &proof)
            .unwrap();

        let by_iroh_before = service.binding_by_iroh.clone();
        let by_sub_before = service.binding_by_sub.clone();
        let requests_before = service.rebind_requests.clone();
        let (challenge_id, proof) = signed_challenge(&mut service, OTHER_SUB, &reserved_key);
        assert_eq!(
            service.request_delayed_rebind(OTHER_SUB, &challenge_id, &proof),
            Err(RebindError::NewKeyBoundElsewhere)
        );
        assert_eq!(service.binding_by_iroh, by_iroh_before);
        assert_eq!(service.binding_by_sub, by_sub_before);
        assert_eq!(service.rebind_requests, requests_before);
        assert_eq!(service.binding_for_sub(OTHER_SUB), Some(&other_binding));
        assert_eq!(
            service.binding_for_sub(SUB).unwrap().binding_id,
            first_binding.binding_id
        );
        assert_eq!(
            service
                .rebind_request(&first_request.rebind_id)
                .unwrap()
                .status,
            RebindRequestStatus::Pending
        );
    }

    #[test]
    fn delayed_rebind_cancel_restores_active_and_restarts_full_wait() {
        let mut service = service();
        let binding = bind(&mut service, SUB, 21);
        let new_key = root_key(22);
        let (challenge_id, proof) = signed_challenge(&mut service, SUB, &new_key);
        let (request, _) = service
            .request_delayed_rebind(SUB, &challenge_id, &proof)
            .unwrap();

        // A signature from the NEW key (or anyone else) cannot cancel.
        let foreign = new_key.sign(&rebind_cancel_bytes(&request.rebind_id));
        assert_eq!(
            service.cancel_rebind(&request.rebind_id, &foreign.to_bytes()),
            Err(RebindError::BadOldKeySignature)
        );
        assert_eq!(
            service
                .pending_rebind_for_binding(&binding.binding_id)
                .unwrap()
                .status,
            RebindRequestStatus::Pending
        );

        let old_key = root_key(21);
        let cancel = old_key.sign(&rebind_cancel_bytes(&request.rebind_id));
        let restored = service
            .cancel_rebind(&request.rebind_id, &cancel.to_bytes())
            .unwrap();
        assert_eq!(restored.status, BindingStatus::Active);
        assert_eq!(restored.iroh_id, binding.iroh_id);
        assert!(service.verify_attestation(&restored));
        assert_eq!(
            service.rebind_request(&request.rebind_id).unwrap().status,
            RebindRequestStatus::Cancelled
        );
        // Cancelled is final: it can no longer take effect.
        service.set_now_override(Some(T0 + REBIND_WAITING_PERIOD_SECS));
        assert_eq!(
            service.effect_due_rebind(&request.rebind_id, &[0u8; 64]),
            Err(RebindError::RebindNotPending)
        );
        // A later request restarts the FULL waiting period (§4.3).
        let (challenge_id, proof) = signed_challenge(&mut service, SUB, &new_key);
        let (second, _) = service
            .request_delayed_rebind(SUB, &challenge_id, &proof)
            .unwrap();
        assert_eq!(
            parse_rfc3339(&second.effective_at),
            Some(T0 + 2 * REBIND_WAITING_PERIOD_SECS)
        );
    }

    #[test]
    fn delayed_rebind_effects_at_effective_at_with_signed_publication() {
        let mut service = service();
        let binding = bind(&mut service, SUB, 21);
        let new_key = root_key(22);
        let (challenge_id, proof) = signed_challenge(&mut service, SUB, &new_key);
        let (request, _) = service
            .request_delayed_rebind(SUB, &challenge_id, &proof)
            .unwrap();
        let counter = countersign(
            &binding.binding_id,
            &binding.iroh_id,
            &new_key,
            REBIND_VIA_DELAYED,
        );

        // Too early: nothing changes.
        assert_eq!(
            service.effect_due_rebind(&request.rebind_id, &counter),
            Err(RebindError::RebindNotYetEffective)
        );
        assert_eq!(
            service.binding_for_sub(SUB).unwrap().iroh_id,
            binding.iroh_id
        );
        // A bad countersignature at the due time also changes nothing.
        service.set_now_override(Some(T0 + REBIND_WAITING_PERIOD_SECS));
        assert_eq!(
            service.effect_due_rebind(&request.rebind_id, &[7u8; 64]),
            Err(RebindError::BadCountersignature)
        );

        let (updated, record) = service
            .effect_due_rebind(&request.rebind_id, &counter)
            .unwrap();
        assert_eq!(updated.status, BindingStatus::Active);
        assert_eq!(updated.iroh_id, request.new_iroh_id);
        assert!(service.verify_attestation(&updated));
        // History: the old key is appended, never deleted (§4.4 step 1).
        assert_eq!(updated.history.len(), 1);
        assert_eq!(updated.history[0].iroh_id, binding.iroh_id);
        assert_eq!(updated.history[0].via, REBIND_VIA_DELAYED);
        assert_eq!(
            updated.history[0].rebind_record_id.as_deref(),
            Some(request.rebind_id.as_str())
        );
        // Publication: the record verifies offline against the pinned
        // service key, and is retained in the published log.
        assert!(HostedBindingService::verify_rebind_record(
            &service.service_public_key(),
            &record
        ));
        assert_eq!(service.rebind_records(), std::slice::from_ref(&record));
        assert_eq!(
            service.rebind_request(&request.rebind_id).unwrap().status,
            RebindRequestStatus::Effected
        );
        // The old key is free again; the new key is the bound one.
        signup(&mut service, OTHER_SUB);
        service
            .complete_browser_ceremony(OTHER_SUB, &root_key(21), None)
            .unwrap();
    }

    #[test]
    fn delayed_rebind_effect_refuses_a_target_bound_elsewhere_without_mutation() {
        let mut service = service();
        let binding = bind(&mut service, SUB, 21);
        let other_binding = bind(&mut service, OTHER_SUB, 23);
        let new_key = root_key(22);
        let (challenge_id, proof) = signed_challenge(&mut service, SUB, &new_key);
        let (request, _) = service
            .request_delayed_rebind(SUB, &challenge_id, &proof)
            .unwrap();

        // Simulate a competing bind that raced with or predates this pending
        // request. Keep the foreign record and its by-key index consistent.
        let mut other_claim = other_binding.clone();
        other_claim.iroh_id = request.new_iroh_id.clone();
        other_claim.service_attestation = hex_encode(&service.attest_bytes(&other_claim).unwrap());
        service.binding_by_iroh.remove(&other_binding.iroh_id);
        service.binding_by_iroh.insert(
            request.new_iroh_id.clone(),
            other_binding.binding_id.clone(),
        );
        service
            .bindings
            .insert(other_claim.binding_id.clone(), other_claim.clone());

        let binding_before = service.binding(&binding.binding_id).unwrap().clone();
        let other_before = service.binding(&other_binding.binding_id).unwrap().clone();
        let by_sub_before = service.binding_by_sub.clone();
        let by_iroh_before = service.binding_by_iroh.clone();
        let request_before = service.rebind_request(&request.rebind_id).unwrap().clone();
        let published_before = service.rebind_records().to_vec();
        let counter = countersign(
            &binding.binding_id,
            &binding.iroh_id,
            &new_key,
            REBIND_VIA_DELAYED,
        );
        service.set_now_override(Some(T0 + REBIND_WAITING_PERIOD_SECS));

        assert_eq!(
            service.effect_due_rebind(&request.rebind_id, &counter),
            Err(RebindError::NewKeyBoundElsewhere)
        );
        assert_eq!(service.binding(&binding.binding_id), Some(&binding_before));
        assert_eq!(
            service.binding(&other_binding.binding_id),
            Some(&other_before)
        );
        assert_eq!(service.binding_by_sub, by_sub_before);
        assert_eq!(service.binding_by_iroh, by_iroh_before);
        assert_eq!(
            service.rebind_request(&request.rebind_id),
            Some(&request_before)
        );
        assert_eq!(service.rebind_records(), published_before);
    }

    #[test]
    fn delayed_rebind_effect_refuses_a_target_reserved_by_another_pending_request() {
        let mut service = service();
        let binding = bind(&mut service, SUB, 21);
        let other_binding = bind(&mut service, OTHER_SUB, 23);
        let new_key = root_key(22);
        let (challenge_id, proof) = signed_challenge(&mut service, SUB, &new_key);
        let (request, _) = service
            .request_delayed_rebind(SUB, &challenge_id, &proof)
            .unwrap();

        // Model a competing pending request that was already present in
        // retained state when this request reaches its effect time.
        let mut other_pending_binding = other_binding.clone();
        other_pending_binding.status = BindingStatus::RebindPending;
        other_pending_binding.service_attestation =
            hex_encode(&service.attest_bytes(&other_pending_binding).unwrap());
        service.bindings.insert(
            other_pending_binding.binding_id.clone(),
            other_pending_binding.clone(),
        );
        let other_request = RebindRequest {
            rebind_id: "earlier-pending-request".to_owned(),
            binding_id: other_binding.binding_id.clone(),
            sub: OTHER_SUB.to_owned(),
            old_iroh_id: other_binding.iroh_id.clone(),
            new_iroh_id: request.new_iroh_id.clone(),
            requested_at: request.requested_at.clone(),
            effective_at: request.effective_at.clone(),
            status: RebindRequestStatus::Pending,
        };
        service
            .rebind_requests
            .insert(other_request.rebind_id.clone(), other_request.clone());

        let binding_before = service.binding(&binding.binding_id).unwrap().clone();
        let other_before = service.binding(&other_binding.binding_id).unwrap().clone();
        let by_sub_before = service.binding_by_sub.clone();
        let by_iroh_before = service.binding_by_iroh.clone();
        let request_before = service.rebind_request(&request.rebind_id).unwrap().clone();
        let other_request_before = service
            .rebind_request(&other_request.rebind_id)
            .unwrap()
            .clone();
        let counter = countersign(
            &binding.binding_id,
            &binding.iroh_id,
            &new_key,
            REBIND_VIA_DELAYED,
        );
        service.set_now_override(Some(T0 + REBIND_WAITING_PERIOD_SECS));

        assert_eq!(
            service.effect_due_rebind(&request.rebind_id, &counter),
            Err(RebindError::NewKeyBoundElsewhere)
        );
        assert_eq!(service.binding(&binding.binding_id), Some(&binding_before));
        assert_eq!(
            service.binding(&other_binding.binding_id),
            Some(&other_before)
        );
        assert_eq!(service.binding_by_sub, by_sub_before);
        assert_eq!(service.binding_by_iroh, by_iroh_before);
        assert_eq!(
            service.rebind_request(&request.rebind_id),
            Some(&request_before)
        );
        assert_eq!(
            service.rebind_request(&other_request.rebind_id),
            Some(&other_request_before)
        );
        assert!(service.rebind_records().is_empty());
    }

    #[test]
    fn apply_rebind_refuses_to_overwrite_another_bindings_key_index() {
        let mut service = service();
        let binding = bind(&mut service, SUB, 21);
        let other_binding = bind(&mut service, OTHER_SUB, 23);
        let record = RebindRecord {
            rebind_id: "conflicting-rebind".to_owned(),
            binding_id: binding.binding_id.clone(),
            old_iroh_id: binding.iroh_id.clone(),
            new_iroh_id: other_binding.iroh_id.clone(),
            effective_at: unix_secs_to_rfc3339(T0),
            via: REBIND_VIA_OLD_KEY.to_owned(),
            service_signature: String::new(),
            new_key_countersignature: String::new(),
        };
        let binding_before = service.binding(&binding.binding_id).unwrap().clone();
        let other_before = service.binding(&other_binding.binding_id).unwrap().clone();
        let by_sub_before = service.binding_by_sub.clone();
        let by_iroh_before = service.binding_by_iroh.clone();

        assert_eq!(
            service.apply_rebind(&record),
            Err(RebindError::NewKeyBoundElsewhere)
        );
        assert_eq!(service.binding(&binding.binding_id), Some(&binding_before));
        assert_eq!(
            service.binding(&other_binding.binding_id),
            Some(&other_before)
        );
        assert_eq!(service.binding_by_sub, by_sub_before);
        assert_eq!(service.binding_by_iroh, by_iroh_before);
        assert!(service.rebind_records().is_empty());
    }

    #[test]
    fn fast_path_rebind_is_immediate_and_rejects_bad_authorization() {
        let mut service = service();
        let binding = bind(&mut service, SUB, 21);
        let old_key = root_key(21);
        let new_key = root_key(22);

        // The NEW key cannot authorize its own takeover.
        let (challenge_id, proof) = signed_challenge(&mut service, SUB, &new_key);
        let forged = new_key.sign(&rebind_authorization_bytes(
            &binding.binding_id,
            &new_key.verifying_key().to_bytes(),
            &challenge_id,
        ));
        let counter = countersign(
            &binding.binding_id,
            &binding.iroh_id,
            &new_key,
            REBIND_VIA_OLD_KEY,
        );
        assert_eq!(
            service.rebind_with_old_key_authorization(
                SUB,
                None,
                &challenge_id,
                &proof,
                &forged.to_bytes(),
                &counter,
            ),
            Err(RebindError::BadOldKeySignature)
        );
        assert_eq!(
            service.binding_for_sub(SUB).unwrap().iroh_id,
            binding.iroh_id
        );

        let (challenge_id, proof) = signed_challenge(&mut service, SUB, &new_key);
        let authorization = old_key.sign(&rebind_authorization_bytes(
            &binding.binding_id,
            &new_key.verifying_key().to_bytes(),
            &challenge_id,
        ));
        let (updated, record) = service
            .rebind_with_old_key_authorization(
                SUB,
                None,
                &challenge_id,
                &proof,
                &authorization.to_bytes(),
                &counter,
            )
            .unwrap();
        // Immediate: no waiting period on the fast path (§4.1).
        assert_eq!(updated.status, BindingStatus::Active);
        assert_eq!(updated.iroh_id, record.new_iroh_id);
        assert_eq!(record.via, REBIND_VIA_OLD_KEY);
        assert_eq!(record.effective_at, unix_secs_to_rfc3339(T0));
        assert_eq!(updated.history[0].via, REBIND_VIA_OLD_KEY);
        assert_eq!(updated.history[0].rebind_record_id, None);
        assert!(HostedBindingService::verify_rebind_record(
            &service.service_public_key(),
            &record
        ));
    }

    #[test]
    fn fast_path_rebind_cannot_take_a_pending_request_target() {
        let mut service = service();
        let binding = bind(&mut service, SUB, 21);
        let other_binding = bind(&mut service, OTHER_SUB, 23);
        let reserved_key = root_key(22);
        let (challenge_id, proof) = signed_challenge(&mut service, SUB, &reserved_key);
        let (request, _) = service
            .request_delayed_rebind(SUB, &challenge_id, &proof)
            .unwrap();

        let (challenge_id, proof) = signed_challenge(&mut service, OTHER_SUB, &reserved_key);
        let target_id = reserved_key.verifying_key().to_bytes();
        let authorization = root_key(23).sign(&rebind_authorization_bytes(
            &other_binding.binding_id,
            &target_id,
            &challenge_id,
        ));
        let counter = countersign(
            &other_binding.binding_id,
            &other_binding.iroh_id,
            &reserved_key,
            REBIND_VIA_OLD_KEY,
        );
        let by_sub_before = service.binding_by_sub.clone();
        let by_iroh_before = service.binding_by_iroh.clone();

        assert_eq!(
            service.rebind_with_old_key_authorization(
                OTHER_SUB,
                None,
                &challenge_id,
                &proof,
                &authorization.to_bytes(),
                &counter,
            ),
            Err(RebindError::NewKeyBoundElsewhere)
        );
        assert_eq!(
            service.binding(&other_binding.binding_id),
            Some(&other_binding)
        );
        assert_eq!(
            service.binding_for_sub(SUB).unwrap().binding_id,
            binding.binding_id
        );
        assert_eq!(service.binding_by_sub, by_sub_before);
        assert_eq!(service.binding_by_iroh, by_iroh_before);
        assert_eq!(
            service.rebind_request(&request.rebind_id).unwrap().status,
            RebindRequestStatus::Pending
        );
        assert!(service.rebind_records().is_empty());
    }

    #[test]
    fn approve_early_converts_pending_delayed_rebind() {
        let mut service = service();
        let binding = bind(&mut service, SUB, 21);
        let old_key = root_key(21);
        let new_key = root_key(22);
        let (challenge_id, proof) = signed_challenge(&mut service, SUB, &new_key);
        let (request, _) = service
            .request_delayed_rebind(SUB, &challenge_id, &proof)
            .unwrap();

        let (challenge_id, proof) = signed_challenge(&mut service, SUB, &new_key);
        let authorization = old_key.sign(&rebind_authorization_bytes(
            &binding.binding_id,
            &new_key.verifying_key().to_bytes(),
            &challenge_id,
        ));
        let counter = countersign(
            &binding.binding_id,
            &binding.iroh_id,
            &new_key,
            REBIND_VIA_OLD_KEY,
        );
        let (updated, record) = service
            .rebind_with_old_key_authorization(
                SUB,
                Some(&request.rebind_id),
                &challenge_id,
                &proof,
                &authorization.to_bytes(),
                &counter,
            )
            .unwrap();
        // Converted to an immediate rebind long before effective_at (§4.3).
        assert_eq!(updated.iroh_id, request.new_iroh_id);
        assert_eq!(record.rebind_id, request.rebind_id);
        assert_eq!(record.via, REBIND_VIA_OLD_KEY);
        assert_eq!(
            service.rebind_request(&request.rebind_id).unwrap().status,
            RebindRequestStatus::Effected
        );
    }

    #[test]
    fn delayed_rebind_request_rejects_expired_challenge() {
        let mut service = service();
        bind(&mut service, SUB, 21);
        let new_key = root_key(22);
        let (challenge_id, proof) = signed_challenge(&mut service, SUB, &new_key);
        service.set_now_override(Some(T0 + CHALLENGE_TTL_SECS));
        assert_eq!(
            service.request_delayed_rebind(SUB, &challenge_id, &proof),
            Err(RebindError::ChallengeExpired)
        );
        // Nothing pends and nothing changed.
        let stored = service.binding_for_sub(SUB).unwrap();
        assert_eq!(stored.status, BindingStatus::Active);
        assert!(
            service
                .pending_rebind_for_binding(&stored.binding_id)
                .is_none()
        );
    }

    #[test]
    fn expired_rebind_challenge_is_not_recorded_as_consumed() {
        let mut service = service();
        bind(&mut service, SUB, 21);
        let new_key = root_key(22);
        let (challenge_id, proof) = signed_challenge(&mut service, SUB, &new_key);
        service.set_now_override(Some(T0 + CHALLENGE_TTL_SECS));
        for _ in 0..2 {
            assert_eq!(
                service.request_delayed_rebind(SUB, &challenge_id, &proof),
                Err(RebindError::ChallengeExpired)
            );
        }
    }
}
