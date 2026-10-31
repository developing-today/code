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
//!
//! [`search_terms`] is the multi-term form (design §9.4, #800). A subject matches a term set when
//! every term matches it (AND): a [`Term::Name`] or [`Term::Author`] term matches when one of the
//! subject's live claims matches it — different terms may be satisfied by different claims — and a
//! [`Term::SubjectKind`] term matches when the subject is an event of that kind. A hit carries the
//! claims that matched a name or author term; a term set with no name or author term (a kind-only
//! search) carries all of the subject's live claims. An empty term set matches every subject.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use p2panda_core::Hash;

use crate::event_store::{EventStore, StoreError};
use crate::tag_claims::Claim;
use crate::tag_index::TagIndex;
use crate::waiting::AuthorKey;

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

/// One term of a multi-term search (design §9.4). Terms combine with AND over subjects:
/// see the module docs for the exact semantics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Term<'a> {
    /// A live claim with this name and value match, on the subject.
    Name {
        /// The tag name.
        name: &'a str,
        /// Which values match.
        value: ValueMatch<'a>,
    },
    /// A live claim by this author, on the subject.
    Author(AuthorKey),
    /// The subject is an event of this kind: the `kind` of the subject event's header
    /// extension, such as `document.put`. A subject that is not an event in the store, or
    /// whose kind the caller's lookup does not know, never matches.
    SubjectKind(&'a str),
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
    search_terms(claims, &[Term::Name { name, value }], |_| None)
}

/// The subjects that match every term in `terms`, among `claims` (multi-term AND, §9.4).
///
/// Name and author terms are answered from the [`TagIndex`] over `claims` — the index carries
/// the author key author terms need. `subject_kind` resolves a subject to the kind of the
/// event it names, for [`Term::SubjectKind`]; it is called at most once per subject, and a
/// subject it cannot resolve (`None`) never matches a kind term. [`search_store_terms`]
/// wires it to the store; a caller with no kind terms may pass a lookup that always
/// returns `None`, as [`search`] does.
#[must_use]
pub fn search_terms(
    claims: &[Claim],
    terms: &[Term<'_>],
    subject_kind: impl Fn(&Hash) -> Option<String>,
) -> Vec<Hit> {
    let index = TagIndex::new(claims);
    let by_id: HashMap<Hash, &Claim> = claims.iter().map(|claim| (claim.id, claim)).collect();
    // Every live claim, grouped by subject, in subject order.
    let mut all_by_subject: BTreeMap<[u8; 32], Vec<Hash>> = BTreeMap::new();
    for claim in claims {
        all_by_subject
            .entry(*claim.subject.as_bytes())
            .or_default()
            .push(claim.id);
    }
    // For each term, the subjects it matches and, for claim terms, the claims that match it.
    let mut matched: Vec<(BTreeSet<[u8; 32]>, BTreeMap<[u8; 32], BTreeSet<[u8; 32]>>)> =
        Vec::with_capacity(terms.len());
    for term in terms {
        let mut claims_by_subject: BTreeMap<[u8; 32], BTreeSet<[u8; 32]>> = BTreeMap::new();
        match term {
            Term::Name { name, value } => {
                let entries = match value {
                    ValueMatch::Any => index.named(name),
                    ValueMatch::Bare => index.value_name(None, name),
                    ValueMatch::Exact(text) => index.value_name(Some(text), name),
                };
                for entry in entries {
                    claims_by_subject
                        .entry(*entry.subject.as_bytes())
                        .or_default()
                        .insert(*entry.claim.as_bytes());
                }
            }
            Term::Author(author) => {
                for entry in index.author(author) {
                    claims_by_subject
                        .entry(*entry.subject.as_bytes())
                        .or_default()
                        .insert(*entry.claim.as_bytes());
                }
            }
            Term::SubjectKind(_) => {}
        }
        let subjects: BTreeSet<[u8; 32]> = match term {
            Term::SubjectKind(kind) => all_by_subject
                .keys()
                .copied()
                .filter(|subject| {
                    subject_kind(&Hash::from_bytes(*subject)).as_deref() == Some(*kind)
                })
                .collect(),
            _ => claims_by_subject.keys().copied().collect(),
        };
        matched.push((subjects, claims_by_subject));
    }
    // AND: a subject must be matched by every term. With no terms, every subject matches.
    let mut subjects: BTreeSet<[u8; 32]> = all_by_subject.keys().copied().collect();
    for (term_subjects, _) in &matched {
        subjects = subjects.intersection(term_subjects).copied().collect();
    }
    let has_claim_terms = terms
        .iter()
        .any(|term| !matches!(term, Term::SubjectKind(_)));
    subjects
        .into_iter()
        .map(|subject| {
            let mut ids: BTreeSet<[u8; 32]> = BTreeSet::new();
            if has_claim_terms {
                for (_, claims_by_subject) in &matched {
                    if let Some(term_ids) = claims_by_subject.get(&subject) {
                        ids.extend(term_ids.iter().copied());
                    }
                }
            } else if let Some(all) = all_by_subject.get(&subject) {
                ids.extend(all.iter().map(|id| *id.as_bytes()));
            }
            Hit {
                subject: Hash::from_bytes(subject),
                claims: ids
                    .into_iter()
                    .filter_map(|id| by_id.get(&Hash::from_bytes(id)))
                    .map(|claim| (*claim).clone())
                    .collect(),
            }
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

/// The multi-term search over the store's live claims, read by [`EventStore::tag_claims`].
///
/// [`Term::SubjectKind`] resolves through the store: the subject's event, read by
/// [`EventStore::event_by_id`], and the kind in its header extension.
///
/// # Errors
///
/// Returns the errors of [`EventStore::tag_claims`] and [`EventStore::event_by_id`].
pub fn search_store_terms(store: &EventStore, terms: &[Term<'_>]) -> Result<Vec<Hit>, StoreError> {
    let claims = store.tag_claims()?;
    let mut kinds: HashMap<Hash, Option<String>> = HashMap::new();
    for subject in claims.iter().map(|claim| claim.subject) {
        if let std::collections::hash_map::Entry::Vacant(slot) = kinds.entry(subject) {
            let kind = store
                .event_by_id(&subject)?
                .map(|event| event.header.extensions.kind);
            slot.insert(kind);
        }
    }
    Ok(search_terms(&claims, terms, |subject| {
        kinds.get(subject).cloned().flatten()
    }))
}
