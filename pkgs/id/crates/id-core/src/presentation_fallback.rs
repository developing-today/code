//! Deterministic four-rung resolver for tagged-cell presentation.
//!
//! The resolver selects authored rich, static, tileset, or glyph output and
//! reports semantic tags and filters that the selected rung cannot preserve.
//! It is deliberately independent of the Screen wire codec; the tag-bearing
//! wire shape is the separately versioned Screen v2 contract.

use std::collections::{BTreeMap, BTreeSet};

use crate::presentation_asset::{AssetRef, FilterRef};
use crate::presentation_policy::{Glyph, PolicyError, Rung};

/// A rendered asset reference or printable glyph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderedArt {
    /// A trusted presenter resolves this immutable asset reference.
    Asset(AssetRef),
    /// A short printable glyph, used by the terminal tier.
    Glyph(Glyph),
}

/// One authored candidate for a rung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FallbackCandidate {
    /// What a renderer uses at this rung.
    pub art: RenderedArt,
    /// Semantic tags that this candidate still represents.
    pub retained_tags: BTreeSet<String>,
    /// Filters that this candidate still applies.
    pub retained_filters: Vec<FilterRef>,
}

impl FallbackCandidate {
    /// Build an asset candidate for the rich, static, or tileset rung.
    #[must_use]
    pub fn asset(
        reference: AssetRef,
        retained_tags: BTreeSet<String>,
        retained_filters: Vec<FilterRef>,
    ) -> Self {
        Self {
            art: RenderedArt::Asset(reference),
            retained_tags,
            retained_filters,
        }
    }

    /// Build the mandatory glyph fallback.
    pub fn glyph(
        glyph: &str,
        retained_tags: BTreeSet<String>,
        retained_filters: Vec<FilterRef>,
    ) -> Result<Self, FallbackError> {
        let glyph = Glyph::new(glyph).map_err(FallbackError::InvalidGlyph)?;
        if glyph.as_str().trim().is_empty() || glyph.as_str() == "?" {
            return Err(FallbackError::BlankOrUnknownGlyph(
                glyph.as_str().to_owned(),
            ));
        }
        Ok(Self {
            art: RenderedArt::Glyph(glyph),
            retained_tags,
            retained_filters,
        })
    }
}

/// The authored candidates for the complete fallback chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FallbackPlan {
    /// Rich asset with all supported filters.
    pub rich: Option<FallbackCandidate>,
    /// Pre-rendered static substitute.
    pub static_substitute: Option<FallbackCandidate>,
    /// World's tileset representation.
    pub tileset: Option<FallbackCandidate>,
    /// Required terminal-safe output.
    pub glyph: FallbackCandidate,
}

/// Why a fallback plan could not be resolved safely.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FallbackError {
    /// The authored glyph failed the existing short-printable-glyph check.
    InvalidGlyph(PolicyError),
    /// A glyph fallback must be visible and cannot be the unknown marker.
    BlankOrUnknownGlyph(String),
    /// A candidate's output kind does not match its rung.
    WrongOutputKind(Rung),
    /// A candidate claims a tag absent from the source cell.
    AddsUnknownTag(String),
    /// A candidate claims a filter occurrence absent from the source chain.
    AddsUnknownFilter(FilterRef),
}

impl FallbackPlan {
    /// Select the richest available rung at or below `start_at`.
    ///
    /// The tag and filter diagnostic lists contain source semantics dropped
    /// by the chosen candidate, in deterministic order. The glyph candidate
    /// makes the result total even when every richer rung is unavailable.
    pub fn resolve(
        &self,
        start_at: Rung,
        source_tags: &BTreeSet<String>,
        source_filters: &[FilterRef],
    ) -> Result<ResolvedFallback, FallbackError> {
        let candidates = [
            (Rung::Rich, self.rich.as_ref()),
            (Rung::Static, self.static_substitute.as_ref()),
            (Rung::Tileset, self.tileset.as_ref()),
            (Rung::Glyph, Some(&self.glyph)),
        ];
        for (rung, candidate) in candidates {
            if rung < start_at {
                continue;
            }
            let Some(candidate) = candidate else {
                continue;
            };
            validate_candidate(rung, candidate, source_tags, source_filters)?;
            let dropped_tags = source_tags
                .difference(&candidate.retained_tags)
                .cloned()
                .collect();
            let dropped_filters = dropped_occurrences(source_filters, &candidate.retained_filters);
            return Ok(ResolvedFallback {
                rung,
                art: candidate.art.clone(),
                dropped_tags,
                dropped_filters,
            });
        }
        // The glyph candidate is always present and is validated by its
        // constructor; reaching this branch would mean internal corruption.
        unreachable!("the required glyph candidate completes every fallback chain")
    }
}

