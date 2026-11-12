//! Transport-neutral world session driver.
//!
//! A *session* is one participant's connection to a [`WorldHandle`]. The
//! wire protocol is one JSON object per frame; this module owns that protocol
//! and nothing else. A transport (WebSocket, Iroh stream, an in-memory test
//! channel) only implements [`SessionIo`] to move whole text frames, so the
//! behavior cannot drift between transports.
//!
//! Client frames (`type` tag): `join {capability, after?}`,
//! `invite {admin_token, display_name}`, `attenuate {name, scopes}`, `chat {text}`, `input {data_hex}`.
//! The first frame must be `join` or `invite`. Server frames: `snapshot`,
//! `event`, `invite`, `error`.
//!
//! Authorization is the capability alone. A transport may know who the peer
//! is (an Iroh node ID); that never grants world authority.

use std::time::Duration;

use iroh_blobs::Hash;
use rand::RngExt as _;
use serde::Serialize;
use subtle::ConstantTimeEq as _;
use tokio::sync::broadcast;

use crate::directory_auth::Signed;
use crate::world::{
    Authority, CapabilityBounds, FileScope, Isolation, JoinCapability, Participant, SessionId,
    WorldHandle, WorldMetrics, WorldPolicy, WorldScopes,
};
#[cfg(feature = "sandbox")]
use crate::world_compile::CompileSpec;
use crate::world_hub::{WorldHub, WorldLease};
pub(crate) use id_core::json_unique::parse_json_without_duplicate_keys;

/// Bound the extra presentation latency while coalescing view-only redraws.
const VIEW_COALESCE_WINDOW: Duration = Duration::from_millis(5);

/// Re-exported so `crate::world_session::MAX_FRAME_BYTES` keeps working: the
/// limit now lives in [`crate::frames`] so the crate builds without the
/// `world` feature.
pub use crate::frames::MAX_FRAME_BYTES;

#[cfg(test)]
mod contract_tests;
#[cfg(test)]
mod tests;
/// Largest Wasm world module that may be installed (16 MiB).
mod types;

/// World-service constructors and module lifecycle (split from this module, #4717).
mod service;

pub use types::MAX_ADMIN_TOKEN_BYTES;
pub(crate) use types::{
    CAPS_CEILING_NOTICE, ClientFrame, MAX_CAPABILITY_BYTES,
    MAX_CONCURRENT_MODULE_UPLOADS, ServerFrame, invite_bounds,
};
pub use types::{
    INVITE_DEFAULT_SECS, INVITE_MAX_SECS, Inbound, InstallError, InviteError, InviteResponse,
    MAX_WORLD_MODULE_BYTES, MAX_WORLD_MODULE_CHUNK_BYTES, ModuleSignature, SessionConfig,
    SessionIo, WorldService, check_invite_bounds,
};

/// Canonical content identifier expected in an install-begin frame.
#[must_use]
pub fn module_hash(wasm: &[u8]) -> String {
    Hash::new(wasm).to_string()
}

/// Verify an upload against the configured publisher trust list. Signatures
/// cover a domain-separated BLAKE3 digest, not a transport-specific encoding.
fn verify_module_signature(
    trusted_keys: &[[u8; 32]],
    wasm: &[u8],
    signature: Option<&ModuleSignature>,
) -> Result<(), InstallError> {
    if trusted_keys.is_empty() {
        return Ok(());
    }
    let signature = signature.ok_or(InstallError::UntrustedModule)?;
    let public_key = decode_hex(&signature.public_key).ok_or(InstallError::UntrustedModule)?;
    let public_key: [u8; 32] = public_key
        .try_into()
        .map_err(|_| InstallError::UntrustedModule)?;
    if !trusted_keys.contains(&public_key) {
        return Err(InstallError::UntrustedModule);
    }
    let signature_bytes = decode_hex(&signature.signature).ok_or(InstallError::UntrustedModule)?;
    let signature_bytes: [u8; 64] = signature_bytes
        .try_into()
        .map_err(|_| InstallError::UntrustedModule)?;
    let key = ed25519_dalek::VerifyingKey::from_bytes(&public_key)
        .map_err(|_| InstallError::UntrustedModule)?;
    let signature = ed25519_dalek::Signature::from_bytes(&signature_bytes);
    let digest = blake3::hash(wasm);
    let mut message = b"developing-today/id-world-module/v1\0".to_vec();
    message.extend_from_slice(digest.as_bytes());
    key.verify_strict(&message, &signature)
        .map_err(|_| InstallError::UntrustedModule)
}

/// Constant-time (for equal lengths) secret comparison.
#[must_use]
pub fn secret_eq(expected: &[u8], supplied: &[u8]) -> bool {
    expected.len() == supplied.len() && bool::from(expected.ct_eq(supplied))
}

/// Run one session to completion with default settings.
pub async fn run_session<I: SessionIo>(hub: &WorldHub, io: &mut I) {
    run_session_with(hub, io, SessionConfig::default()).await;
}

/// The admin secret a first frame carries, if it is an administrative one.
/// Most admin frames can create the world they name; history reads cannot.
pub(crate) fn frame_admin_token(frame: &ClientFrame) -> Option<&str> {
    match frame {
        ClientFrame::Invite { admin_token, .. }
        | ClientFrame::Compile { admin_token, .. }
        | ClientFrame::CreateWorld { admin_token }
        | ClientFrame::ListWorlds { admin_token } => Some(admin_token),
        // A joined builder's install_begin omits the token (design #835
        // phase 3); only a supplied one is an admin credential.
        ClientFrame::InstallBegin { admin_token, .. } => admin_token.as_deref(),
        ClientFrame::EventHistoryAdmin { admin_token } => Some(admin_token),
        ClientFrame::SetExecutionProfile { admin_token, .. } => Some(admin_token),
        _ => None,
    }
}

/// Run one session to completion.
///
/// The first frame names the world (`"world": "<name>"`, default world if
/// absent), and the session holds a lease on it until it ends.
pub async fn run_session_with<I: SessionIo>(hub: &WorldHub, io: &mut I, config: SessionConfig) {
    // Frames sent before a world lease exists belong to no world, so
    // their bytes go to a throwaway meter rather than being attributed
    // to a world that did not send them.
    let unattributed = WorldMetrics::default();
    let first = match tokio::time::timeout(config.join_timeout, io.recv()).await {
        Ok(Inbound::Text(text)) => text,
        Ok(Inbound::Oversized) => {
            let _ = send_error(&unattributed, io, "world frame is too large").await;
            return;
        }
        Ok(Inbound::Unsupported) => {
            let _ = send_error(&unattributed, io, "frames must be UTF-8 text").await;
            return;
        }
        Ok(Inbound::Closed) => return,
        Err(_) => {
            let _ = send_error(&unattributed, io, "timed out waiting for join").await;
            return;
        }
    };
    let Ok(mut value) = parse_json_without_duplicate_keys(&first) else {
        let _ = send_error(
            &unattributed,
            io,
            "first frame must be a valid join or invite request",
        )
        .await;
        return;
    };
    // `world` is a routing field, not part of any frame.
    let world = match value
        .as_object_mut()
        .and_then(|object| object.remove("world"))
    {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(name)) => Some(name),
        Some(_) => {
            let _ = send_error(&unattributed, io, "world must be a string").await;
            return;
        }
    };
    let Ok(frame) = serde_json::from_value::<ClientFrame>(value) else {
        let _ = send_error(
            &unattributed,
            io,
            "first frame must be a valid join or invite request",
        )
        .await;
        return;
    };
    // Hub-level frames are about the server, not one world.
    match &frame {
        ClientFrame::Directory {} => {
            // The public directory (#812): names, loaded state, default,
            // and presence counts only — no admin token, no identities,
            // and listing never opens a world.
            let worlds = hub.list();
            let _ = send_json(
                &unattributed,
                io,
                &ServerFrame::Worlds {
                    default: hub.default_world(),
                    worlds,
                },
            )
            .await;
            return;
        }
        ClientFrame::ListWorlds { admin_token } => {
            if admin_token.len() > MAX_ADMIN_TOKEN_BYTES || !hub.admin_ok(admin_token) {
                let _ = send_error(&unattributed, io, "admin denied").await;
                return;
            }
            io.mark_authorized();
            let worlds = hub.list();
            let _ = send_json(
                &unattributed,
                io,
                &ServerFrame::Worlds {
                    default: hub.default_world(),
                    worlds,
                },
            )
            .await;
            return;
        }
        ClientFrame::CreateWorld { admin_token } => {
            if admin_token.len() > MAX_ADMIN_TOKEN_BYTES || !hub.admin_ok(admin_token) {
                let _ = send_error(&unattributed, io, "admin denied").await;
                return;
            }
            io.mark_authorized();
            let name = world.as_deref().unwrap_or_else(|| hub.default_world());
            match hub.create(name).await {
                Ok(created) => {
                    let _ = send_json(
                        &unattributed,
                        io,
                        &ServerFrame::WorldCreated {
                            world: name,
                            created,
                        },
                    )
                    .await;
                }
                Err(error) => {
                    let _ = send_error(&unattributed, io, error.message()).await;
                }
            }
            return;
        }
        _ => {}
    }
    if let ClientFrame::EventHistoryAdmin { admin_token } = &frame
        && (admin_token.len() > MAX_ADMIN_TOKEN_BYTES || !hub.admin_ok(admin_token))
    {
        let _ = send_error(&unattributed, io, "admin denied").await;
        return;
    }
    let admin = frame_admin_token(&frame).filter(|token| token.len() <= MAX_ADMIN_TOKEN_BYTES);
    let create_token = if matches!(&frame, ClientFrame::EventHistoryAdmin { .. }) {
        None
    } else {
        admin
    };
    let lease = match hub.lease(world.as_deref(), create_token).await {
        Ok(lease) => lease,
        Err(error) => {
            let _ = send_error(&unattributed, io, error.message()).await;
            return;
        }
    };
    if admin.is_some_and(|token| lease.service().authorize_admin(Some(token)).is_ok()) {
        io.mark_authorized();
    }
    if let ClientFrame::SetIsolation {
        isolation,
        admin_token,
        signed,
    } = &frame
    {
        match set_isolation_frame(
            hub,
            &lease,
            *isolation,
            admin_token.as_deref(),
            signed.as_ref(),
        )
        .await
        {
            Ok(isolation) => {
                io.mark_authorized();
                let _ = send_json(
                    lease.service().frame_meter(),
                    io,
                    &ServerFrame::Isolation {
                        world: lease.name(),
                        isolation,
                    },
                )
                .await;
            }
            Err(error) => {
                let _ = send_error(lease.service().frame_meter(), io, &format!("{error:#}")).await;
            }
        }
        return;
    }
    if let Some(result) = policy_frame(hub, &lease, &frame).await {
        match result {
            Ok(policy) => {
                io.mark_authorized();
                let _ = send_json(
                    lease.service().frame_meter(),
                    io,
                    &ServerFrame::Policy {
                        world: lease.name(),
                        policy,
                    },
                )
                .await;
            }
            Err(error) => {
                let _ = send_error(lease.service().frame_meter(), io, &format!("{error:#}")).await;
            }
        }
        return;
    }
    run_world_session(lease.service(), io, frame).await;
}

