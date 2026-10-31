//! Friendly-name reference resolution over the local store — proof of concept (#2259).
//!
//! This module is the PoC specified by `doc/2026-10-30T13-25-00Z_design_friendly_name_references.md`
//! (from the #2047 investigation). It is **not** a production resolution feature: it resolves
//! `@`/`#` phrases against exactly one in-memory local index (the notes/documents surface),
//! demonstrates the spec's ranking, and renders stored references in all four states. There is
//! no cross-store fan-out, no locator fetch, and no hosted search here.
//!
//! The rules that matter, from the spec:
//!
//! - A stored [`Reference`] is the target's stable ID plus the surface the author chose. The
//!   typed phrase is a query, never the reference; once stored, ambiguity ends.
//! - Ranking is petname-first, then exact alias claim, exact title, prefix, token, full-text.
//!   Boosts never cross tiers (the PoC has no boosts beyond this order).
//! - A surface claimed by two or more targets is **contested**: every candidate is shown with
//!   its provenance and nothing is picked silently. There is no commit-on-Enter for an
//!   ambiguous list — [`LocalIndex::insert_unambiguous`] refuses, the author picks a candidate
//!   explicitly with [`LocalIndex::insert`].
//! - Resolution is viewer-relative: the viewer's own petnames live only in their local index,
//!   so the same phrase may resolve differently for different viewers, and a reference whose
//!   target a viewer cannot see renders as inert text that reveals only the stored surface.

use std::collections::BTreeMap;

use p2panda_core::Hash;

use crate::waiting::AuthorKey;

/// The sigil that introduced a reference: `@` for an actor, `#` for a topic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Sigil {
    /// `@` — an actor (a person, account, or agent).
    Actor,
    /// `#` — a topic (a document, note, or tag subject).
    Topic,
}

impl Sigil {
    /// The character this sigil is typed with.
    #[must_use]
    pub const fn chr(self) -> char {
        match self {
            Self::Actor => '@',
            Self::Topic => '#',
        }
    }

    /// The sigil for a character, if it is one.
    #[must_use]
    pub const fn from_chr(chr: char) -> Option<Self> {
        match chr {
            '@' => Some(Self::Actor),
            '#' => Some(Self::Topic),
            _ => None,
        }
    }
}

/// A store's identity, as a locator hint and as candidate provenance. The PoC has exactly one
/// local store; the type is the spec's, so a v2 fan-out can name more.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct StoreId(pub String);

impl StoreId {
    /// The local store of the PoC.
    #[must_use]
    pub fn local() -> Self {
        Self("local".to_owned())
    }
}

/// A stored reference: the meaning is the target ID, the surface is only a label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    /// The target object's stable ID.
    pub target_id: Hash,
    /// Which sigil the author typed.
    pub sigil: Sigil,
    /// The phrase as the author typed/chose it. Kept verbatim: a renamed target still renders
    /// with this surface (historical fidelity); it is never silently rewritten.
    pub surface: String,
    /// Stores where the target was found, as hints for a later locator fetch (v2, not here).
    pub locator_hints: Vec<StoreId>,
    /// Who authored the reference.
    pub author_context: AuthorKey,
}

impl Reference {
    /// The plain-text/markdown degradation: surface as the text, the ID as the link target.
    /// The surface's `\`, `[` and `]` are backslash-escaped, so a surface containing markdown
    /// syntax keeps its text and cannot break out of the label.
    #[must_use]
    pub fn to_markdown(&self) -> String {
        let mut escaped = String::with_capacity(self.surface.len());
        for chr in self.surface.chars() {
            if matches!(chr, '\\' | '[' | ']') {
                escaped.push('\\');
            }
            escaped.push(chr);
        }
        format!("[{escaped}](id://{})", self.target_id.to_hex())
    }

