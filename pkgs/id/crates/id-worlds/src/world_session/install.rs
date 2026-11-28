//! Module upload and install sessions (split from the parent module,
//! #16255): the shared upload receiver (chunk framing, offsets, and
//! bounds safeguards), the pre-join admin install, and the joined
//! builder install authorized by the session's live capability. No
//! behavior or public-path change: the parent re-exports the shared
//! items.

use super::*;

/// The wire code carried by error frames that refuse a joined
/// session's role/capability action (design #835 §5): one stable,
/// non-oracular code for invalid, expired, revoked, and insufficient
/// capabilities alike.
pub(crate) const ACTOR_DENIED: &str = "actor_denied";

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
pub(crate) async fn install_module_session_joined<I: SessionIo>(
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
            Ok((module_hash, sequence)) => {
                send_json(
                    service.frame_meter(),
                    io,
                    &ServerFrame::ModuleInstalled {
                        module_hash: &module_hash,
                        sequence,
                    },
                )
                .await
            }
            Err(InstallError::ActorDenied) => {
                send_error_with_code(
                    service.frame_meter(),
                    io,
                    Some(ACTOR_DENIED),
                    "module install denied",
                )
                .await
            }
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

pub(crate) async fn install_module_session<I: SessionIo>(
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
