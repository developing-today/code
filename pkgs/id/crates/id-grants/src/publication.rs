//! Publish and unpublish for public shares (#5802 decision record
//! `doc/2026-11-24T09-39-40Z_decision_id5802_public_shares.md`, D1,
//! D2.3, D5; slice #14781).
//!
//! A public share is an ordinary grant whose subject is
//! [`Subject::Public`], created by the owner's publish action together
//! with a [`PublicationRecord`] and a [`ShareManifest`]. This module
//! holds the three together:
//!
//! - [`Publications::publish`] checks everything first, then creates
//!   the public grant and the record in one step, so either both exist
//!   or neither does. The manifest is derived from the live record, so
//!   it exists exactly while the record is live.
//! - [`Publications::unpublish`] revokes the grant and retires the
//!   record in one step. The manifest stops being served with it, and
//!   the retired record stays on the owner's private audit surface
//!   ([`Publications::audit`]).
//! - [`Publications::resolve`] answers a share identifier for anyone.
//!   An unpublished identifier and one that never existed give the same
//!   `None`, and an identifier is never minted twice, so a dead link
//!   stays dead.
//!
//! Share identifiers are minted from caller-supplied entropy
//! ([`ShareId::mint`]); the caller must draw it from a cryptographic
//! random source, which keeps this crate free of I/O. Signing the
//! manifest belongs to the #5801 manifest work and is not done here.
//! Classification levels are opaque ordered values
//! ([`ClassLevel`]); the classification design line owns their
//! meaning.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{
    Delegation, Duration, Grant, GrantError, GrantId, GrantSet, Operation, Scope, Subject,
};

/// The publish-time copy for the forward-only boundary (D5). Show it
/// before the owner confirms a publish.
pub const UNPUBLISH_NOTICE: &str =
    "Unpublishing stops new access through ID. It cannot take back copies anyone already made.";

/// The honest label for an unlisted share (D2): unlisted controls
/// discovery, not access.
pub const UNLISTED_LABEL: &str = "anyone with the link";

/// An unguessable public share identifier, not derived from any object
/// or collection id (D1).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ShareId(String);

impl ShareId {
    /// Mints an identifier from 32 bytes of entropy. The caller must
    /// draw them from a cryptographic random source.
    #[must_use]
    pub fn mint(entropy: [u8; 32]) -> Self {
        use fmt::Write as _;
        let mut out = String::with_capacity(64);
        for byte in entropy {
            let _ = write!(out, "{byte:02x}");
        }
        Self(out)
    }

    /// The identifier as text, for links.
    #[must_use]
    pub const fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for ShareId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// How a share can be found (D2). Unlisted is the default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryMode {
    /// Fetchable by anyone holding the link; listed nowhere.
    #[default]
    Unlisted,
    /// Also appears in the owner's public listing.
    Listed,
}

/// A sensitivity classification level. Higher is more sensitive; an
/// object may be published only at or below a share's ceiling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ClassLevel(pub u8);

/// What the owner asks to publish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishRequest {
    /// The granted scope, each unit with its classification level.
    pub scope: BTreeMap<Scope, ClassLevel>,
    /// Whether readers may also subscribe to updates. `read` is always
    /// granted; `write` and `append` never are.
    pub subscribe: bool,
    /// How long the share runs.
    pub duration: Duration,
    /// How the share can be found.
    pub discovery: DiscoveryMode,
    /// The classification ceiling the share is issued under.
    pub ceiling: ClassLevel,
    /// The owner's own public-release ceiling; `ceiling` may not exceed
    /// it.
    pub owner_ceiling: ClassLevel,
    /// The owner-supplied title.
    pub title: String,
    /// The owner-supplied description.
    pub description: String,
    /// The publish time (Unix seconds, UTC).
    pub now: u64,
}

