//! Runtime feature-object library attachment (`#12640`): the registry
//! core implementing the semantics the `#6003` design note states and
//! the 2026-11-22 decision record on `#6003` adopts for the worlds
//! program (`#5955`).
//!
//! The design note's three semantics, as this module enforces them:
//!
//! 1. **Tick-boundary attach/detach.** Attach and detach are staged
//!    when requested and take effect only at a tick boundary
//!    ([`AttachmentRegistry::begin_tick`]). The attachment set a tick's
//!    invocations resolve against is the [`TickSnapshot`] the boundary
//!    hands out: a detach staged mid-tick never pulls behaviour out
//!    from under an invocation that started under an earlier snapshot.
//!    Who may attach or detach: the entity's owner, or a world author
//!    holding an explicit attach capability on the entity's class —
//!    nobody else. Attach is atomic: every check runs before anything
//!    is staged, so a refused attach leaves no partial state visible.
//! 2. **Intersection-only capability composition, checked at attach
//!    time.** A library may exercise only capabilities the entity
//!    already holds *and* the world's ceiling permits. An attach whose
//!    requirement footprint reaches outside that intersection is
//!    refused, with the widening capabilities named exhaustively:
//!    borrowing a library never borrows its author's privileges.
//! 3. **Namespaced library state.** State is keyed by (entity,
//!    library, major version). On detach it is kept but inert — not
//!    deleted — so a reattach of the same major version revives it;
//!    the entity owner may explicitly drop it. Version change: a
//!    minor upgrade runs the library's declared migration (modelled
//!    here as the state carrying over live) or, when none is
//!    declared, keeps the state inert and flagged on the entity; a
//!    major upgrade parks the old state until the entity owner
//!    adopts it into the new major version or drops it. No silent
//!    migration, no silent drop.
//!
//! Scope of this step (`#12640`, first slice): the registry core as a
//! pure state machine. Runtime wiring — dispatching invocations
//! through the snapshot, executing declared migrations, persisting
//! the registry — consumes this seam later, the same shape as
//! [`crate::library_manifest`]'s parser seam for the registry/upload
//! path. Declared migrations are modelled by their contract (state
//! carried live across a minor upgrade) rather than executed here.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::library_manifest::LibraryManifest;

/// A library version, parsed fail-closed from the author-assigned
/// version string a [`LibraryManifest`] carries. The attachment
/// semantics distinguish major changes (a new state namespace, with
/// the old state parked) from minor changes (migration or inert), so
/// the registry works with the parsed form only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct LibraryVersion {
    /// The major version: the state-namespace component.
    pub major: u64,
    /// The minor version: the migration component.
    pub minor: u64,
    /// The patch version: no state semantics of its own.
    pub patch: u64,
}

/// Why a version string could not be parsed: it is not
/// `MAJOR[.MINOR[.PATCH]]` with numeric components.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionParseError {
    /// The offending version string, verbatim.
    pub text: String,
}

impl fmt::Display for VersionParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "library version {:?} is not MAJOR[.MINOR[.PATCH]] with numeric components",
            self.text
        )
    }
}

impl std::error::Error for VersionParseError {}

impl LibraryVersion {
    /// Parse `MAJOR[.MINOR[.PATCH]]`; every component must be a
    /// non-empty run of ASCII digits. Anything else is refused — a
    /// version the registry cannot order is a version it cannot
    /// attach.
    ///
    /// # Errors
    ///
    /// [`VersionParseError`] naming the offending string.
    pub fn parse(text: &str) -> Result<Self, VersionParseError> {
        let bad = || VersionParseError {
            text: text.to_string(),
        };
        let mut parts = text.split('.');
        let component = |parts: &mut std::str::Split<'_, char>| -> Result<u64, VersionParseError> {
            match parts.next() {
                None => Ok(0),
                Some(part) if !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()) => {
                    part.parse::<u64>().map_err(|_| bad())
                }
                Some(_) => Err(bad()),
            }
        };
        let version = Self {
            major: component(&mut parts)?,
            minor: component(&mut parts)?,
            patch: component(&mut parts)?,
        };
        if parts.next().is_some() {
            return Err(bad());
        }
        Ok(version)
    }
}

impl fmt::Display for LibraryVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// One library offered for attachment: its identity, version, and the
/// platform-requirement footprint the attach-time check runs against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LibraryOffer {
    /// The library identity (`#5969` §2: identity is the content
    /// hash, not the name).
    pub library_id: String,
    /// The offered version.
    pub version: LibraryVersion,
    /// Catalog names the library requires — its declared platform
    /// requirements (`#5956` §1). This set, and only this set, is
    /// what the attach-time intersection check judges: whatever else
    /// the library's author holds is not part of the offer.
    pub requirements: BTreeSet<String>,
    /// Whether the library declares a migration for minor upgrades
    /// (`#6003` design note §3). The registry models the migration's
    /// contract — state carried live — and does not execute it.
    pub declares_migration: bool,
}

impl LibraryOffer {
    /// Build an offer from a parsed [`LibraryManifest`]: identity is
    /// the manifest hash, the footprint is its declared requirement
    /// names, the version is parsed fail-closed, and the migration
    /// flag is derived from the manifest's `migrate` declarations —
    /// a version declares a migration by naming a source version, so
    /// the flag is manifest data, never a caller assertion. A
    /// `migrate from` that is not a parseable version fails the
    /// build: the registry must never admit an offer whose migration
    /// source it cannot name.
    ///
    /// # Errors
    ///
    /// [`VersionParseError`] when the manifest's version string — or
    /// one of its migration sources — is not parseable.
    pub fn from_manifest(manifest: &LibraryManifest) -> Result<Self, VersionParseError> {
        for migration in &manifest.migrations {
            LibraryVersion::parse(&migration.from)?;
        }
        Ok(Self {
            library_id: manifest.hash.clone(),
            version: LibraryVersion::parse(&manifest.version)?,
            requirements: manifest
                .requirements
                .iter()
                .map(|requirement| requirement.name.clone())
                .collect(),
            declares_migration: !manifest.migrations.is_empty(),
        })
    }
}

