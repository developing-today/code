//! Property tests for the event header in `src/chain.rs`.
//!
//! The properties use the default case count, which `PROPTEST_CASES` raises.
//! Failures are not written to `proptest-regressions/`: the minimal input goes
//! into `tests/regressions/chainext.txt` instead (see
//! `tests/regressions/README.md`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::chain::check_event_bytes;
use id::chain::{ChainExt, DOMAIN_TAG, MAX_AFTER, decode, sign};
use p2panda_core::{Extensions, Hash, Header, SigningKey};
use proptest::prelude::*;
use proptest::sample::Index;
use proptest::test_runner::TestCaseError;
use serde::{Deserialize, Serialize};

/// The test key: the 32 bytes 00 through 1f. It is not a real key.
fn key() -> SigningKey {
    SigningKey::from_bytes(&std::array::from_fn(|i| u8::try_from(i).unwrap()))
}

fn config() -> ProptestConfig {
    ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

/// Text of 1 to 20 characters from ASCII and from characters that are already
/// NFC. At most 60 bytes, so it is always inside `MAX_KIND_BYTES`.
fn kind() -> impl Strategy<Value = String> {
    "[a-z.é日-]{1,20}"
}

fn hash() -> impl Strategy<Value = Hash> {
    any::<[u8; 32]>().prop_map(Hash::from)
}

/// Up to `max` distinct hashes, in no particular order.
fn frontier(max: usize) -> impl Strategy<Value = Vec<Hash>> {
    prop::collection::hash_set(any::<[u8; 32]>(), 0..=max)
        .prop_map(|set| set.into_iter().map(Hash::from).collect())
}

/// More than `MAX_AFTER` distinct hashes.
fn oversized_frontier() -> impl Strategy<Value = Vec<Hash>> {
    prop::collection::hash_set(any::<[u8; 32]>(), MAX_AFTER + 1..=MAX_AFTER + 40)
        .prop_map(|set| set.into_iter().map(Hash::from).collect())
}

/// Any extension the builder accepts.
fn chain_ext() -> impl Strategy<Value = ChainExt> {
    (kind(), prop::option::of(hash()), frontier(MAX_AFTER)).prop_map(|(kind, group, after)| {
        ChainExt {
            kind,
            group,
            after,
            tag: None,
            document: None,
            removes: None,
            note: None,
        }
    })
}

/// An extension the builder would never sign. `after` is not limited here, so
/// the decoder is the only check that can refuse it.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Forged {
    domain: String,
    kind: String,
    group: Option<Hash>,
    after: Vec<Hash>,
}

/// Signs any extension as sequence 0 with no body, the way `sign` does.
fn sign_raw<E: Extensions>(ext: E) -> Vec<u8> {
    Header::<E>::builder().build(&key(), ext).encode()
}

