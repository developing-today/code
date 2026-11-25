//! The peer plane of the config document: `peers[]`.
//!
//! First part of slice 9111-03 in the #9111 record: §3.2 (peer entries),
//! the `peers[]` rule of §6.3 (one entry per peer, a duplicate `peer_id`
//! is a load error), §6.4 (unknown keys in `peers[]` fail closed) and the
//! peer-class consistency checks of §6.5 step 3. The entry is the #9141
//! `PeerPolicy` (record §2 to §4) with the fan-out fields `timeout_ms`
//! and `priority` added.
//!
//! The class is a ceiling and `operations` is the actual grant (#9141
//! §3). Load-time validation rejects a grant the class can never use, so
//! an operator sees the mistake instead of a silent runtime denial:
//!
//! - `relay_granted` only with `fully_trusted` (record §3.2, §7.3);
//! - the `relay` operation only with `fully_trusted` and `relay_granted`;
//! - `send_only` has no `sync.accept` (inbound is class-forbidden);
//! - `accept_signed_from` has no `sync.send` (it grants no outbound sends);
//! - `unknown` has no operations.
//!
//! `peers[].overrides` (per-operation link selection, record §3.3) and
//! `fanout.remotes[]` are later parts of the slice; until then an
//! `overrides` key in a peer entry is an unknown key and rejects the
//! document.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{ConfigError, PER_PEER_TIMEOUT_MS_LIMIT};

/// A peer's trust class (#9141 §2). A directional policy label, never a
/// cryptographic fact. Defaults to [`PeerClass::Unknown`].
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum PeerClass {
    /// Broad configured operations in both directions; never a
    /// validation bypass.
    FullyTrusted,
    /// We send to this peer and never accept peer-originated data.
    SendOnly,
    /// Inbound acceptance only, for objects passing the full gate.
    AcceptSignedFrom,
    /// No configured trust: deny everything.
    #[default]
    Unknown,
}

impl PeerClass {
    /// The wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FullyTrusted => "fully_trusted",
            Self::SendOnly => "send_only",
            Self::AcceptSignedFrom => "accept_signed_from",
            Self::Unknown => "unknown",
        }
    }
}

/// An operation a peer entry may grant (record §3.2, the #9141 matrix
/// operations).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum PeerOperation {
    /// Accept peer-originated data (inbound).
    #[serde(rename = "sync.accept")]
    SyncAccept,
    /// Send already-accepted data to the peer (outbound read).
    #[serde(rename = "sync.send")]
    SyncSend,
    /// Ask this peer a query.
    #[serde(rename = "query.out")]
    QueryOut,
    /// Answer this peer's queries.
    #[serde(rename = "query.answer")]
    QueryAnswer,
    /// Relay a third party's query; governed only by `relay_granted`.
    #[serde(rename = "relay")]
    Relay,
}

impl PeerOperation {
    /// The wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SyncAccept => "sync.accept",
            Self::SyncSend => "sync.send",
            Self::QueryOut => "query.out",
            Self::QueryAnswer => "query.answer",
            Self::Relay => "relay",
        }
    }
}

/// One peer relationship (record §3.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PeerEntry {
    /// The peer's signing identity. Required, non-empty, unique in the
    /// document.
    pub peer_id: String,
    /// Trust class. Default `unknown`.
    #[serde(default)]
    pub class: PeerClass,
    /// Operations granted, each at most once. Default none. The resolved
    /// form is sorted, so authoring order carries no meaning.
    #[serde(default)]
    pub operations: Vec<PeerOperation>,
    /// Separate relay grant. Default false; only with `fully_trusted`.
    #[serde(default)]
    pub relay_granted: bool,
    /// Scope the policy applies to. Absent means the deployment's default
    /// scope; present must be non-empty.
    #[serde(default)]
    pub scope: Option<String>,
    /// Monotonic per-peer policy version (#9141 §2). Required.
    pub policy_version: u64,
    /// RFC 3339 instant the policy comes into force, if bounded.
    #[serde(default)]
    pub valid_from: Option<String>,
    /// RFC 3339 instant the policy stops being in force, if bounded. Must
    /// be after `valid_from` when both are set.
    #[serde(default)]
    pub expires_at: Option<String>,
    /// Per-peer timeout in milliseconds (record §7.2), within
    /// `1 ..= PER_PEER_TIMEOUT_MS_LIMIT`. Default none: the fan-out
    /// `per_peer_timeout_ms` applies.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// Fan-out ask order (record §7.1). Default none: list position.
    #[serde(default)]
    pub priority: Option<u32>,
}

