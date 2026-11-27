//! Explain output: every effective value with the layer that set it.
//!
//! Slice 9111-F2 of the #9111 record (§6.8): "The binary can print the
//! resolved document with the source of each value (which layer set it)
//! and the digest. Secrets appear as references. This output is what an
//! operator reviews before activation."
//!
//! [`explain`] takes the document as written, so it can tell a key the
//! document set from one left to its default, resolves it exactly as
//! [`ConfigDocument::resolve`] does, and returns one [`ExplainEntry`]
//! per effective value in a fixed order, plus the resolved-config digest
//! of [`resolved_digest`]. Its [`Display`](fmt::Display) form is the
//! text an operator reads; equal inputs give equal text.
//!
//! The schema carries no secrets yet; when it does, they appear here as
//! their references, never as values.

use std::fmt;

use serde_json::Value;

use crate::{
    CeilingOverride, ConfigDocument, ConfigError, Fanout, ResolvedConfig, resolved_digest,
};

/// Source of a value the document did not set and no layer changed.
pub const SOURCE_DEFAULT: &str = "default";
/// Source of a value the document set.
pub const SOURCE_DOCUMENT: &str = "document";
/// Source of a fixed gate: compiled in, never configurable.
pub const SOURCE_FIXED: &str = "fixed";

/// One effective value and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplainEntry {
    /// Dotted path of the value, as in the document.
    pub path: String,
    /// The effective value as compact JSON.
    pub value: String,
    /// The layer that set it: `default`, `document`, `fixed`,
    /// `mode:<mode>`, `override:<scope>`, or `clamped:<path>`.
    pub source: String,
}

/// The explain output for one document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Explanation {
    /// Every effective value, in a fixed order.
    pub entries: Vec<ExplainEntry>,
    /// The resolved-config digest (lowercase hex SHA-256).
    pub digest: String,
}

impl Explanation {
    /// The entry for `path`, if there is one.
    #[must_use]
    pub fn entry(&self, path: &str) -> Option<&ExplainEntry> {
        self.entries.iter().find(|entry| entry.path == path)
    }
}

impl fmt::Display for Explanation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for entry in &self.entries {
            writeln!(f, "{} = {}  # {}", entry.path, entry.value, entry.source)?;
        }
        write!(f, "digest = sha256:{}", self.digest)
    }
}

/// Explain a config document given as JSON text.
///
/// # Errors
///
/// Any error [`ConfigDocument::parse`] or [`ConfigDocument::resolve`]
/// returns: a document that would not load is not explained.
pub fn explain(input: &str) -> Result<Explanation, ConfigError> {
    let document = ConfigDocument::parse(input)?;
    let resolved = document.resolve()?;
    let digest = resolved_digest(&resolved)?;
    // The text parsed as a typed document, so it is a JSON object; this
    // second read only asks which keys the document wrote.
    let written: Value = serde_json::from_str(input)?;
    let mut entries = Vec::new();
    top_level(&resolved, &mut entries)?;
    ceilings(&document, &resolved, &mut entries);
    gates(&resolved, &mut entries);
    behavior(&resolved, &mut entries)?;
    fanout(&written, &resolved, &mut entries)?;
    peers(&written, &resolved, &mut entries)?;
    storage(&written, &resolved, &mut entries);
    tiers(&written, &resolved, &mut entries);
    links(&written, &resolved, &mut entries)?;
    namespaces(&written, &resolved, &mut entries);
    extensions(&resolved, &mut entries)?;
    Ok(Explanation { entries, digest })
}

fn push(entries: &mut Vec<ExplainEntry>, path: &str, value: String, source: &str) {
    entries.push(ExplainEntry {
        path: path.to_owned(),
        value,
        source: source.to_owned(),
    });
}

fn json<T: serde::Serialize>(value: &T) -> Result<String, ConfigError> {
    Ok(serde_json::to_string(value)?)
}

