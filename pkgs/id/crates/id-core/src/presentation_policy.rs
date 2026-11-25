//! World presentation policy (slice 2 of the #14731 plan; design record
//! `doc/2026-11-25T01-25-06Z_design_id5970_user_presentation_assets.md`,
//! §5 "World presentation policy", with §4's rungs and §6's abuse rules).
//!
//! A world declares which presentation capabilities it enables:
//!
//! ```json
//! {
//!   "assets":   { "user_uploads": "allow", "max_rung": "rich" },
//!   "filters":  { "allowed": ["tint@1", "fx/*"], "denied": ["fx/strobe@1"], "disguise": "deny" },
//!   "shaders":  { "enabled": true, "formats": ["shadertoy.glsl.fragment/v1"] },
//!   "fallback": { "world_glyphs": { "tree": "♣" }, "world_substitutes": { "hero": "hero-static@1" }, "tileset": "forest-tiles@3" }
//! }
//! ```
//!
//! Every section and field may be omitted. The defaults deny user shaders
//! and disguise-class filters (§5, §6) and allow everything else up to the
//! rich rung, so an empty policy is the conservative one, not the open
//! one.
//!
//! [`PresentationPolicy::admit`] is the adapter-side check (§5: policy is
//! enforced where rendering happens, never trusted to client UI). It never
//! answers "blank": a denial always names the rung the reference falls to
//! and every reason, so the presentation diagnostic can say why.
//!
//! Shader formats come from a closed, code-owned registry
//! ([`ShaderFormat`], #6013 §3.1). A policy that names any other format is
//! refused when parsed, so a world cannot enable a format no trusted
//! adapter renders.
//!
//! Fallback overrides (§4: world glyphs, world substitutes, a tileset) are
//! validated and carried here; choosing between rungs is the fallback
//! resolver's, a later slice.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::presentation_asset::{AssetId, AssetRef, FilterName, FilterRef};

/// Longest glyph override, in bytes.
pub const MAX_GLYPH_BYTES: usize = 16;

/// Most entries in any one policy list or map.
pub const MAX_POLICY_ENTRIES: usize = 256;

/// One rung of the authored fallback chain (§4), richest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rung {
    /// The asset plus its composed filters.
    Rich,
    /// The asset author's pre-rendered static substitute.
    Static,
    /// The cell's tile from the world's tileset.
    Tileset,
    /// The cell's glyph. Always available.
    Glyph,
}

impl Rung {
    /// The poorer of two rungs.
    pub fn poorer(self, other: Self) -> Self {
        self.max(other)
    }
}

/// Allow or deny.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permit {
    /// Allowed.
    Allow,
    /// Denied.
    Deny,
}

/// A shader format from the closed registry (#6013 §3.1). Only a trusted
/// adapter adds a variant; uploaded metadata never registers a format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum ShaderFormat {
    /// `shadertoy.glsl.fragment/v1`: a Shadertoy-compatible GLSL fragment
    /// unit.
    ShadertoyGlslFragmentV1,
}

impl ShaderFormat {
    /// Every registered format.
    pub const ALL: [Self; 1] = [Self::ShadertoyGlslFragmentV1];

    /// The registered name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ShadertoyGlslFragmentV1 => "shadertoy.glsl.fragment/v1",
        }
    }
}

impl fmt::Display for ShaderFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ShaderFormat {
    type Err = PolicyError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|f| f.as_str() == s)
            .ok_or_else(|| PolicyError::UnregisteredShaderFormat(s.to_owned()))
    }
}

impl TryFrom<String> for ShaderFormat {
    type Error = PolicyError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl From<ShaderFormat> for String {
    fn from(f: ShaderFormat) -> Self {
        f.as_str().to_owned()
    }
}

/// Why a policy document was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyError {
    /// A shader format that is not in the registry.
    UnregisteredShaderFormat(String),
    /// A filter pattern that is neither `name@version`, `name` nor
    /// `family/*`.
    InvalidFilterPattern(String),
    /// A glyph override that is empty, too long, or holds a control
    /// character.
    InvalidGlyph(String),
    /// A list or map with more than [`MAX_POLICY_ENTRIES`] entries.
    TooManyEntries(&'static str),
}

impl fmt::Display for PolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnregisteredShaderFormat(s) => write!(f, "shader format {s:?} is not registered"),
            Self::InvalidFilterPattern(s) => {
                write!(
                    f,
                    "filter pattern {s:?}: use name@version, name, or family/*"
                )
            }
            Self::InvalidGlyph(s) => write!(
                f,
                "glyph override {s:?} is empty, too long or a control character"
            ),
            Self::TooManyEntries(what) => {
                write!(f, "{what} has more than {MAX_POLICY_ENTRIES} entries")
            }
        }
    }
}