/// Apply an isolation change for the admin token, or for a signed request
/// whose account moderates the world.
pub(crate) async fn set_isolation_frame(
    hub: &WorldHub,
    lease: &WorldLease,
    isolation: Isolation,
    admin_token: Option<&str>,
    signed: Option<&Signed>,
) -> anyhow::Result<Isolation> {
    let target = format!("/world/{}/isolation", lease.name());
    let authority =
        moderation_authority(hub, lease, admin_token, signed, &target, isolation.as_str())?;
    lease
        .service()
        .world()
        .set_isolation(authority, isolation)
        .await
}

/// Handle a cross-world write or file scope frame; `None` for any other frame.
pub(crate) async fn policy_frame(
    hub: &WorldHub,
    lease: &WorldLease,
    frame: &ClientFrame,
) -> Option<anyhow::Result<WorldPolicy>> {
    Some(match frame {
        ClientFrame::SetCrossWorldWrite {
            enabled,
            admin_token,
            signed,
        } => {
            set_cross_world_write_frame(
                hub,
                lease,
                *enabled,
                admin_token.as_deref(),
                signed.as_ref(),
            )
            .await
        }
        ClientFrame::ForceCrossWorldWrite { admin_token } => {
            force_cross_world_write_frame(lease, admin_token).await
        }
        ClientFrame::SetFileScope {
            scope,
            write_override,
            admin_token,
            signed,
        } => {
            set_file_scope_frame(
                hub,
                lease,
                *scope,
                *write_override,
                admin_token.as_deref(),
                signed.as_ref(),
            )
            .await
        }
        _ => return None,
    })
}

pub(crate) async fn set_cross_world_write_frame(
    hub: &WorldHub,
    lease: &WorldLease,
    enabled: bool,
    admin_token: Option<&str>,
    signed: Option<&Signed>,
) -> anyhow::Result<WorldPolicy> {
    let target = format!("/world/{}/cross-world-write", lease.name());
    let body = if enabled { "on" } else { "off" };
    let authority = moderation_authority(hub, lease, admin_token, signed, &target, body)?;
    lease
        .service()
        .world()
        .set_cross_world_write(authority, enabled)
        .await
}

pub(crate) async fn force_cross_world_write_frame(
    lease: &WorldLease,
    admin_token: &str,
) -> anyhow::Result<WorldPolicy> {
    anyhow::ensure!(is_admin(lease, Some(admin_token)), "admin denied");
    lease
        .service()
        .world()
        .force_cross_world_write(Authority::Admin)
        .await
}

pub(crate) async fn set_file_scope_frame(
    hub: &WorldHub,
    lease: &WorldLease,
    scope: FileScope,
    write_override: Option<bool>,
    admin_token: Option<&str>,
    signed: Option<&Signed>,
) -> anyhow::Result<WorldPolicy> {
    let target = format!("/world/{}/file-scope", lease.name());
    let body = match scope {
        FileScope::Confined => "confined",
        FileScope::Unrestricted => "unrestricted",
    };
    let authority = moderation_authority(hub, lease, admin_token, signed, &target, body)?;
    lease
        .service()
        .world()
        .set_file_scope(authority, scope, write_override)
        .await
}

pub(crate) fn is_admin(lease: &WorldLease, admin_token: Option<&str>) -> bool {
    admin_token
        .filter(|token| token.len() <= MAX_ADMIN_TOKEN_BYTES)
        .is_some_and(|token| lease.service().authorize_admin(Some(token)).is_ok())
}

/// The admin when the token matches; otherwise a signed request's account,
/// which the world's moderator check then judges.
pub(crate) fn moderation_authority(
    hub: &WorldHub,
    lease: &WorldLease,
    admin_token: Option<&str>,
    signed: Option<&Signed>,
    target: &str,
    body: &str,
) -> anyhow::Result<Authority> {
    if is_admin(lease, admin_token) {
        return Ok(Authority::Admin);
    }
    let Some(signed) = signed else {
        anyhow::bail!("admin denied");
    };
    let key = hub.verify_signed(signed, "PUT", target, body.as_bytes())?;
    Ok(Authority::Signed(key))
}

