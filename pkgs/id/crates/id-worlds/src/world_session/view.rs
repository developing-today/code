//! View state and view sending (split from the parent module,
//! #8594): presence accounting, the per-session view-revision state,
//! and the coalesced view sender, plus the viewport shape and the
//! paste normalizer the frame handlers share. No behavior or
//! public-path change: the parent re-exports the shared items.

use super::*;

/// Returns the participant's presence when the session ends, however it ends.
pub(crate) struct PresenceGuard {
    pub(crate) world: WorldHandle,
    pub(crate) participant: u64,
}

impl Drop for PresenceGuard {
    fn drop(&mut self) {
        let world = self.world.clone();
        let participant = self.participant;
        tokio::spawn(async move {
            world.presence(participant, false).await;
        });
    }
}

/// An agent-profile session's active screen subscription (contract
/// revision 11, #6300 section 4.2), set by a `screen_subscribe` frame.
/// Connection-local presentation context, like the viewport: it is
/// never journaled and never seen by other sessions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ScreenSubscriptionState {
    /// The subscribed screen region, if any.
    pub(crate) region: Option<ScreenRegion>,
    /// The subscribed semantic event classes: `None` means no event
    /// diffs (a region-only subscription), an empty list means all
    /// classes.
    pub(crate) event_classes: Option<Vec<String>>,
}

#[derive(Default)]
pub(crate) struct ViewRevisionState {
    next_revision: u64,
    screen: Option<(u64, crate::world_presentation::ScreenSurface)>,
    /// The participant this connection joined as, bound into resume tokens.
    pub(crate) participant: u64,
    /// A resume token presented at join and not yet consumed by a view send.
    pub(crate) pending_resume: Option<String>,
    /// Agent profile (contract revision 11): set from the join frame's
    /// `agent_profile` capability flag.
    pub(crate) agent_profile: bool,
    /// The active screen subscription, once the agent sends
    /// `screen_subscribe`.
    pub(crate) subscription: Option<ScreenSubscriptionState>,
    /// The screen revision most recently sent to this session — the
    /// revision an input's stale-decision token is judged against.
    current_revision: Option<u64>,
    /// The last screen sent to an agent-profile session, retained so the
    /// next view can be described as a semantic region diff. Kept apart
    /// from the patch baseline above: patches are a negotiated human
    /// shell feature, region diffs are the agent profile's.
    agent_screen: Option<(u64, crate::world_presentation::ScreenSurface)>,
}

impl ViewRevisionState {
    pub(crate) fn reset(&mut self) {
        self.screen = None;
    }

    /// The screen revision this session most recently received, if it
    /// has received any view at all.
    pub(crate) fn current_revision(&self) -> Option<u64> {
        self.current_revision
    }

    /// Record that the view at `view_revision` reached this session:
    /// it becomes the current screen revision, and an agent-profile
    /// session retains the screen as its next region-diff baseline.
    fn note_view_sent(
        &mut self,
        view_revision: u64,
        screen: Option<&crate::world_presentation::ScreenSurface>,
    ) {
        self.current_revision = Some(view_revision);
        if self.agent_profile {
            if let Some(screen) = screen {
                self.agent_screen = Some((view_revision, screen.clone()));
            }
        }
    }

    /// Whether this session wants a semantic diff for a world event of
    /// semantic class `class` (#6300 section 4.2 subscription scoping).
    pub(crate) fn wants_event_diff(&self, class: &str) -> bool {
        if !self.agent_profile {
            return false;
        }
        let Some(subscription) = &self.subscription else {
            return false;
        };
        match &subscription.event_classes {
            None => false,
            Some(classes) => classes.is_empty() || classes.iter().any(|named| named == class),
        }
    }
}

