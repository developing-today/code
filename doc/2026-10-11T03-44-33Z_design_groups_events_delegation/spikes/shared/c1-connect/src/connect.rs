//! A declarative profile for an iroh endpoint, and the one function that binds it.
//!
//! Nothing is on by default. A profile names each relay and each lookup it wants. Every
//! publisher takes the profile's address policy, so an endpoint publishes only what the
//! policy allows and nothing the profile did not name.

use anyhow::{Result, bail};
use iroh::address_lookup::{
    AddrFilter, DnsAddressLookup, MemoryLookup, PkarrPublisher, PkarrResolver,
};
use iroh::endpoint::presets::Minimal;
use iroh::{Endpoint, EndpointAddr, RelayConfig, RelayMode, RelayUrl, TransportAddr};
use iroh_mainline_address_lookup::DhtAddressLookup;
use iroh_mdns_address_lookup::MdnsAddressLookup;
use n0_mainline::DhtBuilder;

/// Which addresses may be published and carried in tickets.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AddressPolicy {
    /// Relay URLs only. No direct IP address leaves the endpoint. The default.
    #[default]
    RelayOnly,
    /// Relay URLs and direct IP addresses.
    RelayAndIp,
}

impl AddressPolicy {
    /// The filter the endpoint and each publisher apply before publishing.
    pub fn filter(self) -> AddrFilter {
        match self {
            Self::RelayOnly => AddrFilter::relay_only(),
            Self::RelayAndIp => AddrFilter::unfiltered(),
        }
    }

    /// `addr` reduced to what this policy allows. Use it for tickets.
    pub fn apply(self, addr: &EndpointAddr) -> EndpointAddr {
        match self {
            Self::RelayOnly => EndpointAddr::from_parts(
                addr.id,
                addr.relay_urls().cloned().map(TransportAddr::Relay),
            ),
            Self::RelayAndIp => addr.clone(),
        }
    }
}

/// n0's production relays, the list that `RelayMode::Default` uses.
pub fn n0_relays() -> Vec<RelayUrl> {
    iroh::defaults::prod::default_relay_map().urls()
}

/// A relay in a profile, and whether this endpoint runs QUIC address discovery (QAD) against it.
///
/// QAD needs a UDP port on the relay. A relay behind a Cloudflare Tunnel has none, so its entry
/// must say `quic: false`. A bare `RelayUrl` converts to `quic: true`, iroh's default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayEntry {
    pub url: RelayUrl,
    pub quic: bool,
}

impl RelayEntry {
    /// The config iroh's relay map holds for this relay. `quic: false` passes `None`, so iroh
    /// sends no QAD probe to it.
    fn config(&self) -> RelayConfig {
        if self.quic {
            RelayConfig::from(self.url.clone())
        } else {
            RelayConfig::new(self.url.clone(), None)
        }
    }
}

impl From<RelayUrl> for RelayEntry {
    fn from(url: RelayUrl) -> Self {
        Self { url, quic: true }
    }
}

/// Lookup switches. All off by default.
#[derive(Debug, Clone, Default)]
pub struct Lookups {
    /// n0's pkarr publisher (`https://dns.iroh.link/pkarr`). Publishes this endpoint's record.
    pub n0_pkarr_publish: bool,
    /// n0's pkarr resolver (same server). Resolves other endpoints' records.
    pub n0_pkarr_resolve: bool,
    /// n0's DNS lookup (`_iroh.<id>.dns.iroh.link`). Resolve only.
    pub n0_dns_resolve: bool,
    /// The mainline DHT on explicit bootstrap nodes.
    pub mainline: Option<Mainline>,
    /// mDNS on the local network.
    pub mdns: Option<Mdns>,
    /// In-memory lookup, fed by tickets. Returned as `Bound::memory`.
    pub memory: bool,
}

/// The mainline DHT lookup.
#[derive(Debug, Clone)]
pub struct Mainline {
    /// Bootstrap nodes as `host:port`. Required and non-empty. iroh's default is the public DHT.
    pub bootstrap: Vec<String>,
    /// Store this endpoint's record. `false` resolves other endpoints only.
    pub publish: bool,
}

/// The mDNS lookup.
#[derive(Debug, Clone, Copy)]
pub struct Mdns {
    /// Announce this endpoint on the LAN. `false` browses only.
    pub advertise: bool,
}