proptest! {
    #![proptest_config(config())]

    /// A signed event decodes to the same event, with the same extension.
    #[test]
    fn test_signed_chain_ext_round_trips(
        ext in chain_ext(),
        body in prop::option::of(prop::collection::vec(any::<u8>(), 0..128)),
        seq in 0u32..1_000,
        backlink in hash(),
    ) {
        let backlink = (seq > 0).then_some(backlink);
        let event = sign(&key(), seq, backlink, body.as_deref(), ext.clone())
            .map_err(|err| TestCaseError::fail(err.to_string()))?;
        let decoded = decode(&event.bytes).map_err(|err| TestCaseError::fail(err.to_string()))?;
        prop_assert_eq!(&decoded, &event);
        prop_assert_eq!(&decoded.header.extensions, &ext);
        prop_assert_eq!(decoded.id, Hash::digest(&event.bytes));
    }

    /// Re-encoding a decoded event gives the same bytes, and signing its
    /// extension again gives the same bytes too.
    #[test]
    fn test_canonical_encoding_is_stable_on_reencode(ext in chain_ext()) {
        let event = sign(&key(), 0, None, None, ext)
            .map_err(|err| TestCaseError::fail(err.to_string()))?;
        let decoded = decode(&event.bytes).map_err(|err| TestCaseError::fail(err.to_string()))?;
        prop_assert_eq!(&decoded.header.encode(), &event.bytes);
        let again = sign(&key(), 0, None, None, decoded.header.extensions)
            .map_err(|err| TestCaseError::fail(err.to_string()))?;
        prop_assert_eq!(&again.bytes, &event.bytes);
    }

    /// More than `MAX_AFTER` hashes are refused by the builder, by validation,
    /// and by the decoder, whatever the count and the hashes.
    #[test]
    fn test_after_above_the_limit_is_always_refused(after in oversized_frontier()) {
        let ext = ChainExt {
            kind: "m".to_owned(),
            group: None,
            after: after.clone(), tag: None, document: None, removes: None, note: None,
        };
        prop_assert!(sign(&key(), 0, None, None, ext.clone()).is_err());
        prop_assert!(ext.validate().is_err());
        let forged = sign_raw(Forged {
            domain: DOMAIN_TAG.to_owned(),
            kind: "m".to_owned(),
            group: None,
            after,
        });
        prop_assert!(decode(&forged).is_err());
    }

    /// The decoder never panics on random bytes, and whatever it accepts holds
    /// up under the replay's checks.
    #[test]
    fn test_decoder_never_panics_on_random_bytes(
        bytes in prop::collection::vec(any::<u8>(), 0..1024),
    ) {
        check_event_bytes(&bytes).map_err(TestCaseError::fail)?;
    }

    /// Changing any one byte of a signed event makes it invalid. The signature
    /// covers every field, and a changed encoding is not canonical, so each
    /// such change is refused.
    #[test]
    fn test_a_single_changed_byte_is_always_refused(
        ext in chain_ext(),
        body in prop::option::of(prop::collection::vec(any::<u8>(), 1..64)),
        index in any::<Index>(),
        flip in 1u8..=255,
    ) {
        let event = sign(&key(), 0, None, body.as_deref(), ext)
            .map_err(|err| TestCaseError::fail(err.to_string()))?;
        let mut bytes = event.bytes;
        let at = index.index(bytes.len());
        bytes[at] ^= flip;
        prop_assert!(decode(&bytes).is_err(), "byte {at} changed by {flip:#x} was accepted");
        check_event_bytes(&bytes).map_err(TestCaseError::fail)?;
    }

    /// A proper prefix of a signed event is never a valid event.
    #[test]
    fn test_a_truncated_event_is_always_refused(ext in chain_ext(), cut in any::<Index>()) {
        let event = sign(&key(), 0, None, None, ext)
            .map_err(|err| TestCaseError::fail(err.to_string()))?;
        let at = cut.index(event.bytes.len());
        prop_assert!(decode(&event.bytes[..at]).is_err(), "a prefix of {at} bytes was accepted");
    }

    /// The sequence and the backlink must agree, and the decoder applies the builder's rule:
    /// sequence 0 takes no backlink, and every later sequence names one. A signed header that
    /// breaks the rule is refused, whatever the sequence number, and one that keeps it is
    /// accepted. p2panda's own decoder accepts both kinds, so this is the only check.
    #[test]
    fn test_decoder_applies_the_sequence_and_backlink_rule(
        seq in 0u32..300,
        backlink in prop::option::of(hash()),
    ) {
        let raw = Header::<ChainExt>::builder()
            .seq_num(seq)
            .backlink(backlink)
            .build(
                &key(),
                ChainExt {
                    kind: "m".to_owned(),
                    group: None,
                    after: Vec::new(), tag: None, document: None, removes: None, note: None,
                },
            )
            .encode();
        let agrees = (seq == 0) == backlink.is_none();
        prop_assert_eq!(
            decode(&raw).is_ok(),
            agrees,
            "sequence {}, backlink {:?}",
            seq,
            backlink
        );
    }
}