/// An entity a library can attach to, with the capabilities it holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntityRecord {
    /// The entity identity.
    pub entity_id: String,
    /// The entity class: attach capabilities are granted per class.
    pub class: String,
    /// The entity owner's principal name.
    pub owner: String,
    /// The capabilities the entity already holds. Attachment can
    /// never widen this set (`#6003` design note §2).
    pub held_caps: BTreeSet<String>,
}

/// One live attachment: the version in force and the requirement
/// footprint it was admitted under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveAttachment {
    /// The attached version.
    pub version: LibraryVersion,
    /// The admitted requirement footprint (a subset of the entity's
    /// held capabilities intersected with the world ceiling).
    pub requirements: BTreeSet<String>,
}

/// Why an attach was refused. Every check runs before anything is
/// staged, so a refusal leaves the registry untouched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttachRefusal {
    /// No entity with that identity is registered.
    UnknownEntity,
    /// The actor is neither the entity's owner nor an author holding
    /// an attach capability on the entity's class.
    NotAuthorized {
        /// The refused actor.
        actor: String,
    },
    /// The library's footprint reaches outside (entity held ∩ world
    /// ceiling). The widening capabilities are named exhaustively —
    /// every one, never just the first.
    CapabilityWidening {
        /// Every requirement outside the intersection, by name.
        widening: BTreeSet<String>,
    },
    /// The same library is already attached at the same version.
    AlreadyAttached {
        /// The version already in force.
        version: LibraryVersion,
    },
}

impl fmt::Display for AttachRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownEntity => write!(f, "no such entity is registered"),
            Self::NotAuthorized { actor } => write!(
                f,
                "{actor} is neither the entity owner nor an attach-capable author for its class"
            ),
            Self::CapabilityWidening { widening } => write!(
                f,
                "attach would widen the entity beyond its held capabilities and the world \
                 ceiling: {}",
                widening
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::AlreadyAttached { version } => {
                write!(f, "already attached at version {version}")
            }
        }
    }
}

impl std::error::Error for AttachRefusal {}

/// Why a detach was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DetachRefusal {
    /// No entity with that identity is registered.
    UnknownEntity,
    /// The actor is neither the entity's owner nor an attach-capable
    /// author for the entity's class.
    NotAuthorized {
        /// The refused actor.
        actor: String,
    },
    /// The library is not attached to the entity (counting staged
    /// changes, so a detach of a not-yet-applied attach is honoured).
    NotAttached,
}

impl fmt::Display for DetachRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownEntity => write!(f, "no such entity is registered"),
            Self::NotAuthorized { actor } => write!(
                f,
                "{actor} is neither the entity owner nor an attach-capable author for its class"
            ),
            Self::NotAttached => write!(f, "the library is not attached to the entity"),
        }
    }
}

impl std::error::Error for DetachRefusal {}

/// Why an owner state decision (adopt / drop) was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StateDecisionRefusal {
    /// No entity with that identity is registered.
    UnknownEntity,
    /// Only the entity owner may adopt or drop library state.
    NotOwner {
        /// The refused actor.
        actor: String,
    },
    /// Adopt was asked for but the library is not attached at a
    /// different major version to adopt the parked state into.
    NotAttached,
    /// No parked state exists at the named major version.
    NoParkedState,
    /// The attached major version already has live state of its own;
    /// adopting would silently overwrite it, so it is refused.
    LiveStatePresent,
}

impl fmt::Display for StateDecisionRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownEntity => write!(f, "no such entity is registered"),
            Self::NotOwner { actor } => {
                write!(f, "{actor} is not the entity owner")
            }
            Self::NotAttached => {
                write!(f, "the library is not attached at another major version")
            }
            Self::NoParkedState => write!(f, "no parked state at that major version"),
            Self::LiveStatePresent => {
                write!(
                    f,
                    "the attached version already has live state; adoption refused"
                )
            }
        }
    }
}

impl std::error::Error for StateDecisionRefusal {}

/// Why library state could not be written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StateWriteRefusal {
    /// The library is not attached to the entity in the current
    /// tick's attachment set.
    NotAttached,
}

impl fmt::Display for StateWriteRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAttached => {
                write!(f, "the library is not attached to the entity")
            }
        }
    }
}

impl std::error::Error for StateWriteRefusal {}

/// Why a generation of library state is inert.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InertReason {
    /// The library detached; a reattach of the same major version
    /// revives this state.
    Detached,
    /// A minor upgrade arrived with no declared migration: the state
    /// is kept inert and flagged on the entity (`#6003` §3).
    MinorUpgradeNoMigration,
    /// A major upgrade parked this state until the entity owner
    /// adopts it into the new major version or drops it.
    MajorParked,
}

/// One inert generation of library state: kept, never silently
/// dropped, and never readable through the live state path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InertEntry {
    /// The state bytes, verbatim.
    pub bytes: Vec<u8>,
    /// Why this generation went inert.
    pub reason: InertReason,
}

