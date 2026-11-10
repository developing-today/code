//! Spike S7 (connect): the six connectivity tests, one subcommand each.
//!
//! `s7 t1` … `s7 t6`. Each prints its measurements and one `RESULT` line, and exits non-zero on
//! failure. Only `t2` and `t4` touch n0's infrastructure. `t4` publishes two throwaway IDs to
//! n0's lookup service, so it is run once. `t7` and `t8` run the Tunnel-style relay from the
//! connectivity design as a child process, on loopback only.

use std::net::SocketAddr;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail, ensure};
use c1_connect::connect::{self, AddressPolicy, Bound, Lookups, Mainline, Profile, RelayEntry};
use c1_connect::harness::{self, Dialed};
use iroh::{Endpoint, EndpointAddr, RelayUrl, SecretKey, TransportAddr};
use iroh_tickets::endpoint::EndpointTicket;
use n0_mainline::Testnet;

const ALPN: &[u8] = b"s7/connect/0";
const LIMIT: Duration = Duration::from_secs(30);
const ROUNDS: usize = 3;
/// The Tunnel relay from `connectivity/relay/tunnel.toml`: plain HTTP on loopback, no UDP.
const TUNNEL_URL: &str = "http://127.0.0.1:3340";
const TUNNEL_CONFIG: &str = "/home/user/work/integrated/doc/2026-10-11T03-44-33Z_design_groups_events_delegation/connectivity/relay/tunnel.toml";
const RELAY_BIN: &str = "/home/user/work/spikes/tools/bin/iroh-relay";

#[tokio::main]
async fn main() {
    // iroh logs to stderr through `tracing`. Without RUST_LOG the level is warn.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_ansi(false)
        .with_writer(std::io::stderr)
        .init();
    let test = std::env::args().nth(1).unwrap_or_default();
    let outcome = match test.as_str() {
        "t1" => t1().await,
        "t2" => t2().await,
        "t3" => t3().await,
        "t4" => t4().await,
        "t5" => t5().await,
        "t6" => t6().await,
        "t7" => t7().await,
        "t8" => t8().await,
        "policy" => policy().await,
        _ => Err(anyhow!("usage: c1 t1|t2|t3|t4|t5|t6|t7|t8|policy")),
    };
    match outcome {
        Ok(()) => println!("RESULT {test}: PASS"),
        Err(err) => {
            println!("RESULT {test}: FAIL: {err:#}");
            std::process::exit(1);
        }
    }
}

fn report_dial(label: &str, d: &Dialed) {
    let handshake = match &d.handshake_relay {
        Some(url) => format!("relay {url}"),
        None => "direct".to_string(),
    };
    println!(
        "{label}: connected {:?}, replied {:?}, echoed {}, handshake over {handshake}",
        d.connected,
        d.replied,
        harness::echoed(d)
    );
    for path in harness::paths(&d.conn) {
        println!("  path after reply: {path}");
    }
}

/// A dialer with the in-memory lookup and the testnet DHT, resolve only.
async fn dialer(net: &Testnet, url: &RelayUrl) -> Result<Bound> {
    connect::bind(&Profile {
        relays: vec![url.clone().into()],
        lookups: Lookups {
            memory: true,
            mainline: Some(Mainline {
                bootstrap: net.bootstrap.clone(),
                publish: false,
            }),
            ..Default::default()
        },
        ..Default::default()
    })
    .await
}

/// A published peer on the testnet, serving the echo protocol.
async fn published_peer(
    net: &Testnet,
    url: &RelayUrl,
) -> Result<(Bound, tokio::task::JoinHandle<()>)> {
    let peer = connect::bind(&Profile {
        relays: vec![url.clone().into()],
        lookups: Lookups {
            mainline: Some(Mainline {
                bootstrap: net.bootstrap.clone(),
                publish: true,
            }),
            ..Default::default()
        },
        alpns: vec![ALPN.to_vec()],
        ..Default::default()
    })
    .await?;
    let serving = harness::serve(peer.endpoint.clone());
    harness::online(&peer.endpoint, LIMIT).await?;
    Ok((peer, serving))
}

