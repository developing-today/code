//! Tag claims over the store (Phase 5, P5.1 and P5.2).
//!
//! A tag event is applied like any content event: the resolution decides whether it applies, and
//! the store keeps it either way. Its operation is in its header (#182), so the stored event
//! carries the claim or clear marker. [`EventStore::tag_claims`] is the default view: it folds only
//! markers authorized by claim authorship or the subject's owning group's settled Admin role.
//! Every marker, including unauthorized ones, remains on the append-only event log.

use std::collections::HashMap;

use p2panda_core::Hash;
use redb::ReadableDatabase;

use super::resolve::{self, author_of};
use super::{ACTIONS, DEVICE_OPS, EVENTS, EventStore, FORKS, PROOF_HEADERS, StoreError, corrupt};
use crate::groups::{Projection, Role as GroupRole};
use id_store::chain::Event;
use id_store::tag_claims::{Applied, Claim, TagOp, compacted_claims, is_tag_kind};
use id_store::tag_index::TagIndex;

/// The events an event names: its `after` hashes, and its backlink.
fn parents_of(event: &Event) -> Vec<Hash> {
    event
        .header
        .extensions
        .after
        .iter()
        .copied()
        .chain(event.header.backlink)
        .collect()
}

impl EventStore {
    /// The default-view claims in the store. An applied `tag.clear` hides a claim only when the
    /// marker's author is that claim's author, is an Admin of the group owning the subject, or
    /// signs as a device an Admin of that group authorized at Admin scope (issue #5656).
    /// Unauthorized markers remain stored and accessible through the raw/applied event APIs.
    ///
    /// # Errors
    ///
    /// Returns a redb error if the read fails, or [`StoreError::Corrupt`] for a stored tag event
    /// that carries no tag operation for its kind.
    pub fn tag_claims(&self) -> Result<Vec<Claim>, StoreError> {
        let txn = self.db.begin_read()?;
        let (stored, forks, headers) = {
            let events = txn.open_table(EVENTS)?;
            let actions = txn.open_table(ACTIONS)?;
            let device_ops = txn.open_table(DEVICE_OPS)?;
            let forks = txn.open_table(FORKS)?;
            let proof_headers = txn.open_table(PROOF_HEADERS)?;
            resolve::load(&events, &actions, &device_ops, &forks, &proof_headers)?
        };
        let resolved = resolve::resolve(&stored, &forks, &headers)?;

        let mut parents: HashMap<Hash, Vec<Hash>> = HashMap::new();
        let mut subject_groups = HashMap::new();
        let mut applied = Vec::new();
        for item in &stored {
            let event = &item.event;
            parents.insert(event.id, parents_of(&event));
            if resolved.applied.contains(&event.id) {
                if let Some(group) = event.header.extensions.group {
                    subject_groups.insert(event.id, group);
                }
            }
            if !is_tag_kind(&event.header.extensions.kind) || !resolved.applied.contains(&event.id)
            {
                continue;
            }
            let Some(op) = event.header.extensions.tag.clone() else {
                return Err(corrupt(
                    "a stored tag event carries no tag operation".to_owned(),
                ));
            };
            applied.push(Applied {
                id: event.id,
                author: author_of(&event),
                op,
            });
        }
        // A fork proof header may be named by a clear and has parents, but never applies.
        for header in &headers {
            parents
                .entry(header.id)
                .or_insert_with(|| parents_of(header));
        }
        let default_view =
            default_view_events(&applied, &subject_groups, resolved.projection.as_ref());
        Ok(compacted_claims(&default_view, &parents))
    }

    /// The tag index over the live claims, built from [`EventStore::tag_claims`].
    ///
    /// # Errors
    ///
    /// Returns the errors of [`EventStore::tag_claims`].
    pub fn tag_index(&self) -> Result<TagIndex, StoreError> {
        Ok(TagIndex::new(&self.tag_claims()?))
    }
}

/// Keep only the claims each marker is authorized to hide in the default view. The group's
/// settled projection is intentional: role is resolved from the store's current group state, not
/// replayed at the marker's causal position. A subject without a stored, applied event with a
/// group extension has no group-admin authority; there is no inferred owner for missing or
/// ungrouped subjects. A marker signed by a device key acts with the effective role the
/// settled projection gives that device in the subject's group (the authorizing account's
/// role capped by the grant's scope), so an Admin-scoped device of an Admin account hides a
/// claim exactly as the account itself would (issue #5656).
fn default_view_events(
    applied: &[Applied],
    subject_groups: &HashMap<Hash, Hash>,
    projection: Option<&Projection>,
) -> Vec<Applied> {
    let claims: HashMap<Hash, (id_core::waiting::AuthorKey, Hash)> = applied
        .iter()
        .filter_map(|event| match &event.op {
            TagOp::Set { subject, .. } => Some((event.id, (event.author, *subject))),
            TagOp::Clear { .. } => None,
        })
        .collect();

    applied
        .iter()
        .map(|event| {
            let mut event = event.clone();
            if let TagOp::Clear { claims: named } = &mut event.op {
                let clear_author = event.author;
                named.retain(|claim_id| {
                    let Some((claim_author, subject)) = claims.get(claim_id) else {
                        return false;
                    };
                    if clear_author == *claim_author {
                        return true;
                    }
                    let Some(group) = subject_groups.get(subject) else {
                        return false;
                    };
                    let Some(projection) = projection else {
                        return false;
                    };
                    projection
                        .group(group)
                        .and_then(|view| view.members.get(&clear_author))
                        == Some(&GroupRole::Admin)
                        || projection.device_role(group, &clear_author) == Some(GroupRole::Admin)
                });
            }
            event
        })
        .collect()
}
