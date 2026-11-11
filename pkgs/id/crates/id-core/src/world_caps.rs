//! Capabilities: what a world program may ask the server for.
//!
//! A program stays a pure function. What it needs from the server is **data
//! it returns** (`wants`, a pure projection of its model) and **events it
//! receives** (delivered through `update` with participant id `0`; accepted
//! live events are journaled so replay feeds the program what it saw). Restore
//! may also synthesize presence-cleanup `left` events for legacy checkpoints
//! whose open-presence set cannot be recovered; those cleanup events can be
//! redundant and are not evidence that a participant was online.
//!
//! ```json
//! {"v":1,
//!  "subscribe":["players","time.tick:5000"],
//!  "requests":[{"id":"r1","cap":"time.now"},
//!              {"id":"r2","cap":"chat.say","args":{"text":"hello"}}]}
//! ```
//!
//! Asking what device the program is rendering on is the same shape —
//! `device.info` (#2338) is a one-shot request whose result is the
//! device capability manifest (taxonomy doc 2026-10-29, §3.1):
//!
//! ```json
//! {"v":1,
//!  "requests":[{"id":"d1","cap":"device.info"}]}
//! ```
//!
//! The server answers with the manifest as the result value:
//! `{"manifest":{"v":1},"display":{...},"input":{...},"sensors":{...},"audio":{...},"network":{...}}`.
//! A program may name the manifest version it understands
//! (`{"id":"d1","cap":"device.info","args":{"v":1}}`); an unsupported
//! version is refused with `bad_args`, never silently interpreted. A
//! host that cannot describe its device refuses with `unknown_cap`.
//!
//! The server answers each new request with
//! `{"cap":"result","id":"r1","ok":true,"value":...}` (or `"ok":false` with an
//! `"error"`), and each subscription with its own events
//! (`{"cap":"time.tick","interval":5000,"now":<unix ms>}`,
//! `{"cap":"players","event":"joined"|"left","participant":{"id":1,"name":"ann"}}`).
//! A `players.left` event removes that participant from the program's online
//! set if present. Legacy checkpoint migration may send it for every known
//! capability because the original online set is unknowable; programs should
//! therefore make this removal idempotent.
//!
//! This module holds the pure parts: the schema and its bounds, the catalog of
//! capability names, and the grant policy with its usage accounting. The
//! actor in [`crate::world`] executes providers because they need the world.
//!
//! ## Refusal codes
//!
//! A refused request or subscription carries a stable wire `"error"` code and,
//! when the refusal has a specific cause, an advisory `"reason"`. Codes are
//! part of the wire contract and never change meaning; the DoS suite asserts
//! them.
//!
//! | Code             | Meaning                                                             |
//! | ---------------- | ------------------------------------------------------------------- |
//! | `cap_denied`     | The world has not granted the capability.                           |
//! | `unknown_cap`    | The server has no such capability.                                  |
//! | `bad_args`       | The request's arguments were not acceptable.                        |
//! | `failed`         | The provider failed.                                                |
//! | `actor_denied`   | The participant whose input caused the request lacks the scope.     |
//! | `outside_ceiling`| The server or the world's capability ceiling excludes it.           |
//! | `result_too_large`| The serialized result event exceeded the maximum event size.       |
//!
//! Location refusals reuse those codes with specific reasons (#2341):
//! `cap_denied` with reason `consent_required` (no per-use consent) or
//! `os_permission_denied` (the OS refused the permission), `outside_ceiling`
//! with reason `precision_ceiling` (the fix is less precise than the
//! ceiling allows), and `failed` with reason `no_fix` (no fix available).
//!
//! **Sensor caps are per-use.** The `loc.*` capabilities are sensor caps:
//! a wildcard (`*`) grant never reaches them and grant-on-use never mints
//! a durable grant for them — a world needs an explicit ceiling entry
//! *and* an explicit grant, and each use still passes the consent seam.
//! [`is_sensor_cap`] names the set; the ledger enforces it.
//!
//! **Local, not signed.** The granted set, the policy and the usage counts
//! ([`CapUsage`]) belong to the hosting node and are journaled there. None of
//! them is signed or replicated. A usage count reports what happened and bounds
//! nothing, so two hosts of one world may keep different counts.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};

/// Version of the `wants` document the host understands.
pub const WANTS_VERSION: u64 = 1;

/// Largest `wants` document, in bytes.
pub const MAX_WANTS_BYTES: usize = 64 * 1024;
/// Most subscriptions and most requests in one `wants` document.
pub const MAX_WANTS_ENTRIES: usize = 64;
/// Longest request id, subscription spec or capability name, in bytes.
pub const MAX_NAME_BYTES: usize = 64;
/// Largest request `args`, serialized, in bytes.
pub const MAX_ARGS_BYTES: usize = 8 * 1024;
/// Largest event the host delivers to a program, in bytes.
pub const MAX_EVENT_BYTES: usize = 64 * 1024;
/// Most distinct tick intervals one program may subscribe to.
pub const MAX_TICK_INTERVALS: usize = 4;
/// Most concurrent `loc.watch` subscriptions one program may hold (#2341).
pub const MAX_LOC_WATCHES: usize = 4;
/// The `loc.watch` interval floor in milliseconds (#2341). A watch may
/// not sample faster than this: GNSS fixes are power-hungry, and a
/// sub-floor interval buys no accuracy — the fix itself takes about
/// this long to refresh on phone and ESP32 GNSS hardware. Hosts should
/// expect a `loc.watch` to dominate a long session's battery budget;
/// programs that only need a position should use one-shot
/// `loc.position` instead.
pub const MIN_LOC_WATCH_INTERVAL_MS: u64 = 5_000;

/// Usage key that aggregates every unknown capability name.
///
/// A guest program controls requested capability names, so recording each
/// distinct unknown name would let it mint one [`CapUsage`] entry (and one
/// [`CapsReport`] entry) per name, forever.
/// Unknown names — including unknown subscription specs, which reach the
/// ledger as their full spec — aggregate here instead, so `usage` holds at
/// most the catalog names plus this one bucket. [`is_known`] decides the
/// split: anything outside [`CATALOG`] lands in this bucket, even a literal
/// request for this key.
pub const UNKNOWN_CAP_USAGE_KEY: &str = "<unknown>";

/// Whether a provider joins the default ceiling — the ceiling a world or a
/// server starts with before anyone narrows or widens it (#838).
///
/// This is the per-provider manifest default the F12.4 registry (#534)
/// carries: registering a provider is not enough to widen any world's
/// authority. A provider joins default ceilings only when its entry says
/// [`DefaultCeiling::Included`] explicitly; a sensitive provider registers
/// as [`DefaultCeiling::Excluded`] and reaches a world only through an
/// explicit ceiling that names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefaultCeiling {
    /// The provider is in every default ceiling.
    Included,
    /// The provider is in no default ceiling; an explicit ceiling must
    /// name it before any world may use it.
    Excluded,
}

/// The kind of provider registered in the catalog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderKind {
    /// A host request in the world's `wants` document.
    Request,
    /// A server event subscription.
    Subscription,
    /// A Roc platform module, checked by the compile gate rather than called
    /// as a host request.
    PlatformModule,
}

/// A JSON value kind accepted by a request field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArgumentType {
    /// UTF-8 JSON string.
    String,
    /// Non-negative JSON integer.
    UnsignedInteger,
}

/// A recognized field in a provider's request arguments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProviderArgument {
    /// Field name.
    pub name: &'static str,
    /// JSON value kind.
    pub value_type: ArgumentType,
    /// Whether the provider requires this field.
    pub required: bool,
}

/// Static description of a provider's arguments or subscription syntax.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArgumentSchema {
    /// A request's recognized JSON object fields. Generic `wants` bounds and
    /// provider validation remain at their existing boundaries in this slice.
    JsonObject(&'static [ProviderArgument]),
    /// A subscription spec, such as `players` or `time.tick:<positive-ms>`.
    Subscription(&'static str),
    /// Platform modules have no host request argument schema.
    NotApplicable,
}

/// A bound already enforced by the current host or protocol implementation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderBound {
    /// Maximum serialized request argument size.
    ArgsBytes(usize),
    /// Maximum count or number of entries.
    Items(usize),
    /// Maximum identifier length in bytes.
    IdentifierBytes(usize),
    /// Maximum response page size in bytes.
    ResponseBytes(usize),
    /// Maximum distance into the future in milliseconds.
    HorizonMillis(u64),
    /// A per-world limit whose value is supplied by `WorldLimits`.
    WorldLimit(&'static str),
}

/// Participant scope required for a provider effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequiredScope {
    /// A joined participant; also the existing fallback for capabilities
    /// without a more specific scope rule.
    Join,
    /// A participant with chat-write authority.
    Chat,
    /// A participant allowed to read chat (`CHAT` or `HISTORY`).
    ChatRead,
}

/// One provider and its versioned manifest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProviderManifest {
    /// The capability or platform-module name.
    pub name: &'static str,
    /// Provider contract version. Existing providers use version 1.
    pub version: u32,
    /// Provider kind (host request, subscription, or platform module).
    pub kind: ProviderKind,
    /// Static argument schema or subscription syntax.
    pub argument_schema: ArgumentSchema,
    /// Provider-specific bounds already enforced by the current implementation.
    pub bounds: &'static [ProviderBound],
    /// Participant scope required for the provider's effect.
    pub required_scope: RequiredScope,
    /// Whether the provider joins default ceilings.
    pub default_ceiling: DefaultCeiling,
}

/// Existing catalog entries as full manifests. The macro generates the
/// compatibility [`CATALOG`] view from this one source of truth.
macro_rules! define_provider_registry {
    ($(
        $name:literal => {
            version: $version:expr,
            kind: $kind:expr,
            argument_schema: $argument_schema:expr,
            bounds: $bounds:expr,
            required_scope: $required_scope:expr,
            default_ceiling: $default_ceiling:expr,
        },
    )+) => {
        /// All statically registered production providers, in stable catalog
        /// order. The registry assembles [`CATALOG_ENTRIES`] from this list so
        /// feature-gated, registry-only providers can join without touching
        /// the production entries.
        pub const PRODUCTION_CATALOG_ENTRIES: &[ProviderManifest] = &[
            $(ProviderManifest {
                name: $name,
                version: $version,
                kind: $kind,
                argument_schema: $argument_schema,
                bounds: $bounds,
                required_scope: $required_scope,
                default_ceiling: $default_ceiling,
            }),+
        ];

        /// Compatibility view of the provider registry's ordered names.
        pub const CATALOG: &[&str] = &[$($name),+];
    };
}

/// Argument schema for providers that take no arguments.
const NO_ARGUMENTS: &[ProviderArgument] = &[];
/// Argument schema for `chat.say`: one required `text` string.
const CHAT_SAY_ARGUMENTS: &[ProviderArgument] = &[ProviderArgument {
    name: "text",
    value_type: ArgumentType::String,
    required: true,
}];
/// Argument schema for `chat.history`: an optional `count`.
const CHAT_HISTORY_ARGUMENTS: &[ProviderArgument] = &[ProviderArgument {
    name: "count",
    value_type: ArgumentType::UnsignedInteger,
    required: false,
}];
/// Argument schema for `records.read`: optional `prefix` and `after`.
const RECORDS_READ_ARGUMENTS: &[ProviderArgument] = &[
    ProviderArgument {
        name: "prefix",
        value_type: ArgumentType::String,
        required: false,
    },
    ProviderArgument {
        name: "after",
        value_type: ArgumentType::String,
        required: false,
    },
];
/// Argument schema for `schedule.at`: a required `id` and `at`.
const SCHEDULE_AT_ARGUMENTS: &[ProviderArgument] = &[
    ProviderArgument {
        name: "id",
        value_type: ArgumentType::String,
        required: true,
    },
    ProviderArgument {
        name: "at",
        value_type: ArgumentType::UnsignedInteger,
        required: true,
    },
];
/// Argument schema for `schedule.cancel`: one required `id` string.
const SCHEDULE_CANCEL_ARGUMENTS: &[ProviderArgument] = &[ProviderArgument {
    name: "id",
    value_type: ArgumentType::String,
    required: true,
}];

