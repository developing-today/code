//! Signed transport advertisements (#8214, implementing the #5793
//! decision record `doc/2026-11-10_rendezvous_discovery_udp_denied_design.md`).
//!
//! A server publishes exactly one current [`TransportAdvertisement`]:
//! a signed object naming every transport path a UDP-denied client
//! may use to reach it. Substrates (last-known-good cache, the
//! `.well-known` HTTPS endpoint, the Workers rendezvous, DNS hints)
//! are untrusted transports for the object — the client trusts only
//! the Ed25519 signature against its pinned subject identity
//! (record Decision 1). This module is slice 1 of #8214: the type,
//! the domain-separated canonical encoding, sign/verify, the
//! freshest-valid-wins selection rule (Decisions 1+3), and the
//! durable sequence counter (invariant 8). Slice 2 added the
//! last-known-good cache and substrate resolution; slice 4 adds the
//! server side: `AdMinter` (minting on transport change and per
//! TTL, Decision 3), whose current bytes the `.well-known` handler
//! in id-web serves byte-for-byte. Slice 3 adds the fetch side:
//! the well-known and Workers-rendezvous HTTPS substrates bound
//! to an injected fetch closure (no HTTP dependency in id-core),
//! DNS bootstrap-hint parsing, and background revalidation.
//!
//! The oracle for the tests below is the decision record itself:
//! invariants 1–3 and 8, and the #8214 acceptance tests
//! (hostile-substrate, disagreement, expired-ad, round-trip).

use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::hex::hex_encode;

/// Domain separator prefixed to the canonical encoding before
/// signing (record Decision 1).
pub const AD_DOMAIN: &[u8] = b"id-transport-advertisement/v1";
/// Maximum ad lifetime: TTL ≤ 24 h (record Decision 1).
pub const MAX_TTL_SECONDS: u64 = 24 * 60 * 60;
/// Clients accept `issued_at` up to 5 minutes in the future
/// (record Decision 3, clock skew).
pub const CLOCK_SKEW_SECONDS: u64 = 5 * 60;

/// One usable transport path named by an advertisement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TransportKind {
    Udp,
    RelayUdp,
    Tcp,
    HttpConnect,
    Websocket,
}

impl TransportKind {
    fn as_str(self) -> &'static str {
        match self {
            TransportKind::Udp => "udp",
            TransportKind::RelayUdp => "relay-udp",
            TransportKind::Tcp => "tcp",
            TransportKind::HttpConnect => "http-connect",
            TransportKind::Websocket => "websocket",
        }
    }
}

/// One entry of [`TransportAdvertisement::transports`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransportEntry {
    pub kind: TransportKind,
    /// `host:port` or an https URL, per kind.
    pub endpoint: String,
    /// Lower = try earlier, consistent with the #5671 fallback order.
    pub priority: u32,
    /// Relay kinds only: the broker's own identity fingerprint, so
    /// the client authenticates the broker from the ad, not from
    /// the substrate (record Decision 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub broker: Option<String>,
}

/// The unsigned advertisement (v1). `subject` is the server's ID
/// identity fingerprint: the lower-case hex of its Ed25519 identity
/// public key — the same key a paired client already pins; no new
/// key type is introduced (record Decision 1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransportAdvertisement {
    pub subject: String,
    pub transports: Vec<TransportEntry>,
    /// Unix seconds.
    pub issued_at: u64,
    /// Unix seconds; `expires_at - issued_at` must be ≤ 24 h.
    pub expires_at: u64,
    /// Strictly increasing per subject; the server persists it
    /// beside the identity key (see [`SequenceFile`]).
    pub sequence: u64,
}

/// An advertisement plus its Ed25519 signature (lower-case hex).
/// This is the byte object substrates store and serve opaquely.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedAdvertisement {
    #[serde(flatten)]
    pub advertisement: TransportAdvertisement,
    pub signature: String,
}

/// Why an advertisement was rejected. Verification fails closed:
/// any error means the client never acts on the ad (invariant 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdError {
    /// The ad's subject is not the identity the client pinned.
    SubjectMismatch,
    /// The subject is not a 32-byte Ed25519 public key in hex.
    BadSubjectKey,
    /// The signature is not 64 bytes in hex.
    BadSignatureEncoding,
    /// The signature does not verify over the canonical encoding.
    SignatureInvalid,
    /// `expires_at` is not after `issued_at`, or the TTL exceeds 24 h.
    TtlInvalid,
}

impl std::fmt::Display for AdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = match self {
            AdError::SubjectMismatch => "advertisement subject does not match the pinned identity",
            AdError::BadSubjectKey => "advertisement subject is not a hex Ed25519 public key",
            AdError::BadSignatureEncoding => "advertisement signature is not hex Ed25519",
            AdError::SignatureInvalid => "advertisement signature does not verify",
            AdError::TtlInvalid => "advertisement TTL is empty or exceeds 24h",
        };
        f.write_str(msg)
    }
}

impl std::error::Error for AdError {}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(s.len() / 2);
    let nibble = |c: u8| -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    };
    for pair in bytes.chunks_exact(2) {
        out.push((nibble(pair[0])? << 4) | nibble(pair[1])?);
    }
    Some(out)
}

// Serializing a string slice with serde_json cannot fail: a string has
// no custom Serialize impl and the output is an in-memory String, so the
// expect in this helper is provably unreachable. The serializer is kept
// rather than a hand-rolled escaper because these bytes are canonical
// signing input and must stay byte-identical to serde_json escaping.
#[allow(clippy::expect_used)]
fn json_string(s: &str) -> String {
    serde_json::to_string(s).expect("serializing a string cannot fail")
}

impl TransportAdvertisement {
    /// The subject fingerprint for an identity verifying key.
    pub fn subject_for(key: &VerifyingKey) -> String {
        hex_encode(key.as_bytes())
    }

