//! Contract-frame state for one native shell session.
//!
//! This stays independent of Bevy so the network task can validate and
//! accumulate frames before handing state to the render thread. The native
//! client does not opt in to screen patches: every received view is therefore
//! a complete view, not a delta that needs a revision cache.

use std::sync::{Arc, Mutex, MutexGuard};

use anyhow::{Context, Result, bail};
use id::frames::{read_frame, write_frame};
use id::world_net::{WorldClient, route_frame};
use id::world_presentation::{ScreenSurface, WorldSurface};
use serde_json::{Value, json};
use tokio::sync::mpsc;

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
    /// The host sent an F13.7 `provider_request` (step 2 frame, step 4
    /// local handling): the validated frame is queued on the state for
    /// the session's provider adapter to answer; session state itself
    /// does not interpret it.
    ProviderRequest,
    /// The host sent an agent-profile `semantic_diff` (contract revision
    /// 11, #6300 section 4.2): what changed, in world terms, scoped to
    /// this session's subscription. Render it with [`semantic_diff_text`].
    SemanticDiff,
    /// The host answered a `screen_subscribe` with the now-active
    /// subscription (`screen_subscription`, contract revision 11).
    ScreenSubscription,
    /// The host answered a `form_result` with `form_result_status`
    /// (#8938): the reply is kept for the presenter to fold through
    /// [`crate::form::FormPresenter::settle`], which rotates the
    /// presentation token on a rejection exactly as the SSH presenter
    /// does.
    FormResultStatus,
}

