//! Capabilities: what a world program may ask the server for.
//!
//! A program stays a pure function. What it needs from the server is **data
//! it returns** (`wants`, a pure projection of its model) and **events it
//! receives** (delivered through `update` with participant id `0`, every one
//! journaled before delivery, so replay feeds the program exactly what it saw
//! the first time).
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
//!
//! This module holds the pure parts: the schema and its bounds, the catalog of
//! capability names, and the grant policy with its usage accounting. The
//! actor in [`crate::world`] executes providers because they need the world.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;

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

/// Capabilities the server knows how to provide. New providers add names;
/// existing names never change meaning.
pub const CATALOG: &[&str] = &[
    "time.now",
    "time.tick",
    "random.u64",
    "players",
    "players.list",
    "chat.say",
    "world.info",
];

/// Whether `name` is a capability the server can provide.
#[must_use]
pub fn is_known(name: &str) -> bool {
    CATALOG.contains(&name)
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
            serde_json::json!({"cap":"result","id":id,"ok":false,"error":error.code()})
        }
    };
    let text = event.to_string();
    if text.len() > MAX_EVENT_BYTES {
        return serde_json::json!({"cap":"result","id":id,"ok":false,"error":"result_too_large"})
            .to_string();
    }
    text
}

/// The event delivered when a subscription is refused.
#[must_use]
pub fn subscription_refused_event(spec: &str, error: CapError) -> String {
    serde_json::json!({"cap":"subscription","spec":spec,"ok":false,"error":error.code()})
        .to_string()
}

/// Why a request or subscription was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapError {
    /// The world has not granted the capability.
    Denied,
    /// The server has no such capability.
    Unknown,
    /// The request's arguments were not acceptable.
    BadArgs,
    /// The provider failed.
    Failed,
}

impl CapError {
    /// The code a program sees.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Denied => "cap_denied",
            Self::Unknown => "unknown_cap",
            Self::BadArgs => "bad_args",
            Self::Failed => "failed",
        }
    }
}

/// How a capability was used since the world opened.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct CapUsage {
    /// Uses the world allowed.
    pub allowed: u64,
    /// Uses refused (not granted, or not a known capability).
    pub denied: u64,
}

/// What a world has granted, and what its program has asked for.
#[derive(Clone, Debug, Default)]
pub struct CapLedger {
    granted: BTreeSet<String>,
    usage: BTreeMap<String, CapUsage>,
    grant_on_use: bool,
}

/// The outcome of asking the ledger about one use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    /// A ledger with `granted` capabilities and the given policy.
    #[must_use]
    pub fn new(granted: BTreeSet<String>, grant_on_use: bool) -> Self {
        Self {
            granted,
            usage: BTreeMap::new(),
            grant_on_use,
        }
    }

    /// Whether `cap` is granted (`*` grants everything the server knows).
    #[must_use]
    pub fn is_granted(&self, cap: &str) -> bool {
        is_known(cap) && (self.granted.contains(cap) || self.granted.contains("*"))
    }

    /// Record one use of `cap` and decide it.
    pub fn decide(&mut self, cap: &str) -> Decision {
        let entry = self.usage.entry(cap.to_owned()).or_default();
        if !is_known(cap) {
            entry.denied += 1;
            return Decision::Deny(CapError::Unknown);
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
        Decision::Deny(CapError::Denied)
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
                CATALOG.join(", ")
            );
        }
    }
    Ok(())
}

/// What `id world caps` shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CapsReport {
    /// Capabilities the world has granted.
    pub granted: Vec<String>,
    /// Capabilities the program's current `wants` needs.
    pub requested: Vec<String>,
    /// Requested but not granted: what an admin would grant to unblock it.
    pub missing: Vec<String>,
    /// Use since the world opened, including refused use.
    pub usage: BTreeMap<String, CapUsage>,
    /// `deny` or `grant-on-use`.
    pub policy: &'static str,
    /// Every capability the server can provide.
    pub catalog: Vec<&'static str>,
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
        assert_eq!(ledger.decide("time.now"), Decision::Deny(CapError::Denied));
        assert_eq!(ledger.decide("time.now"), Decision::Deny(CapError::Denied));
        assert_eq!(
            ledger.decide("telepathy"),
            Decision::Deny(CapError::Unknown)
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
            Decision::Deny(CapError::Unknown)
        );
    }

    #[test]
    fn grant_on_use_grants_known_capabilities_once() {
        let mut ledger = CapLedger::new(BTreeSet::new(), true);
        assert_eq!(ledger.decide("random.u64"), Decision::AllowAndGrant);
        assert_eq!(ledger.decide("random.u64"), Decision::Allow);
        assert_eq!(
            ledger.decide("telepathy"),
            Decision::Deny(CapError::Unknown)
        );
        assert!(ledger.granted().contains("random.u64"));
        assert!(!ledger.granted().contains("telepathy"));
    }

    #[test]
    fn grant_names_must_exist() {
        validate_grant_names(&["time.now".to_owned(), "*".to_owned()]).unwrap();
        let error = validate_grant_names(&["time.nwo".to_owned()]).unwrap_err();
        assert!(error.to_string().contains("time.nwo"), "{error}");
    }

    #[test]
    fn result_events_are_bounded_and_carry_codes() {
        let ok: serde_json::Value =
            serde_json::from_str(&result_event("a", Ok(serde_json::json!({"n":1})))).unwrap();
        assert_eq!(ok["ok"], true);
        assert_eq!(ok["value"]["n"], 1);
        let denied: serde_json::Value =
            serde_json::from_str(&result_event("a", Err(CapError::Denied))).unwrap();
        assert_eq!(
            (denied["ok"].as_bool(), denied["error"].as_str()),
            (Some(false), Some("cap_denied"))
        );
        let huge = result_event(
            "a",
            Ok(serde_json::Value::from("x".repeat(MAX_EVENT_BYTES))),
        );
        assert!(huge.contains("result_too_large") && huge.len() < 200);
    }
}
