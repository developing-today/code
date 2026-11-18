//! The app's one residency evidence verifier and claims builder (#8851, design §2–§3).
//!
//! Placement evidence is an Ed25519 signature by its issuer over
//! [`evidence_signing_bytes`]. The issuer is named by its public key in hex, so the
//! issuer string the residency evaluator checks against the operator's trust roots
//! is the same key that verifies the signature.
//!
//! [`AppEvidenceVerifier`] reads its revocation state on every call, so revoking an
//! issuer or one serial takes effect on the next check without a restart. It fails
//! closed: evidence from an issuer that is neither registered as local nor a peer
//! whose policy is in force counts as revoked.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

use id_core::boundary_surfaces::peer_asserted_claim;
use id_core::residency::{
    AssuranceLevel, Evidence, EvidenceVerifier, Plane, PrivacyClass, SubjectEvidence,
};
use id_directory::directory::hex_to_array;

/// The domain tag every evidence signature starts with, so a signature made for
/// anything else never verifies as evidence.
pub const EVIDENCE_DOMAIN: &[u8] = b"id.boundary.evidence.v1\0";

/// A plane's name as the config and the signed bytes write it.
#[must_use]
pub const fn plane_name(plane: Plane) -> &'static str {
    match plane {
        Plane::Storage => "storage",
        Plane::Processing => "processing",
        Plane::Transport => "transport",
        Plane::Metadata => "metadata",
    }
}

/// The plane a name names, if any.
#[must_use]
pub fn plane_from_name(name: &str) -> Option<Plane> {
    Plane::ALL
        .into_iter()
        .find(|plane| plane_name(*plane) == name)
}

/// The Ed25519 key an issuer string names: 64 lowercase or uppercase hex digits.
#[must_use]
pub fn issuer_key(issuer: &str) -> Option<VerifyingKey> {
    let bytes = hex_to_array::<32>(issuer)?;
    VerifyingKey::from_bytes(&bytes).ok()
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

fn push_field(out: &mut Vec<u8>, field: &[u8]) {
    out.extend_from_slice(&(field.len() as u64).to_be_bytes());
    out.extend_from_slice(field);
}

/// The bytes an issuer signs: the domain tag, then each field length-prefixed so
/// no two different evidence records share signing bytes.
#[must_use]
pub fn evidence_signing_bytes(evidence: &Evidence) -> Vec<u8> {
    let mut out = EVIDENCE_DOMAIN.to_vec();
    push_field(&mut out, evidence.issuer.as_bytes());
    push_field(&mut out, evidence.resource.0.as_bytes());
    push_field(&mut out, plane_name(evidence.plane).as_bytes());
    push_field(&mut out, evidence.jurisdiction.0.as_bytes());
    push_field(&mut out, evidence.source.as_bytes());
    out.extend_from_slice(&evidence.expires_at.to_be_bytes());
    push_field(&mut out, evidence.serial.as_bytes());
    out
}

/// Signs `evidence` as `key`: sets its issuer to the key's public half in hex and
/// its signature over the resulting signing bytes.
pub fn sign_evidence(key: &SigningKey, evidence: &mut Evidence) {
    evidence.issuer = hex(key.verifying_key().as_bytes());
    evidence.signature = key
        .sign(&evidence_signing_bytes(evidence))
        .to_bytes()
        .to_vec();
}

/// When a peer's policy is in force, in unix seconds. Either bound may be open.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PeerWindow {
    /// The first instant the policy is in force, if bounded.
    pub valid_from: Option<u64>,
    /// The instant the policy stops being in force, if bounded.
    pub expires_at: Option<u64>,
}

impl PeerWindow {
    /// Whether the window holds `now`.
    #[must_use]
    pub const fn holds(&self, now: u64) -> bool {
        if let Some(from) = self.valid_from
            && now < from
        {
            return false;
        }
        if let Some(until) = self.expires_at
            && now >= until
        {
            return false;
        }
        true
    }
}

#[derive(Default)]
struct RevocationState {
    local: BTreeSet<String>,
    peers: BTreeMap<String, PeerWindow>,
    revoked_issuers: BTreeSet<String>,
    revoked_serials: BTreeSet<(String, String)>,
}

/// The app's one [`EvidenceVerifier`] (design §3).
pub struct AppEvidenceVerifier {
    state: RwLock<RevocationState>,
    clock: Box<dyn Fn() -> u64 + Send + Sync>,
}

