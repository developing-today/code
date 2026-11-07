// Probe: WorldClient::info_details() active_module_hash shape handling.
use id::frames::{read_frame, write_frame};
use id::world_net::{WorldClient, WorldModuleInfo, WORLD_ALPN};
use iroh::{
    endpoint::{presets, RelayMode},
    protocol::{AcceptError, ProtocolHandler, Router},
    Endpoint, EndpointAddr,
};

#[derive(Debug, Clone)]
struct ScriptedHost { reply: serde_json::Value }

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
    Endpoint::builder(presets::Minimal).relay_mode(RelayMode::Disabled).bind().await.unwrap()
}

async fn run_case(reply: serde_json::Value) -> anyhow::Result<WorldModuleInfo> {
    let server_ep = endpoint().await;
    let router = Router::builder(server_ep.clone()).accept(WORLD_ALPN, ScriptedHost { reply }).spawn();
    let addr = EndpointAddr::from_parts(server_ep.id(), server_ep.bound_sockets().iter().map(|a| {
        let ip = if a.ip().is_unspecified() { std::net::Ipv4Addr::LOCALHOST.into() } else { a.ip() };
        iroh::TransportAddr::Ip(std::net::SocketAddr::new(ip, a.port()))
    }));
    let client_ep = endpoint().await;
    let mut client = WorldClient::connect(&client_ep, addr).await.unwrap();
    let result = client.info_details("cap").await;
    client.close(); client_ep.close().await; router.shutdown().await.unwrap();
    result
}

#[tokio::test]
async fn probe_info_reply_shapes() {
    let control = run_case(serde_json::json!({"type":"module_info","world_id":"w1","active_module_hash":"abc123","current_sequence":7})).await;
    println!("CONTROL string hash -> {control:?}");
    assert_eq!(control.unwrap().active_module_hash.as_deref(), Some("abc123"));

    let legit_none = run_case(serde_json::json!({"type":"module_info","world_id":"w1","active_module_hash":null,"current_sequence":7})).await;
    println!("CONTROL null hash -> {legit_none:?}");
    assert_eq!(legit_none.unwrap().active_module_hash, None);

    for (name, v) in [("number", serde_json::json!(123)), ("bool", serde_json::json!(true)), ("array", serde_json::json!(["abc"])), ("object", serde_json::json!({"h":"abc"}))] {
        let r = run_case(serde_json::json!({"type":"module_info","world_id":"w1","active_module_hash":v,"current_sequence":7})).await;
        println!("CASE mistyped {name} -> {r:?}");
    }
    // oracle contrast: mistyped provenance in same reply IS refused
    let prov = run_case(serde_json::json!({"type":"module_info","world_id":"w1","active_module_hash":"abc","current_sequence":7,"active_module_provenance":123})).await;
    println!("ORACLE mistyped provenance -> {prov:?}");
}
