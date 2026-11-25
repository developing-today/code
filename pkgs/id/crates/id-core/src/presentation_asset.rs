//! Immutable presentation asset references and declared part maps
//! (slice 1 of the #14731 plan; design record
//! `doc/2026-11-25T01-25-06Z_design_id5970_user_presentation_assets.md`,
//! §2 "Assets by ID").
//!
//! An uploaded asset is stored once and referenced by an immutable
//! `asset-id@revision`. New artwork is a new revision: publishing never
//! changes a revision that already exists, so a reference keeps rendering
//! the revision it names and nothing changes under a viewer by silent
//! replacement.
//!
//! A revision declares its parts (named regions or layers such as
//! `cape`). A presentation reference is
//!
//! ```json
//! { "asset": "hero@2", "filters": ["tint@1"], "parts": { "cape": ["sparkle@1", "tint@2"] } }
//! ```
//!
//! `filters` is the ordered chain for the whole asset and `parts` holds an
//! ordered chain per declared part; order is part of the reference. A
//! chain that targets a part the revision does not declare, or a revision
//! the store does not hold, does not apply to the whole asset instead: the
//! reference steps down to the next authored rung (§4) and the outcome
//! names why. Which rung that is, and the filters themselves, are later
//! slices (the fallback resolver and the named filter registry); this
//! module only reports the step down.
//!
//! Asset ids are per deployment: the store that owns uploads issues them.
//! Naming an asset grants nothing; whether a world may see another world's
//! asset is an authorization question this module does not answer.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Longest asset id, part name or filter name accepted.
pub const MAX_NAME_LEN: usize = 64;

/// Most parts one revision may declare.
pub const MAX_PARTS: usize = 64;

/// Why a reference or name was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetRefError {
    /// The name is empty, too long, or uses a character outside
    /// `[a-z0-9._-]`, or does not start with a letter or digit.
    InvalidName(String),
    /// The text has no `@` separating the name from its revision or
    /// version.
    MissingRevision(String),
    /// The revision or version is not a decimal integer of at least 1
    /// without a sign or leading zero.
    InvalidRevision(String),
    /// A revision declares more parts than [`MAX_PARTS`].
    TooManyParts(usize),
}

impl fmt::Display for AssetRefError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidName(s) => write!(
                f,
                "invalid name {s:?}: use 1 to {MAX_NAME_LEN} of [a-z0-9._-], starting with a letter or digit"
            ),
            Self::MissingRevision(s) => write!(f, "{s:?} has no @revision"),
            Self::InvalidRevision(s) => write!(
                f,
                "invalid revision {s:?}: a decimal integer of at least 1, no sign or leading zero"
            ),
            Self::TooManyParts(n) => write!(f, "{n} parts declared, at most {MAX_PARTS}"),
        }
    }
}

impl std::error::Error for AssetRefError {}

fn check_name(s: &str) -> Result<(), AssetRefError> {
    let ok = !s.is_empty()
        && s.len() <= MAX_NAME_LEN
        && s.bytes()
            .next()
            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && s.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')
        });
    if ok {
        Ok(())
    } else {
        Err(AssetRefError::InvalidName(s.to_owned()))
    }
}

fn check_filter_name(s: &str) -> Result<(), AssetRefError> {
    if s.len() > MAX_NAME_LEN || s.split('/').any(|seg| check_name(seg).is_err()) {
        return Err(AssetRefError::InvalidName(s.to_owned()));
    }
    Ok(())
}

fn parse_revision(s: &str) -> Result<u32, AssetRefError> {
    let bad = || AssetRefError::InvalidRevision(s.to_owned());
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) || s.starts_with('0') {
        return Err(bad());
    }
    s.parse::<u32>().ok().ok_or_else(bad)
}

/// Split `name@number`, validating both halves.
fn split_versioned(s: &str) -> Result<(&str, u32), AssetRefError> {
    let (name, rev) = s
        .rsplit_once('@')
        .ok_or_else(|| AssetRefError::MissingRevision(s.to_owned()))?;
    Ok((name, parse_revision(rev)?))
}

