//! Versioned, signed match-audit log primitives.
//!
//! This is the format and integrity layer only. It does not claim that a
//! signed log is a fair match: replay validity and observed-metadata screening
//! are separate verdicts in [`AuditReportV0`]. The wire encoding is versioned
//! Postcard; changing field order or meaning requires a new format version.

use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use p2panda_core::{Hash, SigningKey as IdSigningKey};
use serde::{Deserialize, Serialize};

const HEADER_DOMAIN: &[u8] = b"id.match-audit.v0/header\0";
const ENTRY_DOMAIN: &[u8] = b"id.match-audit.v0/entry\0";
const FOOTER_DOMAIN: &[u8] = b"id.match-audit.v0/footer\0";

/// The only format version currently defined.
pub const FORMAT_VERSION_V0: u8 = 0;

/// Player identity: the bytes of the player's Ed25519 verifying key.
pub type PlayerId = [u8; 32];

/// A participant in the match, in stable seat order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerV0 {
    /// Public signing key identifying this participant.
    pub id: PlayerId,
    /// Game-defined role, such as `x` or `o` for tic-tac-toe.
    pub role: String,
}

/// Signed, pre-match provenance and replay parameters.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeaderV0 {
    /// Wire-format version; currently [`FORMAT_VERSION_V0`].
    pub version: u8,
    /// Stable identifier for this match.
    pub match_id: String,
    /// World/program identifier and immutable build digest.
    pub program_id: String,
    /// Hash of the exact world program build.
    pub program_hash: Hash,
    /// Hash of the match rules/configuration.
    pub rules_hash: Hash,
    /// Simulation ticks per second.
    pub tick_rate: u32,
    /// Versioned RNG algorithm identifier.
    pub rng_algorithm: String,
    /// Commitment to the seed later revealed in the footer.
    pub seed_commitment: Hash,
    /// Hash of the initial state.
    pub initial_state_hash: Hash,
    /// Players in seat order.
    pub players: Vec<PlayerV0>,
    /// Tick at which the match begins.
    pub start_tick: u64,
}

/// A participant's signature over the versioned header or footer message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerSignatureV0 {
    /// The roster key that made this signature.
    pub player: PlayerId,
    /// Ed25519 signature bytes (exactly 64 bytes).
    pub signature: Vec<u8>,
}

/// One input in a block. Payload meaning belongs to the pinned world program.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputRecordV0 {
    /// Ticks since the preceding record in this block; the first record uses zero.
    pub tick_delta: u64,
    /// Index into [`HeaderV0::players`].
    pub player_slot: u16,
    /// Game-defined input bytes.
    pub payload: Vec<u8>,
}

/// Inputs grouped into one hash-chained block to amortize hash overhead.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputBlockV0 {
    /// Tick of the first input record in this block.
    pub first_tick: u64,
    /// Tick-delta encoded records, in consumption order.
    pub records: Vec<InputRecordV0>,
}

/// A journal event that changes match execution but is not a player input.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventV0 {
    /// Tick at which the event takes effect.
    pub tick: u64,
    /// Stable event-kind identifier.
    pub kind: String,
    /// Event-specific bytes.
    pub payload: Vec<u8>,
}

/// A replay checkpoint. The hash authenticates the submitted state only;
/// replay must independently reproduce it before it is considered valid.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointV0 {
    /// Tick represented by this checkpoint.
    pub tick: u64,
    /// Hash of the world state at `tick`.
    pub state_hash: Hash,
    /// Optional serialized state snapshot for checkpoint-resume audits.
    pub snapshot: Option<Vec<u8>>,
}

/// One ordered body in the tamper-evident match stream.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryBodyV0 {
    /// A block of player inputs consumed by the world.
    InputBlock(InputBlockV0),
    /// Non-input journal event affecting the match.
    Event(EventV0),
    /// State commitment produced by the recorder.
    Checkpoint(CheckpointV0),
}

/// A stream entry linked to the previous entry (or the signed header).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainedEntryV0 {
    /// Hash of the preceding stream element.
    pub previous_hash: Hash,
    /// Hash of the encoded body and `previous_hash`.
    pub hash: Hash,
    /// The input, event, or checkpoint committed by this entry.
    pub body: EntryBodyV0,
}

/// Metadata records retain their provenance instead of conflating claims with
/// independent observations.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetadataRecordV0 {
    /// Stable field name, e.g. `client_build` or `rtt_p95_ms`.
    pub key: String,
    /// Text representation defined by the named field's schema.
    pub value: String,
    /// Who supplied or derived this value.
    pub source: MetadataSourceV0,
}

