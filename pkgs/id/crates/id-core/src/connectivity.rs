//! Endpoint policy for the wider network: which relay servers an endpoint
//! uses, how it finds peers, and how long a dial may take.
//!
//! Every endpoint that talks to the wider network is built with
//! [`public_builder`] and dials with [`connect`], so the relay and lookup
//! policy lives in this one module. Loopback endpoints (to a running
//! `id serve`) and test endpoints do not use it.

use std::{future::Future, sync::Arc, time::Duration};

use iroh::{
    EndpointAddr, RelayConfig, RelayMap,
    endpoint::{Builder, Connection, Endpoint, RelayMode, presets},
};

/// Which relay servers an endpoint may use.
///
/// `N0` is the default: n0's public relays, which also carry traffic when two
/// peers cannot reach each other directly. `Disabled` uses direct addresses
/// only, as the `--no-relay` flag does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RelayPolicy {
    /// n0's public relay servers.
    #[default]
    N0,
    /// No relay servers: direct addresses only.
    Disabled,
}

impl RelayPolicy {
    /// Maps the `--no-relay` flag to a policy.
    pub const fn from_no_relay(no_relay: bool) -> Self {
        if no_relay { Self::Disabled } else { Self::N0 }
    }
}

/// Starts an endpoint builder with n0's address lookup, then applies `relays`.
///
/// The N0 preset publishes to and resolves from n0's pkarr and DNS services.
/// `N0` keeps the preset's own relay mode. `Disabled` turns relays off.
pub fn public_builder(relays: RelayPolicy) -> Builder {
    let builder = Endpoint::builder(presets::N0);
    match relays {
        RelayPolicy::N0 => builder,
        RelayPolicy::Disabled => builder.relay_mode(RelayMode::Disabled),
    }
}

/// How long one dial may take, covering the address lookup and the handshake.
///
/// The pkarr lookup over HTTP has no timeout of its own, and a single dead
/// address was measured holding a dial for about 30 s, so [`connect`] runs
/// every dial under this deadline.
pub const DIAL_TIMEOUT: Duration = Duration::from_secs(45);

/// Runs `fut` for at most `limit`. On expiry the error says what was in
/// progress. An error from `fut` itself passes through unchanged.
pub async fn within<T>(
    limit: Duration,
    what: &str,
    fut: impl Future<Output = anyhow::Result<T>>,
) -> anyhow::Result<T> {
    match tokio::time::timeout(limit, fut).await {
        Ok(result) => result,
        Err(_elapsed) => anyhow::bail!("timed out after {limit:?} while {what}"),
    }
}

/// Dials `target` for `alpn`, failing after [`DIAL_TIMEOUT`].
///
/// `target` is a node ID or a full [`EndpointAddr`]. A bare node ID is
/// resolved through the endpoint's address lookup, which is the step that
/// can hang.
pub async fn connect(
    endpoint: &Endpoint,
    target: impl Into<EndpointAddr>,
    alpn: &[u8],
) -> anyhow::Result<Connection> {
    within(DIAL_TIMEOUT, "connecting to a peer", async move {
        endpoint
            .connect(target, alpn)
            .await
            .map_err(anyhow::Error::from)
    })
    .await
}

/// How long startup waits for the home relay before carrying on without one.
pub const HOME_RELAY_DEADLINE: Duration = Duration::from_secs(10);

/// Waits until the endpoint has a connected home relay, for at most `limit`.
///
/// Only the home relay gates this wait. A map entry that is down must not hold
/// startup, so no other entry is waited for. Call it only when relays are on:
/// with [`RelayMode::Disabled`] no relay connects, and the wait ends at `limit`.
pub async fn await_home_relay(endpoint: &Endpoint, limit: Duration) -> anyhow::Result<()> {
    within(limit, "waiting for the home relay", async {
        endpoint.online().await;
        Ok(())
    })
    .await
}

/// How many relays iroh's address probe reaches, in map order. This is
/// `MAX_RELAYS` in iroh's net report.
pub const QAD_PROBE_LIMIT: usize = 5;