/// Bounds shared by simple request providers: the argument byte cap.
const COMMON_REQUEST_BOUNDS: &[ProviderBound] = &[ProviderBound::ArgsBytes(MAX_ARGS_BYTES)];
/// Bounds for `chat.say`: argument bytes plus the world chat byte limit.
const CHAT_SAY_BOUNDS: &[ProviderBound] = &[
    ProviderBound::ArgsBytes(MAX_ARGS_BYTES),
    ProviderBound::WorldLimit("chat_bytes"),
];
/// Bounds for `chat.history`: argument bytes plus the history line cap.
const CHAT_HISTORY_BOUNDS: &[ProviderBound] = &[
    ProviderBound::ArgsBytes(MAX_ARGS_BYTES),
    ProviderBound::Items(MAX_CHAT_HISTORY_LINES),
];
/// Bounds for `records.read`: argument bytes plus the page byte cap.
const RECORDS_READ_BOUNDS: &[ProviderBound] = &[
    ProviderBound::ArgsBytes(MAX_ARGS_BYTES),
    ProviderBound::ResponseBytes(MAX_RECORDS_RESULT_PAGE_BYTES),
];
/// Bounds for `schedule.at`: argument bytes, id bytes, pending cap, and horizon.
const SCHEDULE_AT_BOUNDS: &[ProviderBound] = &[
    ProviderBound::ArgsBytes(MAX_ARGS_BYTES),
    ProviderBound::IdentifierBytes(MAX_SCHEDULE_ID_BYTES),
    ProviderBound::Items(MAX_PENDING_SCHEDULES),
    ProviderBound::HorizonMillis(MAX_SCHEDULE_HORIZON_MS),
];
/// Bounds for `schedule.cancel`: argument bytes plus the id byte cap.
const SCHEDULE_CANCEL_BOUNDS: &[ProviderBound] = &[
    ProviderBound::ArgsBytes(MAX_ARGS_BYTES),
    ProviderBound::IdentifierBytes(MAX_SCHEDULE_ID_BYTES),
];
/// Bounds for tick providers: the interval cap plus the world tick floor.
const TICK_BOUNDS: &[ProviderBound] = &[
    ProviderBound::Items(MAX_TICK_INTERVALS),
    ProviderBound::WorldLimit("min_tick_ms"),
];
/// Argument schema for `loc.position`: an optional precision ceiling in
/// whole metres. A fix less precise than the ceiling is refused with
/// `outside_ceiling` / `precision_ceiling`, never silently degraded.
const LOC_POSITION_ARGUMENTS: &[ProviderArgument] = &[ProviderArgument {
    name: "max_accuracy_m",
    value_type: ArgumentType::UnsignedInteger,
    required: false,
}];
/// Argument schema for `device.info`: an optional manifest version
/// the program understands. An unsupported version is refused rather
/// than silently interpreted (#2338).
const DEVICE_INFO_ARGUMENTS: &[ProviderArgument] = &[ProviderArgument {
    name: "v",
    value_type: ArgumentType::UnsignedInteger,
    required: false,
}];
/// Bounds for `loc.position`: the argument byte cap.
const LOC_POSITION_BOUNDS: &[ProviderBound] = &[ProviderBound::ArgsBytes(MAX_ARGS_BYTES)];
/// Bounds for `loc.watch`: the concurrent-watch cap. The interval
/// floor ([`MIN_LOC_WATCH_INTERVAL_MS`]) is enforced by
/// [`parse_loc_watch_interval_ms`].
const LOC_WATCH_BOUNDS: &[ProviderBound] = &[ProviderBound::Items(MAX_LOC_WATCHES)];

define_provider_registry! {
    "time.now" => {
        version: 1,
        kind: ProviderKind::Request,
        argument_schema: ArgumentSchema::JsonObject(NO_ARGUMENTS),
        bounds: COMMON_REQUEST_BOUNDS,
        required_scope: RequiredScope::Join,
        default_ceiling: DefaultCeiling::Included,
    },
    "time.tick" => {
        version: 1,
        kind: ProviderKind::Subscription,
        argument_schema: ArgumentSchema::Subscription("time.tick:<positive-milliseconds>"),
        bounds: TICK_BOUNDS,
        required_scope: RequiredScope::Join,
        default_ceiling: DefaultCeiling::Included,
    },
    "random.u64" => {
        version: 1,
        kind: ProviderKind::Request,
        argument_schema: ArgumentSchema::JsonObject(NO_ARGUMENTS),
        bounds: COMMON_REQUEST_BOUNDS,
        required_scope: RequiredScope::Join,
        default_ceiling: DefaultCeiling::Included,
    },
    "players" => {
        version: 1,
        kind: ProviderKind::Subscription,
        argument_schema: ArgumentSchema::Subscription("players"),
        bounds: &[],
        required_scope: RequiredScope::Join,
        default_ceiling: DefaultCeiling::Included,
    },
    "players.list" => {
        version: 1,
        kind: ProviderKind::Request,
        argument_schema: ArgumentSchema::JsonObject(NO_ARGUMENTS),
        bounds: COMMON_REQUEST_BOUNDS,
        required_scope: RequiredScope::Join,
        default_ceiling: DefaultCeiling::Included,
    },
    "chat.say" => {
        version: 1,
        kind: ProviderKind::Request,
        argument_schema: ArgumentSchema::JsonObject(CHAT_SAY_ARGUMENTS),
        bounds: CHAT_SAY_BOUNDS,
        required_scope: RequiredScope::Chat,
        default_ceiling: DefaultCeiling::Included,
    },
    "chat.history" => {
        version: 1,
        kind: ProviderKind::Request,
        argument_schema: ArgumentSchema::JsonObject(CHAT_HISTORY_ARGUMENTS),
        bounds: CHAT_HISTORY_BOUNDS,
        required_scope: RequiredScope::ChatRead,
        default_ceiling: DefaultCeiling::Included,
    },
    "records.read" => {
        version: 1,
        kind: ProviderKind::Request,
        argument_schema: ArgumentSchema::JsonObject(RECORDS_READ_ARGUMENTS),
        bounds: RECORDS_READ_BOUNDS,
        required_scope: RequiredScope::Join,
        default_ceiling: DefaultCeiling::Included,
    },
    "schedule.at" => {
        version: 1,
        kind: ProviderKind::Request,
        argument_schema: ArgumentSchema::JsonObject(SCHEDULE_AT_ARGUMENTS),
        bounds: SCHEDULE_AT_BOUNDS,
        required_scope: RequiredScope::Join,
        default_ceiling: DefaultCeiling::Included,
    },
    "schedule.cancel" => {
        version: 1,
        kind: ProviderKind::Request,
        argument_schema: ArgumentSchema::JsonObject(SCHEDULE_CANCEL_ARGUMENTS),
        bounds: SCHEDULE_CANCEL_BOUNDS,
        required_scope: RequiredScope::Join,
        default_ceiling: DefaultCeiling::Included,
    },
    "world.info" => {
        version: 1,
        kind: ProviderKind::Request,
        argument_schema: ArgumentSchema::JsonObject(NO_ARGUMENTS),
        bounds: COMMON_REQUEST_BOUNDS,
        required_scope: RequiredScope::Join,
        default_ceiling: DefaultCeiling::Included,
    },
    "device.info" => {
        version: 1,
        kind: ProviderKind::Request,
        argument_schema: ArgumentSchema::JsonObject(DEVICE_INFO_ARGUMENTS),
        bounds: COMMON_REQUEST_BOUNDS,
        required_scope: RequiredScope::Join,
        default_ceiling: DefaultCeiling::Included,
    },
    // Sensor caps (#2341): excluded from default ceilings, so a world
    // reaches them only through an explicit ceiling entry and an
    // explicit grant, and each use passes the consent seam.
    "loc.position" => {
        version: 1,
        kind: ProviderKind::Request,
        argument_schema: ArgumentSchema::JsonObject(LOC_POSITION_ARGUMENTS),
        bounds: LOC_POSITION_BOUNDS,
        required_scope: RequiredScope::Join,
        default_ceiling: DefaultCeiling::Excluded,
    },
    "loc.watch" => {
        version: 1,
        kind: ProviderKind::Subscription,
        argument_schema: ArgumentSchema::Subscription("loc.watch:<positive-milliseconds>"),
        bounds: LOC_WATCH_BOUNDS,
        required_scope: RequiredScope::Join,
        default_ceiling: DefaultCeiling::Excluded,
    },
    "Key" => {
        version: 1,
        kind: ProviderKind::PlatformModule,
        argument_schema: ArgumentSchema::NotApplicable,
        bounds: &[],
        required_scope: RequiredScope::Join,
        default_ceiling: DefaultCeiling::Included,
    },
    "Screen" => {
        version: 1,
        kind: ProviderKind::PlatformModule,
        argument_schema: ArgumentSchema::NotApplicable,
        bounds: &[],
        required_scope: RequiredScope::Join,
        default_ceiling: DefaultCeiling::Included,
    },
    "RocJson" => {
        version: 1,
        kind: ProviderKind::PlatformModule,
        argument_schema: ArgumentSchema::NotApplicable,
        bounds: &[],
        required_scope: RequiredScope::Join,
        default_ceiling: DefaultCeiling::Included,
    },
    "RocGrid" => {
        version: 1,
        kind: ProviderKind::PlatformModule,
        argument_schema: ArgumentSchema::NotApplicable,
        bounds: &[],
        required_scope: RequiredScope::Join,
        default_ceiling: DefaultCeiling::Included,
    },
    "RocCap" => {
        version: 1,
        kind: ProviderKind::PlatformModule,
        argument_schema: ArgumentSchema::NotApplicable,
        bounds: &[],
        required_scope: RequiredScope::Join,
        default_ceiling: DefaultCeiling::Included,
    },
    "RocText" => {
        version: 1,
        kind: ProviderKind::PlatformModule,
        argument_schema: ArgumentSchema::NotApplicable,
        bounds: &[],
        required_scope: RequiredScope::Join,
        default_ceiling: DefaultCeiling::Included,
    },
}

/// Name of the registry-only test provider (#1061 exit proof).
///
/// Only present with the `test-provider` feature, which production builds
/// never enable. It is a plain platform-module name so the compile gate can
/// recognize an `import` of it as a module import.
#[cfg(feature = "test-provider")]
pub const TEST_PROVIDER_NAME: &str = "RegistryProbe";

/// The registry-only test provider (#1061 exit proof).
///
/// It registers exactly like a production provider — one manifest entry — and
/// deliberately opts out of every default ceiling: adding a provider must
/// never widen a world's authority by itself (#838).
#[cfg(feature = "test-provider")]
const TEST_PROVIDER_MANIFEST: ProviderManifest = ProviderManifest {
    name: TEST_PROVIDER_NAME,
    version: 1,
    kind: ProviderKind::PlatformModule,
    argument_schema: ArgumentSchema::NotApplicable,
    bounds: &[],
    required_scope: RequiredScope::Join,
    default_ceiling: DefaultCeiling::Excluded,
};

/// Registry-only entries added on top of [`PRODUCTION_CATALOG_ENTRIES`].
#[cfg(feature = "test-provider")]
const EXTRA_CATALOG_ENTRIES: &[ProviderManifest] = &[TEST_PROVIDER_MANIFEST];
/// Registry-only entries added on top of [`PRODUCTION_CATALOG_ENTRIES`].
#[cfg(not(feature = "test-provider"))]
const EXTRA_CATALOG_ENTRIES: &[ProviderManifest] = &[];

