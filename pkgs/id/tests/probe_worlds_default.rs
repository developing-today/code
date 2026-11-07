// Probe: WorldClient::list_worlds reply-entry handling.
// A "worlds" reply whose entries are malformed: are they refused, or
// silently dropped / defaulted while the reply as a whole succeeds?
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

async fn run_case(reply: serde_json::Value) -> anyhow::Result<(String, Vec<(String, bool, usize)>)> {
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
    let result = client.list_worlds("admin").await;
    client.close();
    client_ep.close().await;
    router.shutdown().await.unwrap();
    result
}

#[tokio::test]
async fn probe_worlds_reply_shapes() {
    let control = run_case(serde_json::json!({
        "type": "worlds", "default": "lobby",
        "worlds": [
            {"name": "lobby", "open": true, "presence": 3},
            {"name": "arena", "open": false, "presence": 0}
        ]
    }))
    .await;
    println!("CONTROL two valid worlds -> {control:?}");
    assert_eq!(control.unwrap().1.len(), 2, "control: both valid worlds listed");

    for (label, default_val) in [
        ("missing", None),
        ("number", Some(serde_json::json!(123))),
        ("bool", Some(serde_json::json!(true))),
        ("array", Some(serde_json::json!([]))),
        ("object", Some(serde_json::json!({}))),
        ("null", Some(serde_json::json!(null))),
    ] {
        let mut reply = serde_json::json!({
            "type": "worlds",
            "worlds": [
                {"name": "lobby", "open": true, "presence": 3},
                {"name": "arena", "open": false, "presence": 0}
            ]
        });
        if let Some(v) = default_val { reply["default"] = v; }
        let r = run_case(reply).await;
        println!("CASE default-{label} -> {r:?}");
    }
    let no_list = run_case(serde_json::json!({
        "type": "worlds", "default": "lobby", "worlds": {"name": "lobby"}
    })).await;
    println!("ORACLE worlds-not-a-list -> {no_list:?}");
    assert!(no_list.is_err());
}