/// The text of one screen row's cell range `from..to` (cell indices are
/// column indices; a wide glyph's continuation cell contributes nothing,
/// exactly as the row renders).
fn row_text(
    screen: &crate::world_presentation::ScreenSurface,
    row: u16,
    from: u16,
    to: u16,
) -> String {
    let width = usize::from(screen.width);
    let start = usize::from(row) * width + usize::from(from);
    let end = usize::from(row) * width + usize::from(to);
    screen.cells[start..end]
        .iter()
        .map(|cell| cell.symbol.as_str())
        .collect()
}

/// The `region_changed` changes between the screen an agent session
/// last received and `next`, restricted to `region` (#6300 section
/// 4.2): one change per row inside the region whose text changed,
/// carrying that row's new text inside the region. Rows outside the
/// region never appear — that absence is the scoping. A previous
/// screen of different dimensions cannot be diffed row-wise, so it
/// yields no changes (the full view frame already carried the truth).
fn region_changes(
    previous: Option<&(u64, crate::world_presentation::ScreenSurface)>,
    next: &crate::world_presentation::ScreenSurface,
    region: ScreenRegion,
) -> Vec<serde_json::Value> {
    let Some((_, previous)) = previous else {
        return Vec::new();
    };
    if previous.width != next.width || previous.height != next.height {
        return Vec::new();
    }
    let mut changes = Vec::new();
    let rows_end = region.y.saturating_add(region.height).min(next.height);
    let cols_end = region.x.saturating_add(region.width).min(next.width);
    if region.x >= cols_end {
        return Vec::new();
    }
    for row in region.y..rows_end {
        let new_text = row_text(next, row, region.x, cols_end);
        let old_text = row_text(previous, row, region.x, cols_end);
        if new_text != old_text {
            changes.push(serde_json::json!({
                "kind": "region_changed",
                "row": row,
                "text": new_text.trim_end(),
            }));
        }
    }
    changes
}