/// Number of manifests in the assembled provider registry.
const CATALOG_ENTRY_COUNT: usize = PRODUCTION_CATALOG_ENTRIES.len() + EXTRA_CATALOG_ENTRIES.len();

/// Assemble the production manifests plus any feature-gated registry-only
/// additions into one array.
///
/// This is the only place the registry grows. Adding a provider means adding a
/// manifest — never editing dispatch or core files (#1061).
const fn assemble_catalog_entries() -> [ProviderManifest; CATALOG_ENTRY_COUNT] {
    let mut entries = [PRODUCTION_CATALOG_ENTRIES[0]; CATALOG_ENTRY_COUNT];
    let mut index = 0;
    while index < PRODUCTION_CATALOG_ENTRIES.len() {
        entries[index] = PRODUCTION_CATALOG_ENTRIES[index];
        index += 1;
    }
    let mut extra = 0;
    while extra < EXTRA_CATALOG_ENTRIES.len() {
        entries[PRODUCTION_CATALOG_ENTRIES.len() + extra] = EXTRA_CATALOG_ENTRIES[extra];
        extra += 1;
    }
    entries
}

/// The assembled registry storage, in stable catalog order.
static ASSEMBLED_CATALOG_ENTRIES: [ProviderManifest; CATALOG_ENTRY_COUNT] =
    assemble_catalog_entries();

/// All statically registered providers, in stable catalog order.
pub const CATALOG_ENTRIES: &[ProviderManifest] = &ASSEMBLED_CATALOG_ENTRIES;

/// A registry view over the statically registered provider manifests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProviderRegistry {
    entries: &'static [ProviderManifest],
}

impl ProviderRegistry {
    /// Construct a registry over static provider manifests.
    #[must_use]
    pub const fn new(entries: &'static [ProviderManifest]) -> Self {
        Self { entries }
    }

    /// Every provider in registration order.
    #[must_use]
    pub const fn entries(&self) -> &'static [ProviderManifest] {
        self.entries
    }

    /// Look up a provider by its stable name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&'static ProviderManifest> {
        self.entries.iter().find(|entry| entry.name == name)
    }

    /// Provider names in stable registration order.
    #[must_use]
    pub fn ordered_names(&self) -> Vec<&'static str> {
        self.entries.iter().map(|entry| entry.name).collect()
    }

    /// Every provider name, as a set.
    #[must_use]
    pub fn catalog_names(&self) -> BTreeSet<String> {
        self.entries
            .iter()
            .map(|entry| entry.name.to_owned())
            .collect()
    }

    /// Names whose manifest opts into the default ceiling.
    #[must_use]
    pub fn default_ceiling_names(&self) -> BTreeSet<String> {
        default_ceiling_from(self.entries)
    }
}

/// The process-wide provider registry.
pub const PROVIDER_REGISTRY: ProviderRegistry = ProviderRegistry::new(CATALOG_ENTRIES);

/// Compatibility name for the former catalog entries.
pub type CatalogEntry = ProviderManifest;

/// The default ceiling a set of catalog entries produces: exactly the
/// entries whose manifest default is [`DefaultCeiling::Included`].
#[must_use]
pub fn default_ceiling_from(entries: &[ProviderManifest]) -> BTreeSet<String> {
    entries
        .iter()
        .filter(|entry| entry.default_ceiling == DefaultCeiling::Included)
        .map(|entry| entry.name.to_owned())
        .collect()
}

/// The default ceiling for worlds and servers: the providers whose
/// manifest default includes them. A provider registered later joins this
/// set only by an explicit `Included` default — never by registration
/// alone (#838).
#[must_use]
pub fn default_ceiling_names() -> BTreeSet<String> {
    PROVIDER_REGISTRY.default_ceiling_names()
}

/// Most chat lines one `chat.history` request may return.
pub const MAX_CHAT_HISTORY_LINES: usize = 64;

/// Most schedules one world may hold pending at once.
pub const MAX_PENDING_SCHEDULES: usize = 64;
/// Longest schedule id, in bytes. Ids are printable ASCII.
pub const MAX_SCHEDULE_ID_BYTES: usize = 64;
/// How far past the host clock a schedule may be set, in days.
pub const MAX_SCHEDULE_HORIZON_DAYS: u64 = 30;
/// How far past the host clock a schedule may be set, in milliseconds.
pub const MAX_SCHEDULE_HORIZON_MS: u64 = MAX_SCHEDULE_HORIZON_DAYS * 24 * 60 * 60 * 1000;
/// Most schedules delivered in one wakeup. Any left due fire on the next.
pub const MAX_SCHEDULE_FIRES_PER_WAKEUP: usize = 64;
/// Largest page of records served over the session protocol and the
/// host view, in bytes. Cap results are not served this way: a
/// `records.read` result is delivered through `result_event` and is
/// bounded by [`MAX_RECORDS_RESULT_PAGE_BYTES`] instead (#4888).
pub const MAX_RECORDS_PAGE_BYTES: usize = 768 * 1024;

/// Bytes of [`MAX_EVENT_BYTES`] reserved for the `result_event`
/// envelope around a `records.read` value: the outer
/// `{"cap":"result","id":...,"ok":true,"value":...}` object (request
/// ids are at most [`MAX_NAME_BYTES`] long), the value's own
/// `{"records":[...],"next_after":...}` wrapper, and a serialized
/// `next_after` key (record keys are at most 256 bytes, and JSON
/// escaping at most doubles that, plus the quotes). The fixed parts
/// measure under 200 bytes and the escaped key under 514, so 1024
/// leaves slack rather than sitting on the edge.
pub const RESULT_EVENT_ENVELOPE_BYTES: usize = 1024;

/// Largest page of records a `records.read` cap result may carry, in
/// bytes of serialized `{"key":...,"value":...}` entries (plus one
/// array comma each). Derived from the event limit minus the envelope
/// allowance, so a page admitted under this bound always fits the
/// `result_event` that delivers it: it can never be replaced by a
/// `result_too_large` error (#4888).
pub const MAX_RECORDS_RESULT_PAGE_BYTES: usize = MAX_EVENT_BYTES - RESULT_EVENT_ENVELOPE_BYTES;

/// Check a schedule id: 1 to [`MAX_SCHEDULE_ID_BYTES`] bytes, all printable
/// ASCII (space through `~`), so no control characters reach the journal.
///
/// # Errors
///
/// Names the rule the id breaks.
pub fn validate_schedule_id(id: &str) -> Result<()> {
    ensure!(
        !id.is_empty() && id.len() <= MAX_SCHEDULE_ID_BYTES,
        "schedule id must be 1 to {MAX_SCHEDULE_ID_BYTES} bytes"
    );
    ensure!(
        id.bytes().all(|byte| (0x20..=0x7e).contains(&byte)),
        "schedule id must be printable ASCII"
    );
    Ok(())
}

/// Whether `name` is a capability the server can provide.
#[must_use]
pub fn is_known(name: &str) -> bool {
    PROVIDER_REGISTRY.get(name).is_some()
}

/// Every catalog capability, as a set. This is the widest ceiling a
/// world or server can be given explicitly; the *default* ceiling is the
/// narrower [`default_ceiling_names`].
#[must_use]
pub fn catalog_names() -> BTreeSet<String> {
    PROVIDER_REGISTRY.catalog_names()
}

/// One thing a program asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    /// Program-chosen id, unique within one `wants` document.
    pub id: String,
    /// Capability name.
    pub cap: String,
    /// Arguments (a JSON object, or `null`).
    pub args: serde_json::Value,
}

/// A subscription a program asked for.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Subscription {
    /// Join/leave events.
    Players,
    /// A tick event every this many milliseconds while someone is connected.
    Tick(u64),
    /// A spec the host does not understand.
    Unknown(String),
}

impl Subscription {
    /// Parse `players` or `time.tick:<ms>`.
    #[must_use]
    pub fn parse(spec: &str) -> Self {
        if spec == "players" {
            return Self::Players;
        }
        if let Some(ms) = spec.strip_prefix("time.tick:")
            && let Ok(ms) = ms.parse::<u64>()
            && ms > 0
        {
            return Self::Tick(ms);
        }
        Self::Unknown(spec.to_owned())
    }

    /// The capability that must be granted for this subscription.
    #[must_use]
    pub fn cap(&self) -> &str {
        match self {
            Self::Players => "players",
            Self::Tick(_) => "time.tick",
            Self::Unknown(spec) => spec,
        }
    }

    /// The spec as the program wrote it.
    #[must_use]
    pub fn spec(&self) -> String {
        match self {
            Self::Players => "players".to_owned(),
            Self::Tick(ms) => format!("time.tick:{ms}"),
            Self::Unknown(spec) => spec.clone(),
        }
    }
}

/// What a program currently wants from the server.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Wants {
    /// Subscriptions, deduplicated.
    pub subscribe: BTreeSet<Subscription>,
    /// Requests in the order the program listed them.
    pub requests: Vec<Request>,
}

impl Wants {
    /// Every capability this document needs, known or not.
    #[must_use]
    pub fn caps(&self) -> BTreeSet<String> {
        self.subscribe
            .iter()
            .map(|sub| sub.cap().to_owned())
            .chain(self.requests.iter().map(|request| request.cap.clone()))
            .collect()
    }
}

/// Parse and bound a `wants` document.
///
/// # Errors
///
/// Anything outside the schema or its bounds. The caller treats that as a
/// program failure, like invalid records.
pub fn parse_wants(text: &str) -> Result<Wants> {
    ensure!(
        text.len() <= MAX_WANTS_BYTES,
        "wants exceeds {MAX_WANTS_BYTES} bytes"
    );
    // A duplicate key is a parse error (#7621, the #6520 rule): the
    // version check and the entry bounds below must see the document as
    // written, never a last-wins collapse of it.
    let value: serde_json::Value =
        crate::json_unique::parse_json_without_duplicate_keys(text).context("wants is not JSON")?;
    let object = value.as_object().context("wants must be a JSON object")?;
    if let Some(version) = object.get("v") {
        ensure!(
            version.as_u64() == Some(WANTS_VERSION),
            "unsupported wants version {version}"
        );
    }
    let mut wants = Wants::default();
    if let Some(subscribe) = object.get("subscribe") {
        let list = subscribe
            .as_array()
            .context("wants.subscribe must be an array")?;
        ensure!(
            list.len() <= MAX_WANTS_ENTRIES,
            "wants has more than {MAX_WANTS_ENTRIES} subscriptions"
        );
        for spec in list {
            let spec = spec
                .as_str()
                .context("wants.subscribe entries must be strings")?;
            ensure!(
                !spec.is_empty() && spec.len() <= MAX_NAME_BYTES,
                "subscription spec must be 1 to {MAX_NAME_BYTES} bytes"
            );
            wants.subscribe.insert(Subscription::parse(spec));
        }
    }
    if let Some(requests) = object.get("requests") {
        let list = requests
            .as_array()
            .context("wants.requests must be an array")?;
        ensure!(
            list.len() <= MAX_WANTS_ENTRIES,
            "wants has more than {MAX_WANTS_ENTRIES} requests"
        );
        let mut seen = BTreeSet::new();
        for entry in list {
            let entry = entry.as_object().context("a request must be an object")?;
            let id = entry
                .get("id")
                .and_then(serde_json::Value::as_str)
                .context("a request needs a string id")?;
            let cap = entry
                .get("cap")
                .and_then(serde_json::Value::as_str)
                .context("a request needs a string cap")?;
            ensure!(
                !id.is_empty() && id.len() <= MAX_NAME_BYTES && !id.chars().any(char::is_control),
                "request id must be 1 to {MAX_NAME_BYTES} bytes without control characters"
            );
            ensure!(
                !cap.is_empty() && cap.len() <= MAX_NAME_BYTES,
                "request cap must be 1 to {MAX_NAME_BYTES} bytes"
            );
            ensure!(seen.insert(id.to_owned()), "duplicate request id {id:?}");
            let args = entry
                .get("args")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            // A request's args are a JSON object or absent/null (#7929):
            // handlers read named fields from them, so any other JSON
            // value silently collapsed to "no arguments" — for
            // records.read that widened the read to every record.
            ensure!(
                args.is_null() || args.is_object(),
                "request args must be a JSON object or null, not {}",
                match &args {
                    serde_json::Value::Number(_) => "a number",
                    serde_json::Value::String(_) => "a string",
                    serde_json::Value::Array(_) => "an array",
                    serde_json::Value::Bool(_) => "a boolean",
                    _ => "a non-object value",
                }
            );
            ensure!(
                serde_json::to_vec(&args).map_or(usize::MAX, |bytes| bytes.len()) <= MAX_ARGS_BYTES,
                "request args exceed {MAX_ARGS_BYTES} bytes"
            );
            wants.requests.push(Request {
                id: id.to_owned(),
                cap: cap.to_owned(),
                args,
            });
        }
    }
    let ticks = wants
        .subscribe
        .iter()
        .filter(|sub| matches!(sub, Subscription::Tick(_)))
        .count();
    ensure!(
        ticks <= MAX_TICK_INTERVALS,
        "at most {MAX_TICK_INTERVALS} tick intervals"
    );
    Ok(wants)
}