    /// Canonical encoding: JSON with object keys sorted, UTF-8, no
    /// insignificant whitespace (record Decision 1). Built field by
    /// field in sorted-key order so the bytes do not depend on any
    /// serializer's map-ordering feature flags.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = String::new();
        out.push('{');
        out.push_str("\"expires_at\":");
        out.push_str(&self.expires_at.to_string());
        out.push_str(",\"issued_at\":");
        out.push_str(&self.issued_at.to_string());
        out.push_str(",\"sequence\":");
        out.push_str(&self.sequence.to_string());
        out.push_str(",\"subject\":");
        out.push_str(&json_string(&self.subject));
        out.push_str(",\"transports\":[");
        for (i, t) in self.transports.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push('{');
            if let Some(broker) = &t.broker {
                out.push_str("\"broker\":");
                out.push_str(&json_string(broker));
                out.push(',');
            }
            out.push_str("\"endpoint\":");
            out.push_str(&json_string(&t.endpoint));
            out.push_str(",\"kind\":");
            out.push_str(&json_string(t.kind.as_str()));
            out.push_str(",\"priority\":");
            out.push_str(&t.priority.to_string());
            out.push('}');
        }
        out.push_str("]}");
        out.into_bytes()
    }

    /// The exact bytes the signature covers: the domain separator
    /// followed by the canonical encoding.
    pub fn signing_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(AD_DOMAIN.len() + 256);
        out.extend_from_slice(AD_DOMAIN);
        out.extend_from_slice(&self.canonical_bytes());
        out
    }

    /// Sign with the subject identity key.
    pub fn sign(&self, key: &SigningKey) -> SignedAdvertisement {
        let signature = key.sign(&self.signing_bytes());
        SignedAdvertisement {
            advertisement: self.clone(),
            signature: hex_encode(&signature.to_bytes()),
        }
    }

    /// Transports in the order a client should try them
    /// (priority ascending; stable for equal priorities).
    pub fn ordered_transports(&self) -> Vec<&TransportEntry> {
        let mut entries: Vec<&TransportEntry> = self.transports.iter().collect();
        entries.sort_by_key(|t| t.priority);
        entries
    }

    fn is_fresh(&self, now: u64) -> bool {
        self.expires_at > now && self.issued_at <= now.saturating_add(CLOCK_SKEW_SECONDS)
    }
}

impl SignedAdvertisement {
    /// Verify the signature against the pinned subject identity
    /// and the record's structural rules (TTL ≤ 24 h). Freshness
    /// is *not* checked here — an expired-but-valid ad is still a
    /// last-known-good candidate (Decision 3); [`select_freshest`]
    /// owns that distinction.
    pub fn verify(&self, pinned_subject: &str) -> Result<&TransportAdvertisement, AdError> {
        let ad = &self.advertisement;
        if ad.subject != pinned_subject {
            return Err(AdError::SubjectMismatch);
        }
        let key_bytes: [u8; 32] = hex_decode(&ad.subject)
            .and_then(|v| v.try_into().ok())
            .ok_or(AdError::BadSubjectKey)?;
        let key = VerifyingKey::from_bytes(&key_bytes).map_err(|_| AdError::BadSubjectKey)?;
        let sig_bytes: [u8; 64] = hex_decode(&self.signature)
            .and_then(|v| v.try_into().ok())
            .ok_or(AdError::BadSignatureEncoding)?;
        let signature = Signature::from_bytes(&sig_bytes);
        key.verify_strict(&ad.signing_bytes(), &signature)
            .map_err(|_| AdError::SignatureInvalid)?;
        if ad.expires_at <= ad.issued_at || ad.expires_at - ad.issued_at > MAX_TTL_SECONDS {
            return Err(AdError::TtlInvalid);
        }
        Ok(ad)
    }
}

/// The outcome of freshest-valid-wins selection across substrates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection<'a> {
    pub advertisement: &'a TransportAdvertisement,
    /// True when the winner is an expired ad used as
    /// last-known-good (record Decision 3): the connection must be
    /// flagged degraded and revalidated in the background.
    pub degraded: bool,
}

/// Freshest valid wins (record Decisions 1+3): among the ads that
/// verify against `pinned_subject`, a fresh ad always beats an
/// expired one; within each class the highest `sequence` wins and
/// ties keep the first seen (a same-sequence rival is discarded).
/// Invalid ads — forged, altered, or for another subject — are
/// ignored entirely: a hostile substrate fails closed.
pub fn select_freshest<'a>(
    ads: &'a [SignedAdvertisement],
    pinned_subject: &str,
    now: u64,
) -> Option<Selection<'a>> {
    let mut fresh: Option<&TransportAdvertisement> = None;
    let mut stale: Option<&TransportAdvertisement> = None;
    for signed in ads {
        let Ok(ad) = signed.verify(pinned_subject) else {
            continue;
        };
        let slot = if ad.is_fresh(now) {
            &mut fresh
        } else {
            &mut stale
        };
        if slot.is_none_or(|cur| ad.sequence > cur.sequence) {
            *slot = Some(ad);
        }
    }
    if let Some(advertisement) = fresh {
        Some(Selection {
            advertisement,
            degraded: false,
        })
    } else {
        stale.map(|advertisement| Selection {
            advertisement,
            degraded: true,
        })
    }
}

/// Durable per-subject sequence counter (invariant 8): persists
/// the last issued sequence beside the identity key so a restart
/// never reissues a lower sequence. The file holds the last issued
/// sequence as bare decimal digits; writes are atomic
/// (write-temp-then-rename in the same directory).
#[derive(Debug, Clone)]
pub struct SequenceFile {
    path: std::path::PathBuf,
}

impl SequenceFile {
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The last issued sequence, or 0 when none was ever issued.
    pub fn last_issued(&self) -> std::io::Result<u64> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => text
                .trim()
                .parse::<u64>()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
            Err(e) => Err(e),
        }
    }

    /// Issue the next sequence, durably. Monotonic across restarts
    /// because the value is read back from disk on every call.
    pub fn next_sequence(&self) -> std::io::Result<u64> {
        let next = self.last_issued()?.saturating_add(1).max(1);
        let tmp = self.path.with_extension("tmp");
        crate::durable_write::write_durable(&self.path, &tmp, next.to_string().as_bytes())?;
        Ok(next)
    }
}