fn top_level(
    resolved: &ResolvedConfig,
    entries: &mut Vec<ExplainEntry>,
) -> Result<(), ConfigError> {
    push(
        entries,
        "schema_version",
        resolved.schema_version.to_string(),
        SOURCE_DOCUMENT,
    );
    push(
        entries,
        "config_id",
        json(&resolved.config_id)?,
        SOURCE_DOCUMENT,
    );
    push(
        entries,
        "revision",
        resolved.revision.to_string(),
        SOURCE_DOCUMENT,
    );
    push(entries, "mode", json(&resolved.mode)?, SOURCE_DOCUMENT);
    push(
        entries,
        "protocol_versions",
        json(&resolved.protocol_versions)?,
        SOURCE_DOCUMENT,
    );
    Ok(())
}

/// Reads one ceiling out of a scope's override, widened to `u64`.
type Narrowing = fn(&CeilingOverride) -> Option<u64>;

/// Each ceiling with the scope that narrowed it last, or `document`.
fn ceilings(document: &ConfigDocument, resolved: &ResolvedConfig, entries: &mut Vec<ExplainEntry>) {
    let mut ordered: Vec<_> = document.overrides.iter().collect();
    ordered.sort_by(|a, b| a.scope.cmp(&b.scope));
    let fields: [(&str, u64, u64, Narrowing); 4] = [
        (
            "ceilings.max_hops",
            u64::from(document.ceilings.max_hops),
            u64::from(resolved.ceilings.max_hops),
            |o| o.max_hops.map(u64::from),
        ),
        (
            "ceilings.max_bytes",
            document.ceilings.max_bytes,
            resolved.ceilings.max_bytes,
            |o| o.max_bytes,
        ),
        (
            "ceilings.max_time_ms",
            document.ceilings.max_time_ms,
            resolved.ceilings.max_time_ms,
            |o| o.max_time_ms,
        ),
        (
            "ceilings.max_fan_out",
            u64::from(document.ceilings.max_fan_out),
            u64::from(resolved.ceilings.max_fan_out),
            |o| o.max_fan_out.map(u64::from),
        ),
    ];
    for (path, written, effective, narrowing) in fields {
        let mut source = SOURCE_DOCUMENT.to_owned();
        let mut current = written;
        for scoped in &ordered {
            if let Some(value) = narrowing(&scoped.ceilings)
                && value < current
            {
                current = value;
                source = format!("override:{}", scoped.scope);
            }
        }
        debug_assert_eq!(current, effective, "{path} narrows as resolve does");
        push(entries, path, effective.to_string(), &source);
    }
}

fn gates(resolved: &ResolvedConfig, entries: &mut Vec<ExplainEntry>) {
    let gates = resolved.gates;
    for (path, on) in [
        ("gates.signature_verification", gates.signature_verification),
        ("gates.validation", gates.validation),
        ("gates.replay_protection", gates.replay_protection),
        ("gates.authorization", gates.authorization),
        ("gates.tenant_scope", gates.tenant_scope),
    ] {
        push(entries, path, on.to_string(), SOURCE_FIXED);
    }
}

fn behavior(resolved: &ResolvedConfig, entries: &mut Vec<ExplainEntry>) -> Result<(), ConfigError> {
    let source = |field: &str| {
        resolved
            .provenance
            .get(field)
            .map_or(SOURCE_DEFAULT, String::as_str)
            .to_owned()
    };
    let behavior = resolved.behavior;
    push(
        entries,
        "behavior.max_hops",
        behavior.max_hops.to_string(),
        &source("max_hops"),
    );
    push(
        entries,
        "behavior.sync_scope",
        json(&behavior.sync_scope)?,
        &source("sync_scope"),
    );
    push(
        entries,
        "behavior.tier",
        json(&behavior.tier)?,
        &source("tier"),
    );
    Ok(())
}