/// Serve one session against its resolved world, starting from its first frame.
async fn run_world_session<I: SessionIo>(service: &WorldService, io: &mut I, frame: ClientFrame) {
    match frame {
        ClientFrame::Join {
            capability,
            after,
            screen_patches,
        } => {
            if capability.len() > MAX_CAPABILITY_BYTES {
                let _ = send_error(service.frame_meter(), io, "invalid join capability").await;
                return;
            }
            joined(
                service,
                io,
                JoinCapability::from_wire(capability),
                after,
                screen_patches,
            )
            .await;
        }
        ClientFrame::Invite {
            admin_token,
            display_name,
            expires_in_secs,
            uses,
            role,
        } => match role.as_deref().map(WorldScopes::from_role) {
            Some(None) => {
                let _ = send_error(service.frame_meter(),
                    io,
                    "unknown invite role; use spectator, observer, player, guest, delegator, builder, or moderator",
                )
                .await;
            }
            role_scopes => match service
                .invite_scoped(
                    Some(&admin_token),
                    display_name,
                    expires_in_secs,
                    uses,
                    role_scopes.flatten().unwrap_or(WorldScopes::GUEST),
                )
                .await
            {
                Ok(invite) => {
                    let _ =
                        send_json(service.frame_meter(), io, &ServerFrame::Invite(&invite)).await;
                }
                Err(InviteError::InvalidBounds(message)) => {
                    let _ = send_error(service.frame_meter(), io, message).await;
                }
                Err(InviteError::Disabled) => {
                    let _ = send_error(service.frame_meter(), io, "invites are not enabled").await;
                }
                Err(InviteError::Unauthorized) => {
                    let _ = send_error(service.frame_meter(), io, "invite denied").await;
                }
                Err(InviteError::Unavailable) => {
                    let _ = send_error(
                        service.frame_meter(),
                        io,
                        "world cannot issue another invite",
                    )
                    .await;
                }
            },
        },
        // Query frames keep the session open for further queries: a client
        // can page through records, and may still join later on.
        ClientFrame::Info { .. }
        | ClientFrame::Metrics { .. }
        | ClientFrame::Records { .. }
        | ClientFrame::RecordsTicket { .. }
        | ClientFrame::DownloadModule { .. } => {
            query_session(service, io, frame).await;
        }
        ClientFrame::InstallBegin {
            admin_token,
            total_bytes,
            module_hash,
            signature,
            seed,
        } => {
            // The pre-join path is admin-only (design #835 phase 3):
            // joined builder sessions are already past this point, so
            // an omitted token here names no authority at all.
            let Some(admin_token) = admin_token else {
                let _ = send_error(service.frame_meter(), io, "module install denied").await;
                return;
            };
            if service.source_only() {
                if service.authorize_admin(Some(&admin_token)).is_err() {
                    let _ = send_error(service.frame_meter(), io, "module install denied").await;
                    return;
                }
                let _ = send_error(
                    service.frame_meter(),
                    io,
                    "raw module uploads are disabled by source-only policy",
                )
                .await;
                return;
            }
            if let Err(message) = install_module_session(
                service,
                io,
                &admin_token,
                total_bytes,
                module_hash.as_deref(),
                signature.as_ref(),
                seed,
            )
            .await
            {
                let _ = send_error(service.frame_meter(), io, message).await;
            }
        }
        ClientFrame::Caps { capability } => {
            let token = JoinCapability::from_wire(capability);
            if service.world().participant_id(token).await.is_err() {
                let _ = send_error(service.frame_meter(), io, "join denied").await;
            } else {
                io.mark_authorized();
                match service.world().caps_report().await {
                    Ok(report) => {
                        let _ = send_json(service.frame_meter(), io, &ServerFrame::Caps { report })
                            .await;
                    }
                    Err(_) => {
                        let _ =
                            send_error(service.frame_meter(), io, "capability report unavailable")
                                .await;
                    }
                }
            }
        }
        ClientFrame::UpdateCaps {
            admin_token,
            grant,
            revoke,
        } => match service.authorize_admin(Some(&admin_token)) {
            Ok(()) => match service.world().update_caps(grant, revoke).await {
                Ok(report) => {
                    let _ =
                        send_json(service.frame_meter(), io, &ServerFrame::Caps { report }).await;
                }
                Err(error) => {
                    let _ = send_error(service.frame_meter(), io, &format!("{error:#}")).await;
                }
            },
            Err(_) => {
                let _ = send_error(service.frame_meter(), io, "admin denied").await;
            }
        },
        ClientFrame::SetCapsCeiling {
            admin_token,
            ceiling,
        } => match service.set_caps_ceiling(Some(&admin_token), ceiling).await {
            Ok(report) => {
                let _ = send_json(
                    service.frame_meter(),
                    io,
                    &ServerFrame::CapsCeiling {
                        report,
                        message: CAPS_CEILING_NOTICE,
                    },
                )
                .await;
            }
            Err(error) => {
                let _ = send_error(service.frame_meter(), io, &format!("{error:#}")).await;
            }
        },
        ClientFrame::SetExecutionProfile {
            admin_token,
            profile,
        } => match service
            .set_execution_profile(Some(&admin_token), profile)
            .await
        {
            Ok(update) => {
                let _ = send_json(service.frame_meter(),
                    io,
                    &ServerFrame::ExecutionProfile {
                        update,
                        message: "profile saved; it applies on the next program install or process restart, not to the running instance",
                    },
                )
                .await;
            }
            Err(error) => {
                let _ = send_error(service.frame_meter(), io, &format!("{error:#}")).await;
            }
        },
        ClientFrame::SetSchedule {
            admin_token,
            id,
            at_ms,
        } => match service.set_schedule(Some(&admin_token), id, at_ms).await {
            Ok(schedules) => {
                let _ = send_json(
                    service.frame_meter(),
                    io,
                    &ServerFrame::Schedules { schedules },
                )
                .await;
            }
            Err(error) => {
                let _ = send_error(service.frame_meter(), io, &format!("{error:#}")).await;
            }
        },
        ClientFrame::CancelSchedule { admin_token, id } => {
            match service.cancel_schedule(Some(&admin_token), id).await {
                Ok(schedules) => {
                    let _ = send_json(
                        service.frame_meter(),
                        io,
                        &ServerFrame::Schedules { schedules },
                    )
                    .await;
                }
                Err(error) => {
                    let _ = send_error(service.frame_meter(), io, &format!("{error:#}")).await;
                }
            }
        }
        ClientFrame::DownloadModuleAdmin {
            admin_token,
            module_hash,
        } => {
            if let Err(message) =
                download_module_admin_session(service, io, &admin_token, &module_hash).await
            {
                let _ = send_error(service.frame_meter(), io, message).await;
            }
        }
        ClientFrame::EventHistoryAdmin { admin_token } => {
            if let Err(message) = event_history_admin_session(service, io, &admin_token).await {
                let _ = send_error(service.frame_meter(), io, message).await;
            }
        }
        ClientFrame::WorldState {
            admin_token,
            capability,
        } => {
            let admin_ok = admin_token
                .as_deref()
                .is_some_and(|token| service.authorize_admin(Some(token)).is_ok());
            let cap_ok = if admin_ok {
                true
            } else if let Some(cap) = capability.as_deref() {
                let token = JoinCapability::from_wire(cap.to_string());
                service.world().participant_id(token).await.is_ok()
            } else {
                false
            };
            if !admin_ok && !cap_ok {
                let _ = send_error(service.frame_meter(), io, "access denied").await;
            } else {
                match service.world().manifest_state().await {
                    Ok((seed, schedules)) => {
                        let _ = send_json(
                            service.frame_meter(),
                            io,
                            &ServerFrame::WorldState {
                                seed: admin_ok.then_some(seed),
                                schedules,
                            },
                        )
                        .await;
                    }
                    Err(error) => {
                        let _ = send_error(service.frame_meter(), io, &format!("{error:#}")).await;
                    }
                }
            }
        }
        ClientFrame::ExecutionProfileInfo {
            admin_token,
            capability,
        } => {
            let auth_ok = if let Some(admin) = admin_token.as_deref() {
                service.authorize_admin(Some(admin)).is_ok()
            } else if let Some(cap) = capability.as_deref() {
                let token = JoinCapability::from_wire(cap.to_string());
                service.world().participant_id(token).await.is_ok()
            } else {
                false
            };
            if !auth_ok {
                let _ = send_error(service.frame_meter(), io, "access denied").await;
            } else {
                match service.world().execution_profile().await {
                    Ok(profile) => {
                        let _ = send_json(
                            service.frame_meter(),
                            io,
                            &ServerFrame::ExecutionProfile {
                                update: crate::world::ExecutionProfileUpdate {
                                    requested: profile,
                                    applies_on_next_program_install_or_process_restart: false,
                                },
                                message: "current persisted execution profile",
                            },
                        )
                        .await;
                    }
                    Err(error) => {
                        let _ = send_error(service.frame_meter(), io, &format!("{error:#}")).await;
                    }
                }
            }
        }
        #[cfg(feature = "sandbox")]
        ClientFrame::Compile {
            admin_token,
            files,
            seed,
            native,
        } => {
            let seed = seed.unwrap_or_else(|| rand::rng().random::<u64>());
            let spec = CompileSpec {
                files: files
                    .into_iter()
                    .map(|file| (file.name, file.content))
                    .collect(),
                seed,
                native,
            };
            match service.compile_and_install(Some(&admin_token), &spec).await {
                Ok((module_hash, sequence, diagnostics)) => {
                    let _ = send_json(
                        service.frame_meter(),
                        io,
                        &ServerFrame::Compiled {
                            module_hash,
                            sequence,
                            diagnostics,
                        },
                    )
                    .await;
                }
                Err(message) => {
                    let _ = send_error(service.frame_meter(), io, &message).await;
                }
            }
        }
        #[cfg(not(feature = "sandbox"))]
        ClientFrame::Compile { .. } => {
            let _ = send_error(
                service.frame_meter(),
                io,
                "compilation is not enabled on this server",
            )
            .await;
        }
        ClientFrame::InstallChunk { .. }
        | ClientFrame::InstallEnd
        | ClientFrame::Directory { .. }
        | ClientFrame::ListWorlds { .. }
        | ClientFrame::CreateWorld { .. }
        | ClientFrame::SetIsolation { .. }
        | ClientFrame::SetCrossWorldWrite { .. }
        | ClientFrame::ForceCrossWorldWrite { .. }
        | ClientFrame::SetFileScope { .. }
        | ClientFrame::Chat { .. }
        | ClientFrame::Attenuate { .. }
        | ClientFrame::DelegateToFriend { .. }
        | ClientFrame::Input { .. }
        | ClientFrame::Resize { .. }
        | ClientFrame::Revoke { .. }
        | ClientFrame::Paste { .. } => {
            let _ = send_error(
                service.frame_meter(),
                io,
                "first frame must be join, invite or install_begin",
            )
            .await;
        }
    }
}

