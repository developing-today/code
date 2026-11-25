//! Android remote-drive transport bridge for issue #1831.
//!
//! The bridge speaks the existing `/id-world/1` shell session protocol and
//! stays a client: it never starts a world host or listener.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use id_worlds::{shell_contract::ShellContract, world_net::WorldClient};
use iroh::endpoint::presets;
use iroh::{Endpoint, EndpointAddr, EndpointId};
use serde::Serialize;
use serde_json::{Value, json};

/// Maximum time the initial join waits for a complete host view.
pub const INITIAL_VIEW_TIMEOUT: Duration = Duration::from_secs(30);

/// Bind a client endpoint with Iroh's standard relay and address lookup.
///
/// The endpoint receives a fresh ephemeral transport identity for this
/// proof-of-concept session; the user's join capability remains the only
/// world authorization.
pub async fn bind_public_client_endpoint() -> Result<Endpoint> {
    Endpoint::builder(presets::N0)
        .bind()
        .await
        .context("bind public Iroh client endpoint")
}

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

/// A connected `/id-world/1` client stream that can read subsequent views.
#[derive(Debug)]
pub struct RemoteWorldSession {
    client: WorldClient,
}

impl RemoteWorldSession {
    /// Wait for and return the next contract-validated host view.
    ///
    /// Non-view session frames are consumed while waiting. Connection closure
    /// and host refusals are explicit errors.
    pub async fn next_view(&mut self) -> Result<RemoteView> {
        loop {
            let frame = self
                .client
                .recv_json()
                .await
                .context("receive world-session frame")?
                .context("world host closed before sending a view")?;
            if let Some(view) = parse_view_frame(&frame)? {
                return Ok(view);
            }
        }
    }
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
    let (_session, view) = join_world_session(endpoint, addr, world, capability).await?;
    Ok(view)
}

/// Join an existing Iroh world session and return its first view plus the
/// still-open stream for later views.
///
/// The caller owns the endpoint/runtime and keeps the user-provided
/// capability in memory only. `addr` can be resolved by the endpoint's
/// configured Iroh address lookup or carry direct transport addresses.
///
/// # Errors
///
/// Returns connection, protocol, refusal, malformed-frame, or timeout errors.
pub async fn join_world_session(
    endpoint: &Endpoint,
    addr: EndpointAddr,
    world: Option<&str>,
    capability: &str,
) -> Result<(RemoteWorldSession, RemoteView)> {
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

    let mut session = RemoteWorldSession { client };
    let first_view = tokio::time::timeout(INITIAL_VIEW_TIMEOUT, session.next_view())
        .await
        .context("timed out waiting for the initial host view")??;
    Ok((session, first_view))
}

/// Parse an Iroh endpoint ID or serialized `EndpointAddr` supplied by the UI.
///
/// # Errors
///
/// Returns an error for empty or malformed input.
pub fn parse_endpoint_addr(text: &str) -> Result<EndpointAddr> {
    let value = text.trim();
    if value.is_empty() {
        bail!("world address is empty");
    }
    if let Ok(addr) = serde_json::from_str::<EndpointAddr>(value) {
        return Ok(addr);
    }
    let endpoint_id: EndpointId = value
        .parse()
        .context("world address must be an endpoint ID or serialized EndpointAddr")?;
    Ok(EndpointAddr::from(endpoint_id))
}

#[cfg(target_os = "android")]
mod android_jni;

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
