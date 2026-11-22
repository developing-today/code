//! Disclosure contexts (#12404): the destination-side half of the
//! #9096 classification model (design
//! `doc/2026-11-21T21-32-00Z_design_atlas_classification_model_v1.md`
//! section 4).
//!
//! A caller's ceiling (the [`Filters`](crate::Filters) grant) says
//! what the *searcher* may see. A disclosure context says what the
//! *destination surface* may receive. The two intersect: neither
//! can widen the other. Enforcement lives at the final output
//! boundary in [`crate::search`], applied to every result payload
//! regardless of caller clearance — snippets, metadata, citations,
//! cached payloads, and generated-output references all derive
//! from the one filtered candidate set.
//!
//! The context table (design section 4):
//!
//! | Context           | Ceiling         |
//! |-------------------|-----------------|
//! | `fleet-internal`  | `local-only`    |
//! | `hosted-per-user` | `personal`      |
//! | `public-surface`  | `shareable`     |
//! | `local-owner`     | `secret-bearing`|
//!
//! `excluded_personal_life` content is ineligible in *every*
//! context, including `local-owner`. `secret-bearing` content
//! under `fleet-internal` requires the dedicated secret path and
//! purpose-bound authorization, neither of which an ordinary
//! search carries — so the fleet-internal ceiling is `local-only`,
//! not `secret-bearing`.
//!
//! Fail-closed rules (design sections 2 and 4): a missing or
//! unrecognized sensitivity value resolves to `secret-bearing`;
//! an absent or unknown disclosure context on an outbound sink
//! resolves to `public-surface`.

use crate::classification::EffectiveClassification;
use crate::filters::Sensitivity;

/// The `excluded_personal_life` disposition as it appears in the
/// legacy label strings carried on search detail rows (the
/// disposition is not a sensitivity level; see the design's
/// section 2 table).
pub const EXCLUDED_PERSONAL_LIFE_LABEL: &str = "excluded_personal_life";

/// One destination surface's disclosure context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DisclosureContext {
    /// Authenticated fleet task agents, within their grant.
    FleetInternal,
    /// Hosted per-user surfaces: tenant/owner boundary + grant.
    HostedPerUser,
    /// Anything posted or transmitted to a public surface.
    PublicSurface,
    /// The owner's own local surface, via the local secret path.
    LocalOwner,
}

impl DisclosureContext {
    /// The design's spelling of this context.
    pub fn name(self) -> &'static str {
        match self {
            Self::FleetInternal => "fleet-internal",
            Self::HostedPerUser => "hosted-per-user",
            Self::PublicSurface => "public-surface",
            Self::LocalOwner => "local-owner",
        }
    }

    /// Parse the design's spelling; anything else is `None`.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "fleet-internal" => Some(Self::FleetInternal),
            "hosted-per-user" => Some(Self::HostedPerUser),
            "public-surface" => Some(Self::PublicSurface),
            "local-owner" => Some(Self::LocalOwner),
            _ => None,
        }
    }

    /// Every context, in the design table's order (for help text
    /// and exhaustive tests).
    pub fn all() -> [Self; 4] {
        [
            Self::FleetInternal,
            Self::HostedPerUser,
            Self::PublicSurface,
            Self::LocalOwner,
        ]
    }

    /// The maximum sensitivity this context may ever receive
    /// (design section 4's table).
    pub fn ceiling(self) -> Sensitivity {
        match self {
            Self::FleetInternal => Sensitivity::LocalOnly,
            Self::HostedPerUser => Sensitivity::Personal,
            Self::PublicSurface => Sensitivity::Shareable,
            Self::LocalOwner => Sensitivity::SecretBearing,
        }
    }

    /// Resolve the context for an outbound sink: an absent or
    /// unknown context fails closed to `public-surface` (design
    /// section 4). Interactive local surfaces state their context
    /// explicitly instead of relying on this default.
    pub fn for_outbound(name: Option<&str>) -> Self {
        name.and_then(Self::from_name)
            .unwrap_or(Self::PublicSurface)
    }

    /// The ceiling a result must satisfy under both the caller's
    /// grant and this context: the intersection is the *lower* of
    /// the two, so neither side can widen the other.
    pub fn effective_ceiling(self, grant: Sensitivity) -> Sensitivity {
        grant.min(self.ceiling())
    }

    /// Whether content with this effective classification may be
    /// disclosed to this context at all (before the grant is even
    /// consulted): excluded content is ineligible in every
    /// context, and nothing above the context ceiling may cross.
    pub fn permits(self, class: EffectiveClassification) -> bool {
        !class.is_excluded() && class.sensitivity <= self.ceiling()
    }

    /// Whether content with this effective classification may be
    /// served to a caller whose grant ceiling is `grant`, under
    /// this context: the full intersection of grant, context
    /// ceiling, and exclusion disposition.
    pub fn permits_with_grant(self, class: EffectiveClassification, grant: Sensitivity) -> bool {
        !class.is_excluded() && class.sensitivity <= self.effective_ceiling(grant)
    }
}

