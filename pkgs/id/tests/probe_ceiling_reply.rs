// Probe: WorldClient::set_caps_ceiling() reply-shape handling.
// Scripted host replies with a canned frame; the client result is printed
// per case. Expectation (typed contract ServerFrame::CapsCeiling { report:
// CapsReport, message } in world_session/types.rs, and the #6624 sibling
// ruling for caps()/update_caps()): a "caps_ceiling" reply whose report is
// missing or not a CapsReport must Err, not return Ok(reply) verbatim.
use id::frames::{read_frame, write_frame};
use id::world_net::{WorldClient, WORLD_ALPN};
use iroh::{
    endpoint::{presets, RelayMode},
    protocol::{AcceptError, ProtocolHandler, Router},
    Endpoint, EndpointAddr,
};

#[derive(Debug, Clone)]
struct ScriptedHost {
    reply: serde_json::Value,
}

impl ProtocolHandler for ScriptedHost {
    async fn accept(&self, conn: iroh::endpoint::Connection) -> Result<(), AcceptError> {
        let reply = self.reply.clone();
        tokio::spawn(async move {
            while let Ok((mut send, mut recv)) = conn.accept_bi().await {
                let reply = reply.clone();
                tokio::spawn(async move {
                    let _ = read_frame(&mut recv).await;
                    let body = serde_json::to_vec(&reply).unwrap();
                    let _ = write_frame(&mut send, &body).await;
                    let _ = send.finish();
                });
            }
        });
        Ok(())
    }
}

async fn endpoint() -> Endpoint {
    Endpoint::builder(presets::Minimal)
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await
        .unwrap()
}

async fn run_case(reply: serde_json::Value) -> anyhow::Result<serde_json::Value> {
    let server_ep = endpoint().await;
    let router = Router::builder(server_ep.clone())
        .accept(WORLD_ALPN, ScriptedHost { reply })
        .spawn();
    let addr = EndpointAddr::from_parts(
        server_ep.id(),
        server_ep.bound_sockets().iter().map(|a| {
            let ip = if a.ip().is_unspecified() {
                std::net::Ipv4Addr::LOCALHOST.into()
            } else {
                a.ip()
            };
            iroh::TransportAddr::Ip(std::net::SocketAddr::new(ip, a.port()))
        }),
    );
    let client_ep = endpoint().await;
    let mut client = WorldClient::connect(&client_ep, addr).await.unwrap();
    let result = client
        .set_caps_ceiling("admin", &["time.now".to_owned()])
        .await;
    client.close();
    client_ep.close().await;
    router.shutdown().await.unwrap();
    result
}

fn valid_report() -> serde_json::Value {
    serde_json::json!({
        "granted": ["time.now"], "requested": [], "missing": [],
        "usage": {}, "policy": "deny", "catalog": ["time.now"],
        "ceiling": ["time.now"], "effective": ["time.now"], "view_failures": 0
    })
}

#[tokio::test]
async fn probe_ceiling_reply_shapes() {
    let control = run_case(serde_json::json!({
        "type": "caps_ceiling", "report": valid_report(),
        "message": "the ceiling applies immediately"
    }))
    .await;
    println!("CONTROL full reply -> {control:?}");
    assert!(
        control.is_ok(),
        "control: a well-formed caps_ceiling reply returns Ok"
    );

    let missing_report = run_case(serde_json::json!({
        "type": "caps_ceiling", "message": "the ceiling applies immediately"
    }))
    .await;
    println!("CASE missing report -> {missing_report:?}");

    let null_report = run_case(serde_json::json!({
        "type": "caps_ceiling", "report": null, "message": "ok"
    }))
    .await;
    println!("CASE report null -> {null_report:?}");

    let mistyped_report = run_case(serde_json::json!({
        "type": "caps_ceiling", "report": 123, "message": "ok"
    }))
    .await;
    println!("CASE report 123 -> {mistyped_report:?}");

    let bad_report_shape = run_case(serde_json::json!({
        "type": "caps_ceiling", "report": {"granted": "time.now"}, "message": "ok"
    }))
    .await;
    println!("CASE report wrong shape -> {bad_report_shape:?}");

    let missing_message = run_case(serde_json::json!({
        "type": "caps_ceiling", "report": valid_report()
    }))
    .await;
    println!("CASE missing message -> {missing_message:?}");
}