    /// Parses the degradation written by [`Reference::to_markdown`], undoing its escapes. The
    /// sigil and author are not carried by the degradation, so the caller names the sigil and
    /// the author defaults to zeroes; the ID and surface — the meaning and the label —
    /// round-trip exactly, whatever the surface contains.
    #[must_use]
    pub fn from_markdown(text: &str, sigil: Sigil) -> Option<Self> {
        let mut surface = String::new();
        let mut chars = text.strip_prefix('[')?.chars().peekable();
        // Consume the label up to the first unescaped "](id://" delimiter.
        let hex = loop {
            match chars.next()? {
                '\\' => {
                    let next = chars.next()?;
                    if !matches!(next, '\\' | '[' | ']') {
                        return None;
                    }
                    surface.push(next);
                }
                ']' if chars.next() == Some('(') => {
                    let rest: String = chars.collect();
                    break rest.strip_prefix("id://")?.strip_suffix(')')?;
                }
                chr => surface.push(chr),
            }
        };
        let target_id = hex.parse::<Hash>().ok()?;
        Some(Self {
            target_id,
            sigil,
            surface,
            locator_hints: Vec::new(),
            author_context: [0; 32],
        })
    }
}

/// Whether the surface a candidate matched on is claimed by more than one target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClaimState {
    /// Only this target claims the matched surface.
    Uncontested,
    /// Two or more targets claim the matched surface. Contested candidates are shown, never
    /// picked silently.
    Contested,
}

/// Where a candidate came from: enough for the author to tell duplicates apart and to judge a
/// claim, without leaking anything about objects outside this store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Provenance {
    /// The store that answered (the local store, in the PoC).
    pub store: StoreId,
    /// The key behind the matched claim or object.
    pub author: AuthorKey,
    /// Whether the matched surface is contested.
    pub claim_state: ClaimState,
}

/// How a candidate matched the typed phrase. Declaration order is ranking order: a lower
/// variant outranks every higher one, and nothing boosts a candidate across tiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchKind {
    /// The viewer's own petname for the target (`@` only). Tier 1.
    Petname,
    /// An exact alias claim. Tier 2.
    ExactClaim,
    /// An exact title match. Tier 2, after claims.
    ExactTitle,
    /// A title or alias the phrase is a prefix of. Tier 3.
    Prefix,
    /// A whole word of the title equals the phrase. Tier 3, after prefixes.
    Token,
    /// The phrase occurs inside the title. Tier 3, last.
    FullText,
}

/// One ranked candidate for a typed phrase.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    /// The candidate target's stable ID.
    pub target_id: Hash,
    /// The sigil the candidate answers.
    pub kind: Sigil,
    /// How this candidate matched.
    pub match_kind: MatchKind,
    /// Where the match came from.
    pub provenance: Provenance,
    /// The display label for the target, per `aliases.rs` rules (the current alias, else the
    /// title; the petname when the match is the viewer's petname).
    pub surface: String,
}

impl Candidate {
    /// Whether this candidate's matched surface is contested.
    #[must_use]
    pub const fn contested(&self) -> bool {
        matches!(self.provenance.claim_state, ClaimState::Contested)
    }
}

/// The outcome of resolving one phrase in one viewer's local index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolution {
    /// The phrase as typed, without the sigil.
    pub phrase: String,
    /// The sigil typed.
    pub sigil: Sigil,
    /// Every candidate, best first. Duplicates and contested candidates are all present:
    /// showing them is the point.
    pub candidates: Vec<Candidate>,
}

impl Resolution {
    /// Whether any candidate's matched surface is contested.
    #[must_use]
    pub fn is_contested(&self) -> bool {
        self.candidates.iter().any(Candidate::contested)
    }

    /// The candidates tied at the best match tier.
    #[must_use]
    pub fn top_tier(&self) -> &[Candidate] {
        let Some(best) = self.candidates.first() else {
            return &[];
        };
        let end = self
            .candidates
            .iter()
            .position(|candidate| candidate.match_kind != best.match_kind)
            .unwrap_or(self.candidates.len());
        &self.candidates[..end]
    }

    /// The one candidate an author may commit without choosing, if there is one: exactly one
    /// candidate at the top tier, and it is not contested. Otherwise `None` — the author must
    /// pick from the list, and the stored ID then makes the choice permanent.
    #[must_use]
    pub fn single(&self) -> Option<&Candidate> {
        let top = self.top_tier();
        match top {
            [only] if !only.contested() => Some(only),
            _ => None,
        }
    }
}

/// One alias claim on an indexed object: the text, and the key that claimed it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AliasClaim {
    /// The alias text.
    pub text: String,
    /// The key that made the claim.
    pub author: AuthorKey,
}

