//! The MCP bridge wiring for #7914 (spike step 2 of #6300).
//!
//! The pinned v1 tool surface lives in [`id_caps::mcp_bridge`]; this
//! module is the translator the design calls the bridge: it maps each
//! step-2 MCP tool call to F13.1 shell-contract frames, sends them over
//! an iroh world session, and renders the host's ServerFrames back as
//! tool results.
//!
//! Wired tools (design §10 step 2): `world.join`, `world.leave`,
//! `screen.read`, `screen.subscribe`, `chat.post`, `object.inspect`.
//! `presence.set` is defined on the pinned surface but has no F13.1
//! frame in contract v1 — presence in a world is the session itself
//! (join is present, leave is absent) — so this slice leaves it
//! unwired rather than inventing a frame for it; wiring it needs a
//! contract addition, not a bridge change.
//!
//! The bridge holds no credentials and no authority of its own: the
//! caller supplies the capability (minted out of band, e.g. by an
//! invite), the host checks the ceiling, and the bridge only carries
//! frames. Tool definitions are pinned at [`McpBridge::start`]; a
//! mid-session definition change terminates the session, exactly as
//! the surface module specifies.
//!
//! Two rules the wiring is built around, both from the contract's
//! agent profile (revision 11):
//!
//! * The stale-decision token is attached to an input only when the
//!   bridge has actually received a `view` frame. The host's
//!   `screen_subscription` acknowledgement and `semantic_diff` frames
//!   carry `view_revision: 0` as a placeholder when no view was ever
//!   sent, and the host's gate compares against `None` then —
//!   attaching the placeholder would make every such input look stale.
//! * `screen.read` and `object.inspect` are answered from the
//!   session's held state: the host pushes views and semantic diffs,
//!   the bridge retains them, and a read reports the latest known
//!   screen. There is no `screen_read` or `object_inspect` frame in
//!   the contract to send.

use std::time::Duration;

use iroh::{Endpoint, EndpointAddr};
use serde::{Deserialize, Serialize};
use tokio::time::Instant;

use crate::world_net::WorldClient;
use id_caps::mcp_bridge::{BridgeSession, v1_tool_definitions};

pub use id_caps::mcp_bridge::ToolDefinition;

/// How long one frame wait may take before the bridge gives up.
const FRAME_TIMEOUT: Duration = Duration::from_secs(10);
/// How long `join` waits for a `view` after the `snapshot`, for a
/// chat-only world that never sends one.
const JOIN_GRACE: Duration = Duration::from_millis(500);
/// How long `screen.read` waits for in-flight frames before reporting
/// the latest known screen.
const READ_QUIET: Duration = Duration::from_millis(100);
/// The longest `screen.read` spends draining in-flight frames.
const READ_DRAIN_MAX: Duration = Duration::from_secs(2);
/// Semantic-diff changes retained for `object.inspect` and reads.
/// A session holds at most this many; older changes drop.
const MAX_HELD_DIFFS: usize = 256;

/// One screen rectangle in cells, origin at the top-left — the region
/// a `screen.subscribe` names and a `screen.read` may scope to. The
/// same 500x200 ceiling the contract's `resize` accepts; the host
/// clamps views to the presentation maximum itself.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ScreenRegion {
    /// Leftmost column, zero-based.
    pub x: u16,
    /// Topmost row, zero-based.
    pub y: u16,
    /// Width in cells, at least 1.
    pub width: u16,
    /// Height in rows, at least 1.
    pub height: u16,
}

/// What `world.join` returns: the screen the host sent on join, if
/// the world renders one — a chat-only world sends no `view`, so the
/// text is empty and the revision is `None`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JoinOutcome {
    /// The latest view text, empty when the world sent no view.
    pub screen_text: String,
    /// The screen revision of that view, if a view was received.
    pub view_revision: Option<u64>,
}

/// What `screen.subscribe` returns: the host's acknowledgement of the
/// now-active subscription.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubscriptionOutcome {
    /// The subscribed region, echoed; absent for event-only
    /// subscriptions.
    pub region: Option<ScreenRegion>,
    /// The subscribed semantic event classes, echoed; absent for
    /// region-only subscriptions.
    pub event_classes: Option<Vec<String>>,
    /// The session's screen revision at subscribe time, as the host
    /// reports it. `0` when the world has sent no view yet — a
    /// placeholder, never a revision to decide an input against.
    pub view_revision: u64,
}

