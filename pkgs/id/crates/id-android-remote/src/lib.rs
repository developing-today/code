//! Android remote-drive transport bridge for issue #1831.
//!
//! The bridge speaks the existing `/id-world/1` shell session protocol and
//! stays a client: it never starts a world host or listener.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use id_worlds::{shell_contract::ShellContract, world_net::WorldClient};
use iroh::{Endpoint, EndpointAddr};
use serde::Serialize;
use serde_json::{Value, json};

/// Maximum time the initial join waits for a complete host view.
pub const INITIAL_VIEW_TIMEOUT: Duration = Duration::from_secs(30);

/// The newest host view received by the Android remote-drive client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RemoteView {
    /// Durable world sequence reflected by this view.
    pub sequence: u64,
    /// Plain-text screen projection supplied by the host.
    pub data: String,
    /// Optional versioned screen or presentation surface from the host.
    pub surface: Option<Value>,
}

/// Parse and validate one server frame, returning a view when it carries one.
///
/// The shell contract is the oracle for frame kind and shape. Non-view
/// session frames are ignored; malformed frames are errors.
pub fn parse_view_frame(frame: &Value) -> Result<Option<RemoteView>> {
    let contract = ShellContract::embedded().map_err(anyhow::Error::msg)?;
    let kind = contract
        .check_server_frame(frame)
        .map_err(anyhow::Error::msg)?;
    match kind {
        "view" => {
            let sequence = frame
                .get("sequence")
                .and_then(Value::as_u64)
                .context("host sent a view without an unsigned sequence")?;
            let data = frame
                .get("data")
                .and_then(Value::as_str)
                .context("host sent a view without a text projection")?
                .to_owned();
            Ok(Some(RemoteView {
                sequence,
                data,
                surface: frame.get("surface").cloned(),
            }))
        }
        "error" => bail!(
            "world host refused the session: {}",
            frame["message"].as_str().unwrap_or("unspecified error")
        ),
        _ => Ok(None),
    }
}

/// Join an existing Iroh world session and return its first validated view.
///
/// The caller owns the endpoint/runtime and keeps the user-provided
/// capability in memory only. `addr` can be resolved by the endpoint's
/// configured Iroh address lookup or carry direct transport addresses.
///
/// # Errors
///
/// Returns a protocol, transport, refusal, malformed-view, or timeout error.
pub async fn connect_and_read_view(
    endpoint: &Endpoint,
    addr: EndpointAddr,
    world: Option<&str>,
    capability: &str,
) -> Result<RemoteView> {
    if capability.trim().is_empty() {
        bail!("join capability is empty");
    }
    let mut client = WorldClient::connect(endpoint, addr)
        .await
        .context("connect to world host")?;
    client = client.with_world(world.filter(|name| !name.trim().is_empty()));
    client
        .send_json(&json!({
            "type": "join",
            "capability": capability,
            "after": null,
        }))
        .await
        .context("send world-session join")?;

    tokio::time::timeout(INITIAL_VIEW_TIMEOUT, async {
        loop {
            let frame = client
                .recv_json()
                .await
                .context("receive world-session frame")?
                .context("world host closed before sending a view")?;
            if let Some(view) = parse_view_frame(&frame)? {
                return Ok(view);
            }
        }
    })
    .await
    .context("timed out waiting for the initial host view")?
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

    use serde_json::json;

    use super::{RemoteView, parse_view_frame};

    #[test]
    fn shell_contract_view_frame_yields_the_host_text_projection() {
        let frame = json!({
            "type": "view",
            "sequence": 7,
            "view_revision": 1,
            "data": "tic-tac-toe: X to move",
        });
        let contract = id_worlds::shell_contract::ShellContract::embedded()
            .expect("embedded shell contract is valid");
        assert_eq!(
            contract
                .check_server_frame(&frame)
                .expect("view frame conforms to the shell contract"),
            "view"
        );

        assert_eq!(
            parse_view_frame(&frame).expect("valid host view parses"),
            Some(RemoteView {
                sequence: 7,
                data: "tic-tac-toe: X to move".to_owned(),
                surface: None,
            })
        );
    }
}