impl InertEntry {
    /// Whether this generation is flagged on the entity — the
    /// minor-upgrade-without-migration case the design note calls
    /// out for an explicit flag.
    #[must_use]
    pub fn flagged(&self) -> bool {
        self.reason == InertReason::MinorUpgradeNoMigration
    }
}

/// The attachment set of one tick, captured at its boundary.
/// Invocations resolve against a snapshot, so an invocation that
/// started under this snapshot finishes under it even when a detach
/// lands at the next boundary (`#6003` design note §1).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TickSnapshot {
    attachments: BTreeMap<AttachKey, ActiveAttachment>,
}

impl TickSnapshot {
    /// The attachment an invocation of `library_id` on `entity_id`
    /// resolves against in this tick, if any.
    #[must_use]
    pub fn resolve(&self, entity_id: &str, library_id: &str) -> Option<&ActiveAttachment> {
        self.attachments.get(&AttachKey {
            entity: entity_id.to_string(),
            library_id: library_id.to_string(),
        })
    }

    /// Whether the library is attached to the entity in this tick.
    #[must_use]
    pub fn is_attached(&self, entity_id: &str, library_id: &str) -> bool {
        self.resolve(entity_id, library_id).is_some()
    }
}

/// (entity, library): at most one version of a library is attached
/// to an entity at a time.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct AttachKey {
    entity: String,
    library_id: String,
}

/// (entity, library, major version): the state namespace.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct StateKey {
    entity: String,
    library_id: String,
    major: u64,
}

/// Live state: readable and writable by the attached version.
#[derive(Clone, Debug, PartialEq, Eq)]
struct LiveState {
    /// The minor version that most recently wrote or carried the
    /// state.
    minor: u64,
    bytes: Vec<u8>,
}

/// One staged change, applied at the next tick boundary in the order
/// it was staged.
#[derive(Clone, Debug, PartialEq, Eq)]
enum StagedChange {
    Attach { entity: String, offer: LibraryOffer },
    Detach { entity: String, library_id: String },
}

/// The attachment registry of one world: entities, the live
/// attachment set, staged changes awaiting the next tick boundary,
/// and the namespaced library state.
#[derive(Clone, Debug)]
pub struct AttachmentRegistry {
    /// The world ceiling: the capabilities the world permits at all.
    ceiling: BTreeSet<String>,
    /// Attach-capable authors, by entity class.
    attach_capable: BTreeMap<String, BTreeSet<String>>,
    entities: BTreeMap<String, EntityRecord>,
    active: BTreeMap<AttachKey, ActiveAttachment>,
    staged: Vec<StagedChange>,
    live_state: BTreeMap<StateKey, LiveState>,
    inert_state: BTreeMap<StateKey, Vec<InertEntry>>,
}

impl AttachmentRegistry {
    /// A registry for a world with the given capability ceiling and
    /// per-class attach-capable authors.
    #[must_use]
    pub fn new(
        ceiling: BTreeSet<String>,
        attach_capable: BTreeMap<String, BTreeSet<String>>,
    ) -> Self {
        Self {
            ceiling,
            attach_capable,
            entities: BTreeMap::new(),
            active: BTreeMap::new(),
            staged: Vec::new(),
            live_state: BTreeMap::new(),
            inert_state: BTreeMap::new(),
        }
    }

    /// Register (or re-register) an entity. Re-registration replaces
    /// the record; attachments and state are keyed by identity and
    /// are not touched.
    pub fn register_entity(&mut self, entity: EntityRecord) {
        self.entities.insert(entity.entity_id.clone(), entity);
    }

    /// Whether `actor` may attach/detach for `entity`: the owner, or
    /// an author holding an attach capability on the entity's class.
    fn may_attach(&self, actor: &str, entity: &EntityRecord) -> bool {
        actor == entity.owner
            || self
                .attach_capable
                .get(&entity.class)
                .is_some_and(|authors| authors.contains(actor))
    }

    /// The attachment in force for (entity, library) once the staged
    /// changes are applied — the projection attach/detach checks run
    /// against, so a detach staged earlier in the tick is honoured.
    fn projected(&self, entity_id: &str, library_id: &str) -> Option<ActiveAttachment> {
        let key = AttachKey {
            entity: entity_id.to_string(),
            library_id: library_id.to_string(),
        };
        let mut current = self.active.get(&key).cloned();
        for change in &self.staged {
            match change {
                StagedChange::Attach { entity, offer }
                    if entity == entity_id && offer.library_id == library_id =>
                {
                    current = Some(ActiveAttachment {
                        version: offer.version,
                        requirements: offer.requirements.clone(),
                    });
                }
                StagedChange::Detach {
                    entity,
                    library_id: staged_library,
                } if entity == entity_id && staged_library == library_id => {
                    current = None;
                }
                _ => {}
            }
        }
        current
    }

    /// Stage an attach of `offer` to the entity, to take effect at
    /// the next tick boundary. All checks run now, at attach time:
    /// authorization, then the intersection check — the library's
    /// footprint must fit inside (entity held ∩ world ceiling), and a
    /// widening attach is refused with the widening capabilities
    /// named exhaustively. A refused attach stages nothing.
    ///
    /// # Errors
    ///
    /// [`AttachRefusal`] naming the failed check.
    pub fn attach(
        &mut self,
        actor: &str,
        entity_id: &str,
        offer: LibraryOffer,
    ) -> Result<(), AttachRefusal> {
        let entity = self
            .entities
            .get(entity_id)
            .ok_or(AttachRefusal::UnknownEntity)?;
        if !self.may_attach(actor, entity) {
            return Err(AttachRefusal::NotAuthorized {
                actor: actor.to_string(),
            });
        }
        if let Some(current) = self.projected(entity_id, &offer.library_id) {
            if current.version == offer.version {
                return Err(AttachRefusal::AlreadyAttached {
                    version: offer.version,
                });
            }
        }
        let permitted: BTreeSet<String> = entity
            .held_caps
            .intersection(&self.ceiling)
            .cloned()
            .collect();
        let widening: BTreeSet<String> =
            offer.requirements.difference(&permitted).cloned().collect();
        if !widening.is_empty() {
            return Err(AttachRefusal::CapabilityWidening { widening });
        }
        self.staged.push(StagedChange::Attach {
            entity: entity_id.to_string(),
            offer,
        });
        Ok(())
    }