fn fanout(
    written: &Value,
    resolved: &ResolvedConfig,
    entries: &mut Vec<ExplainEntry>,
) -> Result<(), ConfigError> {
    let set = |key: &str| written.get("fanout").and_then(|f| f.get(key)).is_some();
    let source = |key: &str| {
        if set(key) {
            SOURCE_DOCUMENT
        } else {
            SOURCE_DEFAULT
        }
    };
    let fanout = &resolved.fanout;
    push(
        entries,
        "fanout.deadline_ms",
        fanout.deadline_ms.to_string(),
        source("deadline_ms"),
    );
    // The resolved timeout differs from the one given (or its default)
    // only when it was clamped to the deadline.
    let given = written
        .get("fanout")
        .and_then(|f| f.get("per_peer_timeout_ms"))
        .and_then(Value::as_u64)
        .unwrap_or_else(|| Fanout::default().per_peer_timeout_ms);
    let timeout_source = if given == fanout.per_peer_timeout_ms {
        source("per_peer_timeout_ms")
    } else {
        "clamped:fanout.deadline_ms"
    };
    push(
        entries,
        "fanout.per_peer_timeout_ms",
        fanout.per_peer_timeout_ms.to_string(),
        timeout_source,
    );
    push(
        entries,
        "fanout.peer_breadth",
        fanout.peer_breadth.to_string(),
        source("peer_breadth"),
    );
    remotes(written, resolved, entries)?;
    Ok(())
}

/// Each fan-out remote's fields under `fanout.remotes[<peer_id>]`, in ask
/// order. The timeout and scope say which plane supplied them.
fn remotes(
    written: &Value,
    resolved: &ResolvedConfig,
    entries: &mut Vec<ExplainEntry>,
) -> Result<(), ConfigError> {
    let written_remotes = written
        .get("fanout")
        .and_then(|f| f.get("remotes"))
        .and_then(Value::as_array);
    for remote in &resolved.fanout.remotes {
        let entry = written_remotes.and_then(|list| {
            list.iter()
                .find(|item| item.get("peer_id").and_then(Value::as_str) == Some(&remote.peer_id))
        });
        let wrote = |key: &str| entry.and_then(|e| e.get(key)).is_some_and(|v| !v.is_null());
        let peer = resolved.peers.iter().find(|p| p.peer_id == remote.peer_id);
        let peer_id = json(&remote.peer_id)?;
        let from_peer = format!("peers[{peer_id}]");
        let prefix = format!("fanout.remotes[{peer_id}]");
        let relay_source = if wrote("relay") {
            SOURCE_DOCUMENT
        } else {
            SOURCE_DEFAULT
        };
        // The effective timeout comes from the remote, else the peer, else
        // the fan-out per-peer timeout, and is then capped by the deadline.
        let given = entry
            .and_then(|e| e.get("timeout_ms"))
            .and_then(Value::as_u64)
            .map(|t| (t, SOURCE_DOCUMENT.to_owned()))
            .or_else(|| {
                peer.and_then(|p| p.timeout_ms)
                    .map(|t| (t, from_peer.clone()))
            })
            .unwrap_or_else(|| {
                (
                    resolved.fanout.per_peer_timeout_ms,
                    "fanout.per_peer_timeout_ms".to_owned(),
                )
            });
        let timeout = remote.timeout_ms.unwrap_or(given.0);
        let timeout_source = if timeout == given.0 {
            given.1
        } else {
            "clamped:fanout.deadline_ms".to_owned()
        };
        let scope_source = if wrote("scope") {
            SOURCE_DOCUMENT.to_owned()
        } else if remote.scope.is_some() {
            from_peer.clone()
        } else {
            SOURCE_DEFAULT.to_owned()
        };
        push(
            entries,
            &format!("{prefix}.priority"),
            remote.priority.to_string(),
            SOURCE_DOCUMENT,
        );
        push(
            entries,
            &format!("{prefix}.relay"),
            remote.relay.to_string(),
            relay_source,
        );
        push(
            entries,
            &format!("{prefix}.timeout_ms"),
            timeout.to_string(),
            &timeout_source,
        );
        push(
            entries,
            &format!("{prefix}.scope"),
            json(&remote.scope)?,
            &scope_source,
        );
    }
    Ok(())
}

