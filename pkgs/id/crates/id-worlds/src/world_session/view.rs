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

#[derive(Default)]
pub(crate) struct ViewRevisionState {
    next_revision: u64,
    screen: Option<(u64, crate::world_presentation::ScreenSurface)>,
    /// The participant this connection joined as, bound into resume tokens.
    pub(crate) participant: u64,
    /// A resume token presented at join and not yet consumed by a view send.
    pub(crate) pending_resume: Option<String>,
}

impl ViewRevisionState {
    pub(crate) fn reset(&mut self) {
        self.screen = None;
    }
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
                        }
                        return sent;
                    }
                }
            }

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
            } else {
                view_state.reset();
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