pub(crate) async fn send_view<I: SessionIo>(
    service: &WorldService,
    io: &mut I,
    token: &JoinCapability,
    sequence: u64,
    viewport: Option<Viewport>,
    view_state: &mut ViewRevisionState,
    screen_patches: bool,
) -> bool {
    let world = service.world();
    let view = match viewport {
        Some(viewport) => {
            let width = u16::try_from(viewport.width).unwrap_or(u16::MAX);
            let height = u16::try_from(viewport.height).unwrap_or(u16::MAX);
            world.view_at(token.clone(), width, height).await
        }
        None => world.view(token.clone()).await,
    };
    match view {
        Ok(Some(data)) => {
            if data.len() > MAX_FRAME_BYTES {
                return send_error(service.frame_meter(), io, "world presentation is too large")
                    .await;
            }
            let surface = match crate::world_presentation::parse_surface(&data) {
                Ok(Some(crate::world_presentation::WorldSurface::Screen(screen))) => {
                    let screen = viewport.map_or(screen.clone(), |viewport| {
                        screen.fit(viewport.width, viewport.height)
                    });
                    Some(crate::world_presentation::WorldSurface::Screen(screen))
                }
                Ok(surface) => surface,
                Err(_) => {
                    return send_error(
                        service.frame_meter(),
                        io,
                        "invalid world screen presentation",
                    )
                    .await;
                }
            };
            let screen = match surface.as_ref() {
                Some(crate::world_presentation::WorldSurface::Screen(screen)) => Some(screen),
                _ => None,
            };

            // A resume token presented at join restores its retained baseline
            // once the viewport is known, so this first frame can be a
            // catch-up patch. An unknown, expired, tampered, foreign or
            // shape-mismatched token is a miss: the client gets a full view.
            if screen_patches && view_state.screen.is_none() {
                if let (Some(resume), Some(current)) = (view_state.pending_resume.take(), screen) {
                    let base = service.resume_baselines.lock().await.take_matching(
                        &resume,
                        view_state.participant,
                        current.version,
                        current.width,
                        current.height,
                    );
                    if let Some(base) = base {
                        view_state.next_revision = view_state
                            .next_revision
                            .max(base.revision.saturating_add(1));
                        view_state.screen = Some((base.revision, base.screen));
                    }
                }
            }

            let view_revision = view_state.next_revision;
            view_state.next_revision = view_state.next_revision.saturating_add(1);
            // Retain the revision this frame applies and name it with a fresh
            // token, so the next reconnect can resume from exactly here.
            let resume_token = match (screen_patches, screen) {
                (true, Some(current)) => Some(service.resume_baselines.lock().await.issue(
                    resume::ScreenBaseline::new(
                        view_state.participant,
                        view_revision,
                        current.version,
                        current.width,
                        current.height,
                        current.clone(),
                    ),
                )),
                _ => None,
            };

            if let (true, Some(screen), Some((base_view_revision, previous))) =
                (screen_patches, screen, view_state.screen.as_ref())
            {
                if let Some(patch) = crate::world_presentation::ScreenPatch::diff(previous, screen)
                {
                    let delta_frame = ServerFrame::View {
                        sequence,
                        data: "",
                        surface: None,
                        view_revision,
                        base_view_revision: Some(*base_view_revision),
                        patch: Some(&patch),
                        resume_token: resume_token.as_deref(),
                    };
                    let full_frame = ServerFrame::View {
                        sequence,
                        data: &data,
                        surface: surface.as_ref(),
                        view_revision,
                        base_view_revision: None,
                        patch: None,
                        resume_token: resume_token.as_deref(),
                    };
                    let delta_size = serde_json::to_vec(&delta_frame).map(|bytes| bytes.len());
                    let full_size = serde_json::to_vec(&full_frame).map(|bytes| bytes.len());
                    if matches!((delta_size, full_size), (Ok(delta), Ok(full)) if delta < full) {
                        let sent = send_json(service.frame_meter(), io, &delta_frame).await;
                        if sent {
                            view_state.screen = Some((view_revision, screen.clone()));
                            view_state.note_view_sent(view_revision, Some(screen));
                        }
                        return sent;
                    }
                }
            }

            // Agent profile (contract revision 11): a region subscriber
            // also gets this view described as a semantic region diff,
            // computed against the last screen the session received.
            let region_diff = match (
                view_state.agent_profile,
                view_state
                    .subscription
                    .as_ref()
                    .and_then(|subscription| subscription.region),
                screen,
            ) {
                (true, Some(region), Some(next)) => {
                    let changes = region_changes(view_state.agent_screen.as_ref(), next, region);
                    let base = view_state
                        .agent_screen
                        .as_ref()
                        .map(|(revision, _)| *revision);
                    (!changes.is_empty()).then_some((region, base, changes))
                }
                _ => None,
            };

            let full_frame = ServerFrame::View {
                sequence,
                data: &data,
                surface: surface.as_ref(),
                view_revision,
                base_view_revision: None,
                patch: None,
                resume_token: resume_token.as_deref(),
            };
            let sent = send_json(service.frame_meter(), io, &full_frame).await;
            if sent && screen_patches {
                view_state.screen = screen.map(|screen| (view_revision, screen.clone()));
            } else if !sent {
                view_state.reset();
            }
            if sent {
                view_state.note_view_sent(view_revision, screen);
                if let Some((region, base, changes)) = region_diff {
                    let diff_frame = ServerFrame::SemanticDiff {
                        sequence,
                        view_revision,
                        base_view_revision: base,
                        changes,
                        region: Some(region),
                    };
                    return send_json(service.frame_meter(), io, &diff_frame).await;
                }
            }
            sent
        }
        Ok(None) => {
            view_state.reset();
            true
        }
        Err(_) => false,
    }
}

/// Connection-local terminal/browser dimensions. This value is intentionally
/// absent from world events and snapshots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Viewport {
    pub(crate) width: usize,
    pub(crate) height: usize,
}

