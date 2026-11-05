//! Contract-frame state for one native shell session.
//!
//! This stays independent of Bevy so the network task can validate and
//! accumulate frames before handing state to the render thread. The native
//! client does not opt in to screen patches: every received view is therefore
//! a complete view, not a delta that needs a revision cache.

use anyhow::{Context, Result, bail};
use id::world_presentation::{WorldSurface, parse_surface};
use serde_json::{Value, json};

/// Create the initial join frame. `None` requests a fresh snapshot; this
/// deliberately omits `screen_patches` until the shell tracks patch revisions.
#[must_use]
pub fn join_frame(capability: &str, after: Option<u64>) -> Value {
    json!({
        "type": "join",
        "capability": capability,
        "after": after,
    })
}

/// A host view in the existing F13.1 contract.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionView {
    /// Durable world sequence the view reflects.
    pub sequence: u64,
    /// Safe legacy text fallback supplied by the host.
    pub data: String,
    /// A recognized, validated presentation surface, if supplied.
    pub surface: Option<WorldSurface>,
}

impl SessionView {
    fn from_frame(frame: &Value) -> Result<Self> {
        let sequence = frame["sequence"]
            .as_u64()
            .context("view frame has no unsigned sequence")?;
        let data = frame["data"]
            .as_str()
            .context("view frame has no text data")?
            .to_owned();
        let surface = frame
            .get("surface")
            .and_then(|surface| serde_json::to_string(surface).ok())
            .and_then(|surface| parse_surface(&surface).ok().flatten());
        Ok(Self {
            sequence,
            data,
            surface,
        })
    }

    /// Plain-text projection for headless output and safe renderer fallback.
    #[must_use]
    pub fn text_projection(&self) -> String {
        self.surface
            .as_ref()
            .map_or_else(|| self.data.clone(), WorldSurface::text_projection)
    }
}

/// The kind of update applied from a validated host frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServerUpdate {
    /// A complete snapshot arrived, including after a server-side resync.
    Snapshot,
    /// A durable event arrived.
    Event,
    /// An ephemeral full view arrived.
    View,
}

/// Accumulated host state for a single session stream.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SessionState {
    snapshot: Option<Value>,
    events: Vec<Value>,
    view: Option<SessionView>,
}

impl SessionState {
    /// Apply one host frame after checking it against the embedded F13.1
    /// contract. Unknown reply kinds are not interpreted as session state.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed, out-of-contract, or non-session frames.
    pub fn apply_server_frame(&mut self, frame: Value) -> Result<ServerUpdate> {
        let contract = id::shell_contract::ShellContract::embedded()
            .map_err(anyhow::Error::msg)
            .context("load shell contract")?;
        let kind = contract
            .check_server_frame(&frame)
            .map_err(anyhow::Error::msg)
            .context("validate host frame")?;

        match kind {
            "snapshot" => {
                self.snapshot = Some(frame["snapshot"].clone());
                self.events.clear();
                Ok(ServerUpdate::Snapshot)
            }
            "event" => {
                self.events.push(frame["event"].clone());
                Ok(ServerUpdate::Event)
            }
            "view" => {
                self.view = Some(SessionView::from_frame(&frame)?);
                Ok(ServerUpdate::View)
            }
            "error" => {
                bail!(
                    "world host refused the session: {}",
                    frame["message"].as_str().unwrap_or("unspecified error")
                );
            }
            _ => bail!("unexpected host frame `{kind}` in world session"),
        }
    }

    /// Most recent complete snapshot received on this stream.
    #[must_use]
    pub fn snapshot(&self) -> Option<&Value> {
        self.snapshot.as_ref()
    }

    /// Durable events received since the most recent snapshot.
    #[must_use]
    pub fn events(&self) -> &[Value] {
        &self.events
    }

    /// Most recent ephemeral view. A view is not a durable event and is not
    /// assumed to follow every event.
    #[must_use]
    pub fn view(&self) -> Option<&SessionView> {
        self.view.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_starts_from_a_fresh_snapshot_without_requesting_patches() {
        assert_eq!(
            join_frame("capability", None),
            json!({"type":"join", "capability":"capability", "after":null})
        );
        assert_eq!(
            join_frame("capability", Some(7)),
            json!({"type":"join", "capability":"capability", "after":7})
        );
    }

    #[test]
    fn session_tracks_snapshot_events_and_ephemeral_views_separately() {
        let mut state = SessionState::default();
        assert_eq!(
            state
                .apply_server_frame(json!({"type":"snapshot", "snapshot":{"sequence":4}}))
                .unwrap(),
            ServerUpdate::Snapshot
        );
        assert_eq!(
            state
                .apply_server_frame(
                    json!({"type":"event", "event":{"sequence":5,"kind":{"kind":"input"}}})
                )
                .unwrap(),
            ServerUpdate::Event
        );
        assert_eq!(
            state
                .apply_server_frame(
                    json!({"type":"view", "sequence":5, "data":"fallback", "view_revision":0})
                )
                .unwrap(),
            ServerUpdate::View
        );

        assert_eq!(state.snapshot(), Some(&json!({"sequence":4})));
        assert_eq!(
            state.events(),
            &[json!({"sequence":5,"kind":{"kind":"input"}})]
        );
        let view = state.view().unwrap();
        assert_eq!(view.sequence, 5);
        assert_eq!(view.text_projection(), "fallback");
    }

    #[test]
    fn a_new_snapshot_resets_events_but_keeps_the_latest_view_until_replaced() {
        let mut state = SessionState::default();
        state
            .apply_server_frame(json!({"type":"snapshot", "snapshot":{"sequence":1}}))
            .unwrap();
        state
            .apply_server_frame(json!({"type":"event", "event":{"sequence":2}}))
            .unwrap();
        state
            .apply_server_frame(
                json!({"type":"view", "sequence":2, "data":"old", "view_revision":0}),
            )
            .unwrap();
        state
            .apply_server_frame(json!({"type":"snapshot", "snapshot":{"sequence":9}}))
            .unwrap();

        assert!(state.events().is_empty());
        assert_eq!(state.view().unwrap().text_projection(), "old");
    }

    #[test]
    fn only_a_validated_surface_replaces_the_legacy_data_fallback() {
        let mut state = SessionState::default();
        state
            .apply_server_frame(json!({
                "type":"view", "sequence":1, "data":"fallback",
                "surface":{"kind":"screen","version":1,"width":2,"height":1,"rows":["hi"]},
                "view_revision":0
            }))
            .unwrap();
        assert_eq!(state.view().unwrap().text_projection(), "hi");

        state
            .apply_server_frame(json!({
                "type":"view", "sequence":2, "data":"safe fallback",
                "surface":{"kind":"future-kind","version":99},
                "view_revision":1
            }))
            .unwrap();
        let view = state.view().unwrap();
        assert!(view.surface.is_none());
        assert_eq!(view.text_projection(), "safe fallback");
    }

    #[test]
    fn malformed_and_non_session_host_frames_are_rejected() {
        let mut state = SessionState::default();
        assert!(
            state
                .apply_server_frame(
                    json!({"type":"view", "data":"missing sequence", "view_revision":0})
                )
                .is_err()
        );
        assert!(
            state
                .apply_server_frame(
                    json!({"type":"module_info", "world_id":"w", "current_sequence":1})
                )
                .is_err()
        );
    }
}