/// What `screen.read` returns: the latest known screen, optionally
/// scoped to a region, plus the semantic diffs received since the
/// last read.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScreenRead {
    /// The latest view text (region-scoped when a region was given),
    /// empty when the world has sent no view.
    pub text: String,
    /// The screen revision of that view, if a view was received.
    pub view_revision: Option<u64>,
    /// Semantic-diff changes received since the last read, in
    /// arrival order.
    pub diffs: Vec<serde_json::Value>,
}

/// What `chat.post` returns: the posted text and the semantic diff
/// that describes it in world terms.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChatOutcome {
    /// The posted text, as the host's diff reports it.
    pub text: String,
    /// The screen revision known when the diff arrived, if a view
    /// was received.
    pub view_revision: Option<u64>,
    /// The `chat_posted` change object from the host's diff.
    pub diff: serde_json::Value,
}

/// What `object.inspect` returns: the latest change the session has
/// seen for one visible object.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObjectOutcome {
    /// The inspected `entity_ref`, as named by the diff.
    pub entity_ref: String,
    /// The latest `object_changed` change object for it.
    pub change: serde_json::Value,
    /// The screen revision known when the change arrived, if a view
    /// was received.
    pub view_revision: Option<u64>,
}

/// Failures of one bridge tool call. A terminated session stays
/// terminated: after [`BridgeError::DefinitionChanged`] the bridge
/// refuses further tool calls.
#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    /// A tool was called before `world.join`.
    #[error("bridge is not joined to a world")]
    NotJoined,
    /// `world.join` was called on a session that already joined.
    #[error("bridge already joined a world")]
    AlreadyJoined,
    /// A tool was called after `world.leave`. A bridge serves one
    /// session; a new session needs a new bridge.
    #[error("bridge left the world; start a new bridge to join again")]
    Left,
    /// The host closed the session stream.
    #[error("world host closed the session")]
    HostClosed,
    /// The host refused a frame, with its message.
    #[error("world host refused the frame: {0}")]
    HostRefused(String),
    /// An input carried a stale-decision token: it was decided
    /// against `decided` while the session's current revision is
    /// `current`. Nothing was committed.
    #[error(
        "stale decision: input was decided against screen revision {decided}, current is {}",
        current.map_or_else(|| "none".to_owned(), |revision| revision.to_string())
    )]
    StaleDecision {
        /// The revision the input was decided against.
        decided: u64,
        /// The session's current revision, if a view was received.
        current: Option<u64>,
    },
    /// `object.inspect` named an object the session has never seen
    /// in a semantic diff.
    #[error("no visible object named {0}")]
    ObjectNotVisible(String),
    /// The definition set could not be serialized for hashing.
    #[error("tool definitions could not be serialized for pinning")]
    InvalidDefinitions,
    /// The definition set changed after the session was pinned. The
    /// session is terminated.
    #[error("tool definitions changed after pinning; session terminated")]
    DefinitionChanged {
        /// Hash recorded at session start.
        expected: [u8; 32],
        /// Hash observed during the later check.
        actual: [u8; 32],
    },
    /// A frame wait ran out: the host did not answer in time.
    #[error("timed out waiting for a world frame")]
    Timeout,
    /// The iroh transport failed.
    #[error("transport: {0}")]
    Transport(#[from] anyhow::Error),
}

impl From<id_caps::mcp_bridge::BridgeError> for BridgeError {
    fn from(error: id_caps::mcp_bridge::BridgeError) -> Self {
        match error {
            id_caps::mcp_bridge::BridgeError::DefinitionChanged { expected, actual } => {
                Self::DefinitionChanged { expected, actual }
            }
            id_caps::mcp_bridge::BridgeError::InvalidDefinitions => Self::InvalidDefinitions,
        }
    }
}