pub(crate) fn normalize_paste(text: &str) -> Result<Vec<u8>, &'static str> {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    if normalized.len() > 4096
        || normalized
            .chars()
            .any(|ch| ch.is_control() && ch != '\n' && ch != '\t')
    {
        return Err("paste must be at most 4096 UTF-8 bytes and contain no control characters");
    }
    Ok(normalized.into_bytes())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn screen(rows: &[&str]) -> crate::world_presentation::ScreenSurface {
        let height = rows.len();
        let width = rows
            .iter()
            .map(|row| row.chars().count())
            .max()
            .unwrap_or(0);
        let mut cells = Vec::new();
        for row in rows {
            let mut chars = row.chars();
            for _ in 0..width {
                cells.push(crate::world_presentation::ScreenCell {
                    symbol: chars.next().unwrap_or(' ').to_string(),
                    fg: 0,
                    bg: 0,
                    bold: false,
                    columns: 1,
                    token: None,
                });
            }
        }
        crate::world_presentation::ScreenSurface::from_cells(width, height, cells).unwrap()
    }

    fn region(x: u16, y: u16, width: u16, height: u16) -> ScreenRegion {
        ScreenRegion {
            x,
            y,
            width,
            height,
        }
    }

    /// Oracle: #6300 section 4.2 region-subscription scoping — a region
    /// diff names only the rows inside the region whose text changed.
    #[test]
    fn region_changes_are_scoped_to_the_region() {
        let before = screen(&["hello world", "second row!", "third row.."]);
        let after = screen(&["hello agent", "second row!", "third ROW.."]);
        let previous = Some((0, before));

        // A region covering only row 0 reports only row 0's change, even
        // though row 2 changed too.
        let changes = region_changes(previous.as_ref(), &after, region(0, 0, 11, 1));
        assert_eq!(changes.len(), 1, "{changes:?}");
        assert_eq!(changes[0]["kind"], "region_changed");
        assert_eq!(changes[0]["row"], 0);
        assert_eq!(changes[0]["text"], "hello agent");

        // A region covering only the changed columns of row 2 sees the
        // change there in its own window.
        let changes = region_changes(previous.as_ref(), &after, region(6, 2, 3, 1));
        assert_eq!(changes.len(), 1, "{changes:?}");
        assert_eq!(changes[0]["text"], "ROW");

        // A region over the unchanged row reports nothing.
        let changes = region_changes(previous.as_ref(), &after, region(0, 1, 11, 1));
        assert!(changes.is_empty(), "{changes:?}");

        // No comparable previous screen, or a resized screen, diffs to
        // nothing: the full view frame already carried the truth.
        assert!(region_changes(None, &after, region(0, 0, 11, 3)).is_empty());
        let resized = screen(&["hello agent"]);
        assert!(region_changes(previous.as_ref(), &resized, region(0, 0, 11, 1)).is_empty());
    }

    /// Oracle: #6300 section 4.2 subscription scoping for event diffs —
    /// an absent class list means no event diffs, an empty one means
    /// all classes, a named list means exactly those classes.
    #[test]
    fn event_diff_scoping_follows_the_subscription() {
        let mut state = ViewRevisionState::default();
        assert!(!state.wants_event_diff("chat_posted"), "no flag, no diffs");
        state.agent_profile = true;
        assert!(
            !state.wants_event_diff("chat_posted"),
            "no subscription yet"
        );
        state.subscription = Some(ScreenSubscriptionState {
            region: Some(region(0, 0, 4, 1)),
            event_classes: None,
        });
        assert!(!state.wants_event_diff("chat_posted"), "region-only");
        state.subscription = Some(ScreenSubscriptionState {
            region: None,
            event_classes: Some(Vec::new()),
        });
        assert!(
            state.wants_event_diff("chat_posted"),
            "empty list means all"
        );
        state.subscription = Some(ScreenSubscriptionState {
            region: None,
            event_classes: Some(vec!["chat_posted".to_owned()]),
        });
        assert!(state.wants_event_diff("chat_posted"));
        assert!(!state.wants_event_diff("input_applied"));
    }
}