    /// Stage a detach of the library from the entity, to take effect
    /// at the next tick boundary. The library's state goes inert at
    /// the boundary — kept, not deleted.
    ///
    /// # Errors
    ///
    /// [`DetachRefusal`] naming the failed check.
    pub fn detach(
        &mut self,
        actor: &str,
        entity_id: &str,
        library_id: &str,
    ) -> Result<(), DetachRefusal> {
        let entity = self
            .entities
            .get(entity_id)
            .ok_or(DetachRefusal::UnknownEntity)?;
        if !self.may_attach(actor, entity) {
            return Err(DetachRefusal::NotAuthorized {
                actor: actor.to_string(),
            });
        }
        if self.projected(entity_id, library_id).is_none() {
            return Err(DetachRefusal::NotAttached);
        }
        self.staged.push(StagedChange::Detach {
            entity: entity_id.to_string(),
            library_id: library_id.to_string(),
        });
        Ok(())
    }

    /// Close the current tick and open the next: apply every staged
    /// change in the order it was staged, run the state transitions
    /// those changes imply, and return the new tick's snapshot.
    /// Snapshots handed out earlier are unaffected — invocations that
    /// started under them finish under them.
    pub fn begin_tick(&mut self) -> TickSnapshot {
        let staged = std::mem::take(&mut self.staged);
        for change in staged {
            match change {
                StagedChange::Attach { entity, offer } => {
                    self.apply_attach(&entity, &offer);
                }
                StagedChange::Detach { entity, library_id } => {
                    self.apply_detach(&entity, &library_id);
                }
            }
        }
        TickSnapshot {
            attachments: self.active.clone(),
        }
    }

    /// The attachment currently in force (post-boundary), if any.
    #[must_use]
    pub fn attachment(&self, entity_id: &str, library_id: &str) -> Option<&ActiveAttachment> {
        self.active.get(&AttachKey {
            entity: entity_id.to_string(),
            library_id: library_id.to_string(),
        })
    }

    fn apply_attach(&mut self, entity_id: &str, offer: &LibraryOffer) {
        let key = AttachKey {
            entity: entity_id.to_string(),
            library_id: offer.library_id.clone(),
        };
        match self.active.remove(&key) {
            Some(current) if current.version != offer.version => {
                self.transition_state_on_version_change(entity_id, offer, current.version);
            }
            Some(_) => {}
            None => {
                // A fresh attach revives the most recent detached
                // generation of the same major version, if one is
                // kept inert.
                let state_key = StateKey {
                    entity: entity_id.to_string(),
                    library_id: offer.library_id.clone(),
                    major: offer.version.major,
                };
                if let Some(entries) = self.inert_state.get_mut(&state_key) {
                    if let Some(position) = entries
                        .iter()
                        .rposition(|entry| entry.reason == InertReason::Detached)
                    {
                        let entry = entries.remove(position);
                        self.live_state.insert(
                            state_key,
                            LiveState {
                                minor: offer.version.minor,
                                bytes: entry.bytes,
                            },
                        );
                    }
                }
            }
        }
        self.active.insert(
            key,
            ActiveAttachment {
                version: offer.version,
                requirements: offer.requirements.clone(),
            },
        );
    }

    fn apply_detach(&mut self, entity_id: &str, library_id: &str) {
        let key = AttachKey {
            entity: entity_id.to_string(),
            library_id: library_id.to_string(),
        };
        if let Some(current) = self.active.remove(&key) {
            let state_key = StateKey {
                entity: entity_id.to_string(),
                library_id: library_id.to_string(),
                major: current.version.major,
            };
            if let Some(live) = self.live_state.remove(&state_key) {
                self.inert_state
                    .entry(state_key)
                    .or_default()
                    .push(InertEntry {
                        bytes: live.bytes,
                        reason: InertReason::Detached,
                    });
            }
        }
    }

    /// The state transitions a version change implies, run when the
    /// change lands at a boundary (`#6003` design note §3).
    fn transition_state_on_version_change(
        &mut self,
        entity_id: &str,
        offer: &LibraryOffer,
        from: LibraryVersion,
    ) {
        let from_key = StateKey {
            entity: entity_id.to_string(),
            library_id: offer.library_id.clone(),
            major: from.major,
        };
        if from.major == offer.version.major {
            if from.minor == offer.version.minor {
                // Patch-only change: no state semantics.
                return;
            }
            if offer.declares_migration {
                // The declared migration runs: the state carries
                // over live under the new minor version.
                if let Some(live) = self.live_state.get_mut(&from_key) {
                    live.minor = offer.version.minor;
                }
            } else if let Some(live) = self.live_state.remove(&from_key) {
                // No migration declared: the state is kept inert and
                // flagged — never silently migrated, never dropped.
                self.inert_state
                    .entry(from_key)
                    .or_default()
                    .push(InertEntry {
                        bytes: live.bytes,
                        reason: InertReason::MinorUpgradeNoMigration,
                    });
            }
        } else if let Some(live) = self.live_state.remove(&from_key) {
            // Major change: the old state is parked in its own
            // namespace until the entity owner adopts or drops it.
            // The new major version starts with no live state.
            self.inert_state
                .entry(from_key)
                .or_default()
                .push(InertEntry {
                    bytes: live.bytes,
                    reason: InertReason::MajorParked,
                });
        }
    }