/// Render one `semantic_diff` frame as plain text: one line per change,
/// in world terms — a chat message, a changed screen row inside the
/// subscribed region, an object change — never repaint instructions.
/// Unknown change kinds render as a named line rather than vanishing,
/// so a newer host's diffs stay legible to this shell.
#[must_use]
pub fn semantic_diff_text(diff: &Value) -> String {
    let Some(changes) = diff["changes"].as_array() else {
        return String::new();
    };
    changes
        .iter()
        .map(|change| {
            let kind = change["kind"].as_str().unwrap_or_default();
            match kind {
                "chat_posted" => {
                    format!("chat: {}", change["text"].as_str().unwrap_or_default())
                }
                "region_changed" => format!(
                    "row {}: {}",
                    change["row"].as_u64().unwrap_or_default(),
                    change["text"].as_str().unwrap_or_default()
                ),
                "object_changed" => format!(
                    "object {} changed",
                    change["entity_ref"].as_str().unwrap_or_default()
                ),
                "input_applied" => "input applied".to_owned(),
                "program_changed" => "program changed".to_owned(),
                "system_event" => "system event".to_owned(),
                other => format!("change: {other}"),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Accumulated host state for a single session stream.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SessionState {
    snapshot: Option<Value>,
    events: Vec<Value>,
    view: Option<SessionView>,
    program_module_hash: Option<String>,
    provider_requests: Vec<Value>,
    semantic_diffs: Vec<Value>,
    screen_subscription: Option<Value>,
    form_result_status: Option<Value>,
}

/// Maximum durable events retained since the most recent snapshot.
///
/// The host only sends `snapshot` on join, lagged catch-up, and program
/// install, so a session that keeps up would otherwise accumulate every
/// `event` frame for its whole lifetime. Nothing in the running binary
/// consumes these (only views feed the presenter), so retention is a
/// bounded tail: the newest events, oldest dropped first.
pub const MAX_RETAINED_EVENTS: usize = 512;

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
                if self.events.len() > MAX_RETAINED_EVENTS {
                    let excess = self.events.len() - MAX_RETAINED_EVENTS;
                    self.events.drain(..excess);
                }
                Ok(ServerUpdate::Event)
            }
            "view" => {
                self.view = Some(SessionView::from_frame(&frame)?);
                Ok(ServerUpdate::View)
            }
            "provider_request" => {
                // F13.7 step 4: a provider request is validated by the
                // contract check above and queued for the provider
                // adapter, which applies the local grant/bounds check
                // and builds the reply. The frame must name its nonce
                // and provider to be answerable at all.
                if frame["nonce"].as_str().is_none_or(str::is_empty)
                    || frame["provider"].as_str().is_none_or(str::is_empty)
                {
                    bail!("provider_request frame has no nonce or provider");
                }
                self.provider_requests.push(frame);
                Ok(ServerUpdate::ProviderRequest)
            }
            "semantic_diff" => {
                // Agent profile (contract revision 11): retained like
                // events, as a bounded tail — a diff is derived
                // presentation, so dropping the oldest loses nothing
                // durable.
                self.semantic_diffs.push(frame);
                if self.semantic_diffs.len() > MAX_RETAINED_EVENTS {
                    let excess = self.semantic_diffs.len() - MAX_RETAINED_EVENTS;
                    self.semantic_diffs.drain(..excess);
                }
                Ok(ServerUpdate::SemanticDiff)
            }
            "screen_subscription" => {
                self.screen_subscription = Some(frame);
                Ok(ServerUpdate::ScreenSubscription)
            }
            "form_result_status" => {
                // #8938: the answer to one `form_result`. Retained for the
                // presenter's `settle`; a form the shell is not rendering
                // is simply ignored there, so this is never a failure.
                self.form_result_status = Some(frame);
                Ok(ServerUpdate::FormResultStatus)
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
    ///
    /// Bounded tail: at most [`MAX_RETAINED_EVENTS`] newest events are
    /// kept; older events are dropped on receipt, oldest first.
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

    /// The most recent `form_result_status` reply, if any (#8938).
    ///
    /// A reply is presentation input, not durable state: the presenter
    /// consumes it once with [`SessionState::take_form_result_status`].
    #[must_use]
    pub fn form_result_status(&self) -> Option<&Value> {
        self.form_result_status.as_ref()
    }

    /// Take the pending `form_result_status` reply, if one arrived.
    pub fn take_form_result_status(&mut self) -> Option<Value> {
        self.form_result_status.take()
    }

    /// Drain the queued F13.7 `provider_request` frames, oldest
    /// first. The session's provider adapter answers each one with a
    /// `provider_reply`; an undrained request is never answered
    /// implicitly.
    pub fn take_provider_requests(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.provider_requests)
    }

    /// Agent-profile semantic diffs received on this stream, newest
    /// last. Bounded tail like [`SessionState::events`].
    #[must_use]
    pub fn semantic_diffs(&self) -> &[Value] {
        &self.semantic_diffs
    }

    /// The active screen subscription the host last acknowledged with a
    /// `screen_subscription` frame, if any.
    #[must_use]
    pub fn screen_subscription(&self) -> Option<&Value> {
        self.screen_subscription.as_ref()
    }
}

/// Lock the shared session state, mapping a poisoned lock to an error
/// instead of panicking (the crate denies `panic` in non-test code).
fn lock_state(state: &Mutex<SessionState>) -> Result<MutexGuard<'_, SessionState>> {
    state
        .lock()
        .map_err(|_| anyhow::anyhow!("session state lock poisoned"))
}

/// Run one live session against a world host: send the join frame for
/// `capability`, then pump frames in both directions until the host ends
/// the session, a host frame fails contract validation, or the local
/// outbound channel closes (the shell exiting).
///
/// The reader runs in its own task because frame reads are not
/// cancel-safe (`id::frames::read_frame` documents this; the CLI's join
/// pump splits its streams the same way). Every host frame is validated
/// and folded into a [`SessionState`] exactly as the transcript harness
/// folds recorded frames, and every new view is published on `views` —
/// in the binary, that channel is what feeds
/// `render::PresentedView`. Outbound frames (built by
/// [`crate::events::native_event_frame`] or the crate's frame builders)
/// are routed to the session's `world` with [`route_frame`],
/// byte-identically to `WorldClient::send_json` on a client built with
/// `with_world(world)`: construct `client` unbound and let this
/// function own the routing, never both.
///
/// Returns the final accumulated state when the session ends.
///
/// # Errors
///
/// Fails when the join frame cannot be sent, a host frame is malformed
/// or out of contract, the host refuses the session (an `error` frame),
/// or an outbound frame cannot be sent.
pub async fn run_session(
    client: WorldClient,
    world: Option<&str>,
    capability: &str,
    after: Option<u64>,
    outbound: mpsc::Receiver<Value>,
    views: mpsc::UnboundedSender<SessionView>,
) -> Result<SessionState> {
    let (provider_requests, _dropped) = mpsc::unbounded_channel();
    run_session_inner(
        client,
        world,
        capability,
        after,
        outbound,
        views,
        provider_requests,
        None,
    )
    .await
}

/// [`run_session`] with F13.7 step-4 provider handling: validated
/// `provider_request` frames are forwarded on `provider_requests`
/// for the caller's [`crate::ProviderAdapter`] to answer through the
/// ordinary outbound channel, and `provider_offer` (the adapter's
/// offer frame for this world, when it has one) is sent immediately
/// after the join frame.
///
/// # Errors
///
/// Same failures as [`run_session`], plus a failure to send the
/// offer frame.
pub async fn run_session_with_providers(
    client: WorldClient,
    world: Option<&str>,
    capability: &str,
    after: Option<u64>,
    outbound: mpsc::Receiver<Value>,
    views: mpsc::UnboundedSender<SessionView>,
    provider_requests: mpsc::UnboundedSender<Value>,
    provider_offer: Option<Value>,
) -> Result<SessionState> {
    run_session_inner(
        client,
        world,
        capability,
        after,
        outbound,
        views,
        provider_requests,
        provider_offer,
    )
    .await
}

async fn run_session_inner(
    client: WorldClient,
    world: Option<&str>,
    capability: &str,
    after: Option<u64>,
    mut outbound: mpsc::Receiver<Value>,
    views: mpsc::UnboundedSender<SessionView>,
    provider_requests: mpsc::UnboundedSender<Value>,
    provider_offer: Option<Value>,
) -> Result<SessionState> {
    let (conn, mut send, mut recv) = client.into_parts();
    let join = route_frame(&join_frame(capability, after), world);
    write_frame(&mut send, join.as_bytes())
        .await
        .context("send join frame")?;
    if let Some(offer) = provider_offer {
        let routed = route_frame(&offer, world);
        write_frame(&mut send, routed.as_bytes())
            .await
            .context("send provider offer frame")?;
    }

    let shared = Arc::new(Mutex::new(SessionState::default()));
    let reader_state = Arc::clone(&shared);
    let mut reader = tokio::spawn(async move {
        loop {
            let Some(body) = read_frame(&mut recv).await.context("receive host frame")? else {
                break;
            };
            let frame: Value = serde_json::from_slice(&body).context("decode host frame")?;
            let view = {
                let mut state = lock_state(&reader_state)?;
                let update = state.apply_server_frame(frame)?;
                if update == ServerUpdate::ProviderRequest {
                    for request in state.take_provider_requests() {
                        // A dropped receiver means the shell is not
                        // answering provider requests this session;
                        // the host broker times the request out and
                        // the local refusal stays the default.
                        let _ = provider_requests.send(request);
                    }
                }
                if update == ServerUpdate::View {
                    state.view().cloned()
                } else {
                    None
                }
            };
            if let Some(view) = view {
                // A dropped receiver means the presenter is gone; the
                // session itself is unaffected, so keep reading.
                let _ = views.send(view);
            }
        }
        anyhow::Ok(())
    });

    let mut failure: Option<anyhow::Error> = None;
    loop {
        tokio::select! {
            outgoing = outbound.recv() => {
                let Some(frame) = outgoing else { break };
                let routed = route_frame(&frame, world);
                if let Err(error) = write_frame(&mut send, routed.as_bytes()).await {
                    failure = Some(error.context("send client frame"));
                    break;
                }
            }
            joined = &mut reader => {
                match joined {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => failure = Some(error),
                    Err(error) => {
                        failure = Some(anyhow::Error::from(error).context("session reader task"));
                    }
                }
                break;
            }
        }
    }
    reader.abort();
    conn.close(0_u32.into(), b"done");
    if let Some(error) = failure {
        return Err(error);
    }
    Ok(lock_state(&shared)?.clone())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
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
    fn provider_requests_are_queued_for_the_adapter_not_folded_into_state() {
        let mut state = SessionState::default();
        let frame = json!({
            "type": "provider_request", "nonce": "ab12",
            "provider": "client.file.read", "args": {"path": "notes.txt"},
            "deadline_ms": 5000
        });
        assert_eq!(
            state.apply_server_frame(frame.clone()).unwrap(),
            ServerUpdate::ProviderRequest
        );
        assert_eq!(state.take_provider_requests(), vec![frame]);
        assert!(state.take_provider_requests().is_empty());
        // A request that cannot be answered (no nonce) is rejected.
        assert!(
            state
                .apply_server_frame(json!({
                    "type": "provider_request", "provider": "client.file.read",
                    "args": {}, "deadline_ms": 5000
                }))
                .is_err()
        );
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

    #[test]
    fn event_retention_is_bounded_without_an_intervening_snapshot() {
        let mut state = SessionState::default();
        state
            .apply_server_frame(json!({"type":"snapshot", "snapshot":{"sequence":0}}))
            .unwrap();
        let total = MAX_RETAINED_EVENTS + 64;
        for sequence in 1..=total as u64 {
            state
                .apply_server_frame(json!({"type":"event", "event":{"sequence":sequence}}))
                .unwrap();
        }
        assert_eq!(state.events().len(), MAX_RETAINED_EVENTS);
        // Tail semantics: newest kept, oldest dropped first.
        assert_eq!(
            state.events().first().unwrap()["sequence"],
            (total - MAX_RETAINED_EVENTS + 1) as u64
        );
        assert_eq!(state.events().last().unwrap()["sequence"], total as u64);
    }
}
