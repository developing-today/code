//! Contract-frame state for one native shell session.
//!
//! This stays independent of Bevy so the network task can validate and
//! accumulate frames before handing state to the render thread. The native
//! client does not opt in to screen patches: every received view is therefore
//! a complete view, not a delta that needs a revision cache.

use anyhow::{Context, Result, bail};
use id::world_presentation::{ScreenSurface, WorldSurface};
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
        let surface = frame.get("surface").and_then(decode_host_surface);
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

/// Decode a view frame's `surface` in the host's normalized wire format.
///
/// The host sends the surface it built from the guest document, serialized
/// as a [`WorldSurface`] — for a screen that is the row-major `cells`
/// format, NOT the guest's `rows` document that
/// `id::world_presentation::parse_surface` accepts. Decoding the frame
/// value directly is the only path that sees real host screens; anything
/// unrecognized or invalid falls back to `None` (the caller then uses the
/// frame's legacy `data` text, exactly as for a surface-less view).
fn decode_host_surface(surface: &Value) -> Option<WorldSurface> {
    let decoded: WorldSurface = serde_json::from_value(surface.clone()).ok()?;
    match decoded {
        WorldSurface::Screen(screen) => {
            // Re-validate the normalized buffer (dimensions, cell count,
            // glyph safety) instead of trusting the wire value as-is.
            let validated = ScreenSurface::from_cells(
                usize::from(screen.width),
                usize::from(screen.height),
                screen.cells,
            )
            .ok()?;
            Some(WorldSurface::Screen(validated))
        }
        other => Some(other),
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
    /// The host replaced the world's program mid-session (a live install
    /// sends snapshot, then this notification, then a replacement view).
    ProgramChanged,
}

/// Accumulated host state for a single session stream.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SessionState {
    snapshot: Option<Value>,
    events: Vec<Value>,
    view: Option<SessionView>,
    program_module_hash: Option<String>,
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
            "program_changed" => {
                // A live program install is a normal point in a session
                // stream (snapshot, this notification, replacement view),
                // not a protocol failure: record the new module and let
                // the caller keep processing the stream.
                self.program_module_hash = frame["module_hash"].as_str().map(str::to_owned);
                Ok(ServerUpdate::ProgramChanged)
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

    /// Module hash named by the most recent `program_changed` notification,
    /// if the host has replaced the world's program during this session.
    #[must_use]
    pub fn program_module_hash(&self) -> Option<&str> {
        self.program_module_hash.as_deref()
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
        // The host's normalized wire format: a screen surface arrives as
        // row-major `cells`, not the guest document's `rows`.
        state
            .apply_server_frame(json!({
                "type":"view", "sequence":1, "data":"fallback",
                "surface":{"kind":"screen","version":1,"width":2,"height":1,"cells":[
                    {"symbol":"h","fg":0,"bg":0,"bold":false,"columns":1},
                    {"symbol":"i","fg":0,"bg":0,"bold":false,"columns":1}
                ]},
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
    fn program_changed_is_a_normal_mid_stream_notification() {
        // A live program install reaches a session as snapshot, then
        // program_changed, then a replacement view (the host's own
        // world_session contract test streams exactly this sequence).
        let mut state = SessionState::default();
        state
            .apply_server_frame(json!({"type":"snapshot", "snapshot":{"sequence":4}}))
            .unwrap();
        assert_eq!(
            state
                .apply_server_frame(
                    json!({"type":"program_changed", "module_hash":"blake3:new", "sequence":5})
                )
                .unwrap(),
            ServerUpdate::ProgramChanged
        );
        assert_eq!(state.program_module_hash(), Some("blake3:new"));
        assert_eq!(
            state
                .apply_server_frame(
                    json!({"type":"view", "sequence":5, "data":"new program", "view_revision":1})
                )
                .unwrap(),
            ServerUpdate::View
        );
        assert_eq!(state.view().unwrap().text_projection(), "new program");
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