/// The event delivered for a finished request.
#[must_use]
pub fn result_event(id: &str, outcome: Result<serde_json::Value, CapError>) -> String {
    let event = match outcome {
        Ok(value) => serde_json::json!({"cap":"result","id":id,"ok":true,"value":value}),
        Err(error) => {
            let mut event =
                serde_json::json!({"cap":"result","id":id,"ok":false,"error":error.code()});
            if let Some(reason) = error.reason() {
                event["reason"] = serde_json::json!(reason);
            }
            event
        }
    };
    let text = event.to_string();
    if text.len() > MAX_EVENT_BYTES {
        return serde_json::json!({"cap":"result","id":id,"ok":false,"error":CapError::result_too_large().code()})
            .to_string();
    }
    text
}

/// The event delivered when a subscription is refused.
#[must_use]
pub fn subscription_refused_event(spec: &str, error: &CapError) -> String {
    let mut event =
        serde_json::json!({"cap":"subscription","spec":spec,"ok":false,"error":error.code()});
    if let Some(reason) = error.reason() {
        event["reason"] = serde_json::json!(reason);
    }
    event.to_string()
}

/// Why a request or subscription was refused.
///
/// Every refusal carries a stable [`CapErrorCode`] and, where the refusal has
/// one, a human-readable reason. Codes are part of the wire contract and never
/// change meaning; reasons are advisory and describe the specific refusal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapError {
    code: CapErrorCode,
    reason: Option<String>,
}

/// The stable wire code of a refused request or subscription.
///
/// These are documented in the capability guide and asserted by the DoS
/// suite; adding a code changes the wire contract, so new refusals reuse an
/// existing code before a new one is considered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapErrorCode {
    /// The world has not granted the capability.
    Denied,
    /// The server has no such capability.
    Unknown,
    /// The request's arguments were not acceptable.
    BadArgs,
    /// The provider failed.
    Failed,
    /// The participant whose input caused the request lacks the scope for it.
    ActorDenied,
    /// The server or the world's capability ceiling excludes it, even if granted.
    OutsideCeiling,
    /// The serialized result event exceeded the maximum event size.
    ResultTooLarge,
}

impl CapError {
    /// The capability was not granted.
    #[must_use]
    pub const fn denied() -> Self {
        Self {
            code: CapErrorCode::Denied,
            reason: None,
        }
    }

    /// The server has no such capability.
    #[must_use]
    pub const fn unknown() -> Self {
        Self {
            code: CapErrorCode::Unknown,
            reason: None,
        }
    }

    /// The request's arguments were not acceptable.
    #[must_use]
    pub const fn bad_args() -> Self {
        Self {
            code: CapErrorCode::BadArgs,
            reason: None,
        }
    }

    /// The request's arguments were not acceptable, and `reason` says why.
    #[must_use]
    pub fn bad_args_with(reason: impl Into<String>) -> Self {
        Self {
            code: CapErrorCode::BadArgs,
            reason: Some(reason.into()),
        }
    }

    /// The capability was not granted, and `reason` says why.
    #[must_use]
    pub fn denied_with(reason: impl Into<String>) -> Self {
        Self {
            code: CapErrorCode::Denied,
            reason: Some(reason.into()),
        }
    }

    /// A ceiling excludes the capability, and `reason` says why.
    #[must_use]
    pub fn outside_ceiling_with(reason: impl Into<String>) -> Self {
        Self {
            code: CapErrorCode::OutsideCeiling,
            reason: Some(reason.into()),
        }
    }

    /// The provider failed, and `reason` says why.
    #[must_use]
    pub fn failed_with(reason: impl Into<String>) -> Self {
        Self {
            code: CapErrorCode::Failed,
            reason: Some(reason.into()),
        }
    }

    /// Location use without per-use consent (#2341): `cap_denied`.
    #[must_use]
    pub fn consent_required() -> Self {
        Self::denied_with("consent_required")
    }

    /// The OS refused the location permission (#2341): `cap_denied`.
    #[must_use]
    pub fn os_permission_denied() -> Self {
        Self::denied_with("os_permission_denied")
    }

    /// The available fix is less precise than the precision ceiling
    /// allows (#2341): `outside_ceiling`.
    #[must_use]
    pub fn precision_ceiling() -> Self {
        Self::outside_ceiling_with("precision_ceiling")
    }

    /// No location fix is available (#2341): `failed`.
    #[must_use]
    pub fn no_fix() -> Self {
        Self::failed_with("no_fix")
    }

    /// The provider failed.
    #[must_use]
    pub const fn failed() -> Self {
        Self {
            code: CapErrorCode::Failed,
            reason: None,
        }
    }

    /// The acting participant lacks the scope for the effect.
    #[must_use]
    pub const fn actor_denied() -> Self {
        Self {
            code: CapErrorCode::ActorDenied,
            reason: None,
        }
    }

    /// The acting participant lacks the scope for the effect, and `reason`
    /// says why.
    #[must_use]
    pub fn actor_denied_with(reason: impl Into<String>) -> Self {
        Self {
            code: CapErrorCode::ActorDenied,
            reason: Some(reason.into()),
        }
    }

    /// A ceiling excludes the capability, even if granted.
    #[must_use]
    pub const fn outside_ceiling() -> Self {
        Self {
            code: CapErrorCode::OutsideCeiling,
            reason: None,
        }
    }

    /// The serialized result event exceeded the maximum event size.
    #[must_use]
    pub const fn result_too_large() -> Self {
        Self {
            code: CapErrorCode::ResultTooLarge,
            reason: None,
        }
    }

    /// The code a program sees.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self.code {
            CapErrorCode::Denied => "cap_denied",
            CapErrorCode::Unknown => "unknown_cap",
            CapErrorCode::BadArgs => "bad_args",
            CapErrorCode::Failed => "failed",
            CapErrorCode::ActorDenied => "actor_denied",
            CapErrorCode::OutsideCeiling => "outside_ceiling",
            CapErrorCode::ResultTooLarge => "result_too_large",
        }
    }

    /// The advisory reason, if the refusal has one.
    #[must_use]
    pub const fn reason(&self) -> Option<&String> {
        self.reason.as_ref()
    }
}

/// Whether `cap` is a sensor capability (#2341). Sensor caps are
/// per-use by default: a wildcard grant never reaches them, and
/// grant-on-use never mints a durable grant for them. Today the sensor
/// caps are the `loc.*` family; naming the prefix here keeps the ledger
/// rule in one place when more sensor families land.
#[must_use]
pub fn is_sensor_cap(cap: &str) -> bool {
    cap.starts_with("loc.")
}

/// Where a location reading came from (#2341). The source is part of
/// the reading: an imprecise source is named, never silently degraded
/// to look like a precise one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocationSource {
    /// Satellite positioning (phone GNSS, ESP32 GNSS module).
    Gnss,
    /// Network positioning (Wi-Fi / cell).
    Network,
    /// IP geolocation — kilometre-scale accuracy, named as such.
    Ip,
    /// The caller supplied the fix itself.
    CallerSupplied,
}

impl LocationSource {
    /// The wire name of the source.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Gnss => "gnss",
            Self::Network => "network",
            Self::Ip => "ip",
            Self::CallerSupplied => "caller_supplied",
        }
    }

    /// Parse a wire name, if it is one.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "gnss" => Some(Self::Gnss),
            "network" => Some(Self::Network),
            "ip" => Some(Self::Ip),
            "caller_supplied" => Some(Self::CallerSupplied),
            _ => None,
        }
    }
}

/// One location reading (#2341): the typed shape every `loc.position`
/// result and `loc.watch` delivery carries.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LocationReading {
    /// Latitude in degrees, -90..=90.
    pub lat: f64,
    /// Longitude in degrees, -180..=180.
    pub lon: f64,
    /// Accuracy radius in metres; smaller is more precise.
    pub accuracy_m: f64,
    /// Age of the fix in milliseconds when delivered.
    pub fix_age_ms: u64,
    /// Where the fix came from.
    pub source: LocationSource,
}

impl LocationReading {
    /// Validate the reading's ranges. A reading that fails is a
    /// provider bug, not data a program should ever see.
    ///
    /// # Errors
    ///
    /// Names the first out-of-range field.
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.lat.is_finite() && (-90.0..=90.0).contains(&self.lat),
            "lat out of range"
        );
        ensure!(
            self.lon.is_finite() && (-180.0..=180.0).contains(&self.lon),
            "lon out of range"
        );
        ensure!(
            self.accuracy_m.is_finite() && self.accuracy_m >= 0.0,
            "accuracy_m out of range"
        );
        Ok(())
    }

    /// The wire object: exactly `{lat, lon, accuracy_m, fix_age_ms, source}`.
    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "lat": self.lat,
            "lon": self.lon,
            "accuracy_m": self.accuracy_m,
            "fix_age_ms": self.fix_age_ms,
            "source": self.source.as_str(),
        })
    }

    /// Refuse the reading when it is less precise than the precision
    /// ceiling allows: an imprecise fix must be refusable rather than
    /// plausible-looking (#2341).
    ///
    /// # Errors
    ///
    /// [`CapError::precision_ceiling`] when `accuracy_m` exceeds the
    /// ceiling.
    pub fn refuse_beyond_ceiling(&self, ceiling_m: f64) -> Result<(), CapError> {
        if self.accuracy_m > ceiling_m {
            return Err(CapError::precision_ceiling());
        }
        Ok(())
    }
}

/// The manifest version this host speaks for `device.info` (#2338).
/// A manifest — or a request — naming any other version is refused,
/// never silently interpreted.
pub const DEVICE_MANIFEST_VERSION: u64 = 1;

/// One device's capability manifest (#2338): the typed shape every
/// `device.info` result carries, in the v1 form the capability
/// taxonomy (2026-10-29, §3.1) specifies. The five sections are opaque
/// JSON objects on purpose: display kinds, sensor sets and transports
/// vary per device family, and the wire contract is the section
/// presence plus the version, not an enumeration of affordances.
///
/// The manifest is per-device, not per-program: the adapter boundary
/// enumerates the device once and caches the manifest, and the world
/// actor serves that cached copy to whoever asks (taxonomy §8 open
/// question 4). Enumeration never happens per request.
#[derive(Clone, Debug, PartialEq)]
pub struct DeviceManifest {
    /// Display affordances object (e.g. `{"kind":"pixel","width":128}`).
    pub display: serde_json::Value,
    /// Input affordances object (e.g. `{"keys":8,"encoder":true}`).
    pub input: serde_json::Value,
    /// Sensor affordances object (e.g. `{"gnss":{"accuracy_m":5}}`).
    pub sensors: serde_json::Value,
    /// Audio affordances object (e.g. `{"out":"pcm","in":null}`).
    pub audio: serde_json::Value,
    /// Network affordances object (e.g. `{"transport":"mqtt"}`).
    pub network: serde_json::Value,
}