/// Provenance category for a match metadata value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MetadataSourceV0 {
    /// Supplied by a participant; useful only as a consistency check.
    SelfReported,
    /// Independently observed by another player.
    PeerObserved,
    /// Independently observed by a relay.
    RelayObserved,
    /// Independently observed by a designated witness.
    WitnessObserved,
    /// Derived from the authenticated input stream.
    Derived,
}

/// Metadata stays separate from the technically-valid replay result.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetadataV0 {
    /// Participant-supplied values, never evidence of fairness by themselves.
    pub self_reported: Vec<MetadataRecordV0>,
    /// Independently observed or log-derived values, with provenance retained.
    pub evidence: Vec<MetadataRecordV0>,
}

/// Signed match outcome and final chain binding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FooterV0 {
    /// Last tick represented by the match.
    pub final_tick: u64,
    /// Hash of the final replayed state as claimed by the recorder.
    pub final_state_hash: Hash,
    /// Result claim encoded by the world/game, not an auditor verdict.
    pub result_claim: Vec<u8>,
    /// Revealed seed; its digest must match the signed header commitment.
    pub seed_reveal: Vec<u8>,
    /// One footer signature from every roster participant.
    pub signatures: Vec<PlayerSignatureV0>,
}

/// Complete v0 match log. The metadata section is covered by the footer
/// signatures, while its source labels remain distinct from replay validity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchLogV0 {
    /// Signed pre-match header.
    pub header: HeaderV0,
    /// One header signature from every roster participant.
    pub header_signatures: Vec<PlayerSignatureV0>,
    /// Ordered, hash-chained inputs, events, and checkpoints.
    pub entries: Vec<ChainedEntryV0>,
    /// Metadata with self-reported and independent evidence kept separate.
    pub metadata: MetadataV0,
    /// Signed final result claim and seed reveal.
    pub footer: FooterV0,
}

/// A closed L0–L2 technical verdict vocabulary. `ResultConfirmed` is reserved
/// for full-history replay; sampled success is never a result confirmation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TechnicalVerdictV0 {
    /// The log failed parsing, provenance, signature, or chain validation.
    InvalidRecord { reason: IntegrityFailureV0 },
    /// Full-history replay reproduced the claimed final state.
    ResultConfirmed { runtime: RuntimeScopeV0 },
    /// Replay diverged; interval endpoints name checkpoints, not exact ticks.
    ReplayDiverges {
        first_mismatching_interval: CheckpointIntervalV0,
    },
    /// Only the listed sampled intervals were replayed successfully.
    PartiallyVerified {
        intervals: Vec<CheckpointIntervalV0>,
        runtime: RuntimeScopeV0,
    },
    /// Verification could not make a claim because a required input was absent.
    Inconclusive { reason: InconclusiveReasonV0 },
}

/// Whether replay validity applies only to a pinned build/runtime or more broadly.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeScopeV0 {
    /// Safe default until cross-machine determinism is independently proved.
    PinnedRuntime,
    /// Use only after the pinned program has passed cross-platform proof.
    CrossPlatform,
}

/// An inclusive pair of checkpoint ticks used to identify replay coverage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointIntervalV0 {
    /// First checkpoint tick in the interval.
    pub start_tick: u64,
    /// Last checkpoint tick in the interval.
    pub end_tick: u64,
}

/// Why a technical audit could not reach a verdict.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InconclusiveReasonV0 {
    /// The pinned program build is unavailable.
    MissingProgramBuild,
    /// A required runtime or replay capability is unavailable.
    MissingReplayCapability,
    /// The requested audit did not include the complete history.
    IncompleteHistory,
}

/// Closed vocabulary for the probabilistic, actually-valid screening result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActualValidityV0 {
    /// No observed metadata was available to assess play plausibility.
    InconclusiveNoObservedMetadata,
    /// Screening ran to the stated depth and found no flags.
    NoFlags { depth: ScreeningDepthV0 },
    /// Screening found named signals; this is not a finding of cheating.
    Flagged { signals: Vec<CheatingSignalV0> },
}

/// Stakes-appropriate depth of probabilistic screening.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScreeningDepthV0 {
    /// No statistical screen; authenticity/replay only.
    IntegrityOnly,
    /// Basic consistency checks against observed metadata.
    Standard,
    /// Full statistical screen for high-stakes matches.
    Full,
}

/// A signal that may merit human review, never proof of misconduct.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CheatingSignalV0 {
    /// Sustained reaction distribution outside the stated human profile.
    SuperhumanReactionDistribution,
    /// Distinguishable pre-quantization timing pattern.
    FramePerfectSequence,
    /// Input geometry inconsistent with independently observed information.
    AimInputGeometry,
    /// Input stream reuse or unusually low entropy.
    EntropyCollapse,
    /// Independently observed metadata conflicts with participant claims.
    MetadataInconsistency,
}