/// One object in the local index: a document/note (topic) or an account identity (actor),
/// with the names the local store knows it by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexedObject {
    /// The object's stable ID.
    pub id: Hash,
    /// Whether it answers `@` or `#`.
    pub kind: Sigil,
    /// The object's title/name in the notes/documents surface.
    pub title: String,
    /// Its alias claims, in claim order.
    pub alias_claims: Vec<AliasClaim>,
    /// The key that authored the object.
    pub author: AuthorKey,
    /// Whether the target has been cleared (tombstoned) in this store.
    pub cleared: bool,
}

impl IndexedObject {
    /// The label a view shows now: the first alias claim, else the title. (The full
    /// `aliases.rs` display rules stay in `aliases.rs`; this is the PoC's one-label view.)
    #[must_use]
    pub fn current_label(&self) -> &str {
        self.alias_claims
            .first()
            .map_or(self.title.as_str(), |claim| claim.text.as_str())
    }
}

/// Why an unambiguous insert refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum InsertError {
    /// The phrase matched nothing in the local index.
    #[error("the phrase matched nothing in the local store")]
    NoMatch,
    /// The phrase is ambiguous or contested; the author must pick a candidate explicitly.
    #[error("the phrase has {0} top-tier candidates; the author must pick one")]
    Ambiguous(usize),
}

/// The PoC's per-store index: one local store's objects, plus the viewing client's petnames
/// (indexed locally only, never replicated — `petname.rs` keeps them in the owner's journal).
#[derive(Clone, Debug, Default)]
pub struct LocalIndex {
    store: StoreId,
    objects: Vec<IndexedObject>,
    petnames: BTreeMap<Hash, String>,
}

impl LocalIndex {
    /// An empty index for `store`.
    #[must_use]
    pub fn new(store: StoreId) -> Self {
        Self {
            store,
            objects: Vec::new(),
            petnames: BTreeMap::new(),
        }
    }

    /// The store this index answers for.
    #[must_use]
    pub const fn store(&self) -> &StoreId {
        &self.store
    }

    /// Adds an object to the index.
    pub fn add(&mut self, object: IndexedObject) {
        self.objects.push(object);
    }

    /// Records the viewer's petname for a target. Local to this index only.
    pub fn set_petname(&mut self, target: Hash, name: &str) {
        self.petnames.insert(target, name.to_owned());
    }

    /// The indexed object with `id`, if this store holds it.
    #[must_use]
    pub fn object(&self, id: &Hash) -> Option<&IndexedObject> {
        self.objects.iter().find(|object| object.id == *id)
    }

    /// Renames a target in this store (its title and shown label move; stored references to
    /// its ID do not). Returns whether the target was present.
    pub fn rename(&mut self, id: &Hash, new_title: &str) -> bool {
        let Some(object) = self.objects.iter_mut().find(|object| object.id == *id) else {
            return false;
        };
        object.title = new_title.to_owned();
        object.alias_claims.clear();
        true
    }

    /// Clears (tombstones) a target in this store. Returns whether it was present.
    pub fn clear(&mut self, id: &Hash) -> bool {
        let Some(object) = self.objects.iter_mut().find(|object| object.id == *id) else {
            return false;
        };
        object.cleared = true;
        true
    }

    /// How many distinct targets **of the requested sigil** claim `surface` as a title or
    /// alias (case-folded). Two or more means the surface is contested for that sigil. An
    /// actor and a topic sharing a title never contest each other: they answer different
    /// sigils, so neither is a competing target for the other's resolution.
    fn claimants(&self, sigil: Sigil, surface: &str) -> usize {
        let folded = fold(surface);
        self.objects
            .iter()
            .filter(|object| object.kind == sigil)
            .filter(|object| {
                fold(&object.title) == folded
                    || object
                        .alias_claims
                        .iter()
                        .any(|claim| fold(&claim.text) == folded)
            })
            .count()
    }

    /// Resolves `phrase` for `sigil` in this viewer's index: ranked candidates with
    /// provenance, duplicates and contested candidates included, nothing picked.
    #[must_use]
    pub fn resolve(&self, sigil: Sigil, phrase: &str) -> Resolution {
        let phrase = phrase.trim().trim_start_matches(['@', '#']);
        let folded = fold(phrase);
        let mut candidates: Vec<Candidate> = Vec::new();
        if !folded.is_empty() {
            for object in self.objects.iter().filter(|object| object.kind == sigil) {
                if let Some(candidate) = self.best_match(object, sigil, &folded) {
                    candidates.push(candidate);
                }
            }
        }
        candidates.sort_by(|a, b| {
            a.match_kind
                .cmp(&b.match_kind)
                .then_with(|| fold(&a.surface).cmp(&fold(&b.surface)))
                .then_with(|| a.target_id.to_hex().cmp(&b.target_id.to_hex()))
        });
        Resolution {
            phrase: phrase.to_owned(),
            sigil,
            candidates,
        }
    }

