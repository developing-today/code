//! Search by tag name and value over the live claims (Phase 5, P5.5; design §9.4).
//!
//! A search takes a name and a [`ValueMatch`], and returns each subject that has a live claim
//! matching them, as [`Hit`]s. It is a thin query: the live set comes from
//! [`EventStore::tag_claims`], and the answer from the [`TagIndex`] built over it. The index is
//! rebuilt on each call, as [`EventStore::tag_index`] rebuilds it (#183). Nothing is persisted here.
//!
//! A value matches as a whole field. The tuple encoding ends every field with `00 00`, so the value
//! `a` does not match `a\0b`, and the name `a` does not match `a\0b` either. The design's "prefix"
//! is a scan over the leading fields of a key, so a search has no string prefix such as `ci` for
//! `city`. Full-text search is out of scope for Phase 5 (task 06).
//!
//! Hits are ordered by subject ID bytes, and the claims of a hit by claim ID bytes. The answer does
//! not depend on the order the events arrived in.

use std::collections::{BTreeMap, HashMap};

use p2panda_core::Hash;

use crate::event_store::{EventStore, StoreError};
use crate::tag_claims::Claim;
use crate::tag_index::TagIndex;

/// Which values a search matches, for one tag name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueMatch<'a> {
    /// Any value, and the bare tag.
    Any,
    /// Only the bare tag, which has no value.
    Bare,
    /// Only this value, matched as a whole field.
    Exact(&'a str),
}

/// A subject that a search matched, with the live claims that matched on it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    /// The subject: an event ID, such as an identity's or a document's.
    pub subject: Hash,
    /// The matching live claims, sorted by claim ID bytes. A contested tag gives more than one.
    pub claims: Vec<Claim>,
}

/// The subjects with a live claim that matches `name` and `value`, among `claims`.
///
/// `claims` is the live set, as [`EventStore::tag_claims`] returns it. The index over it answers the
/// query: an Ω scan on the value and the name for [`ValueMatch::Exact`] and [`ValueMatch::Bare`],
/// and a scan of the name for [`ValueMatch::Any`].
#[must_use]
pub fn search(claims: &[Claim], name: &str, value: ValueMatch<'_>) -> Vec<Hit> {
    let index = TagIndex::new(claims);
    let entries = match value {
        ValueMatch::Any => index.named(name),
        ValueMatch::Bare => index.value_name(None, name),
        ValueMatch::Exact(text) => index.value_name(Some(text), name),
    };
    let by_id: HashMap<Hash, &Claim> = claims.iter().map(|claim| (claim.id, claim)).collect();
    let mut hits: BTreeMap<[u8; 32], Hit> = BTreeMap::new();
    for entry in entries {
        let Some(claim) = by_id.get(&entry.claim) else {
            continue;
        };
        hits.entry(*entry.subject.as_bytes())
            .or_insert_with(|| Hit {
                subject: entry.subject,
                claims: Vec::new(),
            })
            .claims
            .push((*claim).clone());
    }
    hits.into_values()
        .map(|mut hit| {
            hit.claims.sort_by_key(|claim| *claim.id.as_bytes());
            hit
        })
        .collect()
}

/// The search over the store's live claims, read by [`EventStore::tag_claims`].
///
/// # Errors
///
/// Returns the errors of [`EventStore::tag_claims`].
pub fn search_store(
    store: &EventStore,
    name: &str,
    value: ValueMatch<'_>,
) -> Result<Vec<Hit>, StoreError> {
    Ok(search(&store.tag_claims()?, name, value))
}