    /// Write the attached library's state. The state is namespaced
    /// by the attached version's major version; writing requires a
    /// live attachment in the current attachment set.
    ///
    /// # Errors
    ///
    /// [`StateWriteRefusal::NotAttached`] when the library is not
    /// attached to the entity.
    pub fn put_state(
        &mut self,
        entity_id: &str,
        library_id: &str,
        bytes: Vec<u8>,
    ) -> Result<(), StateWriteRefusal> {
        let attachment = self
            .attachment(entity_id, library_id)
            .ok_or(StateWriteRefusal::NotAttached)?;
        let key = StateKey {
            entity: entity_id.to_string(),
            library_id: library_id.to_string(),
            major: attachment.version.major,
        };
        let minor = attachment.version.minor;
        self.live_state.insert(key, LiveState { minor, bytes });
        Ok(())
    }

    /// Read the attached library's live state. Inert state is never
    /// readable through this path — it is kept, not live.
    #[must_use]
    pub fn get_state(&self, entity_id: &str, library_id: &str) -> Option<&[u8]> {
        let attachment = self.attachment(entity_id, library_id)?;
        let key = StateKey {
            entity: entity_id.to_string(),
            library_id: library_id.to_string(),
            major: attachment.version.major,
        };
        self.live_state.get(&key).map(|live| live.bytes.as_slice())
    }

    /// The inert generations kept for (entity, library, major
    /// version), oldest first — the owner's view for adopt/drop
    /// decisions and for the minor-upgrade flag.
    #[must_use]
    pub fn inert_entries(&self, entity_id: &str, library_id: &str, major: u64) -> &[InertEntry] {
        let key = StateKey {
            entity: entity_id.to_string(),
            library_id: library_id.to_string(),
            major,
        };
        self.inert_state
            .get(&key)
            .map_or(&[], |entries| entries.as_slice())
    }

    /// Adopt the state parked by a major upgrade into the attached
    /// (new major) version's live state. Owner only: the design note
    /// makes adopt-or-drop the entity owner's explicit decision.
    ///
    /// # Errors
    ///
    /// [`StateDecisionRefusal`] naming the failed check.
    pub fn adopt_parked(
        &mut self,
        actor: &str,
        entity_id: &str,
        library_id: &str,
        from_major: u64,
    ) -> Result<(), StateDecisionRefusal> {
        let entity = self
            .entities
            .get(entity_id)
            .ok_or(StateDecisionRefusal::UnknownEntity)?;
        if actor != entity.owner {
            return Err(StateDecisionRefusal::NotOwner {
                actor: actor.to_string(),
            });
        }
        let (to_major, attached_minor) = self
            .attachment(entity_id, library_id)
            .filter(|attachment| attachment.version.major != from_major)
            .map(|attachment| (attachment.version.major, attachment.version.minor))
            .ok_or(StateDecisionRefusal::NotAttached)?;
        let to_key = StateKey {
            entity: entity_id.to_string(),
            library_id: library_id.to_string(),
            major: to_major,
        };
        if self.live_state.contains_key(&to_key) {
            return Err(StateDecisionRefusal::LiveStatePresent);
        }
        let from_key = StateKey {
            entity: entity_id.to_string(),
            library_id: library_id.to_string(),
            major: from_major,
        };
        let entries = self
            .inert_state
            .get_mut(&from_key)
            .ok_or(StateDecisionRefusal::NoParkedState)?;
        let position = entries
            .iter()
            .rposition(|entry| entry.reason == InertReason::MajorParked)
            .ok_or(StateDecisionRefusal::NoParkedState)?;
        let entry = entries.remove(position);
        self.live_state.insert(
            to_key,
            LiveState {
                minor: attached_minor,
                bytes: entry.bytes,
            },
        );
        Ok(())
    }

    /// Drop every inert generation kept for (entity, library, major
    /// version) — the entity owner's explicit drop, the only way
    /// inert state is ever deleted. Returns how many generations
    /// were dropped. Live state is not droppable through this path.
    ///
    /// # Errors
    ///
    /// [`StateDecisionRefusal`] naming the failed check.
    pub fn drop_inert(
        &mut self,
        actor: &str,
        entity_id: &str,
        library_id: &str,
        major: u64,
    ) -> Result<usize, StateDecisionRefusal> {
        let entity = self
            .entities
            .get(entity_id)
            .ok_or(StateDecisionRefusal::UnknownEntity)?;
        if actor != entity.owner {
            return Err(StateDecisionRefusal::NotOwner {
                actor: actor.to_string(),
            });
        }
        let key = StateKey {
            entity: entity_id.to_string(),
            library_id: library_id.to_string(),
            major,
        };
        Ok(self
            .inert_state
            .remove(&key)
            .map_or(0, |entries| entries.len()))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::library_manifest::{BuildMode, LibraryKind, PlatformRequirement};

    fn caps(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|name| (*name).to_string()).collect()
    }