impl PeerEntry {
    /// Check this entry on its own: identity, scope, class consistency,
    /// validity window and timeout.
    fn validate(&self) -> Result<(), ConfigError> {
        let peer = || self.peer_id.clone();
        if self.peer_id.trim().is_empty() {
            return Err(ConfigError::EmptyPeerId);
        }
        if self
            .scope
            .as_deref()
            .is_some_and(|scope| scope.trim().is_empty())
        {
            return Err(ConfigError::EmptyPeerScope(peer()));
        }
        let mut seen = BTreeSet::new();
        for &operation in &self.operations {
            if !seen.insert(operation) {
                return Err(ConfigError::DuplicatePeerOperation {
                    peer_id: peer(),
                    operation: operation.as_str(),
                });
            }
        }
        if let Some(rule) = self.class_conflict() {
            return Err(ConfigError::PeerClassConflict {
                peer_id: peer(),
                class: self.class.as_str(),
                rule,
            });
        }
        let from = self.instant("valid_from", self.valid_from.as_deref())?;
        let until = self.instant("expires_at", self.expires_at.as_deref())?;
        if let (Some(from), Some(until)) = (from, until)
            && until <= from
        {
            return Err(ConfigError::EmptyPeerWindow(peer()));
        }
        if let Some(timeout) = self.timeout_ms
            && (timeout == 0 || timeout > PER_PEER_TIMEOUT_MS_LIMIT)
        {
            return Err(ConfigError::OutOfRange {
                field: "peers[].timeout_ms",
                value: timeout,
                limit: PER_PEER_TIMEOUT_MS_LIMIT,
            });
        }
        Ok(())
    }

    /// The first class-consistency rule this entry breaks, if any.
    fn class_conflict(&self) -> Option<&'static str> {
        let grants = |operation| self.operations.contains(&operation);
        if self.relay_granted && self.class != PeerClass::FullyTrusted {
            return Some("relay_granted is only allowed with class fully_trusted");
        }
        if grants(PeerOperation::Relay) && !self.relay_granted {
            return Some("the relay operation needs relay_granted");
        }
        match self.class {
            PeerClass::Unknown if !self.operations.is_empty() => {
                Some("class unknown grants no operations")
            }
            PeerClass::SendOnly if grants(PeerOperation::SyncAccept) => {
                Some("class send_only never accepts inbound data (sync.accept)")
            }
            PeerClass::AcceptSignedFrom if grants(PeerOperation::SyncSend) => {
                Some("class accept_signed_from grants no outbound sends (sync.send)")
            }
            _ => None,
        }
    }

    fn instant(&self, field: &'static str, text: Option<&str>) -> Result<Option<i64>, ConfigError> {
        text.map(|text| {
            parse_rfc3339(text).ok_or_else(|| ConfigError::InvalidPeerInstant {
                peer_id: self.peer_id.clone(),
                field,
                value: text.to_owned(),
            })
        })
        .transpose()
    }

    /// The resolved form: operations in canonical order.
    fn resolved(&self) -> Self {
        let mut entry = self.clone();
        entry.operations.sort();
        entry
    }
}

