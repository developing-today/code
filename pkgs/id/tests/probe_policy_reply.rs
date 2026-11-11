// Probe: WorldClient::policy_request + manifest_state reply-shape handling.
// Scripted host replies with a canned frame; the client result is printed
// per case. Expectation (siblings set_isolation / schedules in the same
// file): a "policy" reply missing or mistyping `policy` must Err, not
// return Ok(null)/Ok(verbatim); a "world_state" reply mistyping `seed`
// must Err, not collapse to the legitimate capability-only None.
use id::frames::{read_frame, write_frame};
use id::world_net::{WORLD_ALPN, WorldClient};
use iroh::{
    Endpoint, EndpointAddr,
    endpoint::{RelayMode, presets},
    protocol::{AcceptError, ProtocolHandler, Router},
};

#[derive(Debug, Clone)]
struct ScriptedHost {
    reply: serde_json::Value,
}

impl ProtocolHandler for ScriptedHost {
    #[allow(clippy::unwrap_used)]
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

#[allow(clippy::unwrap_used)]
async fn endpoint() -> Endpoint {
    Endpoint::builder(presets::Minimal)
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await
        .unwrap()
}

#[allow(clippy::unwrap_used)]
async fn client_for(reply: serde_json::Value) -> (WorldClient, Endpoint, iroh::protocol::Router) {
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
    let client = WorldClient::connect(&client_ep, addr).await.unwrap();
    (client, client_ep, router)
}

#[tokio::test]
#[allow(clippy::unwrap_used)]
async fn probe_policy_reply_shapes() {
    // Control: well-formed policy reply returns the policy object.
    let (mut c, ep, r) = client_for(serde_json::json!({
        "type": "policy", "world": "arena",
        "policy": {"isolation": "unisolated", "cross_world_write": true,
                   "cross_world_forced": false, "file_scope": "confined",
                   "file_write_override": false}
    }))
    .await;
    let control = c.set_cross_world_write("admin", true).await;
    println!("CONTROL policy -> {control:?}");
    assert_eq!(
        control.unwrap()["cross_world_write"],
        serde_json::json!(true),
        "control: well-formed policy reply returns its policy"
    );
    c.close();
    ep.close().await;
    r.shutdown().await.unwrap();

    let (mut c, ep, r) = client_for(serde_json::json!({"type": "policy", "world": "arena"})).await;
    let missing = c.set_cross_world_write("admin", true).await;
    println!("CASE missing policy -> {missing:?}");
    c.close();
    ep.close().await;
    r.shutdown().await.unwrap();

    let (mut c, ep, r) = client_for(serde_json::json!({
        "type": "policy", "world": "arena", "policy": 123
    }))
    .await;
    let mistyped_num = c.set_file_scope("admin", "unrestricted", None).await;
    println!("CASE mistyped policy (123, via set_file_scope) -> {mistyped_num:?}");
    c.close();
    ep.close().await;
    r.shutdown().await.unwrap();

    let (mut c, ep, r) = client_for(serde_json::json!({
        "type": "policy", "world": "arena", "policy": "deny"
    }))
    .await;
    let mistyped_str = c.force_cross_world_write("admin").await;
    println!("CASE mistyped policy (\"deny\", via force) -> {mistyped_str:?}");
    c.close();
    ep.close().await;
    r.shutdown().await.unwrap();
}

#[tokio::test]
#[allow(clippy::unwrap_used)]
async fn probe_manifest_state_seed_shapes() {
    let (mut c, ep, r) = client_for(serde_json::json!({
        "type": "world_state", "seed": 42, "schedules": {"a": 1}
    }))
    .await;
    let control = c.manifest_state(Some("admin"), None).await;
    println!("CONTROL seed=42 -> {control:?}");
    assert_eq!(
        control.unwrap().0,
        Some(42),
        "control: numeric seed reads back"
    );
    c.close();
    ep.close().await;
    r.shutdown().await.unwrap();

    // Legitimate capability-only shape: no seed field at all.
    let (mut c, ep, r) = client_for(serde_json::json!({
        "type": "world_state", "schedules": {}
    }))
    .await;
    let legit_none = c.manifest_state(None, Some("cap")).await;
    println!("CONTROL no seed field -> {legit_none:?}");
    c.close();
    ep.close().await;
    r.shutdown().await.unwrap();

    for (name, seed) in [
        ("string \"42\"", serde_json::json!("42")),
        ("negative -1", serde_json::json!(-1)),
        ("float 1.5", serde_json::json!(1.5)),
        ("object {}", serde_json::json!({})),
    ] {
        let (mut c, ep, r) = client_for(serde_json::json!({
            "type": "world_state", "seed": seed, "schedules": {}
        }))
        .await;
        let case = c.manifest_state(Some("admin"), None).await;
        println!("CASE mistyped seed {name} -> {case:?}");
        c.close();
        ep.close().await;
        r.shutdown().await.unwrap();
    }

    // Oracle: the sibling `schedules` field in the same reply IS validated.
    let (mut c, ep, r) = client_for(serde_json::json!({
        "type": "world_state", "seed": 42, "schedules": {"a": "x"}
    }))
    .await;
    let oracle = c.manifest_state(Some("admin"), None).await;
    println!("ORACLE mistyped schedules -> {oracle:?}");
    assert!(oracle.is_err(), "oracle: mistyped schedules must Err");
    c.close();
    ep.close().await;
    r.shutdown().await.unwrap();
}
