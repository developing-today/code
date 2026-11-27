//! Invocation-time resolution of runtime-attached library behavior (`#12640`).
//!
//! The registry core in [`id_caps::library_attach`] stages attach/detach
//! requests and applies them at tick boundaries, handing out a
//! [`TickSnapshot`](id_caps::library_attach::TickSnapshot) per tick.
//! This module is the `id-worlds` side of that seam: it owns one registry
//! plus the current tick's snapshot, and answers the one question future
//! verb dispatch needs — "may this entity invoke this library *in this
//! tick*?" — against the snapshot, never against the live set.
//!
//! The design note on `#6003` (§1) freezes the semantics this enforces: a
//! detach staged mid-tick never pulls behaviour out from under an
//! invocation that started under an earlier snapshot. Later slices wire
//! live verb dispatch, declared-migration execution and registry
//! persistence through this seam; none of those change this contract.

use std::collections::{BTreeMap, BTreeSet};

use id_caps::library_attach::{
    ActiveAttachment, AttachRefusal, AttachmentRegistry, DetachRefusal, EntityRecord, LibraryOffer,
    TickSnapshot,
};

/// The attachment state of one world as invocation dispatch sees it: the
/// registry that stages attach/detach plus the snapshot of the tick whose
/// invocations are currently resolving.
#[derive(Debug)]
pub struct AttachmentDispatch {
    registry: AttachmentRegistry,
    snapshot: TickSnapshot,
}

impl AttachmentDispatch {
    /// A dispatch view over a fresh registry for a world with the given
    /// capability ceiling and per-class attach-capable authors.
    #[must_use]
    pub fn new(
        ceiling: BTreeSet<String>,
        attach_capable: BTreeMap<String, BTreeSet<String>>,
    ) -> Self {
        let mut registry = AttachmentRegistry::new(ceiling, attach_capable);
        let snapshot = registry.begin_tick();
        Self { registry, snapshot }
    }

    /// Register (or re-register) an entity. Re-registration replaces the
    /// record; attachments and state are keyed by identity and untouched.
    pub fn register_entity(&mut self, entity: EntityRecord) {
        self.registry.register_entity(entity);
    }

    /// Stage an attach, to take effect at the next tick boundary. All
    /// checks run now; a refused attach stages nothing.
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
        self.registry.attach(actor, entity_id, offer)
    }

    /// Stage a detach, to take effect at the next tick boundary. An
    /// in-flight invocation holding an earlier snapshot is unaffected.
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
        self.registry.detach(actor, entity_id, library_id)
    }

    /// Close the current tick and open the next: staged changes land and
    /// the snapshot invocations resolve against is refreshed. Snapshots
    /// handed out earlier are unaffected.
    pub fn begin_tick(&mut self) {
        self.snapshot = self.registry.begin_tick();
    }

    /// The tick snapshot invocations currently resolve against.
    #[must_use]
    pub fn snapshot(&self) -> &TickSnapshot {
        &self.snapshot
    }

    /// What `library_id` on `entity_id` resolves to *in the current tick*,
    /// if attached. This is the call future verb dispatch makes per
    /// invocation; it never sees staged-but-unapplied changes.
    #[must_use]
    pub fn resolve(&self, entity_id: &str, library_id: &str) -> Option<&ActiveAttachment> {
        self.snapshot.resolve(entity_id, library_id)
    }

    /// Whether the library is attached to the entity in the current tick.
    #[must_use]
    pub fn is_attached(&self, entity_id: &str, library_id: &str) -> bool {
        self.snapshot.is_attached(entity_id, library_id)
    }

    /// Resolve one live invocation against the current tick's snapshot.
    ///
    /// This is the boundary future verb dispatch calls per invocation:
    /// `Ok` carries the attached behavior in force for this tick;
    /// `Err` means the invocation must not dispatch — the library is
    /// absent, was refused at attach time, or is staged-but-unapplied.
    /// It delegates to [`Self::resolve`], so tick-boundary semantics
    /// are unchanged: staged changes stay invisible until
    /// [`Self::begin_tick`], and a snapshot handed out earlier keeps
    /// resolving after a later detach lands.
    ///
    /// # Errors
    ///
    /// [`InvocationRefusal::NotAttached`] when nothing is attached
    /// for `(entity_id, library_id)` in the current tick.
    pub fn resolve_invocation(
        &self,
        entity_id: &str,
        library_id: &str,
    ) -> Result<&ActiveAttachment, InvocationRefusal> {
        self.resolve(entity_id, library_id)
            .ok_or_else(|| InvocationRefusal::NotAttached {
                entity_id: entity_id.to_string(),
                library_id: library_id.to_string(),
            })
    }
}