/// Why a publish was refused. These are owner-facing; nothing here is
/// shown on a public surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublishError {
    /// The scope is empty.
    EmptyScope,
    /// The share's ceiling is above the owner's public-release ceiling.
    CeilingAboveOwner,
    /// This unit is classified above the share's ceiling.
    AboveCeiling(Scope),
    /// The identifier has been minted before, live or retired.
    IdentifierUsed,
    /// The public grant failed the grants model's checks.
    Grant(GrantError),
}

impl fmt::Display for PublishError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyScope => {
                f.write_str("a share must publish at least one object or collection")
            }
            Self::CeilingAboveOwner => {
                f.write_str("the share's ceiling is above your public-release ceiling")
            }
            Self::AboveCeiling(_) => {
                f.write_str("an item is classified above this share's ceiling")
            }
            Self::IdentifierUsed => f.write_str("this share identifier was already used"),
            Self::Grant(err) => write!(f, "public grant refused: {err}"),
        }
    }
}

impl std::error::Error for PublishError {}

/// The owner-side record of one publication (D1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicationRecord {
    /// The share identifier.
    pub share: ShareId,
    /// The public grant this publication created.
    pub grant: GrantId,
    /// The granted scope.
    pub scope: BTreeSet<Scope>,
    /// The granted operations.
    pub operations: BTreeSet<Operation>,
    /// How long the share runs.
    pub duration: Duration,
    /// How the share can be found.
    pub discovery: DiscoveryMode,
    /// The classification ceiling the share was issued under.
    pub ceiling: ClassLevel,
    /// The owner-supplied title.
    pub title: String,
    /// The owner-supplied description.
    pub description: String,
    /// When it was published (Unix seconds, UTC).
    pub published_at: u64,
    /// When it was unpublished, once it has been.
    pub unpublished_at: Option<u64>,
}

impl PublicationRecord {
    /// Whether an object at `level` may be emitted under this share,
    /// at publish time or at each update. An object above the ceiling
    /// is simply never emitted, and its absence is not signalled.
    #[must_use]
    pub const fn may_emit(&self, level: ClassLevel) -> bool {
        level.0 <= self.ceiling.0
    }
}

/// The public share manifest (#5801 D4 fields minus key material, with
/// the publication record's public fields, D2.3). Unsigned here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShareManifest {
    /// The share identifier.
    pub share: ShareId,
    /// The owner, who issued the grant.
    pub owner: String,
    /// The grant lineage: the public grant, which is always a root.
    pub grant: GrantId,
    /// The published set, enumerated.
    pub scope: BTreeSet<Scope>,
    /// The granted operations.
    pub operations: BTreeSet<Operation>,
    /// The manifest epoch.
    pub epoch: u64,
    /// The share's duration.
    pub duration: Duration,
    /// The classification ceiling.
    pub ceiling: ClassLevel,
    /// The title.
    pub title: String,
    /// The description.
    pub description: String,
    /// How the share can be found.
    pub discovery: DiscoveryMode,
    /// When it was published (Unix seconds, UTC).
    pub published_at: u64,
}

/// The publication grant id for a share. Prefixed so it cannot collide
/// with an id the owner chose for an ordinary grant.
fn grant_id_for(share: &ShareId) -> GrantId {
    GrantId(format!("public-share:{share}"))
}

/// One owner's publications: live and retired records, and every
/// identifier ever used.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Publications {
    owner: String,
    live: BTreeMap<ShareId, PublicationRecord>,
    retired: BTreeMap<ShareId, PublicationRecord>,
    used: BTreeSet<ShareId>,
}

impl Publications {
    /// An owner's empty publication book.
    #[must_use]
    pub fn new(owner: impl Into<String>) -> Self {
        Self {
            owner: owner.into(),
            live: BTreeMap::new(),
            retired: BTreeMap::new(),
            used: BTreeSet::new(),
        }
    }

