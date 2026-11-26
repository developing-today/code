//! Named filter registry: what each `name@version` filter needs and costs.
//!
//! Slice 3 of the #14731 plan; design record
//! `doc/2026-11-25T01-25-06Z_design_id5970_user_presentation_assets.md`,
//! §3 "Named composable filters".
//!
//! A filter is a named, versioned transform with a declared contract, not
//! free-form code. Each [`FilterDef`] states what it applies to, which
//! client capabilities it needs, measurable cost inputs for #6012's
//! ranker, the authored rung it falls to, who authored it, and whether it
//! is disguise-class.
//!
//! [`FilterRegistry::compose`] checks an ordered chain against a client:
//!
//! - A filter the registry does not hold, or whose capability the client
//!   lacks, does not silently no-op: the chain falls to the poorest
//!   authored rung among the failing filters and the diagnostic names each
//!   one and why.
//! - A chain longer than its tier's bound is rejected outright, so
//!   composition cannot become an unbounded per-frame cost.
//! - A shader-backed filter's capability is its #6013 registry format
//!   ([`ShaderFormat`]); the client must run that format through its
//!   trusted adapter.
//!
//! [`FilterRegistry::facts`] is the bridge to world policy (slice 2): it
//! gives [`PresentationPolicy::admit`](crate::presentation_policy::PresentationPolicy::admit)
//! the disguise and shader facts it needs.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::presentation_asset::{FilterRef, PartName};
use crate::presentation_policy::{FilterFacts, Rung, ShaderFormat};

/// Presentation tier of the client (#786), richest first. Mirrors the
/// terminal adapter's tier; this crate takes it as input and never probes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    /// Graphics and synchronized output.
    Tier1,
    /// ANSI cell grid.
    Tier2,
    /// Line mode.
    Tier3,
}

impl Tier {
    /// The longest filter chain composed for one part at this tier.
    #[must_use]
    pub const fn max_chain(self) -> usize {
        match self {
            Self::Tier1 => 8,
            Self::Tier2 => 4,
            Self::Tier3 => 0,
        }
    }
}

/// One capability a filter may need.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Capability {
    /// Raster image output.
    Raster,
    /// GPU fragment execution.
    GpuFragment,
    /// Animation over frames.
    Animation,
    /// A named blend mode.
    BlendMode(String),
    /// A client tier at least this rich.
    MinTier(Tier),
    /// A shader format the client's trusted adapter runs.
    Shader(ShaderFormat),
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Raster => f.write_str("raster"),
            Self::GpuFragment => f.write_str("gpu-fragment"),
            Self::Animation => f.write_str("animation"),
            Self::BlendMode(m) => write!(f, "blend-mode:{m}"),
            Self::MinTier(t) => write!(f, "min-tier:{t:?}"),
            Self::Shader(s) => write!(f, "shader:{s}"),
        }
    }
}

/// What a filter applies to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppliesTo {
    /// The whole asset.
    Whole,
    /// Only these declared parts.
    Parts(BTreeSet<PartName>),
}

/// Who authored a filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Authority {
    /// Built into the client.
    ClientBuiltIn,
    /// Declared in the world's policy.
    World,
    /// Shipped with the asset.
    AssetAuthor,
}

/// Measurable cost inputs for #6012's ranker. The ranker measures; these
/// only say what to measure against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CostInputs {
    /// Target area in cells or pixels, as the adapter counts it.
    pub target_area: u32,
    /// Channels read or written.
    pub channels: u8,
    /// Render passes.
    pub passes: u8,
}

/// A registered filter's contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterDef {
    /// The filter's name and version.
    pub reference: FilterRef,
    /// What it applies to.
    pub applies_to: AppliesTo,
    /// What the client must have.
    pub capabilities: BTreeSet<Capability>,
    /// Cost inputs for the ranker.
    pub cost: CostInputs,
    /// The authored rung it falls to when it cannot apply. Never `Rich`.
    pub fallback: Rung,
    /// Who authored it.
    pub authority: Authority,
    /// Whether it is disguise-class.
    pub disguise: bool,
}

impl FilterDef {
    /// The shader format this filter runs, if it is shader-backed.
    #[must_use]
    pub fn shader(&self) -> Option<ShaderFormat> {
        self.capabilities.iter().find_map(|c| match c {
            Capability::Shader(s) => Some(*s),
            _ => None,
        })
    }
}

