// Probe: WorldClient::compile() reply-shape handling.
// Scripted host replies with a canned frame; the client result is printed
// per case. Expectation (siblings install_wasm/info_details/invite):
// a "compiled" reply missing or mistyping module_hash/sequence must Err,
// not silently default to ("", 0).
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
    async fn accept(
        &self,
        conn: iroh::endpoint::Connection,
    ) -> Result<(), AcceptError> {
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

async fn run_case(reply: serde_json::Value) -> anyhow::Result<(String, u64, String)> {
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
        .compile(
            "admin",
            vec![("main.roc".to_owned(), "x = 1".to_owned())],
            7,
            false,
        )
        .await;
    client.close();
    client_ep.close().await;
    router.shutdown().await.unwrap();
    result
}

#[tokio::test]
async fn probe_compile_reply_shapes() {
    let control = run_case(serde_json::json!({
        "type": "compiled", "module_hash": "abc123", "sequence": 7, "diagnostics": "warn"
    }))
    .await;
    println!("CONTROL full reply -> {control:?}");
    assert_eq!(
        control.unwrap(),
        ("abc123".to_owned(), 7, "warn".to_owned()),
        "control: a well-formed compiled reply returns its fields"
    );

    let missing_hash = run_case(serde_json::json!({
        "type": "compiled", "sequence": 7, "diagnostics": ""
    }))
    .await;
    println!("CASE missing module_hash -> {missing_hash:?}");

    let mistyped = run_case(serde_json::json!({
        "type": "compiled", "module_hash": 123, "sequence": "7", "diagnostics": 42
    }))
    .await;
    println!("CASE mistyped fields -> {mistyped:?}");

    let bare = run_case(serde_json::json!({"type": "compiled"})).await;
    println!("CASE bare compiled -> {bare:?}");
}