/// The effective classification of one search detail row, from
/// the legacy label string the row carries (`ChunkDetail`'s
/// chunk-or-label sensitivity). Fail-closed, per design
/// sections 2–3: a missing, empty, or unrecognized value is
/// `secret-bearing` (the locked default for unclassified
/// content), and the `excluded_personal_life` label is the
/// exclusion disposition, which is sticky and outranks every
/// sensitivity value. Where full classification records exist,
/// compose them with
/// [`EffectiveClassification::compose`] instead.
pub fn chunk_classification(eff_sensitivity: Option<&str>) -> EffectiveClassification {
    match eff_sensitivity {
        Some(EXCLUDED_PERSONAL_LIFE_LABEL) => {
            EffectiveClassification::from_parts(Sensitivity::SecretBearing, true)
        }
        Some(name) => EffectiveClassification::from_parts(
            Sensitivity::from_name(name).unwrap_or(Sensitivity::SecretBearing),
            false,
        ),
        None => EffectiveClassification::from_parts(Sensitivity::SecretBearing, false),
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{DisclosureContext, chunk_classification};
    use crate::classification::EffectiveClassification;
    use crate::filters::Sensitivity;

    const LADDER: [Sensitivity; 4] = [
        Sensitivity::Shareable,
        Sensitivity::Personal,
        Sensitivity::LocalOnly,
        Sensitivity::SecretBearing,
    ];

    #[test]
    fn context_table_matches_the_design() {
        // Oracle: design section 4's disclosure-context table.
        assert_eq!(
            DisclosureContext::FleetInternal.ceiling(),
            Sensitivity::LocalOnly
        );
        assert_eq!(
            DisclosureContext::HostedPerUser.ceiling(),
            Sensitivity::Personal
        );
        assert_eq!(
            DisclosureContext::PublicSurface.ceiling(),
            Sensitivity::Shareable
        );
        assert_eq!(
            DisclosureContext::LocalOwner.ceiling(),
            Sensitivity::SecretBearing
        );
        for ctx in DisclosureContext::all() {
            assert_eq!(DisclosureContext::from_name(ctx.name()), Some(ctx));
        }
    }

    #[test]
    fn absent_or_unknown_context_fails_closed_to_public_surface() {
        assert_eq!(
            DisclosureContext::for_outbound(None),
            DisclosureContext::PublicSurface
        );
        assert_eq!(
            DisclosureContext::for_outbound(Some("everywhere")),
            DisclosureContext::PublicSurface
        );
        assert_eq!(
            DisclosureContext::for_outbound(Some("")),
            DisclosureContext::PublicSurface
        );
        assert_eq!(
            DisclosureContext::for_outbound(Some("fleet-internal")),
            DisclosureContext::FleetInternal
        );
    }

    #[test]
    fn chunk_classification_is_fail_closed() {
        // Oracle: design decisions 2-3 — missing/invalid
        // sensitivity takes the most-locked-down value, never
        // shareable.
        for bad in [None, Some(""), Some("internal"), Some("Shareable")] {
            let class = chunk_classification(bad);
            assert_eq!(class.sensitivity, Sensitivity::SecretBearing, "{bad:?}");
            assert!(!class.is_excluded(), "{bad:?}");
        }
        let excluded = chunk_classification(Some("excluded_personal_life"));
        assert!(excluded.is_excluded());
        assert_eq!(
            chunk_classification(Some("personal")).sensitivity,
            Sensitivity::Personal
        );
    }

    #[test]
    fn excluded_content_is_ineligible_in_every_context() {
        // Oracle: design section 4 — excluded_personal_life is
        // ineligible in every context, including local-owner.
        let excluded = chunk_classification(Some("excluded_personal_life"));
        for ctx in DisclosureContext::all() {
            assert!(!ctx.permits(excluded), "{}", ctx.name());
            for grant in LADDER {
                assert!(!ctx.permits_with_grant(excluded, grant), "{}", ctx.name());
            }
        }
    }

    fn context_strategy() -> impl Strategy<Value = DisclosureContext> {
        prop_oneof![
            Just(DisclosureContext::FleetInternal),
            Just(DisclosureContext::HostedPerUser),
            Just(DisclosureContext::PublicSurface),
            Just(DisclosureContext::LocalOwner),
        ]
    }

    fn sensitivity_strategy() -> impl Strategy<Value = Sensitivity> {
        prop_oneof![
            Just(Sensitivity::Shareable),
            Just(Sensitivity::Personal),
            Just(Sensitivity::LocalOnly),
            Just(Sensitivity::SecretBearing),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        #[test]
        fn intersection_never_exceeds_either_side(
            ctx in context_strategy(),
            grant in sensitivity_strategy(),
        ) {
            let effective = ctx.effective_ceiling(grant);
            prop_assert!(effective <= grant);
            prop_assert!(effective <= ctx.ceiling());
            prop_assert_eq!(effective, grant.min(ctx.ceiling()));
        }

        #[test]
        fn permits_with_grant_is_exactly_the_intersection(
            ctx in context_strategy(),
            grant in sensitivity_strategy(),
            sensitivity in sensitivity_strategy(),
            excluded in any::<bool>(),
        ) {
            let class = EffectiveClassification::from_parts(sensitivity, excluded);
            let expected = !excluded && sensitivity <= grant.min(ctx.ceiling());
            prop_assert_eq!(ctx.permits_with_grant(class, grant), expected);
        }
    }
}
