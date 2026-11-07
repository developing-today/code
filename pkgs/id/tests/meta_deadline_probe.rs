//! Probe (2026-10-31, lane A): does id::meta_client::request_over return on
//! its own when the peer accepts the stream but never sends a response?
//! The server reads the request and then stays silent. The client call is
//! observed for 5 seconds; if request_over had any internal deadline, it
//! would return Err before the observation window ends.
use std::net::Ipv4Addr;
use std::time::Duration;

use id::meta_client::request_over;
use id::protocol::MetaRequest;
use iroh::{Endpoint, EndpointAddr, RelayMode, TransportAddr, endpoint::presets};

const ALPN: &[u8] = b"id-meta-deadline-probe";

async fn endpoint(alpns: Vec<Vec<u8>>) -> Endpoint {
    Endpoint::builder(presets::Minimal)
        .alpns(alpns)
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await
        .expect("bind")
}

fn loopback_addr(ep: &Endpoint) -> EndpointAddr {
    EndpointAddr::from_parts(
        ep.id(),
        ep.bound_sockets().iter().map(|addr| {
            let ip = if addr.ip().is_unspecified() {
                Ipv4Addr::LOCALHOST.into()
            } else {
                addr.ip()
            };
            TransportAddr::Ip(std::net::SocketAddr::new(ip, addr.port()))
        }),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn request_over_to_a_silent_peer_never_returns() {
    let server = endpoint(vec![ALPN.to_vec()]).await;
    let server_addr = loopback_addr(&server);
    let client = endpoint(vec![]).await;

    // Silent server: accept, read the request, then never answer.
    let silent = tokio::spawn(async move {
        let incoming = server.accept().await.expect("incoming");
        let conn = incoming.await.expect("connecting");
        let (_send, mut recv) = conn.accept_bi().await.expect("accept_bi");
        let _req = recv.read_to_end(4096).await.expect("read request");
        tokio::time::sleep(Duration::from_secs(60)).await;
    });

    let conn = client.connect(server_addr, ALPN).await.expect("connect");
    let req = MetaRequest::Find {
        query: "anything".to_owned(),
        prefer_name: false,
    };
    let observed = tokio::time::timeout(Duration::from_secs(5), request_over(&conn, &req)).await;
    assert!(
        observed.is_err(),
        "request_over returned within 5s against a silent peer: {observed:?} \
         (expected: still pending — it has no deadline of its own)"
    );
    println!("PROBE RESULT: request_over still pending after 5s vs silent peer (no internal deadline)");
    silent.abort();
}