fn n0_lookups() -> Lookups {
    Lookups {
        n0_pkarr_publish: true,
        n0_pkarr_resolve: true,
        n0_dns_resolve: true,
        ..Default::default()
    }
}

/// A dial that must fail, and should fail at once because there is nothing to dial.
async fn expect_fail(label: &str, client: &Endpoint, addr: impl Into<EndpointAddr>) -> Result<()> {
    let started = Instant::now();
    match tokio::time::timeout(Duration::from_secs(60), client.connect(addr, ALPN)).await {
        Ok(Err(err)) => {
            println!("{label}: failed after {:?}: {err:#}", started.elapsed());
            Ok(())
        }
        Ok(Ok(_)) => bail!("{label}: connected, expected a failure"),
        Err(_) => bail!("{label}: no failure within 60 s"),
    }
}

/// (1) Our relay only. No lookup, and no direct address in the ticket.
async fn t1() -> Result<()> {
    let (relay, url) = harness::local_relay().await?;
    let profile = Profile {
        relays: vec![url.clone().into()],
        alpns: vec![ALPN.to_vec()],
        ..Default::default()
    };
    println!("server: {}", profile.summary());
    let server = connect::bind(&profile).await?;
    let client = connect::bind(&Profile {
        alpns: vec![],
        ..profile.clone()
    })
    .await?;
    println!("client: {}", profile.summary());
    let serving = harness::serve(server.endpoint.clone());
    println!(
        "online: server {:?}, client {:?}",
        harness::online(&server.endpoint, LIMIT).await?,
        harness::online(&client.endpoint, LIMIT).await?
    );

    let own = server.endpoint.addr();
    println!(
        "server's own address: {} relay, {} direct",
        own.relay_urls().count(),
        own.ip_addrs().count()
    );
    let text = EndpointTicket::new(AddressPolicy::RelayOnly.apply(&own)).to_string();
    let addr = text.parse::<EndpointTicket>()?.endpoint_addr().clone();
    println!(
        "ticket: {} chars, {} relay, {} direct",
        text.len(),
        addr.relay_urls().count(),
        addr.ip_addrs().count()
    );
    ensure!(
        addr.ip_addrs().count() == 0,
        "the ticket carries a direct address"
    );
    ensure!(
        client.endpoint.address_lookup()?.is_empty(),
        "the client has a lookup configured"
    );

    let dialed = harness::dial(&client.endpoint, addr, ALPN).await?;
    report_dial("dial by ticket", &dialed);
    ensure!(harness::echoed(&dialed), "reply was not the echo");
    ensure!(
        dialed.handshake_relay == Some(url),
        "the handshake was not over our relay"
    );

    dialed.conn.close(0u32.into(), b"done");
    client.endpoint.close().await;
    server.endpoint.close().await;
    serving.abort();
    relay.shutdown().await?;
    Ok(())
}

