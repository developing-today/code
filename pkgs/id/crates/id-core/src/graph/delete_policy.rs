//! Server-local true-delete policy evaluation for role-aware clear markers.
//!
//! This is the policy layer above `tag.clear`: a marker remains evidence on
//! its own, and only a configured server policy can turn it into a local
//! deletion.

use std::collections::BTreeSet;

use p2panda_core::SigningKey;

use super::deletion::{DeletionError, DeletionStateMachine, TransitionOutcome};

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
    /// Identity of the verified `tag.clear` event.
    pub event_id: [u8; 32],
    /// Authority resolved from the verified, role-aware event-store view.
    /// Raw `tag.clear` events must not be mapped to this type before their
    /// author and target claim have been validated.
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
    let Some(policy) = policy else {
        return false;
    };
    policy.clear_authorities.contains(&marker.authority)
        && now.saturating_sub(marker.created_at) >= policy.grace_period_seconds
}

/// Apply a qualifying clear marker as a signed, server-local deletion.
///
/// Missing policies, unconfigured authorities, and markers inside their
/// grace period are deferred without changing state or appending evidence.
pub(crate) fn apply_clear_policy(
    state: &mut DeletionStateMachine,
    actor: &str,
    marker: &ClearMarker,
    policy: Option<&ServerDeletePolicy>,
    now: u64,
    key: &SigningKey,
) -> Result<TransitionOutcome, DeletionError> {
    let Some(policy) = policy.filter(|policy| authorizes_clear_delete(Some(policy), marker, now))
    else {
        return Ok(TransitionOutcome::Deferred);
    };

    let authority = match marker.authority {
        ClearAuthority::ClaimAuthor => "claim-author",
        ClearAuthority::GroupAdmin => "group-admin",
        ClearAuthority::ServerOwner => "server-owner",
    };
    state.apply_policy_delete_on_clear(
        actor,
        &policy.id,
        &marker.event_id,
        authority,
        marker.created_at,
        key,
    )
}

#[cfg(test)]
mod tests {
    use super::{
        ClearAuthority, ClearMarker, ServerDeletePolicy, apply_clear_policy,
        authorizes_clear_delete,
    };
    use crate::graph::deletion::{DeletionState, DeletionStateMachine, TransitionOutcome};
    use p2panda_core::SigningKey;
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
            event_id: [1; 32],
            authority: ClearAuthority::ClaimAuthor,
            created_at: 100,
        };

        assert!(authorizes_clear_delete(Some(&policy), &marker, 160));
    }

    #[test]
    fn eligible_clear_records_signed_server_policy_deletion() {
        let policy = ServerDeletePolicy {
            id: "server-policy-1".to_owned(),
            clear_authorities: BTreeSet::from([ClearAuthority::ClaimAuthor]),
            grace_period_seconds: 60,
        };
        let marker = ClearMarker {
            event_id: [2; 32],
            authority: ClearAuthority::ClaimAuthor,
            created_at: 100,
        };
        let mut state = DeletionStateMachine::new([3; 32]);
        let key = SigningKey::from_bytes(&[7; 32]);

        let outcome = apply_clear_policy(
            &mut state,
            "server-owner",
            &marker,
            Some(&policy),
            160,
            &key,
        )
        .expect("eligible policy transition succeeds");

        assert_eq!(outcome, TransitionOutcome::Applied);
        assert!(matches!(
            state.state(),
            DeletionState::TrueDeletedOnThisServer { .. }
        ));
        let evidence = state.evidence().last().expect("signed deletion evidence");
        assert_eq!(evidence.action, "policy_deleted_on_clear");
        assert!(evidence.rationale.contains("policy=server-policy-1"));
        assert!(evidence.rationale.contains("clear_marker=02020202"));
        assert!(evidence.rationale.contains("authority=claim-author"));
        evidence.verify().expect("evidence signature verifies");
    }

    #[test]
    fn unconfigured_authority_or_unelapsed_grace_does_not_delete() {
        let policy = ServerDeletePolicy {
            id: "server-policy-1".to_owned(),
            clear_authorities: BTreeSet::from([ClearAuthority::ClaimAuthor]),
            grace_period_seconds: 60,
        };
        let key = SigningKey::from_bytes(&[7; 32]);

        for (authority, created_at, now) in [
            (ClearAuthority::GroupAdmin, 100, 1_000),
            (ClearAuthority::ClaimAuthor, 100, 159),
            (ClearAuthority::ClaimAuthor, 200, 100),
        ] {
            let marker = ClearMarker {
                event_id: [3; 32],
                authority,
                created_at,
            };
            let mut state = DeletionStateMachine::new([4; 32]);
            let outcome = apply_clear_policy(
                &mut state,
                "server-owner",
                &marker,
                Some(&policy),
                now,
                &key,
            )
            .expect("nonqualifying marker is deferred");
            assert_eq!(outcome, TransitionOutcome::Deferred);
            assert_eq!(state.state(), &DeletionState::Available);
            assert!(state.evidence().is_empty());
        }
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
            event_id in any::<[u8; 32]>(),
        ) {
            let marker = ClearMarker { event_id, authority, created_at: marker_time };
            prop_assert!(!authorizes_clear_delete(None, &marker, now));
            let mut state = DeletionStateMachine::new([5; 32]);
            let key = SigningKey::from_bytes(&[7; 32]);
            let outcome = apply_clear_policy(
                &mut state,
                "server-owner",
                &marker,
                None,
                now,
                &key,
            ).expect("missing policy is deferred");
            prop_assert_eq!(outcome, TransitionOutcome::Deferred);
            prop_assert_eq!(state.state(), &DeletionState::Available);
            prop_assert!(state.evidence().is_empty());
        }
    }
}