    /// The best (lowest-tier) match of one object against the folded phrase, if it matches.
    fn best_match(&self, object: &IndexedObject, sigil: Sigil, folded: &str) -> Option<Candidate> {
        let claim_state = |surface: &str| {
            if self.claimants(sigil, surface) > 1 {
                ClaimState::Contested
            } else {
                ClaimState::Uncontested
            }
        };
        let candidate = |match_kind: MatchKind, surface: String, author: AuthorKey, state: ClaimState| {
            Candidate {
                target_id: object.id,
                kind: sigil,
                match_kind,
                provenance: Provenance {
                    store: self.store.clone(),
                    author,
                    claim_state: state,
                },
                surface,
            }
        };
        // Tier 1: the viewer's petname, `@` only.
        if sigil == Sigil::Actor {
            if let Some(petname) = self.petnames.get(&object.id) {
                if fold(petname) == folded {
                    return Some(candidate(
                        MatchKind::Petname,
                        petname.clone(),
                        object.author,
                        ClaimState::Uncontested,
                    ));
                }
            }
        }
        // Tier 2: exact alias claim, then exact title.
        if let Some(claim) = object
            .alias_claims
            .iter()
            .find(|claim| fold(&claim.text) == folded)
        {
            let state = claim_state(&claim.text);
            return Some(candidate(MatchKind::ExactClaim, object.current_label().to_owned(), claim.author, state));
        }
        if fold(&object.title) == folded {
            let state = claim_state(&object.title);
            return Some(candidate(MatchKind::ExactTitle, object.current_label().to_owned(), object.author, state));
        }
        // Tier 3: prefix, then token, then full-text. A prefix that matches an alias claim
        // keeps that claim's author as provenance — the claim, not the object, is what the
        // author is evaluating.
        if let Some(claim) = object
            .alias_claims
            .iter()
            .find(|claim| fold(&claim.text).starts_with(folded))
        {
            return Some(candidate(MatchKind::Prefix, object.current_label().to_owned(), claim.author, ClaimState::Uncontested));
        }
        if fold(&object.title).starts_with(folded) {
            return Some(candidate(MatchKind::Prefix, object.current_label().to_owned(), object.author, ClaimState::Uncontested));
        }
        if object.title.split_whitespace().any(|word| fold(word.trim_matches(|chr: char| !chr.is_alphanumeric())) == folded) {
            return Some(candidate(MatchKind::Token, object.current_label().to_owned(), object.author, ClaimState::Uncontested));
        }
        if fold(&object.title).contains(folded) {
            return Some(candidate(MatchKind::FullText, object.current_label().to_owned(), object.author, ClaimState::Uncontested));
        }
        None
    }

    /// Stores a reference to a candidate the author picked, with the surface as typed. This
    /// explicit pick is the only way an ambiguous phrase becomes a reference.
    #[must_use]
    pub fn insert(&self, candidate: &Candidate, surface: &str, author: AuthorKey) -> Reference {
        Reference {
            target_id: candidate.target_id,
            sigil: candidate.kind,
            surface: surface.to_owned(),
            locator_hints: vec![self.store.clone()],
            author_context: author,
        }
    }

    /// Stores a reference for a phrase only when it resolves unambiguously.
    ///
    /// # Errors
    ///
    /// Returns [`InsertError::NoMatch`] when nothing matched, and
    /// [`InsertError::Ambiguous`] when the top tier is tied or contested — the author must
    /// pick from the [`Resolution`] instead of the phrase being committed silently.
    pub fn insert_unambiguous(
        &self,
        sigil: Sigil,
        phrase: &str,
        author: AuthorKey,
    ) -> Result<Reference, InsertError> {
        let resolution = self.resolve(sigil, phrase);
        match resolution.single() {
            Some(candidate) => Ok(self.insert(candidate, &resolution.phrase, author)),
            None if resolution.candidates.is_empty() => Err(InsertError::NoMatch),
            None => Err(InsertError::Ambiguous(resolution.top_tier().len())),
        }
    }
}