impl DeviceManifest {
    /// The wire object: exactly
    /// `{"manifest":{"v":1},"display":...,"input":...,"sensors":...,"audio":...,"network":...}`.
    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "manifest": {"v": DEVICE_MANIFEST_VERSION},
            "display": self.display,
            "input": self.input,
            "sensors": self.sensors,
            "audio": self.audio,
            "network": self.network,
        })
    }

    /// Parse and validate a manifest in wire form. The oracle is the
    /// declared v1 manifest contract: `manifest.v` must equal
    /// [`DEVICE_MANIFEST_VERSION`] and every section must be present
    /// as a JSON object.
    ///
    /// # Errors
    ///
    /// [`CapError::bad_args`] naming the defect: an unsupported
    /// manifest version, a missing section, or a section that is not
    /// an object.
    pub fn from_json(value: &serde_json::Value) -> Result<Self, CapError> {
        let version = value
            .get("manifest")
            .and_then(|manifest| manifest.get("v"))
            .and_then(serde_json::Value::as_u64);
        match version {
            Some(DEVICE_MANIFEST_VERSION) => {}
            Some(_) => {
                return Err(CapError::bad_args_with(
                    "unsupported device manifest version",
                ));
            }
            None => {
                return Err(CapError::bad_args_with(
                    "device manifest carries no version",
                ));
            }
        }
        let section = |name: &str| -> Result<serde_json::Value, CapError> {
            match value.get(name) {
                Some(section) if section.is_object() => Ok(section.clone()),
                Some(_) => Err(CapError::bad_args_with(format!(
                    "device manifest section {name} is not an object"
                ))),
                None => Err(CapError::bad_args_with(format!(
                    "device manifest is missing section {name}"
                ))),
            }
        };
        Ok(Self {
            display: section("display")?,
            input: section("input")?,
            sensors: section("sensors")?,
            audio: section("audio")?,
            network: section("network")?,
        })
    }

    /// The manifest version a `device.info` request asks for: the
    /// optional `v` argument, defaulting to
    /// [`DEVICE_MANIFEST_VERSION`]. A program asking for a version
    /// this host does not speak is refused, never silently served a
    /// manifest it may misinterpret.
    ///
    /// # Errors
    ///
    /// [`CapError::bad_args`] for a non-integer or unsupported `v`.
    pub fn requested_version(args: &serde_json::Value) -> Result<u64, CapError> {
        let Some(version) = args.get("v") else {
            return Ok(DEVICE_MANIFEST_VERSION);
        };
        let Some(version) = version.as_u64() else {
            return Err(CapError::bad_args_with(
                "device.info v is not an unsigned integer",
            ));
        };
        if version != DEVICE_MANIFEST_VERSION {
            return Err(CapError::bad_args_with(
                "unsupported device manifest version",
            ));
        }
        Ok(version)
    }
}

/// Per-use consent for a sensor capability (#2341). Consent is a seam
/// the host fills in (browser permission prompt, phone OS permission,
/// device policy); the pure core only maps the state to the refusal
/// vocabulary so every host refuses the same way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocationConsent {
    /// Consent granted for this use.
    Granted,
    /// No consent on record: refuse with `consent_required`.
    ConsentRequired,
    /// The OS refused the permission: refuse with `os_permission_denied`.
    OsPermissionDenied,
}

impl LocationConsent {
    /// The refusal for this consent state, or `None` when use may proceed.
    #[must_use]
    pub fn refusal(&self) -> Option<CapError> {
        match self {
            Self::Granted => None,
            Self::ConsentRequired => Some(CapError::consent_required()),
            Self::OsPermissionDenied => Some(CapError::os_permission_denied()),
        }
    }
}

/// Parse a `loc.watch` subscription spec (#2341): `loc.watch` alone
/// takes the interval floor, `loc.watch:<milliseconds>` names an
/// interval at or above the floor.
///
/// # Errors
///
/// [`CapError::bad_args`] for a foreign spec, a non-numeric interval,
/// or an interval below [`MIN_LOC_WATCH_INTERVAL_MS`].
pub fn parse_loc_watch_interval_ms(spec: &str) -> Result<u64, CapError> {
    let Some(rest) = spec.strip_prefix("loc.watch") else {
        return Err(CapError::bad_args_with("not a loc.watch spec"));
    };
    if rest.is_empty() {
        return Ok(MIN_LOC_WATCH_INTERVAL_MS);
    }
    let Some(ms) = rest.strip_prefix(':') else {
        return Err(CapError::bad_args_with("not a loc.watch spec"));
    };
    let ms: u64 = ms
        .parse()
        .map_err(|_| CapError::bad_args_with("loc.watch interval is not a number"))?;
    if ms < MIN_LOC_WATCH_INTERVAL_MS {
        return Err(CapError::bad_args_with(
            "loc.watch interval below the floor",
        ));
    }
    Ok(ms)
}

/// The event that closes an open `loc.watch` when consent is withdrawn
/// (#2341): exactly
/// `{"cap":"loc.watch","event":"revoked","reason":"consent_withdrawn"}`.
#[must_use]
pub fn loc_watch_revocation_event() -> String {
    serde_json::json!({"cap":"loc.watch","event":"revoked","reason":"consent_withdrawn"})
        .to_string()
}

/// A `loc.watch` delivery event: the reading under the `loc.watch` cap.
#[must_use]
pub fn loc_watch_reading_event(reading: &LocationReading) -> String {
    let mut event = reading.to_json();
    event["cap"] = serde_json::json!("loc.watch");
    event.to_string()
}

/// How a capability was used since the world opened.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapUsage {
    /// Uses the world allowed.
    pub allowed: u64,
    /// Uses refused (not granted, or not a known capability).
    pub denied: u64,
}

/// What a world has granted, what the ceilings allow, and what its program has
/// asked for.
///
/// A use is allowed only when the capability is allowed **and** granted. `*` in
/// the grant set counts as every allowed name — except sensor caps
/// ([#2341](is_sensor_cap)), which a wildcard never reaches.
#[derive(Clone, Debug)]
pub struct CapLedger {
    granted: BTreeSet<String>,
    /// What the server and world ceilings allow: their intersection, within the catalog.
    allowed: BTreeSet<String>,
    usage: BTreeMap<String, CapUsage>,
    grant_on_use: bool,
}

/// The outcome of asking the ledger about one use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Allowed; the capability was already granted.
    Allow,
    /// Allowed because the world's policy grants on first use; the grant must
    /// be made durable.
    AllowAndGrant,
    /// Refused.
    Deny(CapError),
}

impl CapLedger {
    /// A ledger with `granted` capabilities and the given policy. The
    /// default ceiling ([`default_ceiling_names`]) is allowed until a
    /// ceiling narrows the set: providers whose manifest default is
    /// `Excluded` are not allowed until a ceiling names them (#838).
    #[must_use]
    pub fn new(granted: BTreeSet<String>, grant_on_use: bool) -> Self {
        Self {
            granted,
            allowed: default_ceiling_names(),
            usage: BTreeMap::new(),
            grant_on_use,
        }
    }

    /// Replace the set the ceilings allow. Names outside the catalog are dropped.
    pub fn set_allowed(&mut self, allowed: BTreeSet<String>) {
        self.allowed = allowed.into_iter().filter(|name| is_known(name)).collect();
    }

    /// What the server and world ceilings allow.
    #[must_use]
    pub const fn allowed(&self) -> &BTreeSet<String> {
        &self.allowed
    }

    /// Whether `cap` is allowed and granted (`*` grants everything
    /// allowed except sensor caps, which need an explicit grant).
    #[must_use]
    pub fn is_granted(&self, cap: &str) -> bool {
        self.allowed.contains(cap)
            && (self.granted.contains(cap) || (self.granted.contains("*") && !is_sensor_cap(cap)))
    }

    /// Record one use of `cap` and decide it.
    pub fn decide(&mut self, cap: &str) -> Decision {
        self.decide_for(cap, true)
    }

    /// Record one use of `cap` and decide it, where `actor_permits` says
    /// whether the participant that caused the use may cause `cap`. A refused
    /// actor is counted as a denial and never triggers a grant.
    pub fn decide_for(&mut self, cap: &str, actor_permits: bool) -> Decision {
        // Unknown names aggregate into one shared bucket: a guest program
        // controls requested names, so one map entry per distinct unknown
        // name would be unbounded host memory (and CapsReport size).
        if !is_known(cap) {
            self.usage
                .entry(UNKNOWN_CAP_USAGE_KEY.to_owned())
                .or_default()
                .denied += 1;
            return Decision::Deny(CapError::unknown());
        }
        let entry = self.usage.entry(cap.to_owned()).or_default();
        // A ceiling is checked before anything else, so grant-on-use can
        // never add a name the server or the world excludes.
        if !self.allowed.contains(cap) {
            entry.denied += 1;
            return Decision::Deny(CapError::outside_ceiling());
        }
        if !actor_permits {
            entry.denied += 1;
            return Decision::Deny(CapError::actor_denied());
        }
        if self.granted.contains(cap) || (self.granted.contains("*") && !is_sensor_cap(cap)) {
            entry.allowed += 1;
            return Decision::Allow;
        }
        if is_sensor_cap(cap) {
            // Sensor caps are per-use (#2341): neither a wildcard grant
            // nor grant-on-use reaches them, so an ungranted use is a
            // consent refusal — never a silently minted durable grant.
            entry.denied += 1;
            return Decision::Deny(CapError::consent_required());
        }
        if self.grant_on_use {
            entry.allowed += 1;
            self.granted.insert(cap.to_owned());
            return Decision::AllowAndGrant;
        }
        entry.denied += 1;
        Decision::Deny(CapError::denied())
    }

    /// The granted set.
    #[must_use]
    pub const fn granted(&self) -> &BTreeSet<String> {
        &self.granted
    }

    /// Replace the granted set (restoring from the journal, or an admin edit).
    pub fn set_granted(&mut self, granted: BTreeSet<String>) {
        self.granted = granted;
    }

    /// What authority reaches now: granted and allowed, with `*` standing for
    /// every allowed name. This is the list a compile gate should see.
    #[must_use]
    pub fn effective(&self) -> BTreeSet<String> {
        if self.granted.contains("*") {
            // The wildcard expands to every allowed name except sensor
            // caps (#2341); explicitly granted sensor caps still count.
            let mut effective: BTreeSet<String> = self
                .allowed
                .iter()
                .filter(|name| !is_sensor_cap(name.as_str()))
                .cloned()
                .collect();
            effective.extend(
                self.granted
                    .intersection(&self.allowed)
                    .filter(|name| is_sensor_cap(name.as_str()))
                    .cloned(),
            );
            return effective;
        }
        self.granted.intersection(&self.allowed).cloned().collect()
    }

    /// Usage by capability name.
    #[must_use]
    pub const fn usage(&self) -> &BTreeMap<String, CapUsage> {
        &self.usage
    }

    /// Whether first use grants.
    #[must_use]
    pub const fn grant_on_use(&self) -> bool {
        self.grant_on_use
    }
}

/// Validate names an admin wants to grant or revoke: known capabilities or
/// `*`. A typo should fail loudly, not silently grant nothing.
///
/// # Errors
///
/// Names the first unknown capability.
pub fn validate_grant_names(names: &[String]) -> Result<()> {
    for name in names {
        if name != "*" && !is_known(name) {
            bail!(
                "unknown capability {name:?} (known: {})",
                PROVIDER_REGISTRY.ordered_names().join(", ")
            );
        }
    }
    Ok(())
}

