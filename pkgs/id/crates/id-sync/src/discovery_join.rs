//! The world join path (#15646): a client's discovery settings
//! applied to the resolver and then to the connection.
//!
//! [`crate::discovery_client`] holds the settings and
//! [`crate::discovery_resolver`] finds a world's freshest valid
//! announcement. This module joins the two in the order design #6287
//! §§5–7 sets, so the settings hold at the join itself and not only
//! where they are stored:
//!
//! 1. **Which registries.** Only the registries the client subscribes
//!    to are asked, in the order the operator added them. A fetched
//!    list from any other registry is dropped, so a fresh self-host,
//!    which follows none, resolves from the invite link and pkarr
//!    alone until its operator chooses a registry (§5).
//! 2. **Which layers.** The resolver asks the invite link, then pkarr,
//!    then those registries, and keeps the freshest valid answer
//!    (§6). A private world has no announcement anywhere, so only its
//!    invite link finds it; a registry that lists an unlisted world
//!    anyway is refused by the resolver.
//! 3. **How to connect.** The chosen announcement names the host's
//!    node and, optionally, a relay hint. A client in relay-required
//!    mode connects only through that relay and never opens a direct
//!    connection; a world that publishes no relay cannot be joined in
//!    that mode, and the plan says so rather than falling back (§7).

use anyhow::{Result, bail};

use crate::announcement::{SignedAnnouncement, Subject};
use crate::announcement_relay::RelaySinks;
use crate::bridge_http::BridgeHttp;
use crate::discovery_client::{
    ClientDiscoveryConfig, RELAY_REQUIRED_DISCLOSURE, direct_connections_allowed,
};
use crate::discovery_resolver::{Resolution, Resolver};
use crate::registry::SignedRegistryList;

/// How the client reaches the world's host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JoinRoute {
    /// Relay-required mode: only through this relay. The peer sees the
    /// relay's address, never the joiner's.
    RelayOnly {
        /// The relay the world's announcement names.
        relay: String,
    },
    /// Direct connections are allowed. The relay hint, when the world
    /// publishes one, is a fallback. A direct connection exposes the
    /// joiner's IP to the peer, which the client must say in its UI.
    DirectAllowed {
        /// The relay the world's announcement names, if any.
        relay: Option<String>,
    },
}

impl JoinRoute {
    /// Whether this route may open a direct (hole-punched) connection.
    #[must_use]
    pub const fn opens_direct(&self) -> bool {
        matches!(self, Self::DirectAllowed { .. })
    }

    /// The disclosure the client shows for this route: the
    /// relay-required text, or the warning that a direct connection
    /// exposes the joiner's IP to the peer.
    #[must_use]
    pub const fn disclosure(&self) -> &'static str {
        match self {
            Self::RelayOnly { .. } => RELAY_REQUIRED_DISCLOSURE,
            Self::DirectAllowed { .. } => DIRECT_DISCLOSURE,
        }
    }
}

/// The UI disclosure for a route that may connect directly (design §7).
pub const DIRECT_DISCLOSURE: &str = "Direct connections are on: the world's host can see your IP. Turn on relay-required to connect only via relay.";

/// Where a join goes: the resolved announcement, the host's node, and
/// the route the client's settings allow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JoinPlan {
    /// The resolver's answer, with every layer's outcome.
    pub resolution: Resolution,
    /// The host's iroh node id, from the announcement.
    pub node_id: [u8; 32],
    /// How to reach it.
    pub route: JoinRoute,
}

/// The resolver a client's settings allow: `pkarr` relays (none skips
/// that layer) and, of the `fetched` registry lists, only those the
/// client subscribes to, in subscription order.
///
/// A list from a registry the client does not follow is dropped, and
/// so is a second list for the same registry; verify each list for
/// its registry key when it is fetched.
#[must_use]
pub fn client_resolver(
    config: &ClientDiscoveryConfig,
    pkarr: Option<RelaySinks>,
    fetched: Vec<SignedRegistryList>,
) -> Resolver {
    let mut fetched = fetched;
    let mut registries = Vec::with_capacity(config.subscribed_registries.len());
    for key in &config.subscribed_registries {
        if let Some(index) = fetched
            .iter()
            .position(|list| list.list.registry_key == *key)
        {
            registries.push(fetched.swap_remove(index));
        }
    }
    Resolver::new(pkarr, registries)
}

/// The route the client's settings allow to an announced endpoint.
///
/// # Errors
///
/// Fails in relay-required mode when the world names no relay, since
/// the only way left would be a direct connection.
pub fn route_for(config: &ClientDiscoveryConfig, relay: Option<&str>) -> Result<JoinRoute> {
    if direct_connections_allowed(config.relay_required) {
        return Ok(JoinRoute::DirectAllowed {
            relay: relay.map(str::to_owned),
        });
    }
    match relay {
        Some(relay) => Ok(JoinRoute::RelayOnly {
            relay: relay.to_owned(),
        }),
        None => bail!(
            "relay-required is on and this world publishes no relay, so joining it would need a direct connection"
        ),
    }
}

