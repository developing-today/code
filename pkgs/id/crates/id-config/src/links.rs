//! The link plane of the config document: `links`.
//!
//! Fifth part of slice 9111-03 in the #9111 record: §3.3 (link kinds and
//! their per-kind defaults), with the object merge rule of §6.3 (an absent
//! key or an explicit `null` takes the built-in default), §6.4 (unknown
//! keys fail closed, the `control` link included) and §6.5 step 4 (each
//! adapter named in `links` is advertised by this build).
//!
//! A link kind is a transport capability whose wire contract is fixed by
//! its protocol; config only chooses the per-kind defaults. Who may use a
//! link is not here: that is the peer's class and operations
//! (`peers[]`), which no link setting can widen.
//!
//! - `sync` carries signed events and objects. Keys: `enabled`,
//!   `adapter`, `sync_scope`, `retention_class`.
//! - `query` carries read-only queries and signed answers. Keys:
//!   `enabled`, `timeout_ms` (a direct query's timeout; fan-out timing
//!   stays in `fanout`).
//! - `send_only` is outbound-only delivery. Keys: `enabled`, `adapter`,
//!   `retention_class`.
//! - `control` carries emergency peer disable and key revocation. It is
//!   always on and has no keys (record §5.3): `{}` is accepted and any key
//!   in it rejects the document.
//!
//! Defaults (record §3.1: "sync and query disabled"; §4: `single-node`
//! has "`links` all off"): every configurable kind is off, on the `iroh`
//! adapter, with the `default` retention class; `sync_scope` unset means
//! the resolved `behavior.sync_scope` applies; the query timeout is the
//! fan-out per-peer timeout. This build advertises only `iroh`.

use serde::{Deserialize, Serialize};

use crate::{ConfigError, PER_PEER_TIMEOUT_MS_LIMIT, SyncScope};

/// The link adapters this build advertises (record §6.5 step 4).
pub const ADVERTISED_LINK_ADAPTERS: &[&str] = &["iroh"];

/// The default link adapter.
pub const DEFAULT_LINK_ADAPTER: &str = "iroh";

/// The default retention class on a link.
pub const DEFAULT_LINK_RETENTION_CLASS: &str = "default";

/// `links.sync` as written.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyncLink {
    /// Whether sync links are on. Default false.
    #[serde(default)]
    pub enabled: Option<bool>,
    /// The adapter. Default `iroh`.
    #[serde(default)]
    pub adapter: Option<String>,
    /// How much a sync link carries. Default: `behavior.sync_scope`.
    #[serde(default)]
    pub sync_scope: Option<SyncScope>,
    /// The retention class. Default `default`.
    #[serde(default)]
    pub retention_class: Option<String>,
}

/// `links.query` as written.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueryLink {
    /// Whether query links are on. Default false.
    #[serde(default)]
    pub enabled: Option<bool>,
    /// A direct query's timeout in milliseconds, within
    /// `1 ..= PER_PEER_TIMEOUT_MS_LIMIT`. Default: the resolved
    /// `fanout.per_peer_timeout_ms`.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

/// `links.send_only` as written.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SendOnlyLink {
    /// Whether send-only links are on. Default false.
    #[serde(default)]
    pub enabled: Option<bool>,
    /// The adapter. Default `iroh`.
    #[serde(default)]
    pub adapter: Option<String>,
    /// The retention class. Default `default`.
    #[serde(default)]
    pub retention_class: Option<String>,
}

/// `links.control`: always on, no keys. Braced so that `{}` parses and
/// any key inside it rejects the document (record §6.4).
#[allow(clippy::empty_structs_with_brackets)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlLink {}

/// The `links` section as written (record §3.3).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Links {
    /// Sync link defaults.
    #[serde(default)]
    pub sync: Option<SyncLink>,
    /// Query link defaults.
    #[serde(default)]
    pub query: Option<QueryLink>,
    /// Send-only link defaults.
    #[serde(default)]
    pub send_only: Option<SendOnlyLink>,
    /// The control link. Accepted only as `{}` or `null`.
    #[serde(default)]
    pub control: Option<ControlLink>,
}