/// The TTL the record recommends for minted ads (Decision 1:
/// TTL <= 24 h, 1 h RECOMMENDED).
pub const RECOMMENDED_TTL_SECONDS: u64 = 60 * 60;

/// Server-side minter (#8214 slice 4): owns the subject identity
/// key and the durable SequenceFile beside it, and produces the
/// one current signed advertisement the server publishes (Decision
/// 1). Minting follows Decision 3: a new ad (sequence + 1) is
/// minted on any transport change and at least once per TTL;
/// between those events the current ad's bytes are stable, so
/// every substrate (the .well-known handler, a broker mirror, the
/// Workers push) serves the identical object byte-for-byte
/// (Decision 2 parity).
pub struct AdMinter {
    key: SigningKey,
    sequence: SequenceFile,
    ttl_seconds: u64,
    current: std::sync::Mutex<Option<MintedAd>>,
}

struct MintedAd {
    transports: Vec<TransportEntry>,
    signed: SignedAdvertisement,
    /// The exact bytes substrates serve: the serde JSON of the
    /// SignedAdvertisement, captured at mint time so a later
    /// serializer change can never alter what an already-minted
    /// ad serves as.
    bytes: Vec<u8>,
}

impl AdMinter {
    /// A minter for the given identity key, issuing sequences from
    /// the counter file that lives beside the identity key. The
    /// TTL is clamped to the record's 24 h maximum.
    pub fn new(key: SigningKey, sequence: SequenceFile, ttl_seconds: u64) -> Self {
        Self {
            key,
            sequence,
            ttl_seconds: ttl_seconds.clamp(1, MAX_TTL_SECONDS),
            current: std::sync::Mutex::new(None),
        }
    }

