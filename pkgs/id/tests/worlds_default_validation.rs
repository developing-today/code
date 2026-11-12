//! Regression tests for issue #6647: `WorldClient::read_worlds_reply`
//! must refuse a `worlds` reply whose top-level `default` is missing or
//! not a string, instead of silently reading it as `Ok("")`. A scripted
//! host answers `list_worlds` with canned frames; the real host always
//! sends a string (ServerFrame::Worlds), so only a scripted peer can
//! produce the malformed shapes.
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
async fn run_case(
    reply: serde_json::Value,
) -> anyhow::Result<(String, Vec<(String, bool, usize)>)> {
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

fn worlds_reply() -> serde_json::Value {
    serde_json::json!({
        "type": "worlds",
        "worlds": [
            {"name": "lobby", "open": true, "presence": 3},
            {"name": "arena", "open": false, "presence": 0}
        ]
    })
}

#[tokio::test]
#[allow(clippy::expect_used)]
async fn worlds_reply_default_must_be_a_string() {
    // Control: a string default is returned unchanged, worlds listed.
    let mut reply = worlds_reply();
    reply["default"] = serde_json::json!("lobby");
    let (default, worlds) = run_case(reply).await.expect("string default must succeed");
    assert_eq!(default, "lobby");
    assert_eq!(worlds.len(), 2);

    // An explicitly empty string is a well-formed shape: it passes.
    // Shape is what the parser judges, not the name's content.
    let mut reply = worlds_reply();
    reply["default"] = serde_json::json!("");
    let (default, _) = run_case(reply)
        .await
        .expect("empty-string default must succeed");
    assert_eq!(default, "");

    // Missing or mistyped defaults are malformed replies: refused with
    // an error naming the field, never collapsed to Ok("").
    for (label, default_val) in [
        ("missing", None),
        ("number", Some(serde_json::json!(123))),
        ("bool", Some(serde_json::json!(true))),
        ("array", Some(serde_json::json!([]))),
        ("object", Some(serde_json::json!({}))),
        ("null", Some(serde_json::json!(null))),
    ] {
        let mut reply = worlds_reply();
        if let Some(v) = default_val {
            reply["default"] = v;
        }
        let err = run_case(reply)
            .await
            .expect_err(&format!("default {label} must be refused"));
        assert!(
            err.to_string().contains("default"),
            "error for {label} default must name the field, got: {err}"
        );
    }
}