/// Effective `links.sync`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedSyncLink {
    /// Whether sync links are on.
    pub enabled: bool,
    /// The adapter.
    pub adapter: String,
    /// The sync scope, or `None` when `behavior.sync_scope` applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_scope: Option<SyncScope>,
    /// The retention class.
    pub retention_class: String,
}

/// Effective `links.query`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedQueryLink {
    /// Whether query links are on.
    pub enabled: bool,
    /// A direct query's timeout in milliseconds, or `None` when the
    /// fan-out per-peer timeout applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

/// Effective `links.send_only`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedSendOnlyLink {
    /// Whether send-only links are on.
    pub enabled: bool,
    /// The adapter.
    pub adapter: String,
    /// The retention class.
    pub retention_class: String,
}

/// The effective link plane. The control link is always on and is not
/// listed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedLinks {
    /// Sync link defaults.
    pub sync: ResolvedSyncLink,
    /// Query link defaults.
    pub query: ResolvedQueryLink,
    /// Send-only link defaults.
    pub send_only: ResolvedSendOnlyLink,
}

impl Default for ResolvedLinks {
    fn default() -> Self {
        Links::default().resolved()
    }
}

impl ResolvedLinks {
    /// Whether this is the built-in default, which the canonical encoding
    /// omits so a document without `links` keeps its digest.
    #[must_use]
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

fn adapter_or_default(adapter: Option<&String>) -> String {
    adapter.map_or_else(|| DEFAULT_LINK_ADAPTER.to_owned(), Clone::clone)
}

fn retention_or_default(retention: Option<&String>) -> String {
    retention.map_or_else(|| DEFAULT_LINK_RETENTION_CLASS.to_owned(), Clone::clone)
}

impl Links {
    /// The effective defaults: every absent key takes its default.
    #[must_use]
    pub fn resolved(&self) -> ResolvedLinks {
        let sync = self.sync.clone().unwrap_or_default();
        let query = self.query.clone().unwrap_or_default();
        let send_only = self.send_only.clone().unwrap_or_default();
        ResolvedLinks {
            sync: ResolvedSyncLink {
                enabled: sync.enabled.unwrap_or(false),
                adapter: adapter_or_default(sync.adapter.as_ref()),
                sync_scope: sync.sync_scope,
                retention_class: retention_or_default(sync.retention_class.as_ref()),
            },
            query: ResolvedQueryLink {
                enabled: query.enabled.unwrap_or(false),
                timeout_ms: query.timeout_ms,
            },
            send_only: ResolvedSendOnlyLink {
                enabled: send_only.enabled.unwrap_or(false),
                adapter: adapter_or_default(send_only.adapter.as_ref()),
                retention_class: retention_or_default(send_only.retention_class.as_ref()),
            },
        }
    }

    /// Check each adapter against what this build advertises, each
    /// retention class, and the query timeout's range.
    pub(crate) fn validate(&self) -> Result<(), ConfigError> {
        let resolved = self.resolved();
        for (link, adapter, retention) in [
            (
                "sync",
                &resolved.sync.adapter,
                &resolved.sync.retention_class,
            ),
            (
                "send_only",
                &resolved.send_only.adapter,
                &resolved.send_only.retention_class,
            ),
        ] {
            if !ADVERTISED_LINK_ADAPTERS.contains(&adapter.as_str()) {
                return Err(ConfigError::UnadvertisedLinkAdapter {
                    link,
                    adapter: adapter.clone(),
                });
            }
            if retention.trim().is_empty() {
                return Err(ConfigError::EmptyLinkRetentionClass(link));
            }
        }
        if let Some(timeout) = resolved.query.timeout_ms
            && (timeout == 0 || timeout > PER_PEER_TIMEOUT_MS_LIMIT)
        {
            return Err(ConfigError::OutOfRange {
                field: "links.query.timeout_ms",
                value: timeout,
                limit: PER_PEER_TIMEOUT_MS_LIMIT,
            });
        }
        Ok(())
    }
}
