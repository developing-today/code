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
//! durable sequence counter (invariant 8). Fetch/cache transport
//! and the `.well-known` serving handler are later slices.
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
        std::fs::write(&tmp, next.to_string())?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(next)
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