impl std::error::Error for PolicyError {}

/// A filter allow or deny entry: one version, every version of a name, or
/// a whole family.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum FilterPattern {
    /// `name@version`: that version only.
    Exact(FilterRef),
    /// `name`: every version of that name.
    AnyVersion(FilterName),
    /// `family/*`: every filter whose name starts with `family/`.
    Family(FilterName),
}

impl FilterPattern {
    /// Whether this pattern covers `filter`.
    pub fn matches(&self, filter: &FilterRef) -> bool {
        match self {
            Self::Exact(r) => r == filter,
            Self::AnyVersion(n) => n == &filter.name,
            Self::Family(fam) => filter
                .name
                .as_str()
                .strip_prefix(fam.as_str())
                .is_some_and(|rest| rest.starts_with('/')),
        }
    }
}

impl FromStr for FilterPattern {
    type Err = PolicyError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parsed = if let Some(fam) = s.strip_suffix("/*") {
            FilterName::new(fam).ok().map(Self::Family)
        } else if s.contains('@') {
            s.parse().ok().map(Self::Exact)
        } else {
            FilterName::new(s).ok().map(Self::AnyVersion)
        };
        parsed.ok_or_else(|| PolicyError::InvalidFilterPattern(s.to_owned()))
    }
}

impl TryFrom<String> for FilterPattern {
    type Error = PolicyError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl From<FilterPattern> for String {
    fn from(p: FilterPattern) -> Self {
        match p {
            FilterPattern::Exact(r) => r.to_string(),
            FilterPattern::AnyVersion(n) => n.to_string(),
            FilterPattern::Family(n) => format!("{n}/*"),
        }
    }
}

/// A glyph override: a short, printable string.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Glyph(String);

impl Glyph {
    /// Validate a glyph override.
    pub fn new(s: &str) -> Result<Self, PolicyError> {
        if s.is_empty() || s.len() > MAX_GLYPH_BYTES || s.chars().any(char::is_control) {
            return Err(PolicyError::InvalidGlyph(s.to_owned()));
        }
        Ok(Self(s.to_owned()))
    }

    /// The glyph text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Glyph {
    type Error = PolicyError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(&s)
    }
}

impl From<Glyph> for String {
    fn from(g: Glyph) -> Self {
        g.0
    }
}

/// `assets:` — whether user uploads render, and the richest rung allowed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AssetsPolicy {
    /// Whether user-uploaded assets render. Default: allow.
    pub user_uploads: Permit,
    /// The richest rung this world renders. Default: rich.
    pub max_rung: Rung,
}

impl Default for AssetsPolicy {
    fn default() -> Self {
        Self {
            user_uploads: Permit::Allow,
            max_rung: Rung::Rich,
        }
    }
}

/// `filters:` — which named filters may apply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FiltersPolicy {
    /// When present, only filters a pattern here covers may apply. When
    /// absent, every filter not denied may apply.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed: Option<Vec<FilterPattern>>,
    /// Filters that never apply; wins over `allowed`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub denied: Vec<FilterPattern>,
    /// Whether disguise-class filters may apply. Default: deny (§6).
    pub disguise: Permit,
}

impl Default for FiltersPolicy {
    fn default() -> Self {
        Self {
            allowed: None,
            denied: Vec::new(),
            disguise: Permit::Deny,
        }
    }
}

/// `shaders:` — whether shader-backed filters run, and in which formats.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ShadersPolicy {
    /// Whether shaders run at all. Default: false.
    pub enabled: bool,
    /// The registered formats this world enables.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub formats: Vec<ShaderFormat>,
}

/// `fallback:` — the world's own lower-rung art (§4). Carried for the
/// fallback resolver; this slice only validates it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FallbackOverrides {
    /// Themed glyphs by cell kind, overriding the client table.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub world_glyphs: BTreeMap<String, Glyph>,
    /// Themed static substitutes by asset id.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub world_substitutes: BTreeMap<AssetId, AssetRef>,
    /// The world's tileset asset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tileset: Option<AssetRef>,
}