/// Serve query frames (`info`, `caps`, `execution_profile`, `metrics`,
/// `records`, `records_ticket`, `download_module`) until the peer closes,
/// denies, or sends a frame that
/// ends the session. A `join` frame switches to the joined session.
async fn query_session<I: SessionIo>(service: &WorldService, io: &mut I, first: ClientFrame) {
    let mut next = Some(first);
    loop {
        let frame = match next.take() {
            Some(frame) => frame,
            None => match io.recv().await {
                Inbound::Text(text) => {
                    if let Ok(frame) = parse_json_without_duplicate_keys(&text)
                        .and_then(|value| serde_json::from_value::<ClientFrame>(value))
                    {
                        frame
                    } else {
                        let _ = send_error(service.frame_meter(), io, "invalid world frame").await;
                        return;
                    }
                }
                Inbound::Oversized => {
                    let _ = send_error(service.frame_meter(), io, "world frame is too large").await;
                    return;
                }
                Inbound::Unsupported => {
                    let _ =
                        send_error(service.frame_meter(), io, "frames must be UTF-8 text").await;
                    return;
                }
                Inbound::Closed => return,
            },
        };
        match frame {
            ClientFrame::Join {
                capability,
                after,
                screen_patches,
            } => {
                if capability.len() > MAX_CAPABILITY_BYTES {
                    let _ = send_error(service.frame_meter(), io, "invalid join capability").await;
                    return;
                }
                joined(
                    service,
                    io,
                    JoinCapability::from_wire(capability),
                    after,
                    screen_patches,
                )
                .await;
                return;
            }
            ClientFrame::Info { capability } => {
                let token = JoinCapability::from_wire(capability);
                if let Ok(snapshot) = service.world().snapshot(token).await {
                    io.mark_authorized();
                    if !send_json(
                        service.frame_meter(),
                        io,
                        &ServerFrame::ModuleInfo {
                            world_id: snapshot.world_id,
                            active_module_hash: snapshot.active_module_hash,
                            active_module_provenance: snapshot.active_module_provenance,
                            current_sequence: snapshot.current_sequence,
                        },
                    )
                    .await
                    {
                        return;
                    }
                } else {
                    let _ = send_error(service.frame_meter(), io, "join denied").await;
                    return;
                }
            }
            ClientFrame::Metrics { capability } => {
                // Authorized exactly like `info`: holding a capability
                // that snapshots the world is enough to read its metrics.
                let token = JoinCapability::from_wire(capability);
                match service.world().snapshot(token).await {
                    Ok(_) => {
                        io.mark_authorized();
                        match service.world().metrics().await {
                            Ok(metrics) => {
                                if !send_json(
                                    service.frame_meter(),
                                    io,
                                    &ServerFrame::Metrics { metrics },
                                )
                                .await
                                {
                                    return;
                                }
                            }
                            Err(error) => {
                                let _ =
                                    send_error(service.frame_meter(), io, &format!("{error:#}"))
                                        .await;
                                return;
                            }
                        }
                    }
                    Err(_) => {
                        let _ = send_error(service.frame_meter(), io, "join denied").await;
                        return;
                    }
                }
            }
            ClientFrame::Records {
                capability,
                prefix,
                after,
            } => {
                let token = JoinCapability::from_wire(capability);
                if service.world().participant_id(token.clone()).await.is_ok() {
                    io.mark_authorized();
                }
                if let Err(message) = records_session(
                    service.frame_meter(),
                    service.world(),
                    io,
                    token,
                    prefix.as_deref(),
                    after.as_deref(),
                )
                .await
                {
                    let _ = send_error(service.frame_meter(), io, message).await;
                    return;
                }
            }
            ClientFrame::RecordsTicket { capability } => {
                let token = JoinCapability::from_wire(capability);
                if service.world().participant_id(token).await.is_err() {
                    let _ = send_error(service.frame_meter(), io, "join denied").await;
                    return;
                }
                io.mark_authorized();
                match records_ticket_session(service, io).await {
                    Ok(()) => {}
                    Err(message) => {
                        let _ = send_error(service.frame_meter(), io, message).await;
                        return;
                    }
                }
            }
            ClientFrame::DownloadModule {
                capability,
                module_hash,
            } => {
                let token = JoinCapability::from_wire(capability);
                if service.world().snapshot(token).await.is_err() {
                    let _ = send_error(service.frame_meter(), io, "join denied").await;
                    return;
                }
                io.mark_authorized();
                if let Err(message) = download_module_session(service, io, &module_hash).await {
                    let _ = send_error(service.frame_meter(), io, message).await;
                    return;
                }
            }
            ClientFrame::Caps { capability } => {
                // Read-only like `info`/`metrics` (#6802): the stats
                // verb chains info -> caps -> profile -> metrics over
                // one connection, so the query session serves caps
                // with the first-frame authorization (a participant
                // capability) instead of refusing the frame.
                let token = JoinCapability::from_wire(capability);
                if service.world().participant_id(token).await.is_err() {
                    let _ = send_error(service.frame_meter(), io, "join denied").await;
                    return;
                }
                io.mark_authorized();
                match service.world().caps_report().await {
                    Ok(report) => {
                        if !send_json(service.frame_meter(), io, &ServerFrame::Caps { report })
                            .await
                        {
                            return;
                        }
                    }
                    Err(_) => {
                        let _ =
                            send_error(service.frame_meter(), io, "capability report unavailable")
                                .await;
                        return;
                    }
                }
            }
            ClientFrame::ExecutionProfileInfo {
                admin_token,
                capability,
            } => {
                // Same authorization as the first-frame handler: an
                // admin token, or a participant capability (the stats
                // verb sends the latter). (#6802)
                let auth_ok = if let Some(admin) = admin_token.as_deref() {
                    service.authorize_admin(Some(admin)).is_ok()
                } else if let Some(cap) = capability.as_deref() {
                    let token = JoinCapability::from_wire(cap.to_string());
                    service.world().participant_id(token).await.is_ok()
                } else {
                    false
                };
                if !auth_ok {
                    let _ = send_error(service.frame_meter(), io, "access denied").await;
                    return;
                }
                match service.world().execution_profile().await {
                    Ok(profile) => {
                        if !send_json(
                            service.frame_meter(),
                            io,
                            &ServerFrame::ExecutionProfile {
                                update: crate::world::ExecutionProfileUpdate {
                                    requested: profile,
                                    applies_on_next_program_install_or_process_restart: false,
                                },
                                message: "current persisted execution profile",
                            },
                        )
                        .await
                        {
                            return;
                        }
                    }
                    Err(error) => {
                        let _ = send_error(service.frame_meter(), io, &format!("{error:#}")).await;
                        return;
                    }
                }
            }
            _ => {
                let _ = send_error(service.frame_meter(), io, "expected a query frame").await;
                return;
            }
        }
    }
}

/// Answer a `records_ticket` query.
async fn records_ticket_session<I: SessionIo>(
    service: &WorldService,
    io: &mut I,
) -> Result<(), &'static str> {
    match service.records_store() {
        Some(store) => match store.read_ticket().await {
            Ok(ticket) => {
                if send_json(
                    service.frame_meter(),
                    io,
                    &ServerFrame::RecordsTicket { ticket },
                )
                .await
                {
                    Ok(())
                } else {
                    Err("connection closed")
                }
            }
            Err(_) => Err("records ticket unavailable"),
        },
        None => Err("world keeps no structured records"),
    }
}

/// Compatibility alias for the provider manifest's shared records-page bound.
pub const MAX_RECORDS_PAGE_BYTES: usize = crate::world_caps::MAX_RECORDS_PAGE_BYTES;

async fn records_session<I: SessionIo>(
    meter: &WorldMetrics,
    world: &WorldHandle,
    io: &mut I,
    token: JoinCapability,
    prefix: Option<&str>,
    after: Option<&str>,
) -> Result<(), &'static str> {
    if world.participant_id(token).await.is_err() {
        return Err("join denied");
    }
    let (sequence, records) = world.records();
    let mut page = crate::world::Records::new();
    let mut used = 0usize;
    let mut last_included: Option<String> = None;
    let mut more = false;
    for (key, value) in records.iter() {
        if let Some(prefix) = prefix
            && !key.starts_with(prefix)
        {
            continue;
        }
        if let Some(after) = after
            && key.as_str() <= after
        {
            continue;
        }
        let size = key.len() + serde_json::to_vec(value).map_or(0, |v| v.len()) + 8;
        if used + size > MAX_RECORDS_PAGE_BYTES {
            // `next_after` is exclusive on the client, so it must be the
            // last key *included* here; anything else would skip a record.
            if page.is_empty() {
                return Err("a record is too large to serve in one page");
            }
            more = true;
            break;
        }
        used += size;
        last_included = Some(key.clone());
        page.insert(key.clone(), value.clone());
    }
    let next_after = more.then_some(last_included).flatten();
    if !send_json(
        meter,
        io,
        &ServerFrame::Records {
            sequence,
            records: page,
            next_after,
        },
    )
    .await
    {
        return Err("connection closed");
    }
    Ok(())
}

async fn event_history_admin_session<I: SessionIo>(
    service: &WorldService,
    io: &mut I,
    admin_token: &str,
) -> Result<(), &'static str> {
    service
        .authorize_admin(Some(admin_token))
        .map_err(|_| "admin denied")?;
    let history = service
        .world()
        .admin_event_history()
        .await
        .map_err(|_| "admin history unavailable")?;
    if !send_json(
        service.frame_meter(),
        io,
        &ServerFrame::EventHistoryBegin {
            world_id: history.world_id,
            module_hash: history.module_hash,
            current_sequence: history.current_sequence,
            complete: history.complete,
            event_count: history.events.len(),
        },
    )
    .await
    {
        return Err("connection closed");
    }
    for event in &history.events {
        if !send_json(
            service.frame_meter(),
            io,
            &ServerFrame::EventHistoryEvent { event },
        )
        .await
        {
            return Err("event history exceeds the frame limit");
        }
    }

    let mut after: Option<String> = None;
    loop {
        let mut page = crate::world::Records::new();
        let mut used = 0usize;
        let mut last_included: Option<String> = None;
        let mut more = false;
        for (key, value) in &history.records {
            if after.as_deref().is_some_and(|after| key.as_str() <= after) {
                continue;
            }
            let size = key.len() + serde_json::to_vec(value).map_or(0, |bytes| bytes.len()) + 8;
            if used + size > MAX_RECORDS_PAGE_BYTES {
                if page.is_empty() {
                    return Err("a record is too large to serve in one page");
                }
                more = true;
                break;
            }
            used += size;
            last_included = Some(key.clone());
            page.insert(key.clone(), value.clone());
        }
        let next_after = more.then_some(last_included).flatten();
        if !send_json(
            service.frame_meter(),
            io,
            &ServerFrame::Records {
                sequence: history.current_sequence,
                records: page,
                next_after: next_after.clone(),
            },
        )
        .await
        {
            return Err("connection closed");
        }
        match next_after {
            Some(next) => after = Some(next),
            None => return Ok(()),
        }
    }
}

