//! Player-signed move capture for world games (#2814).
//!
//! The attestation design (#2037, synthesis #2039) rests on move evidence
//! being **player-signed**: a witness replays both players' signed move
//! logs, and a rewritten move history collapses via the event store's fork
//! proofs. The world path used to journal only host-committed events —
//! `WorldEvent` carries a host sequence and a server-assigned participant
//! id, with no signature and no chain link — so the signed chains the
//! event store already verifies ([`crate::chain`]) were not what game
//! moves were.
//!
//! Design (recorded on issue #2814, 2026-11-01, lane F): the player signs
//! each move with [`chain::sign`] under their own key **before**
//! submitting it; the world verifies at the input seam and cross-references
//! the signed event in its accepted-move records. Actor-side signing was
//! rejected: the actor holds no player keys, and a host-held key would
//! make the signature prove the host, not the player — exactly the gap
//! Codex flagged on PR #2805.
//!
//! A signed move is a chain event of kind [`MOVE_KIND`] whose extension
//! group names the world ([`world_group`]) and whose body is exactly the
//! submitted input bytes, so the header's payload hash commits to the
//! move. Verification checks, in order: canonical decode and signature
//! ([`chain::decode`]), kind, world group, author equals the participant's
//! bound key, sequence equals the author's next expected sequence, and
//! backlink equals the author's current frontier. Two distinct signed
//! events at one author sequence are therefore both valid chain events
//! whose conflict the event store's existing ingest rules turn into a
//! fork proof; neither is ever accepted here as the author's next move
//! once the other has been.
//!
//! Scope of this first pass: plain [`WorldEventKind::Input`] moves only,
//! with the binding and the accepted-move records held by
//! [`WorldCore`](super::WorldCore) at runtime. Since #4346 the binding
//! and the accepted records are journaled (`JournalEntry::AuthorKeyBound`
//! / `JournalEntry::SignedMoveAccepted`) and restored by journal replay
//! with chain re-verification, so signed play survives a restart without
//! re-anchoring; the accepted bytes reach the event store through
//! `WorldCore::ingest_signed_moves`. Entity-addressed input
//! (`WorldEventKind::EntityInput`, #3870) is out of scope here.
//!
//! [`WorldEventKind::Input`]: super::WorldEventKind::Input

use anyhow::{Result, ensure};
use p2panda_core::{Hash, SigningKey};

use crate::chain::{self, ChainExt, Event};

/// The chain event kind every signed world move carries.
pub const MOVE_KIND: &str = "world.move";

/// The chain group naming the world a move belongs to: the BLAKE3 digest
/// of the world id, so a move event names its world inside its signed
/// extension and cannot be replayed as a move in another world.
#[must_use]
pub fn world_group(world_id: &str) -> Hash {
    Hash::digest(world_id.as_bytes())
}

/// Sign one move with the player's own key.
///
/// `seq_num` counts the author's signed moves from 0; sequence 0 takes no
/// backlink, and every later sequence names the author's previous signed
/// move's event id. The event body is exactly `input`, so the signed
/// header commits to the submitted bytes.
///
/// # Errors
///
/// Returns [`chain::ChainError::BlankWorldId`] if `world_id` is empty or
/// whitespace-only and [`chain::ChainError::EmptyInput`] if `input` is
/// empty, the same domain a world accepts (`WorldCore::new` and
/// `prepare_input_signed`), so no move is signed that no world can accept.
/// Returns the chain error if the sequence/backlink pair is inconsistent.
pub fn sign_move(
    key: &SigningKey,
    world_id: &str,
    seq_num: u32,
    backlink: Option<Hash>,
    input: &[u8],
) -> Result<Event, chain::ChainError> {
    if world_id.trim().is_empty() {
        return Err(chain::ChainError::BlankWorldId);
    }
    if input.is_empty() {
        return Err(chain::ChainError::EmptyInput);
    }
    chain::sign(
        key,
        seq_num,
        backlink,
        Some(input),
        ChainExt {
            kind: MOVE_KIND.to_owned(),
            group: Some(world_group(world_id)),
            after: Vec::new(),
            tag: None,
            document: None,
            removes: None,
            note: None,
            link: None,
            checkpoint: None,
            rotation: None,

            chat: None,
        },
    )
}