/// Refuses a relay map with no QAD-capable relay in its first
/// [`QAD_PROBE_LIMIT`] entries.
///
/// Only those entries are probed for the public address that a direct path
/// needs, so a QAD relay past them gives none. The map cannot be reordered to fix this. iroh keeps its entries in a map
/// keyed by URL, so their order is the URL order, not the order of insertion.
/// The map is refused, and its URLs must change.
pub fn check_qad_order(map: &RelayMap) -> anyhow::Result<()> {
    let relays: Vec<Arc<RelayConfig>> = map.relays();
    if relays.is_empty() {
        return Ok(());
    }
    let probed = &relays[..relays.len().min(QAD_PROBE_LIMIT)];
    anyhow::ensure!(
        probed.iter().any(|relay| relay.quic.is_some()),
        "no QAD-capable relay among the first {} of {} relays, which iroh probes: {}",
        QAD_PROBE_LIMIT,
        relays.len(),
        probed
            .iter()
            .map(|relay| relay.url.to_string())
            .collect::<Vec<_>>()
            .join(", "),
    );
    Ok(())
}

/// Dials that cannot finish, for tests of the deadlines in the modules that dial.
#[cfg(test)]
#[allow(clippy::unwrap_used)]
pub(crate) mod silent {
    use std::net::{Ipv4Addr, SocketAddr};

    use futures_lite::stream;
    use iroh::{
        EndpointAddr, EndpointId, TransportAddr,
        address_lookup::{AddressLookup, Error, Item},
        endpoint::{Endpoint, RelayMode, presets},
    };
    use iroh_base::SecretKey;

    /// An address nothing answers on. TEST-NET-1 (192.0.2.0/24) is never routed,
    /// so a dial to it can only wait for its handshake to time out.
    pub(crate) fn unanswered_addr() -> EndpointAddr {
        let id = SecretKey::from_bytes(&[9; 32]).public();
        let socket = SocketAddr::new(Ipv4Addr::new(192, 0, 2, 1).into(), 9);
        EndpointAddr::from_parts(id, [TransportAddr::Ip(socket)])
    }

    /// An address lookup whose answer never comes, so a dial by node ID alone waits.
    #[derive(Debug)]
    pub(crate) struct NeverAnswers;

    impl AddressLookup for NeverAnswers {
        fn resolve(&self, _endpoint_id: EndpointId) -> Option<stream::Boxed<Result<Item, Error>>> {
            Some(Box::pin(stream::pending()))
        }
    }

    /// A loopback-only endpoint whose address lookup is [`NeverAnswers`].
    pub(crate) async fn endpoint_with_silent_lookup() -> Endpoint {
        Endpoint::builder(presets::Minimal)
            .relay_mode(RelayMode::Disabled)
            .address_lookup(NeverAnswers)
            .bind()
            .await
            .unwrap()
    }