    /// A world whose ceiling permits chat + records; `author-a` holds
    /// an attach capability on the `genre` class.
    fn registry() -> AttachmentRegistry {
        let mut attach_capable = BTreeMap::new();
        attach_capable.insert("genre".to_string(), caps(&["author-a"]));
        let mut registry =
            AttachmentRegistry::new(caps(&["chat.say", "records.read"]), attach_capable);
        registry.register_entity(EntityRecord {
            entity_id: "entity-1".to_string(),
            class: "genre".to_string(),
            owner: "owner-1".to_string(),
            held_caps: caps(&["chat.say", "records.read", "records.write"]),
        });
        registry
    }

    fn offer(version: &str, requirements: &[&str]) -> LibraryOffer {
        LibraryOffer {
            library_id: "lib-hash-1".to_string(),
            version: LibraryVersion::parse(version).expect("test version parses"),
            requirements: caps(requirements),
            declares_migration: false,
        }
    }

    fn offer_with_migration(version: &str, requirements: &[&str]) -> LibraryOffer {
        LibraryOffer {
            declares_migration: true,
            ..offer(version, requirements)
        }
    }

    /// Attach as the owner and cross the boundary; returns the new
    /// tick's snapshot.
    fn attached(registry: &mut AttachmentRegistry, offer: LibraryOffer) -> TickSnapshot {
        registry
            .attach("owner-1", "entity-1", offer)
            .expect("attach stages");
        registry.begin_tick()
    }

    // Oracle for the intersection check: the design note's §2 rule —
    // a library may exercise only (entity held ∩ world ceiling), and
    // a refusal names the widening capabilities exhaustively.

    #[test]
    fn attach_refuses_caps_the_entity_does_not_hold() {
        let mut registry = registry();
        let refusal = registry
            .attach(
                "owner-1",
                "entity-1",
                offer("1.0.0", &["chat.say", "net.fetch"]),
            )
            .expect_err("net.fetch is not held by the entity");
        assert_eq!(
            refusal,
            AttachRefusal::CapabilityWidening {
                widening: caps(&["net.fetch"]),
            }
        );
    }

    #[test]
    fn attach_refuses_caps_above_the_world_ceiling() {
        let mut registry = registry();
        // records.write is held by the entity but above the ceiling.
        let refusal = registry
            .attach("owner-1", "entity-1", offer("1.0.0", &["records.write"]))
            .expect_err("records.write is above the world ceiling");
        assert_eq!(
            refusal,
            AttachRefusal::CapabilityWidening {
                widening: caps(&["records.write"]),
            }
        );
    }

    #[test]
    fn attach_refusal_names_every_widening_cap() {
        let mut registry = registry();
        let refusal = registry
            .attach(
                "owner-1",
                "entity-1",
                offer("1.0.0", &["net.fetch", "records.write", "chat.say"]),
            )
            .expect_err("two caps widen");
        assert_eq!(
            refusal,
            AttachRefusal::CapabilityWidening {
                widening: caps(&["net.fetch", "records.write"]),
            }
        );
    }

    #[test]
    fn attach_within_the_intersection_is_admitted() {
        let mut registry = registry();
        let snapshot = attached(&mut registry, offer("1.0.0", &["chat.say", "records.read"]));
        let attachment = snapshot
            .resolve("entity-1", "lib-hash-1")
            .expect("attached in the new tick");
        assert_eq!(attachment.version, LibraryVersion::parse("1.0.0").unwrap());
        assert_eq!(attachment.requirements, caps(&["chat.say", "records.read"]));
    }

    #[test]
    fn refused_attach_stages_nothing() {
        let mut registry = registry();
        let _ = registry.attach("owner-1", "entity-1", offer("1.0.0", &["net.fetch"]));
        let snapshot = registry.begin_tick();
        assert!(!snapshot.is_attached("entity-1", "lib-hash-1"));
        assert!(registry.attachment("entity-1", "lib-hash-1").is_none());
    }

    // Oracle for authorization: the design note's §1 rule — the
    // entity owner, or a world author holding an explicit attach
    // capability on the entity's class; nobody else.

    #[test]
    fn attach_requires_owner_or_attach_capability() {
        let mut registry = registry();
        let refusal = registry
            .attach("stranger", "entity-1", offer("1.0.0", &["chat.say"]))
            .expect_err("a stranger may not attach");
        assert_eq!(
            refusal,
            AttachRefusal::NotAuthorized {
                actor: "stranger".to_string(),
            }
        );
        // The attach-capable author for the class may attach.
        registry
            .attach("author-a", "entity-1", offer("1.0.0", &["chat.say"]))
            .expect("attach-capable author stages");
        // And may detach.
        registry
            .detach("author-a", "entity-1", "lib-hash-1")
            .expect("attach-capable author detaches");
    }

    #[test]
    fn attach_to_unknown_entity_is_refused() {
        let mut registry = registry();
        assert_eq!(
            registry.attach("owner-1", "ghost", offer("1.0.0", &[])),
            Err(AttachRefusal::UnknownEntity)
        );
    }

    #[test]
    fn duplicate_attach_at_same_version_is_refused() {
        let mut registry = registry();
        let _ = attached(&mut registry, offer("1.0.0", &["chat.say"]));
        assert_eq!(
            registry.attach("owner-1", "entity-1", offer("1.0.0", &["chat.say"])),
            Err(AttachRefusal::AlreadyAttached {
                version: LibraryVersion::parse("1.0.0").unwrap(),
            })
        );
    }

    #[test]
    fn detach_when_not_attached_is_refused() {
        let mut registry = registry();
        assert_eq!(
            registry.detach("owner-1", "entity-1", "lib-hash-1"),
            Err(DetachRefusal::NotAttached)
        );
    }