/// One accepted signed move, cross-referenced both ways: the world
/// sequence it committed as, and the chain position (author, author
/// sequence, event id) it occupies in the author's signed chain. The
/// signed bytes are kept with the record so an attestation witness can
/// ingest them into the event store without further capture work.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedMoveRecord {
    /// Host world sequence of the committed input event.
    pub world_sequence: u64,
    /// Server-assigned participant that submitted the move.
    pub participant_id: u64,
    /// The author's verifying key bytes, as bound to the participant.
    pub author: [u8; 32],
    /// The move's sequence in the author's own chain.
    pub author_seq: u32,
    /// The signed event's id (BLAKE3 over its encoded header).
    pub event_id: Hash,
    /// The signed event's encoded header bytes.
    pub bytes: Vec<u8>,
}

/// One author's expected chain position: the next sequence a signed move
/// must carry, and the event id it must name as its backlink.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MoveChain {
    next_seq: u32,
    frontier: Option<Hash>,
}

impl MoveChain {
    /// A fresh chain position: sequence 0, no frontier.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The sequence the author's next signed move must carry.
    #[must_use]
    pub fn next_seq(&self) -> u32 {
        self.next_seq
    }

    /// The event id the author's next signed move must name as backlink.
    #[must_use]
    pub fn frontier(&self) -> Option<Hash> {
        self.frontier
    }

    /// Verify `bytes` as `author`'s next signed move in `world_id`,
    /// committing to exactly `input`. Does not advance the chain: the
    /// move is only the author's next move once the world commits it
    /// (see [`MoveChain::accept`]).
    ///
    /// # Errors
    ///
    /// Refuses bytes that do not decode as a signed chain event, a kind
    /// or world group other than a move in this world, an author other
    /// than the bound key, a sequence or backlink other than the expected
    /// next position, or a payload hash that does not commit to `input`
    /// (a tampered or substituted move).
    pub fn verify(
        &self,
        bytes: &[u8],
        world_id: &str,
        input: &[u8],
        author: &[u8; 32],
    ) -> Result<Event> {
        // #6447, defense in depth for the witness-replay path (#4346):
        // the same acceptance domain sign_move refuses to sign. A move
        // for a blank world verifies against that same blank ID, yet no
        // WorldCore can ever bear it; an empty input can never commit.
        // Neither arises from a constructed world, whose constructor
        // and input seam refuse both before verify is reached.
        ensure!(!world_id.trim().is_empty(), "world ID must not be empty");
        ensure!(!input.is_empty(), "world input must not be empty");
        let event = chain::decode(bytes)
            .map_err(|err| anyhow::anyhow!("signed move does not decode: {err}"))?;
        ensure!(
            event.header.extensions.kind == MOVE_KIND,
            "signed move has kind {:?}, not {MOVE_KIND:?}",
            event.header.extensions.kind
        );
        ensure!(
            event.header.extensions.group == Some(world_group(world_id)),
            "signed move names another world"
        );
        ensure!(
            event.header.verifying_key.as_bytes() == author,
            "signed move author is not the participant's bound key"
        );
        ensure!(
            event.header.seq_num == self.next_seq,
            "signed move sequence {} is not the expected {}",
            event.header.seq_num,
            self.next_seq
        );
        ensure!(
            event.header.backlink == self.frontier,
            "signed move backlink does not name the author's current frontier"
        );
        ensure!(
            event.header.payload_hash == Some(Hash::digest(input)),
            "signed move does not commit to the submitted input bytes"
        );
        Ok(event)
    }

