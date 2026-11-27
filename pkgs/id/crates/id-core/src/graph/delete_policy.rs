//! Server-local true-delete policy evaluation for role-aware clear markers.
//!
//! This is the policy layer above `tag.clear`: a marker remains evidence on
//! its own, and only a configured server policy can turn it into a local
//! deletion.

use std::collections::BTreeSet;

/// Authority that signed the clear marker a server policy evaluates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ClearAuthority {
    /// The author of the claim being cleared.
    ClaimAuthor,
    /// An administrator of the claim's group.
    GroupAdmin,
    /// An administrator of the serving server.
    ServerOwner,
}

/// The policy-relevant facts for one role-aware clear marker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClearMarker {
    /// Authority that signed this clear.
    pub authority: ClearAuthority,
    /// Unix time when the marker was accepted by this server.
    pub created_at: u64,
}

/// Server-local policy that permits true deletion after an eligible clear.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerDeletePolicy {
    /// Stable identifier recorded in deletion evidence.
    pub id: String,
    /// Authorities whose clear markers this server accepts for deletion.
    pub clear_authorities: BTreeSet<ClearAuthority>,
    /// Minimum age of a marker before this policy may delete its object.
    pub grace_period_seconds: u64,
}

/// Whether a configured server policy authorizes a local delete for `marker`.
///
/// A missing policy is explicitly deny-by-default. The time comparison uses
/// saturating subtraction so a clock value before the marker cannot qualify.
#[must_use]
pub fn authorizes_clear_delete(
    policy: Option<&ServerDeletePolicy>,
    marker: &ClearMarker,
    now: u64,
) -> bool {
    // Test-first stub: this incorrectly treats any configured policy as
    // authorization, regardless of the clear authority or grace period.
    policy.is_some()
}

#[cfg(test)]
mod tests {
    use super::{ClearAuthority, ClearMarker, ServerDeletePolicy, authorizes_clear_delete};
    use std::collections::BTreeSet;

    use proptest::prelude::*;

    #[test]
    fn eligible_clear_after_grace_is_authorized() {
        let policy = ServerDeletePolicy {
            id: "server-policy-1".to_owned(),
            clear_authorities: BTreeSet::from([ClearAuthority::ClaimAuthor]),
            grace_period_seconds: 60,
        };
        let marker = ClearMarker {
            authority: ClearAuthority::ClaimAuthor,
            created_at: 100,
        };

        assert!(authorizes_clear_delete(Some(&policy), &marker, 160));
    }

    proptest! {
        // Oracle: #2091 requires that a server without a configured policy
        // never hard-deletes from a clear marker alone.
        #[test]
        fn absent_policy_never_authorizes_clear_delete(
            marker_time in any::<u64>(),
            now in any::<u64>(),
            authority in prop_oneof![
                Just(ClearAuthority::ClaimAuthor),
                Just(ClearAuthority::GroupAdmin),
                Just(ClearAuthority::ServerOwner),
            ],
        ) {
            let marker = ClearMarker { authority, created_at: marker_time };
            prop_assert!(!authorizes_clear_delete(None, &marker, now));
        }
    }
}