async fn download_module_session<I: SessionIo>(
    service: &WorldService,
    io: &mut I,
    requested_hash: &str,
) -> Result<(), &'static str> {
    let Some(active_hash) = service.active_module_hash().await else {
        return Err("world has no installed module");
    };
    if requested_hash != active_hash {
        return Err("requested module is not the active world module");
    }
    let bytes = service
        .module_bytes(&active_hash)
        .await
        .map_err(|_| "module bytes are unavailable")?;
    send_module_bytes(service, io, active_hash, &bytes).await
}

/// The admin module download (#807): any module the host holds in its blob
/// store, by hash, active or not - so `id world apply` can install the
/// module a manifest names when another module is currently active.
async fn download_module_admin_session<I: SessionIo>(
    service: &WorldService,
    io: &mut I,
    admin_token: &str,
    requested_hash: &str,
) -> Result<(), &'static str> {
    if admin_token.len() > MAX_ADMIN_TOKEN_BYTES
        || service.authorize_admin(Some(admin_token)).is_err()
    {
        return Err("admin denied");
    }
    // The admin credential validated: this session is authorized, so its
    // outbound chunks charge the authorized bucket, not the pre-auth one.
    io.mark_authorized();
    let bytes = service
        .stored_module_bytes(requested_hash)
        .await
        .map_err(|_| "the host does not hold a module with that hash")?;
    send_module_bytes(service, io, requested_hash.to_owned(), &bytes).await
}

/// Stream one module's bytes as a begin/chunk/end transfer naming `module_hash`.
async fn send_module_bytes<I: SessionIo>(
    service: &WorldService,
    io: &mut I,
    module_hash: String,
    bytes: &[u8],
) -> Result<(), &'static str> {
    if !send_json(
        service.frame_meter(),
        io,
        &ServerFrame::ModuleBegin {
            module_hash: module_hash.clone(),
            total_bytes: bytes.len(),
        },
    )
    .await
    {
        return Err("connection closed");
    }
    for (index, chunk) in bytes.chunks(MAX_WORLD_MODULE_CHUNK_BYTES).enumerate() {
        if !send_json(
            service.frame_meter(),
            io,
            &ServerFrame::ModuleChunk {
                offset: index * MAX_WORLD_MODULE_CHUNK_BYTES,
                data_hex: encode_hex(chunk),
            },
        )
        .await
        {
            return Err("connection closed");
        }
    }
    if !send_json(
        service.frame_meter(),
        io,
        &ServerFrame::ModuleEnd { module_hash },
    )
    .await
    {
        return Err("connection closed");
    }
    Ok(())
}

/// The wire code carried by error frames that refuse a joined
/// session's role/capability action (design #835 §5): one stable,
/// non-oracular code for invalid, expired, revoked, and insufficient
/// capabilities alike.
const ACTOR_DENIED: &str = "actor_denied";

/// Receive one upload's chunks and its terminating `install_end`,
/// exactly as the install paths have always framed them. Shared by
/// the pre-join admin install and the joined builder install so the
/// upload safeguards (offsets, chunk bounds, strict frame parsing)
/// cannot drift apart.
#[cfg(feature = "sandbox")]
async fn receive_module_upload<I: SessionIo>(
    io: &mut I,
    total_bytes: usize,
) -> Result<Vec<u8>, &'static str> {
    let mut bytes = Vec::with_capacity(total_bytes);
    while bytes.len() < total_bytes {
        let incoming = tokio::time::timeout(Duration::from_secs(30), io.recv())
            .await
            .map_err(|_| "timed out receiving module")?;
        let Inbound::Text(text) = incoming else {
            return Err("invalid module chunk frame");
        };
        let Ok(ClientFrame::InstallChunk { offset, data_hex }) =
            parse_json_without_duplicate_keys(&text)
                .and_then(|value| serde_json::from_value::<ClientFrame>(value))
        else {
            return Err("expected module chunk frame");
        };
        if offset != bytes.len() || data_hex.len() > MAX_WORLD_MODULE_CHUNK_BYTES * 2 {
            return Err("invalid module chunk offset or size");
        }
        let Some(chunk) = decode_hex(&data_hex) else {
            return Err("module chunk must be hexadecimal");
        };
        if chunk.is_empty() || chunk.len() > total_bytes.saturating_sub(bytes.len()) {
            return Err("invalid module chunk length");
        }
        bytes.extend_from_slice(&chunk);
    }

    let incoming = tokio::time::timeout(Duration::from_secs(30), io.recv())
        .await
        .map_err(|_| "timed out waiting for install completion")?;
    if !matches!(incoming, Inbound::Text(ref text) if matches!(parse_json_without_duplicate_keys(text).and_then(|value| serde_json::from_value::<ClientFrame>(value)), Ok(ClientFrame::InstallEnd)))
    {
        return Err("expected install_end frame");
    }
    Ok(bytes)
}