    /// The subject fingerprint this minter signs for.
    pub fn subject(&self) -> String {
        TransportAdvertisement::subject_for(&self.key.verifying_key())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<MintedAd>> {
        self.current
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn mint_locked(
        &self,
        slot: &mut Option<MintedAd>,
        transports: Vec<TransportEntry>,
        now: u64,
    ) -> std::io::Result<Vec<u8>> {
        let sequence = self.sequence.next_sequence()?;
        let ad = TransportAdvertisement {
            subject: self.subject(),
            transports,
            issued_at: now,
            expires_at: now.saturating_add(self.ttl_seconds),
            sequence,
        };
        let signed = ad.sign(&self.key);
        let bytes = serde_json::to_vec(&signed).map_err(std::io::Error::other)?;
        *slot = Some(MintedAd {
            transports: ad.transports.clone(),
            signed,
            bytes: bytes.clone(),
        });
        Ok(bytes)
    }

    /// Mint a new ad over the transports unconditionally
    /// (sequence + 1) and return the bytes substrates should
    /// serve.
    pub fn mint(&self, transports: Vec<TransportEntry>, now: u64) -> std::io::Result<Vec<u8>> {
        let mut slot = self.lock();
        self.mint_locked(&mut slot, transports, now)
    }

    /// The Decision 3 refresh: mint only when there is no current
    /// ad, the transport set changed, or the current ad has
    /// expired; otherwise return the current bytes unchanged (a
    /// no-op refresh consumes no sequence).
    pub fn refresh(&self, transports: Vec<TransportEntry>, now: u64) -> std::io::Result<Vec<u8>> {
        let mut slot = self.lock();
        let stale = match &*slot {
            None => true,
            Some(minted) => {
                minted.transports != transports || minted.signed.advertisement.expires_at <= now
            }
        };
        if stale {
            return self.mint_locked(&mut slot, transports, now);
        }
        Ok(slot.as_ref().map(|m| m.bytes.clone()).unwrap_or_default())
    }

    /// The bytes to serve right now, or None when nothing was
    /// ever minted. An expired current ad is re-minted in place
    /// over its last transport set (the per-TTL mint Decision 3
    /// requires), so a server that is asked always answers with
    /// a fresh ad.
    pub fn current_bytes(&self, now: u64) -> std::io::Result<Option<Vec<u8>>> {
        let mut slot = self.lock();
        let expired = matches!(
            &*slot,
            Some(minted) if minted.signed.advertisement.expires_at <= now
        );
        if expired {
            let transports = slot
                .as_ref()
                .map(|m| m.transports.clone())
                .unwrap_or_default();
            return self.mint_locked(&mut slot, transports, now).map(Some);
        }
        Ok(slot.as_ref().map(|m| m.bytes.clone()))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    // Oracle: doc/2026-11-10_rendezvous_discovery_udp_denied_design.md
    // (the #5793 decision record), invariants 1-3 and 8.

    fn server_key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    fn attacker_key() -> SigningKey {
        SigningKey::from_bytes(&[9u8; 32])
    }

    fn ad(key: &SigningKey, sequence: u64, issued_at: u64, ttl: u64) -> TransportAdvertisement {
        TransportAdvertisement {
            subject: TransportAdvertisement::subject_for(&key.verifying_key()),
            transports: vec![
                TransportEntry {
                    kind: TransportKind::Tcp,
                    endpoint: "203.0.113.7:443".to_string(),
                    priority: 30,
                    broker: None,
                },
                TransportEntry {
                    kind: TransportKind::Udp,
                    endpoint: "203.0.113.7:51820".to_string(),
                    priority: 10,
                    broker: None,
                },
                TransportEntry {
                    kind: TransportKind::RelayUdp,
                    endpoint: "relay.example:51820".to_string(),
                    priority: 20,
                    broker: Some("broker-fingerprint".to_string()),
                },
            ],
            issued_at,
            expires_at: issued_at + ttl,
            sequence,
        }
    }

    const NOW: u64 = 1_700_000_000;

    #[test]
    fn round_trip_sign_verify_and_priority_order() {
        let key = server_key();
        let ad = ad(&key, 1, NOW, 3600);
        let signed = ad.sign(&key);
        let verified = signed
            .verify(&TransportAdvertisement::subject_for(&key.verifying_key()))
            .expect("a freshly signed ad verifies against its subject");
        assert_eq!(verified.sequence, 1);
        // Parent #5671 fallback order: udp, then relay-udp, then tcp.
        let kinds: Vec<TransportKind> = verified
            .ordered_transports()
            .iter()
            .map(|t| t.kind)
            .collect();
        assert_eq!(
            kinds,
            vec![
                TransportKind::Udp,
                TransportKind::RelayUdp,
                TransportKind::Tcp
            ]
        );
    }

    #[test]
    fn canonical_bytes_are_key_sorted_and_stable() {
        let key = server_key();
        let ad = ad(&key, 3, NOW, 3600);
        let bytes = ad.canonical_bytes();
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(text.starts_with("{\"expires_at\":"));
        assert_eq!(bytes, ad.canonical_bytes(), "encoding is deterministic");
        // Signing bytes carry the domain separator.
        assert!(ad.signing_bytes().starts_with(AD_DOMAIN));
    }

    #[test]
    fn hostile_substrate_forged_signature_fails_closed() {
        // Invariant 1/2: an attacker who knows the victim's subject
        // (it is public) still cannot forge an ad for it.
        let victim = server_key();
        let subject = TransportAdvertisement::subject_for(&victim.verifying_key());
        let mut forged = ad(&victim, 1, NOW, 3600);
        forged.subject = subject.clone();
        let forged = forged.sign(&attacker_key());
        assert_eq!(forged.verify(&subject), Err(AdError::SignatureInvalid));
        assert!(select_freshest(&[forged], &subject, NOW).is_none());
    }

    #[test]
    fn hostile_substrate_altered_transports_fail_closed() {
        // Invariant 2: altering the transport set after signing
        // invalidates the ad; the client never acts on it.
        let key = server_key();
        let subject = TransportAdvertisement::subject_for(&key.verifying_key());
        let mut signed = ad(&key, 1, NOW, 3600).sign(&key);
        signed.advertisement.transports[0].endpoint = "198.51.100.99:443".to_string();
        assert_eq!(signed.verify(&subject), Err(AdError::SignatureInvalid));
        assert!(select_freshest(&[signed], &subject, NOW).is_none());
    }

    #[test]
    fn hostile_substrate_replayed_older_sequence_never_displaces() {
        // Invariant 3: a replayed older ad never displaces a
        // fresher cached one, even if unexpired.
        let key = server_key();
        let subject = TransportAdvertisement::subject_for(&key.verifying_key());
        let fresh = ad(&key, 7, NOW, 3600).sign(&key);
        let replayed = ad(&key, 3, NOW, 3600).sign(&key);
        for ads in [&[fresh.clone(), replayed.clone()], &[replayed, fresh]] {
            let sel = select_freshest(ads, &subject, NOW).expect("a valid ad exists");
            assert_eq!(sel.advertisement.sequence, 7);
            assert!(!sel.degraded);
        }
    }

    #[test]
    fn disagreement_highest_sequence_wins_regardless_of_fetch_order() {
        // Decision 2 disagreement rule: no substrate outranks
        // another; the highest valid sequence wins.
        let key = server_key();
        let subject = TransportAdvertisement::subject_for(&key.verifying_key());
        let a = ad(&key, 4, NOW, 3600).sign(&key);
        let b = ad(&key, 6, NOW, 3600).sign(&key);
        for ads in [&[a.clone(), b.clone()], &[b, a]] {
            let sel = select_freshest(ads, &subject, NOW).expect("a valid ad exists");
            assert_eq!(sel.advertisement.sequence, 6);
        }
    }

    #[test]
    fn expired_ad_is_degraded_last_known_good_only() {
        // Decision 3 / invariant 4: an expired valid ad is used
        // only as flagged-degraded last-known-good, and any fresh
        // ad beats it.
        let key = server_key();
        let subject = TransportAdvertisement::subject_for(&key.verifying_key());
        let expired = ad(&key, 9, NOW - 7200, 3600).sign(&key);
        let only_expired = [expired.clone()];
        let sel = select_freshest(&only_expired, &subject, NOW).expect("LKG exists");
        assert!(sel.degraded);
        assert_eq!(sel.advertisement.sequence, 9);

        let fresh = ad(&key, 5, NOW, 3600).sign(&key);
        let both = [expired, fresh];
        let sel = select_freshest(&both, &subject, NOW).expect("fresh ad exists");
        assert!(!sel.degraded);
        assert_eq!(sel.advertisement.sequence, 5);
    }

    #[test]
    fn wrong_subject_and_overlong_ttl_are_rejected() {
        let key = server_key();
        let other = attacker_key();
        let other_subject = TransportAdvertisement::subject_for(&other.verifying_key());
        let signed = ad(&key, 1, NOW, 3600).sign(&key);
        assert_eq!(signed.verify(&other_subject), Err(AdError::SubjectMismatch));

        let long_ttl = ad(&key, 1, NOW, MAX_TTL_SECONDS + 1).sign(&key);
        let subject = TransportAdvertisement::subject_for(&key.verifying_key());
        assert_eq!(long_ttl.verify(&subject), Err(AdError::TtlInvalid));
    }

    #[test]
    fn sequence_file_is_monotonic_across_reloads() {
        // Invariant 8: the counter persists beside the identity key
        // and never regresses across restarts.
        let dir =
            std::env::temp_dir().join(format!("id-transport-ad-seq-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ad-sequence");
        let _ = std::fs::remove_file(&path);
        let counter = SequenceFile::new(path.clone());
        assert_eq!(counter.last_issued().unwrap(), 0);
        assert_eq!(counter.next_sequence().unwrap(), 1);
        assert_eq!(counter.next_sequence().unwrap(), 2);
        // A "restart": a fresh handle on the same file continues up.
        let restarted = SequenceFile::new(path);
        assert_eq!(restarted.last_issued().unwrap(), 2);
        assert_eq!(restarted.next_sequence().unwrap(), 3);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

/// Last-known-good cache for one pinned subject (record Decision 3).
/// Holds at most one ad — the freshest valid one ever accepted —
/// and never lets a replayed or invalid ad displace it.
#[derive(Debug, Clone, Default)]
pub struct AdCache {
    current: Option<SignedAdvertisement>,
}

impl AdCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Offer an ad fetched from any substrate. Returns true when
    /// the ad became the cached one: it must verify against
    /// `pinned_subject` and carry a strictly higher sequence than
    /// the cached ad (invariant 3 — a replay never displaces).
    pub fn accept(&mut self, signed: SignedAdvertisement, pinned_subject: &str) -> bool {
        let Ok(ad) = signed.verify(pinned_subject) else {
            return false;
        };
        if let Some(cur) = &self.current {
            if ad.sequence <= cur.advertisement.sequence {
                return false;
            }
        }
        self.current = Some(signed);
        true
    }

    /// The cached ad as a selection at `now`: fresh, or
    /// flagged-degraded when past expiry (Decision 3).
    pub fn current(&self, now: u64) -> Option<Selection<'_>> {
        let signed = self.current.as_ref()?;
        let ad = &signed.advertisement;
        Some(Selection {
            advertisement: ad,
            degraded: !ad.is_fresh(now),
        })
    }

    /// The cached sequence, or 0 when the cache is empty.
    pub fn cached_sequence(&self) -> u64 {
        self.current
            .as_ref()
            .map_or(0, |s| s.advertisement.sequence)
    }
}

/// A source of signed advertisements: the `.well-known` HTTPS
/// endpoint, the Workers rendezvous, or a broker mirror. Substrates
/// are untrusted (Decision 1): an implementation may return forged,
/// stale, or no data, and resolution stays correct. The HTTPS
/// implementations live with the client's HTTP stack; this trait
/// is the seam they bind to.
pub trait Substrate {
    /// Fetch the substrate's current ad for `subject`, if it has
    /// one. `None` means unreachable or empty — never an error
    /// worth failing resolution over.
    fn fetch(&self, subject: &str) -> Option<SignedAdvertisement>;
}

/// Resolve a subject's advertisement (record Decision 2): consult
/// the cache first, then every substrate, and let the signed
/// object decide — freshest valid wins across all sources, with
/// the cache's last-known-good as the degraded fallback. Every
/// valid ad seen is folded into the cache, so a fresher ad from
/// any substrate upgrades later resolutions. Returns the winning
/// selection (borrowed from the cache) plus whether any substrate
/// answered at all.
pub fn resolve<'c>(
    cache: &'c mut AdCache,
    substrates: &[&dyn Substrate],
    pinned_subject: &str,
    now: u64,
) -> Option<Selection<'c>> {
    for substrate in substrates {
        if let Some(signed) = substrate.fetch(pinned_subject) {
            cache.accept(signed, pinned_subject);
        }
    }
    cache.current(now)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod cache_tests {
    use super::*;

    // Oracle: the #5793 decision record, Decisions 2+3, invariant 3.

    fn key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    fn subject() -> String {
        TransportAdvertisement::subject_for(&key().verifying_key())
    }

    fn signed(sequence: u64, issued_at: u64, ttl: u64) -> SignedAdvertisement {
        TransportAdvertisement {
            subject: subject(),
            transports: vec![TransportEntry {
                kind: TransportKind::Tcp,
                endpoint: "203.0.113.7:443".to_string(),
                priority: 30,
                broker: None,
            }],
            issued_at,
            expires_at: issued_at + ttl,
            sequence,
        }
        .sign(&key())
    }

    const NOW: u64 = 1_700_000_000;

    struct StaticSubstrate(Option<SignedAdvertisement>);
    impl Substrate for StaticSubstrate {
        fn fetch(&self, _subject: &str) -> Option<SignedAdvertisement> {
            self.0.clone()
        }
    }

    #[test]
    fn cache_rejects_replays_and_forgeries() {
        let mut cache = AdCache::new();
        assert!(cache.accept(signed(5, NOW, 3600), &subject()));
        // Replay of an older sequence never displaces (invariant 3).
        assert!(!cache.accept(signed(5, NOW, 3600), &subject()));
        assert!(!cache.accept(signed(2, NOW, 3600), &subject()));
        assert_eq!(cache.cached_sequence(), 5);
        // A forgery for the same subject is refused at the door.
        let forged = TransportAdvertisement {
            subject: subject(),
            transports: vec![],
            issued_at: NOW,
            expires_at: NOW + 3600,
            sequence: 99,
        }
        .sign(&SigningKey::from_bytes(&[9u8; 32]));
        assert!(!cache.accept(forged, &subject()));
        assert_eq!(cache.cached_sequence(), 5);
    }

    #[test]
    fn resolve_walks_substrates_and_upgrades_cache() {
        // Decision 2: no substrate outranks another — the highest
        // valid sequence wins even when a later substrate is stale,
        // and a hostile substrate's forgery is ignored.
        let hostile = StaticSubstrate(Some(
            TransportAdvertisement {
                subject: subject(),
                transports: vec![],
                issued_at: NOW,
                expires_at: NOW + 3600,
                sequence: 99,
            }
            .sign(&SigningKey::from_bytes(&[9u8; 32])),
        ));
        let stale = StaticSubstrate(Some(signed(3, NOW, 3600)));
        let fresh = StaticSubstrate(Some(signed(8, NOW, 3600)));
        let empty = StaticSubstrate(None);
        let mut cache = AdCache::new();
        let substrates: [&dyn Substrate; 4] = [&hostile, &stale, &fresh, &empty];
        let sel = resolve(&mut cache, &substrates, &subject(), NOW).expect("resolution succeeds");
        assert_eq!(sel.advertisement.sequence, 8);
        assert!(!sel.degraded);
        assert_eq!(cache.cached_sequence(), 8);
    }

    #[test]
    fn resolve_falls_back_to_degraded_cache_when_substrates_fail() {
        // Decision 3: with every substrate down, the expired
        // last-known-good is used, flagged degraded.
        let mut cache = AdCache::new();
        assert!(cache.accept(signed(4, NOW - 7200, 3600), &subject()));
        let down = StaticSubstrate(None);
        let substrates: [&dyn Substrate; 1] = [&down];
        let sel = resolve(&mut cache, &substrates, &subject(), NOW).expect("LKG fallback");
        assert!(sel.degraded);
        assert_eq!(sel.advertisement.sequence, 4);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod minter_tests {
    use super::*;

    // Oracle: the #5793 decision record, Decision 3 (mint on
    // transport change and at least once per TTL) and invariant 8
    // (sequence durably monotonic per subject).

    const NOW: u64 = 1_700_000_000;

    fn key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    fn transports(port: u16) -> Vec<TransportEntry> {
        vec![TransportEntry {
            kind: TransportKind::Tcp,
            endpoint: format!("203.0.113.7:{port}"),
            priority: 30,
            broker: None,
        }]
    }

    fn parse(bytes: &[u8]) -> SignedAdvertisement {
        serde_json::from_slice(bytes).expect("served bytes parse as a SignedAdvertisement")
    }

    fn minter(dir: &std::path::Path, ttl: u64) -> AdMinter {
        AdMinter::new(
            key(),
            SequenceFile::new(dir.join("transport-ad.sequence")),
            ttl,
        )
    }

    #[test]
    fn minted_ad_verifies_and_carries_the_recommended_shape() {
        let dir = tempfile::tempdir().unwrap();
        let m = minter(dir.path(), RECOMMENDED_TTL_SECONDS);
        let bytes = m.mint(transports(443), NOW).unwrap();
        let signed = parse(&bytes);
        let ad = signed
            .verify(&m.subject())
            .expect("a minted ad verifies against its subject");
        assert_eq!(ad.sequence, 1);
        assert_eq!(ad.expires_at - ad.issued_at, RECOMMENDED_TTL_SECONDS);
        // Serving is byte-for-byte: what current_bytes returns is
        // exactly what mint produced.
        assert_eq!(
            m.current_bytes(NOW).unwrap().as_deref(),
            Some(bytes.as_slice())
        );
    }

    #[test]
    fn refresh_mints_only_on_change_or_expiry() {
        let dir = tempfile::tempdir().unwrap();
        let m = minter(dir.path(), 3600);
        let first = m.refresh(transports(443), NOW).unwrap();
        // Same transports, still fresh: no new sequence.
        let again = m.refresh(transports(443), NOW + 60).unwrap();
        assert_eq!(first, again);
        assert_eq!(parse(&again).advertisement.sequence, 1);
        // Transport change: Decision 3 mints sequence + 1.
        let changed = m.refresh(transports(8443), NOW + 120).unwrap();
        assert_eq!(parse(&changed).advertisement.sequence, 2);
        // Expiry of the current ad: the per-TTL mint fires even
        // with unchanged transports.
        let expired = m.refresh(transports(8443), NOW + 120 + 3600).unwrap();
        assert_eq!(parse(&expired).advertisement.sequence, 3);
    }

    #[test]
    fn current_bytes_remints_an_expired_ad() {
        let dir = tempfile::tempdir().unwrap();
        let m = minter(dir.path(), 3600);
        assert!(m.current_bytes(NOW).unwrap().is_none());
        m.mint(transports(443), NOW).unwrap();
        let bytes = m.current_bytes(NOW + 3600).unwrap().expect("re-minted");
        let ad = parse(&bytes).advertisement;
        assert_eq!(ad.sequence, 2);
        assert_eq!(ad.issued_at, NOW + 3600);
    }

    #[test]
    fn sequence_never_regresses_across_minter_restarts() {
        // Invariant 8: the counter lives beside the identity key,
        // so a restarted minter continues from the persisted
        // sequence instead of reissuing a lower one.
        let dir = tempfile::tempdir().unwrap();
        {
            let m = minter(dir.path(), 3600);
            m.mint(transports(443), NOW).unwrap();
            m.mint(transports(443), NOW + 10).unwrap();
        }
        let restarted = minter(dir.path(), 3600);
        let bytes = restarted.mint(transports(443), NOW + 20).unwrap();
        assert_eq!(parse(&bytes).advertisement.sequence, 3);
    }

    #[test]
    fn ttl_is_clamped_to_the_record_maximum() {
        let dir = tempfile::tempdir().unwrap();
        let m = minter(dir.path(), MAX_TTL_SECONDS * 10);
        let bytes = m.mint(transports(443), NOW).unwrap();
        let ad = parse(&bytes).advertisement;
        assert_eq!(ad.expires_at - ad.issued_at, MAX_TTL_SECONDS);
    }
}

// ---------------------------------------------------------------------
// Slice 3 (#8214): fetch-side HTTPS substrates.
//
// HTTP-stack decision (recorded on issue #8214 by lane any-j,
// 2026-11-17, following lane any-c's framing): id-core takes NO
// HTTP client dependency. The substrates below bind the Substrate
// trait to an injected fetch closure (FetchBytes) owned by the
// eventual caller's HTTP stack, so the choice of client library
// stays with the client binary and never gets buried in id-core.
// Substrates are untrusted (Decision 1): they return the opaque
// signed object, and resolve()/AdCache verification against the
// pinned subject is what makes a hostile substrate fail closed.

/// An injected HTTPS GET: returns the response body for `url`, or
/// `None` when the fetch fails or the substrate has nothing. This
/// is the seam the caller's HTTP stack binds to; implementations
/// must use plain HTTPS (Decision 2, invariant 5).
pub trait FetchBytes {
    fn get(&self, url: &str) -> Option<Vec<u8>>;
}

impl<F> FetchBytes for F
where
    F: Fn(&str) -> Option<Vec<u8>>,
{
    fn get(&self, url: &str) -> Option<Vec<u8>> {
        self(url)
    }
}

/// Parse substrate bytes as a signed advertisement. Malformed
/// bytes are `None` (an empty substrate), never an error worth
/// failing resolution over; well-formed but forged bytes parse
/// here and are rejected later by signature verification.
fn parse_served(bytes: &[u8]) -> Option<SignedAdvertisement> {
    serde_json::from_slice(bytes).ok()
}

/// The well-known URL for `subject` on `host` (Decision 2.2):
/// `https://<host>/.well-known/id-transport-advertisement?subject=<fp>`.
pub fn well_known_url(host: &str, subject: &str) -> String {
    format!("https://{host}/.well-known/id-transport-advertisement?subject={subject}")
}

/// `H(subject)` for the Workers rendezvous (Decision 2.3): the
/// lowercase hex SHA-256 of the subject fingerprint string. The
/// rendezvous learns only this hash, never the named subject
/// (record privacy note, Decision 3). The Workers component must
/// key its stored blobs by this same value.
pub fn subject_hash(subject: &str) -> String {
    use sha2::Digest as _;
    hex_encode(&sha2::Sha256::digest(subject.as_bytes()))
}

/// The Workers rendezvous fetch URL for `subject` under
/// `rendezvous_base` (Decision 2.3):
/// `https://<rendezvous-host>/v1/advertisement/<H(subject)>`.
pub fn rendezvous_url(rendezvous_base: &str, subject: &str) -> String {
    format!(
        "{}/v1/advertisement/{}",
        rendezvous_base.trim_end_matches('/'),
        subject_hash(subject)
    )
}

/// The authoritative self-hosted substrate (Decision 2.2): fetch
/// the subject's current signed ad from a known host's
/// `.well-known` endpoint over the injected HTTPS fetch.
#[derive(Debug)]
pub struct WellKnownSubstrate<F> {
    host: String,
    fetch: F,
}

impl<F> WellKnownSubstrate<F> {
    pub fn new(host: impl Into<String>, fetch: F) -> Self {
        Self {
            host: host.into(),
            fetch,
        }
    }
}

impl<F: FetchBytes> Substrate for WellKnownSubstrate<F> {
    fn fetch(&self, subject: &str) -> Option<SignedAdvertisement> {
        let bytes = self.fetch.get(&well_known_url(&self.host, subject))?;
        parse_served(&bytes)
    }
}

/// The hosted-mirror substrate (Decision 2.3): fetch the subject's
/// signed ad from the Workers rendezvous by `H(subject)`. The
/// rendezvous stores opaque signed blobs only; it holds no keys
/// and cannot mint or alter ads.
#[derive(Debug)]
pub struct RendezvousSubstrate<F> {
    base: String,
    fetch: F,
}

impl<F> RendezvousSubstrate<F> {
    pub fn new(base: impl Into<String>, fetch: F) -> Self {
        Self {
            base: base.into(),
            fetch,
        }
    }
}

impl<F: FetchBytes> Substrate for RendezvousSubstrate<F> {
    fn fetch(&self, subject: &str) -> Option<SignedAdvertisement> {
        let bytes = self.fetch.get(&rendezvous_url(&self.base, subject))?;
        parse_served(&bytes)
    }
}

/// Bootstrap hints learned from DNS `_id.<host>` TXT records
/// (Decision 2.4). DNS is bootstrap-only and never authoritative:
/// hints name the hosts for the well-known and rendezvous
/// substrates, and everything those substrates return is still
/// authenticated by signature (Decision 1). A TXT record carries
/// hints as whitespace-separated `key=value` tokens:
/// `well-known=<host>` and `rendezvous=<base-url>`; unknown tokens
/// are ignored, so records may carry other bootstrap data.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BootstrapHints {
    pub well_known_hosts: Vec<String>,
    pub rendezvous_bases: Vec<String>,
}

/// Parse `_id.<host>` TXT record values into bootstrap hints.
/// Duplicate hints are kept once, in first-seen order.
pub fn parse_bootstrap_hints(txt_records: &[String]) -> BootstrapHints {
    let mut hints = BootstrapHints::default();
    for record in txt_records {
        for token in record.split_whitespace() {
            if let Some(host) = token.strip_prefix("well-known=") {
                if !host.is_empty() && !hints.well_known_hosts.iter().any(|h| h == host) {
                    hints.well_known_hosts.push(host.to_string());
                }
            } else if let Some(base) = token.strip_prefix("rendezvous=") {
                if !base.is_empty() && !hints.rendezvous_bases.iter().any(|b| b == base) {
                    hints.rendezvous_bases.push(base.to_string());
                }
            }
        }
    }
    hints
}

/// Background revalidation (Decision 3): when a resolution came
/// back degraded — an expired last-known-good in use — the caller
/// revalidates in the background by re-walking every substrate.
/// This is that re-walk: it folds any fresher valid ad into the
/// cache and returns the post-revalidation selection, which is
/// non-degraded exactly when a substrate produced a fresh ad.
/// Calling it on a timer or from a background task never blocks
/// the connection that is proceeding on the degraded selection.
pub fn revalidate<'c>(
    cache: &'c mut AdCache,
    substrates: &[&dyn Substrate],
    pinned_subject: &str,
    now: u64,
) -> Option<Selection<'c>> {
    resolve(cache, substrates, pinned_subject, now)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod substrate_tests {
    use super::*;
    use std::cell::RefCell;

    // Oracle: the #5793 decision record, Decision 2 (substrate
    // selection + disagreement rule) and Decision 3 (freshness,
    // replay, last-known-good + background revalidation).

    const NOW: u64 = 1_700_000_000;

    fn key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    fn subject() -> String {
        TransportAdvertisement::subject_for(&key().verifying_key())
    }

    fn signed(sequence: u64, issued_at: u64, ttl: u64) -> SignedAdvertisement {
        TransportAdvertisement {
            subject: subject(),
            transports: vec![TransportEntry {
                kind: TransportKind::Tcp,
                endpoint: "203.0.113.7:443".to_string(),
                priority: 30,
                broker: None,
            }],
            issued_at,
            expires_at: issued_at + ttl,
            sequence,
        }
        .sign(&key())
    }

    fn served(signed: &SignedAdvertisement) -> Vec<u8> {
        serde_json::to_vec(signed).unwrap()
    }

    #[test]
    fn well_known_substrate_fetches_the_decision_2_url_and_parses() {
        let seen = RefCell::new(Vec::new());
        let body = served(&signed(3, NOW, 3600));
        let substrate = WellKnownSubstrate::new("id.example.com", |url: &str| {
            seen.borrow_mut().push(url.to_string());
            Some(body.clone())
        });
        let fetched = substrate.fetch(&subject()).expect("substrate has the ad");
        assert_eq!(fetched.advertisement.sequence, 3);
        let urls = seen.borrow();
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0], well_known_url("id.example.com", &subject()));
        assert!(
            urls[0].starts_with(
                "https://id.example.com/.well-known/id-transport-advertisement?subject="
            )
        );
    }

    #[test]
    fn rendezvous_substrate_fetches_by_subject_hash_only() {
        // Decision 3 privacy: the rendezvous URL carries H(subject),
        // never the subject fingerprint itself.
        let seen = RefCell::new(Vec::new());
        let body = served(&signed(4, NOW, 3600));
        let substrate = RendezvousSubstrate::new("https://rendezvous.example", |url: &str| {
            seen.borrow_mut().push(url.to_string());
            Some(body.clone())
        });
        let fetched = substrate.fetch(&subject()).expect("substrate has the ad");
        assert_eq!(fetched.advertisement.sequence, 4);
        let urls = seen.borrow();
        assert_eq!(urls.len(), 1);
        assert_eq!(
            urls[0],
            format!(
                "https://rendezvous.example/v1/advertisement/{}",
                subject_hash(&subject())
            )
        );
        assert!(!urls[0].contains(&subject()));
        // H is a stable 32-byte SHA-256 hex digest.
        assert_eq!(subject_hash(&subject()).len(), 64);
        assert_eq!(subject_hash(&subject()), subject_hash(&subject()));
        assert_ne!(subject_hash(&subject()), subject_hash("other-subject"));
    }

    #[test]
    fn hostile_https_substrate_fails_closed_through_resolve() {
        // Invariants 1-3 via a real substrate: forged bytes parse
        // but never verify, so resolution ignores them entirely.
        let forged = TransportAdvertisement {
            subject: subject(),
            transports: vec![],
            issued_at: NOW,
            expires_at: NOW + 3600,
            sequence: 99,
        }
        .sign(&SigningKey::from_bytes(&[9u8; 32]));
        let body = served(&forged);
        let substrate =
            WellKnownSubstrate::new("id.example.com", move |_url: &str| Some(body.clone()));
        let mut cache = AdCache::new();
        let substrates: [&dyn Substrate; 1] = [&substrate];
        assert!(resolve(&mut cache, &substrates, &subject(), NOW).is_none());
        assert_eq!(cache.cached_sequence(), 0);
    }

    #[test]
    fn disagreement_across_https_substrates_highest_sequence_wins() {
        // Decision 2 disagreement rule across the two HTTPS
        // substrates, in both fetch orders.
        let wk_body = served(&signed(5, NOW, 3600));
        let rv_body = served(&signed(8, NOW, 3600));
        let wk = WellKnownSubstrate::new("id.example.com", move |_url: &str| Some(wk_body.clone()));
        let rv = RendezvousSubstrate::new("https://rendezvous.example", move |_url: &str| {
            Some(rv_body.clone())
        });
        let forward: [&dyn Substrate; 2] = [&wk, &rv];
        let reverse: [&dyn Substrate; 2] = [&rv, &wk];
        for substrates in [forward, reverse] {
            let mut cache = AdCache::new();
            let sel = resolve(&mut cache, &substrates, &subject(), NOW).expect("resolution");
            assert_eq!(sel.advertisement.sequence, 8);
            assert!(!sel.degraded);
        }
    }

    #[test]
    fn unreachable_or_malformed_substrate_is_empty_not_an_error() {
        let down = WellKnownSubstrate::new("id.example.com", |_url: &str| None);
        assert!(down.fetch(&subject()).is_none());
        let garbage = RendezvousSubstrate::new("https://rendezvous.example", |_url: &str| {
            Some(b"not json".to_vec())
        });
        assert!(garbage.fetch(&subject()).is_none());
    }

    #[test]
    fn bootstrap_hints_name_hosts_only_and_ignore_the_rest() {
        // Decision 2.4: TXT hints name substrate hosts; they carry
        // no advertisement and vouch for nothing.
        let records = vec![
            "well-known=id.example.com rendezvous=https://rendezvous.example".to_string(),
            "well-known=id.example.com other=data v=spf1".to_string(),
            "rendezvous=https://rendezvous.example well-known=backup.example.com".to_string(),
        ];
        let hints = parse_bootstrap_hints(&records);
        assert_eq!(
            hints.well_known_hosts,
            vec![
                "id.example.com".to_string(),
                "backup.example.com".to_string()
            ]
        );
        assert_eq!(
            hints.rendezvous_bases,
            vec!["https://rendezvous.example".to_string()]
        );
        assert_eq!(parse_bootstrap_hints(&[]), BootstrapHints::default());
    }

    #[test]
    fn degraded_selection_is_upgraded_by_background_revalidation() {
        // Decision 3: an expired last-known-good is used degraded
        // while substrates are down; background revalidation with
        // a recovered substrate upgrades it to a fresh selection.
        let mut cache = AdCache::new();
        assert!(cache.accept(signed(4, NOW - 7200, 3600), &subject()));
        let down = WellKnownSubstrate::new("id.example.com", |_url: &str| None);
        let substrates: [&dyn Substrate; 1] = [&down];
        let sel = resolve(&mut cache, &substrates, &subject(), NOW).expect("LKG fallback");
        assert!(sel.degraded);

        let body = served(&signed(6, NOW, 3600));
        let up = WellKnownSubstrate::new("id.example.com", move |_url: &str| Some(body.clone()));
        let substrates: [&dyn Substrate; 1] = [&up];
        let sel = revalidate(&mut cache, &substrates, &subject(), NOW).expect("revalidated");
        assert!(!sel.degraded);
        assert_eq!(sel.advertisement.sequence, 6);
    }
}
