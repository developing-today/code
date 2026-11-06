//! Versioned, signed match-audit log primitives.
//!
//! This is the format and integrity layer only. It does not claim that a
//! signed log is a fair match: replay validity and observed-metadata screening
//! are separate verdicts in [`AuditReportV0`]. The wire encoding is versioned
//! Postcard; changing field order or meaning requires a new format version.

use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use p2panda_core::{Hash, SigningKey as IdSigningKey};
use serde::{Deserialize, Serialize};

#[cfg(feature = "sandbox")]
use crate::sandbox::{Sandbox, SandboxLimits};
#[cfg(feature = "world")]
use crate::world::signed::MoveChain;

const HEADER_DOMAIN: &[u8] = b"id.match-audit.v0/header\0";
const ENTRY_DOMAIN: &[u8] = b"id.match-audit.v0/entry\0";
const FOOTER_DOMAIN: &[u8] = b"id.match-audit.v0/footer\0";

/// Stable program identifier used by the bundled tic-tac-toe reference.
pub const TIC_TAC_TOE_REFERENCE_ID: &str = "id.apps.tic-tac-toe/reference-v0";
/// Bundled Wasm build used by the reference exporter and verifier.
pub const TIC_TAC_TOE_REFERENCE_WASM: &[u8] =
    include_bytes!("../apps/tic-tac-toe/program/tic-tac-toe.wasm");
const TIC_TAC_TOE_RULES_V0: &[u8] =
    b"tic-tac-toe/v0; cells=0..8; x-first; invalid-occupied-post-win-ignored";
/// RNG identifier for the reference match's eight-byte little-endian seed.
pub const U64_LE_RNG_V0: &str = "id.seed.u64-le/v0";

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
    /// World identifier used by the player-signed move events.
    pub world_id: String,
    /// Stable identifier for the program implementing the match rules.
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
    /// The player's already-signed `world.move` event from #2814.
    pub signed_move: Vec<u8>,
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

/// Metadata records retain their claimed provenance instead of conflating
/// participant reports with independent observations.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetadataRecordV0 {
    /// Stable field name, e.g. `client_build` or `rtt_p95_ms`.
    pub key: String,
    /// Text representation defined by the named field's schema.
    pub value: String,
    /// Who supplied or derived this value.
    pub source: MetadataSourceV0,
}

/// Provenance category claimed for a match metadata value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MetadataSourceV0 {
    /// Supplied by a participant; useful only as a consistency check.
    SelfReported,
    /// Labeled as independently observed by another player.
    PeerObserved,
    /// Labeled as independently observed by a relay.
    RelayObserved,
    /// Labeled as independently observed by a designated witness.
    WitnessObserved,
    /// Derived from the authenticated input stream.
    Derived,
}

/// Metadata stays separate from the technically-valid replay result.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetadataV0 {
    /// Participant-supplied values, never evidence of fairness by themselves.
    pub self_reported: Vec<MetadataRecordV0>,
    /// Values labeled as observed or log-derived, with provenance retained.
    /// These labels are claims until separately countersigned by their source.
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
    /// Metadata with self-reports and source-labeled records kept separate.
    pub metadata: MetadataV0,
    /// Signed final result claim and seed reveal.
    pub footer: FooterV0,
}

/// A closed L0–L2 technical verdict vocabulary. `ResultConfirmed` is reserved
/// for full-history replay; sampled success is never a result confirmation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TechnicalVerdictV0 {
    /// The log failed parsing, provenance, signature, or chain validation.
    InvalidRecord {
        /// Closed reason code for the rejected record.
        reason: IntegrityFailureV0,
    },
    /// Full-history replay reproduced the claimed final state.
    ResultConfirmed {
        /// Runtime scope for this full-history result.
        runtime: RuntimeScopeV0,
    },
    /// Replay diverged; interval endpoints name checkpoints, not exact ticks.
    ReplayDiverges {
        /// The first checkpoint interval whose state hashes differ.
        first_mismatching_interval: CheckpointIntervalV0,
    },
    /// Only the listed sampled intervals were replayed successfully.
    PartiallyVerified {
        /// Intervals replayed successfully; this does not confirm the result.
        intervals: Vec<CheckpointIntervalV0>,
        /// Runtime scope for the sampled replays.
        runtime: RuntimeScopeV0,
    },
    /// Verification could not make a claim because a required input was absent.
    Inconclusive {
        /// Closed reason code for the missing verification input.
        reason: InconclusiveReasonV0,
    },
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
    /// The log's signed rules hash is not supported by this verifier.
    UnsupportedRules,
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
    /// Observations exist, but no L3 screening policy was applied.
    InconclusiveScreeningUnavailable,
    /// Screening ran to the stated depth and found no flags.
    NoFlags {
        /// Screening depth at which no flags were found.
        depth: ScreeningDepthV0,
    },
    /// Screening found named signals; this is not a finding of cheating.
    Flagged {
        /// Signals found; each is probabilistic and may have false positives.
        signals: Vec<CheatingSignalV0>,
    },
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
    /// Actual-play screening status, never inferred from technical validity.
    pub actual: ActualValidityV0,
}