    // Oracle for tick boundaries: the design note's §1 rule — the
    // attachment set is snapshotted at tick start, so in-flight
    // invocations finish under the set they started with.

    #[test]
    fn staged_attach_is_invisible_until_the_boundary() {
        let mut registry = registry();
        let before = registry.begin_tick();
        registry
            .attach("owner-1", "entity-1", offer("1.0.0", &["chat.say"]))
            .expect("attach stages");
        assert!(!before.is_attached("entity-1", "lib-hash-1"));
        let after = registry.begin_tick();
        assert!(after.is_attached("entity-1", "lib-hash-1"));
    }

    #[test]
    fn detach_never_pulls_behaviour_from_an_in_flight_invocation() {
        let mut registry = registry();
        let in_flight = attached(&mut registry, offer("1.0.0", &["chat.say"]));
        registry
            .detach("owner-1", "entity-1", "lib-hash-1")
            .expect("detach stages");
        // The detach has not landed yet: the current set still has it.
        assert!(registry.attachment("entity-1", "lib-hash-1").is_some());
        let next = registry.begin_tick();
        // The snapshot the in-flight invocation holds still resolves
        // the library; the new tick's snapshot does not.
        assert!(in_flight.is_attached("entity-1", "lib-hash-1"));
        assert!(!next.is_attached("entity-1", "lib-hash-1"));
    }

    // Oracle for state on detach: the design note's §3 rule — state
    // is kept inert on detach, revived by a reattach of the same
    // major version, and droppable only by the entity owner.

    #[test]
    fn detach_keeps_state_inert_and_reattach_revives_it() {
        let mut registry = registry();
        let _ = attached(&mut registry, offer("1.0.0", &["chat.say"]));
        registry
            .put_state("entity-1", "lib-hash-1", b"save-data".to_vec())
            .expect("attached library writes state");
        registry
            .detach("owner-1", "entity-1", "lib-hash-1")
            .expect("detach stages");
        let _ = registry.begin_tick();
        // Inert: not readable, but kept.
        assert_eq!(registry.get_state("entity-1", "lib-hash-1"), None);
        let inert = registry.inert_entries("entity-1", "lib-hash-1", 1);
        assert_eq!(inert.len(), 1);
        assert_eq!(inert[0].reason, InertReason::Detached);
        assert_eq!(inert[0].bytes, b"save-data".to_vec());
        assert!(!inert[0].flagged());
        // Reattach of the same major version revives the state.
        let _ = attached(&mut registry, offer("1.1.0", &["chat.say"]));
        assert_eq!(
            registry.get_state("entity-1", "lib-hash-1"),
            Some(b"save-data".as_slice())
        );
        assert_eq!(registry.inert_entries("entity-1", "lib-hash-1", 1), []);
    }

    #[test]
    fn state_write_requires_a_live_attachment() {
        let mut registry = registry();
        assert_eq!(
            registry.put_state("entity-1", "lib-hash-1", b"x".to_vec()),
            Err(StateWriteRefusal::NotAttached)
        );
    }

    #[test]
    fn only_the_owner_may_drop_inert_state() {
        let mut registry = registry();
        let _ = attached(&mut registry, offer("1.0.0", &["chat.say"]));
        registry
            .put_state("entity-1", "lib-hash-1", b"save-data".to_vec())
            .unwrap();
        registry
            .detach("owner-1", "entity-1", "lib-hash-1")
            .unwrap();
        let _ = registry.begin_tick();
        assert_eq!(
            registry.drop_inert("author-a", "entity-1", "lib-hash-1", 1),
            Err(StateDecisionRefusal::NotOwner {
                actor: "author-a".to_string(),
            })
        );
        assert_eq!(
            registry.drop_inert("owner-1", "entity-1", "lib-hash-1", 1),
            Ok(1)
        );
        assert_eq!(registry.inert_entries("entity-1", "lib-hash-1", 1), []);
        // After the drop, a reattach revives nothing.
        let _ = attached(&mut registry, offer("1.0.0", &["chat.say"]));
        assert_eq!(registry.get_state("entity-1", "lib-hash-1"), None);
    }

    // Oracle for version change: the design note's §3 rule — a minor
    // upgrade runs the declared migration or keeps the state inert
    // and flagged; a major upgrade parks the state for the owner's
    // adopt-or-drop decision. No silent migration, no silent drop.

    #[test]
    fn minor_upgrade_without_migration_keeps_state_inert_and_flagged() {
        let mut registry = registry();
        let _ = attached(&mut registry, offer("1.1.0", &["chat.say"]));
        registry
            .put_state("entity-1", "lib-hash-1", b"v1-state".to_vec())
            .unwrap();
        let _ = attached(&mut registry, offer("1.2.0", &["chat.say"]));
        // The new minor version is attached but cannot read the old
        // state; the state is kept, inert and flagged.
        assert_eq!(
            registry
                .attachment("entity-1", "lib-hash-1")
                .map(|a| a.version),
            Some(LibraryVersion::parse("1.2.0").unwrap())
        );
        assert_eq!(registry.get_state("entity-1", "lib-hash-1"), None);
        let inert = registry.inert_entries("entity-1", "lib-hash-1", 1);
        assert_eq!(inert.len(), 1);
        assert_eq!(inert[0].reason, InertReason::MinorUpgradeNoMigration);
        assert!(inert[0].flagged());
        assert_eq!(inert[0].bytes, b"v1-state".to_vec());
    }

