//! Opt-in mainline DHT address lookup (P7.3, decision 15).
//!
//! The DHT is off by default. A world turns it on through this server's own
//! setting, which is local and is not a protocol rule. The scope is the
//! endpoint, not the world. iroh fixes an endpoint's address lookups when the
//! endpoint is bound. `AddressLookup::publish` is called once for the
//! endpoint's own address, and `AddressLookup::resolve` takes only the peer's
//! ID, with no world. So one opted-in world turns the DHT on for every world
//! this endpoint serves, and the endpoint publishes its record to the DHT. An
//! opt-out takes effect on the next session, when the endpoint is bound
//! again. A record already published stays on the DHT until its TTL ends.

use std::collections::BTreeSet;

use iroh::{address_lookup::AddressLookupBuilder, endpoint::Builder};

/// Whether an endpoint uses the mainline DHT for address lookup.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum DhtPolicy {
    /// No DHT: no publish and no lookup. The default.
    #[default]
    Off,
    /// The DHT is enabled as a server-wide setting.
    Server,
    /// The DHT is on for the endpoint because these worlds opted in. Worlds
    /// that did not opt in share the endpoint, so they use the DHT too.
    Endpoint(BTreeSet<String>),
}

impl DhtPolicy {
    /// Server-wide DHT policy.
    pub const fn from_server(enabled: bool) -> Self {
        if enabled { Self::Server } else { Self::Off }
    }

    /// The policy for the worlds that opted in. With no world opted in, the
    /// DHT is off.
    pub fn from_worlds<I, S>(opted_in: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let worlds: BTreeSet<String> = opted_in.into_iter().map(Into::into).collect();
        if worlds.is_empty() {
            Self::Off
        } else {
            Self::Endpoint(worlds)
        }
    }

    /// Whether the endpoint uses the DHT.
    pub const fn is_on(&self) -> bool {
        matches!(self, Self::Server | Self::Endpoint(_))
    }

    /// The worlds that opted in, in name order. Empty when the DHT is off or server-wide.
    pub fn opted_in(&self) -> impl Iterator<Item = &str> {
        let worlds = match self {
            Self::Off | Self::Server => None,
            Self::Endpoint(worlds) => Some(worlds),
        };
        worlds.into_iter().flatten().map(String::as_str)
    }
}

/// Mounts `lookup` on an endpoint builder when `policy` turns the DHT on. With
/// the DHT off, `lookup` is dropped unmounted, so the endpoint never publishes
/// to it and never asks it for a peer.
pub fn with_lookup(
    policy: &DhtPolicy,
    builder: Builder,
    lookup: impl AddressLookupBuilder,
) -> Builder {
    if policy.is_on() {
        builder.address_lookup(lookup)
    } else {
        builder
    }
}

/// The mainline DHT lookup. It publishes the endpoint's relay addresses,
/// signed with the endpoint's key, and resolves peers by ID. Only the `dht`
/// feature builds it.
#[cfg(feature = "dht")]
pub fn mainline() -> impl AddressLookupBuilder {
    iroh_mainline_address_lookup::DhtAddressLookup::builder()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_default_policy_is_off() {
        assert_eq!(DhtPolicy::default(), DhtPolicy::Off);
        assert!(!DhtPolicy::default().is_on());
    }

    #[test]
    fn test_server_wide_dht_policy() {
        assert_eq!(DhtPolicy::from_server(false), DhtPolicy::Off);
        assert!(!DhtPolicy::from_server(false).is_on());

        let policy = DhtPolicy::from_server(true);
        assert_eq!(policy, DhtPolicy::Server);
        assert!(policy.is_on());
        assert_eq!(policy.opted_in().count(), 0);
    }

    #[test]
    fn test_no_opted_in_world_is_off() {
        let policy = DhtPolicy::from_worlds(Vec::<String>::new());
        assert_eq!(policy, DhtPolicy::Off);
        assert_eq!(policy.opted_in().count(), 0);
    }

    /// One opted-in world turns the DHT on for the endpoint, and the other
    /// worlds on that endpoint are not asked.
    #[test]
    fn test_one_opted_in_world_turns_the_dht_on_for_the_endpoint() {
        let policy = DhtPolicy::from_worlds(["world-a"]);
        assert!(policy.is_on());
        assert_eq!(policy.opted_in().collect::<Vec<_>>(), ["world-a"]);
    }

    #[test]
    fn test_opted_in_worlds_are_listed_in_name_order_without_repeats() {
        let policy = DhtPolicy::from_worlds(["world-b", "world-a", "world-b"]);
        assert_eq!(
            policy.opted_in().collect::<Vec<_>>(),
            ["world-a", "world-b"]
        );
    }
}