/// Plan a join of `world_id` under `config`.
///
/// Resolves through [`client_resolver`] (the invite link's
/// announcement first, if the client holds one), then picks the route
/// with [`route_for`].
///
/// # Errors
///
/// Fails when no allowed layer served a valid announcement, when the
/// announcement names a program rather than a world endpoint, or when
/// relay-required mode leaves no route.
pub async fn plan_join<H: BridgeHttp>(
    config: &ClientDiscoveryConfig,
    pkarr: Option<RelaySinks>,
    fetched: Vec<SignedRegistryList>,
    http: &mut H,
    world_id: &[u8; 32],
    invite: Option<SignedAnnouncement>,
    now: u64,
) -> Result<JoinPlan> {
    let resolver = client_resolver(config, pkarr, fetched);
    let resolution = resolver.resolve(http, world_id, invite, now).await?;
    let (node_id, route) = match &resolution.announcement.announcement.subject {
        Subject::Endpoint { node_id, relay } => (*node_id, route_for(config, relay.as_deref())?),
        Subject::Program { .. } => {
            bail!("the announcement names a program, not a world to join")
        }
    };
    Ok(JoinPlan {
        resolution,
        node_id,
        route,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use ed25519_dalek::SigningKey;

    use super::*;
    use crate::announcement::{Announcement, Occupancy, Visibility};
    use crate::bridge_http::{HttpRequest, HttpResponse};
    use crate::discovery_resolver::{Layer, Outcome};
    use crate::registry::Registry;

    const NOW: u64 = 1_763_500_000;
    const RELAY: &str = "https://relay.example";

    fn world_key() -> SigningKey {
        SigningKey::from_bytes(&[7; 32])
    }

    fn world_id() -> [u8; 32] {
        world_key().verifying_key().to_bytes()
    }

    fn announced(seq: u64, visibility: Visibility, relay: Option<&str>) -> SignedAnnouncement {
        let key = world_key();
        SignedAnnouncement::sign(
            &key,
            Announcement {
                world_id: world_id(),
                name: "Harbor".to_owned(),
                description: "a quiet world".to_owned(),
                subject: Subject::Endpoint {
                    node_id: [8; 32],
                    relay: relay.map(str::to_owned),
                },
                occupancy: Occupancy::HasRoom,
                tags: Vec::new(),
                visibility,
                issued_at: NOW - 60,
                expires_at: NOW + 3_600,
                seq,
            },
        )
        .unwrap()
    }

    fn registry(byte: u8, listed: Option<SignedAnnouncement>) -> SignedRegistryList {
        let mut registry =
            Registry::new(SigningKey::from_bytes(&[byte; 32]), "worlds we like").unwrap();
        if let Some(announcement) = listed {
            registry.include(announcement, NOW - 30).unwrap();
        }
        registry.publish(NOW - 10).unwrap()
    }

    /// Pkarr relays that hold nothing.
    struct EmptyRelays;

    impl BridgeHttp for EmptyRelays {
        fn request(
            &mut self,
            _request: HttpRequest,
        ) -> impl Future<Output = Result<HttpResponse>> + Send {
            std::future::ready(Ok(HttpResponse {
                status: 404,
                headers: Vec::new(),
                body: Vec::new(),
            }))
        }
    }

    fn relays() -> RelaySinks {
        RelaySinks::new(["https://pkarr.example"]).unwrap()
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    fn plan(
        config: &ClientDiscoveryConfig,
        fetched: Vec<SignedRegistryList>,
        invite: Option<SignedAnnouncement>,
    ) -> Result<JoinPlan> {
        block_on(plan_join(
            config,
            Some(relays()),
            fetched,
            &mut EmptyRelays,
            &world_id(),
            invite,
            NOW,
        ))
    }

    #[test]
    fn a_fresh_self_host_follows_no_registry_until_its_operator_chooses_one() {
        let listed = announced(4, Visibility::Listed, Some(RELAY));
        let curated = registry(1, Some(listed.clone()));
        let mut config = ClientDiscoveryConfig::fresh_self_host();

        // The list is at hand, but nothing follows it: only the invite
        // link and pkarr are asked, and neither knows this world.
        let curated_layer = Layer::Registry(curated.list.registry_key).to_string();
        let error = format!(
            "{:#}",
            plan(&config, vec![curated.clone()], None).unwrap_err()
        );
        assert!(error.contains("pkarr"), "{error}");
        assert!(!error.contains(&curated_layer), "{error}");

        config.subscribe(curated.list.registry_key);
        let joined = plan(&config, vec![curated.clone()], None).unwrap();
        assert_eq!(
            joined.resolution.layer,
            Layer::Registry(curated.list.registry_key)
        );
        assert_eq!(joined.resolution.announcement, listed);
        assert_eq!(joined.node_id, [8; 32]);
    }

    #[test]
    fn only_subscribed_registries_are_asked_in_subscription_order() {
        let listed = announced(4, Visibility::Listed, None);
        let followed = registry(2, Some(listed.clone()));
        let ignored = registry(3, Some(listed));
        let config = ClientDiscoveryConfig::hosted_with_curated(followed.list.registry_key);
        let resolver = client_resolver(
            &config,
            None,
            vec![ignored, followed.clone(), followed.clone()],
        );
        assert_eq!(resolver.registries(), std::slice::from_ref(&followed));
    }

    #[test]
    fn a_private_world_is_undiscoverable_by_every_layer() {
        // A private world has no announcement: pkarr has nothing, and
        // an honest registry can't list what was never announced.
        let config =
            ClientDiscoveryConfig::hosted_with_curated(registry(1, None).list.registry_key);
        let error = plan(&config, vec![registry(1, None)], None).unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("pkarr"), "{message}");
        assert!(message.contains("registry"), "{message}");

        // Its invite link carries the announcement, exactly, and that
        // is the one way in.
        let invite = announced(1, Visibility::Unlisted, Some(RELAY));
        let joined = plan(&config, vec![registry(1, None)], Some(invite.clone())).unwrap();
        assert_eq!(joined.resolution.layer, Layer::Invite);
        assert_eq!(joined.resolution.announcement, invite);
    }

    #[test]
    fn a_registry_that_lists_an_unlisted_world_is_refused() {
        // A registry refuses an unlisted announcement at inclusion, so
        // forge the entry into a published list.
        let mut rogue = registry(4, Some(announced(2, Visibility::Listed, None)));
        rogue.list.entries[0].announcement = announced(3, Visibility::Unlisted, None);
        let config = ClientDiscoveryConfig::hosted_with_curated(rogue.list.registry_key);
        let error = plan(&config, vec![rogue.clone()], None).unwrap_err();
        assert!(
            format!("{error:#}").contains("a registry listed a world that is not listed"),
            "{error:#}"
        );
        let browse = crate::discovery_resolver::search(std::slice::from_ref(&rogue), "", NOW);
        assert!(browse.is_empty());
    }

    #[test]
    fn relay_required_never_opens_a_direct_connection() {
        let mut config = ClientDiscoveryConfig::fresh_self_host();
        config.relay_required = true;

        let with_relay = announced(1, Visibility::Unlisted, Some(RELAY));
        let joined = plan(&config, Vec::new(), Some(with_relay)).unwrap();
        assert_eq!(
            joined.route,
            JoinRoute::RelayOnly {
                relay: RELAY.to_owned()
            }
        );
        assert!(!joined.route.opens_direct());
        assert_eq!(joined.route.disclosure(), RELAY_REQUIRED_DISCLOSURE);

        // No relay to go through: the join is refused, not sent direct.
        let no_relay = announced(1, Visibility::Unlisted, None);
        let error = plan(&config, Vec::new(), Some(no_relay.clone())).unwrap_err();
        assert!(error.to_string().contains("relay-required"), "{error:#}");

        // The same world with relay-required off may connect directly,
        // and the client is told what that exposes.
        config.relay_required = false;
        let direct = plan(&config, Vec::new(), Some(no_relay)).unwrap();
        assert_eq!(direct.route, JoinRoute::DirectAllowed { relay: None });
        assert!(direct.route.opens_direct());
        assert!(direct.route.disclosure().contains("IP"));
    }

    #[test]
    fn a_program_announcement_is_not_a_world_to_join() {
        let key = world_key();
        let program = SignedAnnouncement::sign(
            &key,
            Announcement {
                subject: Subject::Program {
                    content_hash: [5; 32],
                },
                ..announced(1, Visibility::Unlisted, None).announcement
            },
        )
        .unwrap();
        let error = plan(
            &ClientDiscoveryConfig::fresh_self_host(),
            Vec::new(),
            Some(program),
        )
        .unwrap_err();
        assert!(error.to_string().contains("program"), "{error:#}");
    }

    #[test]
    fn every_layer_outcome_is_kept_on_the_plan() {
        let listed = announced(4, Visibility::Listed, Some(RELAY));
        let followed = registry(1, Some(listed));
        let config = ClientDiscoveryConfig::hosted_with_curated(followed.list.registry_key);
        let joined = plan(
            &config,
            vec![followed],
            Some(announced(3, Visibility::Listed, Some(RELAY))),
        )
        .unwrap();
        assert_eq!(
            joined.resolution.considered[0],
            (Layer::Invite, Outcome::Superseded { seq: 3 })
        );
        assert!(matches!(
            joined.resolution.considered[1].1,
            Outcome::Failed(_) | Outcome::Absent
        ));
        assert_eq!(joined.resolution.considered[2].1, Outcome::Chosen);
    }
}