/// Each peer's fields under `peers[<peer_id>]`, in `peer_id` order; a
/// field the entry did not write shows its default.
fn peers(
    written: &Value,
    resolved: &ResolvedConfig,
    entries: &mut Vec<ExplainEntry>,
) -> Result<(), ConfigError> {
    let written_peers = written.get("peers").and_then(Value::as_array);
    for peer in &resolved.peers {
        let entry = written_peers.and_then(|list| {
            list.iter()
                .find(|item| item.get("peer_id").and_then(Value::as_str) == Some(&peer.peer_id))
        });
        let source = |key: &str| {
            if entry.and_then(|e| e.get(key)).is_some() {
                SOURCE_DOCUMENT
            } else {
                SOURCE_DEFAULT
            }
        };
        let prefix = format!("peers[{}]", json(&peer.peer_id)?);
        let fields = [
            ("class", json(&peer.class)?),
            ("operations", json(&peer.operations)?),
            ("relay_granted", peer.relay_granted.to_string()),
            ("scope", json(&peer.scope)?),
            ("policy_version", peer.policy_version.to_string()),
            ("valid_from", json(&peer.valid_from)?),
            ("expires_at", json(&peer.expires_at)?),
            ("timeout_ms", json(&peer.timeout_ms)?),
            ("priority", json(&peer.priority)?),
        ];
        for (key, value) in fields {
            push(entries, &format!("{prefix}.{key}"), value, source(key));
        }
        // Overrides that set nothing are dropped at resolution, so each
        // one listed here was written.
        for (operation, over) in &peer.overrides {
            if let Some(adapter) = &over.adapter {
                push(
                    entries,
                    &format!("{prefix}.overrides.{}.adapter", operation.as_str()),
                    adapter.clone(),
                    SOURCE_DOCUMENT,
                );
            }
            if let Some(timeout) = over.timeout_ms {
                push(
                    entries,
                    &format!("{prefix}.overrides.{}.timeout_ms", operation.as_str()),
                    timeout.to_string(),
                    SOURCE_DOCUMENT,
                );
            }
        }
    }
    Ok(())
}

fn storage(written: &Value, resolved: &ResolvedConfig, entries: &mut Vec<ExplainEntry>) {
    // An absent key and an explicit `null` both take the default (record
    // §6.3), so only a non-null written value is the document's.
    let source = |store: &str, key: &str| {
        let set = written
            .get("storage")
            .and_then(|s| s.get(store))
            .and_then(|b| b.get(key))
            .is_some_and(|v| !v.is_null());
        if set { SOURCE_DOCUMENT } else { SOURCE_DEFAULT }
    };
    let storage = &resolved.storage;
    for (store, binding) in [
        ("local_store", &storage.local_store),
        ("event_log", &storage.event_log),
    ] {
        let fields = [
            ("adapter", binding.adapter.clone()),
            ("durability", binding.durability.clone()),
            (
                "sync_participation",
                binding.sync_participation.as_str().to_owned(),
            ),
        ];
        for (key, value) in fields {
            push(
                entries,
                &format!("storage.{store}.{key}"),
                value,
                source(store, key),
            );
        }
    }
    if let Some(object_store) = &storage.object_store {
        push(
            entries,
            "storage.object_store.adapter",
            object_store.adapter.clone(),
            SOURCE_DOCUMENT,
        );
        push(
            entries,
            "storage.object_store.retention_class",
            object_store.retention_class.clone(),
            source("object_store", "retention_class"),
        );
    }
}

