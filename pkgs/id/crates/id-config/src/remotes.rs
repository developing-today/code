//! The fan-out remote list: `fanout.remotes[]`.
//!
//! Fourth part of slice 9111-03 in the #9111 record: §7.1 (the remote
//! list), §7.2 (each remote's timeout) and §7.3 (relay permission).
//!
//! A remote names a peer of the `peers[]` plane that fan-out queries may
//! ask, with its ask order, whether it may relay, and optionally its own
//! timeout and query scope. Absent means `[]`: local-only queries.
//!
//! Load rejects a remote that:
//! - names a `peer_id` with no `peers[]` entry, or one already listed;
//! - names a peer that can never be asked: its class is `send_only` or
//!   `unknown`, or its entry does not grant `query.out` (record §7.1,
//!   "a send_only peer is never asked");
//! - asks to relay through a peer that is not `fully_trusted` with
//!   `relay_granted` (record §7.3, a load error, never a silent drop);
//! - has a `timeout_ms` outside `1 ..= PER_PEER_TIMEOUT_MS_LIMIT`, or an
//!   empty `scope`.
//!
//! `priority` is ask order only, never authority (record §7.1 from #9144
//! §3.2). The resolved list is sorted by `priority`, ties kept in list
//! order, and each entry carries its effective timeout, which is
//! `min(its timeout_ms, else the peer's timeout_ms, else
//! fanout.per_peer_timeout_ms; fanout.deadline_ms)` (record §7.2), and its
//! effective scope, which is its own `scope` or else the peer's.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{ConfigError, PER_PEER_TIMEOUT_MS_LIMIT, PeerClass, PeerEntry, PeerOperation};

/// One fan-out remote (record §7.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FanoutRemote {
    /// The peer to ask. Must have a `peers[]` entry that grants
    /// `query.out`; at most one remote per peer.
    pub peer_id: String,
    /// Ask order, lowest first; ties keep list order. Ordering only.
    pub priority: u32,
    /// Whether this peer may relay the query onward. Default false;
    /// `true` needs a `fully_trusted` peer with `relay_granted`.
    #[serde(default)]
    pub relay: bool,
    /// This remote's timeout in milliseconds. In the resolved config it
    /// is always set, to the effective timeout (record §7.2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// The allowed query scope. Default: the peer's `scope`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

impl FanoutRemote {
    /// Why this remote can never be asked through `peer`, if it can't.
    fn unaskable(peer: &PeerEntry) -> Option<&'static str> {
        match peer.class {
            PeerClass::SendOnly => Some("class send_only is never asked"),
            PeerClass::Unknown => Some("class unknown is never asked"),
            PeerClass::FullyTrusted | PeerClass::AcceptSignedFrom
                if !peer.operations.contains(&PeerOperation::QueryOut) =>
            {
                Some("its peers[] entry does not grant query.out")
            }
            PeerClass::FullyTrusted | PeerClass::AcceptSignedFrom => None,
        }
    }
}

fn peer<'a>(peers: &'a [PeerEntry], peer_id: &str) -> Option<&'a PeerEntry> {
    peers.iter().find(|entry| entry.peer_id == peer_id)
}

/// Check every remote against the peer plane. Runs after the peer plane
/// itself is valid.
pub(crate) fn validate(remotes: &[FanoutRemote], peers: &[PeerEntry]) -> Result<(), ConfigError> {
    let mut seen = BTreeSet::new();
    for remote in remotes {
        let id = || remote.peer_id.clone();
        let Some(entry) = peer(peers, &remote.peer_id) else {
            return Err(ConfigError::UnknownRemotePeer(id()));
        };
        if !seen.insert(remote.peer_id.as_str()) {
            return Err(ConfigError::DuplicateRemote(id()));
        }
        if let Some(rule) = FanoutRemote::unaskable(entry) {
            return Err(ConfigError::RemoteNotAskable {
                peer_id: id(),
                rule,
            });
        }
        if remote.relay && !(entry.class == PeerClass::FullyTrusted && entry.relay_granted) {
            return Err(ConfigError::RemoteRelayNotGranted(id()));
        }
        if let Some(timeout) = remote.timeout_ms
            && (timeout == 0 || timeout > PER_PEER_TIMEOUT_MS_LIMIT)
        {
            return Err(ConfigError::OutOfRange {
                field: "fanout.remotes[].timeout_ms",
                value: timeout,
                limit: PER_PEER_TIMEOUT_MS_LIMIT,
            });
        }
        if remote
            .scope
            .as_deref()
            .is_some_and(|scope| scope.trim().is_empty())
        {
            return Err(ConfigError::EmptyRemoteScope(id()));
        }
    }
    Ok(())
}

/// The resolved remote list: sorted by priority (stable, so ties keep
/// list order), each with its effective timeout and scope.
///
/// `per_peer_timeout_ms` is the fan-out per-peer timeout already clamped
/// to `deadline_ms`.
pub(crate) fn resolved(
    remotes: &[FanoutRemote],
    peers: &[PeerEntry],
    per_peer_timeout_ms: u64,
    deadline_ms: u64,
) -> Vec<FanoutRemote> {
    let mut out: Vec<FanoutRemote> = remotes
        .iter()
        .map(|remote| {
            let entry = peer(peers, &remote.peer_id);
            let timeout = remote
                .timeout_ms
                .or_else(|| entry.and_then(|e| e.timeout_ms))
                .unwrap_or(per_peer_timeout_ms)
                .min(deadline_ms);
            FanoutRemote {
                timeout_ms: Some(timeout),
                scope: remote
                    .scope
                    .clone()
                    .or_else(|| entry.and_then(|e| e.scope.clone())),
                ..remote.clone()
            }
        })
        .collect();
    out.sort_by_key(|remote| remote.priority);
    out
}
