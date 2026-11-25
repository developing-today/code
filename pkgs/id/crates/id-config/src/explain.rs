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
    fanout(&written, &resolved, &mut entries);
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

/// Each ceiling with the scope that narrowed it last, or `document`.
fn ceilings(document: &ConfigDocument, resolved: &ResolvedConfig, entries: &mut Vec<ExplainEntry>) {
    let mut ordered: Vec<_> = document.overrides.iter().collect();
    ordered.sort_by(|a, b| a.scope.cmp(&b.scope));
    let fields: [(&str, u64, u64, fn(&CeilingOverride) -> Option<u64>); 4] = [
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
            if let Some(value) = narrowing(&scoped.ceilings) {
                if value < current {
                    current = value;
                    source = format!("override:{}", scoped.scope);
                }
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

fn fanout(written: &Value, resolved: &ResolvedConfig, entries: &mut Vec<ExplainEntry>) {
    let set = |key: &str| written.get("fanout").and_then(|f| f.get(key)).is_some();
    let source = |key: &str| {
        if set(key) {
            SOURCE_DOCUMENT
        } else {
            SOURCE_DEFAULT
        }
    };
    let fanout = resolved.fanout;
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
        .unwrap_or(Fanout::default().per_peer_timeout_ms);
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