macro_rules! name_type {
    ($(#[$doc:meta])* $ty:ident) => {
        name_type!($(#[$doc])* $ty, check_name);
    };
    ($(#[$doc:meta])* $ty:ident, $check:path) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $ty(String);

        impl $ty {
            /// Validate and wrap a name.
            pub fn new(s: &str) -> Result<Self, AssetRefError> {
                $check(s)?;
                Ok(Self(s.to_owned()))
            }

            /// The name as text.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $ty {
            type Error = AssetRefError;
            fn try_from(s: String) -> Result<Self, Self::Error> {
                $check(&s)?;
                Ok(Self(s))
            }
        }

        impl From<$ty> for String {
            fn from(v: $ty) -> Self {
                v.0
            }
        }

        impl fmt::Display for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

name_type!(
    /// An asset id, issued by the deployment's asset store.
    AssetId
);
name_type!(
    /// A part declared on an asset revision, such as `cape`.
    PartName
);

macro_rules! versioned_type {
    ($(#[$doc:meta])* $ty:ident, $name:ident: $name_ty:ty, $num:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $ty {
            /// The name half.
            pub $name: $name_ty,
            /// The number half, at least 1.
            pub $num: u32,
        }

        impl FromStr for $ty {
            type Err = AssetRefError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                let (name, n) = split_versioned(s)?;
                Ok(Self { $name: <$name_ty>::new(name)?, $num: n })
            }
        }

        impl TryFrom<String> for $ty {
            type Error = AssetRefError;
            fn try_from(s: String) -> Result<Self, Self::Error> {
                s.parse()
            }
        }

        impl From<$ty> for String {
            fn from(v: $ty) -> Self {
                v.to_string()
            }
        }

        impl fmt::Display for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}@{}", self.$name, self.$num)
            }
        }
    };
}

name_type!(
    /// A filter name, such as `tint` or `fx/sparkle`.
    ///
    /// One or more `/`-separated segments, each a plain name. The leading
    /// segments name the filter's family, which world policy can allow or
    /// deny as a whole (`fx/*`). What a filter does is the filter
    /// registry's, a later slice.
    FilterName,
    check_filter_name
);

versioned_type!(
    /// An immutable `asset-id@revision` reference.
    AssetRef, id: AssetId, revision
);
versioned_type!(
    /// A `name@version` filter reference. This slice only names filters;
    /// what a filter does and needs is the filter registry's.
    FilterRef, name: FilterName, version
);

/// One stored revision: the reference that names it and the parts it
/// declares. A revision never changes once published.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetRevision {
    reference: AssetRef,
    parts: BTreeSet<PartName>,
}

impl AssetRevision {
    /// The reference that names this revision.
    pub const fn reference(&self) -> &AssetRef {
        &self.reference
    }

    /// The parts this revision declares.
    pub const fn parts(&self) -> &BTreeSet<PartName> {
        &self.parts
    }

    /// Whether this revision declares `part`.
    pub fn declares(&self, part: &PartName) -> bool {
        self.parts.contains(part)
    }
}

/// The per-deployment store of asset revisions. Publishing appends a new
/// revision; nothing replaces or edits one already published.
#[derive(Debug, Default, Clone)]
pub struct AssetStore {
    revisions: BTreeMap<AssetId, Vec<AssetRevision>>,
}

impl AssetStore {
    /// An empty store.
    pub const fn new() -> Self {
        Self {
            revisions: BTreeMap::new(),
        }
    }

    /// Publish new artwork for `id` with its declared parts. Returns the
    /// new revision's reference: revision 1 for a new id, otherwise one
    /// past the latest. Earlier revisions are untouched.
    pub fn publish<I>(&mut self, id: AssetId, parts: I) -> Result<AssetRef, AssetRefError>
    where
        I: IntoIterator<Item = PartName>,
    {
        let parts: BTreeSet<PartName> = parts.into_iter().collect();
        if parts.len() > MAX_PARTS {
            return Err(AssetRefError::TooManyParts(parts.len()));
        }
        let revs = self.revisions.entry(id.clone()).or_default();
        let next = u32::try_from(revs.len() + 1)
            .ok()
            .ok_or_else(|| AssetRefError::InvalidRevision((revs.len() + 1).to_string()))?;
        let reference = AssetRef { id, revision: next };
        revs.push(AssetRevision {
            reference: reference.clone(),
            parts,
        });
        Ok(reference)
    }

    /// The revision a reference names, if this store holds it.
    pub fn get(&self, reference: &AssetRef) -> Option<&AssetRevision> {
        let idx = usize::try_from(reference.revision.checked_sub(1)?).ok()?;
        self.revisions.get(&reference.id)?.get(idx)
    }

    /// The newest revision of `id`. Used when publishing or choosing art,
    /// never when rendering an existing reference.
    pub fn latest(&self, id: &AssetId) -> Option<&AssetRevision> {
        self.revisions.get(id)?.last()
    }
}

/// A presentation reference: an asset revision plus ordered filter chains
/// for the whole asset and for declared parts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PresentationRef {
    /// The revision shown.
    pub asset: AssetRef,
    /// Ordered filter chain applied to the whole asset.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub filters: Vec<FilterRef>,
    /// Ordered filter chains per declared part.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub parts: BTreeMap<PartName, Vec<FilterRef>>,
}