impl std::fmt::Debug for AppEvidenceVerifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppEvidenceVerifier")
            .finish_non_exhaustive()
    }
}

impl Default for AppEvidenceVerifier {
    /// A verifier on the system clock that knows no issuer yet.
    fn default() -> Self {
        Self::new(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_secs())
        })
    }
}

impl AppEvidenceVerifier {
    /// A verifier that knows no issuer yet and reads peer windows against `clock`
    /// (unix seconds).
    pub fn new(clock: impl Fn() -> u64 + Send + Sync + 'static) -> Self {
        Self {
            state: RwLock::new(RevocationState::default()),
            clock: Box::new(clock),
        }
    }

    fn write(&self, change: impl FnOnce(&mut RevocationState)) {
        let mut state = self
            .state
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        change(&mut state);
    }

    /// Registers an issuer that runs on this server, such as the server's own
    /// placement attestor.
    pub fn register_local(&self, issuer: &str) {
        let issuer = issuer.to_ascii_lowercase();
        self.write(|state| {
            state.local.insert(issuer);
        });
    }

    /// Registers a federated peer as an issuer while `window` holds, replacing any
    /// window it had.
    pub fn register_peer(&self, issuer: &str, window: PeerWindow) {
        let issuer = issuer.to_ascii_lowercase();
        self.write(|state| {
            state.peers.insert(issuer, window);
        });
    }

    /// Forgets a peer, so its evidence counts as revoked from the next check on.
    pub fn forget_peer(&self, issuer: &str) {
        let issuer = issuer.to_ascii_lowercase();
        self.write(|state| {
            state.peers.remove(&issuer);
        });
    }

    /// Revokes everything an issuer has signed.
    pub fn revoke_issuer(&self, issuer: &str) {
        let issuer = issuer.to_ascii_lowercase();
        self.write(|state| {
            state.revoked_issuers.insert(issuer);
        });
    }

    /// Revokes one serial of one issuer.
    pub fn revoke_serial(&self, issuer: &str, serial: &str) {
        let key = (issuer.to_ascii_lowercase(), serial.to_owned());
        self.write(|state| {
            state.revoked_serials.insert(key);
        });
    }
}

impl EvidenceVerifier for AppEvidenceVerifier {
    fn signature_valid(&self, evidence: &Evidence) -> bool {
        let Some(key) = issuer_key(&evidence.issuer) else {
            return false;
        };
        let Ok(signature) = Signature::from_slice(&evidence.signature) else {
            return false;
        };
        key.verify_strict(&evidence_signing_bytes(evidence), &signature)
            .is_ok()
    }

    fn revoked(&self, evidence: &Evidence) -> bool {
        let Ok(state) = self.state.read() else {
            return true;
        };
        let issuer = evidence.issuer.to_ascii_lowercase();
        if state.revoked_issuers.contains(&issuer)
            || state
                .revoked_serials
                .contains(&(issuer.clone(), evidence.serial.clone()))
        {
            return true;
        }
        let known = state.local.contains(&issuer)
            || state
                .peers
                .get(&issuer)
                .is_some_and(|window| window.holds((self.clock)()));
        !known
    }
}

/// Builds the claims one surface check receives (design §3): this server's own
/// evidence first, then each peer's claims graded by [`peer_asserted_claim`].
#[derive(Clone, Debug)]
pub struct ClaimsBuilder<'a> {
    trust_roots: &'a BTreeSet<String>,
    subject: String,
    local: Vec<SubjectEvidence>,
    peer: Vec<SubjectEvidence>,
}

impl<'a> ClaimsBuilder<'a> {
    /// A builder for claims about `subject`, graded against `trust_roots`.
    #[must_use]
    pub fn new(trust_roots: &'a BTreeSet<String>, subject: &str) -> Self {
        Self {
            trust_roots,
            subject: subject.to_owned(),
            local: Vec::new(),
            peer: Vec::new(),
        }
    }

    /// Adds one hop this server attests itself. Its assurance is
    /// [`AssuranceLevel::ProviderVerified`] when its issuer is a trust root, and
    /// [`AssuranceLevel::SelfDeclared`] otherwise.
    #[must_use]
    pub fn local(mut self, basis: &str, privacy_class: PrivacyClass, evidence: Evidence) -> Self {
        let assurance = if self.trust_roots.contains(&evidence.issuer) {
            AssuranceLevel::ProviderVerified
        } else {
            AssuranceLevel::SelfDeclared
        };
        self.local.push(SubjectEvidence {
            subject: self.subject.clone(),
            assurance,
            basis: basis.to_owned(),
            privacy_class,
            placement_evidence: evidence,
        });
        self
    }