/// The rendering state of a stored reference, for one viewer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderState {
    /// The target is live under the stored surface.
    Resolved,
    /// The target is live but its current label differs from the stored surface. The link
    /// still works by ID; the stored surface is kept, the current label is shown on hover.
    Renamed,
    /// The target resolves to a clear/tombstone: the citation degrades to its label.
    Cleared,
    /// This viewer's store does not hold the target. The rendering is inert text: the viewer
    /// learns only the words the author typed — not the content, not metadata, not existence.
    Unresolved,
}

/// A stored reference as rendered for one viewer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rendered {
    /// Which of the four states the reference is in for this viewer.
    pub state: RenderState,
    /// The text to show. Always built from the stored surface; a renamed target is never
    /// silently rewritten to its new label.
    pub text: String,
    /// The target's current label in this viewer's store, when the target is live here.
    pub current_label: Option<String>,
    /// The target the reference means.
    pub target_id: Hash,
}

/// Renders `reference` against the viewer's local index.
#[must_use]
pub fn render(reference: &Reference, index: &LocalIndex) -> Rendered {
    let sigil = reference.sigil.chr();
    let Some(object) = index.object(&reference.target_id) else {
        return Rendered {
            state: RenderState::Unresolved,
            text: format!("{sigil}{} (unresolved)", reference.surface),
            current_label: None,
            target_id: reference.target_id,
        };
    };
    if object.cleared {
        return Rendered {
            state: RenderState::Cleared,
            text: format!("{sigil}{} (cleared)", reference.surface),
            current_label: None,
            target_id: reference.target_id,
        };
    }
    let current = object.current_label().to_owned();
    if fold(&current) == fold(&reference.surface) {
        Rendered {
            state: RenderState::Resolved,
            text: format!("{sigil}{}", reference.surface),
            current_label: Some(current),
            target_id: reference.target_id,
        }
    } else {
        Rendered {
            state: RenderState::Renamed,
            text: format!("{sigil}{}", reference.surface),
            current_label: Some(current),
            target_id: reference.target_id,
        }
    }
}

/// Parses a typed phrase like `@ann` or `#atlas` into its sigil and phrase.
#[must_use]
pub fn parse_typed(typed: &str) -> Option<(Sigil, &str)> {
    let first = typed.chars().next()?;
    let sigil = Sigil::from_chr(first)?;
    Some((sigil, &typed[first.len_utf8()..]))
}