fn links(
    written: &Value,
    resolved: &ResolvedConfig,
    entries: &mut Vec<ExplainEntry>,
) -> Result<(), ConfigError> {
    // As in `storage`: only a non-null written value is the document's.
    let source = |link: &str, key: &str| {
        let set = written
            .get("links")
            .and_then(|l| l.get(link))
            .and_then(|k| k.get(key))
            .is_some_and(|v| !v.is_null());
        if set { SOURCE_DOCUMENT } else { SOURCE_DEFAULT }
    };
    let links = &resolved.links;
    // An unset sync scope and query timeout come from another plane.
    let sync_scope = links
        .sync
        .sync_scope
        .unwrap_or(resolved.behavior.sync_scope);
    let sync_scope_source = if links.sync.sync_scope.is_some() {
        SOURCE_DOCUMENT
    } else {
        "behavior.sync_scope"
    };
    let timeout = links
        .query
        .timeout_ms
        .unwrap_or(resolved.fanout.per_peer_timeout_ms);
    let timeout_source = if links.query.timeout_ms.is_some() {
        SOURCE_DOCUMENT
    } else {
        "fanout.per_peer_timeout_ms"
    };
    let fields: [(&str, &str, String, &str); 9] = [
        (
            "sync",
            "enabled",
            links.sync.enabled.to_string(),
            source("sync", "enabled"),
        ),
        (
            "sync",
            "adapter",
            links.sync.adapter.clone(),
            source("sync", "adapter"),
        ),
        ("sync", "sync_scope", json(&sync_scope)?, sync_scope_source),
        (
            "sync",
            "retention_class",
            links.sync.retention_class.clone(),
            source("sync", "retention_class"),
        ),
        (
            "query",
            "enabled",
            links.query.enabled.to_string(),
            source("query", "enabled"),
        ),
        ("query", "timeout_ms", timeout.to_string(), timeout_source),
        (
            "send_only",
            "enabled",
            links.send_only.enabled.to_string(),
            source("send_only", "enabled"),
        ),
        (
            "send_only",
            "adapter",
            links.send_only.adapter.clone(),
            source("send_only", "adapter"),
        ),
        (
            "send_only",
            "retention_class",
            links.send_only.retention_class.clone(),
            source("send_only", "retention_class"),
        ),
    ];
    for (link, key, value, from) in fields {
        push(entries, &format!("links.{link}.{key}"), value, from);
    }
    push(
        entries,
        "links.control.enabled",
        "true".to_owned(),
        SOURCE_FIXED,
    );
    Ok(())
}

fn namespaces(written: &Value, resolved: &ResolvedConfig, entries: &mut Vec<ExplainEntry>) {
    // As in `storage`: only a non-null written value is the document's.
    let set = written
        .get("namespaces")
        .and_then(|n| n.get("registry"))
        .is_some_and(|v| !v.is_null());
    push(
        entries,
        "namespaces.registry",
        Value::String(resolved.namespaces.registry.clone()).to_string(),
        if set { SOURCE_DOCUMENT } else { SOURCE_DEFAULT },
    );
}

fn tiers(written: &Value, resolved: &ResolvedConfig, entries: &mut Vec<ExplainEntry>) {
    // As in `storage`: only a non-null written value is the document's.
    let source = |tier: &str, key: &str| {
        let set = written
            .get("tiers")
            .and_then(|t| t.get(tier))
            .and_then(|p| p.get(key))
            .is_some_and(|v| !v.is_null());
        if set { SOURCE_DOCUMENT } else { SOURCE_DEFAULT }
    };
    for (tier, policy) in &resolved.tiers.0 {
        push(
            entries,
            &format!("tiers.{tier}.placement"),
            policy.placement.as_str().to_owned(),
            source(tier, "placement"),
        );
        push(
            entries,
            &format!("tiers.{tier}.retention"),
            policy.retention.clone(),
            source(tier, "retention"),
        );
    }
}

fn extensions(
    resolved: &ResolvedConfig,
    entries: &mut Vec<ExplainEntry>,
) -> Result<(), ConfigError> {
    for (key, value) in &resolved.extensions {
        push(
            entries,
            &format!("extensions.{key}"),
            json(value)?,
            "document (retained, not executed)",
        );
    }
    Ok(())
}
