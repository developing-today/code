//! Admission check for joined records (#12279, #12232 slice 3).
//!
//! A joined record is a part B version carrying a part A signed
//! record under the D14 join (recorded on #12232): the version's
//! body is the record's canonical envelope bytes, its event id is
//! the envelope's BLAKE3 digest (the same 32 bytes as in the
//! record's `id://` address), its object is the record's logical
//! graph, and the version's key is the record's key.
//!
//! The B32 ingest gate checks the version's own signature and never
//! opens the body, so before this module a broken join was admitted,
//! stored and synced, and refused only by the reader. [`check_join`]
//! is the one admission check for joined records: the store's ingest
//! calls it after the gate (whose refusals keep their precedence)
//! and before any store state changes. A refused version changes no
//! store state and, because a replica only offers what it kept, is
//! never offered to a peer.
//!
//! A version counts as a joined record when it claims the join
//! either way a reader would find it: its body decodes as a record
//! envelope, or its event id is its body's BLAKE3 digest (the
//! content-address claim an `id://` address makes). A version that
//! claims neither is not a record carrier and is left to the gate
//! alone, exactly as before.
//!
//! This module names no other federation module's types — it takes
//! the four joined fields as plain byte arrays — so the path-loaded
//! federation tests, which load the slice files as their own crate
//! roots, can load it unchanged beside them.

use id_core::graph::record::open;
use id_core::hash_profile::ProfileDispatch;
use id_core::signed_envelope::Envelope as RecordEnvelope;

/// Why a joined record was refused at admission. One class per way
/// the join can break; the gate stays unaware of bodies, so these
/// are deliberately not `RejectReason` variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinRefusal {
    /// The body is not a decodable signed-record envelope at all.
    BodyNotEnvelope,
    /// The body decodes as an envelope but does not open as a valid
    /// signed record (for example its signature was changed).
    BodyNotValidRecord,
    /// The body's BLAKE3 digest, or the opened record's own content
    /// id, is not the version's event id.
    EventIdMismatch,
    /// The header's object is not the record's logical graph.
    ObjectMismatch,
    /// The version's verifying key is not the record's key: the
    /// record was signed by someone else.
    SignerMismatch,
}

/// Whether a version with this event id and body claims the record
/// join: the body either decodes as a record envelope or is
/// addressed by its own BLAKE3 digest.
pub fn is_joined_record(event_id: &[u8; 32], body: &[u8]) -> bool {
    blake3::hash(body).as_bytes() == event_id || RecordEnvelope::decode(body).is_ok()
}

/// Check every part of the join for a version already known to
/// claim it. The oracle is the reader's own checks in the walking
/// skeleton's sync test, moved to admission: what a reader would
/// refuse on the way out is refused here on the way in.
pub fn check_join(
    event_id: &[u8; 32],
    verifying_key: &[u8; 32],
    object_id: &[u8; 32],
    body: &[u8],
) -> Result<(), JoinRefusal> {
    if blake3::hash(body).as_bytes() != event_id {
        return Err(JoinRefusal::EventIdMismatch);
    }
    let record_envelope = RecordEnvelope::decode(body).map_err(|_| JoinRefusal::BodyNotEnvelope)?;
    let dispatch = ProfileDispatch::default();
    let opened = open(record_envelope, &dispatch).map_err(|_| JoinRefusal::BodyNotValidRecord)?;
    if opened.id.content_id().digest() != event_id {
        return Err(JoinRefusal::EventIdMismatch);
    }
    if object_id != opened.record.graph.as_bytes() {
        return Err(JoinRefusal::ObjectMismatch);
    }
    if verifying_key != &opened.envelope.body.key {
        return Err(JoinRefusal::SignerMismatch);
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    const GRAPH: [u8; 32] = [0x11; 32];

    #[test]
    fn a_body_addressed_by_its_own_digest_claims_the_join() {
        let body = b"not an envelope".to_vec();
        let event = *blake3::hash(&body).as_bytes();
        assert!(is_joined_record(&event, &body));
        assert_eq!(
            check_join(&event, &[0; 32], &GRAPH, &body),
            Err(JoinRefusal::BodyNotEnvelope)
        );
    }

    #[test]
    fn a_body_with_an_unrelated_event_id_claims_nothing() {
        let body = b"not an envelope".to_vec();
        assert!(!is_joined_record(&[0x33; 32], &body));
    }

    #[test]
    fn a_record_shaped_body_under_the_wrong_event_id_is_an_event_id_mismatch() {
        use ed25519_dalek::SigningKey;
        use id_core::graph::record::GraphRecord;
        use id_core::graph::term::{Term, Triple};
        use id_core::signed_envelope::LogicalGraphId;

        let key = SigningKey::from_bytes(&[7; 32]);
        let record = GraphRecord::new(
            LogicalGraphId::new(GRAPH),
            vec![Triple::new(
                Term::typed(1, b"urn:note:1".to_vec()),
                Term::typed(2, b"body".to_vec()),
                Term::typed(3, b"hi".to_vec()),
            )],
        );
        let signed = record
            .sign("namespace/notes", "alice", &key)
            .expect("signing works");
        let bytes = signed
            .envelope
            .canonical_bytes()
            .expect("canonical bytes work");
        assert!(is_joined_record(&[0x33; 32], &bytes));
        assert_eq!(
            check_join(&[0x33; 32], &key.verifying_key().to_bytes(), &GRAPH, &bytes),
            Err(JoinRefusal::EventIdMismatch)
        );
    }
}