/// A world's `presentation:` declaration. Published in join and preview
/// metadata so a user sees it before joining.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PresentationPolicy {
    /// Asset rules.
    pub assets: AssetsPolicy,
    /// Filter rules.
    pub filters: FiltersPolicy,
    /// Shader rules.
    pub shaders: ShadersPolicy,
    /// The world's fallback art.
    pub fallback: FallbackOverrides,
}

/// What the adapter knows about one filter in a reference. Where the
/// facts come from (the filter registry) is a later slice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterFacts {
    /// The filter.
    pub filter: FilterRef,
    /// Whether it is disguise-class (shape change, near-invisibility,
    /// false name plates).
    pub disguise: bool,
    /// The shader format it runs, if it is shader-backed.
    pub shader: Option<ShaderFormat>,
}

/// What the adapter knows about one presentation reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceFacts {
    /// Whether the asset is a user upload.
    pub user_upload: bool,
    /// Every filter the reference applies, whole-asset and per-part.
    pub filters: Vec<FilterFacts>,
}

/// One reason a reference cannot render at the rung it asked for.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Denial {
    /// The world renders nothing richer than this rung.
    MaxRung(Rung),
    /// The world does not render user uploads.
    UserUploads,
    /// The filter is on the world's deny list.
    FilterDenied(FilterRef),
    /// The world has an allow list and it does not cover the filter.
    FilterNotAllowed(FilterRef),
    /// The filter is disguise-class and the world denies disguise.
    Disguise(FilterRef),
    /// The filter is shader-backed and the world has shaders off.
    ShadersDisabled(FilterRef),
    /// The filter's shader format is not one the world enables.
    ShaderFormatNotEnabled(FilterRef, ShaderFormat),
}

/// The adapter's decision for one reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Admission {
    /// Renders at the rich rung, as authored.
    Rich,
    /// Falls to the named rung, for every listed reason (sorted).
    Falls {
        /// The rung it renders at instead.
        to: Rung,
        /// Why, every reason.
        because: Vec<Denial>,
    },
}

impl PresentationPolicy {
    /// Parse and validate a policy from JSON.
    pub fn from_json(text: &str) -> Result<Self, String> {
        let policy: Self = serde_json::from_str(text).map_err(|e| e.to_string())?;
        policy.validate().map_err(|e| e.to_string())?;
        Ok(policy)
    }

    /// Check the size bounds serde cannot express.
    pub fn validate(&self) -> Result<(), PolicyError> {
        let f = &self.filters;
        if f.allowed
            .as_ref()
            .is_some_and(|a| a.len() > MAX_POLICY_ENTRIES)
        {
            return Err(PolicyError::TooManyEntries("filters.allowed"));
        }
        if f.denied.len() > MAX_POLICY_ENTRIES {
            return Err(PolicyError::TooManyEntries("filters.denied"));
        }
        if self.fallback.world_glyphs.len() > MAX_POLICY_ENTRIES {
            return Err(PolicyError::TooManyEntries("fallback.world_glyphs"));
        }
        if self.fallback.world_substitutes.len() > MAX_POLICY_ENTRIES {
            return Err(PolicyError::TooManyEntries("fallback.world_substitutes"));
        }
        Ok(())
    }

    fn filter_denials(&self, facts: &FilterFacts, out: &mut Vec<Denial>) {
        let f = &facts.filter;
        let p = &self.filters;
        if p.denied.iter().any(|d| d.matches(f)) {
            out.push(Denial::FilterDenied(f.clone()));
        } else if p
            .allowed
            .as_ref()
            .is_some_and(|a| !a.iter().any(|x| x.matches(f)))
        {
            out.push(Denial::FilterNotAllowed(f.clone()));
        }
        if facts.disguise && p.disguise == Permit::Deny {
            out.push(Denial::Disguise(f.clone()));
        }
        if let Some(format) = facts.shader {
            if !self.shaders.enabled {
                out.push(Denial::ShadersDisabled(f.clone()));
            } else if !self.shaders.formats.contains(&format) {
                out.push(Denial::ShaderFormatNotEnabled(f.clone(), format));
            }
        }
    }