/// A selected output and its loss diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedFallback {
    /// The rung selected by the resolver.
    pub rung: Rung,
    /// The authored output for that rung.
    pub art: RenderedArt,
    /// Tags no longer represented at this rung.
    pub dropped_tags: Vec<String>,
    /// Filter occurrences no longer applied at this rung.
    pub dropped_filters: Vec<FilterRef>,
}

fn validate_candidate(
    rung: Rung,
    candidate: &FallbackCandidate,
    source_tags: &BTreeSet<String>,
    source_filters: &[FilterRef],
) -> Result<(), FallbackError> {
    let kind_matches = matches!(
        (&candidate.art, rung),
        (
            RenderedArt::Asset(_),
            Rung::Rich | Rung::Static | Rung::Tileset
        ) | (RenderedArt::Glyph(_), Rung::Glyph)
    );
    if !kind_matches {
        return Err(FallbackError::WrongOutputKind(rung));
    }
    if let RenderedArt::Glyph(glyph) = &candidate.art {
        if glyph.as_str().trim().is_empty() || glyph.as_str() == "?" {
            return Err(FallbackError::BlankOrUnknownGlyph(
                glyph.as_str().to_owned(),
            ));
        }
    }
    if let Some(tag) = candidate.retained_tags.difference(source_tags).next() {
        return Err(FallbackError::AddsUnknownTag(tag.clone()));
    }
    let mut available: BTreeMap<&FilterRef, usize> = BTreeMap::new();
    for filter in source_filters {
        *available.entry(filter).or_default() += 1;
    }
    for filter in &candidate.retained_filters {
        let Some(count) = available.get_mut(filter) else {
            return Err(FallbackError::AddsUnknownFilter(filter.clone()));
        };
        if *count == 0 {
            return Err(FallbackError::AddsUnknownFilter(filter.clone()));
        }
        *count -= 1;
    }
    Ok(())
}