/// Why a filter definition was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    /// The same `name@version` is already registered.
    Duplicate(FilterRef),
    /// The fallback rung is `Rich`, which is not a fallback.
    FallbackIsRich(FilterRef),
    /// The filter declares more than one shader format.
    ManyShaders(FilterRef),
    /// A part-scoped filter names no parts.
    NoParts(FilterRef),
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Duplicate(r) => write!(f, "filter {r} is already registered"),
            Self::FallbackIsRich(r) => write!(f, "filter {r} names rich as its fallback"),
            Self::ManyShaders(r) => write!(f, "filter {r} declares more than one shader format"),
            Self::NoParts(r) => write!(f, "filter {r} applies to parts but names none"),
        }
    }
}

impl std::error::Error for RegistryError {}

/// What a client can render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientCaps {
    /// The client's tier.
    pub tier: Tier,
    /// Capabilities the client has, apart from its tier.
    pub has: BTreeSet<Capability>,
}

impl ClientCaps {
    fn satisfies(&self, need: &Capability) -> bool {
        match need {
            Capability::MinTier(t) => self.tier <= *t,
            other => self.has.contains(other),
        }
    }
}

/// Why one filter in a chain cannot apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterIssue {
    /// The registry holds no such filter.
    Unknown(FilterRef),
    /// The client lacks these capabilities.
    Missing(FilterRef, Vec<Capability>),
    /// The filter cannot apply to the target it was put on.
    WrongTarget(FilterRef),
}

/// The target a chain is composed for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target<'a> {
    /// The whole asset.
    Whole,
    /// One declared part.
    Part(&'a PartName),
}

/// The outcome of composing one chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Composition<'a> {
    /// Every filter applies, in order.
    Applies(Vec<&'a FilterDef>),
    /// The chain cannot apply; it falls to `to` and `issues` says why,
    /// one entry per failing filter, in chain order.
    Falls {
        /// The poorest authored rung among the failing filters.
        to: Rung,
        /// Each failing filter and why.
        issues: Vec<FilterIssue>,
    },
    /// The chain is longer than the tier allows; it is rejected.
    TooLong {
        /// The chain's length.
        len: usize,
        /// The tier's bound.
        max: usize,
    },
}

/// The registry of named filters.
#[derive(Debug, Clone, Default)]
pub struct FilterRegistry {
    defs: BTreeMap<FilterRef, FilterDef>,
}