/// Why a live library invocation must not dispatch.
///
/// The refusal is fail-closed and names the invocation pair; the
/// caller reports it without retrying against the live registry set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InvocationRefusal {
    /// No attachment is in force for the pair in the current tick:
    /// never attached, refused at attach time, detached, or
    /// staged-but-unapplied.
    NotAttached {
        /// The invoking entity.
        entity_id: String,
        /// The invoked library.
        library_id: String,
    },
}

impl std::fmt::Display for InvocationRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAttached {
                entity_id,
                library_id,
            } => write!(
                f,
                "library {library_id} is not attached to entity {entity_id} in the current tick"
            ),
        }
    }
}

impl std::error::Error for InvocationRefusal {}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use id_caps::library_attach::LibraryVersion;

    fn caps(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|name| (*name).to_string()).collect()
    }

    /// A world whose ceiling permits chat + records.read; `author-a`
    /// holds an attach capability on the `genre` class.
    fn dispatch() -> AttachmentDispatch {
        let mut attach_capable = BTreeMap::new();
        attach_capable.insert("genre".to_string(), caps(&["author-a"]));
        let mut dispatch =
            AttachmentDispatch::new(caps(&["chat.say", "records.read"]), attach_capable);
        dispatch.register_entity(EntityRecord {
            entity_id: "entity-1".to_string(),
            class: "genre".to_string(),
            owner: "owner-1".to_string(),
            held_caps: caps(&["chat.say", "records.read", "records.write"]),
        });
        dispatch
    }

    fn offer(version: &str, requirements: &[&str]) -> LibraryOffer {
        LibraryOffer {
            library_id: "lib-hash-1".to_string(),
            version: LibraryVersion::parse(version).expect("test version parses"),
            requirements: caps(requirements),
            declares_migration: false,
        }
    }

    // Oracle: the design note's §1 rule — the attachment set is
    // snapshotted at tick start, so a staged change is invisible until
    // the boundary and in-flight invocations finish under the set they
    // started with.

    #[test]
    fn test_staged_attach_invisible_until_tick() {
        let mut dispatch = dispatch();
        dispatch
            .attach("owner-1", "entity-1", offer("1.0.0", &["chat.say"]))
            .expect("attach stages");
        assert!(!dispatch.is_attached("entity-1", "lib-hash-1"));
        assert!(dispatch.resolve("entity-1", "lib-hash-1").is_none());
        dispatch.begin_tick();
        assert!(dispatch.is_attached("entity-1", "lib-hash-1"));
        let attachment = dispatch
            .resolve("entity-1", "lib-hash-1")
            .expect("attached after the boundary");
        assert_eq!(
            attachment.version,
            LibraryVersion::parse("1.0.0").expect("test version parses")
        );
    }

    #[test]
    fn test_detach_never_pulls_behavior_from_an_in_flight_invocation() {
        let mut dispatch = dispatch();
        dispatch
            .attach("owner-1", "entity-1", offer("1.0.0", &["chat.say"]))
            .expect("attach stages");
        dispatch.begin_tick();
        let in_flight = dispatch.snapshot().clone();
        dispatch
            .detach("owner-1", "entity-1", "lib-hash-1")
            .expect("detach stages");
        // The detach has not landed yet: the current tick still resolves.
        assert!(dispatch.is_attached("entity-1", "lib-hash-1"));
        dispatch.begin_tick();
        // The in-flight snapshot still resolves the library; the new
        // tick's snapshot does not.
        assert!(in_flight.is_attached("entity-1", "lib-hash-1"));
        assert!(!dispatch.is_attached("entity-1", "lib-hash-1"));
        assert!(dispatch.resolve("entity-1", "lib-hash-1").is_none());
    }

    #[test]
    fn test_refused_attach_never_resolves() {
        let mut dispatch = dispatch();
        // records.write is held by the entity but above the world ceiling.
        dispatch
            .attach("owner-1", "entity-1", offer("1.0.0", &["records.write"]))
            .expect_err("widening attach is refused");
        dispatch.begin_tick();
        assert!(!dispatch.is_attached("entity-1", "lib-hash-1"));
    }

    #[test]
    fn test_unauthorized_actor_cannot_stage() {
        let mut dispatch = dispatch();
        assert_eq!(
            dispatch.attach("stranger", "entity-1", offer("1.0.0", &["chat.say"])),
            Err(AttachRefusal::NotAuthorized {
                actor: "stranger".to_string(),
            })
        );
        // The attach-capable author for the class may attach and detach.
        dispatch
            .attach("author-a", "entity-1", offer("1.0.0", &["chat.say"]))
            .expect("attach-capable author stages");
        dispatch
            .detach("author-a", "entity-1", "lib-hash-1")
            .expect("attach-capable author detaches");
    }

    // Oracle: the #12640 slice acceptance — live invocations resolve
    // through the current tick snapshot only, and anything else
    // refuses fail-closed instead of dispatching.

    #[test]
    fn test_live_invocation_resolves_attached_behavior() {
        let mut dispatch = dispatch();
        dispatch
            .attach("owner-1", "entity-1", offer("1.0.0", &["chat.say"]))
            .expect("attach stages");
        dispatch.begin_tick();
        let attachment = dispatch
            .resolve_invocation("entity-1", "lib-hash-1")
            .expect("attached behavior dispatches");
        assert_eq!(
            attachment.version,
            LibraryVersion::parse("1.0.0").expect("test version parses")
        );
    }

    #[test]
    fn test_live_invocation_ignores_staged_attach_until_tick() {
        let mut dispatch = dispatch();
        dispatch
            .attach("owner-1", "entity-1", offer("1.0.0", &["chat.say"]))
            .expect("attach stages");
        assert_eq!(
            dispatch.resolve_invocation("entity-1", "lib-hash-1"),
            Err(super::InvocationRefusal::NotAttached {
                entity_id: "entity-1".to_string(),
                library_id: "lib-hash-1".to_string(),
            })
        );
        dispatch.begin_tick();
        dispatch
            .resolve_invocation("entity-1", "lib-hash-1")
            .expect("visible after the boundary");
    }

    #[test]
    fn test_live_invocation_keeps_prior_snapshot_after_detach_lands() {
        let mut dispatch = dispatch();
        dispatch
            .attach("owner-1", "entity-1", offer("1.0.0", &["chat.say"]))
            .expect("attach stages");
        dispatch.begin_tick();
        let in_flight = dispatch.snapshot().clone();
        dispatch
            .detach("owner-1", "entity-1", "lib-hash-1")
            .expect("detach stages");
        // Staged detach is invisible to the live boundary until the tick.
        dispatch
            .resolve_invocation("entity-1", "lib-hash-1")
            .expect("still dispatches before the boundary");
        dispatch.begin_tick();
        assert_eq!(
            dispatch.resolve_invocation("entity-1", "lib-hash-1"),
            Err(super::InvocationRefusal::NotAttached {
                entity_id: "entity-1".to_string(),
                library_id: "lib-hash-1".to_string(),
            })
        );
        // The in-flight invocation keeps the snapshot it started with.
        assert!(in_flight.resolve("entity-1", "lib-hash-1").is_some());
    }

    #[test]
    fn test_live_invocation_refuses_absent_and_refused_attachments() {
        let mut dispatch = dispatch();
        // Never attached: fail closed with the named pair.
        assert_eq!(
            dispatch.resolve_invocation("entity-1", "lib-hash-1"),
            Err(super::InvocationRefusal::NotAttached {
                entity_id: "entity-1".to_string(),
                library_id: "lib-hash-1".to_string(),
            })
        );
        // Refused at attach time (widening): stages nothing, still refuses.
        dispatch
            .attach("owner-1", "entity-1", offer("1.0.0", &["records.write"]))
            .expect_err("widening attach is refused");
        dispatch.begin_tick();
        assert_eq!(
            dispatch.resolve_invocation("entity-1", "lib-hash-1"),
            Err(super::InvocationRefusal::NotAttached {
                entity_id: "entity-1".to_string(),
                library_id: "lib-hash-1".to_string(),
            })
        );
        // Unknown entity and unknown library likewise refuse.
        assert_eq!(
            dispatch.resolve_invocation("no-such-entity", "lib-hash-1"),
            Err(super::InvocationRefusal::NotAttached {
                entity_id: "no-such-entity".to_string(),
                library_id: "lib-hash-1".to_string(),
            })
        );
    }
}