/// Why a reference cannot render at the rich rung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepDown {
    /// The store holds no such revision (never published, or not
    /// synced here). Never resolved to another revision instead.
    UnknownRevision(AssetRef),
    /// A chain targets parts the revision does not declare. The chain is
    /// never applied to the whole asset instead. Lists every such part,
    /// in name order.
    UndeclaredParts(Vec<PartName>),
}

/// A reference resolved against a store at the rich rung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved<'a> {
    /// The exact revision named.
    pub revision: &'a AssetRevision,
    /// The whole-asset chain, in order.
    pub whole: &'a [FilterRef],
    /// Each declared part's chain, in order.
    pub parts: Vec<(&'a PartName, &'a [FilterRef])>,
}

/// The outcome of resolving a reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution<'a> {
    /// Renders at the rich rung.
    Rich(Resolved<'a>),
    /// Steps down to the next authored rung, for the stated reason.
    StepDown(StepDown),
}

impl PresentationRef {
    /// Resolve this reference against `store`. The exact named revision
    /// is used; a newer revision is never substituted.
    pub fn resolve<'a>(&'a self, store: &'a AssetStore) -> Resolution<'a> {
        let Some(revision) = store.get(&self.asset) else {
            return Resolution::StepDown(StepDown::UnknownRevision(self.asset.clone()));
        };
        let undeclared: Vec<PartName> = self
            .parts
            .keys()
            .filter(|p| !revision.declares(p))
            .cloned()
            .collect();
        if !undeclared.is_empty() {
            return Resolution::StepDown(StepDown::UndeclaredParts(undeclared));
        }
        Resolution::Rich(Resolved {
            revision,
            whole: &self.filters,
            parts: self.parts.iter().map(|(p, c)| (p, c.as_slice())).collect(),
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn id(s: &str) -> AssetId {
        AssetId::new(s).unwrap()
    }
    fn part(s: &str) -> PartName {
        PartName::new(s).unwrap()
    }
    fn f(s: &str) -> FilterRef {
        s.parse().unwrap()
    }

    #[test]
    fn asset_ref_text_form_round_trips_and_refuses_bad_forms() {
        let r: AssetRef = "hero-sprite@12".parse().unwrap();
        assert_eq!(r.id.as_str(), "hero-sprite");
        assert_eq!(r.revision, 12);
        assert_eq!(r.to_string(), "hero-sprite@12");

        for bad in [
            "hero",
            "hero@",
            "@1",
            "hero@0",
            "hero@01",
            "hero@+1",
            "hero@-1",
            "Hero@1",
            "he ro@1",
            "-hero@1",
            "hero@4294967296",
        ] {
            assert!(bad.parse::<AssetRef>().is_err(), "{bad} should be refused");
        }
        let long = format!("{}@1", "a".repeat(MAX_NAME_LEN + 1));
        assert!(long.parse::<AssetRef>().is_err());
        let max = format!("{}@1", "a".repeat(MAX_NAME_LEN));
        assert!(max.parse::<AssetRef>().is_ok());
        // An id cannot itself contain '@': the last '@' splits, and the
        // id half is then refused.
        assert!("a@b@1".parse::<AssetRef>().is_err());

        // Filter names may carry a family path; asset ids may not.
        let fx: FilterRef = "fx/glow/soft@2".parse().unwrap();
        assert_eq!(fx.name.as_str(), "fx/glow/soft");
        for bad in ["fx//glow@1", "/glow@1", "fx/@1", "fx/Glow@1"] {
            assert!(bad.parse::<FilterRef>().is_err(), "{bad} should be refused");
        }
        assert!("fx/glow@1".parse::<AssetRef>().is_err());
    }

    #[test]
    fn replacing_artwork_creates_a_revision_and_old_references_keep_theirs() {
        let mut store = AssetStore::new();
        let v1 = store.publish(id("hero"), [part("cape")]).unwrap();
        let reference = PresentationRef {
            asset: v1.clone(),
            filters: vec![],
            parts: BTreeMap::from([(part("cape"), vec![f("sparkle@1")])]),
        };
        let v2 = store.publish(id("hero"), [part("hood")]).unwrap();
        assert_eq!(v1.revision, 1);
        assert_eq!(v2.revision, 2);
        assert_eq!(store.latest(&id("hero")).unwrap().reference(), &v2);

        // The existing reference still resolves to revision 1, with the
        // parts revision 1 declared.
        match reference.resolve(&store) {
            Resolution::Rich(r) => {
                assert_eq!(r.revision.reference(), &v1);
                assert!(r.revision.declares(&part("cape")));
                assert!(!r.revision.declares(&part("hood")));
            }
            other @ Resolution::StepDown(_) => panic!("expected rich, got {other:?}"),
        }
        // Revision 1 is byte-for-byte what was published.
        assert_eq!(
            store.get(&v1).unwrap().parts(),
            &BTreeSet::from([part("cape")])
        );
    }

    #[test]
    fn undeclared_part_steps_down_instead_of_applying_to_the_whole_asset() {
        let mut store = AssetStore::new();
        let v1 = store.publish(id("hero"), [part("cape")]).unwrap();
        let reference = PresentationRef {
            asset: v1,
            filters: vec![f("tint@1")],
            parts: BTreeMap::from([
                (part("cape"), vec![f("sparkle@1")]),
                (part("wings"), vec![f("glow@2")]),
                (part("aura"), vec![f("glow@2")]),
            ]),
        };
        assert_eq!(
            reference.resolve(&store),
            Resolution::StepDown(StepDown::UndeclaredParts(vec![part("aura"), part("wings")]))
        );
    }

    #[test]
    fn unknown_revision_steps_down_and_is_never_swapped_for_another() {
        let mut store = AssetStore::new();
        store.publish(id("hero"), []).unwrap();
        for missing in ["hero@2", "villain@1"] {
            let asset: AssetRef = missing.parse().unwrap();
            let reference = PresentationRef {
                asset: asset.clone(),
                filters: vec![],
                parts: BTreeMap::new(),
            };
            assert_eq!(
                reference.resolve(&store),
                Resolution::StepDown(StepDown::UnknownRevision(asset))
            );
        }
    }

    #[test]
    fn chains_keep_their_order() {
        let mut store = AssetStore::new();
        let v1 = store.publish(id("hero"), [part("cape")]).unwrap();
        let reference = PresentationRef {
            asset: v1,
            filters: vec![f("tint@2"), f("blur@1")],
            parts: BTreeMap::from([(part("cape"), vec![f("sparkle@1"), f("tint@2")])]),
        };
        let Resolution::Rich(r) = reference.resolve(&store) else {
            panic!("expected rich");
        };
        assert_eq!(r.whole, &[f("tint@2"), f("blur@1")]);
        assert_eq!(
            r.parts,
            vec![(&part("cape"), &[f("sparkle@1"), f("tint@2")][..])]
        );
    }

    #[test]
    fn json_form_matches_the_design_record_and_refuses_unknown_fields() {
        let text =
            r#"{"asset":"hero@2","filters":["tint@1"],"parts":{"cape":["sparkle@1","tint@2"]}}"#;
        let r: PresentationRef = serde_json::from_str(text).unwrap();
        assert_eq!(r.asset.to_string(), "hero@2");
        assert_eq!(r.parts[&part("cape")], vec![f("sparkle@1"), f("tint@2")]);
        assert_eq!(serde_json::to_string(&r).unwrap(), text);

        let bare: PresentationRef = serde_json::from_str(r#"{"asset":"hero@1"}"#).unwrap();
        assert_eq!(
            serde_json::to_string(&bare).unwrap(),
            r#"{"asset":"hero@1"}"#
        );

        for bad in [
            r#"{"asset":"hero"}"#,
            r#"{"asset":"hero@1","shader":"x"}"#,
            r#"{"asset":"hero@1","parts":{"Cape":[]}}"#,
            r#"{"asset":"hero@1","filters":["tint"]}"#,
        ] {
            assert!(
                serde_json::from_str::<PresentationRef>(bad).is_err(),
                "{bad} should be refused"
            );
        }
    }

    #[test]
    fn part_count_is_bounded() {
        let mut store = AssetStore::new();
        let many = (0..=MAX_PARTS).map(|i| part(&format!("p{i}")));
        assert_eq!(
            store.publish(id("hero"), many),
            Err(AssetRefError::TooManyParts(MAX_PARTS + 1))
        );
        assert!(store.latest(&id("hero")).is_none());
    }
}