/// The screen state one session has received: the latest view and the
/// semantic diffs, kept so `screen.read` and `object.inspect` can be
/// answered without a host round-trip.
#[derive(Clone, Debug, Default)]
struct ScreenTracker {
    /// Latest view text, empty until the first `view` frame.
    view_text: String,
    /// Revision of the latest `view` frame, if one was received.
    view_revision: Option<u64>,
    /// Every semantic-diff change received, newest last, capped at
    /// [`MAX_HELD_DIFFS`].
    seen_changes: Vec<serde_json::Value>,
    /// Changes received since the last `screen.read`, capped the
    /// same way; a read takes them.
    pending_changes: Vec<serde_json::Value>,
}

impl ScreenTracker {
    /// Fold one ServerFrame into the held state. Only `view` frames
    /// move the revision: `semantic_diff` and `screen_subscription`
    /// frames carry `view_revision: 0` as a placeholder when the
    /// world has sent no view, and the host's stale-decision gate
    /// compares against `None` then — adopting the placeholder would
    /// make every later input look stale.
    fn apply_frame(&mut self, frame: &serde_json::Value) {
        let Some(frame_type) = frame.get("type").and_then(serde_json::Value::as_str) else {
            return;
        };
        match frame_type {
            "view" => {
                if let Some(data) = frame.get("data").and_then(serde_json::Value::as_str) {
                    self.view_text = data.to_owned();
                }
                if let Some(revision) = frame
                    .get("view_revision")
                    .and_then(serde_json::Value::as_u64)
                {
                    self.view_revision = Some(revision);
                }
            }
            "semantic_diff" => {
                if let Some(changes) = frame.get("changes").and_then(serde_json::Value::as_array) {
                    for change in changes {
                        push_capped(&mut self.seen_changes, change.clone());
                        push_capped(&mut self.pending_changes, change.clone());
                    }
                }
            }
            _ => {}
        }
    }

    /// The latest change the session has seen for one object, if any.
    fn latest_change_for(&self, entity_ref: &str) -> Option<&serde_json::Value> {
        self.seen_changes.iter().rev().find(|change| {
            change.get("kind").and_then(serde_json::Value::as_str) == Some("object_changed")
                && change.get("entity_ref").and_then(serde_json::Value::as_str) == Some(entity_ref)
        })
    }
}

/// Push while keeping at most [`MAX_HELD_DIFFS`] entries, dropping the
/// oldest first.
fn push_capped(changes: &mut Vec<serde_json::Value>, change: serde_json::Value) {
    changes.push(change);
    if changes.len() > MAX_HELD_DIFFS {
        let excess = changes.len() - MAX_HELD_DIFFS;
        changes.drain(..excess);
    }
}