/// Why an integrity check failed. These variants are safe to expose in a CLI.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum IntegrityFailureV0 {
    /// The byte stream could not be decoded as a v0 log.
    MalformedEncoding,
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
    /// An input does not contain a valid signed move by its roster player.
    SignedInput,
    /// The footer's final tick does not match the ordered event stream.
    FooterTickMismatch,
    /// An entry's tick ordering or block encoding is invalid.
    EntryOrder,
    /// An input refers to a player slot outside the signed roster.
    PlayerSlot,
    /// The seed reveal does not use the encoding named by the header.
    SeedRevealEncoding,
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
    /// The signing key does not belong to any player in the header roster.
    #[error("match-audit signer is not in the header roster")]
    SignerNotInRoster,
    /// The footer seed reveal does not match the header seed commitment.
    #[error("match-audit seed reveal does not match the header seed commitment")]
    SeedCommitmentMismatch,
    /// The reference exporter or verifier could not run its pinned module.
    #[error("match-audit reference runtime failed: {0}")]
    ReferenceRuntime(String),
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
    Ok(hash_parts(&[HEADER_DOMAIN, &bytes]))
}

/// Refuses to sign for a caller whose key is not on the header roster:
/// the signature set a log carries is exactly its roster, so a non-roster
/// signature is dead on arrival at verification.
fn validate_signer(key: &IdSigningKey, header: &HeaderV0) -> Result<(), AuditErrorV0> {
    let player = *key.verifying_key().as_bytes();
    if header.players.iter().any(|roster| roster.id == player) {
        Ok(())
    } else {
        Err(AuditErrorV0::SignerNotInRoster)
    }
}

/// Signs the header with an ID player key.
///
/// # Errors
///
/// Returns [`AuditErrorV0::UnsupportedVersion`] for a header version this
/// implementation does not understand, [`AuditErrorV0::Roster`] for an
/// empty or duplicate-key roster, and [`AuditErrorV0::SignerNotInRoster`]
/// when the caller's key is absent from `header.players` — the same
/// header-local rules [`MatchLogV0::verify_integrity`] applies, checked
/// before a signature exists instead of after.
pub fn sign_header(
    key: &IdSigningKey,
    header: &HeaderV0,
) -> Result<PlayerSignatureV0, AuditErrorV0> {
    if header.version != FORMAT_VERSION_V0 {
        return Err(AuditErrorV0::UnsupportedVersion(header.version));
    }
    validate_roster(&header.players)?;
    validate_signer(key, header)?;
    let message = [HEADER_DOMAIN, &postcard::to_allocvec(header)?].concat();
    Ok(sign_message(key, &message))
}

/// Builds a stream entry whose hash binds the previous entry and its body.
pub fn chain_entry(previous_hash: Hash, body: EntryBodyV0) -> Result<ChainedEntryV0, AuditErrorV0> {
    let hash = entry_hash(previous_hash, &body)?;
    Ok(ChainedEntryV0 {
        previous_hash,
        hash,
        body,
    })
}

fn entry_hash(previous_hash: Hash, body: &EntryBodyV0) -> Result<Hash, AuditErrorV0> {
    let body_bytes = postcard::to_allocvec(body)?;
    Ok(hash_parts(&[
        ENTRY_DOMAIN,
        previous_hash.as_bytes(),
        &body_bytes,
    ]))
}

