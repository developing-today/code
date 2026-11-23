//! Capability classes — the user-meaningful disclosure each capability
//! name maps to (#5999 §2), and the shipped class table.
//!
//! A class is one sentence of consequence a person can act on. Class
//! assignment is registry data: [`ProviderManifest`] carries a `class`
//! field naming an id in this table (#5999 §2.4 — prefix guessing is
//! explicitly rejected, so nothing here derives a class from a name).
//! Message *text* is client-side, keyed by class id: the registry entry
//! supplies the id, never the sentence, so a server cannot soften the
//! wording of the warning it triggers. The sentences below are the
//! §2.2/§2.3 semantics; final copy is #5960's wording review (#5999
//! §9.4).
//!
//! Fail-closed in both directions (#5999 §2.4): a capability name with
//! no class assignment, and a class id this table does not know, both
//! resolve to [`CLASS_UNKNOWN`] — disclosing, and subsumed by nothing,
//! so it triggers whenever it is new. Names registered after #5999's
//! table was written carry [`CLASS_UNKNOWN`] in their manifest until a
//! design pass assigns them a class; the tests below pin exactly which
//! names those are, so the gap cannot grow silently.
//!
//! The comparison that consumes these classes (message sets,
//! subsumption, trigger rules) is #5999 §6's contract, implemented by
//! #12921; this module is the vocabulary it reads.
//!
//! [`ProviderManifest`]: crate::world_caps::ProviderManifest

use crate::world_caps::{ProviderManifest, ProviderRegistry};

/// The version of the shipped class table, recorded alongside every
/// comparison a client computes (#5999 §2.4): if the version changed
/// since a baseline was accepted, the comparison is recomputed under
/// the newer table. Bump it whenever a class is added, split, or
/// merged — never for a wording-only edit.
pub const CAP_CLASS_TABLE_VERSION: u32 = 1;

/// Messageless: reading a clock, a random number, platform data types,
/// or the world's own metadata discloses nothing a person would decide
/// on (#5999 §2.2).
pub const CLASS_NONE: &str = "cap.none";
/// "See who is in this world, and when they arrive or leave."
pub const CLASS_WORLD_PRESENCE: &str = "world.presence";
/// "Read this world's chat history."
pub const CLASS_CHAT_READ: &str = "chat.read";
/// "Post messages to this world's chat as this world."
pub const CLASS_CHAT_WRITE: &str = "chat.write";
/// "Read the records this world stores."
pub const CLASS_WORLD_RECORDS: &str = "world.records";
/// "Schedule actions that run later, without you present."
pub const CLASS_WORLD_SCHEDULE: &str = "world.schedule";
/// "Draw a full-screen UI in your client."
pub const CLASS_DEVICE_SCREEN: &str = "device.screen";
/// "Interpret your keyboard input."
pub const CLASS_DEVICE_KEYBOARD: &str = "device.keyboard";
/// "Use this device's location."
pub const CLASS_DEVICE_LOCATION: &str = "device.location";
/// Reserved (#5999 §2.3): sandboxed file read+write, a slot held ahead
/// of the provider that needs it (#5958 S12).
pub const CLASS_FS_SANDBOX: &str = "fs.sandbox";
/// Reserved (#5999 §2.3): arbitrary file read (#5958 S12). Subsumes
/// [`CLASS_FS_SANDBOX`] under #5999 §3's order.
pub const CLASS_FS_READ_ARBITRARY: &str = "fs.read.arbitrary";
/// Reserved (#5999 §2.3): arbitrary network out (#5958 S12).
pub const CLASS_NET_OUT: &str = "net.out";
/// The fail-closed class: a name with no assignment, or an id this
/// table does not know, resolves here. Always disclosing; subsumed by
/// nothing (#5999 §2.4).
pub const CLASS_UNKNOWN: &str = "unknown";

/// One class in the shipped table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CapClass {
    /// The class id a manifest's `class` field names.
    pub id: &'static str,
    /// The client-side sentence for this class, or `None` when the
    /// class is messageless (it discloses nothing).
    pub message: Option<&'static str>,
    /// True for the §2.3 classes, whose slots the table holds ahead of
    /// the vocabulary that needs them.
    pub reserved: bool,
}

impl CapClass {
    /// Whether this class discloses — it has a message a person can
    /// act on. Only [`CLASS_NONE`] is messageless.
    #[must_use]
    pub const fn disclosing(&self) -> bool {
        self.message.is_some()
    }
}

/// The `unknown` class, as a value: the resolution target for every
/// fail-closed lookup in this module.
pub const UNKNOWN_CAP_CLASS: CapClass = CapClass {
    id: CLASS_UNKNOWN,
    message: Some("Do something this version of the app cannot describe."),
    reserved: false,
};