/// A joined builder session's module install (design #835 phase 3):
/// the same upload sequence and safeguards as the pre-join admin
/// install, but authorized by the session's live capability — an
/// early `BUILDER` check before any bytes are received, and the
/// actor's own re-check at the effect boundary inside
/// [`WorldService::install_wasm_authorized`]. Role denials carry the
/// `actor_denied` wire code. Returns whether the session continues.
async fn install_module_session_joined<I: SessionIo>(
    service: &WorldService,
    io: &mut I,
    token: &JoinCapability,
    total_bytes: usize,
    claimed_hash: Option<&str>,
    signature: Option<&ModuleSignature>,
    seed: u64,
) -> bool {
    if service
        .world()
        .authorize(token.clone(), WorldScopes::BUILDER)
        .await
        .is_err()
    {
        return send_error_with_code(
            service.frame_meter(),
            io,
            Some(ACTOR_DENIED),
            "module install denied",
        )
        .await;
    }
    if total_bytes == 0
        || total_bytes > MAX_WORLD_MODULE_BYTES
        || claimed_hash.is_some_and(|hash| {
            hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
    {
        return send_error(
            service.frame_meter(),
            io,
            "invalid module size or content hash",
        )
        .await;
    }
    let upload_permit = match service.try_reserve_module_upload() {
        Ok(permit) => permit,
        Err(_) => {
            return send_error(
                service.frame_meter(),
                io,
                "world is busy installing another module",
            )
            .await;
        }
    };
    #[cfg(not(feature = "sandbox"))]
    {
        let _ = (token, upload_permit, signature, seed);
        return send_error(
            service.frame_meter(),
            io,
            "module installation requires the sandbox feature",
        )
        .await;
    }

    #[cfg(feature = "sandbox")]
    {
        if !send_json(
            service.frame_meter(),
            io,
            &ServerFrame::UploadReady {
                chunk_bytes: MAX_WORLD_MODULE_CHUNK_BYTES,
            },
        )
        .await
        {
            return false;
        }
        let bytes = match receive_module_upload(io, total_bytes).await {
            Ok(bytes) => bytes,
            Err(message) => return send_error(service.frame_meter(), io, message).await,
        };
        let actual_hash = module_hash(&bytes);
        if claimed_hash.is_some_and(|claimed| actual_hash != claimed.to_ascii_lowercase()) {
            return send_error(service.frame_meter(), io, "module content hash mismatch").await;
        }
        let installed = service
            .install_wasm_authorized(
                token.clone(),
                bytes,
                seed,
                &actual_hash,
                upload_permit,
                signature,
            )
            .await;
        match installed {
            Ok((module_hash, sequence)) => send_json(
                service.frame_meter(),
                io,
                &ServerFrame::ModuleInstalled {
                    module_hash: &module_hash,
                    sequence,
                },
            )
            .await,
            Err(InstallError::ActorDenied) => send_error_with_code(
                service.frame_meter(),
                io,
                Some(ACTOR_DENIED),
                "module install denied",
            )
            .await,
            Err(error) => {
                let message = match error {
                    InstallError::Disabled => "module installation is not enabled on this host",
                    InstallError::Unauthorized | InstallError::ActorDenied => {
                        "module install denied"
                    }
                    InstallError::InvalidUpload => "invalid module upload",
                    InstallError::RawModuleDisabled => {
                        "raw module uploads are disabled by source-only policy"
                    }
                    InstallError::InvalidModule => {
                        "Wasm module failed validation or initialization"
                    }
                    InstallError::UntrustedModule => "module signature is not trusted",
                    InstallError::Unavailable => "world could not activate the module",
                };
                send_error(service.frame_meter(), io, message).await
            }
        }
    }
}

async fn install_module_session<I: SessionIo>(
    service: &WorldService,
    io: &mut I,
    admin_token: &str,
    total_bytes: usize,
    claimed_hash: Option<&str>,
    signature: Option<&ModuleSignature>,
    seed: u64,
) -> Result<(), &'static str> {
    if admin_token.len() > MAX_ADMIN_TOKEN_BYTES
        || service.authorize_admin(Some(admin_token)).is_err()
    {
        return Err("module install denied");
    }
    if total_bytes == 0
        || total_bytes > MAX_WORLD_MODULE_BYTES
        || claimed_hash.is_some_and(|hash| {
            hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
    {
        return Err("invalid module size or content hash");
    }
    let upload_permit = service
        .try_reserve_module_upload()
        .map_err(|_| "world is busy installing another module")?;
    #[cfg(not(feature = "sandbox"))]
    {
        let _ = (io, seed, upload_permit, signature);
        return Err("module installation requires the sandbox feature");
    }

    #[cfg(feature = "sandbox")]
    {
        if !send_json(
            service.frame_meter(),
            io,
            &ServerFrame::UploadReady {
                chunk_bytes: MAX_WORLD_MODULE_CHUNK_BYTES,
            },
        )
        .await
        {
            return Err("connection closed");
        }
        let bytes = receive_module_upload(io, total_bytes).await?;
        let actual_hash = module_hash(&bytes);
        if claimed_hash.is_some_and(|claimed| actual_hash != claimed.to_ascii_lowercase()) {
            return Err("module content hash mismatch");
        }
        let (module_hash, sequence) = service
            .install_wasm_reserved(
                Some(admin_token),
                bytes,
                seed,
                &actual_hash,
                upload_permit,
                false,
                None,
                signature,
            )
            .await
            .map_err(|error| match error {
                InstallError::Disabled => "module installation is not enabled on this host",
                InstallError::Unauthorized | InstallError::ActorDenied => "module install denied",
                InstallError::InvalidUpload => "invalid module upload",
                InstallError::RawModuleDisabled => {
                    "raw module uploads are disabled by source-only policy"
                }
                InstallError::InvalidModule => "Wasm module failed validation or initialization",
                InstallError::UntrustedModule => "module signature is not trusted",
                InstallError::Unavailable => "world could not activate the module",
            })?;
        if !send_json(
            service.frame_meter(),
            io,
            &ServerFrame::ModuleInstalled {
                module_hash: &module_hash,
                sequence,
            },
        )
        .await
        {
            return Err("connection closed");
        }
        Ok(())
    }
}

async fn joined<I: SessionIo>(
    service: &WorldService,
    io: &mut I,
    token: JoinCapability,
    after: Option<u64>,
    screen_patches: bool,
) {
    let world = service.world();
    let session_id = service.next_session_id();
    // Subscribe before taking the snapshot so no event can fall in the gap;
    // events at or below the cursor are skipped when delivered.
    let mut events = world.subscribe();
    let mut revocations = world.subscribe_revocations();
    let Ok(participant_id) = world.participant_id(token.clone()).await else {
        let _ = send_error(service.frame_meter(), io, "join denied").await;
        return;
    };
    let Ok(snapshot) = world.snapshot(token.clone()).await else {
        let _ = send_error(service.frame_meter(), io, "join denied").await;
        return;
    };
    io.mark_authorized();
    let mut cursor = snapshot.current_sequence;
    let mut viewport = None;
    let mut view_state = ViewRevisionState::default();
    if let Some(after) = after {
        match world.events_after(token.clone(), after).await {
            Ok(page) if page.needs_snapshot => {
                let Ok(fresh) = world.snapshot(token.clone()).await else {
                    return;
                };
                cursor = fresh.current_sequence;
                if !send_json(
                    service.frame_meter(),
                    io,
                    &ServerFrame::Snapshot { snapshot: &fresh },
                )
                .await
                {
                    return;
                }
            }
            Ok(page) => {
                cursor = page.current_sequence;
                for event in &page.events {
                    if !send_json(service.frame_meter(), io, &ServerFrame::Event { event }).await {
                        return;
                    }
                    if !send_view(
                        service.frame_meter(),
                        world,
                        io,
                        &token,
                        event.sequence,
                        viewport,
                        &mut view_state,
                        screen_patches,
                    )
                    .await
                    {
                        return;
                    }
                }
            }
            Err(_) => {
                let _ = send_error(service.frame_meter(), io, "join denied").await;
                return;
            }
        }
    } else if !send_json(
        service.frame_meter(),
        io,
        &ServerFrame::Snapshot {
            snapshot: &snapshot,
        },
    )
    .await
    {
        return;
    }
    if !send_view(
        service.frame_meter(),
        world,
        io,
        &token,
        cursor,
        viewport,
        &mut view_state,
        screen_patches,
    )
    .await
    {
        return;
    }

    // Presence starts here and ends whenever this session does. Views are
    // re-sent whenever program state changes for another reason (host events
    // are not broadcast as events).
    world.presence(participant_id, true).await;
    let _guard = PresenceGuard {
        world: world.clone(),
        participant: participant_id,
    };
    let mut views = world.watch_views();
    let mut pending_view_deadline = None;
    let mut pending_view_sequence = cursor;

    loop {
        let view_deadline = pending_view_deadline;
        tokio::select! {
            () = async {
                match view_deadline {
                    Some(deadline) => tokio::time::sleep_until(deadline).await,
                    None => std::future::pending::<()>().await,
                }
            } => {
                pending_view_deadline = None;
                if !send_view(service.frame_meter(), world, io, &token, pending_view_sequence, viewport, &mut view_state, screen_patches).await {
                    break;
                }
            },
            inbound = io.recv() => match inbound {
                Inbound::Text(text) => {
                    if !handle_client_frame(
                        service,
                        io,
                        &token,
                        &text,
                        &mut viewport,
                        cursor,
                        session_id,
                        &mut view_state,
                        screen_patches,
                        &mut pending_view_deadline,
                    ).await {
                        break;
                    }
                }
                Inbound::Oversized => {
                    if !send_error(service.frame_meter(), io, "world frame is too large").await { break; }
                }
                Inbound::Unsupported => {
                    if !send_error(service.frame_meter(), io, "frames must be UTF-8 text").await { break; }
                }
                Inbound::Closed => break,
            },
            event = events.recv() => match event {
                Ok(event) => {
                    if let crate::world::WorldEventKind::ProgramInstalled { ref module_hash, .. } = event.kind {
                        if event.sequence > cursor {
                            pending_view_deadline = None;
                            let _ = views.borrow_and_update();
                            let Ok(snapshot) = world.snapshot(token.clone()).await else { break };
                            if !send_json(service.frame_meter(), io, &ServerFrame::Snapshot { snapshot: &snapshot }).await {
                                break;
                            }
                            cursor = event.sequence;
                            if !send_json(service.frame_meter(), io, &ServerFrame::ProgramChanged {
                                module_hash,
                                sequence: cursor,
                            }).await {
                                break;
                            }
                            view_state.reset();
                            if !send_view(service.frame_meter(), world, io, &token, cursor, viewport, &mut view_state, screen_patches).await {
                                break;
                            }
                        }
                        continue;
                    }
                    if event.sequence <= cursor {
                        continue;
                    }
                    // Projecting the event re-validates the capability on every
                    // delivery, which is what disconnects a revoked
                    // participant, and applies what this viewer may see now.
                    let Ok(projected) = world.project_event(token.clone(), event.clone()).await else {
                        break;
                    };
                    // A withheld event still moves the cursor past it.
                    cursor = event.sequence;
                    let Some(shown) = projected else {
                        continue;
                    };
                    if !send_json(service.frame_meter(), io, &ServerFrame::Event { event: &shown }).await {
                        break;
                    }
                    pending_view_sequence = cursor;
                    pending_view_deadline.get_or_insert_with(|| tokio::time::Instant::now() + VIEW_COALESCE_WINDOW);
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    pending_view_deadline = None;
                    let Ok(snapshot) = world.snapshot(token.clone()).await else { break };
                    let _ = views.borrow_and_update();
                    if !send_json(service.frame_meter(), io, &ServerFrame::Snapshot { snapshot: &snapshot }).await {
                        break;
                    }
                    cursor = snapshot.current_sequence;
                    view_state.reset();
                    if !send_view(service.frame_meter(), world, io, &token, cursor, viewport, &mut view_state, screen_patches).await {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
            changed = views.changed() => match changed {
                Ok(()) => {
                    let sequence = *views.borrow_and_update();
                    pending_view_sequence = sequence;
                    pending_view_deadline.get_or_insert_with(|| tokio::time::Instant::now() + VIEW_COALESCE_WINDOW);
                }
                Err(_) => break,
            },
            revoked = revocations.recv() => match revoked {
                Ok(revoked_id) if revoked_id == participant_id => {
                    let _ = send_error(service.frame_meter(), io, "world capability revoked").await;
                    break;
                }
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {},
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    }
}

/// Returns the participant's presence when the session ends, however it ends.
pub(crate) struct PresenceGuard {
    world: WorldHandle,
    participant: u64,
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
struct ViewRevisionState {
    next_revision: u64,
    screen: Option<(u64, crate::world_presentation::ScreenSurface)>,
}

impl ViewRevisionState {
    fn reset(&mut self) {
        self.screen = None;
    }
}

async fn send_view<I: SessionIo>(
    meter: &WorldMetrics,
    world: &WorldHandle,
    io: &mut I,
    token: &JoinCapability,
    sequence: u64,
    viewport: Option<Viewport>,
    view_state: &mut ViewRevisionState,
    screen_patches: bool,
) -> bool {
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
                return send_error(meter, io, "world presentation is too large").await;
            }
            let surface = match crate::world_presentation::parse_surface(&data) {
                Ok(Some(crate::world_presentation::WorldSurface::Screen(screen))) => {
                    let screen = viewport.map_or(screen.clone(), |viewport| {
                        screen.fit(viewport.width, viewport.height)
                    });
                    Some(crate::world_presentation::WorldSurface::Screen(screen))
                }
                Ok(surface) => surface,
                Err(_) => return send_error(meter, io, "invalid world screen presentation").await,
            };
            let view_revision = view_state.next_revision;
            view_state.next_revision = view_state.next_revision.saturating_add(1);
            let screen = match surface.as_ref() {
                Some(crate::world_presentation::WorldSurface::Screen(screen)) => Some(screen),
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
                    };
                    let full_frame = ServerFrame::View {
                        sequence,
                        data: &data,
                        surface: surface.as_ref(),
                        view_revision,
                        base_view_revision: None,
                        patch: None,
                    };
                    let delta_size = serde_json::to_vec(&delta_frame).map(|bytes| bytes.len());
                    let full_size = serde_json::to_vec(&full_frame).map(|bytes| bytes.len());
                    if matches!((delta_size, full_size), (Ok(delta), Ok(full)) if delta < full) {
                        let sent = send_json(meter, io, &delta_frame).await;
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
            };
            let sent = send_json(meter, io, &full_frame).await;
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
    width: usize,
    height: usize,
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

const USES_REFUSED: &str = "a delegation may not carry `uses`: a delegated grant has no use count, and the host's limit still applies";

/// Whether a delegation frame names `uses` anywhere in it. The field was
/// removed from delegations, and serde would ignore it, minting an unbounded
/// grant from a client that still sends it. Any depth is refused, so no
/// position hides the field.
pub(crate) fn delegation_names_uses(value: &serde_json::Value) -> bool {
    let delegating = matches!(
        value.get("type").and_then(serde_json::Value::as_str),
        Some("attenuate" | "delegate_to_friend")
    );
    delegating && names_uses(value)
}

pub(crate) fn names_uses(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(map) => map
            .iter()
            .any(|(key, inner)| key == "uses" || names_uses(inner)),
        serde_json::Value::Array(items) => items.iter().any(names_uses),
        _ => false,
    }
}

/// Handle one frame from a joined participant. Returns `false` to end the session.
async fn handle_client_frame<I: SessionIo>(
    service: &WorldService,
    io: &mut I,
    token: &JoinCapability,
    text: &str,
    viewport: &mut Option<Viewport>,
    sequence: u64,
    session_id: SessionId,
    view_state: &mut ViewRevisionState,
    screen_patches: bool,
    pending_view_deadline: &mut Option<tokio::time::Instant>,
) -> bool {
    let world = service.world();
    let Ok(value) = parse_json_without_duplicate_keys(text) else {
        return send_error(service.frame_meter(), io, "invalid world frame").await;
    };
    if delegation_names_uses(&value) {
        return send_error(service.frame_meter(), io, USES_REFUSED).await;
    }
    let Ok(frame) = serde_json::from_value::<ClientFrame>(value) else {
        return send_error(service.frame_meter(), io, "invalid world frame").await;
    };
    let result = match frame {
        ClientFrame::Attenuate {
            name,
            scopes,
            expires_at,
        } => {
            let Some(scopes) = WorldScopes::from_names(scopes.iter().map(String::as_str)) else {
                return send_error(service.frame_meter(), io, "unknown scope").await;
            };
            let bounds = CapabilityBounds {
                expires_at,
                uses: None,
            };
            let result = world.attenuate(token.clone(), scopes, bounds, name).await;
            return send_delegation(service.frame_meter(), io, result).await;
        }
        ClientFrame::DelegateToFriend {
            friend,
            name,
            scopes,
            expires_at,
        } => {
            let Some(scopes) = WorldScopes::from_names(scopes.iter().map(String::as_str)) else {
                return send_error(service.frame_meter(), io, "unknown scope").await;
            };
            let bounds = CapabilityBounds {
                expires_at,
                uses: None,
            };
            let result = world
                .delegate_to_friend(token.clone(), friend, scopes, bounds, name)
                .await;
            return send_delegation(service.frame_meter(), io, result).await;
        }
        ClientFrame::Chat { text } => world.chat(token.clone(), text).await,
        ClientFrame::Resize { width, height } => {
            if width == 0 || height == 0 || width > 500 || height > 200 {
                return send_error(
                    service.frame_meter(),
                    io,
                    "viewport dimensions must be width 1..=500 and height 1..=200",
                )
                .await;
            }
            // The clamp to the presentation maximum (120x50) is intentional,
            // not a validation gap: F14.2 (.opencode/plans/phase-14-close-out.md)
            // accepts viewports up to the transport bound (500x200) and clamps
            // them to MAX_SCREEN_WIDTH x MAX_SCREEN_HEIGHT for presentation
            // (#942 verdict). Do not narrow the accepted range to 120x50.
            let clamped_width = usize::from(width).min(crate::world_presentation::MAX_SCREEN_WIDTH);
            let clamped_height =
                usize::from(height).min(crate::world_presentation::MAX_SCREEN_HEIGHT);
            *viewport = Some(Viewport {
                width: clamped_width,
                height: clamped_height,
            });
            // The immediate viewport-specific view includes the latest state,
            // so it supersedes any coalesced redraw already pending.
            *pending_view_deadline = None;
            view_state.reset();
            return send_view(
                service.frame_meter(),
                world,
                io,
                token,
                sequence,
                *viewport,
                view_state,
                screen_patches,
            )
            .await;
        }
        ClientFrame::Paste { text } => {
            let Ok(bytes) = normalize_paste(&text) else {
                return send_error(
                    service.frame_meter(),
                    io,
                    "paste must be at most 4096 UTF-8 bytes and contain no control characters",
                )
                .await;
            };
            world
                .input_from_session(token.clone(), bytes, session_id)
                .await
        }
        ClientFrame::Input { data_hex } => match decode_hex(&data_hex) {
            Some(data) => {
                world
                    .input_from_session(token.clone(), data, session_id)
                    .await
            }
            None => {
                return send_error(service.frame_meter(), io, "input data must be hexadecimal")
                    .await;
            }
        },
        ClientFrame::Records { prefix, after, .. } => {
            // Authorized by the session's own capability.
            if let Err(message) = records_session(
                service.frame_meter(),
                world,
                io,
                token.clone(),
                prefix.as_deref(),
                after.as_deref(),
            )
            .await
            {
                return send_error(service.frame_meter(), io, message).await;
            }
            return true;
        }
        ClientFrame::SetExecutionProfile {
            admin_token,
            profile,
        } => {
            match service
                .set_execution_profile(Some(&admin_token), profile)
                .await
            {
                Ok(update) => {
                    let _ = send_json(service.frame_meter(),
                        io,
                        &ServerFrame::ExecutionProfile {
                            update,
                            message: "profile saved; it applies on the next program install or process restart, not to the running instance",
                        },
                    )
                    .await;
                }
                Err(error) => {
                    return send_error(service.frame_meter(), io, &format!("{error:#}")).await;
                }
            }
            return true;
        }
        ClientFrame::ExecutionProfileInfo { .. } => {
            match service.world().execution_profile().await {
                Ok(profile) => {
                    let _ = send_json(
                        service.frame_meter(),
                        io,
                        &ServerFrame::ExecutionProfile {
                            update: crate::world::ExecutionProfileUpdate {
                                requested: profile,
                                applies_on_next_program_install_or_process_restart: false,
                            },
                            message: "current persisted execution profile",
                        },
                    )
                    .await;
                }
                Err(error) => {
                    return send_error(service.frame_meter(), io, &format!("{error:#}")).await;
                }
            }
            return true;
        }
        ClientFrame::RecordsTicket { .. } => {
            if let Err(message) = records_ticket_session(service, io).await {
                return send_error(service.frame_meter(), io, message).await;
            }
            return true;
        }
        // Authorized by the session's own capability, like `records`; the
        // shell contract lists `info` in the joined phase (`/stats`).
        ClientFrame::Info { .. } => {
            let Ok(snapshot) = world.snapshot(token.clone()).await else {
                return send_error(service.frame_meter(), io, "join denied").await;
            };
            return send_json(
                service.frame_meter(),
                io,
                &ServerFrame::ModuleInfo {
                    world_id: snapshot.world_id,
                    active_module_hash: snapshot.active_module_hash,
                    active_module_provenance: snapshot.active_module_provenance,
                    current_sequence: snapshot.current_sequence,
                },
            )
            .await;
        }
        // Like `info`, the joined phase may read the world's metrics with
        // the session's own capability — gated on a snapshot with the
        // session token first, so a revoked participant is refused here
        // even when this frame wins the race against the revocation
        // broadcast (#1506).
        ClientFrame::Metrics { .. } => {
            if world.snapshot(token.clone()).await.is_err() {
                return send_error(service.frame_meter(), io, "join denied").await;
            }
            match world.metrics().await {
                Ok(metrics) => {
                    return send_json(service.frame_meter(), io, &ServerFrame::Metrics { metrics })
                        .await;
                }
                Err(error) => {
                    return send_error(service.frame_meter(), io, &format!("{error:#}")).await;
                }
            }
        }
        ClientFrame::Caps { .. } => {
            // Same capability gate as the query phase (#1506): the
            // session token must still name a live participant.
            if world.participant_id(token.clone()).await.is_err() {
                return send_error(service.frame_meter(), io, "join denied").await;
            }
            match service.world().caps_report().await {
                Ok(report) => {
                    let _ =
                        send_json(service.frame_meter(), io, &ServerFrame::Caps { report }).await;
                }
                Err(_) => {
                    return send_error(service.frame_meter(), io, "capability report unavailable")
                        .await;
                }
            }
            return true;
        }
        ClientFrame::UpdateCaps {
            admin_token,
            grant,
            revoke,
        } => {
            match service.authorize_admin(Some(&admin_token)) {
                Ok(()) => match service.world().update_caps(grant, revoke).await {
                    Ok(report) => {
                        let _ = send_json(service.frame_meter(), io, &ServerFrame::Caps { report })
                            .await;
                    }
                    Err(error) => {
                        return send_error(service.frame_meter(), io, &format!("{error:#}")).await;
                    }
                },
                Err(_) => {
                    return send_error(service.frame_meter(), io, "admin denied").await;
                }
            }
            return true;
        }
        ClientFrame::SetCapsCeiling {
            admin_token,
            ceiling,
        } => {
            match service.set_caps_ceiling(Some(&admin_token), ceiling).await {
                Ok(report) => {
                    let _ = send_json(
                        service.frame_meter(),
                        io,
                        &ServerFrame::CapsCeiling {
                            report,
                            message: CAPS_CEILING_NOTICE,
                        },
                    )
                    .await;
                }
                Err(error) => {
                    return send_error(service.frame_meter(), io, &format!("{error:#}")).await;
                }
            }
            return true;
        }
        ClientFrame::SetSchedule {
            admin_token,
            id,
            at_ms,
        } => {
            match service.set_schedule(Some(&admin_token), id, at_ms).await {
                Ok(schedules) => {
                    let _ = send_json(
                        service.frame_meter(),
                        io,
                        &ServerFrame::Schedules { schedules },
                    )
                    .await;
                }
                Err(error) => {
                    return send_error(service.frame_meter(), io, &format!("{error:#}")).await;
                }
            }
            return true;
        }
        ClientFrame::CancelSchedule { admin_token, id } => {
            match service.cancel_schedule(Some(&admin_token), id).await {
                Ok(schedules) => {
                    let _ = send_json(
                        service.frame_meter(),
                        io,
                        &ServerFrame::Schedules { schedules },
                    )
                    .await;
                }
                Err(error) => {
                    return send_error(service.frame_meter(), io, &format!("{error:#}")).await;
                }
            }
            return true;
        }
        ClientFrame::WorldState { admin_token, .. } => {
            let admin_ok = admin_token
                .as_deref()
                .is_some_and(|token| service.authorize_admin(Some(token)).is_ok());
            // The session's own capability authorizes the read, like `info`;
            // only a valid admin token in the frame unlocks the seed (#240).
            match world.manifest_state().await {
                Ok((seed, schedules)) => {
                    return send_json(
                        service.frame_meter(),
                        io,
                        &ServerFrame::WorldState {
                            seed: admin_ok.then_some(seed),
                            schedules,
                        },
                    )
                    .await;
                }
                Err(error) => {
                    return send_error(service.frame_meter(), io, &format!("{error:#}")).await;
                }
            }
        }
        ClientFrame::InstallBegin {
            admin_token,
            total_bytes,
            module_hash,
            signature,
            seed,
        } => {
            // A joined session installs on its capability's authority
            // (design #835 phase 3). An admin token here is mixed
            // credentials — the frame must choose one authority — and
            // is refused before any scope check, so the refusal cannot
            // be read as a statement about either credential.
            if admin_token.is_some() {
                return send_error(
                    service.frame_meter(),
                    io,
                    "install_begin in a joined session must omit the admin token",
                )
                .await;
            }
            return install_module_session_joined(
                service,
                io,
                token,
                total_bytes,
                module_hash.as_deref(),
                signature.as_ref(),
                seed,
            )
            .await;
        }
        ClientFrame::InstallChunk { .. } | ClientFrame::InstallEnd => {
            return send_error(service.frame_meter(), io, "no module upload in progress").await;
        }
        ClientFrame::Revoke { participant_id } => {
            // The early scope check classifies a certain denial with
            // the stable wire code; `revoke_authorized` re-checks
            // inside the actor at the effect boundary and derives the
            // audit actor from the capability, never from this frame.
            // A denial the command itself returns — the capability
            // expired, or a revocation landed, after the early check —
            // is classified from the command's own error and carries
            // the same code.
            if world
                .authorize(token.clone(), WorldScopes::MODERATOR)
                .await
                .is_err()
            {
                return send_error_with_code(
                    service.frame_meter(),
                    io,
                    Some(ACTOR_DENIED),
                    "revoke denied",
                )
                .await;
            }
            match world.revoke_authorized(token.clone(), participant_id).await {
                Ok(revoked) => {
                    return send_json(
                        service.frame_meter(),
                        io,
                        &ServerFrame::Revoked {
                            participant_id,
                            revoked,
                        },
                    )
                    .await;
                }
                Err(error) if crate::world::is_capability_denied(&error) => {
                    return send_error_with_code(
                        service.frame_meter(),
                        io,
                        Some(ACTOR_DENIED),
                        "revoke denied",
                    )
                    .await;
                }
                Err(error) => {
                    // The in-memory revocation stands (fail-closed);
                    // what failed is the audited success, so the reply
                    // is a failure, never a `Revoked` frame.
                    return send_error(
                        service.frame_meter(),
                        io,
                        &format!("revoke failed: {error:#}"),
                    )
                    .await;
                }
            }
        }
        ClientFrame::Join { .. }
        | ClientFrame::Invite { .. }
        | ClientFrame::Directory { .. }
        | ClientFrame::ListWorlds { .. }
        | ClientFrame::CreateWorld { .. }
        | ClientFrame::SetIsolation { .. }
        | ClientFrame::SetCrossWorldWrite { .. }
        | ClientFrame::ForceCrossWorldWrite { .. }
        | ClientFrame::SetFileScope { .. }
        | ClientFrame::Compile { .. }
        | ClientFrame::DownloadModule { .. }
        | ClientFrame::DownloadModuleAdmin { .. }
        | ClientFrame::EventHistoryAdmin { .. } => {
            return send_error(service.frame_meter(), io, "already joined").await;
        }
    };
    // Accepted events reach everyone, including the sender, through the
    // broadcast branch, so only rejections are answered directly. A
    // read-only replica names its mode: the refusal is the replica, not the
    // spectator's capability.
    if result.is_ok() {
        return true;
    }
    if service.world().is_read_only() {
        return send_error(
            service.frame_meter(),
            io,
            "world is read-only (replica); local mutations are refused",
        )
        .await;
    }
    send_error(service.frame_meter(), io, "world event rejected").await
}

/// Serialize and send one server frame, metering its bytes into the
/// world's host metrics (F11.7 frame bytes).
async fn send_json<I: SessionIo>(meter: &WorldMetrics, io: &mut I, frame: &impl Serialize) -> bool {
    match serde_json::to_string(frame) {
        Ok(encoded) if encoded.len() <= MAX_FRAME_BYTES => {
            let bytes = encoded.len() as u64;
            let sent = io.send(encoded).await;
            if sent {
                meter.add_frame_bytes(bytes);
            }
            sent
        }
        _ => false,
    }
}

async fn send_error<I: SessionIo>(meter: &WorldMetrics, io: &mut I, message: &str) -> bool {
    send_error_with_code(meter, io, None, message).await
}

/// An error frame carrying a stable denial `code` (design #835 §5).
/// Existing errors pass `None`, so their serialized shape is unchanged.
async fn send_error_with_code<I: SessionIo>(
    meter: &WorldMetrics,
    io: &mut I,
    code: Option<&str>,
    message: &str,
) -> bool {
    send_json(meter, io, &ServerFrame::Error { message, code }).await
}

async fn send_delegation<I: SessionIo>(
    meter: &WorldMetrics,
    io: &mut I,
    result: anyhow::Result<(Participant, JoinCapability)>,
) -> bool {
    match result {
        Ok((participant, capability)) => {
            let delegated = InviteResponse {
                capability: capability.expose().to_owned(),
                participant_id: participant.id,
                display_name: participant.display_name,
            };
            send_json(meter, io, &ServerFrame::Invite(&delegated)).await
        }
        Err(_) => send_error(meter, io, "delegation denied").await,
    }
}

/// Decode a bounded hex string.
#[must_use]
pub fn decode_hex(encoded: &str) -> Option<Vec<u8>> {
    if !encoded.len().is_multiple_of(2) || encoded.len() > MAX_FRAME_BYTES * 2 {
        return None;
    }
    encoded
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| Some((hex_nibble(pair[0])? << 4) | hex_nibble(pair[1])?))
        .collect()
}

pub(crate) fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(char::from(HEX[usize::from(byte >> 4)]));
        result.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    result
}

const fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

pub(crate) fn enqueue_all(
    queue: &mut crate::envelope_outbox::Outbox,
    outbound: Vec<crate::directory_view::Outbound>,
) -> anyhow::Result<Vec<String>> {
    use crate::envelope_outbox::Payload;
    let mut keys = Vec::with_capacity(outbound.len());
    for crate::directory_view::Outbound { target, payload } in outbound {
        let key = payload.key(&target);
        let queued = match payload {
            Payload::Envelope(envelope) => queue.enqueue(&target, envelope),
            Payload::Artifact(artifact) => queue.enqueue_artifact(&target, artifact),
        };
        if let Err(error) = queued {
            withdraw_all(queue, &keys);
            return Err(error);
        }
        keys.push(key);
    }
    Ok(keys)
}

pub(crate) fn withdraw_all(queue: &mut crate::envelope_outbox::Outbox, keys: &[String]) {
    for key in keys {
        if let Err(withdrawn) = queue.withdraw(key) {
            tracing::error!("{key} stays queued though its change was refused: {withdrawn:#}");
        }
    }
}