    /// Verify `bytes` as `author`'s next signed move during journal
    /// replay (#4346): every [`MoveChain::verify`] check except the
    /// payload commitment, which replay cannot recompute when the
    /// committed input event has aged out of the journal's retained
    /// window under compaction. The signature, kind, world group,
    /// author, sequence, and backlink are all still verified, and the
    /// caller cross-references the committed input event whenever it
    /// is retained. The payload commitment itself was verified at
    /// acceptance time, before the move was journaled.
    ///
    /// # Errors
    ///
    /// As [`MoveChain::verify`], minus the payload-commitment refusal.
    pub fn verify_journaled(
        &self,
        bytes: &[u8],
        world_id: &str,
        author: &[u8; 32],
    ) -> Result<Event> {
        ensure!(!world_id.trim().is_empty(), "world ID must not be empty");
        let event = chain::decode(bytes)
            .map_err(|err| anyhow::anyhow!("signed move does not decode: {err}"))?;
        ensure!(
            event.header.extensions.kind == MOVE_KIND,
            "signed move has kind {:?}, not {MOVE_KIND:?}",
            event.header.extensions.kind
        );
        ensure!(
            event.header.extensions.group == Some(world_group(world_id)),
            "signed move names another world"
        );
        ensure!(
            event.header.verifying_key.as_bytes() == author,
            "signed move author is not the participant's bound key"
        );
        ensure!(
            event.header.seq_num == self.next_seq,
            "signed move sequence {} is not the expected {}",
            event.header.seq_num,
            self.next_seq
        );
        ensure!(
            event.header.backlink == self.frontier,
            "signed move backlink does not name the author's current frontier"
        );
        Ok(event)
    }

    /// Advance the chain past an accepted move.
    pub fn accept(&mut self, event: &Event) {
        self.next_seq = event.header.seq_num + 1;
        self.frontier = Some(event.id);
    }

    /// Verify a move and advance the chain past it in one step.
    ///
    /// # Errors
    ///
    /// As [`MoveChain::verify`]; the chain does not advance on failure.
    pub fn verify_and_accept(
        &mut self,
        bytes: &[u8],
        world_id: &str,
        input: &[u8],
        author: &[u8; 32],
    ) -> Result<Event> {
        let event = self.verify(bytes, world_id, input, author)?;
        self.accept(&event);
        Ok(event)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn key() -> SigningKey {
        SigningKey::from_bytes(&[7; 32])
    }

    #[test]
    fn sign_move_refuses_a_blank_world_id() {
        for world_id in ["", " ", "   ", "\t", "\n "] {
            let err = sign_move(&key(), world_id, 0, None, b"cell-0").unwrap_err();
            assert!(
                matches!(err, chain::ChainError::BlankWorldId),
                "{world_id:?}"
            );
        }
    }

    #[test]
    fn sign_move_refuses_an_empty_input() {
        let err = sign_move(&key(), "ttt", 0, None, b"").unwrap_err();
        assert!(matches!(err, chain::ChainError::EmptyInput));
    }

    #[test]
    fn sign_move_still_signs_and_verifies_a_control_move() {
        let key = key();
        let author = *key.verifying_key().as_bytes();
        let event = sign_move(&key, "ttt", 0, None, b"cell-0").unwrap();
        MoveChain::new()
            .verify(&event.bytes, "ttt", b"cell-0", &author)
            .unwrap();
    }

    #[test]
    fn verify_refuses_a_blank_world_id_and_an_empty_input() {
        let key = key();
        let author = *key.verifying_key().as_bytes();
        let event = sign_move(&key, "ttt", 0, None, b"cell-0").unwrap();
        let chain = MoveChain::new();
        // The control verifies; the same bytes against a blank world
        // or with an empty input do not (#6447 triage decision).
        assert!(
            chain
                .verify(&event.bytes, "ttt", b"cell-0", &author)
                .is_ok()
        );
        for world_id in ["", " ", "\t"] {
            assert!(
                chain
                    .verify(&event.bytes, world_id, b"cell-0", &author)
                    .is_err(),
                "{world_id:?}"
            );
        }
        assert!(chain.verify(&event.bytes, "ttt", b"", &author).is_err());
    }

    #[test]
    fn sign_move_keeps_the_sequence_and_backlink_refusals() {
        assert!(sign_move(&key(), "ttt", 1, None, b"cell-0").is_err());
        let first = sign_move(&key(), "ttt", 0, None, b"cell-0").unwrap();
        assert!(sign_move(&key(), "ttt", 0, Some(first.id), b"cell-1").is_err());
    }
}