    /// The node ID of no real endpoint, to dial by ID alone.
    pub(crate) fn unanswered_id() -> EndpointAddr {
        EndpointAddr::from(SecretKey::from_bytes(&[9; 32]).public())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::time::Instant;

    use anyhow::Context as _;
    use iroh::{RelayConfig, RelayMap, RelayUrl, TransportAddr, Watcher as _};

    use super::*;

    const TEST_ALPN: &[u8] = b"/id-connectivity-test/1";

    #[test]
    fn test_from_no_relay_maps_the_flag() {
        assert_eq!(RelayPolicy::from_no_relay(false), RelayPolicy::N0);
        assert_eq!(RelayPolicy::from_no_relay(true), RelayPolicy::Disabled);
    }

    #[test]
    fn test_default_relay_policy_is_n0() {
        assert_eq!(RelayPolicy::default(), RelayPolicy::N0);
    }

    #[tokio::test]
    async fn test_within_returns_the_value() {
        let value = within(Duration::from_secs(5), "testing", async { anyhow::Ok(7) })
            .await
            .unwrap();
        assert_eq!(value, 7);
    }

    #[tokio::test]
    async fn test_within_passes_the_inner_error_through() {
        let err = within::<()>(Duration::from_secs(5), "testing", async {
            anyhow::bail!("inner failure")
        })
        .await
        .unwrap_err();
        assert_eq!(err.to_string(), "inner failure");
    }

    #[tokio::test]
    async fn test_within_times_out_and_names_the_step() {
        let err = within::<()>(
            Duration::from_millis(10),
            "waiting for a peer",
            std::future::pending(),
        )
        .await
        .unwrap_err();
        let message = err.to_string();
        assert!(message.contains("timed out"), "{message}");
        assert!(message.contains("waiting for a peer"), "{message}");
    }

    /// Address lookup is cleared so this test stays offline. The N0 preset
    /// would otherwise publish the endpoint to n0's pkarr service on bind.
    #[tokio::test]
    async fn test_disabled_endpoint_has_no_relay_address() {
        let endpoint = public_builder(RelayPolicy::Disabled)
            .clear_address_lookup()
            .bind()
            .await
            .unwrap();
        let addrs = endpoint.addr().addrs;
        endpoint.close().await;
        assert!(
            !addrs
                .iter()
                .any(|addr| matches!(addr, TransportAddr::Relay(_))),
            "{addrs:?}"
        );
    }

    /// Adds an entry on an address nothing answers on. TEST-NET-1 is never routed,
    /// so the entry stays dead and its handshake waits for a timeout.
    fn with_dead_entry(map: &RelayMap) {
        let url: RelayUrl = "https://192.0.2.1:443".parse().unwrap();
        map.insert(url.clone(), Arc::new(RelayConfig::from(url)));
    }

    /// A map whose only entry is dead cannot give a home relay. The wait must end
    /// at its deadline, and must not wait without one.
    #[tokio::test]
    async fn test_startup_wait_ends_at_the_deadline_with_only_a_dead_relay() {
        let map = RelayMap::empty();
        with_dead_entry(&map);
        let endpoint = Endpoint::builder(presets::Minimal)
            .relay_mode(RelayMode::Custom(map))
            .bind()
            .await
            .unwrap();
        let limit = Duration::from_secs(2);
        let started = Instant::now();
        let result =
            tokio::time::timeout(Duration::from_secs(30), await_home_relay(&endpoint, limit))
                .await
                .expect("the startup wait had no deadline");
        let elapsed = started.elapsed();
        endpoint.close().await;
        let err = result.expect_err("a dead relay gave a home relay");
        assert!(err.to_string().contains("timed out"), "{err:#}");
        assert!(elapsed < limit + Duration::from_secs(2), "{elapsed:?}");
    }

    /// The home relay alone gates startup. A dead entry beside a live relay must
    /// not hold the wait, and the connected home relay is the live one.
    #[tokio::test]
    async fn test_startup_wait_is_gated_by_the_home_relay_alone() {
        let (map, live_url, _server) = iroh::test_utils::run_relay_server().await.unwrap();
        with_dead_entry(&map);
        let endpoint = Endpoint::builder(presets::Minimal)
            .relay_mode(RelayMode::Custom(map))
            .ca_tls_config(iroh::tls::CaTlsConfig::insecure_skip_verify())
            .bind()
            .await
            .unwrap();
        let started = Instant::now();
        let result = tokio::time::timeout(
            Duration::from_secs(60),
            await_home_relay(&endpoint, HOME_RELAY_DEADLINE),
        )
        .await
        .expect("the startup wait had no deadline");
        let elapsed = started.elapsed();
        let homes = endpoint.home_relay_status().get();
        endpoint.close().await;
        result.unwrap();
        assert!(elapsed < HOME_RELAY_DEADLINE, "{elapsed:?}");
        assert!(
            homes
                .iter()
                .any(|home| home.is_connected() && home.url() == &live_url),
            "{homes:?}"
        );
    }

    /// A relay entry on a fixed host, with QUIC address discovery on or off.
    fn relay_entry(host: &str, qad: bool) -> Arc<RelayConfig> {
        let url: RelayUrl = format!("https://{host}").parse().unwrap();
        let mut config = RelayConfig::from(url);
        if !qad {
            config.quic = None;
        }
        Arc::new(config)
    }

    /// A relay map from `(host, qad)` pairs. iroh orders it by URL.
    fn relay_map(entries: &[(&str, bool)]) -> RelayMap {
        entries
            .iter()
            .map(|(host, qad)| relay_entry(host, *qad))
            .collect()
    }

    #[test]
    fn test_qad_check_accepts_the_n0_default_map() {
        check_qad_order(&RelayMode::Default.relay_map()).unwrap();
    }

    #[test]
    fn test_qad_check_accepts_an_empty_map() {
        check_qad_order(&RelayMap::empty()).unwrap();
    }

    /// The only QAD relay is sixth in URL order, so iroh's probe never reaches it.
    /// Map order cannot be changed, so the map must be refused.
    #[test]
    fn test_qad_check_refuses_a_qad_relay_that_is_sixth() {
        let map = relay_map(&[
            ("relay-a.test", false),
            ("relay-b.test", false),
            ("relay-c.test", false),
            ("relay-d.test", false),
            ("relay-e.test", false),
            ("relay-f.test", true),
        ]);
        let err = check_qad_order(&map).expect_err("a sixth-place QAD relay was accepted");
        assert!(err.to_string().contains("QAD"), "{err:#}");
    }

    #[test]
    fn test_qad_check_accepts_a_qad_relay_that_is_fifth() {
        let map = relay_map(&[
            ("relay-a.test", false),
            ("relay-b.test", false),
            ("relay-c.test", false),
            ("relay-d.test", false),
            ("relay-e.test", true),
            ("relay-f.test", false),
        ]);
        check_qad_order(&map).unwrap();
    }

    #[test]
    fn test_qad_check_refuses_a_map_with_no_qad_relay() {
        let map = relay_map(&[("relay-a.test", false), ("relay-b.test", false)]);
        assert!(check_qad_order(&map).is_err());
    }

    /// The second endpoint dials the first by node ID alone, over n0's
    /// relays and lookup. Needs internet access, so it is ignored by default.
    /// Run it by hand with `cargo test --lib --features world connectivity -- --ignored`.
    #[ignore = "needs internet, run by hand"]
    #[tokio::test]
    async fn test_n0_endpoints_dial_by_id() {
        within(
            Duration::from_secs(240),
            "running the N0 dial test",
            async {
                let server = public_builder(RelayPolicy::N0)
                    .alpns(vec![TEST_ALPN.to_vec()])
                    .bind()
                    .await?;
                let client = public_builder(RelayPolicy::N0).bind().await?;
                // Wait for a home relay on both sides. The server publishes its
                // address once the relay is known, so an early lookup can miss it.
                server.online().await;
                client.online().await;
                let server_id = server.id();

                let accepting = tokio::spawn({
                    let server = server.clone();
                    async move {
                        let incoming = server.accept().await.context("server closed")?;
                        anyhow::Ok(incoming.await?)
                    }
                });
                let mut attempt = 1;
                let conn = loop {
                    match connect(&client, server_id, TEST_ALPN).await {
                        Ok(conn) => break conn,
                        Err(err) if attempt < 3 => {
                            eprintln!("dial attempt {attempt} failed, retrying: {err:#}");
                            attempt += 1;
                            tokio::time::sleep(Duration::from_secs(2)).await;
                        }
                        Err(err) => return Err(err),
                    }
                };
                anyhow::ensure!(conn.remote_id() == server_id, "dialed a different endpoint");
                let accepted = accepting.await??;
                anyhow::ensure!(
                    accepted.remote_id() == client.id(),
                    "accepted from a different endpoint"
                );

                conn.close(0u32.into(), b"done");
                client.close().await;
                server.close().await;
                anyhow::Ok(())
            },
        )
        .await
        .unwrap();
    }
}