/// Validate every peer entry and the one-entry-per-peer rule.
pub(crate) fn validate(peers: &[PeerEntry]) -> Result<(), ConfigError> {
    let mut ids = BTreeSet::new();
    for entry in peers {
        entry.validate()?;
        if !ids.insert(entry.peer_id.as_str()) {
            return Err(ConfigError::DuplicatePeer(entry.peer_id.clone()));
        }
    }
    Ok(())
}

/// The resolved peer plane: sorted by `peer_id`, each entry's operations
/// sorted, so authoring order never changes the digest.
pub(crate) fn resolved(peers: &[PeerEntry]) -> Vec<PeerEntry> {
    let mut out: Vec<PeerEntry> = peers.iter().map(PeerEntry::resolved).collect();
    out.sort_by(|a, b| a.peer_id.cmp(&b.peer_id));
    out
}

/// Seconds since the Unix epoch for an RFC 3339 date-time
/// (`YYYY-MM-DDTHH:MM:SS[.fraction](Z|+HH:MM|-HH:MM)`), or `None` if the
/// text is not one. The fraction is checked and dropped.
fn parse_rfc3339(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    if bytes.len() < 20 || !text.is_ascii() {
        return None;
    }
    let number = |range: std::ops::Range<usize>| -> Option<i64> {
        let part = &text[range];
        if part.bytes().all(|b| b.is_ascii_digit()) {
            part.parse().ok()
        } else {
            None
        }
    };
    let separators = [(4, b'-'), (7, b'-'), (13, b':'), (16, b':')];
    if separators.iter().any(|&(at, sep)| bytes[at] != sep) || !matches!(bytes[10], b'T' | b't') {
        return None;
    }
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) {
        return None;
    }
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let mut rest = &text[19..];
    if let Some(fraction) = rest.strip_prefix('.') {
        let digits = fraction.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        rest = &fraction[digits..];
    }
    let offset = match rest {
        "Z" | "z" => 0,
        _ => {
            let sign = match rest.as_bytes().first()? {
                b'+' => 1,
                b'-' => -1,
                _ => return None,
            };
            if rest.len() != 6 || rest.as_bytes()[3] != b':' {
                return None;
            }
            let hours: i64 = rest[1..3].parse().ok()?;
            let minutes: i64 = rest[4..6].parse().ok()?;
            if !rest[1..3]
                .bytes()
                .chain(rest[4..6].bytes())
                .all(|b| b.is_ascii_digit())
                || hours > 23
                || minutes > 59
            {
                return None;
            }
            sign * (hours * 3600 + minutes * 60)
        }
    };
    let days = days_from_civil(year, month, day);
    Some(days * 86_400 + hour * 3600 + minute * 60 + second - offset)
}

const fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

const fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if is_leap_year(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days since 1970-01-01 for a proleptic Gregorian date (H. Hinnant's
/// `days_from_civil`).
const fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let shifted_month = (month + 9) % 12;
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::parse_rfc3339;

    #[test]
    fn rfc3339_instants() {
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339("2000-03-01T00:00:00Z"), Some(951_868_800));
        assert_eq!(
            parse_rfc3339("2026-11-25T06:00:00.123Z"),
            Some(1_795_586_400)
        );
        assert_eq!(
            parse_rfc3339("2026-11-25T07:30:00+01:30"),
            parse_rfc3339("2026-11-25T06:00:00Z")
        );
        assert_eq!(parse_rfc3339("2024-02-29T00:00:00Z"), Some(1_709_164_800));
        for bad in [
            "",
            "2026-11-25",
            "2026-11-25 06:00:00Z",
            "2026-13-01T00:00:00Z",
            "2023-02-29T00:00:00Z",
            "2026-11-25T24:00:00Z",
            "2026-11-25T06:00:00",
            "2026-11-25T06:00:00.Z",
            "2026-11-25T06:00:00+0100",
            "2026-11-25T06:00:00+01:60",
            "+026-11-25T06:00:00Z",
        ] {
            assert_eq!(parse_rfc3339(bad), None, "{bad:?}");
        }
    }
}
