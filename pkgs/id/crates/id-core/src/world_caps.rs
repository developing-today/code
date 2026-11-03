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
        /// All statically registered providers, in stable catalog order.
        pub const CATALOG_ENTRIES: &[ProviderManifest] = &[
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
    ProviderBound::ResponseBytes(MAX_RECORDS_PAGE_BYTES),
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
/// Largest page of records returned by `records.read`, in bytes.
pub const MAX_RECORDS_PAGE_BYTES: usize = 768 * 1024;

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
    let value: serde_json::Value = serde_json::from_str(text).context("wants is not JSON")?;
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
pub fn subscription_refused_event(spec: &str, error: CapError) -> String {
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
/// the grant set counts as every allowed name.
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

    /// Whether `cap` is allowed and granted (`*` grants everything allowed).
    #[must_use]
    pub fn is_granted(&self, cap: &str) -> bool {
        self.allowed.contains(cap) && (self.granted.contains(cap) || self.granted.contains("*"))
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
        if self.granted.contains(cap) || self.granted.contains("*") {
            entry.allowed += 1;
            return Decision::Allow;
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
            return self.allowed.clone();
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
        assert_eq!(
            entry_names, CATALOG,
            "legacy catalog derives from manifests"
        );
        assert_eq!(
            entry_names,
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
                "Key",
                "Screen",
                "RocJson",
                "RocGrid",
                "RocCap",
                "RocText",
            ],
            "registry retains the exact pre-change 17-name catalog and order"
        );
        assert_eq!(PROVIDER_REGISTRY.entries().len(), 17);
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
                .contains(&ProviderBound::ResponseBytes(MAX_RECORDS_PAGE_BYTES))
        );
        // Every provider registered today opted in explicitly, so the
        // default ceiling is still the whole catalog — naming the choice
        // changed no world's effective ceiling.
        assert_eq!(default_ceiling_names(), catalog_names());
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
    fn the_default_ceiling_is_the_whole_catalog() {
        let mut ledger = CapLedger::new(names(&["time.now"]), false);
        assert_eq!(ledger.allowed(), &default_ceiling_names());
        assert!(ledger.is_granted("time.now"));
        assert_eq!(ledger.decide("time.now"), Decision::Allow);
        assert_eq!(
            ledger.decide("chat.say"),
            Decision::Deny(CapError::denied())
        );
        ledger.set_granted(names(&["*"]));
        assert_eq!(ledger.effective(), catalog_names());
        assert_eq!(
            ledger.decide("chat.say"),
            Decision::Allow,
            "a wildcard grant reaches every catalog name"
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
}