    /// Adds one claim a peer asserted about the subject. An untrusted peer's claim
    /// is capped at [`AssuranceLevel::SelfDeclared`]. A claim about another subject
    /// is dropped, since the check would never read it.
    #[must_use]
    pub fn peer(mut self, peer: &str, claim: SubjectEvidence) -> Self {
        if claim.subject == self.subject {
            self.peer
                .push(peer_asserted_claim(peer, self.trust_roots, claim));
        }
        self
    }

    /// The claims, this server's own first.
    #[must_use]
    pub fn build(self) -> Vec<SubjectEvidence> {
        let mut claims = self.local;
        claims.extend(self.peer);
        claims
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::fmt::Write as _;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    use ed25519_dalek::SigningKey;
    use id_core::boundary::{ActorId, CapabilityKind, EnforcementPoint};
    use id_core::boundary_surfaces::{FrameworkOutcome, check_surface};
    use id_core::residency::{
        AssuranceLevel, Evidence, EvidenceVerifier, Jurisdiction, Plane, PrivacyClass, ResourceId,
    };

    use super::{AppEvidenceVerifier, ClaimsBuilder, PeerWindow, hex, sign_evidence};
    use crate::server_config::{ScopesInPlay, ServerConfig};

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn pub_hex(seed: u8) -> String {
        hex(key(seed).verifying_key().as_bytes())
    }

    fn evidence(plane: Plane, serial: &str) -> Evidence {
        Evidence {
            issuer: String::new(),
            resource: ResourceId("r".to_owned()),
            plane,
            jurisdiction: Jurisdiction("us".to_owned()),
            source: "attestor".to_owned(),
            expires_at: 1_000,
            serial: serial.to_owned(),
            signature: Vec::new(),
        }
    }

    fn signed(seed: u8, plane: Plane, serial: &str) -> Evidence {
        let mut evidence = evidence(plane, serial);
        sign_evidence(&key(seed), &mut evidence);
        evidence
    }

    #[test]
    fn a_signature_verifies_only_for_its_exact_evidence() {
        let verifier = AppEvidenceVerifier::new(|| 0);
        let good = signed(1, Plane::Storage, "s1");
        assert!(verifier.signature_valid(&good));

        let mut tampered = good.clone();
        tampered.jurisdiction = Jurisdiction("eu".to_owned());
        assert!(!verifier.signature_valid(&tampered));

        let mut wrong_key = good.clone();
        wrong_key.issuer = pub_hex(2);
        assert!(!verifier.signature_valid(&wrong_key));

        let mut garbled = good;
        garbled.issuer = "not hex".to_owned();
        assert!(!verifier.signature_valid(&garbled));
    }

    #[test]
    fn revocation_takes_effect_on_the_next_check() {
        let verifier = AppEvidenceVerifier::new(|| 0);
        let first = signed(1, Plane::Storage, "s1");
        let second = signed(1, Plane::Storage, "s2");
        assert!(verifier.revoked(&first), "an unknown issuer fails closed");

        verifier.register_local(&pub_hex(1));
        assert!(!verifier.revoked(&first));

        verifier.revoke_serial(&pub_hex(1), "s1");
        assert!(verifier.revoked(&first));
        assert!(!verifier.revoked(&second));

        verifier.revoke_issuer(&pub_hex(1).to_ascii_uppercase());
        assert!(verifier.revoked(&second));
    }

    #[test]
    fn a_peer_issuer_counts_only_while_its_window_holds() {
        let now = Arc::new(AtomicU64::new(10));
        let clock = Arc::clone(&now);
        let verifier = AppEvidenceVerifier::new(move || clock.load(Ordering::SeqCst));
        let claim = signed(3, Plane::Transport, "p1");

        verifier.register_peer(
            &pub_hex(3),
            PeerWindow {
                valid_from: Some(5),
                expires_at: Some(20),
            },
        );
        assert!(!verifier.revoked(&claim));
        now.store(20, Ordering::SeqCst);
        assert!(verifier.revoked(&claim), "expired peer");
        now.store(4, Ordering::SeqCst);
        assert!(verifier.revoked(&claim), "not yet in force");

        now.store(10, Ordering::SeqCst);
        verifier.forget_peer(&pub_hex(3));
        assert!(verifier.revoked(&claim), "forgotten peer");
    }

    #[test]
    fn claims_are_graded_and_ordered() {
        let roots = std::iter::once(pub_hex(1)).collect();
        let trusted_peer_claim = ClaimsBuilder::new(&roots, "ann")
            .local(
                "hop",
                PrivacyClass::VerdictOnly,
                signed(1, Plane::Storage, "a"),
            )
            .build()
            .remove(0);
        let untrusted_peer_claim = ClaimsBuilder::new(&roots, "ann")
            .local(
                "hop",
                PrivacyClass::VerdictOnly,
                signed(2, Plane::Storage, "b"),
            )
            .build()
            .remove(0);
        assert_eq!(
            trusted_peer_claim.assurance,
            AssuranceLevel::ProviderVerified
        );
        assert_eq!(untrusted_peer_claim.assurance, AssuranceLevel::SelfDeclared);

        let mut strong_from_stranger = trusted_peer_claim.clone();
        strong_from_stranger.basis = "stranger".to_owned();
        let mut other_subject = trusted_peer_claim.clone();
        other_subject.subject = "bo".to_owned();

        let claims = ClaimsBuilder::new(&roots, "ann")
            .peer(&pub_hex(2), strong_from_stranger)
            .peer(&pub_hex(1), trusted_peer_claim)
            .peer(&pub_hex(1), other_subject)
            .local(
                "own",
                PrivacyClass::VerdictOnly,
                signed(1, Plane::Transport, "c"),
            )
            .build();
        assert_eq!(claims.len(), 3);
        assert_eq!(claims[0].basis, "own");
        assert_eq!(claims[1].basis, "stranger");
        assert_eq!(claims[1].assurance, AssuranceLevel::SelfDeclared);
        assert_eq!(claims[2].assurance, AssuranceLevel::ProviderVerified);
    }

    #[test]
    fn signed_evidence_passes_a_declared_boundary_until_revoked() {
        let root = pub_hex(1);
        let mut text = format!(
            "[residency]\ntrust_roots = [\"{root}\"]\nrequired = [\"transport\", \"storage\"]\n\n"
        );
        for plane in ["transport", "storage"] {
            let _ = write!(
                text,
                "[[residency.planes]]\nplane = \"{plane}\"\njurisdictions = [\"us\"]\n\
                 resources = [\"r\"]\nevidence_sources = [\"attestor\"]\n\n"
            );
        }
        text.push_str(
            "[[boundaries]]\ncapability = \"residency\"\nscope = { kind = \"server\", id = \"main\" }\n\
             subject_class = \"user\"\n\
             evidence_req = { minimum_assurance = \"provider_verified\", accepted_sources = [] }\n\
             enforcement = [\"login\"]\nfailure = \"deny\"\nenabled = true\nversion = 1\n\
             declared_by = \"operator\"\ndeclared_at = 1\n",
        );
        let config = ServerConfig::parse(&text).unwrap();
        let verifier = Arc::new(AppEvidenceVerifier::new(|| 0));
        verifier.register_local(&root);
        let shared: Arc<dyn EvidenceVerifier + Send + Sync> =
            Arc::<AppEvidenceVerifier>::clone(&verifier);
        let inputs = config
            .boundaries()
            .inputs(
                CapabilityKind::Residency,
                ScopesInPlay {
                    server: "main",
                    world: None,
                    account: None,
                },
                shared,
            )
            .unwrap();
        let claims = ClaimsBuilder::new(config.boundaries().trust_roots(), "ann")
            .local(
                "hop",
                PrivacyClass::VerdictOnly,
                signed(1, Plane::Transport, "t"),
            )
            .local(
                "hop",
                PrivacyClass::VerdictOnly,
                signed(1, Plane::Storage, "s"),
            )
            .build();
        let check = |now| {
            check_surface(
                EnforcementPoint::Login,
                &inputs.effective,
                inputs.version,
                &inputs.scope_ref,
                &ActorId("ann".to_owned()),
                "ann",
                &inputs.scope,
                &claims,
                now,
                inputs.verifier.as_ref(),
            )
        };
        assert_eq!(check(10).outcome, FrameworkOutcome::Allow);
        assert!(
            matches!(check(1_000).outcome, FrameworkOutcome::Deny { .. }),
            "expired evidence"
        );

        verifier.revoke_serial(&root, "s");
        let revoked = check(10);
        assert!(matches!(revoked.outcome, FrameworkOutcome::Deny { .. }));
        assert!(revoked.log_entry.is_some());
    }
}