/// The view text scoped to a region: the rows inside it, each clipped
/// to its columns. Rows past the end of the text are empty.
fn scope_text_to_region(text: &str, region: &ScreenRegion) -> String {
    let x = usize::from(region.x);
    let y = usize::from(region.y);
    let width = usize::from(region.width);
    let height = usize::from(region.height);
    text.lines()
        .skip(y)
        .take(height)
        .map(|line| line.chars().skip(x).take(width).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The MCP bridge: one pinned session over one iroh world connection.
#[derive(Debug)]
pub struct McpBridge {
    /// The pinned tool definitions; a mismatch terminates the session.
    session: BridgeSession,
    /// The live world session, once `world.join` opens it.
    client: Option<WorldClient>,
    /// Whether `world.leave` closed the session. Terminal: a bridge
    /// serves one session.
    left: bool,
    /// The screen state this session has received.
    screen: ScreenTracker,
}

impl McpBridge {
    /// Start a bridge and pin the v1 tool definitions.
    ///
    /// # Errors
    ///
    /// Fails only if the static surface cannot be serialized for
    /// hashing, which never happens for the pinned definitions.
    pub fn start() -> Result<Self, BridgeError> {
        Ok(Self {
            session: BridgeSession::start(&v1_tool_definitions())?,
            client: None,
            left: false,
            screen: ScreenTracker::default(),
        })
    }

    /// Whether the bridge currently holds a live world session.
    #[must_use]
    pub fn is_joined(&self) -> bool {
        self.client.is_some()
    }

    /// The screen revision of the latest `view` frame, if one was
    /// received. `None` until the first view — including after a
    /// `screen_subscription` acknowledgement, whose `0` is a
    /// placeholder, not a revision.
    #[must_use]
    pub const fn view_revision(&self) -> Option<u64> {
        self.screen.view_revision
    }

    /// The pinned definition hash, for audit/debug output without
    /// exposing definitions.
    #[must_use]
    pub const fn pinned_definition_hash(&self) -> [u8; 32] {
        self.session.pinned_definition_hash()
    }

    /// Whether a definition mismatch has terminated this session.
    #[must_use]
    pub const fn is_terminated(&self) -> bool {
        self.session.is_terminated()
    }

    /// Check that the server still serves the pinned definitions.
    /// A mismatch terminates the session; later checks keep failing.
    ///
    /// # Errors
    ///
    /// Fails with [`BridgeError::DefinitionChanged`] on a mismatch,
    /// or [`BridgeError::InvalidDefinitions`] when the supplied set
    /// cannot be serialized.
    pub fn verify_definitions(
        &mut self,
        definitions: &[ToolDefinition],
    ) -> Result<(), BridgeError> {
        self.session.verify_definitions(definitions)?;
        Ok(())
    }

    /// The live client, or the reason there is none.
    fn client_mut(&mut self) -> Result<&mut WorldClient, BridgeError> {
        if self.left {
            return Err(BridgeError::Left);
        }
        self.client.as_mut().ok_or(BridgeError::NotJoined)
    }

    /// Read one frame, waiting until `deadline`, and fold it into the
    /// held screen state. `None` on a closed stream is reported by the
    /// caller; a wait past the deadline is [`BridgeError::Timeout`].
    async fn recv_until(
        &mut self,
        deadline: Instant,
    ) -> Result<Option<serde_json::Value>, BridgeError> {
        let frame = {
            let client = self.client_mut()?;
            match tokio::time::timeout_at(deadline, client.recv_json()).await {
                Ok(Ok(frame)) => frame,
                Ok(Err(error)) => return Err(BridgeError::Transport(error)),
                Err(_) => return Err(BridgeError::Timeout),
            }
        };
        if let Some(frame) = &frame {
            self.screen.apply_frame(frame);
        }
        Ok(frame)
    }

    /// Read one frame, waiting up to [`FRAME_TIMEOUT`].
    async fn next_frame(&mut self) -> Result<serde_json::Value, BridgeError> {
        let deadline = Instant::now() + FRAME_TIMEOUT;
        self.recv_until(deadline)?.ok_or(BridgeError::HostClosed)
    }

    /// The host's message from an `error` frame, or a fallback when
    /// the frame carries none.
    fn error_message(frame: &serde_json::Value) -> String {
        frame
            .get("message")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("host refused the frame")
            .to_owned()
    }

    /// `world.join`: dial the host over iroh, open a session for
    /// `world`, and join it as an agent-profile shell
    /// (`agent_profile: true`), so the host answers with semantic
    /// diffs. The capability is supplied by the caller — minted out
    /// of band, e.g. by an invite — and the bridge never sees any
    /// credential beyond it.
    ///
    /// A world that renders a screen answers with a `view`; a
    /// chat-only world answers with a `snapshot` alone, and the join
    /// succeeds without a revision — the screen is then read purely
    /// from the semantic diffs the subscription delivers.
    ///
    /// # Errors
    ///
    /// Fails when already joined or after leaving, when the host
    /// refuses the join, when the transport fails, or when no
    /// snapshot or view arrives in time.
    pub async fn join(
        &mut self,
        endpoint: &Endpoint,
        addr: EndpointAddr,
        world: Option<&str>,
        capability: &str,
    ) -> Result<JoinOutcome, BridgeError> {
        if self.left {
            return Err(BridgeError::Left);
        }
        if self.client.is_some() {
            return Err(BridgeError::AlreadyJoined);
        }
        let client = WorldClient::connect(endpoint, addr)
            .await?
            .with_world(world);
        self.client = Some(client);
        let join = serde_json::json!({
            "type": "join",
            "capability": capability,
            "after": null,
            "agent_profile": true,
        });
        self.client_mut()?.send_json(&join).await?;
        let deadline = Instant::now() + FRAME_TIMEOUT;
        let mut grace: Option<Instant> = None;
        let mut seen_snapshot = false;
        loop {
            let until = match grace {
                Some(grace) if grace < deadline => grace,
                _ => deadline,
            };
            match self.recv_until(until).await {
                Ok(Some(frame)) => match frame.get("type").and_then(serde_json::Value::as_str) {
                    Some("view") => {
                        return Ok(JoinOutcome {
                            screen_text: self.screen.view_text.clone(),
                            view_revision: self.screen.view_revision,
                        });
                    }
                    Some("snapshot") => {
                        seen_snapshot = true;
                        if grace.is_none() {
                            grace = Some(Instant::now() + JOIN_GRACE);
                        }
                    }
                    Some("error") => {
                        return Err(BridgeError::HostRefused(Self::error_message(&frame)));
                    }
                    _ => {}
                },
                Ok(None) => return Err(BridgeError::HostClosed),
                Err(BridgeError::Timeout) => {
                    if seen_snapshot {
                        return Ok(JoinOutcome {
                            screen_text: self.screen.view_text.clone(),
                            view_revision: self.screen.view_revision,
                        });
                    }
                    return Err(BridgeError::Timeout);
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// `screen.subscribe`: subscribe this session to a screen region
    /// and/or semantic event classes. At least one of the two must be
    /// present; an empty `event_classes` list means all classes. The
    /// host answers with a `screen_subscription` frame echoing the
    /// active subscription.
    ///
    /// # Errors
    ///
    /// Fails when not joined, when the host refuses the subscription
    /// (e.g. an unknown event class or an out-of-bounds region), or
    /// when no acknowledgement arrives in time.
    pub async fn subscribe(
        &mut self,
        region: Option<ScreenRegion>,
        event_classes: Option<Vec<String>>,
    ) -> Result<SubscriptionOutcome, BridgeError> {
        let subscribe = serde_json::json!({
            "type": "screen_subscribe",
            "region": region,
            "event_classes": event_classes,
        });
        self.client_mut()?.send_json(&subscribe).await?;
        loop {
            let frame = self.next_frame().await?;
            match frame.get("type").and_then(serde_json::Value::as_str) {
                Some("screen_subscription") => {
                    let ack_region = frame
                        .get("region")
                        .and_then(|value| serde_json::from_value(value.clone()).ok())
                        .flatten();
                    let ack_classes = frame
                        .get("event_classes")
                        .and_then(|value| serde_json::from_value(value.clone()).ok())
                        .flatten();
                    let revision = frame
                        .get("view_revision")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    return Ok(SubscriptionOutcome {
                        region: ack_region,
                        event_classes: ack_classes,
                        view_revision: revision,
                    });
                }
                Some("error") => {
                    return Err(BridgeError::HostRefused(Self::error_message(&frame)));
                }
                _ => {}
            }
        }
    }

    /// Drain frames that arrive within `quiet` of each other, up to
    /// `max` total, folding them into the held screen state.
    async fn drain_quiet(&mut self, quiet: Duration, max: Duration) -> Result<(), BridgeError> {
        let end = Instant::now() + max;
        loop {
            let now = Instant::now();
            if now >= end {
                return Ok(());
            }
            let until = {
                let quiet_end = now + quiet;
                if quiet_end < end { quiet_end } else { end }
            };
            match self.recv_until(until).await {
                Ok(Some(_)) => {}
                Ok(None) => return Err(BridgeError::HostClosed),
                Err(BridgeError::Timeout) => return Ok(()),
                Err(error) => return Err(error),
            }
        }
    }

    /// `screen.read`: report the latest known screen — the last `view`
    /// the host sent, scoped to `region` when one is given — plus the
    /// semantic diffs received since the last read. The host pushes
    /// views and diffs; a read reports what the session holds, so it
    /// never waits on the host beyond in-flight frames.
    ///
    /// # Errors
    ///
    /// Fails when not joined, or when the host closed the session.
    pub async fn read_screen(
        &mut self,
        region: Option<ScreenRegion>,
    ) -> Result<ScreenRead, BridgeError> {
        self.drain_quiet(READ_QUIET, READ_DRAIN_MAX).await?;
        let text = match region {
            Some(region) => scope_text_to_region(&self.screen.view_text, &region),
            None => self.screen.view_text.clone(),
        };
        Ok(ScreenRead {
            text,
            view_revision: self.screen.view_revision,
            diffs: std::mem::take(&mut self.screen.pending_changes),
        })
    }

    /// `chat.post`: post a chat message in-world, carrying the
    /// screen revision the message was decided against when one is
    /// known (the stale-decision token). The call returns when the
    /// host's `chat_posted` semantic diff for this message arrives —
    /// the diff is the screen read of the post, in world terms.
    ///
    /// # Errors
    ///
    /// Fails when not joined, when the host refuses the post, when
    /// the revision is stale ([`BridgeError::StaleDecision`], nothing
    /// committed), or when no diff arrives in time.
    pub async fn post_chat(&mut self, text: &str) -> Result<ChatOutcome, BridgeError> {
        let decided = self.screen.view_revision;
        let mut chat = serde_json::json!({
            "type": "chat",
            "text": text,
        });
        if let Some(revision) = decided {
            chat["screen_revision"] = serde_json::json!(revision);
        }
        self.client_mut()?.send_json(&chat).await?;
        let deadline = Instant::now() + FRAME_TIMEOUT;
        loop {
            if Instant::now() >= deadline {
                return Err(BridgeError::Timeout);
            }
            let frame = self.next_frame().await?;
            match frame.get("type").and_then(serde_json::Value::as_str) {
                Some("semantic_diff") => {
                    if let Some(changes) =
                        frame.get("changes").and_then(serde_json::Value::as_array)
                    {
                        for change in changes {
                            let is_ours = change.get("kind").and_then(serde_json::Value::as_str)
                                == Some("chat_posted")
                                && change.get("text").and_then(serde_json::Value::as_str)
                                    == Some(text);
                            if is_ours {
                                return Ok(ChatOutcome {
                                    text: text.to_owned(),
                                    view_revision: self.screen.view_revision,
                                    diff: change.clone(),
                                });
                            }
                        }
                    }
                }
                Some("error") => {
                    let code = frame.get("code").and_then(serde_json::Value::as_str);
                    if code == Some("stale_decision") {
                        if let Some(decided) = decided {
                            return Err(BridgeError::StaleDecision {
                                decided,
                                current: self.screen.view_revision,
                            });
                        }
                    }
                    return Err(BridgeError::HostRefused(Self::error_message(&frame)));
                }
                _ => {}
            }
        }
    }

    /// `object.inspect`: report the latest change the session has
    /// seen for one visible object, by the `entity_ref` its diffs
    /// name. Inspection reads what the session has observed — a
    /// shell client cannot make the host produce an `object_changed`
    /// diff, so an object the session has never seen is not visible.
    ///
    /// # Errors
    ///
    /// Fails when not joined, or with [`BridgeError::ObjectNotVisible`]
    /// when no diff has named the object.
    pub fn inspect_object(&self, entity_ref: &str) -> Result<ObjectOutcome, BridgeError> {
        if self.left {
            return Err(BridgeError::Left);
        }
        if self.client.is_none() {
            return Err(BridgeError::NotJoined);
        }
        match self.screen.latest_change_for(entity_ref) {
            Some(change) => Ok(ObjectOutcome {
                entity_ref: entity_ref.to_owned(),
                change: change.clone(),
                view_revision: self.screen.view_revision,
            }),
            None => Err(BridgeError::ObjectNotVisible(entity_ref.to_owned())),
        }
    }

    /// `world.leave`: close the session stream. There is no leave
    /// frame in contract v1 — leaving is closing the session, and
    /// presence ends with it. Terminal: further tool calls fail.
    ///
    /// # Errors
    ///
    /// Fails when not joined.
    pub async fn leave(&mut self) -> Result<(), BridgeError> {
        if self.left {
            return Err(BridgeError::Left);
        }
        match self.client.take() {
            Some(client) => {
                client.close();
                self.left = true;
                Ok(())
            }
            None => Err(BridgeError::NotJoined),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn bridge_starts_pinned_and_tampering_terminates() {
        let mut bridge = McpBridge::start().expect("the static surface pins");
        let definitions = v1_tool_definitions();
        bridge
            .verify_definitions(&definitions)
            .expect("the pinned surface verifies");
        assert!(!bridge.is_terminated());
        let mut tampered = definitions.clone();
        tampered[0].ceiling = "credential.read".to_owned();
        let error = bridge
            .verify_definitions(&tampered)
            .expect_err("tampering must terminate");
        assert!(matches!(error, BridgeError::DefinitionChanged { .. }));
        assert!(bridge.is_terminated());
        assert!(bridge.verify_definitions(&definitions).is_err());
    }

    #[test]
    fn tool_calls_before_join_fail_without_io() {
        let mut bridge = McpBridge::start().expect("the static surface pins");
        assert!(!bridge.is_joined());
        assert_eq!(bridge.view_revision(), None);
        assert!(matches!(
            bridge.inspect_object("door:1"),
            Err(BridgeError::NotJoined)
        ));
    }

    #[tokio::test]
    async fn async_tool_calls_before_join_fail_without_io() {
        let mut bridge = McpBridge::start().expect("the static surface pins");
        assert!(matches!(
            bridge.post_chat("hello").await,
            Err(BridgeError::NotJoined)
        ));
        assert!(matches!(
            bridge.read_screen(None).await,
            Err(BridgeError::NotJoined)
        ));
        assert!(matches!(bridge.leave().await, Err(BridgeError::NotJoined)));
    }

    #[test]
    fn tracker_keeps_only_real_view_revisions() {
        let mut tracker = ScreenTracker::default();
        // A subscription acknowledgement's 0 is a placeholder, not a
        // revision: the tracker only learns revisions from views.
        tracker.apply_frame(&json!({
            "type": "screen_subscription",
            "event_classes": ["chat_posted"],
            "view_revision": 0,
        }));
        assert_eq!(tracker.view_revision, None);
        // A diff's revision is the same placeholder when no view was
        // ever sent.
        tracker.apply_frame(&json!({
            "type": "semantic_diff",
            "sequence": 1,
            "view_revision": 0,
            "changes": [{ "kind": "chat_posted", "participant_id": 1, "text": "hi" }],
        }));
        assert_eq!(tracker.view_revision, None);
        assert_eq!(tracker.pending_changes.len(), 1);
        tracker.apply_frame(&json!({
            "type": "view",
            "sequence": 1,
            "data": "hello ada",
            "view_revision": 7,
        }));
        assert_eq!(tracker.view_text, "hello ada");
        assert_eq!(tracker.view_revision, Some(7));
    }

    #[test]
    fn inspect_finds_the_newest_change_for_an_object() {
        let mut tracker = ScreenTracker::default();
        tracker.apply_frame(&json!({
            "type": "semantic_diff",
            "sequence": 1,
            "view_revision": 0,
            "changes": [{ "kind": "object_changed", "participant_id": 1, "entity_ref": "door:1" }],
        }));
        tracker.apply_frame(&json!({
            "type": "semantic_diff",
            "sequence": 2,
            "view_revision": 0,
            "changes": [
                { "kind": "object_changed", "participant_id": 1, "entity_ref": "door:1", "state": "open" },
                { "kind": "chat_posted", "participant_id": 1, "text": "hi" },
            ],
        }));
        let latest = tracker
            .latest_change_for("door:1")
            .expect("the object was seen twice");
        assert_eq!(latest["state"], "open");
        assert!(tracker.latest_change_for("door:2").is_none());
        assert!(tracker.latest_change_for("door").is_none());
    }

    #[test]
    fn region_scoping_clips_rows_and_columns() {
        let text = "abcdef\nghijkl\nmnopqr";
        let region = ScreenRegion {
            x: 2,
            y: 1,
            width: 3,
            height: 2,
        };
        assert_eq!(scope_text_to_region(text, &region), "hij\nopq");
        let past_end = ScreenRegion {
            x: 0,
            y: 5,
            width: 4,
            height: 2,
        };
        assert_eq!(scope_text_to_region(text, &past_end), "");
    }
}