/// (2) A ticket with two relays, one local and one n0. The local relay goes down, and the dial
/// must fall back to the n0 relay.
async fn t2() -> Result<()> {
    let (local, local_url) = harness::local_relay().await?;
    let n0_url = connect::n0_relays()
        .into_iter()
        .next()
        .context("no n0 relay")?;
    let urls = [local_url.clone(), n0_url.clone()];
    let relays: Vec<RelayEntry> = urls.iter().cloned().map(RelayEntry::from).collect();
    println!("relays: local {local_url}, n0 {n0_url}");

    let server = connect::bind(&Profile {
        relays: relays.clone(),
        alpns: vec![ALPN.to_vec()],
        ..Default::default()
    })
    .await?;
    let serving = harness::serve(server.endpoint.clone());
    println!(
        "server online after {:?}",
        harness::online(&server.endpoint, LIMIT * 2).await?
    );
    let ticket = EndpointTicket::new(EndpointAddr::from_parts(
        server.endpoint.id(),
        urls.iter().cloned().map(TransportAddr::Relay),
    ))
    .to_string();
    let addr = ticket.parse::<EndpointTicket>()?.endpoint_addr().clone();
    println!("ticket: {} chars, 2 relay URLs, 0 direct", ticket.len());

    println!(
        "server home relays at start: {:?}",
        harness::home_relays(&server.endpoint)
    );

    // Phase A: both relays up. The handshake should use the local one.
    let a_client = connect::bind(&Profile {
        relays: relays.clone(),
        ..Default::default()
    })
    .await?;
    harness::online(&a_client.endpoint, LIMIT * 2).await?;
    let a = harness::dial(&a_client.endpoint, addr.clone(), ALPN).await?;
    report_dial("A (both relays up)", &a);
    ensure!(
        a.handshake_relay.as_ref() == Some(&local_url),
        "A did not start on the local relay"
    );
    a.conn.close(0u32.into(), b"done");

    // A warm client, online on the local relay before the failure. Phase C uses it.
    let warm = connect::bind(&Profile {
        relays: relays.clone(),
        ..Default::default()
    })
    .await?;
    harness::online(&warm.endpoint, LIMIT * 2).await?;
    println!(
        "warm client home relays before: {:?}",
        harness::home_relays(&warm.endpoint)
    );

    // Phase B: the local relay goes down. The clock starts before the shutdown.
    let t0 = Instant::now();
    local.shutdown().await?;
    let rehomed = loop {
        let now: Vec<RelayUrl> = server.endpoint.addr().relay_urls().cloned().collect();
        if now.contains(&n0_url) && !now.contains(&local_url) {
            break t0.elapsed();
        }
        ensure!(
            t0.elapsed() < LIMIT * 4,
            "the server did not re-home onto the n0 relay"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    println!("server re-homed onto n0 {rehomed:?} after the shutdown");

    // B: a fresh client, started after the failure.
    let b_client = connect::bind(&Profile {
        relays: relays.clone(),
        ..Default::default()
    })
    .await?;
    let b_online = harness::online(&b_client.endpoint, LIMIT * 4).await?;
    println!(
        "B: client online {:?} after the shutdown (its own wait: {b_online:?})",
        t0.elapsed()
    );
    let b_dial_at = t0.elapsed();
    let b = harness::dial(&b_client.endpoint, addr.clone(), ALPN).await?;
    report_dial("B (fresh client, local relay down)", &b);
    println!(
        "B: handshake complete {:?} after the shutdown",
        b_dial_at + b.connected
    );
    ensure!(harness::echoed(&b), "reply was not the echo");
    ensure!(
        b.handshake_relay.as_ref() == Some(&n0_url),
        "B did not fall back to the n0 relay"
    );

    // C: the warm client, whose home relay is still the local one until its own re-home.
    let c_dial_at = t0.elapsed();
    println!(
        "C: home relays at dial time: {:?}",
        harness::home_relays(&warm.endpoint)
    );
    let c = harness::dial(&warm.endpoint, addr, ALPN).await?;
    report_dial("C (warm client, local relay down)", &c);
    println!(
        "C: handshake complete {:?} after the shutdown, over {:?}",
        c_dial_at + c.connected,
        c.handshake_relay.as_ref().map(|u| u.to_string())
    );

    c.conn.close(0u32.into(), b"done");
    b.conn.close(0u32.into(), b"done");
    warm.endpoint.close().await;
    a_client.endpoint.close().await;
    b_client.endpoint.close().await;
    server.endpoint.close().await;
    serving.abort();
    Ok(())
}

/// (3) Mainline DHT on a local testnet. The publisher stores relay URLs only, and a second
/// endpoint resolves it by ID with no internet.
async fn t3() -> Result<()> {
    let net = harness::testnet(8).await?;
    let inspector = harness::testnet_dht(&net)?;
    let (relay, url) = harness::local_relay().await?;

    let profile = Profile {
        relays: vec![url.clone().into()],
        lookups: Lookups {
            mainline: Some(Mainline {
                bootstrap: net.bootstrap.clone(),
                publish: true,
            }),
            ..Default::default()
        },
        policy: AddressPolicy::RelayOnly,
        alpns: vec![ALPN.to_vec()],
    };
    println!("publisher: {}", profile.summary());
    let publisher = connect::bind(&profile).await?;
    let serving = harness::serve(publisher.endpoint.clone());
    println!(
        "publisher online after {:?}",
        harness::online(&publisher.endpoint, LIMIT).await?
    );
    let own = publisher.endpoint.addr();
    println!(
        "publisher's own address: {} relay, {} direct (the record must hold none of the direct ones)",
        own.relay_urls().count(),
        own.ip_addrs().count()
    );

    let (packet, visible) =
        harness::wait_for_record(&inspector, publisher.endpoint.id(), Duration::from_secs(60))
            .await?;
    println!("record readable on the testnet {visible:?} after online");
    let audit = harness::audit(&packet)?;
    for text in &audit.text {
        println!("  record text: {text}");
    }
    println!(
        "  parsed: {} relay URL(s), {} direct address(es); IP literals in record bytes: {}",
        audit.relay_urls.len(),
        audit.parsed_ip_addrs,
        audit.ip_literals.len()
    );

    let resolver = connect::bind(&Profile {
        relays: vec![url.clone().into()],
        lookups: Lookups {
            mainline: Some(Mainline {
                bootstrap: net.bootstrap.clone(),
                publish: false,
            }),
            ..Default::default()
        },
        ..Default::default()
    })
    .await?;
    println!(
        "resolver online after {:?}",
        harness::online(&resolver.endpoint, LIMIT).await?
    );
    let by_id = harness::dial_by_id(
        &resolver.endpoint,
        publisher.endpoint.id(),
        ALPN,
        Duration::from_secs(30),
    )
    .await?;
    println!(
        "resolve and dial by ID: {} attempt(s), {:?} from the first attempt",
        by_id.attempts, by_id.since_first
    );
    for failure in &by_id.failures {
        println!("  failed attempt: {failure}");
    }
    report_dial("resolver dial by ID", &by_id.dialed);

    ensure!(
        audit.parsed_ip_addrs == 0,
        "the record holds a direct address"
    );
    ensure!(
        audit.ip_literals.is_empty(),
        "an IP literal appears in the record"
    );
    ensure!(
        audit.relay_urls == vec![url.to_string()],
        "the record's relay URLs are not exactly ours"
    );
    ensure!(harness::echoed(&by_id.dialed), "reply was not the echo");
    ensure!(
        by_id.dialed.handshake_relay == Some(url),
        "the handshake was not over our relay"
    );

    by_id.dialed.conn.close(0u32.into(), b"done");
    resolver.endpoint.close().await;
    publisher.endpoint.close().await;
    serving.abort();
    relay.shutdown().await?;
    Ok(())
}

/// (4) n0's defaults on the real internet: relays plus pkarr and DNS. One endpoint dials the
/// other by node ID, with no ticket. Run once: this publishes two throwaway IDs to n0.
async fn t4() -> Result<()> {
    let profile = Profile {
        relays: connect::n0_relays()
            .into_iter()
            .map(RelayEntry::from)
            .collect(),
        lookups: n0_lookups(),
        policy: AddressPolicy::RelayOnly,
        alpns: vec![ALPN.to_vec()],
    };
    println!("profile: {}", profile.summary());
    println!(
        "IROH_FORCE_STAGING_RELAYS set: {}",
        std::env::var_os("IROH_FORCE_STAGING_RELAYS").is_some()
    );
    let server = connect::bind(&profile).await?;
    let serving = harness::serve(server.endpoint.clone());
    let client = connect::bind(&Profile {
        alpns: vec![],
        ..profile.clone()
    })
    .await?;
    println!(
        "server online after {:?}",
        harness::online(&server.endpoint, LIMIT * 2).await?
    );
    println!(
        "client online after {:?}",
        harness::online(&client.endpoint, LIMIT * 2).await?
    );
    let id = server.endpoint.id();

    // Wait until n0's lookup returns the server's record to the client. This polls; it does not
    // publish again.
    let started = Instant::now();
    let items = loop {
        let items = harness::lookup_items(&client.endpoint, id, Duration::from_secs(5)).await?;
        if items
            .iter()
            .any(|i| !i.starts_with("error") && !i.starts_with("no result"))
        {
            break items;
        }
        ensure!(
            started.elapsed() < Duration::from_secs(180),
            "the server's record was not visible on n0 after 180 s"
        );
        tokio::time::sleep(Duration::from_secs(2)).await;
    };
    println!(
        "record visible through n0 lookups after {:?}: {items:?}",
        started.elapsed()
    );

    let dialed = tokio::time::timeout(
        Duration::from_secs(90),
        harness::dial(&client.endpoint, id, ALPN),
    )
    .await
    .context("dial by ID timed out after 90 s")??;
    report_dial("dial by ID (no ticket)", &dialed);
    tokio::time::sleep(Duration::from_secs(5)).await;
    for path in harness::paths(&dialed.conn) {
        println!("  after 5 s: path {path}");
    }
    ensure!(harness::echoed(&dialed), "reply was not the echo");

    dialed.conn.close(0u32.into(), b"done");
    client.endpoint.close().await;
    server.endpoint.close().await;
    serving.abort();
    Ok(())
}

/// (5) Multi-lookup precedence. The dialer has an in-memory lookup and the testnet DHT. Each
/// case gives the peer's address to one lookup or to both, and the dial must succeed.
async fn t5() -> Result<()> {
    let net = harness::testnet(8).await?;
    let inspector = harness::testnet_dht(&net)?;
    let (relay, url) = harness::local_relay().await?;
    // A second live relay that the publisher is not registered on.
    let (wrong_relay, wrong_url) = harness::local_relay().await?;

    let (publisher, serving_p) = published_peer(&net, &url).await?;
    let p_id = publisher.endpoint.id();
    let (_, visible) = harness::wait_for_record(&inspector, p_id, Duration::from_secs(60)).await?;
    println!("publisher's record on the testnet {visible:?} after online");

    // Q is not in the DHT. Its relay-only address is known only to the memory lookup.
    let peer = connect::bind(&Profile {
        relays: vec![url.clone().into()],
        alpns: vec![ALPN.to_vec()],
        ..Default::default()
    })
    .await?;
    let serving_q = harness::serve(peer.endpoint.clone());
    harness::online(&peer.endpoint, LIMIT).await?;
    let q_addr = AddressPolicy::RelayOnly.apply(&peer.endpoint.addr());

    let dead = closed_loopback_port()?;
    let on_wrong_relay = EndpointAddr::from_parts(p_id, [TransportAddr::Relay(wrong_url.clone())]);
    let at_dead_ip = EndpointAddr::from_parts(p_id, [TransportAddr::Ip(dead)]);

    let mut rows = Vec::new();
    for round in 1..=ROUNDS {
        // A: P is known only to the DHT.
        let a = dialer(&net, &url).await?;
        let r = harness::dial_by_id(&a.endpoint, p_id, ALPN, Duration::from_secs(30)).await?;
        if round == 1 {
            report_dial("A dht only", &r.dialed);
        }
        rows.push(Row::new("A dht only", round, &r, &url));
        a.endpoint.close().await;

        // B: Q is known only to the memory lookup.
        let b = dialer(&net, &url).await?;
        b.memory
            .as_ref()
            .context("no memory lookup")?
            .add_endpoint_info(q_addr.clone());
        let r = harness::dial_by_id(
            &b.endpoint,
            peer.endpoint.id(),
            ALPN,
            Duration::from_secs(30),
        )
        .await?;
        rows.push(Row::new("B memory only (Q)", round, &r, &url));
        b.endpoint.close().await;

        // C: memory has P on a live relay that P is not on. The DHT has P on the right relay.
        let c = dialer(&net, &url).await?;
        c.memory
            .as_ref()
            .context("no memory lookup")?
            .add_endpoint_info(on_wrong_relay.clone());
        if round == 1 {
            for item in harness::lookup_items(&c.endpoint, p_id, Duration::from_secs(3)).await? {
                println!("  C before dial, item: {item}");
            }
        }
        let r = harness::dial_by_id(&c.endpoint, p_id, ALPN, Duration::from_secs(30)).await?;
        if round == 1 {
            report_dial("C memory: wrong relay, dht: right relay", &r.dialed);
        }
        rows.push(Row::new(
            "C memory: wrong relay, dht: right relay",
            round,
            &r,
            &url,
        ));
        c.endpoint.close().await;

        // D: memory has P at a dead direct address. The DHT has P on the right relay.
        let d = dialer(&net, &url).await?;
        d.memory
            .as_ref()
            .context("no memory lookup")?
            .add_endpoint_info(at_dead_ip.clone());
        if round == 1 {
            for item in harness::lookup_items(&d.endpoint, p_id, Duration::from_secs(3)).await? {
                println!("  D before dial, item: {item}");
            }
        }
        let r = harness::dial_by_id(&d.endpoint, p_id, ALPN, Duration::from_secs(30)).await?;
        if round == 1 {
            report_dial("D memory: dead direct, dht: right relay", &r.dialed);
        }
        rows.push(Row::new(
            "D memory: dead direct, dht: right relay",
            round,
            &r,
            &url,
        ));
        d.endpoint.close().await;
    }

    // E, a control, not a pass criterion: Q's only address is a relay Q is not on, and the DHT
    // has no record for Q. Nothing works, so this shows the cost of a failing address alone.
    let wrong_only = EndpointAddr::from_parts(
        peer.endpoint.id(),
        [TransportAddr::Relay(wrong_url.clone())],
    );
    let e = dialer(&net, &url).await?;
    e.memory
        .as_ref()
        .context("no memory lookup")?
        .add_endpoint_info(wrong_only);
    let started = Instant::now();
    match tokio::time::timeout(
        Duration::from_secs(60),
        e.endpoint.connect(peer.endpoint.id(), ALPN),
    )
    .await
    {
        Ok(Err(err)) => println!(
            "E control (only a wrong relay known): failed after {:?}: {err:#}",
            started.elapsed()
        ),
        Ok(Ok(_)) => println!("E control: connected, which was not expected"),
        Err(_) => println!("E control: no result within 60 s"),
    }
    e.endpoint.close().await;

    println!("case, round, reply time, attempts, selected path");
    for row in &rows {
        println!(
            "{:<42} {}  {:>8.1} ms  {} attempt(s)  {}",
            row.case, row.round, row.reply_ms, row.attempts, row.path
        );
    }
    ensure!(
        rows.iter().all(|r| r.ok),
        "a case failed, or a dial did not select our relay"
    );

    peer.endpoint.close().await;
    publisher.endpoint.close().await;
    serving_q.abort();
    serving_p.abort();
    relay.shutdown().await?;
    wrong_relay.shutdown().await?;
    Ok(())
}

struct Row {
    case: &'static str,
    round: usize,
    reply_ms: f64,
    attempts: u32,
    path: String,
    ok: bool,
}

impl Row {
    fn new(case: &'static str, round: usize, r: &harness::ByIdDial, expected: &RelayUrl) -> Self {
        let selected = r.dialed.handshake_relay.clone();
        Self {
            case,
            round,
            reply_ms: r.dialed.replied.as_secs_f64() * 1000.0,
            attempts: r.attempts,
            path: match &selected {
                Some(url) => format!("relay {url}"),
                None => "direct (address withheld)".to_string(),
            },
            ok: harness::echoed(&r.dialed) && selected.as_ref() == Some(expected),
        }
    }
}

/// Policy switch, not one of the six tests. `RelayAndIp` must publish direct addresses, which
/// `RelayOnly` withholds in t3. The record is written to the local testnet only, and only counts
/// are printed.
async fn policy() -> Result<()> {
    let net = harness::testnet(8).await?;
    let inspector = harness::testnet_dht(&net)?;
    let (relay, url) = harness::local_relay().await?;
    let profile = Profile {
        relays: vec![url.clone().into()],
        lookups: Lookups {
            mainline: Some(Mainline {
                bootstrap: net.bootstrap.clone(),
                publish: true,
            }),
            ..Default::default()
        },
        policy: AddressPolicy::RelayAndIp,
        alpns: vec![ALPN.to_vec()],
    };
    println!("publisher: {}", profile.summary());
    let publisher = connect::bind(&profile).await?;
    let serving = harness::serve(publisher.endpoint.clone());
    harness::online(&publisher.endpoint, LIMIT).await?;
    println!(
        "publisher's own address: {} relay, {} direct",
        publisher.endpoint.addr().relay_urls().count(),
        publisher.endpoint.addr().ip_addrs().count()
    );

    // An earlier record can hold only the direct addresses, before the relay was known. Wait for
    // one that has both.
    let started = Instant::now();
    let audit = loop {
        if let Some(packet) = harness::fetch_record(&inspector, publisher.endpoint.id()).await? {
            let audit = harness::audit(&packet)?;
            if audit.parsed_ip_addrs > 0 && !audit.relay_urls.is_empty() {
                break audit;
            }
        }
        ensure!(
            started.elapsed() < Duration::from_secs(60),
            "RelayAndIp never published a record with both a relay and a direct address"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    println!(
        "record: {} relay URL(s), {} direct address(es) (values withheld)",
        audit.relay_urls.len(),
        audit.parsed_ip_addrs
    );
    ensure!(
        audit.relay_urls == vec![url.to_string()],
        "relay URL missing from the record"
    );

    publisher.endpoint.close().await;
    serving.abort();
    relay.shutdown().await?;
    Ok(())
}

/// A loopback UDP port with nothing listening on it, for a dead direct address.
fn closed_loopback_port() -> Result<SocketAddr> {
    let socket = std::net::UdpSocket::bind("127.0.0.1:0")?;
    let addr = socket.local_addr()?;
    drop(socket);
    Ok(addr)
}

/// (6) Negative tests: an unknown ID on the DHT, a ticket with no addresses, and a bare ID with
/// no lookup and no ticket. A published peer is included as a positive control.
async fn t6() -> Result<()> {
    let net = harness::testnet(8).await?;
    let (relay, url) = harness::local_relay().await?;
    let (publisher, serving) = published_peer(&net, &url).await?;
    let p_id = publisher.endpoint.id();
    let inspector = harness::testnet_dht(&net)?;
    harness::wait_for_record(&inspector, p_id, Duration::from_secs(60)).await?;

    // Relay on, no lookup.
    let bare = connect::bind(&Profile {
        relays: vec![url.clone().into()],
        ..Default::default()
    })
    .await?;
    harness::online(&bare.endpoint, LIMIT).await?;
    expect_fail(
        "(b) ticket with no addresses, no lookup",
        &bare.endpoint,
        EndpointAddr::new(p_id),
    )
    .await?;
    expect_fail("(c) bare ID, no lookup, no ticket", &bare.endpoint, p_id).await?;

    // Relay on, DHT on. The same address-less ticket now resolves through the DHT.
    let dht = dialer(&net, &url).await?;
    harness::online(&dht.endpoint, LIMIT).await?;
    let positive = harness::dial_by_id(&dht.endpoint, p_id, ALPN, Duration::from_secs(30)).await?;
    println!(
        "(b, DHT on) address-less ticket: resolved after {} attempt(s), {:?} from the first",
        positive.attempts, positive.since_first
    );
    ensure!(
        harness::echoed(&positive.dialed),
        "positive control did not echo"
    );

    for round in 1..=ROUNDS {
        let unknown = SecretKey::generate().public();
        let started = Instant::now();
        match tokio::time::timeout(
            Duration::from_secs(120),
            dht.endpoint.connect(unknown, ALPN),
        )
        .await
        {
            Ok(Err(err)) => println!(
                "(a) unknown ID on the testnet DHT, round {round}: failed after {:?}: {err:#}",
                started.elapsed()
            ),
            Ok(Ok(_)) => bail!("an unknown ID connected"),
            Err(_) => bail!("an unknown ID produced no failure within 120 s"),
        }
    }

    positive.dialed.conn.close(0u32.into(), b"done");
    dht.endpoint.close().await;
    bare.endpoint.close().await;
    publisher.endpoint.close().await;
    serving.abort();
    relay.shutdown().await?;
    Ok(())
}

/// (7) The Tunnel relay, with QAD off. The relay runs as a child process from the connectivity
/// design's `tunnel.toml`. Server and client use only that relay, relay-only policy, and no
/// direct address in the ticket.
async fn t7() -> Result<()> {
    tunnel_dial(false, "t7").await
}

/// (8) The same relay with QAD left on. That is the misconfiguration: QAD needs a UDP port the
/// Tunnel does not carry. The dial should still succeed. The log shows what QAD does instead.
async fn t8() -> Result<()> {
    tunnel_dial(true, "t8").await
}

async fn tunnel_dial(quic: bool, name: &str) -> Result<()> {
    let log = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("logs/{name}-relay.txt"));
    let relay = harness::external_relay(
        Path::new(RELAY_BIN),
        Path::new(TUNNEL_CONFIG),
        &log,
        TUNNEL_URL,
    )
    .await?;
    println!("relay: {TUNNEL_URL} answers /ping, log {}", log.display());

    let url: RelayUrl = TUNNEL_URL.parse()?;
    let profile = Profile {
        relays: vec![RelayEntry {
            url: url.clone(),
            quic,
        }],
        alpns: vec![ALPN.to_vec()],
        policy: AddressPolicy::RelayOnly,
        ..Default::default()
    };
    println!("profile: {}", profile.summary());
    let server = connect::bind(&profile).await?;
    println!("server bound, endpoint started");
    let client = connect::bind(&Profile {
        alpns: vec![],
        ..profile.clone()
    })
    .await?;
    println!("client bound, endpoint started");
    let serving = harness::serve(server.endpoint.clone());
    // Both waits run together. Awaited one after the other, the client's wait would start only
    // after the server's had finished, and would read near zero.
    let (s, c) = tokio::join!(
        harness::online(&server.endpoint, LIMIT),
        harness::online(&client.endpoint, LIMIT)
    );
    println!("online: server {:?}, client {:?}", s?, c?);

    let own = server.endpoint.addr();
    let text = EndpointTicket::new(AddressPolicy::RelayOnly.apply(&own)).to_string();
    let addr = text.parse::<EndpointTicket>()?.endpoint_addr().clone();
    println!(
        "ticket: {} chars, {} relay, {} direct",
        text.len(),
        addr.relay_urls().count(),
        addr.ip_addrs().count()
    );
    ensure!(
        addr.ip_addrs().count() == 0,
        "the ticket carries a direct address"
    );
    ensure!(
        client.endpoint.address_lookup()?.is_empty(),
        "the client has a lookup configured"
    );

    println!("dial start");
    let dialed = harness::dial(&client.endpoint, addr, ALPN).await?;
    report_dial("dial by ticket", &dialed);
    ensure!(harness::echoed(&dialed), "reply was not the echo");
    ensure!(
        dialed.handshake_relay == Some(url),
        "the handshake was not over the tunnel relay"
    );

    dialed.conn.close(0u32.into(), b"done");
    client.endpoint.close().await;
    server.endpoint.close().await;
    serving.abort();
    drop(relay); // kills the relay child
    Ok(())
}