/// The shipped class table: #5999 §2.2's classes, then §2.3's reserved
/// classes, then `unknown`. Versioned by [`CAP_CLASS_TABLE_VERSION`].
pub const CAP_CLASSES: &[CapClass] = &[
    CapClass {
        id: CLASS_NONE,
        message: None,
        reserved: false,
    },
    CapClass {
        id: CLASS_WORLD_PRESENCE,
        message: Some("See who is in this world, and when they arrive or leave."),
        reserved: false,
    },
    CapClass {
        id: CLASS_CHAT_READ,
        message: Some("Read this world's chat history."),
        reserved: false,
    },
    CapClass {
        id: CLASS_CHAT_WRITE,
        message: Some("Post messages to this world's chat as this world."),
        reserved: false,
    },
    CapClass {
        id: CLASS_WORLD_RECORDS,
        message: Some("Read the records this world stores."),
        reserved: false,
    },
    CapClass {
        id: CLASS_WORLD_SCHEDULE,
        message: Some("Schedule actions that run later, without you present."),
        reserved: false,
    },
    CapClass {
        id: CLASS_DEVICE_SCREEN,
        message: Some("Draw a full-screen UI in your client."),
        reserved: false,
    },
    CapClass {
        id: CLASS_DEVICE_KEYBOARD,
        message: Some("Interpret your keyboard input."),
        reserved: false,
    },
    CapClass {
        id: CLASS_DEVICE_LOCATION,
        message: Some("Use this device's location."),
        reserved: false,
    },
    CapClass {
        id: CLASS_FS_SANDBOX,
        message: Some("Read and write files in this world's own storage."),
        reserved: true,
    },
    CapClass {
        id: CLASS_FS_READ_ARBITRARY,
        message: Some("Read any file this server can read."),
        reserved: true,
    },
    CapClass {
        id: CLASS_NET_OUT,
        message: Some("Connect to any host on the network."),
        reserved: true,
    },
    UNKNOWN_CAP_CLASS,
];

/// Look a class id up in the shipped table. `None` for an id the table
/// does not know — callers that cannot tolerate that use
/// [`resolve_cap_class`], which fails closed.
#[must_use]
pub fn cap_class(id: &str) -> Option<&'static CapClass> {
    CAP_CLASSES.iter().find(|class| class.id == id)
}

/// Resolve a class id to a class in the shipped table, failing closed:
/// an id the table does not know (a newer registry than the client)
/// resolves to [`UNKNOWN_CAP_CLASS`] (#5999 §2.4).
#[must_use]
pub fn resolve_cap_class(id: &str) -> &'static CapClass {
    cap_class(id).unwrap_or(&UNKNOWN_CAP_CLASS)
}

impl ProviderManifest {
    /// This provider's capability class, resolved against the shipped
    /// table. A manifest whose `class` id the table does not know
    /// resolves to the `unknown` class — fail-closed (#5999 §2.4).
    #[must_use]
    pub fn cap_class(&self) -> &'static CapClass {
        resolve_cap_class(self.class)
    }
}