/// Everything needed to bind one endpoint.
#[derive(Debug, Clone, Default)]
pub struct Profile {
    /// Relays in this endpoint's relay map, each with its QAD setting. Empty means no relay.
    pub relays: Vec<RelayEntry>,
    pub lookups: Lookups,
    pub policy: AddressPolicy,
    /// ALPNs this endpoint accepts. Empty for a dial-only endpoint.
    pub alpns: Vec<Vec<u8>>,
}

impl Profile {
    /// One line for logs: what this profile will use and publish.
    pub fn summary(&self) -> String {
        let l = &self.lookups;
        let mut lookups = Vec::new();
        if l.n0_pkarr_publish {
            lookups.push("n0-pkarr publish".to_string());
        }
        if l.n0_pkarr_resolve {
            lookups.push("n0-pkarr resolve".to_string());
        }
        if l.n0_dns_resolve {
            lookups.push("n0-dns resolve".to_string());
        }
        if let Some(m) = &l.mainline {
            lookups.push(format!(
                "mainline(bootstrap={}, publish={})",
                m.bootstrap.len(),
                m.publish
            ));
        }
        if let Some(m) = l.mdns {
            lookups.push(format!("mdns(advertise={})", m.advertise));
        }
        if l.memory {
            lookups.push("memory".to_string());
        }
        let lookups = if lookups.is_empty() {
            "no lookups".to_string()
        } else {
            lookups.join(", ")
        };
        let no_quic = self.relays.iter().filter(|r| !r.quic).count();
        format!(
            "{} relay(s) ({no_quic} with QAD off), policy {:?}, {lookups}",
            self.relays.len(),
            self.policy
        )
    }
}

/// A bound endpoint and the handles its profile asked for.
pub struct Bound {
    pub endpoint: Endpoint,
    /// The in-memory lookup, if `Lookups::memory` was set.
    pub memory: Option<MemoryLookup>,
}

/// Binds an endpoint with exactly the relays, lookups, and policy in `profile`.
///
/// Each call gets a fresh random key, because the profile never sets one.
pub async fn bind(profile: &Profile) -> Result<Bound> {
    let filter = profile.policy.filter();
    let relay_mode = if profile.relays.is_empty() {
        RelayMode::Disabled
    } else {
        // Not `RelayMode::custom`: that gives every relay the QUIC default, so QAD runs for all.
        RelayMode::Custom(profile.relays.iter().map(RelayEntry::config).collect())
    };
    // The endpoint-level filter applies to every service, and each publisher applies its own
    // filter after it. pkarr and the DHT default to relay-only, which would drop IPs under
    // RelayAndIp, so the policy's filter is set on each one. mDNS defaults to no filter.
    let mut builder = Endpoint::builder(Minimal)
        .alpns(profile.alpns.clone())
        .relay_mode(relay_mode)
        .addr_filter(filter.clone());

    let l = &profile.lookups;
    if l.n0_pkarr_publish {
        builder = builder.address_lookup(PkarrPublisher::n0_dns().addr_filter(filter.clone()));
    }
    if l.n0_pkarr_resolve {
        builder = builder.address_lookup(PkarrResolver::n0_dns());
    }
    if l.n0_dns_resolve {
        builder = builder.address_lookup(DnsAddressLookup::n0_dns());
    }
    if let Some(m) = &l.mainline {
        // iroh's default bootstrap is the public DHT. An empty list is refused, not defaulted.
        if m.bootstrap.is_empty() {
            bail!("mainline lookup needs explicit bootstrap nodes");
        }
        let mut dht = DhtBuilder::default();
        dht.bootstrap(&m.bootstrap);
        let mut mainline = DhtAddressLookup::builder()
            .dht_builder(dht)
            .addr_filter(filter.clone());
        if !m.publish {
            mainline = mainline.no_publish();
        }
        builder = builder.address_lookup(mainline);
    }
    if let Some(m) = l.mdns {
        // mDNS advertises by default and has no filter by default. Set both from the profile.
        builder = builder.address_lookup(
            MdnsAddressLookup::builder()
                .advertise(m.advertise)
                .addr_filter(filter.clone()),
        );
    }
    let memory = l.memory.then(MemoryLookup::new);
    if let Some(memory) = &memory {
        builder = builder.address_lookup(memory.clone());
    }

    let endpoint = builder.bind().await?;
    Ok(Bound { endpoint, memory })
}
