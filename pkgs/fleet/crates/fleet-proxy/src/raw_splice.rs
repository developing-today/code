//! Loopback RustDesk listener backed by one raw broker tunnel per
//! accepted client connection.

use std::net::{TcpListener, TcpStream};

use fleet_config::{Config, PORT_RUSTDESK};
use fleet_proxy_proto::{read_frame, write_frame};

use crate::client::{ClientError, Negotiated, ensure_broker};
use crate::raw_stream::relay;

pub const LISTEN_ADDR: &str = "127.0.0.1:21118";
const CAPABILITY: &str = "proxy-raw-splice";

fn supports_raw_splice(negotiated: &Negotiated) -> bool {
    match negotiated {
        Negotiated::Fleet(response) => {
            response.capabilities.iter().any(|cap| cap == CAPABILITY)
                && response
                    .services
                    .get(&PORT_RUSTDESK.to_string())
                    .is_some_and(|service| service.name == "rustdesk")
        }
        Negotiated::Legacy(_) => false,
    }
}

/// Start the loopback listener and serve each accepted RustDesk
/// connection through a fresh broker session.
pub fn run(config: &Config, client: &str, capabilities: &[String]) -> Result<i32, ClientError> {
    let listener = TcpListener::bind(LISTEN_ADDR)
        .map_err(|e| ClientError::Transport(format!("cannot bind {LISTEN_ADDR}: {e}")))?;
    println!("fleet proxy splice listening on {LISTEN_ADDR}");
    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => {
                let config = config.clone();
                let client = client.to_string();
                let capabilities = capabilities.to_vec();
                std::thread::spawn(move || {
                    if let Err(e) = splice_one(stream, &config, &client, &capabilities) {
                        eprintln!("fleet proxy splice: {e}");
                    }
                });
            }
            Err(e) => eprintln!("fleet proxy splice accept failed: {e}"),
        }
    }
    Ok(0)
}

fn splice_one(
    local: TcpStream,
    config: &Config,
    client: &str,
    capabilities: &[String],
) -> Result<(), ClientError> {
    let mut ensured = ensure_broker(config, client, capabilities)?;
    if !supports_raw_splice(&ensured.negotiated) {
        return Err(ClientError::Transport(
            "broker does not advertise RustDesk raw-splice support".to_string(),
        ));
    }

    write_frame(
        &mut ensured.stream,
        &serde_json::json!({"raw_splice": {"port": PORT_RUSTDESK}}),
    )
    .map_err(|e| ClientError::Transport(format!("raw-splice request: {e}")))?;
    let ready = read_frame(&mut ensured.stream)
        .map_err(|e| ClientError::Transport(format!("raw-splice response: {e}")))?;
    if ready.get("raw_splice").and_then(|v| v.get("ready")) != Some(&serde_json::Value::Bool(true))
    {
        let why = ready
            .get("raw_splice")
            .and_then(|v| v.get("error"))
            .and_then(|v| v.as_str())
            .unwrap_or("broker rejected the raw-splice request");
        return Err(ClientError::Transport(why.to_string()));
    }

    relay(local, ensured.stream)
        .map_err(|e| ClientError::Transport(format!("raw-splice relay: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fleet_proxy_proto::{HelloResponse, Response, ServiceInfo};
    use std::collections::BTreeMap;

    fn fleet_response(capabilities: &[&str], rustdesk: bool) -> Negotiated {
        let mut services = BTreeMap::new();
        if rustdesk {
            services.insert(
                PORT_RUSTDESK.to_string(),
                ServiceInfo {
                    name: "rustdesk".to_string(),
                    version: "rustdesk-direct-v1".to_string(),
                },
            );
        }
        Negotiated::Fleet(Box::new(HelloResponse {
            proto: fleet_proxy_proto::PROTO_MAJOR,
            services,
            capabilities: capabilities.iter().map(|cap| cap.to_string()).collect(),
        }))
    }

    #[test]
    fn listener_is_loopback_only_and_uses_rustdesk_port() {
        assert!(LISTEN_ADDR.starts_with("127.0.0.1:"));
        assert!(LISTEN_ADDR.ends_with(&PORT_RUSTDESK.to_string()));
    }

    #[test]
    fn legacy_and_older_fleet_brokers_refuse_the_splice_cleanly() {
        assert!(!supports_raw_splice(&Negotiated::Legacy(Box::new(
            Response {
                status: 502,
                body: serde_json::json!({"error":"legacy"}),
            }
        ))));
        assert!(!supports_raw_splice(&fleet_response(
            &["proxy-serve"],
            true
        )));
        assert!(!supports_raw_splice(&fleet_response(&[CAPABILITY], false)));
        assert!(supports_raw_splice(&fleet_response(&[CAPABILITY], true)));
    }
}