    #[test]
    fn minor_upgrade_with_migration_carries_state_live() {
        let mut registry = registry();
        let _ = attached(&mut registry, offer("1.1.0", &["chat.say"]));
        registry
            .put_state("entity-1", "lib-hash-1", b"v1-state".to_vec())
            .unwrap();
        let _ = attached(&mut registry, offer_with_migration("1.2.0", &["chat.say"]));
        assert_eq!(
            registry.get_state("entity-1", "lib-hash-1"),
            Some(b"v1-state".as_slice())
        );
        assert_eq!(registry.inert_entries("entity-1", "lib-hash-1", 1), []);
    }

    #[test]
    fn major_upgrade_parks_state_until_the_owner_adopts_it() {
        let mut registry = registry();
        let _ = attached(&mut registry, offer("1.2.0", &["chat.say"]));
        registry
            .put_state("entity-1", "lib-hash-1", b"v1-state".to_vec())
            .unwrap();
        let _ = attached(&mut registry, offer("2.0.0", &["chat.say"]));
        // Parked: the new major version starts with no live state.
        assert_eq!(registry.get_state("entity-1", "lib-hash-1"), None);
        let inert = registry.inert_entries("entity-1", "lib-hash-1", 1);
        assert_eq!(inert.len(), 1);
        assert_eq!(inert[0].reason, InertReason::MajorParked);
        // Adoption is the owner's decision, nobody else's.
        assert_eq!(
            registry.adopt_parked("author-a", "entity-1", "lib-hash-1", 1),
            Err(StateDecisionRefusal::NotOwner {
                actor: "author-a".to_string(),
            })
        );
        registry
            .adopt_parked("owner-1", "entity-1", "lib-hash-1", 1)
            .expect("owner adopts the parked state");
        assert_eq!(
            registry.get_state("entity-1", "lib-hash-1"),
            Some(b"v1-state".as_slice())
        );
        assert_eq!(registry.inert_entries("entity-1", "lib-hash-1", 1), []);
    }

    #[test]
    fn major_upgrade_parked_state_can_be_dropped_instead() {
        let mut registry = registry();
        let _ = attached(&mut registry, offer("1.0.0", &["chat.say"]));
        registry
            .put_state("entity-1", "lib-hash-1", b"v1-state".to_vec())
            .unwrap();
        let _ = attached(&mut registry, offer("2.0.0", &["chat.say"]));
        assert_eq!(
            registry.drop_inert("owner-1", "entity-1", "lib-hash-1", 1),
            Ok(1)
        );
        assert_eq!(
            registry.adopt_parked("owner-1", "entity-1", "lib-hash-1", 1),
            Err(StateDecisionRefusal::NoParkedState)
        );
        assert_eq!(registry.get_state("entity-1", "lib-hash-1"), None);
    }

    // The manifest seam: an offer built from a parsed manifest takes
    // its identity from the hash and its footprint from the declared
    // requirements, and an unparseable version is refused.

    #[test]
    fn offer_from_manifest_uses_hash_identity_and_declared_requirements() {
        let manifest = LibraryManifest {
            name: "chess-clock".to_string(),
            version: "1.2.0".to_string(),
            kind: LibraryKind::SourceLibrary,
            build_mode: BuildMode::CompileIn,
            hash: "hash-of-chess-clock".to_string(),
            license: "MIT".to_string(),
            requirements: vec![PlatformRequirement {
                name: "records.read".to_string(),
                version_pin: None,
            }],
            subtypes: Vec::new(),
            attachments: Vec::new(),
            migrations: Vec::new(),
            source_url: None,
        };
        let offer = LibraryOffer::from_manifest(&manifest).expect("offer builds");
        assert_eq!(offer.library_id, "hash-of-chess-clock");
        assert_eq!(offer.version, LibraryVersion::parse("1.2.0").unwrap());
        assert_eq!(offer.requirements, caps(&["records.read"]));
        assert!(!offer.declares_migration);
    }

    #[test]
    fn offer_from_manifest_derives_migration_from_declarations() {
        // Oracle: #12640 — declares_migration is manifest data: a
        // manifest naming a migration source offers a migration, a
        // bare manifest offers none, and an unparsable source fails
        // the build instead of admitting a migration to nowhere.
        let text = r#"
library "clock" version="1.1.0" {
    kind "source-library"
    build-mode "compile-in"
    hash "blake3:11"
    license "MIT"
    platform {
        requires "records.read"
    }
    migrate from="1.0.0"
}
"#;
        let manifest = LibraryManifest::parse(text).expect("manifest parses");
        let offer = LibraryOffer::from_manifest(&manifest).expect("offer builds");
        assert!(offer.declares_migration);

        let bare = LibraryManifest::parse(&text.replace("    migrate from=\"1.0.0\"\n", ""))
            .expect("bare manifest parses");
        let bare_offer = LibraryOffer::from_manifest(&bare).expect("bare offer builds");
        assert!(!bare_offer.declares_migration);

        let bad = LibraryManifest::parse(&text.replace("1.0.0", "one-point-oh"))
            .expect("bad-source manifest parses");
        assert!(
            LibraryOffer::from_manifest(&bad).is_err(),
            "an unparsable migration source must fail the offer build"
        );
    }

    #[test]
    fn version_parse_is_fail_closed() {
        assert_eq!(
            LibraryVersion::parse("2"),
            Ok(LibraryVersion {
                major: 2,
                minor: 0,
                patch: 0,
            })
        );
        for bad in ["", "v1.2", "1.x", "1.2.3.4", "1..2", "-1"] {
            assert!(
                LibraryVersion::parse(bad).is_err(),
                "version {bad:?} must be refused"
            );
        }
    }
}
