//! Friendly-name reference resolution — proof of concept (#2259) plus the first
//! cross-store slice of v2 (#5178).
//!
//! This module is the PoC specified by `doc/2026-10-30T13-25-00Z_design_friendly_name_references.md`
//! (from the #2047 investigation). It is **not** a production resolution feature: it resolves
//! `@`/`#` phrases against in-memory indexes (the notes/documents surface), demonstrates
//! the spec's ranking, and renders stored references in all four states. The #5178 slice
//! adds [`FanOut`]: resolution across several stores' indexes with a deterministic merge
//! (see its docs for the ordering and provenance rules). The #5927 slice adds locator
//! fetch ([`FanOut::fetch_located`] / [`FanOut::render_located`]): following a stored
//! reference's hints to fetch and render a target in another store, with the renamed,
//! cleared, and unresolved-for-viewer states kept distinct across the boundary. There
//! is still no hosted-store search here; that remains a later #5178 slice. The fan-out design
//! record is `doc/2026-11-04T15-33-00Z_design_friendly_name_fanout.md`.
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
//!   Petnames are claims for this rule among themselves: a petname the viewer gave to two or
//!   more actor targets is contested, while one target's petname never contests another
//!   target's title or alias claim — the petname is the viewer's own disambiguation of its
//!   target, which is why it outranks those claims instead of tying with them.
//! - Resolution is viewer-relative: the viewer's own petnames live only in their local index,
//!   so the same phrase may resolve differently for different viewers, and a reference whose
//!   target a viewer cannot see renders as inert text that reveals only the stored surface.
//! - Cleared (tombstoned) objects are excluded from resolution candidates and from surface
//!   claimant counts. A reference inserted while its target is live still renders as cleared if
//!   that target is cleared later.
//! - Cross-store fan-out ([`FanOut`], #5178) merges the per-store resolutions under the same
//!   tier rules: the match tier dominates the merged order, so no store-local boost can lift
//!   a candidate across tiers, and exact-tier contested state is recomputed across **all**
//!   stores — a surface claimed by two targets in two different stores is contested exactly
//!   as if both lived in one. A target a viewer may not see in a store is hidden from that
//!   store's index for the viewer ([`LocalIndex::hide`]): it produces no candidate, does not
//!   contest a visible target's surface, and renders as unresolved — its existence never
//!   leaks through another store's answer.

use std::collections::{BTreeMap, BTreeSet};

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

impl Default for StoreId {
    /// The PoC has exactly one local store, so the default store is the
    /// local one (this is what lets `LocalIndex` derive `Default`).
    fn default() -> Self {
        Self::local()
    }
}