fn hash_parts(parts: &[&[u8]]) -> Hash {
    let mut hasher = blake3::Hasher::new();
    for part in parts {
        hasher.update(part);
    }
    Hash::from_bytes(*hasher.finalize().as_bytes())
}

/// Signs the final chain, result claim, seed reveal, and metadata.
///
/// # Errors
///
/// Returns [`AuditErrorV0::SignerNotInRoster`] when the caller's key is
/// absent from `header.players`, and
/// [`AuditErrorV0::SeedCommitmentMismatch`] when the digest of
/// `footer.seed_reveal` differs from `header.seed_commitment` — the
/// footer-local rules [`MatchLogV0::verify_integrity`] applies, checked
/// before a signature exists instead of after.
pub fn sign_footer(
    key: &IdSigningKey,
    header: &HeaderV0,
    final_chain_hash: Hash,
    metadata: &MetadataV0,
    footer: &FooterV0,
) -> Result<PlayerSignatureV0, AuditErrorV0> {
    validate_signer(key, header)?;
    if Hash::digest(&footer.seed_reveal) != header.seed_commitment {
        return Err(AuditErrorV0::SeedCommitmentMismatch);
    }
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
        #[cfg(feature = "world")]
        let mut move_chains = vec![MoveChain::new(); self.header.players.len()];
        for entry in &self.entries {
            if entry.previous_hash != previous {
                return Err(AuditErrorV0::Integrity(IntegrityFailureV0::BrokenChainLink));
            }
            let (first_tick, last_tick) = entry_tick_range(&entry.body)?;
            if first_tick < previous_tick {
                return Err(AuditErrorV0::EntryOrder);
            }
            let expected = entry_hash(previous, &entry.body)?;
            if entry.hash != expected {
                return Err(AuditErrorV0::Integrity(
                    IntegrityFailureV0::EntryHashMismatch,
                ));
            }
            if let EntryBodyV0::InputBlock(block) = &entry.body {
                for input in &block.records {
                    let slot = usize::from(input.player_slot);
                    let Some(player) = self.header.players.get(slot) else {
                        return Err(AuditErrorV0::PlayerSlot);
                    };
                    #[cfg(not(feature = "world"))]
                    {
                        let _ = player;
                        return Err(AuditErrorV0::Integrity(IntegrityFailureV0::SignedInput));
                    }
                    #[cfg(feature = "world")]
                    move_chains[slot]
                        .verify_and_accept(
                            &input.signed_move,
                            &self.header.world_id,
                            &input.payload,
                            &player.id,
                        )
                        .map_err(|_| AuditErrorV0::Integrity(IntegrityFailureV0::SignedInput))?;
                }
            }
            previous = entry.hash;
            previous_tick = last_tick;
        }

        if self.footer.final_tick != previous_tick {
            return Err(AuditErrorV0::Integrity(
                IntegrityFailureV0::FooterTickMismatch,
            ));
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

/// Exports the bundled tic-tac-toe reference game as a signed v0 transcript.
///
/// The caller supplies the two players' ID keys. The exporter runs the exact
/// checked-in Wasm build, signs each move using the existing #2814
/// `world.move` format, and records a checkpoint after each move. The output
/// contains no observed metadata; its actual-validity assessment therefore
/// remains inconclusive.
///
/// # Errors
///
/// Returns an encoding or signing error, or a reference-runtime error if the
/// bundled Wasm cannot run or does not expose the expected snapshot/records.
#[cfg(feature = "sandbox")]
pub fn export_tic_tac_toe_reference(
    player_keys: [&IdSigningKey; 2],
    match_id: impl Into<String>,
    world_id: impl Into<String>,
    seed: u64,
) -> Result<MatchLogV0, AuditErrorV0> {
    let runner = Sandbox::compile(TIC_TAC_TOE_REFERENCE_WASM, SandboxLimits::default())
        .map_err(|error| AuditErrorV0::ReferenceRuntime(error.to_string()))?;
    let mut world = runner
        .instantiate(seed)
        .map_err(|error| AuditErrorV0::ReferenceRuntime(error.to_string()))?;
    let initial_snapshot = world
        .snapshot()
        .map_err(|error| AuditErrorV0::ReferenceRuntime(error.to_string()))?
        .ok_or_else(|| AuditErrorV0::ReferenceRuntime("reference has no snapshots".to_owned()))?;

    let seed_reveal = seed.to_le_bytes().to_vec();
    let header = HeaderV0 {
        version: FORMAT_VERSION_V0,
        match_id: match_id.into(),
        world_id: world_id.into(),
        program_id: TIC_TAC_TOE_REFERENCE_ID.to_owned(),
        program_hash: Hash::digest(TIC_TAC_TOE_REFERENCE_WASM),
        rules_hash: Hash::digest(TIC_TAC_TOE_RULES_V0),
        tick_rate: 1,
        rng_algorithm: U64_LE_RNG_V0.to_owned(),
        seed_commitment: Hash::digest(&seed_reveal),
        initial_state_hash: Hash::digest(&initial_snapshot),
        players: player_keys
            .iter()
            .enumerate()
            .map(|(index, key)| PlayerV0 {
                id: *key.verifying_key().as_bytes(),
                role: if index == 0 { "x" } else { "o" }.to_owned(),
            })
            .collect(),
        start_tick: 0,
    };
    let header_signatures = player_keys
        .iter()
        .map(|key| sign_header(key, &header))
        .collect::<Result<Vec<_>, _>>()?;

    let mut entries = Vec::new();
    let mut previous = header_hash(&header)?;
    let mut move_sequences = [0_u32; 2];
    let mut move_frontiers: [Option<Hash>; 2] = [None, None];
    let mut final_snapshot = initial_snapshot;

    // X wins on the final move: X0, O3, X1, O4, X2.
    for (index, cell) in [0_u8, 3, 1, 4, 2].into_iter().enumerate() {
        let slot = index % 2;
        let tick = u64::try_from(index + 1)
            .map_err(|_| AuditErrorV0::ReferenceRuntime("tick overflow".to_owned()))?;
        let payload = vec![b'0' + cell];
        let signed_move = crate::world::signed::sign_move(
            player_keys[slot],
            &header.world_id,
            move_sequences[slot],
            move_frontiers[slot],
            &payload,
        )
        .map_err(|error| AuditErrorV0::ReferenceRuntime(error.to_string()))?;
        move_sequences[slot] += 1;
        move_frontiers[slot] = Some(signed_move.id);

        let input = InputRecordV0 {
            tick_delta: 0,
            player_slot: u16::try_from(slot)
                .map_err(|_| AuditErrorV0::ReferenceRuntime("player slot overflow".to_owned()))?,
            payload: payload.clone(),
            signed_move: signed_move.bytes,
        };
        let input_entry = chain_entry(
            previous,
            EntryBodyV0::InputBlock(InputBlockV0 {
                first_tick: tick,
                records: vec![input],
            }),
        )?;
        previous = input_entry.hash;
        entries.push(input_entry);

        world
            .update(slot as u64, &payload)
            .map_err(|error| AuditErrorV0::ReferenceRuntime(error.to_string()))?;
        final_snapshot = world
            .snapshot()
            .map_err(|error| AuditErrorV0::ReferenceRuntime(error.to_string()))?
            .ok_or_else(|| {
                AuditErrorV0::ReferenceRuntime("reference has no snapshots".to_owned())
            })?;
        let checkpoint = chain_entry(
            previous,
            EntryBodyV0::Checkpoint(CheckpointV0 {
                tick,
                state_hash: Hash::digest(&final_snapshot),
                snapshot: Some(final_snapshot.clone()),
            }),
        )?;
        previous = checkpoint.hash;
        entries.push(checkpoint);
    }

    let metadata = MetadataV0::default();
    let result_claim = world
        .records()
        .map_err(|error| AuditErrorV0::ReferenceRuntime(error.to_string()))?
        .ok_or_else(|| {
            AuditErrorV0::ReferenceRuntime("reference has no result records".to_owned())
        })?;
    let mut footer = FooterV0 {
        final_tick: 5,
        final_state_hash: Hash::digest(&final_snapshot),
        result_claim,
        seed_reveal,
        signatures: Vec::new(),
    };
    footer.signatures = player_keys
        .iter()
        .map(|key| sign_footer(key, &header, previous, &metadata, &footer))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(MatchLogV0 {
        header,
        header_signatures,
        entries,
        metadata,
        footer,
    })
}

/// Verifies a v0 log against the bundled tic-tac-toe Wasm build.
///
/// Cryptographic failures are `InvalidRecord`; snapshot/result disagreement is
/// `ReplayDiverges`; unavailable builds or replay capabilities are
/// `Inconclusive`. Successful L2 results are always scoped to the pinned
/// runtime. The independent `actual` field remains inconclusive unless a
/// separate L3 screening implementation is added.
#[cfg(feature = "sandbox")]
pub fn verify_tic_tac_toe_v0(bytes: &[u8]) -> AuditReportV0 {
    let log = match MatchLogV0::decode(bytes) {
        Ok(log) => log,
        Err(error) => {
            return report(
                TechnicalVerdictV0::InvalidRecord {
                    reason: integrity_reason(&error),
                },
                None,
            );
        }
    };
    let actual = actual_validity(&log.metadata);
    if let Err(error) = log.verify_integrity() {
        return AuditReportV0 {
            technical: TechnicalVerdictV0::InvalidRecord {
                reason: integrity_reason(&error),
            },
            actual,
        };
    }
    let technical = replay_tic_tac_toe(&log);
    AuditReportV0 { technical, actual }
}

#[cfg(feature = "sandbox")]
fn replay_tic_tac_toe(log: &MatchLogV0) -> TechnicalVerdictV0 {
    let inconclusive = |reason| TechnicalVerdictV0::Inconclusive { reason };
    if log.header.program_id != TIC_TAC_TOE_REFERENCE_ID
        || log.header.program_hash != Hash::digest(TIC_TAC_TOE_REFERENCE_WASM)
    {
        return inconclusive(InconclusiveReasonV0::MissingProgramBuild);
    }
    if log.header.rules_hash != Hash::digest(TIC_TAC_TOE_RULES_V0) {
        return inconclusive(InconclusiveReasonV0::UnsupportedRules);
    }
    if log.header.tick_rate != 1 || log.header.rng_algorithm != U64_LE_RNG_V0 {
        return inconclusive(InconclusiveReasonV0::MissingReplayCapability);
    }
    let seed_bytes: [u8; 8] = match log.footer.seed_reveal.as_slice().try_into() {
        Ok(bytes) => bytes,
        Err(_) => {
            return TechnicalVerdictV0::InvalidRecord {
                reason: IntegrityFailureV0::SeedRevealEncoding,
            };
        }
    };
    let seed = u64::from_le_bytes(seed_bytes);
    let runner = match Sandbox::compile(TIC_TAC_TOE_REFERENCE_WASM, SandboxLimits::default()) {
        Ok(runner) => runner,
        Err(_) => return inconclusive(InconclusiveReasonV0::MissingReplayCapability),
    };
    let mut world = match runner.instantiate(seed) {
        Ok(world) => world,
        Err(_) => return inconclusive(InconclusiveReasonV0::MissingReplayCapability),
    };
    let Some(initial_snapshot) = snapshot(&mut world) else {
        return inconclusive(InconclusiveReasonV0::MissingReplayCapability);
    };
    if Hash::digest(&initial_snapshot) != log.header.initial_state_hash {
        return diverges(log.header.start_tick, log.header.start_tick);
    }

    let mut last_checkpoint_tick = log.header.start_tick;
    for entry in &log.entries {
        match &entry.body {
            EntryBodyV0::InputBlock(block) => {
                for input in &block.records {
                    if world
                        .update(u64::from(input.player_slot), &input.payload)
                        .is_err()
                    {
                        return inconclusive(InconclusiveReasonV0::MissingReplayCapability);
                    }
                }
            }
            EntryBodyV0::Event(_) => {
                return inconclusive(InconclusiveReasonV0::MissingReplayCapability);
            }
            EntryBodyV0::Checkpoint(checkpoint) => {
                let Some(state) = snapshot(&mut world) else {
                    return inconclusive(InconclusiveReasonV0::MissingReplayCapability);
                };
                if Hash::digest(&state) != checkpoint.state_hash
                    || checkpoint
                        .snapshot
                        .as_ref()
                        .is_some_and(|expected| expected != &state)
                {
                    return diverges(last_checkpoint_tick, checkpoint.tick);
                }
                last_checkpoint_tick = checkpoint.tick;
            }
        }
    }

    let Some(final_state) = snapshot(&mut world) else {
        return inconclusive(InconclusiveReasonV0::MissingReplayCapability);
    };
    let interval_start = last_checkpoint_tick.min(log.footer.final_tick);
    if Hash::digest(&final_state) != log.footer.final_state_hash {
        return diverges(interval_start, log.footer.final_tick);
    }
    match world.records() {
        Ok(Some(records)) if records == log.footer.result_claim => {}
        Ok(Some(_)) => return diverges(interval_start, log.footer.final_tick),
        Ok(_) => return inconclusive(InconclusiveReasonV0::MissingReplayCapability),
        Err(_) => return inconclusive(InconclusiveReasonV0::MissingReplayCapability),
    }
    TechnicalVerdictV0::ResultConfirmed {
        runtime: RuntimeScopeV0::PinnedRuntime,
    }
}

#[cfg(feature = "sandbox")]
fn snapshot(world: &mut crate::sandbox::WorldInstance) -> Option<Vec<u8>> {
    world.snapshot().ok().flatten()
}

#[cfg(feature = "sandbox")]
fn diverges(start_tick: u64, end_tick: u64) -> TechnicalVerdictV0 {
    TechnicalVerdictV0::ReplayDiverges {
        first_mismatching_interval: CheckpointIntervalV0 {
            start_tick,
            end_tick,
        },
    }
}

#[cfg(feature = "sandbox")]
fn report(technical: TechnicalVerdictV0, metadata: Option<&MetadataV0>) -> AuditReportV0 {
    AuditReportV0 {
        technical,
        actual: metadata.map_or(
            ActualValidityV0::InconclusiveNoObservedMetadata,
            actual_validity,
        ),
    }
}

#[cfg(feature = "sandbox")]
fn actual_validity(_metadata: &MetadataV0) -> ActualValidityV0 {
    // V0 records provenance labels but has no observer countersignature yet.
    // Treating a participant-signed label as an independent observation would
    // conflate a claim about play with evidence of play.
    ActualValidityV0::InconclusiveNoObservedMetadata
}

#[cfg(feature = "sandbox")]
fn integrity_reason(error: &AuditErrorV0) -> IntegrityFailureV0 {
    match error {
        AuditErrorV0::Wire(_) => IntegrityFailureV0::MalformedEncoding,
        AuditErrorV0::UnsupportedVersion(_) => IntegrityFailureV0::UnsupportedVersion,
        AuditErrorV0::Signature("header") => IntegrityFailureV0::HeaderSignature,
        AuditErrorV0::Signature("footer") => IntegrityFailureV0::FooterSignature,
        AuditErrorV0::Signature(_) => IntegrityFailureV0::HeaderSignature,
        AuditErrorV0::Roster => IntegrityFailureV0::InvalidRoster,
        AuditErrorV0::SignerNotInRoster => IntegrityFailureV0::InvalidRoster,
        AuditErrorV0::SeedCommitmentMismatch => IntegrityFailureV0::SeedCommitmentMismatch,
        AuditErrorV0::EntryOrder => IntegrityFailureV0::EntryOrder,
        AuditErrorV0::PlayerSlot => IntegrityFailureV0::PlayerSlot,
        AuditErrorV0::Integrity(reason) => *reason,
        AuditErrorV0::ReferenceRuntime(_) => IntegrityFailureV0::MalformedEncoding,
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
    let roster: std::collections::HashSet<_> = players.iter().map(|player| player.id).collect();
    let mut seen = std::collections::HashSet::with_capacity(signatures.len());
    for signed in signatures {
        if !roster.contains(&signed.player) || !seen.insert(signed.player) {
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
    let signing_key = SigningKey::from_bytes(&key.as_bytes());
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

#[cfg(all(test, feature = "sandbox"))]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

    use super::*;

    fn key(seed: u8) -> IdSigningKey {
        IdSigningKey::from_bytes(&[seed; 32])
    }

    #[test]
    fn streaming_hash_preserves_concatenated_v0_digest() {
        let parts: [&[u8]; 3] = [b"domain", b"previous", b"payload"];
        assert_eq!(hash_parts(&parts), Hash::digest(parts.concat()));
    }

    fn fixture() -> MatchLogV0 {
        let players = [key(1), key(2)];
        let seed = b"tic-tac-toe seed".to_vec();
        let header = HeaderV0 {
            version: FORMAT_VERSION_V0,
            match_id: "fixture-1".to_owned(),
            world_id: "fixture-world".to_owned(),
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
        let signed_move =
            crate::world::signed::sign_move(&players[0], &header.world_id, 0, None, b"place:0")
                .unwrap()
                .bytes;
        let first = chain_entry(
            header_hash(&header).unwrap(),
            EntryBodyV0::InputBlock(InputBlockV0 {
                first_tick: 1,
                records: vec![InputRecordV0 {
                    tick_delta: 0,
                    player_slot: 0,
                    payload: b"place:0".to_vec(),
                    signed_move,
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
    fn sign_header_refuses_unsupported_version() {
        let log = fixture();
        let mut header = log.header.clone();
        header.version = FORMAT_VERSION_V0 + 1;
        let error = sign_header(&key(1), &header).unwrap_err();
        assert!(
            matches!(error, AuditErrorV0::UnsupportedVersion(version) if version == FORMAT_VERSION_V0 + 1)
        );
    }

    #[test]
    fn sign_header_refuses_empty_roster() {
        let log = fixture();
        let mut header = log.header.clone();
        header.players.clear();
        let error = sign_header(&key(1), &header).unwrap_err();
        assert!(matches!(error, AuditErrorV0::Roster));
    }

    #[test]
    fn sign_header_refuses_duplicate_roster() {
        let log = fixture();
        let mut header = log.header.clone();
        let duplicate = header.players[0].clone();
        header.players.push(duplicate);
        let error = sign_header(&key(1), &header).unwrap_err();
        assert!(matches!(error, AuditErrorV0::Roster));
    }

    #[test]
    fn sign_header_refuses_non_roster_signer() {
        let log = fixture();
        let error = sign_header(&key(3), &log.header).unwrap_err();
        assert!(matches!(error, AuditErrorV0::SignerNotInRoster));
    }

    #[test]
    fn sign_footer_refuses_non_roster_signer() {
        let log = fixture();
        let error = sign_footer(
            &key(3),
            &log.header,
            log.entries[1].hash,
            &log.metadata,
            &log.footer,
        )
        .unwrap_err();
        assert!(matches!(error, AuditErrorV0::SignerNotInRoster));
    }

    #[test]
    fn sign_footer_refuses_seed_commitment_mismatch() {
        let log = fixture();
        let mut footer = log.footer.clone();
        footer.seed_reveal = b"a different seed".to_vec();
        let error = sign_footer(
            &key(1),
            &log.header,
            log.entries[1].hash,
            &log.metadata,
            &footer,
        )
        .unwrap_err();
        assert!(matches!(error, AuditErrorV0::SeedCommitmentMismatch));
    }

    #[test]
    fn sign_header_and_footer_accept_roster_signers() {
        let log = fixture();
        for player in [key(1), key(2)] {
            assert!(sign_header(&player, &log.header).is_ok());
            assert!(
                sign_footer(
                    &player,
                    &log.header,
                    log.entries[1].hash,
                    &log.metadata,
                    &log.footer
                )
                .is_ok()
            );
        }
    }

    fn reseal(log: &mut MatchLogV0, players: &[IdSigningKey; 2]) {
        log.header_signatures = players
            .iter()
            .map(|key| sign_header(key, &log.header).unwrap())
            .collect();
        let mut previous = header_hash(&log.header).unwrap();
        for entry in &mut log.entries {
            *entry = chain_entry(previous, entry.body.clone()).unwrap();
            previous = entry.hash;
        }
        let mut footer = log.footer.clone();
        footer.signatures = players
            .iter()
            .map(|key| sign_footer(key, &log.header, previous, &log.metadata, &footer).unwrap())
            .collect();
        log.footer = footer;
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

    #[test]
    fn input_must_match_its_players_signed_move() {
        let players = [key(1), key(2)];
        let mut log = fixture();
        let EntryBodyV0::InputBlock(block) = &mut log.entries[0].body else {
            panic!("fixture starts with input")
        };
        block.records[0].player_slot = 1;
        reseal(&mut log, &players);

        assert!(matches!(
            log.verify_integrity(),
            Err(AuditErrorV0::Integrity(IntegrityFailureV0::SignedInput))
        ));
    }

    #[test]
    fn footer_tick_must_match_the_stream() {
        let players = [key(1), key(2)];
        let mut log = fixture();
        log.footer.final_tick = 2;
        reseal(&mut log, &players);

        assert!(matches!(
            log.verify_integrity(),
            Err(AuditErrorV0::Integrity(
                IntegrityFailureV0::FooterTickMismatch
            ))
        ));
    }

    #[cfg(feature = "sandbox")]
    #[test]
    fn reference_export_replays_and_keeps_actual_validity_inconclusive() {
        let players = [key(1), key(2)];
        let log = export_tic_tac_toe_reference(
            [&players[0], &players[1]],
            "reference-match-1",
            "reference-world-1",
            7,
        )
        .unwrap();
        let report = verify_tic_tac_toe_v0(&log.encode().unwrap());

        assert_eq!(
            report.technical,
            TechnicalVerdictV0::ResultConfirmed {
                runtime: RuntimeScopeV0::PinnedRuntime,
            }
        );
        assert_eq!(
            report.actual,
            ActualValidityV0::InconclusiveNoObservedMetadata
        );
        assert!(log.entries.iter().any(|entry| matches!(
            &entry.body,
            EntryBodyV0::InputBlock(block)
                if block.records.iter().all(|input| !input.signed_move.is_empty())
        )));
    }

    #[cfg(feature = "sandbox")]
    #[test]
    fn replayed_checkpoint_tampering_uses_the_divergence_verdict() {
        let players = [key(1), key(2)];
        let mut log = export_tic_tac_toe_reference(
            [&players[0], &players[1]],
            "reference-match-2",
            "reference-world-2",
            7,
        )
        .unwrap();
        let EntryBodyV0::Checkpoint(checkpoint) = &mut log.entries[1].body else {
            panic!("second entry is the first checkpoint")
        };
        checkpoint.state_hash = Hash::digest(b"tampered checkpoint");
        reseal(&mut log, &players);

        let report = verify_tic_tac_toe_v0(&log.encode().unwrap());
        assert!(matches!(
            report.technical,
            TechnicalVerdictV0::ReplayDiverges {
                first_mismatching_interval: CheckpointIntervalV0 {
                    start_tick: 0,
                    end_tick: 1,
                }
            }
        ));
    }

    #[test]
    fn unsupported_rules_hash_is_inconclusive_even_when_the_log_is_signed() {
        let players = [key(1), key(2)];
        let mut log = export_tic_tac_toe_reference(
            [&players[0], &players[1]],
            "reference-match-rules",
            "reference-world-rules",
            7,
        )
        .unwrap();
        log.header.rules_hash = Hash::digest(b"different rules");
        reseal(&mut log, &players);

        let report = verify_tic_tac_toe_v0(&log.encode().unwrap());
        assert_eq!(
            report.technical,
            TechnicalVerdictV0::Inconclusive {
                reason: InconclusiveReasonV0::UnsupportedRules,
            }
        );
    }

    #[test]
    fn replayed_result_claim_mismatch_is_a_divergence() {
        let players = [key(1), key(2)];
        let mut log = export_tic_tac_toe_reference(
            [&players[0], &players[1]],
            "reference-match-result",
            "reference-world-result",
            7,
        )
        .unwrap();
        log.footer.result_claim = b"claimed a different result".to_vec();
        reseal(&mut log, &players);

        let report = verify_tic_tac_toe_v0(&log.encode().unwrap());
        assert!(matches!(
            report.technical,
            TechnicalVerdictV0::ReplayDiverges {
                first_mismatching_interval: CheckpointIntervalV0 {
                    start_tick: 5,
                    end_tick: 5,
                }
            }
        ));
    }

    #[test]
    fn participant_signed_observation_labels_do_not_prove_actual_validity() {
        let metadata = MetadataV0 {
            self_reported: Vec::new(),
            evidence: vec![MetadataRecordV0 {
                key: "rtt_p95_ms".to_owned(),
                value: "42".to_owned(),
                source: MetadataSourceV0::WitnessObserved,
            }],
        };

        assert_eq!(
            actual_validity(&metadata),
            ActualValidityV0::InconclusiveNoObservedMetadata
        );
    }
}