/// Validate a capability ceiling, from an admin or the server flag: known
/// capabilities only. `*` is a grant shorthand, not a limit, so it is refused.
///
/// # Errors
///
/// Refuses `*` and names the first unknown capability.
pub fn validate_ceiling_names(names: &[String]) -> Result<()> {
    ensure!(
        !names.iter().any(|name| name == "*"),
        "a capability ceiling lists names; `*` only grants"
    );
    validate_grant_names(names)
}

/// What `id world caps` shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapsReport {
    /// Capabilities the world has granted.
    pub granted: Vec<String>,
    /// Capabilities the program's current `wants` needs.
    pub requested: Vec<String>,
    /// Requested but not effective: what an admin would grant, or which a
    /// ceiling would have to admit, to unblock it.
    pub missing: Vec<String>,
    /// Use since the world opened, including refused use.
    pub usage: BTreeMap<String, CapUsage>,
    /// `deny` or `grant-on-use`.
    pub policy: String,
    /// Every capability the server can provide.
    pub catalog: Vec<String>,
    /// The world's own capability ceiling. Empty in reports from servers
    /// that predate ceilings.
    #[serde(default)]
    pub ceiling: Vec<String>,
    /// Granted and within the server and world ceilings: what the program can
    /// use now, with `*` expanded. Empty in reports from servers that predate
    /// ceilings.
    #[serde(default)]
    pub effective: Vec<String>,
    /// Times a `view` or `layout` call trapped since the world opened
    /// (F11.17). A trapping view degrades to an error screen instead of
    /// poisoning the world, so this is the only trace of it.
    #[serde(default)]
    pub view_failures: u64,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn parses_subscriptions_and_requests() {
        let wants = parse_wants(
            r#"{"v":1,"subscribe":["players","time.tick:5000","bogus"],
                "requests":[{"id":"a","cap":"time.now"},
                            {"id":"b","cap":"chat.say","args":{"text":"hi"}}]}"#,
        )
        .unwrap();
        assert!(wants.subscribe.contains(&Subscription::Players));
        assert!(wants.subscribe.contains(&Subscription::Tick(5000)));
        assert!(
            wants
                .subscribe
                .contains(&Subscription::Unknown("bogus".to_owned()))
        );
        assert_eq!(wants.requests.len(), 2);
        assert_eq!(wants.requests[1].args["text"], "hi");
        assert_eq!(
            wants.caps().into_iter().collect::<Vec<_>>(),
            vec!["bogus", "chat.say", "players", "time.now", "time.tick"]
        );
    }

    #[test]
    fn non_object_request_args_are_rejected() {
        // #7929: a present args that is neither an object nor null used
        // to collapse to no arguments in every handler, silently
        // widening records.read to every record.
        for bad in ["42", "\"text\"", "[1,2]", "true"] {
            let doc =
                format!(r#"{{"requests":[{{"id":"r1","cap":"records.read","args":{bad}}}]}}"#);
            let error = parse_wants(&doc).unwrap_err();
            assert!(
                format!("{error:#}").contains("args must be a JSON object"),
                "{error:#}"
            );
        }
        // Absent args and args:null behave exactly as before.
        parse_wants(r#"{"requests":[{"id":"r1","cap":"records.read"}]}"#).unwrap();
        parse_wants(r#"{"requests":[{"id":"r1","cap":"records.read","args":null}]}"#).unwrap();
        // An object args is carried through unchanged.
        let wants = parse_wants(
            r#"{"requests":[{"id":"r1","cap":"records.read","args":{"prefix":"co"}}]}"#,
        )
        .unwrap();
        assert_eq!(wants.requests[0].args, serde_json::json!({"prefix": "co"}));
    }

    #[test]
    fn an_empty_document_wants_nothing() {
        assert_eq!(parse_wants("{}").unwrap(), Wants::default());
    }

    #[test]
    fn rejects_malformed_or_oversized_documents() {
        for bad in [
            "",
            "[]",
            "null",
            r#"{"v":2}"#,
            r#"{"subscribe":"players"}"#,
            r#"{"subscribe":[1]}"#,
            r#"{"requests":[{"cap":"time.now"}]}"#,
            r#"{"requests":[{"id":"a"}]}"#,
            r#"{"requests":[{"id":"","cap":"time.now"}]}"#,
            r#"{"requests":[{"id":"a\nb","cap":"time.now"}]}"#,
            r#"{"requests":[{"id":"a","cap":"time.now"},{"id":"a","cap":"time.now"}]}"#,
            r#"{"subscribe":["time.tick:1","time.tick:2","time.tick:3","time.tick:4","time.tick:5"]}"#,
        ] {
            assert!(parse_wants(bad).is_err(), "{bad:?}");
        }
        let many = format!(
            r#"{{"requests":[{}]}}"#,
            (0..=MAX_WANTS_ENTRIES)
                .map(|i| format!(r#"{{"id":"r{i}","cap":"time.now"}}"#))
                .collect::<Vec<_>>()
                .join(",")
        );
        assert!(parse_wants(&many).is_err());
        let args = format!(
            r#"{{"requests":[{{"id":"a","cap":"chat.say","args":{{"text":"{}"}}}}]}}"#,
            "x".repeat(MAX_ARGS_BYTES)
        );
        assert!(parse_wants(&args).is_err());
        assert!(parse_wants(&" ".repeat(MAX_WANTS_BYTES + 1)).is_err());
    }

    #[test]
    fn rejects_duplicate_keys_at_any_depth() {
        // #7621: every duplicate-key document below was accepted by the
        // bare from_str gate (last-wins collapse) before this test existed.
        for bad in [
            // The version check saw only the collapsed last "v".
            r#"{"v":2,"v":1}"#,
            r#"{"v":1,"v":2}"#,
            // A duplicate inside one request object collapsed its cap.
            r#"{"requests":[{"id":"r1","cap":"time.now","cap":"chat.say"}]}"#,
            // A duplicate inside request args.
            r#"{"requests":[{"id":"r1","cap":"chat.say","args":{"text":"a","text":"b"}}]}"#,
        ] {
            assert!(parse_wants(bad).is_err(), "{bad:?}");
        }
        // #6499 pins: an over-long cap and a control-character id hidden
        // behind benign duplicates inside one request entry.
        let long_cap = "x".repeat(MAX_NAME_BYTES + 1);
        let hidden_cap = format!(r#"{{"requests":[{{"id":"r1","cap":"{long_cap}","cap":"ok"}}]}}"#);
        assert!(parse_wants(&hidden_cap).is_err(), "{hidden_cap:?}");
        let hidden_id = "{\"requests\":[{\"id\":\"a\\u0001b\",\"id\":\"ok\",\"cap\":\"x\"}]}";
        assert!(parse_wants(hidden_id).is_err(), "{hidden_id:?}");
        // #6499 pin: a whole non-conforming requests array hidden behind
        // a duplicate top-level "requests" key.
        let hidden_requests =
            format!(r#"{{"requests":[{{"id":"r1","cap":"{long_cap}"}}],"requests":[]}}"#);
        assert!(
            parse_wants(&hidden_requests).is_err(),
            "{hidden_requests:?}"
        );
        // The 64-entry bound counted only the collapsed second array.
        let hidden = format!(
            r#"{{"subscribe":[{}],"subscribe":["players"]}}"#,
            (0..=MAX_WANTS_ENTRIES)
                .map(|i| format!(r#""cap{i}""#))
                .collect::<Vec<_>>()
                .join(",")
        );
        assert!(parse_wants(&hidden).is_err(), "{hidden:?}");
    }

    #[test]
    fn subscription_specs_round_trip() {
        for spec in ["players", "time.tick:1000"] {
            assert_eq!(Subscription::parse(spec).spec(), spec);
        }
        assert_eq!(
            Subscription::parse("time.tick:0"),
            Subscription::Unknown("time.tick:0".to_owned())
        );
        assert_eq!(
            Subscription::parse("time.tick:soon"),
            Subscription::Unknown("time.tick:soon".to_owned())
        );
    }

    #[test]
    fn the_ledger_denies_until_granted_and_counts_every_use() {
        let mut ledger = CapLedger::new(BTreeSet::new(), false);
        assert_eq!(
            ledger.decide("time.now"),
            Decision::Deny(CapError::denied())
        );
        assert_eq!(
            ledger.decide("time.now"),
            Decision::Deny(CapError::denied())
        );
        assert_eq!(
            ledger.decide("telepathy"),
            Decision::Deny(CapError::unknown())
        );
        ledger.set_granted(BTreeSet::from(["time.now".to_owned()]));
        assert_eq!(ledger.decide("time.now"), Decision::Allow);
        assert_eq!(
            ledger.usage()["time.now"],
            CapUsage {
                allowed: 1,
                denied: 2
            }
        );
        assert!(!ledger.is_granted("chat.say"));
        ledger.set_granted(BTreeSet::from(["*".to_owned()]));
        assert!(ledger.is_granted("chat.say"));
        assert!(
            !ledger.is_granted("telepathy"),
            "a wildcard grants only what exists"
        );
        assert_eq!(
            ledger.decide("telepathy"),
            Decision::Deny(CapError::unknown())
        );
    }

    #[test]
    fn grant_on_use_grants_known_capabilities_once() {
        let mut ledger = CapLedger::new(BTreeSet::new(), true);
        assert_eq!(ledger.decide("random.u64"), Decision::AllowAndGrant);
        assert_eq!(ledger.decide("random.u64"), Decision::Allow);
        assert_eq!(
            ledger.decide("telepathy"),
            Decision::Deny(CapError::unknown())
        );
        assert!(ledger.granted().contains("random.u64"));
        assert!(!ledger.granted().contains("telepathy"));
    }

    #[test]
    fn a_refused_actor_is_denied_and_never_grants() {
        let mut ledger = CapLedger::new(BTreeSet::new(), true);
        assert_eq!(
            ledger.decide_for("chat.say", false),
            Decision::Deny(CapError::actor_denied())
        );
        assert!(!ledger.granted().contains("chat.say"));
        assert_eq!(
            ledger.usage()["chat.say"],
            CapUsage {
                allowed: 0,
                denied: 1
            }
        );
        assert_eq!(ledger.decide_for("chat.say", true), Decision::AllowAndGrant);
    }

    #[test]
    fn grant_names_must_exist() {
        validate_grant_names(&["time.now".to_owned(), "*".to_owned()]).unwrap();
        validate_grant_names(&[
            "chat.history".to_owned(),
            "records.read".to_owned(),
            "schedule.at".to_owned(),
            "schedule.cancel".to_owned(),
            "RocJson".to_owned(),
            "RocGrid".to_owned(),
            "RocCap".to_owned(),
            "RocText".to_owned(),
        ])
        .unwrap();
        let error = validate_grant_names(&["time.nwo".to_owned()]).unwrap_err();
        assert!(error.to_string().contains("time.nwo"), "{error}");
    }

    #[test]
    fn schedule_ids_are_bounded_printable_ascii() {
        validate_schedule_id("a").unwrap();
        validate_schedule_id("round 3: start!").unwrap();
        validate_schedule_id(&"x".repeat(MAX_SCHEDULE_ID_BYTES)).unwrap();
        let long = "x".repeat(MAX_SCHEDULE_ID_BYTES + 1);
        for bad in [
            "",
            "a\nb",
            "tab\there",
            "del\u{7f}",
            "caf\u{e9}",
            long.as_str(),
        ] {
            assert!(validate_schedule_id(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn result_events_are_bounded_and_carry_codes() {
        let ok: serde_json::Value =
            serde_json::from_str(&result_event("a", Ok(serde_json::json!({"n":1})))).unwrap();
        assert_eq!(ok["ok"], true);
        assert_eq!(ok["value"]["n"], 1);
        let denied: serde_json::Value =
            serde_json::from_str(&result_event("a", Err(CapError::denied()))).unwrap();
        assert_eq!(
            (denied["ok"].as_bool(), denied["error"].as_str()),
            (Some(false), Some("cap_denied"))
        );
        let huge = result_event(
            "a",
            Ok(serde_json::Value::from("x".repeat(MAX_EVENT_BYTES))),
        );
        assert!(huge.len() < 200, "fallback event stays bounded: {huge}");
        let huge: serde_json::Value = serde_json::from_str(&huge).unwrap();
        assert_eq!(
            (huge["ok"].as_bool(), huge["error"].as_str()),
            (Some(false), Some(CapError::result_too_large().code())),
            "the oversized fallback carries the documented ResultTooLarge code"
        );
    }

    #[test]
    fn records_result_pages_always_fit_the_result_event() {
        // A page admitted at the cap-result bound, delivered with the
        // longest request id and a longest, escape-heavy next_after key,
        // stays a records result — never result_too_large (#4888).
        let entry = serde_json::json!({"key": "k", "value": "x".repeat(1024)});
        let entry_bytes = serde_json::to_vec(&entry).unwrap().len() + 1;
        let mut records = Vec::new();
        let mut used = 0;
        while used + entry_bytes <= MAX_RECORDS_RESULT_PAGE_BYTES {
            records.push(entry.clone());
            used += entry_bytes;
        }
        assert!(!records.is_empty());
        // 256 raw key bytes (128 backslash+quote pairs), the longest a
        // record key may be, and the worst case for JSON escaping.
        let next_after: String = (0..128)
            .flat_map(|_| [char::from_u32(92).unwrap(), '"'])
            .collect();
        assert_eq!(next_after.len(), 256);
        let value = serde_json::json!({
            "records": records,
            "next_after": next_after,
        });
        let event = result_event(&"i".repeat(MAX_NAME_BYTES), Ok(value));
        assert!(event.len() <= MAX_EVENT_BYTES, "event is {}", event.len());
        let event: serde_json::Value = serde_json::from_str(&event).unwrap();
        assert_eq!(event["ok"], true, "{event}");
        assert!(event["value"]["records"].is_array());
        // The registry reports the cap-result bound for records.read.
        assert!(
            RECORDS_READ_BOUNDS
                .contains(&ProviderBound::ResponseBytes(MAX_RECORDS_RESULT_PAGE_BYTES))
        );
        // The session-protocol page budget is a separate, larger bound.
        assert_eq!(MAX_RECORDS_PAGE_BYTES, 768 * 1024);
    }

    fn names(list: &[&str]) -> BTreeSet<String> {
        list.iter().map(|name| (*name).to_owned()).collect()
    }

    fn manifest(name: &'static str, default_ceiling: DefaultCeiling) -> ProviderManifest {
        ProviderManifest {
            name,
            version: 1,
            kind: ProviderKind::PlatformModule,
            argument_schema: ArgumentSchema::NotApplicable,
            bounds: &[],
            required_scope: RequiredScope::Join,
            default_ceiling,
        }
    }

    #[test]
    fn test_provider_registry_matches_the_legacy_catalog_and_carries_manifests() {
        let entry_names = PROVIDER_REGISTRY.ordered_names();
        // The registry assembles the production catalog plus any
        // feature-gated registry-only entries. Under `test-provider`
        // that is exactly one extra — the RegistryProbe (#1061),
        // whose manifest default is Excluded. The root package's
        // dev-dependencies enable the feature, and workspace test
        // builds (the Nix check phase among them) unify it into this
        // test binary, so the production expectations below are stated
        // against the catalog with the probe removed again.
        #[cfg(feature = "test-provider")]
        let production_names: Vec<&str> = entry_names
            .iter()
            .copied()
            .filter(|name| *name != TEST_PROVIDER_NAME)
            .collect();
        #[cfg(not(feature = "test-provider"))]
        let production_names: Vec<&str> = entry_names.clone();
        assert_eq!(
            production_names, CATALOG,
            "legacy catalog derives from manifests"
        );
        assert_eq!(
            production_names,
            vec![
                "time.now",
                "time.tick",
                "random.u64",
                "players",
                "players.list",
                "chat.say",
                "chat.history",
                "records.read",
                "schedule.at",
                "schedule.cancel",
                "world.info",
                "device.info",
                "loc.position",
                "loc.watch",
                "Key",
                "Screen",
                "RocJson",
                "RocGrid",
                "RocCap",
                "RocText",
            ],
            "registry retains the catalog names and order (17 pre-#2341 names plus loc.position/loc.watch and device.info)"
        );
        assert_eq!(
            PROVIDER_REGISTRY.entries().len(),
            production_names.len() + EXTRA_CATALOG_ENTRIES.len()
        );
        assert!(
            PROVIDER_REGISTRY
                .entries()
                .iter()
                .all(|entry| entry.version > 0)
        );
        assert!(PROVIDER_REGISTRY.entries().iter().all(|entry| {
            matches!(
                (entry.kind, entry.argument_schema),
                (ProviderKind::Request, ArgumentSchema::JsonObject(_))
                    | (ProviderKind::Subscription, ArgumentSchema::Subscription(_))
                    | (ProviderKind::PlatformModule, ArgumentSchema::NotApplicable)
            )
        }));
        assert_eq!(
            PROVIDER_REGISTRY.get("chat.say").unwrap().required_scope,
            RequiredScope::Chat
        );
        assert_eq!(
            PROVIDER_REGISTRY
                .get("chat.history")
                .unwrap()
                .required_scope,
            RequiredScope::ChatRead
        );
        assert!(matches!(
            PROVIDER_REGISTRY.get("RocText").unwrap().kind,
            ProviderKind::PlatformModule
        ));
        assert!(
            PROVIDER_REGISTRY
                .get("records.read")
                .unwrap()
                .bounds
                .contains(&ProviderBound::ResponseBytes(MAX_RECORDS_RESULT_PAGE_BYTES))
        );
        // Every non-sensor provider opted in explicitly. The sensor
        // caps (#2341) registered as Excluded, so the default ceiling
        // is the catalog minus loc.* — registering them widened no
        // existing world's authority. The RegistryProbe is Excluded
        // for the same reason (#838), so under `test-provider` it is
        // subtracted the same way.
        let mut excluded_from_ceiling = names(&["loc.position", "loc.watch"]);
        #[cfg(feature = "test-provider")]
        excluded_from_ceiling.insert(TEST_PROVIDER_NAME.to_owned());
        assert_eq!(
            default_ceiling_names(),
            catalog_names()
                .difference(&excluded_from_ceiling)
                .cloned()
                .collect::<BTreeSet<String>>()
        );
        assert_eq!(
            PROVIDER_REGISTRY
                .get("loc.position")
                .unwrap()
                .default_ceiling,
            DefaultCeiling::Excluded
        );
        assert_eq!(
            PROVIDER_REGISTRY.get("loc.watch").unwrap().default_ceiling,
            DefaultCeiling::Excluded
        );
    }

    #[test]
    fn device_manifest_round_trips_through_its_wire_form() {
        // Oracle: the declared v1 manifest contract (taxonomy §3.1) —
        // the wire object carries manifest.v plus the five sections,
        // and parsing it returns the same manifest.
        let manifest = DeviceManifest {
            display: serde_json::json!({"kind": "pixel", "width": 128, "height": 64, "colors": 1}),
            input: serde_json::json!({"keys": 8, "encoder": true, "pointer": false, "text_entry": "none"}),
            sensors: serde_json::json!({"gnss": {"accuracy_m": 5}, "microphone": false, "camera": false}),
            audio: serde_json::json!({"out": "pcm", "in": null}),
            network: serde_json::json!({"transport": "mqtt", "rtt_class": "slow"}),
        };
        let wire = manifest.to_json();
        assert_eq!(wire["manifest"]["v"], DEVICE_MANIFEST_VERSION);
        for section in ["display", "input", "sensors", "audio", "network"] {
            assert!(wire[section].is_object(), "section {section} present");
        }
        assert_eq!(DeviceManifest::from_json(&wire).unwrap(), manifest);
    }

    #[test]
    fn device_manifest_refuses_unsupported_versions_and_missing_sections() {
        // Oracle: the declared v1 manifest contract — an unsupported
        // version is refused, never silently interpreted.
        let mut wire = DeviceManifest {
            display: serde_json::json!({}),
            input: serde_json::json!({}),
            sensors: serde_json::json!({}),
            audio: serde_json::json!({}),
            network: serde_json::json!({}),
        }
        .to_json();
        wire["manifest"]["v"] = serde_json::json!(2);
        let error = DeviceManifest::from_json(&wire).unwrap_err();
        assert_eq!(error.code(), CapError::bad_args().code());
        assert!(
            error
                .reason()
                .is_some_and(|reason| reason.contains("unsupported")),
            "{error:?}"
        );
        let mut missing = DeviceManifest {
            display: serde_json::json!({}),
            input: serde_json::json!({}),
            sensors: serde_json::json!({}),
            audio: serde_json::json!({}),
            network: serde_json::json!({}),
        }
        .to_json();
        missing.as_object_mut().unwrap().remove("audio");
        assert!(DeviceManifest::from_json(&missing).is_err());
        // The request side negotiates by the same rule.
        assert_eq!(
            DeviceManifest::requested_version(&serde_json::json!({})).unwrap(),
            DEVICE_MANIFEST_VERSION
        );
        assert_eq!(
            DeviceManifest::requested_version(&serde_json::json!({"v": 1})).unwrap(),
            DEVICE_MANIFEST_VERSION
        );
        assert!(
            DeviceManifest::requested_version(&serde_json::json!({"v": 2})).is_err(),
            "a request for an unsupported version is refused"
        );
    }

    #[test]
    fn test_a_newly_registered_excluded_provider_does_not_join_existing_ceilings() {
        // An existing world's ceiling, snapshotted before the upgrade.
        let existing_world_ceiling = default_ceiling_names();
        // The upgrade registers two new providers: a sensitive one whose
        // manifest default is Excluded, and one that explicitly opts in.
        let mut upgraded: Vec<ProviderManifest> = CATALOG_ENTRIES.to_vec();
        upgraded.push(manifest("http.fetch", DefaultCeiling::Excluded));
        upgraded.push(manifest("time.rfc3339", DefaultCeiling::Included));
        let new_default = default_ceiling_from(&upgraded);
        assert!(
            !new_default.contains("http.fetch"),
            "an excluded provider joins no default ceiling by registration alone"
        );
        assert!(
            new_default.contains("time.rfc3339"),
            "an explicit Included default is the one way in"
        );
        assert!(
            existing_world_ceiling.is_subset(&new_default)
                && new_default.difference(&existing_world_ceiling).count() == 1,
            "existing worlds gain nothing but the opted-in provider: {new_default:?}"
        );
        // And a fresh ledger starts from the default ceiling, so an
        // excluded provider is outside it until a ceiling names it.
        let ledger = CapLedger::new(BTreeSet::new(), false);
        assert_eq!(ledger.allowed(), &default_ceiling_names());
        for entry in CATALOG_ENTRIES {
            if entry.default_ceiling == DefaultCeiling::Excluded {
                assert!(!ledger.allowed().contains(entry.name));
            }
        }
    }

    #[test]
    fn the_default_ceiling_is_the_catalog_minus_sensor_caps() {
        let mut ledger = CapLedger::new(names(&["time.now"]), false);
        assert_eq!(ledger.allowed(), &default_ceiling_names());
        assert!(ledger.is_granted("time.now"));
        assert_eq!(ledger.decide("time.now"), Decision::Allow);
        assert_eq!(
            ledger.decide("chat.say"),
            Decision::Deny(CapError::denied())
        );
        ledger.set_granted(names(&["*"]));
        // The wildcard reaches every allowed name except the sensor
        // caps (#2341), which the default ceiling already excludes.
        assert_eq!(ledger.effective(), default_ceiling_names());
        assert_eq!(
            ledger.decide("chat.say"),
            Decision::Allow,
            "a wildcard grant reaches every non-sensor catalog name"
        );
    }

    #[test]
    fn a_ceiling_denies_a_grant_and_a_wildcard_alike() {
        let mut ledger = CapLedger::new(names(&["*"]), false);
        ledger.set_allowed(names(&["time.now"]));
        assert!(ledger.is_granted("time.now"));
        assert!(!ledger.is_granted("chat.say"));
        assert_eq!(
            ledger.decide_for("chat.say", true),
            Decision::Deny(CapError::outside_ceiling())
        );
        ledger.set_granted(names(&["chat.say"]));
        assert_eq!(
            ledger.decide_for("chat.say", true),
            Decision::Deny(CapError::outside_ceiling()),
            "an explicit grant does not reach past the ceiling"
        );
        assert_eq!(
            ledger.usage()["chat.say"],
            CapUsage {
                allowed: 0,
                denied: 2
            }
        );
        // The explicit grant replaced the wildcard, and chat.say is outside the
        // ceiling, so nothing is effective.
        assert_eq!(ledger.effective(), names(&[]));
        ledger.set_granted(names(&["*"]));
        assert_eq!(ledger.effective(), names(&["time.now"]));
    }

    #[test]
    fn grant_on_use_never_grants_a_name_outside_the_ceiling() {
        let mut ledger = CapLedger::new(BTreeSet::new(), true);
        ledger.set_allowed(names(&["time.now"]));
        assert_eq!(
            ledger.decide("chat.say"),
            Decision::Deny(CapError::outside_ceiling())
        );
        assert!(!ledger.granted().contains("chat.say"));
        assert_eq!(ledger.decide("time.now"), Decision::AllowAndGrant);
        assert!(ledger.granted().contains("time.now"));
    }

    #[test]
    fn unknown_names_stay_unknown_under_a_ceiling() {
        let mut ledger = CapLedger::new(BTreeSet::new(), true);
        ledger.set_allowed(names(&["time.now", "telepathy"]));
        assert_eq!(
            ledger.decide("telepathy"),
            Decision::Deny(CapError::unknown()),
            "a name the catalog lacks is never allowed, even if a ceiling lists it"
        );
        assert_eq!(ledger.allowed(), &names(&["time.now"]));
    }

    #[test]
    fn unknown_names_and_specs_share_one_bounded_usage_bucket() {
        let mut ledger = CapLedger::new(BTreeSet::new(), false);
        for i in 0..10_000 {
            let name = format!("evil.cap.{i}");
            assert_eq!(ledger.decide(&name), Decision::Deny(CapError::unknown()));
            // Unknown subscription specs reach the ledger as their full spec.
            let spec = format!("evil.spec:{i}");
            let cap = Subscription::parse(&spec).cap().to_owned();
            assert_eq!(ledger.decide(&cap), Decision::Deny(CapError::unknown()));
        }
        assert_eq!(
            ledger.usage().len(),
            1,
            "20,000 distinct unknown names must not mint 20,000 entries"
        );
        assert_eq!(
            ledger.usage()[UNKNOWN_CAP_USAGE_KEY],
            CapUsage {
                allowed: 0,
                denied: 20_000
            }
        );

        // Known capabilities keep per-name accounting beside the bucket.
        ledger.set_granted(BTreeSet::from(["time.now".to_owned()]));
        assert_eq!(ledger.decide("time.now"), Decision::Allow);
        assert_eq!(
            ledger.decide("chat.say"),
            Decision::Deny(CapError::denied())
        );
        assert_eq!(
            ledger.usage()["time.now"],
            CapUsage {
                allowed: 1,
                denied: 0
            }
        );
        assert_eq!(
            ledger.usage()["chat.say"],
            CapUsage {
                allowed: 0,
                denied: 1
            }
        );
        assert!(
            ledger.usage().len() <= CATALOG.len() + 1,
            "usage stays bounded by the catalog plus the unknown bucket"
        );
    }

    #[test]
    fn a_ceiling_takes_the_server_and_world_intersection() {
        let mut ledger = CapLedger::new(names(&["chat.say", "time.now"]), false);
        ledger.set_allowed(names(&["chat.say"]));
        assert_eq!(ledger.effective(), names(&["chat.say"]));
        assert!(!ledger.is_granted("time.now"));
    }

    #[test]
    fn ceiling_names_refuse_the_wildcard_and_unknown_names() {
        validate_ceiling_names(&["time.now".to_owned()]).unwrap();
        assert!(validate_ceiling_names(&["*".to_owned()]).is_err());
        assert!(validate_ceiling_names(&["time.nwo".to_owned()]).is_err());
    }

    /// A ledger whose ceiling explicitly names the sensor caps, as a
    /// world that wants location must set up (#2341).
    fn loc_ledger(granted: &[&str], grant_on_use: bool) -> CapLedger {
        let mut ledger = CapLedger::new(names(granted), grant_on_use);
        let mut allowed = default_ceiling_names();
        allowed.insert("loc.position".to_owned());
        allowed.insert("loc.watch".to_owned());
        ledger.set_allowed(allowed);
        ledger
    }

    #[test]
    fn a_wildcard_grant_never_reaches_loc_caps() {
        let mut ledger = loc_ledger(&["*"], false);
        assert!(!ledger.is_granted("loc.position"));
        assert!(!ledger.is_granted("loc.watch"));
        assert_eq!(
            ledger.decide("loc.position"),
            Decision::Deny(CapError::consent_required()),
            "a wildcard grant must never reach loc.*"
        );
        assert!(
            !ledger.effective().contains("loc.position"),
            "effective authority excludes sensor caps under a wildcard"
        );
        // The same wildcard still reaches ordinary caps.
        assert_eq!(ledger.decide("time.now"), Decision::Allow);
    }

    #[test]
    fn an_explicit_grant_reaches_loc_caps() {
        let mut ledger = loc_ledger(&["loc.position", "loc.watch"], false);
        assert!(ledger.is_granted("loc.position"));
        assert_eq!(ledger.decide("loc.position"), Decision::Allow);
        assert_eq!(ledger.decide("loc.watch"), Decision::Allow);
        assert!(ledger.effective().contains("loc.watch"));
    }

    #[test]
    fn grant_on_use_never_mints_a_sensor_grant() {
        let mut ledger = loc_ledger(&[], true);
        assert_eq!(
            ledger.decide("loc.position"),
            Decision::Deny(CapError::consent_required())
        );
        assert!(
            !ledger.granted().contains("loc.position"),
            "grant-on-use must not record a durable sensor grant"
        );
        // Grant-on-use still works for ordinary caps.
        assert_eq!(ledger.decide("time.now"), Decision::AllowAndGrant);
    }

    #[test]
    fn loc_caps_stay_outside_the_default_ceiling() {
        // No explicit ceiling entry: even an explicit grant is refused
        // at the ceiling, like every other excluded provider (#838).
        let mut ledger = CapLedger::new(names(&["loc.position"]), false);
        assert_eq!(
            ledger.decide("loc.position"),
            Decision::Deny(CapError::outside_ceiling())
        );
    }

    #[test]
    fn a_location_reading_carries_the_typed_shape() {
        let reading = LocationReading {
            lat: 30.2672,
            lon: -97.7431,
            accuracy_m: 5.0,
            fix_age_ms: 250,
            source: LocationSource::Gnss,
        };
        reading.validate().unwrap();
        assert_eq!(
            reading.to_json(),
            serde_json::json!({
                "lat": 30.2672,
                "lon": -97.7431,
                "accuracy_m": 5.0,
                "fix_age_ms": 250,
                "source": "gnss",
            })
        );
        for (wire, source) in [
            ("gnss", LocationSource::Gnss),
            ("network", LocationSource::Network),
            ("ip", LocationSource::Ip),
            ("caller_supplied", LocationSource::CallerSupplied),
        ] {
            assert_eq!(LocationSource::parse(wire), Some(source));
            assert_eq!(source.as_str(), wire);
        }
        assert_eq!(LocationSource::parse("gps"), None);
        let bad = LocationReading {
            lat: 91.0,
            ..reading
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn an_imprecise_fix_is_refused_at_the_precision_ceiling() {
        // IP geolocation: kilometre-scale, named as such.
        let ip_fix = LocationReading {
            lat: 30.2672,
            lon: -97.7431,
            accuracy_m: 5_000.0,
            fix_age_ms: 60_000,
            source: LocationSource::Ip,
        };
        let refusal = ip_fix.refuse_beyond_ceiling(100.0).unwrap_err();
        assert_eq!(refusal, CapError::precision_ceiling());
        assert_eq!(refusal.code(), "outside_ceiling");
        assert_eq!(
            refusal.reason().map(String::as_str),
            Some("precision_ceiling")
        );
        // The same fix is fine for a consumer whose ceiling allows it,
        // and a precise fix passes a tight ceiling.
        ip_fix.refuse_beyond_ceiling(10_000.0).unwrap();
        let gnss_fix = LocationReading {
            accuracy_m: 5.0,
            source: LocationSource::Gnss,
            ..ip_fix
        };
        gnss_fix.refuse_beyond_ceiling(100.0).unwrap();
    }

    #[test]
    fn consent_states_map_to_the_refusal_vocabulary() {
        assert_eq!(LocationConsent::Granted.refusal(), None);
        let required = LocationConsent::ConsentRequired.refusal().unwrap();
        assert_eq!(required, CapError::consent_required());
        assert_eq!(required.code(), "cap_denied");
        assert_eq!(
            required.reason().map(String::as_str),
            Some("consent_required")
        );
        let os_denied = LocationConsent::OsPermissionDenied.refusal().unwrap();
        assert_eq!(os_denied, CapError::os_permission_denied());
        assert_eq!(os_denied.code(), "cap_denied");
        assert_eq!(
            os_denied.reason().map(String::as_str),
            Some("os_permission_denied")
        );
        let no_fix = CapError::no_fix();
        assert_eq!(no_fix.code(), "failed");
        assert_eq!(no_fix.reason().map(String::as_str), Some("no_fix"));
    }

    #[test]
    fn loc_watch_specs_enforce_the_interval_floor() {
        assert_eq!(
            parse_loc_watch_interval_ms("loc.watch"),
            Ok(MIN_LOC_WATCH_INTERVAL_MS)
        );
        assert_eq!(parse_loc_watch_interval_ms("loc.watch:60000"), Ok(60_000));
        assert_eq!(
            parse_loc_watch_interval_ms("loc.watch:100"),
            Err(CapError::bad_args_with(
                "loc.watch interval below the floor"
            ))
        );
        assert!(parse_loc_watch_interval_ms("loc.watch:fast").is_err());
        assert!(parse_loc_watch_interval_ms("time.tick:5000").is_err());
    }

    #[test]
    fn consent_withdrawal_closes_an_open_watch_with_the_revocation_event() {
        assert_eq!(
            loc_watch_revocation_event(),
            "{\"cap\":\"loc.watch\",\"event\":\"revoked\",\"reason\":\"consent_withdrawn\"}"
        );
        let reading = LocationReading {
            lat: 30.2672,
            lon: -97.7431,
            accuracy_m: 12.0,
            fix_age_ms: 0,
            source: LocationSource::Network,
        };
        let delivered: serde_json::Value =
            serde_json::from_str(&loc_watch_reading_event(&reading)).unwrap();
        assert_eq!(delivered["cap"], "loc.watch");
        assert_eq!(delivered["source"], "network");
        assert_eq!(delivered["accuracy_m"], 12.0);
    }
}