    /// Decide whether a reference renders rich under this policy and, if
    /// not, which rung it falls to. Deterministic for equal inputs.
    ///
    /// - A denied user upload loses the asset author's rungs (rich and
    ///   static) and falls to the tileset.
    /// - A denied filter loses the rich rung only; the asset author's
    ///   static substitute still renders.
    /// - `max_rung` caps the result.
    pub fn admit(&self, reference: &ReferenceFacts) -> Admission {
        let mut because = Vec::new();
        let mut to = Rung::Rich;
        if reference.user_upload && self.assets.user_uploads == Permit::Deny {
            because.push(Denial::UserUploads);
            to = to.poorer(Rung::Tileset);
        }
        let before = because.len();
        for facts in &reference.filters {
            self.filter_denials(facts, &mut because);
        }
        if because.len() > before {
            to = to.poorer(Rung::Static);
        }
        if self.assets.max_rung > to {
            because.push(Denial::MaxRung(self.assets.max_rung));
            to = self.assets.max_rung;
        }
        if because.is_empty() {
            return Admission::Rich;
        }
        because.sort();
        because.dedup();
        Admission::Falls { to, because }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn f(s: &str) -> FilterRef {
        s.parse().unwrap()
    }
    fn plain(s: &str) -> FilterFacts {
        FilterFacts {
            filter: f(s),
            disguise: false,
            shader: None,
        }
    }
    fn refs(filters: Vec<FilterFacts>) -> ReferenceFacts {
        ReferenceFacts {
            user_upload: true,
            filters,
        }
    }

    #[test]
    fn user_shaders_and_disguise_are_denied_by_default_and_name_their_rung() {
        let policy = PresentationPolicy::default();
        assert_eq!(policy.admit(&refs(vec![plain("tint@1")])), Admission::Rich);

        let shader = FilterFacts {
            filter: f("fx/sparkle@1"),
            disguise: false,
            shader: Some(ShaderFormat::ShadertoyGlslFragmentV1),
        };
        assert_eq!(
            policy.admit(&refs(vec![shader])),
            Admission::Falls {
                to: Rung::Static,
                because: vec![Denial::ShadersDisabled(f("fx/sparkle@1"))]
            }
        );

        let disguise = FilterFacts {
            filter: f("shape-change@2"),
            disguise: true,
            shader: None,
        };
        assert_eq!(
            policy.admit(&refs(vec![disguise])),
            Admission::Falls {
                to: Rung::Static,
                because: vec![Denial::Disguise(f("shape-change@2"))]
            }
        );
    }

    #[test]
    fn a_world_opts_in_to_shaders_by_registered_format_only() {
        let policy = PresentationPolicy::from_json(
            r#"{"shaders":{"enabled":true,"formats":["shadertoy.glsl.fragment/v1"]}}"#,
        )
        .unwrap();
        let shader = FilterFacts {
            filter: f("fx/sparkle@1"),
            disguise: false,
            shader: Some(ShaderFormat::ShadertoyGlslFragmentV1),
        };
        assert_eq!(policy.admit(&refs(vec![shader.clone()])), Admission::Rich);

        let on_no_formats =
            PresentationPolicy::from_json(r#"{"shaders":{"enabled":true}}"#).unwrap();
        assert_eq!(
            on_no_formats.admit(&refs(vec![shader])),
            Admission::Falls {
                to: Rung::Static,
                because: vec![Denial::ShaderFormatNotEnabled(
                    f("fx/sparkle@1"),
                    ShaderFormat::ShadertoyGlslFragmentV1
                )],
            }
        );

        for bad in [
            r#"{"shaders":{"enabled":true,"formats":["shadertoy.glsl.fragment/v2"]}}"#,
            r#"{"shaders":{"enabled":true,"formats":["wgsl.compute/v1"]}}"#,
        ] {
            let err = PresentationPolicy::from_json(bad).unwrap_err();
            assert!(err.contains("not registered"), "{bad}: {err}");
        }
    }

    #[test]
    fn allow_and_deny_lists_cover_versions_names_and_families() {
        let policy = PresentationPolicy::from_json(
            r#"{"filters":{"allowed":["tint@1","blur","fx/*"],"denied":["fx/strobe"]}}"#,
        )
        .unwrap();
        for ok in [
            "tint@1",
            "blur@1",
            "blur@7",
            "fx/sparkle@3",
            "fx/glow/soft@1",
        ] {
            assert_eq!(
                policy.admit(&refs(vec![plain(ok)])),
                Admission::Rich,
                "{ok}"
            );
        }
        assert_eq!(
            policy.admit(&refs(vec![plain("tint@2")])),
            Admission::Falls {
                to: Rung::Static,
                because: vec![Denial::FilterNotAllowed(f("tint@2"))]
            }
        );
        // A family covers names under it, not a name that merely starts
        // with the same letters.
        assert_eq!(
            policy.admit(&refs(vec![plain("fxx/sparkle@1")])),
            Admission::Falls {
                to: Rung::Static,
                because: vec![Denial::FilterNotAllowed(f("fxx/sparkle@1"))]
            }
        );
        // Deny wins over allow.
        assert_eq!(
            policy.admit(&refs(vec![plain("fx/strobe@1")])),
            Admission::Falls {
                to: Rung::Static,
                because: vec![Denial::FilterDenied(f("fx/strobe@1"))]
            }
        );
    }

    #[test]
    fn upload_denial_and_max_rung_pick_the_poorer_rung_with_every_reason() {
        let policy = PresentationPolicy::from_json(
            r#"{"assets":{"user_uploads":"deny","max_rung":"static"},"filters":{"denied":["tint"]}}"#,
        )
        .unwrap();
        assert_eq!(
            policy.admit(&refs(vec![plain("tint@1"), plain("blur@1")])),
            Admission::Falls {
                to: Rung::Tileset,
                because: vec![Denial::UserUploads, Denial::FilterDenied(f("tint@1"))],
            }
        );
        let world_art = ReferenceFacts {
            user_upload: false,
            filters: vec![],
        };
        assert_eq!(
            policy.admit(&world_art),
            Admission::Falls {
                to: Rung::Static,
                because: vec![Denial::MaxRung(Rung::Static)]
            }
        );
        let glyph_world =
            PresentationPolicy::from_json(r#"{"assets":{"max_rung":"glyph"}}"#).unwrap();
        assert_eq!(
            glyph_world.admit(&world_art),
            Admission::Falls {
                to: Rung::Glyph,
                because: vec![Denial::MaxRung(Rung::Glyph)]
            }
        );
    }

    #[test]
    fn policy_round_trips_as_published_metadata() {
        let text = r#"{"assets":{"user_uploads":"allow","max_rung":"rich"},"filters":{"allowed":["tint@1","fx/*"],"denied":["fx/strobe@1"],"disguise":"deny"},"shaders":{"enabled":true,"formats":["shadertoy.glsl.fragment/v1"]},"fallback":{"world_glyphs":{"tree":"♣"},"world_substitutes":{"hero":"hero-static@1"},"tileset":"forest-tiles@3"}}"#;
        let policy = PresentationPolicy::from_json(text).unwrap();
        assert_eq!(serde_json::to_string(&policy).unwrap(), text);
        assert_eq!(
            PresentationPolicy::from_json(&serde_json::to_string(&policy).unwrap()).unwrap(),
            policy
        );

        let empty = PresentationPolicy::from_json("{}").unwrap();
        assert_eq!(empty, PresentationPolicy::default());
        assert_eq!(empty.filters.disguise, Permit::Deny);
        assert!(!empty.shaders.enabled);
    }

    #[test]
    fn malformed_policies_are_refused() {
        for bad in [
            r#"{"assets":{"max_rung":"blank"}}"#,
            r#"{"assets":{"user_uploads":"maybe"}}"#,
            r#"{"filters":{"allowed":["Tint@1"]}}"#,
            r#"{"filters":{"allowed":["/*"]}}"#,
            r#"{"filters":{"denied":["tint@0"]}}"#,
            r#"{"fallback":{"world_glyphs":{"tree":""}}}"#,
            r#"{"fallback":{"world_glyphs":{"tree":"\u0007"}}}"#,
            r#"{"fallback":{"tileset":"forest"}}"#,
            r#"{"policy":"open"}"#,
        ] {
            assert!(
                PresentationPolicy::from_json(bad).is_err(),
                "{bad} should be refused"
            );
        }
        let many: Vec<String> = (0..=MAX_POLICY_ENTRIES)
            .map(|i| format!("\"f{i}\""))
            .collect();
        let text = format!(r#"{{"filters":{{"denied":[{}]}}}}"#, many.join(","));
        assert!(
            PresentationPolicy::from_json(&text)
                .unwrap_err()
                .contains("more than")
        );
    }
}