/// Case-folds a name for comparison. (Display keeps the original case; only matching folds.)
fn fold(text: &str) -> String {
    text.trim().to_lowercase()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn author(n: u8) -> AuthorKey {
        [n; 32]
    }

    fn object(seed: &[u8], kind: Sigil, title: &str, claims: Vec<(&str, u8)>, by: u8) -> IndexedObject {
        IndexedObject {
            id: Hash::digest(seed),
            kind,
            title: title.to_owned(),
            alias_claims: claims
                .into_iter()
                .map(|(text, by)| AliasClaim {
                    text: text.to_owned(),
                    author: author(by),
                })
                .collect(),
            author: author(by),
            cleared: false,
        }
    }

    fn index_with(objects: Vec<IndexedObject>) -> LocalIndex {
        let mut index = LocalIndex::new(StoreId::local());
        for object in objects {
            index.add(object);
        }
        index
    }

    #[test]
    fn petname_outranks_an_exact_claim_for_actors() {
        let ann = object(b"ann", Sigil::Actor, "Ann Lee", vec![], 1);
        let other = object(b"other", Sigil::Actor, "Annette", vec![("ann", 2)], 2);
        let mut index = index_with(vec![ann.clone(), other]);
        index.set_petname(ann.id, "ann");

        let resolution = index.resolve(Sigil::Actor, "ann");
        assert_eq!(resolution.candidates.len(), 2);
        assert_eq!(resolution.candidates[0].target_id, ann.id);
        assert_eq!(resolution.candidates[0].match_kind, MatchKind::Petname);
        assert_eq!(resolution.candidates[1].match_kind, MatchKind::ExactClaim);
        // The petname tier is a single uncontested candidate: it may be committed directly.
        assert_eq!(resolution.single().map(|c| c.target_id), Some(ann.id));
    }

    #[test]
    fn ranking_orders_exact_prefix_token_fulltext() {
        let index = index_with(vec![
            object(b"a", Sigil::Topic, "Atlas", vec![], 1),
            object(b"b", Sigil::Topic, "Atlas Shrugged", vec![], 1),
            object(b"c", Sigil::Topic, "The Atlas", vec![], 1),
            object(b"d", Sigil::Topic, "Myatlas Guide", vec![], 1),
        ]);
        let resolution = index.resolve(Sigil::Topic, "atlas");
        let kinds: Vec<MatchKind> = resolution
            .candidates
            .iter()
            .map(|candidate| candidate.match_kind)
            .collect();
        assert_eq!(
            kinds,
            vec![
                MatchKind::ExactTitle,
                MatchKind::Prefix,
                MatchKind::Token,
                MatchKind::FullText
            ]
        );
        assert!(resolution
            .candidates
            .iter()
            .all(|candidate| candidate.provenance.store == StoreId::local()));
    }

    #[test]
    fn duplicate_titles_are_all_shown_and_never_auto_picked() {
        let first = object(b"m1", Sigil::Topic, "Minutes", vec![], 1);
        let second = object(b"m2", Sigil::Topic, "Minutes", vec![], 2);
        let index = index_with(vec![first, second]);

        let resolution = index.resolve(Sigil::Topic, "minutes");
        assert_eq!(resolution.candidates.len(), 2);
        assert!(resolution.is_contested());
        assert_eq!(resolution.single(), None);
        assert_eq!(
            index.insert_unambiguous(Sigil::Topic, "minutes", author(9)),
            Err(InsertError::Ambiguous(2))
        );
        // The author picks explicitly; provenance tells the two apart.
        let authors: Vec<AuthorKey> = resolution
            .candidates
            .iter()
            .map(|candidate| candidate.provenance.author)
            .collect();
        assert!(authors.contains(&author(1)) && authors.contains(&author(2)));
        let picked = index.insert(&resolution.candidates[0], "minutes", author(9));
        assert_eq!(picked.locator_hints, vec![StoreId::local()]);
    }

    #[test]
    fn a_contested_alias_is_shown_as_contested_with_no_silent_pick() {
        let index = index_with(vec![
            object(b"s1", Sigil::Actor, "Sam One", vec![("sam", 1)], 1),
            object(b"s2", Sigil::Actor, "Sam Two", vec![("sam", 2)], 2),
        ]);
        let resolution = index.resolve(Sigil::Actor, "sam");
        assert_eq!(resolution.candidates.len(), 2);
        assert!(resolution
            .candidates
            .iter()
            .all(|candidate| candidate.contested()));
        assert_eq!(resolution.single(), None);
        assert!(index
            .insert_unambiguous(Sigil::Actor, "sam", author(9))
            .is_err());
    }

    #[test]
    fn insert_then_render_resolved_and_markdown_round_trips() {
        let doc = object(b"doc", Sigil::Topic, "Roadmap", vec![], 1);
        let index = index_with(vec![doc.clone()]);
        let reference = index
            .insert_unambiguous(Sigil::Topic, "roadmap", author(7))
            .unwrap();
        assert_eq!(reference.target_id, doc.id);
        assert_eq!(reference.surface, "roadmap");

        let rendered = render(&reference, &index);
        assert_eq!(rendered.state, RenderState::Resolved);
        assert_eq!(rendered.text, "#roadmap");

        let parsed = Reference::from_markdown(&reference.to_markdown(), Sigil::Topic).unwrap();
        assert_eq!(parsed.target_id, reference.target_id);
        assert_eq!(parsed.surface, reference.surface);
    }

    #[test]
    fn a_renamed_target_keeps_the_stored_surface() {
        let doc = object(b"doc", Sigil::Topic, "Roadmap", vec![], 1);
        let mut index = index_with(vec![doc.clone()]);
        let reference = index
            .insert_unambiguous(Sigil::Topic, "roadmap", author(7))
            .unwrap();

        assert!(index.rename(&doc.id, "Roadmap 2027"));
        let rendered = render(&reference, &index);
        assert_eq!(rendered.state, RenderState::Renamed);
        assert_eq!(rendered.text, "#roadmap");
        assert_eq!(rendered.current_label.as_deref(), Some("Roadmap 2027"));
        assert_eq!(rendered.target_id, doc.id);
    }

    #[test]
    fn a_cleared_target_degrades_to_its_label() {
        let doc = object(b"doc", Sigil::Topic, "Roadmap", vec![], 1);
        let mut index = index_with(vec![doc.clone()]);
        let reference = index
            .insert_unambiguous(Sigil::Topic, "roadmap", author(7))
            .unwrap();

        assert!(index.clear(&doc.id));
        let rendered = render(&reference, &index);
        assert_eq!(rendered.state, RenderState::Cleared);
        assert_eq!(rendered.text, "#roadmap (cleared)");
        assert_eq!(rendered.current_label, None);
    }

    #[test]
    fn a_target_another_viewer_cannot_see_is_unresolved_and_leaks_nothing() {
        let doc = object(b"doc", Sigil::Topic, "Roadmap", vec![], 1);
        let mine = index_with(vec![doc]);
        let reference = mine
            .insert_unambiguous(Sigil::Topic, "roadmap", author(7))
            .unwrap();

        let other_viewer = LocalIndex::new(StoreId::local());
        let rendered = render(&reference, &other_viewer);
        assert_eq!(rendered.state, RenderState::Unresolved);
        assert_eq!(rendered.text, "#roadmap (unresolved)");
        assert_eq!(rendered.current_label, None);
    }

    #[test]
    fn sigils_do_not_cross_and_typed_phrases_parse() {
        let index = index_with(vec![
            object(b"a", Sigil::Actor, "Atlas", vec![], 1),
            object(b"t", Sigil::Topic, "Atlas", vec![], 1),
        ]);
        assert_eq!(index.resolve(Sigil::Actor, "atlas").candidates.len(), 1);
        assert_eq!(index.resolve(Sigil::Topic, "atlas").candidates.len(), 1);
        assert_eq!(
            index.resolve(Sigil::Actor, "atlas").candidates[0].kind,
            Sigil::Actor
        );
        assert_eq!(parse_typed("@ann"), Some((Sigil::Actor, "ann")));
        assert_eq!(parse_typed("#atlas"), Some((Sigil::Topic, "atlas")));
        assert_eq!(parse_typed("ann"), None);
    }

    #[test]
    fn a_title_shared_across_sigils_is_not_contested() {
        // Codex finding on PR 2756: claimants must be scoped to the requested sigil.
        let index = index_with(vec![
            object(b"a", Sigil::Actor, "Atlas", vec![], 1),
            object(b"t", Sigil::Topic, "Atlas", vec![], 1),
        ]);
        let actor = index.resolve(Sigil::Actor, "atlas");
        assert!(!actor.is_contested());
        assert!(actor.single().is_some());
        assert!(index
            .insert_unambiguous(Sigil::Actor, "atlas", author(9))
            .is_ok());
        assert!(index
            .insert_unambiguous(Sigil::Topic, "atlas", author(9))
            .is_ok());
    }

    #[test]
    fn markdown_round_trips_a_surface_containing_markdown_syntax() {
        // Codex finding on PR 2756: the degradation must escape the surface.
        let reference = Reference {
            target_id: Hash::digest(b"target"),
            sigil: Sigil::Topic,
            surface: "Plan ](id://draft)".to_owned(),
            locator_hints: vec![StoreId::local()],
            author_context: author(3),
        };
        let parsed = Reference::from_markdown(&reference.to_markdown(), Sigil::Topic).unwrap();
        assert_eq!(parsed.target_id, reference.target_id);
        assert_eq!(parsed.surface, reference.surface);
    }

    #[test]
    fn a_prefix_match_on_an_alias_keeps_the_claim_author() {
        // Codex finding on PR 2756: provenance follows the matched claim, not the object.
        let index = index_with(vec![object(
            b"o",
            Sigil::Topic,
            "Field Notes",
            vec![("atlas", 2)],
            1,
        )]);
        let resolution = index.resolve(Sigil::Topic, "atl");
        assert_eq!(resolution.candidates.len(), 1);
        assert_eq!(resolution.candidates[0].match_kind, MatchKind::Prefix);
        assert_eq!(resolution.candidates[0].provenance.author, author(2));
    }
}