fn dropped_occurrences(source: &[FilterRef], retained: &[FilterRef]) -> Vec<FilterRef> {
    let mut counts: BTreeMap<&FilterRef, usize> = BTreeMap::new();
    for filter in retained {
        *counts.entry(filter).or_default() += 1;
    }
    let mut dropped = Vec::new();
    for filter in source {
        let count = counts.entry(filter).or_default();
        if *count == 0 {
            dropped.push(filter.clone());
        } else {
            *count -= 1;
        }
    }
    dropped
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn asset(s: &str) -> AssetRef {
        s.parse().unwrap()
    }

    fn parse_filter(s: &str) -> FilterRef {
        s.parse().unwrap()
    }

    fn art_asset(id: &str, tags: &[&str], filters: &[&str]) -> FallbackCandidate {
        FallbackCandidate::asset(
            asset(id),
            tags.iter().map(|tag| (*tag).to_owned()).collect(),
            filters.iter().map(|filter| parse_filter(filter)).collect(),
        )
    }

    fn blue_cape_plan() -> FallbackPlan {
        FallbackPlan {
            rich: Some(art_asset(
                "hero@1",
                &[
                    "sys.color.blue",
                    "world.appearance.cape",
                    "sys.effect.sparkle",
                ],
                &["fx/sparkle@1"],
            )),
            static_substitute: Some(art_asset(
                "hero-static@1",
                &[
                    "sys.color.blue",
                    "world.appearance.cape",
                    "sys.effect.sparkle",
                ],
                &[],
            )),
            tileset: Some(art_asset(
                "tiles.blue-cape@1",
                &["sys.color.blue", "world.appearance.cape"],
                &[],
            )),
            glyph: FallbackCandidate::glyph("C", BTreeSet::new(), vec![]).unwrap(),
        }
    }

    #[test]
    fn resolver_selects_richest_available_at_or_below_start() {
        let plan = blue_cape_plan();
        let tags: BTreeSet<String> = [
            "sys.color.blue",
            "world.appearance.cape",
            "sys.effect.sparkle",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        let source_filters = vec![parse_filter("fx/sparkle@1")];
        let expected = [
            (Rung::Rich, Rung::Rich),
            (Rung::Static, Rung::Static),
            (Rung::Tileset, Rung::Tileset),
            (Rung::Glyph, Rung::Glyph),
        ];
        for (start_at, expected_rung) in expected {
            let resolved = plan.resolve(start_at, &tags, &source_filters).unwrap();
            assert_eq!(resolved.rung, expected_rung);
            match start_at {
                Rung::Rich => {
                    assert!(resolved.dropped_tags.is_empty());
                    assert!(resolved.dropped_filters.is_empty());
                }
                Rung::Static => {
                    assert!(resolved.dropped_tags.is_empty());
                    assert_eq!(resolved.dropped_filters, source_filters);
                }
                Rung::Tileset => {
                    assert_eq!(resolved.dropped_tags, vec!["sys.effect.sparkle"]);
                    assert_eq!(resolved.dropped_filters, source_filters);
                }
                Rung::Glyph => {
                    assert_eq!(
                        resolved.dropped_tags,
                        tags.iter().cloned().collect::<Vec<_>>()
                    );
                    assert_eq!(resolved.dropped_filters, source_filters);
                    assert_eq!(resolved.art, RenderedArt::Glyph(Glyph::new("C").unwrap()));
                }
            }
        }
    }

    #[test]
    fn missing_rungs_step_down_and_glyph_is_never_blank_or_unknown() {
        let plan = FallbackPlan {
            rich: None,
            static_substitute: None,
            tileset: None,
            glyph: FallbackCandidate::glyph("C", BTreeSet::new(), vec![]).unwrap(),
        };
        let resolved = plan.resolve(Rung::Rich, &BTreeSet::new(), &[]).unwrap();
        assert_eq!(resolved.rung, Rung::Glyph);
        assert!(FallbackCandidate::glyph("", BTreeSet::new(), vec![]).is_err());
        assert!(FallbackCandidate::glyph("?", BTreeSet::new(), vec![]).is_err());
        let bypassed_constructor = FallbackPlan {
            rich: None,
            static_substitute: None,
            tileset: None,
            glyph: FallbackCandidate {
                art: RenderedArt::Glyph(Glyph::new("?").unwrap()),
                retained_tags: BTreeSet::new(),
                retained_filters: vec![],
            },
        };
        assert_eq!(
            bypassed_constructor.resolve(Rung::Rich, &BTreeSet::new(), &[]),
            Err(FallbackError::BlankOrUnknownGlyph("?".to_owned()))
        );
    }

    proptest::proptest! {
        #[test]
        fn resolution_is_total_and_selects_first_available_rung(
            start_idx in 0u8..4,
            available in 0u8..8,
        ) {
            let rungs = [Rung::Rich, Rung::Static, Rung::Tileset, Rung::Glyph];
            let has = |idx: usize| available & (1 << idx) != 0;
            let plan = FallbackPlan {
                rich: has(0).then(|| art_asset("rich@1", &[], &[])),
                static_substitute: has(1).then(|| art_asset("static@1", &[], &[])),
                tileset: has(2).then(|| art_asset("tiles@1", &[], &[])),
                glyph: FallbackCandidate::glyph("C", BTreeSet::new(), vec![]).unwrap(),
            };
            let start_at = rungs[usize::from(start_idx)];
            let expected = (usize::from(start_idx)..rungs.len())
                .find(|idx| *idx == 3 || has(*idx))
                .map(|idx| rungs[idx])
                .unwrap_or(Rung::Glyph);
            let expected_art = match expected {
                Rung::Rich => RenderedArt::Asset(asset("rich@1")),
                Rung::Static => RenderedArt::Asset(asset("static@1")),
                Rung::Tileset => RenderedArt::Asset(asset("tiles@1")),
                Rung::Glyph => RenderedArt::Glyph(Glyph::new("C").unwrap()),
            };
            let resolved = plan.resolve(start_at, &BTreeSet::new(), &[]).unwrap();
            proptest::prop_assert_eq!(resolved.rung, expected);
            proptest::prop_assert_eq!(resolved.art, expected_art);
        }
    }
}