    /// Publishes: creates the public grant in `grants`, issued by this
    /// book's owner, and the record together, under `share`. Nothing
    /// changes when it fails.
    ///
    /// # Errors
    ///
    /// A [`PublishError`] naming the first check that failed.
    pub fn publish(
        &mut self,
        grants: &mut GrantSet,
        share: ShareId,
        request: PublishRequest,
    ) -> Result<&PublicationRecord, PublishError> {
        if request.scope.is_empty() {
            return Err(PublishError::EmptyScope);
        }
        if request.ceiling > request.owner_ceiling {
            return Err(PublishError::CeilingAboveOwner);
        }
        if let Some((unit, _)) = request
            .scope
            .iter()
            .find(|(_, level)| **level > request.ceiling)
        {
            return Err(PublishError::AboveCeiling(unit.clone()));
        }
        if self.used.contains(&share) {
            return Err(PublishError::IdentifierUsed);
        }
        let mut operations = BTreeSet::from([Operation::Read]);
        if request.subscribe {
            operations.insert(Operation::Subscribe);
        }
        let scope: BTreeSet<Scope> = request.scope.into_keys().collect();
        let grant = Grant {
            id: grant_id_for(&share),
            issuer: self.owner.clone(),
            subject: Subject::Public,
            scopes: scope.clone(),
            operations: operations.clone(),
            duration: request.duration,
            delegation: Delegation::default(),
            parent: None,
        };
        let grant_id = grant.id.clone();
        // The one fallible step that changes anything; it changes
        // nothing when it fails, so the record is only added after it.
        grants.insert_root(grant).map_err(PublishError::Grant)?;
        let record = PublicationRecord {
            share: share.clone(),
            grant: grant_id,
            scope,
            operations,
            duration: request.duration,
            discovery: request.discovery,
            ceiling: request.ceiling,
            title: request.title,
            description: request.description,
            published_at: request.now,
            unpublished_at: None,
        };
        self.used.insert(share.clone());
        Ok(self.live.entry(share).or_insert(record))
    }

    /// Unpublishes: revokes the public grant and retires the record
    /// together. Returns whether `share` was live.
    pub fn unpublish(&mut self, grants: &mut GrantSet, share: &ShareId, now: u64) -> bool {
        let Some(mut record) = self.live.remove(share) else {
            return false;
        };
        grants.revoke(&record.grant);
        record.unpublished_at = Some(now);
        self.retired.insert(share.clone(), record);
        true
    }

    /// Resolves a share identifier for anyone: the manifest while the
    /// share is live, else `None`, the same for an unpublished
    /// identifier as for one that never existed.
    #[must_use]
    pub fn resolve(&self, share: &ShareId, grants: &GrantSet, now: u64) -> Option<ShareManifest> {
        let record = self.live.get(share)?;
        let grant = grants.get(&record.grant)?;
        if grant.is_expired(now) {
            return None;
        }
        Some(ShareManifest {
            share: record.share.clone(),
            owner: self.owner.clone(),
            grant: record.grant.clone(),
            scope: record.scope.clone(),
            operations: record.operations.clone(),
            epoch: 0,
            duration: record.duration,
            ceiling: record.ceiling,
            title: record.title.clone(),
            description: record.description.clone(),
            discovery: record.discovery,
            published_at: record.published_at,
        })
    }

    /// The owner's listing: manifests of live, listed shares only.
    #[must_use]
    pub fn listing(&self, grants: &GrantSet, now: u64) -> Vec<ShareManifest> {
        self.live
            .values()
            .filter(|record| record.discovery == DiscoveryMode::Listed)
            .filter_map(|record| self.resolve(&record.share, grants, now))
            .collect()
    }

    /// The owner's private audit surface: every record, live or
    /// retired. Never serve this publicly.
    pub fn audit(&self) -> impl Iterator<Item = &PublicationRecord> {
        self.live.values().chain(self.retired.values())
    }

    /// Drops a retired record from the audit surface, at the owner's
    /// choice. The identifier stays used, so it is never minted again.
    /// Returns whether a retired record was dropped.
    pub fn purge(&mut self, share: &ShareId) -> bool {
        self.retired.remove(share).is_some()
    }
}
