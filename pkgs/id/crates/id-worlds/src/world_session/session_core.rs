//! Session core loop (split from the parent module, #8594): the
//! joined-session driver and the one-shot query path. Frame handlers,
//! session glue for the satellite surfaces (records, documents,
//! modules), and the view machinery stay in the parent and sibling
//! modules; everything here resolves them through `super::` exactly
//! as it did inline. No behavior or public-path change.

use super::*;

/// Serve one session against its resolved world, starting from its first frame.
pub(crate) async fn run_world_session<I: SessionIo>(
    service: &WorldService,
    io: &mut I,
    frame: ClientFrame,
    world_name: &str,
    documents_store: Option<Store>,
) {
    match frame {
        ClientFrame::Join {
            capability,
            after,
            screen_patches,
            screen_msgpack,
            resume_token,
            agent_profile,
            device_input,
        } => {
            if capability.len() > MAX_CAPABILITY_BYTES {
                let _ = send_error(service.frame_meter(), io, "invalid join capability").await;
                return;
            }
            joined(
                service,
                io,
                world_name,
                JoinCapability::from_wire(capability),
                after,
                screen_patches,
                screen_msgpack,
                resume_token,
                agent_profile,
                device_input,
                documents_store,
            )
            .await;
        }
        ClientFrame::FormResult { .. } => {
            let _ = send_error(
                service.frame_meter(),
                io,
                "form results need a joined session",
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
                audit_admin_frame(world_name, "Invite", "refused", serde_json::json!({}));
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
                    audit_admin_frame(world_name, "Invite", "accepted", serde_json::json!({}));
                }
                Err(InviteError::InvalidBounds(message)) => {
                    let _ = send_error(service.frame_meter(), io, message).await;
                    audit_admin_frame(world_name, "Invite", "refused", serde_json::json!({}));
                }
                Err(InviteError::Disabled) => {
                    let _ = send_error(service.frame_meter(), io, "invites are not enabled").await;
                    audit_admin_frame(world_name, "Invite", "refused", serde_json::json!({}));
                }
                Err(InviteError::Unauthorized) => {
                    let _ = send_error(service.frame_meter(), io, "invite denied").await;
                    audit_admin_frame(world_name, "Invite", "refused", serde_json::json!({}));
                }
                Err(InviteError::Unavailable) => {
                    let _ = send_error(
                        service.frame_meter(),
                        io,
                        "world cannot issue another invite",
                    )
                    .await;
                    audit_admin_frame(world_name, "Invite", "refused", serde_json::json!({}));
                }
            },
        },
        // Query frames keep the session open for further queries: a client
        // can page through records, and may still join later on.
        ClientFrame::Info { .. }
        | ClientFrame::Metrics { .. }
        | ClientFrame::Records { .. }
        | ClientFrame::RecordsTicket { .. }
        | ClientFrame::Documents { .. }
        | ClientFrame::Document { .. }
        | ClientFrame::DownloadModule { .. } => {
            query_session(service, io, frame, world_name, documents_store).await;
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
                    audit_admin_frame(world_name, "InstallBegin", "refused", serde_json::json!({}));
                    let _ = send_error(service.frame_meter(), io, "module install denied").await;
                    return;
                }
                audit_admin_frame(
                    world_name,
                    "InstallBegin",
                    "refused",
                    serde_json::json!({"reason": "source_only"}),
                );
                let _ = send_error(
                    service.frame_meter(),
                    io,
                    "raw module uploads are disabled by source-only policy",
                )
                .await;
                return;
            }
            let install_result = install_module_session(
                service,
                io,
                &admin_token,
                total_bytes,
                module_hash.as_deref(),
                signature.as_ref(),
                seed,
            )
            .await;
            audit_admin_frame(
                world_name,
                "InstallBegin",
                if install_result.is_ok() {
                    "accepted"
                } else {
                    "refused"
                },
                serde_json::json!({"total_bytes": total_bytes}),
            );
            if let Err(message) = install_result {
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
                audit_admin_frame(
                    world_name,
                    "SetExecutionProfile",
                    "accepted",
                    serde_json::json!({}),
                );
            }
            Err(error) => {
                audit_admin_frame(
                    world_name,
                    "SetExecutionProfile",
                    "refused",
                    serde_json::json!({}),
                );
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
            let result = event_history_admin_session(service, io, &admin_token).await;
            audit_admin_frame(
                world_name,
                "EventHistoryAdmin",
                if result.is_ok() {
                    "accepted"
                } else {
                    "refused"
                },
                serde_json::json!({}),
            );
            if let Err(message) = result {
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
                    audit_admin_frame(world_name, "Compile", "accepted", serde_json::json!({}));
                }
                Err(message) => {
                    audit_admin_frame(world_name, "Compile", "refused", serde_json::json!({}));
                    let _ = send_error(service.frame_meter(), io, &message).await;
                }
            }
        }
        #[cfg(not(feature = "sandbox"))]
        ClientFrame::Compile { .. } => {
            audit_admin_frame(world_name, "Compile", "refused", serde_json::json!({}));
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
        | ClientFrame::Paste { .. }
        | ClientFrame::Pointer { .. }
        | ClientFrame::Hover { .. }
        | ClientFrame::ProviderOffer { .. }
        | ClientFrame::ProviderReply { .. }
        | ClientFrame::ScreenSubscribe { .. }
        | ClientFrame::Pointer { .. }
        | ClientFrame::Hover { .. }
        | ClientFrame::FormResult { .. } => {
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
async fn query_session<I: SessionIo>(
    service: &WorldService,
    io: &mut I,
    first: ClientFrame,
    world_name: &str,
    documents_store: Option<Store>,
) {
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
                Inbound::Unsupported | Inbound::Bytes(_) => {
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
                screen_msgpack,
                resume_token,
                agent_profile,
                device_input,
            } => {
                if capability.len() > MAX_CAPABILITY_BYTES {
                    let _ = send_error(service.frame_meter(), io, "invalid join capability").await;
                    return;
                }
                joined(
                    service,
                    io,
                    world_name,
                    JoinCapability::from_wire(capability),
                    after,
                    screen_patches,
                    screen_msgpack,
                    resume_token,
                    agent_profile,
                    device_input,
                    documents_store.clone(),
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
            ClientFrame::Documents {
                capability,
                prefix,
                after,
            } => {
                let token = JoinCapability::from_wire(capability);
                if service.world().participant_id(token.clone()).await.is_ok() {
                    io.mark_authorized();
                }
                let store = documents_store.as_ref().or(service.blobs.as_ref());
                if let Err(message) = documents_session(
                    service.frame_meter(),
                    service.world(),
                    io,
                    token,
                    store,
                    prefix.as_deref(),
                    after.as_deref(),
                )
                .await
                {
                    let _ = send_error(service.frame_meter(), io, message).await;
                    return;
                }
            }
            ClientFrame::Document { capability, name } => {
                let token = JoinCapability::from_wire(capability);
                if service.world().participant_id(token.clone()).await.is_ok() {
                    io.mark_authorized();
                }
                let store = documents_store.as_ref().or(service.blobs.as_ref());
                if let Err(message) = document_session(
                    service.frame_meter(),
                    service.world(),
                    io,
                    token,
                    store,
                    &name,
                )
                .await
                {
                    let _ = send_error(service.frame_meter(), io, message).await;
                    return;
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