/// A stored reference: the meaning is the target ID, the surface is only a label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    /// The target object's stable logical ID. For notes this is the create-event ID of the
    /// #3814/#3816 model: revisions chain under it, so the reference follows the note across
    /// saves and renames (#2048 design, carried decision on #5178).
    pub target_id: Hash,
    /// An optional exact-revision pin. When `Some`, the reference names that exact revision
    /// of the target; when `None` (the usual case) it follows the stable logical ID to the
    /// target's current revision.
    pub pinned_revision: Option<Hash>,
    /// Which sigil the author typed.
    pub sigil: Sigil,
    /// The phrase as the author typed/chose it. Kept verbatim: a renamed target still renders
    /// with this surface (historical fidelity); it is never silently rewritten.
    pub surface: String,
    /// Stores where the target was found, as hints for the locator fetch
    /// ([`FanOut::fetch_located`] / [`FanOut::render_located`], #5927).
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
        match self.pinned_revision {
            Some(revision) => format!(
                "[{escaped}](id://{}@{})",
                self.target_id.to_hex(),
                revision.to_hex()
            ),
            None => format!("[{escaped}](id://{})", self.target_id.to_hex()),
        }
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
                // Peek, never consume, to test for the delimiter: an
                // unescaped ']' that is not followed by '(' is a literal
                // label character (documented choice for #4465: the surface
                // is kept intact rather than refusing the parse), so the
                // character after it must not be swallowed by the guard.
                ']' if chars.peek() == Some(&'(') => {
                    // Consume the '(' the peek just confirmed.
                    chars.next();
                    let rest: String = chars.collect();
                    // Own the hex slice: `rest` is dropped at the end of
                    // this match arm, so the loop value cannot borrow it.
                    break rest.strip_prefix("id://")?.strip_suffix(')')?.to_owned();
                }
                chr => surface.push(chr),
            }
        };
        // The link target is the stable logical ID, with an optional `@<hex>`
        // exact-revision pin appended by `to_markdown` when one is set.
        let (target_hex, pinned_revision) = match hex.split_once('@') {
            Some((target, revision)) => (target, Some(revision.parse::<Hash>().ok()?)),
            None => (hex.as_str(), None),
        };
        let target_id = target_hex.parse::<Hash>().ok()?;
        Some(Self {
            target_id,
            pinned_revision,
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
    /// The store that answered.
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
/// An index is built for one viewer: targets that viewer may not see are [`hide`](Self::hide)d
/// and then behave exactly as if the store did not hold them.
#[derive(Clone, Debug, Default)]
pub struct LocalIndex {
    store: StoreId,
    objects: Vec<IndexedObject>,
    petnames: BTreeMap<Hash, String>,
    hidden: BTreeSet<Hash>,
}

impl LocalIndex {
    /// An empty index for `store`.
    #[must_use]
    pub fn new(store: StoreId) -> Self {
        Self {
            store,
            objects: Vec::new(),
            petnames: BTreeMap::new(),
            hidden: BTreeSet::new(),
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

    /// Marks a target as not visible to this index's viewer (viewer-specific access
    /// filtering, #5178). A hidden target is indistinguishable from an absent one for
    /// every reader of this index: it yields no resolution candidate, does not count as
    /// a surface claimant (so it cannot even leak its existence by contesting a visible
    /// target's surface), and renders as unresolved. Store maintenance (`rename`,
    /// `clear`) still operates on the underlying object.
    pub fn hide(&mut self, id: &Hash) {
        self.hidden.insert(*id);
    }

    /// Whether this index's viewer may see the object.
    fn visible(&self, object: &IndexedObject) -> bool {
        !self.hidden.contains(&object.id)
    }

    /// The indexed object with `id`, if this store holds it and the viewer may see it.
    #[must_use]
    pub fn object(&self, id: &Hash) -> Option<&IndexedObject> {
        self.objects
            .iter()
            .find(|object| object.id == *id && self.visible(object))
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

    /// How many distinct targets **of the requested sigil** claim `surface` the way
    /// `match_kind` claimed it (case-folded): a petname match counts the targets the viewer
    /// gave that petname (actors only — topics have no petname tier), and every other exact
    /// match counts title and alias claims. Two or more means the surface is contested for
    /// that sigil and claim class. An actor and a topic sharing a title never contest each
    /// other: they answer different sigils, so neither is a competing target for the other's
    /// resolution. Claim classes do not cross either: a petname is the viewer's own
    /// disambiguation of its target, so it outranks a title or alias claim instead of
    /// contesting it — but two targets sharing one petname contest each other.
    fn claimants(&self, sigil: Sigil, surface: &str, match_kind: MatchKind) -> usize {
        self.claimant_ids(sigil, surface, match_kind).len()
    }

    /// The distinct targets claiming `surface` the way `match_kind` claimed it, per
    /// [`claimants`](Self::claimants). Cross-store fan-out unions these sets, so one
    /// target replicated in several stores still counts as a single claimant.
    fn claimant_ids(&self, sigil: Sigil, surface: &str, match_kind: MatchKind) -> BTreeSet<Hash> {
        let folded = fold(surface);
        self.objects
            .iter()
            .filter(|object| object.kind == sigil)
            .filter(|object| !object.cleared && self.visible(object))
            .filter(|object| {
                if match_kind == MatchKind::Petname {
                    sigil == Sigil::Actor
                        && self
                            .petnames
                            .get(&object.id)
                            .is_some_and(|petname| fold(petname) == folded)
                } else {
                    fold(&object.title) == folded
                        || object
                            .alias_claims
                            .iter()
                            .any(|claim| fold(&claim.text) == folded)
                }
            })
            .map(|object| object.id)
            .collect()
    }

    /// Resolves `phrase` for `sigil` in this viewer's index: ranked candidates with
    /// provenance, duplicates and contested candidates included, nothing picked.
    #[must_use]
    pub fn resolve(&self, sigil: Sigil, phrase: &str) -> Resolution {
        let phrase = normalize_phrase(phrase);
        let folded = fold(phrase);
        let mut candidates: Vec<Candidate> = Vec::new();
        if !folded.is_empty() {
            for object in self
                .objects
                .iter()
                .filter(|object| object.kind == sigil && !object.cleared && self.visible(object))
            {
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
        let claim_state = |match_kind: MatchKind, surface: &str| {
            if self.claimants(sigil, surface, match_kind) > 1 {
                ClaimState::Contested
            } else {
                ClaimState::Uncontested
            }
        };
        let candidate =
            |match_kind: MatchKind, surface: String, author: AuthorKey, state: ClaimState| {
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
                    let state = claim_state(MatchKind::Petname, petname);
                    return Some(candidate(
                        MatchKind::Petname,
                        petname.clone(),
                        object.author,
                        state,
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
            let state = claim_state(MatchKind::ExactClaim, &claim.text);
            return Some(candidate(
                MatchKind::ExactClaim,
                object.current_label().to_owned(),
                claim.author,
                state,
            ));
        }
        if fold(&object.title) == folded {
            let state = claim_state(MatchKind::ExactTitle, &object.title);
            return Some(candidate(
                MatchKind::ExactTitle,
                object.current_label().to_owned(),
                object.author,
                state,
            ));
        }
        // Tier 3: prefix, then token, then full-text. A prefix that matches an alias claim
        // keeps that claim's author as provenance — the claim, not the object, is what the
        // author is evaluating.
        if let Some(claim) = object
            .alias_claims
            .iter()
            .find(|claim| fold(&claim.text).starts_with(folded))
        {
            return Some(candidate(
                MatchKind::Prefix,
                object.current_label().to_owned(),
                claim.author,
                ClaimState::Uncontested,
            ));
        }
        if fold(&object.title).starts_with(folded) {
            return Some(candidate(
                MatchKind::Prefix,
                object.current_label().to_owned(),
                object.author,
                ClaimState::Uncontested,
            ));
        }
        if object
            .title
            .split_whitespace()
            .any(|word| fold(word.trim_matches(|chr: char| !chr.is_alphanumeric())) == folded)
        {
            return Some(candidate(
                MatchKind::Token,
                object.current_label().to_owned(),
                object.author,
                ClaimState::Uncontested,
            ));
        }
        if fold(&object.title).contains(folded) {
            return Some(candidate(
                MatchKind::FullText,
                object.current_label().to_owned(),
                object.author,
                ClaimState::Uncontested,
            ));
        }
        None
    }

    /// Stores a reference to a candidate the author picked, with the surface as typed. This
    /// explicit pick is the only way an ambiguous phrase becomes a reference.
    #[must_use]
    pub fn insert(&self, candidate: &Candidate, surface: &str, author: AuthorKey) -> Reference {
        Reference {
            target_id: candidate.target_id,
            pinned_revision: None,
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

/// Cross-store fan-out (#5178, first v2 slice): one viewer's indexes for several stores,
/// resolved as a set.
///
/// The merge contract, from the #5178 design record on the issue:
///
/// - **Tier-first merge.** Candidates from every store are merged under the single
///   [`MatchKind`] ranking: the tier dominates, so a candidate's store can never boost it
///   past a better-tier candidate from another store. Within a tier the order is the
///   folded surface, then the answering store's ID, then the target ID — a total order,
///   so the merged list is deterministic and independent of the order stores were added.
/// - **Provenance is per candidate.** Every candidate keeps the store that answered in
///   its [`Provenance`]; a target held by two stores appears once per store, never
///   silently deduplicated — telling duplicates apart is the author's job, with the
///   provenance in front of them.
/// - **Contested is cross-store.** For the exact tiers (petname, exact claim, exact
///   title), the claimant set is unioned over **all** stores by target ID: a surface
///   claimed by one target in store A and another in store B is contested, and neither
///   candidate may be committed without an explicit pick — the single-store rule,
///   unweakened by fan-out — while one target replicated in both stores stays a single
///   claimant.
/// - **Viewer-relative.** Each store's index is the viewer's own (their petnames, their
///   hidden targets), so fan-out inherits the per-store access filtering: an
///   inaccessible target contributes no candidate and no claimant count in any store.
///
/// What this type is not: hosted-store indexing/search (a later #5178 slice). A
/// fan-out [`Reference`] records every store that holds its target as locator hints,
/// and the locator-fetch slice (#5927) follows them — see
/// [`fetch_located`](Self::fetch_located) and [`render_located`](Self::render_located).
#[derive(Clone, Debug, Default)]
pub struct FanOut {
    stores: Vec<LocalIndex>,
}

impl FanOut {
    /// A fan-out over `stores` (each already scoped to the one viewer).
    #[must_use]
    pub fn new(stores: Vec<LocalIndex>) -> Self {
        Self { stores }
    }

    /// The stores this fan-out queries, in the order they were added.
    #[must_use]
    pub fn stores(&self) -> &[LocalIndex] {
        &self.stores
    }

    /// Resolves `phrase` for `sigil` across every store: the merged, deterministically
    /// ordered candidates with per-store provenance and cross-store contested state.
    #[must_use]
    pub fn resolve(&self, sigil: Sigil, phrase: &str) -> Resolution {
        let phrase = normalize_phrase(phrase);
        let mut candidates: Vec<Candidate> = Vec::new();
        for store in &self.stores {
            candidates.extend(store.resolve(sigil, phrase).candidates);
        }
        // Exact tiers: recompute contested over the union of stores. Each store's
        // claimant set already excludes cleared and viewer-hidden targets, so the
        // union leaks nothing and misses no cross-store claim; unioning IDs (not
        // counts) keeps one target replicated in several stores a single claimant.
        for candidate in &mut candidates {
            if matches!(
                candidate.match_kind,
                MatchKind::Petname | MatchKind::ExactClaim | MatchKind::ExactTitle
            ) {
                let claimants: BTreeSet<Hash> = self
                    .stores
                    .iter()
                    .flat_map(|store| store.claimant_ids(sigil, phrase, candidate.match_kind))
                    .collect();
                candidate.provenance.claim_state = if claimants.len() > 1 {
                    ClaimState::Contested
                } else {
                    ClaimState::Uncontested
                };
            }
        }
        candidates.sort_by(|a, b| {
            a.match_kind
                .cmp(&b.match_kind)
                .then_with(|| fold(&a.surface).cmp(&fold(&b.surface)))
                .then_with(|| a.provenance.store.cmp(&b.provenance.store))
                .then_with(|| a.target_id.to_hex().cmp(&b.target_id.to_hex()))
        });
        Resolution {
            phrase: phrase.to_owned(),
            sigil,
            candidates,
        }
    }

    /// Stores a reference to a candidate the author picked, with the surface as typed.
    /// The locator hints name every store that holds the target live and visible to
    /// this viewer — the input the later locator-fetch slice will follow.
    #[must_use]
    pub fn insert(&self, candidate: &Candidate, surface: &str, author: AuthorKey) -> Reference {
        let locator_hints = self
            .stores
            .iter()
            .filter(|store| {
                store
                    .object(&candidate.target_id)
                    .is_some_and(|object| !object.cleared)
            })
            .map(|store| store.store().clone())
            .collect();
        Reference {
            target_id: candidate.target_id,
            pinned_revision: None,
            sigil: candidate.kind,
            surface: surface.to_owned(),
            locator_hints,
            author_context: author,
        }
    }

    /// Stores a reference for a phrase only when it resolves unambiguously across all
    /// stores. The same target offered by several stores is one choice, not an
    /// ambiguity — its reference simply gains every store as a locator hint.
    ///
    /// # Errors
    ///
    /// Returns [`InsertError::NoMatch`] when nothing matched in any store, and
    /// [`InsertError::Ambiguous`] when the top tier names more than one distinct
    /// target or is contested — the author must pick from the [`Resolution`].
    pub fn insert_unambiguous(
        &self,
        sigil: Sigil,
        phrase: &str,
        author: AuthorKey,
    ) -> Result<Reference, InsertError> {
        let resolution = self.resolve(sigil, phrase);
        if resolution.candidates.is_empty() {
            return Err(InsertError::NoMatch);
        }
        let top = resolution.top_tier();
        let distinct: BTreeSet<Hash> = top.iter().map(|candidate| candidate.target_id).collect();
        match top.first() {
            Some(only) if distinct.len() == 1 && !only.contested() => {
                Ok(self.insert(only, &resolution.phrase, author))
            }
            _ => Err(InsertError::Ambiguous(top.len())),
        }
    }
}

/// A stored reference rendered by following its locator hints across stores
/// (#5927, the locator-fetch slice of #5178): the [`Rendered`] view plus the store
/// that held the target for this viewer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocatedRender {
    /// The rendered reference, in the state the answering store gives it:
    /// resolved, renamed, or cleared when a hinted store holds the target, and
    /// unresolved-for-viewer when none does.
    pub rendered: Rendered,
    /// The store that answered the fetch — the first locator hint whose store
    /// holds the target visibly for this viewer. `None` in the
    /// unresolved-for-viewer state: the render is then built from the stored
    /// surface alone and names no store, so a target that is inaccessible to
    /// this viewer is indistinguishable from an absent one — no candidate, no
    /// claimant, no existence information crosses the boundary.
    pub store: Option<StoreId>,
}

impl FanOut {
    /// The hinted store that holds `reference`'s target for this viewer, if any.
    ///
    /// The hints are the locator contract: when a reference records hints, only
    /// those stores are consulted, in hint order, and the first one holding the
    /// target decides — live, renamed, or cleared. A store where the target is
    /// hidden from this viewer holds nothing (`LocalIndex::object` is
    /// viewer-scoped), so the search passes to the next hint without the hidden
    /// target's existence ever surfacing. With no hints recorded — a reference
    /// parsed back from its markdown degradation carries none — the viewer's
    /// stores are tried in fan-out order.
    fn located_store(&self, reference: &Reference) -> Option<&LocalIndex> {
        if reference.locator_hints.is_empty() {
            return self
                .stores
                .iter()
                .find(|store| store.object(&reference.target_id).is_some());
        }
        for hint in &reference.locator_hints {
            // Every index carrying the hinted store id is examined, in
            // fan-out order: an earlier index with the same id that does
            // not hold the target does not shadow a later one that does.
            for store in &self.stores {
                if store.store() == hint && store.object(&reference.target_id).is_some() {
                    return Some(store);
                }
            }
        }
        None
    }

    /// Fetches the target of `reference` by following its locator hints across
    /// this fan-out's stores (#5927): the answering store and the object as
    /// that store holds it for this viewer (a cleared target is returned with
    /// `cleared` set — rendering decides what that means). `None` when no
    /// hinted store holds the target visibly for this viewer, which covers
    /// both "not there" and "there but inaccessible": the two are deliberately
    /// indistinguishable, so nothing about an inaccessible target leaks.
    ///
    /// The fetch follows the reference's stable logical ID; an exact-revision
    /// pin (`Reference::pinned_revision`) stays on the reference and does not
    /// change which object is fetched — the per-store indexes hold the logical
    /// object, not its revision chain.
    #[must_use]
    pub fn fetch_located(&self, reference: &Reference) -> Option<(&StoreId, &IndexedObject)> {
        let store = self.located_store(reference)?;
        let object = store.object(&reference.target_id)?;
        Some((store.store(), object))
    }

    /// Renders `reference` for this viewer by following its locator hints
    /// across the store boundary (#5927): the single-store [`render`] of the
    /// answering store, so the renamed, cleared, and resolved states mean
    /// exactly what they mean locally, plus the store that answered. When no
    /// hinted store holds the target for this viewer, the state is
    /// [`RenderState::Unresolved`] — the unresolved-for-viewer state, distinct
    /// from cleared (a tombstone the viewer may know about) — the text is
    /// built from the stored surface alone, and no store is named.
    #[must_use]
    pub fn render_located(&self, reference: &Reference) -> LocatedRender {
        match self.located_store(reference) {
            Some(store) => LocatedRender {
                rendered: render(reference, store),
                store: Some(store.store().clone()),
            },
            None => LocatedRender {
                rendered: Rendered {
                    state: RenderState::Unresolved,
                    text: format!(
                        "{}{} (unresolved)",
                        reference.sigil.chr(),
                        reference.surface
                    ),
                    current_label: None,
                    target_id: reference.target_id,
                },
                store: None,
            },
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

/// Normalizes a typed phrase for matching: trims whitespace and strips the leading
/// sigil run, tolerating whitespace between repeated sigils (`# #atlas`). Matching
/// and claimant counting must both see this one normalized phrase — if counting saw
/// a partially stripped phrase, its folded form would keep a sigil, match no claim,
/// and silently clear a real contest (Codex finding on PR #5553).
fn normalize_phrase(phrase: &str) -> &str {
    let mut phrase = phrase.trim();
    while phrase.starts_with(['@', '#']) {
        phrase = phrase[1..].trim();
    }
    phrase
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn author(n: u8) -> AuthorKey {
        [n; 32]
    }

    fn object(
        seed: &[u8],
        kind: Sigil,
        title: &str,
        claims: Vec<(&str, u8)>,
        by: u8,
    ) -> IndexedObject {
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
    fn a_petname_shared_by_two_actors_is_contested() {
        // Regression for #4654: the petname tier used to hardcode Uncontested, so a
        // viewer petname shared by two actors reported no contest at all.
        let first = object(b"p1", Sigil::Actor, "Ann Lee", vec![], 1);
        let second = object(b"p2", Sigil::Actor, "Ann Kim", vec![], 2);
        let mut index = index_with(vec![first.clone(), second.clone()]);
        index.set_petname(first.id, "ann");
        index.set_petname(second.id, "ANN");

        let resolution = index.resolve(Sigil::Actor, "ann");
        assert_eq!(resolution.candidates.len(), 2);
        assert!(
            resolution
                .candidates
                .iter()
                .all(|candidate| candidate.match_kind == MatchKind::Petname)
        );
        assert!(
            resolution
                .candidates
                .iter()
                .all(|candidate| candidate.contested())
        );
        assert!(resolution.is_contested());
        assert_eq!(resolution.single(), None);
        assert!(
            index
                .insert_unambiguous(Sigil::Actor, "ann", author(9))
                .is_err()
        );
    }

    #[test]
    fn a_petname_shared_with_a_topic_is_not_contested() {
        // Sigils do not cross: a topic recorded with the same petname string is not a
        // competing target for the actor's resolution (topics have no petname tier).
        let actor = object(b"pa", Sigil::Actor, "Ann Lee", vec![], 1);
        let topic = object(b"pt", Sigil::Topic, "Ann Notes", vec![], 2);
        let mut index = index_with(vec![actor.clone(), topic.clone()]);
        index.set_petname(actor.id, "ann");
        index.set_petname(topic.id, "ann");

        let resolution = index.resolve(Sigil::Actor, "ann");
        assert_eq!(resolution.candidates.len(), 1);
        assert!(!resolution.is_contested());
        assert_eq!(resolution.single().map(|c| c.target_id), Some(actor.id));
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
        assert!(
            resolution
                .candidates
                .iter()
                .all(|candidate| candidate.provenance.store == StoreId::local())
        );
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
        assert!(
            resolution
                .candidates
                .iter()
                .all(|candidate| candidate.contested())
        );
        assert_eq!(resolution.single(), None);
        assert!(
            index
                .insert_unambiguous(Sigil::Actor, "sam", author(9))
                .is_err()
        );
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
    fn a_cleared_only_target_does_not_resolve_or_insert() {
        let cleared = object(b"cleared", Sigil::Topic, "Roadmap", vec![], 1);
        let mut index = index_with(vec![cleared.clone()]);
        assert!(index.clear(&cleared.id));

        assert!(index.resolve(Sigil::Topic, "roadmap").candidates.is_empty());
        assert_eq!(
            index.insert_unambiguous(Sigil::Topic, "roadmap", author(7)),
            Err(InsertError::NoMatch)
        );
    }

    #[test]
    fn a_cleared_duplicate_does_not_contest_a_live_target() {
        let live = object(b"live", Sigil::Topic, "Roadmap", vec![], 1);
        let cleared = object(b"cleared", Sigil::Topic, "Roadmap", vec![], 2);
        let mut index = index_with(vec![live.clone(), cleared.clone()]);
        assert!(index.clear(&cleared.id));

        let resolution = index.resolve(Sigil::Topic, "roadmap");
        assert_eq!(resolution.candidates.len(), 1);
        assert_eq!(resolution.candidates[0].target_id, live.id);
        assert!(!resolution.is_contested());
        assert_eq!(
            resolution.single().map(|candidate| candidate.target_id),
            Some(live.id)
        );
        let inserted = index
            .insert_unambiguous(Sigil::Topic, "roadmap", author(7))
            .unwrap();
        assert_eq!(inserted.target_id, live.id);
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
        assert!(
            index
                .insert_unambiguous(Sigil::Actor, "atlas", author(9))
                .is_ok()
        );
        assert!(
            index
                .insert_unambiguous(Sigil::Topic, "atlas", author(9))
                .is_ok()
        );
    }

    #[test]
    fn markdown_round_trips_a_surface_containing_markdown_syntax() {
        // Codex finding on PR 2756: the degradation must escape the surface.
        let reference = Reference {
            target_id: Hash::digest(b"target"),
            pinned_revision: None,
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

    #[test]
    fn from_markdown_keeps_the_character_after_an_unescaped_closing_bracket() {
        // Regression for #4465: the delimiter guard used to consume the
        // character after an unescaped ']' while testing for '(', so
        // "[a]b](id://...)" parsed to surface "a]" - the 'b' silently dropped.
        let target_id = Hash::digest(b"target");
        let text = format!("[a]b](id://{})", target_id.to_hex());
        let parsed = Reference::from_markdown(&text, Sigil::Topic).unwrap();
        assert_eq!(parsed.target_id, target_id);
        assert_eq!(parsed.surface, "a]b");
    }

    #[test]
    fn from_markdown_keeps_an_unescaped_closing_bracket_at_the_label_end() {
        // Same regression at the label's end: the first ']' is a literal,
        // the second ']' opens the real "](id://...)" delimiter.
        let target_id = Hash::digest(b"target");
        let text = format!("[a]](id://{})", target_id.to_hex());
        let parsed = Reference::from_markdown(&text, Sigil::Topic).unwrap();
        assert_eq!(parsed.target_id, target_id);
        assert_eq!(parsed.surface, "a]");
    }

    fn store_index(store: &str, objects: Vec<IndexedObject>) -> LocalIndex {
        let mut index = LocalIndex::new(StoreId(store.to_owned()));
        for object in objects {
            index.add(object);
        }
        index
    }

    #[test]
    fn fanout_merges_under_one_tier_order_with_per_store_provenance() {
        // Store B holds an exact-title match, store A only a prefix match: the exact
        // tier wins across stores (nothing boosts a candidate across tiers), and each
        // candidate keeps the store that answered as its provenance.
        let exact = object(b"fanout-exact", Sigil::Topic, "Atlas", vec![], 1);
        let prefix = object(b"fanout-prefix", Sigil::Topic, "Atlas Shrugged", vec![], 2);
        let fanout = FanOut::new(vec![
            store_index("a-store", vec![prefix.clone()]),
            store_index("b-store", vec![exact.clone()]),
        ]);
        let resolution = fanout.resolve(Sigil::Topic, "atlas");
        assert_eq!(resolution.candidates.len(), 2);
        assert_eq!(resolution.candidates[0].target_id, exact.id);
        assert_eq!(resolution.candidates[0].match_kind, MatchKind::ExactTitle);
        assert_eq!(
            resolution.candidates[0].provenance.store,
            StoreId("b-store".to_owned())
        );
        assert_eq!(resolution.candidates[1].match_kind, MatchKind::Prefix);
        assert_eq!(
            resolution.candidates[1].provenance.store,
            StoreId("a-store".to_owned())
        );
        // Deterministic: adding the stores in the other order changes nothing.
        let reversed = FanOut::new(vec![
            store_index("b-store", vec![exact]),
            store_index("a-store", vec![prefix]),
        ]);
        assert_eq!(reversed.resolve(Sigil::Topic, "atlas"), resolution);
    }

    #[test]
    fn fanout_duplicate_titles_across_stores_are_contested_and_never_auto_picked() {
        let a = object(b"dup-a", Sigil::Topic, "Minutes", vec![], 1);
        let b = object(b"dup-b", Sigil::Topic, "Minutes", vec![], 2);
        let fanout = FanOut::new(vec![
            store_index("a-store", vec![a]),
            store_index("b-store", vec![b]),
        ]);
        let resolution = fanout.resolve(Sigil::Topic, "minutes");
        assert_eq!(resolution.candidates.len(), 2);
        assert!(resolution.candidates.iter().all(|c| c.contested()));
        assert!(resolution.is_contested());
        assert_eq!(resolution.single(), None);
        assert!(
            fanout
                .insert_unambiguous(Sigil::Topic, "minutes", author(9))
                .is_err()
        );
    }

    #[test]
    fn fanout_contested_alias_across_stores_is_never_auto_picked() {
        let a = object(b"alias-a", Sigil::Actor, "Sam A", vec![("sam", 1)], 1);
        let b = object(b"alias-b", Sigil::Actor, "Sam B", vec![("sam", 2)], 2);
        let fanout = FanOut::new(vec![
            store_index("a-store", vec![a]),
            store_index("b-store", vec![b]),
        ]);
        let resolution = fanout.resolve(Sigil::Actor, "sam");
        assert_eq!(resolution.candidates.len(), 2);
        assert!(
            resolution
                .candidates
                .iter()
                .all(|c| c.match_kind == MatchKind::ExactClaim && c.contested())
        );
        assert!(
            fanout
                .insert_unambiguous(Sigil::Actor, "sam", author(9))
                .is_err()
        );
    }

    #[test]
    fn fanout_petname_in_one_store_outranks_exact_claim_in_another() {
        let ann = object(b"fan-ann", Sigil::Actor, "Ann Lee", vec![], 1);
        let claimant = object(b"fan-claim", Sigil::Actor, "Annette", vec![("ann", 2)], 2);
        let mut b = store_index("b-store", vec![ann.clone()]);
        b.set_petname(ann.id, "ann");
        let fanout = FanOut::new(vec![store_index("a-store", vec![claimant]), b]);
        let resolution = fanout.resolve(Sigil::Actor, "ann");
        assert_eq!(resolution.candidates.len(), 2);
        assert_eq!(resolution.candidates[0].target_id, ann.id);
        assert_eq!(resolution.candidates[0].match_kind, MatchKind::Petname);
        // The petname tier is a single uncontested candidate across stores too: it may
        // be committed directly, with only the store that holds the target as a hint.
        let reference = fanout
            .insert_unambiguous(Sigil::Actor, "ann", author(9))
            .unwrap();
        assert_eq!(reference.target_id, ann.id);
        assert_eq!(reference.locator_hints, vec![StoreId("b-store".to_owned())]);
    }

    #[test]
    fn fanout_same_target_in_two_stores_is_one_choice_with_both_locator_hints() {
        let shared = object(b"shared", Sigil::Topic, "Roadmap", vec![], 1);
        let fanout = FanOut::new(vec![
            store_index("a-store", vec![shared.clone()]),
            store_index("b-store", vec![shared.clone()]),
        ]);
        let resolution = fanout.resolve(Sigil::Topic, "roadmap");
        // One candidate per store — provenance is shown, not deduplicated away — but
        // a single distinct target, uncontested: replication is not a contest.
        assert_eq!(resolution.candidates.len(), 2);
        assert!(resolution.candidates.iter().all(|c| !c.contested()));
        let reference = fanout
            .insert_unambiguous(Sigil::Topic, "roadmap", author(9))
            .unwrap();
        assert_eq!(reference.target_id, shared.id);
        assert_eq!(
            reference.locator_hints,
            vec![StoreId("a-store".to_owned()), StoreId("b-store".to_owned())]
        );
    }

    #[test]
    fn fanout_hidden_target_leaks_nothing_and_does_not_contest() {
        let visible = object(b"vis", Sigil::Topic, "Atlas", vec![], 1);
        let secret_twin = object(b"secret-twin", Sigil::Topic, "Atlas", vec![], 2);
        let secret_only = object(b"secret-only", Sigil::Topic, "Zephyr", vec![], 2);
        let secret_only_id = secret_only.id;
        let mut b = store_index("b-store", vec![secret_twin.clone(), secret_only]);
        b.hide(&secret_twin.id);
        b.hide(&secret_only_id);
        let fanout = FanOut::new(vec![store_index("a-store", vec![visible.clone()]), b]);
        // The hidden twin produces no candidate and does not even contest the
        // visible target's surface — its existence must not leak.
        let resolution = fanout.resolve(Sigil::Topic, "atlas");
        assert_eq!(resolution.candidates.len(), 1);
        assert_eq!(resolution.candidates[0].target_id, visible.id);
        assert!(!resolution.candidates[0].contested());
        // A phrase matching only a hidden target resolves to nothing at all.
        assert!(fanout.resolve(Sigil::Topic, "zephyr").candidates.is_empty());
        // And a stored reference to the hidden target renders as unresolved in that
        // store: only the stored surface shows, never a label or existence signal.
        let reference = Reference {
            target_id: secret_twin.id,
            pinned_revision: None,
            sigil: Sigil::Topic,
            surface: "Atlas".to_owned(),
            locator_hints: Vec::new(),
            author_context: author(9),
        };
        let rendered = render(&reference, &fanout.stores()[1]);
        assert_eq!(rendered.state, RenderState::Unresolved);
        assert_eq!(rendered.current_label, None);
    }

    #[test]
    fn fanout_repeated_sigils_still_count_claimants_and_stay_contested() {
        // Regression (Codex finding on PR #5553): "# #atlas" was normalized one way
        // for matching and another way for claimant counting — the counting form kept
        // a sigil, matched no claim, and cleared the contest, so insert_unambiguous
        // silently picked the exact-claim candidate over the exact-title one.
        let claimed = object(
            b"sigil-claim",
            Sigil::Topic,
            "Field Notes",
            vec![("atlas", 2)],
            1,
        );
        let titled = object(b"sigil-title", Sigil::Topic, "Atlas", vec![], 3);
        let fanout = FanOut::new(vec![
            store_index("a-store", vec![claimed]),
            store_index("b-store", vec![titled]),
        ]);
        let resolution = fanout.resolve(Sigil::Topic, "# #atlas");
        assert_eq!(resolution.candidates.len(), 2);
        assert!(resolution.candidates.iter().all(|c| c.contested()));
        assert!(
            fanout
                .insert_unambiguous(Sigil::Topic, "# #atlas", author(9))
                .is_err()
        );
    }

    #[test]
    fn a_pinned_revision_round_trips_through_markdown() {
        let reference = Reference {
            target_id: Hash::digest(b"note"),
            pinned_revision: Some(Hash::digest(b"revision-3")),
            sigil: Sigil::Topic,
            surface: "Atlas".to_owned(),
            locator_hints: Vec::new(),
            author_context: author(3),
        };
        let parsed = Reference::from_markdown(&reference.to_markdown(), Sigil::Topic).unwrap();
        assert_eq!(parsed.target_id, reference.target_id);
        assert_eq!(parsed.pinned_revision, reference.pinned_revision);
        // Unpinned references keep the original degradation shape exactly, and parse
        // back with no pin.
        let mut unpinned = reference.clone();
        unpinned.pinned_revision = None;
        assert_eq!(
            unpinned.to_markdown(),
            format!("[Atlas](id://{})", reference.target_id.to_hex())
        );
        let parsed_unpinned =
            Reference::from_markdown(&unpinned.to_markdown(), Sigil::Topic).unwrap();
        assert_eq!(parsed_unpinned.pinned_revision, None);
    }

    fn located_reference(target: Hash, surface: &str, hints: &[&str]) -> Reference {
        Reference {
            target_id: target,
            pinned_revision: None,
            sigil: Sigil::Topic,
            surface: surface.to_owned(),
            locator_hints: hints
                .iter()
                .map(|hint| StoreId((*hint).to_owned()))
                .collect(),
            author_context: author(9),
        }
    }

    #[test]
    fn locator_fetch_renders_a_target_from_another_store() {
        // The target lives only in b-store; the hints name a-store first, so the
        // fetch must cross the boundary to render it at all (#5927 acceptance 1).
        let doc = object(b"loc-doc", Sigil::Topic, "Atlas", vec![], 1);
        let fanout = FanOut::new(vec![
            store_index("a-store", vec![]),
            store_index("b-store", vec![doc.clone()]),
        ]);
        let reference = located_reference(doc.id, "Atlas", &["a-store", "b-store"]);
        let (store, fetched) = fanout.fetch_located(&reference).unwrap();
        assert_eq!(*store, StoreId("b-store".to_owned()));
        assert_eq!(fetched.id, doc.id);
        let located = fanout.render_located(&reference);
        assert_eq!(located.rendered.state, RenderState::Resolved);
        assert_eq!(located.rendered.current_label.as_deref(), Some("Atlas"));
        assert_eq!(located.store, Some(StoreId("b-store".to_owned())));
    }

    #[test]
    fn locator_fetch_renders_renamed_across_the_boundary() {
        // The target was renamed in its home store after the reference was
        // stored: the stored surface is kept, the current label is the new one,
        // and the state is Renamed — distinct from Resolved and Cleared.
        let doc = object(b"loc-renamed", Sigil::Topic, "Atlas Revised", vec![], 1);
        let fanout = FanOut::new(vec![store_index("b-store", vec![doc.clone()])]);
        let reference = located_reference(doc.id, "Atlas", &["b-store"]);
        let located = fanout.render_located(&reference);
        assert_eq!(located.rendered.state, RenderState::Renamed);
        assert_eq!(located.rendered.text, "#Atlas");
        assert_eq!(
            located.rendered.current_label.as_deref(),
            Some("Atlas Revised")
        );
        assert_eq!(located.store, Some(StoreId("b-store".to_owned())));
    }

    #[test]
    fn locator_fetch_renders_cleared_across_the_boundary() {
        // A tombstone in the hinted store is a state the viewer may know about:
        // Cleared, with the answering store named — distinct from unresolved.
        let doc = object(b"loc-cleared", Sigil::Topic, "Atlas", vec![], 1);
        let mut b = store_index("b-store", vec![doc.clone()]);
        assert!(b.clear(&doc.id));
        let fanout = FanOut::new(vec![b]);
        let reference = located_reference(doc.id, "Atlas", &["b-store"]);
        let located = fanout.render_located(&reference);
        assert_eq!(located.rendered.state, RenderState::Cleared);
        assert_eq!(located.rendered.current_label, None);
        assert_eq!(located.store, Some(StoreId("b-store".to_owned())));
        // The fetch itself returns the tombstone; rendering decides its state.
        let (_, fetched) = fanout.fetch_located(&reference).unwrap();
        assert!(fetched.cleared);
    }

    #[test]
    fn locator_fetch_unresolved_for_viewer_is_distinct_and_names_no_store() {
        // No hinted store holds the target for this viewer (it lives in a store
        // the hints do not name, and one hint names a store not in the set at
        // all): unresolved-for-viewer — inert text from the stored surface
        // alone, no current label, no store named.
        let doc = object(b"loc-elsewhere", Sigil::Topic, "Atlas", vec![], 1);
        let fanout = FanOut::new(vec![
            store_index("a-store", vec![]),
            store_index("b-store", vec![doc.clone()]),
        ]);
        let reference = located_reference(doc.id, "Atlas", &["a-store", "gone-store"]);
        assert_eq!(fanout.fetch_located(&reference), None);
        let located = fanout.render_located(&reference);
        assert_eq!(located.rendered.state, RenderState::Unresolved);
        assert_eq!(located.rendered.text, "#Atlas (unresolved)");
        assert_eq!(located.rendered.current_label, None);
        assert_eq!(located.store, None);
    }

    #[test]
    fn locator_fetch_hidden_target_leaks_nothing() {
        // The hinted store holds the target but this viewer may not see it:
        // the fetch must behave exactly as if the target were absent — no
        // object, no label, no store named, no existence signal (#5927
        // acceptance 3).
        let secret = object(b"loc-secret", Sigil::Topic, "Zephyr", vec![], 2);
        let mut b = store_index("b-store", vec![secret.clone()]);
        b.hide(&secret.id);
        let fanout = FanOut::new(vec![b]);
        let reference = located_reference(secret.id, "Zephyr", &["b-store"]);
        assert_eq!(fanout.fetch_located(&reference), None);
        let located = fanout.render_located(&reference);
        assert_eq!(located.rendered.state, RenderState::Unresolved);
        assert_eq!(located.rendered.text, "#Zephyr (unresolved)");
        assert_eq!(located.rendered.current_label, None);
        assert_eq!(located.store, None);
    }

    #[test]
    fn locator_fetch_follows_hint_order() {
        // One target replicated in two stores whose copies have diverged: the
        // hint order, not the fan-out order, picks the answering store.
        let in_a = object(b"loc-order", Sigil::Topic, "Atlas Renamed", vec![], 1);
        let in_b = object(b"loc-order", Sigil::Topic, "Atlas", vec![], 1);
        assert_eq!(in_a.id, in_b.id);
        let target_id = in_a.id;
        let fanout = FanOut::new(vec![
            store_index("a-store", vec![in_a]),
            store_index("b-store", vec![in_b]),
        ]);
        let b_first = located_reference(target_id, "Atlas", &["b-store", "a-store"]);
        let located = fanout.render_located(&b_first);
        assert_eq!(located.rendered.state, RenderState::Resolved);
        assert_eq!(located.store, Some(StoreId("b-store".to_owned())));
        let a_first = located_reference(target_id, "Atlas", &["a-store", "b-store"]);
        let located = fanout.render_located(&a_first);
        assert_eq!(located.rendered.state, RenderState::Renamed);
        assert_eq!(located.store, Some(StoreId("a-store".to_owned())));
    }

    #[test]
    fn locator_fetch_first_holding_hint_decides_even_when_cleared() {
        // Cleared in the first hinted store, live in the second: the first
        // store holding the target decides, so the render is Cleared — a
        // tombstone is an answer, not an absence to skip past.
        let doc = object(b"loc-tombstone", Sigil::Topic, "Atlas", vec![], 1);
        let mut a = store_index("a-store", vec![doc.clone()]);
        assert!(a.clear(&doc.id));
        let fanout = FanOut::new(vec![a, store_index("b-store", vec![doc.clone()])]);
        let reference = located_reference(doc.id, "Atlas", &["a-store", "b-store"]);
        let located = fanout.render_located(&reference);
        assert_eq!(located.rendered.state, RenderState::Cleared);
        assert_eq!(located.store, Some(StoreId("a-store".to_owned())));
    }

    #[test]
    fn locator_fetch_without_hints_uses_the_store_set_in_order() {
        // A reference parsed back from markdown carries no hints; it still
        // renders against the viewer's store set, in fan-out order.
        let doc = object(b"loc-nohints", Sigil::Topic, "Atlas", vec![], 1);
        let fanout = FanOut::new(vec![
            store_index("a-store", vec![]),
            store_index("b-store", vec![doc.clone()]),
        ]);
        let reference = located_reference(doc.id, "Atlas", &[]);
        let located = fanout.render_located(&reference);
        assert_eq!(located.rendered.state, RenderState::Resolved);
        assert_eq!(located.store, Some(StoreId("b-store".to_owned())));
    }

    #[test]
    fn locator_fetch_keeps_an_exact_revision_pin_working() {
        // A pinned reference follows its logical ID across the boundary
        // exactly like an unpinned one; the pin stays on the reference.
        let doc = object(b"loc-pinned", Sigil::Topic, "Atlas", vec![], 1);
        let fanout = FanOut::new(vec![store_index("b-store", vec![doc.clone()])]);
        let mut reference = located_reference(doc.id, "Atlas", &["b-store"]);
        reference.pinned_revision = Some(Hash::digest(b"loc-pinned-rev-3"));
        let located = fanout.render_located(&reference);
        assert_eq!(located.rendered.state, RenderState::Resolved);
        assert_eq!(located.rendered.target_id, doc.id);
        assert_eq!(
            reference.pinned_revision,
            Some(Hash::digest(b"loc-pinned-rev-3"))
        );
    }
    #[test]
    fn locator_fetch_checks_every_index_carrying_a_hinted_store_id() {
        // Two indexes share one store id; only the second holds the
        // target. The hint must reach the second index -- the first
        // index carrying the id does not shadow it (CodeRabbit
        // finding on PR #5932).
        let doc = object(b"loc-dup-id", Sigil::Topic, "Atlas", vec![], 1);
        let fanout = FanOut::new(vec![
            store_index("b-store", vec![]),
            store_index("b-store", vec![doc.clone()]),
        ]);
        let reference = located_reference(doc.id, "Atlas", &["b-store"]);
        let located = fanout.render_located(&reference);
        assert_eq!(located.rendered.state, RenderState::Resolved);
        assert_eq!(located.store, Some(StoreId("b-store".to_owned())));
    }

    // ---- Locator-selection property coverage (Codex P1 on PR #5932) ----
    //
    // The locator_fetch_* tests above are hand-written examples. The
    // locator contract also has invariants that must hold for EVERY
    // store layout and hint list, so they are stated here as
    // properties checked against a small selection model (the
    // oracle): the answering store is the first store in hint order
    // -- or in fan-out order when the reference carries no hints --
    // whose copy of the target is visible to this viewer (live or
    // cleared); hidden and absent copies never answer, and stores
    // the hints do not name cannot affect a hinted fetch at all.
    use proptest::prelude::*;

    /// One store's placement of the property-test target.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Placement {
        Absent,
        LiveSame,
        LiveRenamed,
        Cleared,
        Hidden,
    }

    const PLACEMENTS: [Placement; 5] = [
        Placement::Absent,
        Placement::LiveSame,
        Placement::LiveRenamed,
        Placement::Cleared,
        Placement::Hidden,
    ];

    const PROP_STORES: [&str; 3] = ["a-store", "b-store", "c-store"];
    const PROP_SURFACE: &str = "Atlas";

    fn prop_fanout(placements: &[Placement; 3]) -> (FanOut, Hash) {
        let target = object(b"loc-prop-target", Sigil::Topic, PROP_SURFACE, vec![], 1);
        let target_id = target.id;
        let mut stores = Vec::new();
        for (i, placement) in placements.iter().enumerate() {
            let decoy = object(b"loc-prop-decoy", Sigil::Topic, "Decoy", vec![], 3);
            let mut index = store_index(PROP_STORES[i], vec![decoy]);
            match placement {
                Placement::Absent => {}
                Placement::LiveSame => index.add(target.clone()),
                Placement::LiveRenamed => {
                    let mut renamed = target.clone();
                    renamed.title = "Atlas Revised".to_owned();
                    index.add(renamed);
                }
                Placement::Cleared => {
                    index.add(target.clone());
                    assert!(index.clear(&target_id));
                }
                Placement::Hidden => {
                    let mut hidden = target.clone();
                    hidden.title = "Zephyr".to_owned();
                    index.add(hidden);
                    index.hide(&target_id);
                }
            }
            stores.push(index);
        }
        (FanOut::new(stores), target_id)
    }

    /// The selection model: which store answers, and with which
    /// placement. Visible means live or cleared; hidden copies are
    /// indistinguishable from absent ones and never answer.
    fn prop_model(placements: &[Placement; 3], hints: &[usize]) -> Option<(usize, Placement)> {
        let visible = |p: Placement| {
            matches!(
                p,
                Placement::LiveSame | Placement::LiveRenamed | Placement::Cleared
            )
        };
        if hints.is_empty() {
            return (0..3).find_map(|i| visible(placements[i]).then_some((i, placements[i])));
        }
        hints
            .iter()
            .filter(|&&h| h < 3)
            .find_map(|&i| visible(placements[i]).then_some((i, placements[i])))
    }

    fn prop_fetch_tuple(
        fanout: &FanOut,
        reference: &Reference,
    ) -> Option<(StoreId, Hash, bool, String)> {
        fanout.fetch_located(reference).map(|(store, object)| {
            (
                store.clone(),
                object.id,
                object.cleared,
                object.title.clone(),
            )
        })
    }

    proptest! {
        #[test]
        fn locator_selection_matches_the_model(
            placement_idx in proptest::collection::vec(0usize..5, 3),
            hint_idx in proptest::collection::vec(0usize..4, 0..5),
        ) {
            let placements: [Placement; 3] = [
                PLACEMENTS[placement_idx[0]],
                PLACEMENTS[placement_idx[1]],
                PLACEMENTS[placement_idx[2]],
            ];
            let (fanout, target_id) = prop_fanout(&placements);
            let hint_names: Vec<String> = hint_idx
                .iter()
                .map(|&h| {
                    if h < 3 {
                        PROP_STORES[h].to_owned()
                    } else {
                        "gone-store".to_owned()
                    }
                })
                .collect();
            let hint_refs: Vec<&str> = hint_names.iter().map(String::as_str).collect();
            let reference = located_reference(target_id, PROP_SURFACE, &hint_refs);

            let model = prop_model(&placements, &hint_idx);
            // Fetch: the model names the answering store and its copy.
            let expected_fetch = model.map(|(i, placement)| {
                let title = match placement {
                    Placement::LiveRenamed => "Atlas Revised".to_owned(),
                    _ => PROP_SURFACE.to_owned(),
                };
                (
                    StoreId(PROP_STORES[i].to_owned()),
                    target_id,
                    placement == Placement::Cleared,
                    title,
                )
            });
            prop_assert_eq!(prop_fetch_tuple(&fanout, &reference), expected_fetch);
            // Render: same answering store; the state follows the
            // copy that store holds.
            let located = fanout.render_located(&reference);
            let (want_store, want_state, want_label, want_text) = match model {
                Some((i, Placement::LiveSame)) => (
                    Some(StoreId(PROP_STORES[i].to_owned())),
                    RenderState::Resolved,
                    Some(PROP_SURFACE.to_owned()),
                    format!("#{}", PROP_SURFACE),
                ),
                Some((i, Placement::LiveRenamed)) => (
                    Some(StoreId(PROP_STORES[i].to_owned())),
                    RenderState::Renamed,
                    Some("Atlas Revised".to_owned()),
                    format!("#{}", PROP_SURFACE),
                ),
                Some((i, Placement::Cleared)) => (
                    Some(StoreId(PROP_STORES[i].to_owned())),
                    RenderState::Cleared,
                    None,
                    format!("#{} (cleared)", PROP_SURFACE),
                ),
                _ => (
                    None,
                    RenderState::Unresolved,
                    None,
                    format!("#{} (unresolved)", PROP_SURFACE),
                ),
            };
            prop_assert_eq!(located.store, want_store);
            prop_assert_eq!(located.rendered.state, want_state);
            prop_assert_eq!(located.rendered.current_label, want_label);
            prop_assert_eq!(located.rendered.text, want_text);

            // Invariant: a hidden copy is indistinguishable from an
            // absent one -- swapping every Hidden placement for
            // Absent changes neither the fetch nor the render.
            let mut absent_swapped = placements;
            for placement in &mut absent_swapped {
                if *placement == Placement::Hidden {
                    *placement = Placement::Absent;
                }
            }
            let (swapped_fanout, _) = prop_fanout(&absent_swapped);
            prop_assert_eq!(
                prop_fetch_tuple(&swapped_fanout, &reference),
                prop_fetch_tuple(&fanout, &reference)
            );
            prop_assert_eq!(
                swapped_fanout.render_located(&reference),
                fanout.render_located(&reference)
            );

            // Invariant: when the reference carries hints, stores
            // the hints do not name cannot affect the outcome --
            // scrambling every unhinted store's placement changes
            // neither the fetch nor the render.
            if !hint_idx.is_empty() {
                let mut scrambled_idx = placement_idx.clone();
                for (i, idx) in scrambled_idx.iter_mut().enumerate() {
                    if !hint_idx.contains(&i) {
                        *idx = (*idx + 2) % 5;
                    }
                }
                let scrambled: [Placement; 3] = [
                    PLACEMENTS[scrambled_idx[0]],
                    PLACEMENTS[scrambled_idx[1]],
                    PLACEMENTS[scrambled_idx[2]],
                ];
                let (scrambled_fanout, _) = prop_fanout(&scrambled);
                prop_assert_eq!(
                    prop_fetch_tuple(&scrambled_fanout, &reference),
                    prop_fetch_tuple(&fanout, &reference)
                );
                prop_assert_eq!(
                    scrambled_fanout.render_located(&reference),
                    fanout.render_located(&reference)
                );
            }
        }
    }
}