impl ProviderRegistry {
    /// The capability class of the provider registered under `name`.
    /// An unregistered name resolves to the `unknown` class, mirroring
    /// the ledger's `<unknown>` usage bucket (#5999 §2.4).
    #[must_use]
    pub fn class_for_name(&self, name: &str) -> &'static CapClass {
        match self.get(name) {
            Some(manifest) => manifest.cap_class(),
            None => &UNKNOWN_CAP_CLASS,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::world_caps::{CATALOG_ENTRIES, PROVIDER_REGISTRY};

    #[test]
    fn shipped_table_is_exactly_the_5999_table() {
        let ids: Vec<&str> = CAP_CLASSES.iter().map(|class| class.id).collect();
        assert_eq!(
            ids,
            [
                CLASS_NONE,
                CLASS_WORLD_PRESENCE,
                CLASS_CHAT_READ,
                CLASS_CHAT_WRITE,
                CLASS_WORLD_RECORDS,
                CLASS_WORLD_SCHEDULE,
                CLASS_DEVICE_SCREEN,
                CLASS_DEVICE_KEYBOARD,
                CLASS_DEVICE_LOCATION,
                CLASS_FS_SANDBOX,
                CLASS_FS_READ_ARBITRARY,
                CLASS_NET_OUT,
                CLASS_UNKNOWN,
            ]
        );
        // Exactly the three §2.3 classes are reserved slots.
        let reserved: Vec<&str> = CAP_CLASSES
            .iter()
            .filter(|class| class.reserved)
            .map(|class| class.id)
            .collect();
        assert_eq!(
            reserved,
            [CLASS_FS_SANDBOX, CLASS_FS_READ_ARBITRARY, CLASS_NET_OUT]
        );
        assert_eq!(CAP_CLASS_TABLE_VERSION, 1);
    }

    #[test]
    fn section_22_assignments_hold_for_every_member() {
        // #5999 §2.2's member lists, name by name, plus http.fetch in
        // §2.3's reserved net.out slot (it is the arbitrary-network-out
        // provider the slot exists for).
        let expected: &[(&str, &str)] = &[
            ("time.now", CLASS_NONE),
            ("time.tick", CLASS_NONE),
            ("random.u64", CLASS_NONE),
            ("world.info", CLASS_NONE),
            ("RocJson", CLASS_NONE),
            ("RocGrid", CLASS_NONE),
            ("RocText", CLASS_NONE),
            ("RocCap", CLASS_NONE),
            ("players", CLASS_WORLD_PRESENCE),
            ("players.list", CLASS_WORLD_PRESENCE),
            ("chat.history", CLASS_CHAT_READ),
            ("chat.say", CLASS_CHAT_WRITE),
            ("records.read", CLASS_WORLD_RECORDS),
            ("schedule.at", CLASS_WORLD_SCHEDULE),
            ("schedule.cancel", CLASS_WORLD_SCHEDULE),
            ("Screen", CLASS_DEVICE_SCREEN),
            ("Key", CLASS_DEVICE_KEYBOARD),
            ("loc.position", CLASS_DEVICE_LOCATION),
            ("loc.watch", CLASS_DEVICE_LOCATION),
            ("http.fetch", CLASS_NET_OUT),
        ];
        for (name, class) in expected {
            let manifest = PROVIDER_REGISTRY.get(name).expect(name);
            assert_eq!(manifest.class, *class, "{name}");
            assert_eq!(manifest.cap_class().id, *class, "{name}");
            assert_eq!(PROVIDER_REGISTRY.class_for_name(name).id, *class, "{name}");
        }
    }

    #[test]
    fn reserved_fs_classes_have_no_members_yet() {
        // The fs.* slots are held ahead of any provider (#5999 §2.3):
        // no registered name may claim one until its design lands.
        for entry in CATALOG_ENTRIES {
            assert_ne!(entry.class, CLASS_FS_SANDBOX, "{}", entry.name);
            assert_ne!(entry.class, CLASS_FS_READ_ARBITRARY, "{}", entry.name);
        }
        // net.out's only member is http.fetch.
        let net_out: Vec<&str> = CATALOG_ENTRIES
            .iter()
            .filter(|entry| entry.class == CLASS_NET_OUT)
            .map(|entry| entry.name)
            .collect();
        assert_eq!(net_out, ["http.fetch"]);
    }

    #[test]
    fn names_without_a_designed_class_carry_unknown_until_assigned() {
        // Registered after #5999's table was written; no design pass
        // has assigned them a class, so their manifests carry
        // CLASS_UNKNOWN and they always disclose (#5959 §1). This list
        // is the tripwire: assigning a class removes a name here in
        // the same change; registering a new unassigned name fails
        // every_registered_class_id_is_in_the_shipped_table only if it
        // invents an id, so this test is where a new name must be
        // accounted for.
        let unassigned = [
            "kv.get",
            "kv.set",
            "kv.delete",
            "blob.put",
            "blob.get",
            "blob.delete",
            "notify",
            "device.info",
            "form.present",
            "form.result",
            "form.cancel",
            "speech.transcribe",
            "speech.capture",
            "speech.synthesize",
            "client.file.read",
            "client.notify",
        ];
        for name in unassigned {
            let manifest = PROVIDER_REGISTRY.get(name).expect(name);
            assert_eq!(manifest.class, CLASS_UNKNOWN, "{name}");
        }
    }

    #[test]
    fn every_registered_class_id_is_in_the_shipped_table() {
        for entry in CATALOG_ENTRIES {
            assert!(
                cap_class(entry.class).is_some(),
                "{} names class {} which the shipped table does not know",
                entry.name,
                entry.class
            );
            assert_eq!(entry.cap_class().id, entry.class, "{}", entry.name);
        }
    }

    #[test]
    fn unassigned_names_and_unknown_ids_fail_closed() {
        // A capability name nobody registered resolves to unknown.
        assert_eq!(
            PROVIDER_REGISTRY.class_for_name("no.such.capability").id,
            CLASS_UNKNOWN
        );
        // A class id the shipped table does not know resolves to
        // unknown, which discloses.
        assert_eq!(resolve_cap_class("some.future.class").id, CLASS_UNKNOWN);
        assert!(resolve_cap_class(CLASS_UNKNOWN).disclosing());
        // cap.none is the only messageless class in the table.
        assert!(!resolve_cap_class(CLASS_NONE).disclosing());
        for class in CAP_CLASSES {
            assert_eq!(class.disclosing(), class.id != CLASS_NONE, "{}", class.id);
        }
    }
}