/// The report cannot collapse replay validity into claims about fair play.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditReportV0 {
    /// Cryptographic and replay validity of the log.
    pub technical: TechnicalVerdictV0,
    /// Probabilistic assessment based on independently observed metadata.
    pub actual: ActualValidityV0,
}

/// Why an integrity check failed. These variants are safe to expose in a CLI.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum IntegrityFailureV0 {
    /// The log uses a version this implementation does not understand.
    UnsupportedVersion,
    /// Roster is empty, duplicated, or inconsistent with signatures.
    InvalidRoster,
    /// A pre-match player signature is invalid or missing.
    HeaderSignature,
    /// An entry points at the wrong predecessor.
    BrokenChainLink,
    /// An entry body does not match its committed hash.
    EntryHashMismatch,
    /// The revealed seed does not match the signed commitment.
    SeedCommitmentMismatch,
    /// Footer signatures are invalid or incomplete.
    FooterSignature,
}

/// Errors while encoding, decoding, signing, or validating a v0 log.
#[derive(Debug, thiserror::Error)]
pub enum AuditErrorV0 {
    /// The wire value could not be encoded or decoded.
    #[error("match-audit wire encoding failed: {0}")]
    Wire(#[from] postcard::Error),
    /// The format version is not supported.
    #[error("unsupported match-audit format version {0}")]
    UnsupportedVersion(u8),
    /// A cryptographic key or signature failed strict Ed25519 validation.
    #[error("invalid {0} signature")]
    Signature(&'static str),
    /// The participant list cannot be used as a signing roster.
    #[error("invalid match-audit roster")]
    Roster,
    /// A log entry's tick precedes an earlier entry.
    #[error("match-audit entries are not ordered by tick")]
    EntryOrder,
    /// A log entry references a player slot outside the signed roster.
    #[error("match-audit input references an unknown player slot")]
    PlayerSlot,
    /// The input stream has not been signed or is internally inconsistent.
    #[error("match-audit integrity failure: {0:?}")]
    Integrity(IntegrityFailureV0),
}

/// The hash that participants sign before the first match input.
pub fn header_hash(header: &HeaderV0) -> Result<Hash, AuditErrorV0> {
    let bytes = postcard::to_allocvec(header)?;
    Ok(Hash::digest([HEADER_DOMAIN, &bytes].concat()))
}

/// Signs the header with an ID player key.
pub fn sign_header(
    key: &IdSigningKey,
    header: &HeaderV0,
) -> Result<PlayerSignatureV0, AuditErrorV0> {
    let message = [HEADER_DOMAIN, &postcard::to_allocvec(header)?].concat();
    Ok(sign_message(key, &message))
}

/// Builds a stream entry whose hash binds the previous entry and its body.
pub fn chain_entry(previous_hash: Hash, body: EntryBodyV0) -> Result<ChainedEntryV0, AuditErrorV0> {
    let body_bytes = postcard::to_allocvec(&body)?;
    let hash = Hash::digest([ENTRY_DOMAIN, previous_hash.as_bytes(), &body_bytes].concat());
    Ok(ChainedEntryV0 {
        previous_hash,
        hash,
        body,
    })
}

/// Signs the final chain, result claim, seed reveal, and metadata.
pub fn sign_footer(
    key: &IdSigningKey,
    header: &HeaderV0,
    final_chain_hash: Hash,
    metadata: &MetadataV0,
    footer: &FooterV0,
) -> Result<PlayerSignatureV0, AuditErrorV0> {
    let message = footer_message(header, final_chain_hash, metadata, footer)?;
    Ok(sign_message(key, &message))
}

impl MatchLogV0 {
    /// Encodes the complete log using the versioned Postcard wire format.
    pub fn encode(&self) -> Result<Vec<u8>, AuditErrorV0> {
        Ok(postcard::to_allocvec(self)?)
    }

    /// Decodes a complete v0 log. Structural and cryptographic checks are
    /// performed separately by [`MatchLogV0::verify_integrity`].
    pub fn decode(bytes: &[u8]) -> Result<Self, AuditErrorV0> {
        Ok(postcard::from_bytes(bytes)?)
    }

    /// Verifies version, signatures, seed commitment, tick ordering, and the
    /// hash chain. This authenticates the submitted checkpoints; it does not
    /// prove that replay reproduces them.
    pub fn verify_integrity(&self) -> Result<Hash, AuditErrorV0> {
        if self.header.version != FORMAT_VERSION_V0 {
            return Err(AuditErrorV0::UnsupportedVersion(self.header.version));
        }
        validate_roster(&self.header.players)?;
        let header_digest = header_hash(&self.header)?;
        verify_signature_set(
            &self.header.players,
            &self.header_signatures,
            &[HEADER_DOMAIN, &postcard::to_allocvec(&self.header)?].concat(),
            "header",
        )
        .map_err(|_| AuditErrorV0::Integrity(IntegrityFailureV0::HeaderSignature))?;

        let mut previous = header_digest;
        let mut previous_tick = self.header.start_tick;
        for entry in &self.entries {
            if entry.previous_hash != previous {
                return Err(AuditErrorV0::Integrity(IntegrityFailureV0::BrokenChainLink));
            }
            let (first_tick, last_tick) = entry_tick_range(&entry.body)?;
            if first_tick < previous_tick {
                return Err(AuditErrorV0::EntryOrder);
            }
            if let EntryBodyV0::InputBlock(block) = &entry.body {
                if block
                    .records
                    .iter()
                    .any(|input| usize::from(input.player_slot) >= self.header.players.len())
                {
                    return Err(AuditErrorV0::PlayerSlot);
                }
            }
            let expected = chain_entry(previous, entry.body.clone())?.hash;
            if entry.hash != expected {
                return Err(AuditErrorV0::Integrity(
                    IntegrityFailureV0::EntryHashMismatch,
                ));
            }
            previous = entry.hash;
            previous_tick = last_tick;
        }

        if Hash::digest(&self.footer.seed_reveal) != self.header.seed_commitment {
            return Err(AuditErrorV0::Integrity(
                IntegrityFailureV0::SeedCommitmentMismatch,
            ));
        }
        let message = footer_message(&self.header, previous, &self.metadata, &self.footer)?;
        verify_signature_set(
            &self.header.players,
            &self.footer.signatures,
            &message,
            "footer",
        )
        .map_err(|_| AuditErrorV0::Integrity(IntegrityFailureV0::FooterSignature))?;
        Ok(previous)
    }
}

fn entry_tick_range(entry: &EntryBodyV0) -> Result<(u64, u64), AuditErrorV0> {
    match entry {
        EntryBodyV0::InputBlock(block) => {
            let Some(first) = block.records.first() else {
                return Err(AuditErrorV0::EntryOrder);
            };
            if first.tick_delta != 0 {
                return Err(AuditErrorV0::EntryOrder);
            }
            let mut tick = block.first_tick;
            for input in block.records.iter().skip(1) {
                tick = tick
                    .checked_add(input.tick_delta)
                    .ok_or(AuditErrorV0::EntryOrder)?;
            }
            Ok((block.first_tick, tick))
        }
        EntryBodyV0::Event(event) => Ok((event.tick, event.tick)),
        EntryBodyV0::Checkpoint(checkpoint) => Ok((checkpoint.tick, checkpoint.tick)),
    }
}

fn validate_roster(players: &[PlayerV0]) -> Result<(), AuditErrorV0> {
    if players.is_empty() {
        return Err(AuditErrorV0::Roster);
    }
    let mut seen = std::collections::HashSet::with_capacity(players.len());
    if players.iter().any(|player| !seen.insert(player.id)) {
        return Err(AuditErrorV0::Roster);
    }
    Ok(())
}

fn verify_signature_set(
    players: &[PlayerV0],
    signatures: &[PlayerSignatureV0],
    message: &[u8],
    kind: &'static str,
) -> Result<(), AuditErrorV0> {
    if signatures.len() != players.len() {
        return Err(AuditErrorV0::Signature(kind));
    }
    let mut seen = std::collections::HashSet::with_capacity(signatures.len());
    for signed in signatures {
        if !players.iter().any(|player| player.id == signed.player) || !seen.insert(signed.player) {
            return Err(AuditErrorV0::Signature(kind));
        }
        let key =
            VerifyingKey::from_bytes(&signed.player).map_err(|_| AuditErrorV0::Signature(kind))?;
        let signature =
            Signature::from_slice(&signed.signature).map_err(|_| AuditErrorV0::Signature(kind))?;
        key.verify_strict(message, &signature)
            .map_err(|_| AuditErrorV0::Signature(kind))?;
    }
    Ok(())
}

fn sign_message(key: &IdSigningKey, message: &[u8]) -> PlayerSignatureV0 {
    let signing_key = SigningKey::from_bytes(&key.to_bytes());
    PlayerSignatureV0 {
        player: *signing_key.verifying_key().as_bytes(),
        signature: signing_key.sign(message).to_bytes().to_vec(),
    }
}

fn footer_message(
    header: &HeaderV0,
    final_chain_hash: Hash,
    metadata: &MetadataV0,
    footer: &FooterV0,
) -> Result<Vec<u8>, AuditErrorV0> {
    let header_digest = header_hash(header)?;
    let footer_claim = postcard::to_allocvec(&(
        footer.final_tick,
        footer.final_state_hash,
        &footer.result_claim,
        &footer.seed_reveal,
        metadata,
    ))?;
    Ok([
        FOOTER_DOMAIN,
        header_digest.as_bytes(),
        final_chain_hash.as_bytes(),
        &footer_claim,
    ]
    .concat())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::*;

    fn key(seed: u8) -> IdSigningKey {
        IdSigningKey::from_bytes(&[seed; 32])
    }

    fn fixture() -> MatchLogV0 {
        let players = [key(1), key(2)];
        let seed = b"tic-tac-toe seed".to_vec();
        let header = HeaderV0 {
            version: FORMAT_VERSION_V0,
            match_id: "fixture-1".to_owned(),
            program_id: "tic-tac-toe".to_owned(),
            program_hash: Hash::digest(b"program"),
            rules_hash: Hash::digest(b"rules"),
            tick_rate: 1,
            rng_algorithm: "none/v1".to_owned(),
            seed_commitment: Hash::digest(&seed),
            initial_state_hash: Hash::digest(b"empty board"),
            players: players
                .iter()
                .enumerate()
                .map(|(index, key)| PlayerV0 {
                    id: *key.verifying_key().as_bytes(),
                    role: if index == 0 { "x" } else { "o" }.to_owned(),
                })
                .collect(),
            start_tick: 0,
        };
        let header_signatures = players
            .iter()
            .map(|key| sign_header(key, &header).unwrap())
            .collect();
        let first = chain_entry(
            header_hash(&header).unwrap(),
            EntryBodyV0::InputBlock(InputBlockV0 {
                first_tick: 1,
                records: vec![InputRecordV0 {
                    tick_delta: 0,
                    player_slot: 0,
                    payload: b"place:0".to_vec(),
                }],
            }),
        )
        .unwrap();
        let second = chain_entry(
            first.hash,
            EntryBodyV0::Checkpoint(CheckpointV0 {
                tick: 1,
                state_hash: Hash::digest(b"X........"),
                snapshot: None,
            }),
        )
        .unwrap();
        let metadata = MetadataV0 {
            self_reported: vec![MetadataRecordV0 {
                key: "client_build".to_owned(),
                value: "build-1".to_owned(),
                source: MetadataSourceV0::SelfReported,
            }],
            evidence: vec![],
        };
        let mut footer = FooterV0 {
            final_tick: 1,
            final_state_hash: Hash::digest(b"X........"),
            result_claim: b"x-wins".to_vec(),
            seed_reveal: seed,
            signatures: vec![],
        };
        footer.signatures = players
            .iter()
            .map(|key| sign_footer(key, &header, second.hash, &metadata, &footer).unwrap())
            .collect();
        MatchLogV0 {
            header,
            header_signatures,
            entries: vec![first, second],
            metadata,
            footer,
        }
    }

    #[test]
    fn signed_log_round_trips_and_verifies() {
        let log = fixture();
        let bytes = log.encode().unwrap();
        let decoded = MatchLogV0::decode(&bytes).unwrap();
        assert_eq!(decoded, log);
        assert_eq!(decoded.verify_integrity().unwrap(), decoded.entries[1].hash);
    }

    #[test]
    fn edited_input_is_rejected_by_chain_verification() {
        let mut log = fixture();
        let EntryBodyV0::InputBlock(block) = &mut log.entries[0].body else {
            panic!("fixture starts with input")
        };
        block.records[0].payload[0] ^= 1;
        assert!(matches!(
            log.verify_integrity(),
            Err(AuditErrorV0::Integrity(
                IntegrityFailureV0::EntryHashMismatch
            ))
        ));
    }

    #[test]
    fn edited_checkpoint_is_rejected_by_chain_verification() {
        let mut log = fixture();
        let EntryBodyV0::Checkpoint(checkpoint) = &mut log.entries[1].body else {
            panic!("fixture ends with checkpoint")
        };
        checkpoint.state_hash = Hash::digest(b"tampered state");
        assert!(matches!(
            log.verify_integrity(),
            Err(AuditErrorV0::Integrity(
                IntegrityFailureV0::EntryHashMismatch
            ))
        ));
    }
}