impl FilterRegistry {
    /// An empty registry.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            defs: BTreeMap::new(),
        }
    }

    /// Register a filter. A `name@version` is registered once and never
    /// replaced; a new contract is a new version.
    ///
    /// # Errors
    ///
    /// Refuses a duplicate `name@version`, a `Rich` fallback, more than
    /// one shader format, or a part-scoped filter naming no parts.
    pub fn register(&mut self, def: FilterDef) -> Result<(), RegistryError> {
        let r = &def.reference;
        if def.fallback == Rung::Rich {
            return Err(RegistryError::FallbackIsRich(r.clone()));
        }
        let shaders = def
            .capabilities
            .iter()
            .filter(|c| matches!(c, Capability::Shader(_)))
            .count();
        if shaders > 1 {
            return Err(RegistryError::ManyShaders(r.clone()));
        }
        if matches!(&def.applies_to, AppliesTo::Parts(p) if p.is_empty()) {
            return Err(RegistryError::NoParts(r.clone()));
        }
        if self.defs.contains_key(r) {
            return Err(RegistryError::Duplicate(r.clone()));
        }
        self.defs.insert(r.clone(), def);
        Ok(())
    }

    /// The contract for a filter, if registered.
    #[must_use]
    pub fn get(&self, reference: &FilterRef) -> Option<&FilterDef> {
        self.defs.get(reference)
    }

    /// The facts world policy needs about a registered filter.
    #[must_use]
    pub fn facts(&self, reference: &FilterRef) -> Option<FilterFacts> {
        self.get(reference).map(|d| FilterFacts {
            filter: d.reference.clone(),
            disguise: d.disguise,
            shader: d.shader(),
        })
    }

    /// Compose an ordered chain for one target on one client.
    #[must_use]
    pub fn compose<'a>(
        &'a self,
        chain: &[FilterRef],
        target: Target<'_>,
        client: &ClientCaps,
    ) -> Composition<'a> {
        let max = client.tier.max_chain();
        if chain.len() > max {
            return Composition::TooLong {
                len: chain.len(),
                max,
            };
        }
        let mut applied = Vec::with_capacity(chain.len());
        let mut issues = Vec::new();
        // An unknown filter has no authored rung; the asset author's static
        // substitute is the next authored rung after rich.
        let mut to = Rung::Rich;
        for r in chain {
            let Some(def) = self.defs.get(r) else {
                issues.push(FilterIssue::Unknown(r.clone()));
                to = to.poorer(Rung::Static);
                continue;
            };
            let fits = match (&def.applies_to, target) {
                (AppliesTo::Whole, Target::Whole) => true,
                (AppliesTo::Parts(ps), Target::Part(p)) => ps.contains(p),
                _ => false,
            };
            if !fits {
                issues.push(FilterIssue::WrongTarget(r.clone()));
                to = to.poorer(def.fallback);
                continue;
            }
            let missing: Vec<Capability> = def
                .capabilities
                .iter()
                .filter(|c| !client.satisfies(c))
                .cloned()
                .collect();
            if missing.is_empty() {
                applied.push(def);
            } else {
                issues.push(FilterIssue::Missing(r.clone(), missing));
                to = to.poorer(def.fallback);
            }
        }
        if issues.is_empty() {
            Composition::Applies(applied)
        } else {
            Composition::Falls { to, issues }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn r(s: &str) -> FilterRef {
        s.parse().unwrap()
    }
    fn part(s: &str) -> PartName {
        PartName::new(s).unwrap()
    }
    fn def(name: &str, caps: &[Capability], fallback: Rung) -> FilterDef {
        FilterDef {
            reference: r(name),
            applies_to: AppliesTo::Whole,
            capabilities: caps.iter().cloned().collect(),
            cost: CostInputs {
                target_area: 64,
                channels: 4,
                passes: 1,
            },
            fallback,
            authority: Authority::ClientBuiltIn,
            disguise: false,
        }
    }
    fn registry() -> FilterRegistry {
        let mut reg = FilterRegistry::new();
        reg.register(def("tint@1", &[Capability::Raster], Rung::Static))
            .unwrap();
        reg.register(def(
            "blur@1",
            &[Capability::Raster, Capability::MinTier(Tier::Tier1)],
            Rung::Static,
        ))
        .unwrap();
        reg.register(def(
            "fx/sparkle@1",
            &[
                Capability::GpuFragment,
                Capability::Shader(ShaderFormat::ShadertoyGlslFragmentV1),
            ],
            Rung::Tileset,
        ))
        .unwrap();
        let mut cape = def("cape-wave@1", &[Capability::Animation], Rung::Static);
        cape.applies_to = AppliesTo::Parts(BTreeSet::from([part("cape")]));
        reg.register(cape).unwrap();
        reg
    }
    fn tier1(has: &[Capability]) -> ClientCaps {
        ClientCaps {
            tier: Tier::Tier1,
            has: has.iter().cloned().collect(),
        }
    }

    #[test]
    fn a_capable_client_applies_the_chain_in_order() {
        let reg = registry();
        let client = tier1(&[Capability::Raster]);
        let Composition::Applies(defs) =
            reg.compose(&[r("blur@1"), r("tint@1")], Target::Whole, &client)
        else {
            panic!("expected applies");
        };
        let names: Vec<String> = defs.iter().map(|d| d.reference.to_string()).collect();
        assert_eq!(names, ["blur@1", "tint@1"]);
    }

    #[test]
    fn an_absent_capability_falls_to_the_authored_rung_and_says_so() {
        let reg = registry();
        let tier2 = ClientCaps {
            tier: Tier::Tier2,
            has: BTreeSet::from([Capability::Raster]),
        };
        assert_eq!(
            reg.compose(
                &[r("tint@1"), r("blur@1"), r("fx/sparkle@1")],
                Target::Whole,
                &tier2
            ),
            Composition::Falls {
                to: Rung::Tileset,
                issues: vec![
                    FilterIssue::Missing(r("blur@1"), vec![Capability::MinTier(Tier::Tier1)]),
                    FilterIssue::Missing(
                        r("fx/sparkle@1"),
                        vec![
                            Capability::GpuFragment,
                            Capability::Shader(ShaderFormat::ShadertoyGlslFragmentV1)
                        ]
                    ),
                ],
            }
        );
        // A client with the GPU but no adapter for the shader format still
        // falls: the format is the capability.
        let gpu_only = tier1(&[Capability::GpuFragment]);
        assert_eq!(
            reg.compose(&[r("fx/sparkle@1")], Target::Whole, &gpu_only),
            Composition::Falls {
                to: Rung::Tileset,
                issues: vec![FilterIssue::Missing(
                    r("fx/sparkle@1"),
                    vec![Capability::Shader(ShaderFormat::ShadertoyGlslFragmentV1)]
                )],
            }
        );
    }

    #[test]
    fn a_chain_over_the_tier_bound_is_rejected() {
        let reg = registry();
        let chain: Vec<FilterRef> =
            std::iter::repeat_n(r("tint@1"), Tier::Tier2.max_chain() + 1).collect();
        let tier2 = ClientCaps {
            tier: Tier::Tier2,
            has: BTreeSet::from([Capability::Raster]),
        };
        assert_eq!(
            reg.compose(&chain, Target::Whole, &tier2),
            Composition::TooLong { len: 5, max: 4 }
        );
        // Line mode composes nothing.
        let tier3 = ClientCaps {
            tier: Tier::Tier3,
            has: BTreeSet::new(),
        };
        assert_eq!(
            reg.compose(&[r("tint@1")], Target::Whole, &tier3),
            Composition::TooLong { len: 1, max: 0 }
        );
        assert_eq!(
            reg.compose(&[], Target::Whole, &tier3),
            Composition::Applies(vec![])
        );
    }

    #[test]
    fn unknown_and_misplaced_filters_never_silently_no_op() {
        let reg = registry();
        let client = tier1(&[Capability::Raster, Capability::Animation]);
        assert_eq!(
            reg.compose(&[r("glow@9"), r("cape-wave@1")], Target::Whole, &client),
            Composition::Falls {
                to: Rung::Static,
                issues: vec![
                    FilterIssue::Unknown(r("glow@9")),
                    FilterIssue::WrongTarget(r("cape-wave@1"))
                ],
            }
        );
        let cape = part("cape");
        assert!(matches!(
            reg.compose(&[r("cape-wave@1")], Target::Part(&cape), &client),
            Composition::Applies(_)
        ));
        let hood = part("hood");
        assert_eq!(
            reg.compose(&[r("cape-wave@1")], Target::Part(&hood), &client),
            Composition::Falls {
                to: Rung::Static,
                issues: vec![FilterIssue::WrongTarget(r("cape-wave@1"))]
            }
        );
    }

    #[test]
    fn registration_rules() {
        let mut reg = registry();
        assert_eq!(
            reg.register(def("tint@1", &[], Rung::Glyph)),
            Err(RegistryError::Duplicate(r("tint@1")))
        );
        assert!(reg.register(def("tint@2", &[], Rung::Glyph)).is_ok());
        assert_eq!(
            reg.register(def("odd@1", &[], Rung::Rich)),
            Err(RegistryError::FallbackIsRich(r("odd@1")))
        );
        let mut empty = def("parts@1", &[], Rung::Static);
        empty.applies_to = AppliesTo::Parts(BTreeSet::new());
        assert_eq!(
            reg.register(empty),
            Err(RegistryError::NoParts(r("parts@1")))
        );
    }

    #[test]
    fn facts_bridge_to_world_policy() {
        use crate::presentation_policy::{Admission, Denial, PresentationPolicy, ReferenceFacts};
        let mut reg = registry();
        let mut disguise = def("shape-change@1", &[Capability::Raster], Rung::Static);
        disguise.disguise = true;
        reg.register(disguise).unwrap();

        let facts = ReferenceFacts {
            user_upload: false,
            filters: [r("fx/sparkle@1"), r("shape-change@1")]
                .iter()
                .map(|x| reg.facts(x).unwrap())
                .collect(),
        };
        assert_eq!(
            PresentationPolicy::default().admit(&facts),
            Admission::Falls {
                to: Rung::Static,
                because: vec![
                    Denial::Disguise(r("shape-change@1")),
                    Denial::ShadersDisabled(r("fx/sparkle@1")),
                ],
            }
        );
        assert!(reg.facts(&r("missing@1")).is_none());
    }
}
