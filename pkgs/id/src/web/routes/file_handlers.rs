//! File-management routes and their supporting transformations.

use super::*;

/// Save the current editor document to the blob store.
///
/// Converts the `ProseMirror` JSON to the appropriate format based on filename,
/// creates a new blob and signed document revision, archives the original under
/// a timestamped name, and updates the legacy filename tag to point to the new blob.
///
/// Rate-limited: rejects saves within the cooldown period (default 5s) per file.
pub(super) async fn save_handler(
    State(state): State<AppState>,
    Json(req): Json<SaveRequest>,
) -> Response {
    tracing::info!(
        "[routes] save_handler: doc_id={}, name={}",
        req.doc_id,
        req.name
    );

    // Check save rate limit and reserve the filename for this save
    // (#2466): a second save of the same file arriving while this one is
    // in flight is refused here instead of racing to the filename tag.
    // The permit releases the reservation on every exit path below.
    let permit = match state.save_limiter.acquire(&req.name).await {
        Ok(permit) => permit,
        Err(remaining) => {
            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            let secs = remaining.as_secs_f64().ceil() as u64;
            tracing::info!(
                "[routes] Save rate limited for '{}': {}s remaining",
                req.name,
                secs
            );
            // Structured refusal (#2244 P0-A): machine-readable reason and
            // retry value in the body, `Retry-After` header alongside.
            return (
                StatusCode::TOO_MANY_REQUESTS,
                [(header::RETRY_AFTER, secs.to_string())],
                Json(serde_json::json!({
                    "error": format!("Save rate limited. Try again in {secs}s."),
                    "reason": "rate_limited",
                    "retry_after_ms": secs.saturating_mul(1_000),
                })),
            )
                .into_response();
        }
    };

    // Convert ProseMirror doc to the appropriate format based on file name
    let bytes = match convert_doc_to_bytes(&req.name, &req.doc) {
        Ok(b) => b,
        Err(err) => {
            tracing::error!("[routes] Failed to convert document: {}", err);
            return (StatusCode::BAD_REQUEST, err).into_response();
        }
    };

    // Add new blob to store
    let add_result = state.store.blobs().add_bytes(bytes.clone()).await;
    let outcome = match add_result {
        Ok(outcome) => outcome,
        Err(err) => {
            tracing::error!("[routes] Failed to add blob: {}", err);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to save file").into_response();
        }
    };
    let new_hash = outcome.hash;
    let new_hash_str = new_hash.to_string();

    // A saved revision is durable only once its bytes are addressable and its signed event is
    // applied. The mutex protects LocalEvents' synchronous redb writes and is released before any
    // subsequent network-backed metadata operations.
    let filename_tag = [("filename".to_owned(), Some(req.name.clone()))];
    let publish_result = {
        let mut events = state.local_events.lock().await;
        events.publish(get_content_type(&req.name), &bytes, &filename_tag)
    };
    let published = match publish_result {
        Ok(published) => published,
        Err(err) => {
            tracing::error!("[routes] Failed to publish saved revision: {:#}", err);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to publish document revision",
            )
                .into_response();
        }
    };
    let document_id = published.id.to_hex();

    // Archive the original blob under a timestamped tag
    let archive_name =
        archive_original_tag(&state.store, &state.tag_store, &req.name, &req.doc_id).await;

    // Set the tag to point to the new blob
    let tag = iroh_blobs::api::Tag::from(req.name.clone());
    if let Err(err) = state.store.tags().set(tag, new_hash).await {
        tracing::error!("[routes] Failed to set tag: {}", err);
        return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to update tag").into_response();
    }

    // Update metadata tags: set created (if first save) and modified
    let now_str = crate::tags::now_unix().to_string();
    // `created_fresh` gates the origin stamp below: a legacy file
    // (created before origin stamping existed) keeps an unknown
    // origin on overwrite instead of claiming this node (Codex
    // review, PR #5191).
    let created_fresh = match state
        .tag_store
        .set_if_absent(
            &state.tag_store.global,
            req.name.as_bytes(),
            b"created",
            Some(now_str.as_bytes()),
            b"",
        )
        .await
    {
        Ok(fresh) => fresh,
        Err(err) => {
            tracing::warn!("[routes] Failed to set created tag: {:#}", err);
            false
        }
    };

    // Stamp this node as the file's origin (#4973) only on a genuine
    // first write (`created` was absent): the first writer stays the
    // origin, and legacy files keep an unknown origin.
    if created_fresh
        && let Err(err) = state
            .tag_store
            .set_if_absent(
                &state.tag_store.global,
                req.name.as_bytes(),
                b"origin",
                Some(state.node_id.as_bytes()),
                b"",
            )
            .await
    {
        tracing::warn!("[routes] Failed to set origin tag: {:#}", err);
    }
    if let Err(err) = state
        .tag_store
        .set_singleton(
            &state.tag_store.global,
            req.name.as_bytes(),
            b"modified",
            Some(now_str.as_bytes()),
            b"",
        )
        .await
    {
        tracing::warn!("[routes] Failed to set modified tag: {:#}", err);
    }
    if let Some(ref archive) = archive_name
        && let Err(err) = state
            .tag_store
            .set_tag(
                &state.tag_store.global,
                req.name.as_bytes(),
                b"archive.save",
                Some(archive.as_bytes()),
                b"",
            )
            .await
    {
        tracing::warn!("[routes] Failed to set archive tag: {}", err);
    }

    tracing::info!(
        "[routes] File saved: name={}, hash={}, archive={:?}",
        req.name,
        new_hash_str,
        archive_name
    );

    // Notify collab clients editing the old hash about the new version
    state
        .collab
        .notify_new_version(&req.name, &new_hash_str)
        .await;

    // Record successful save for rate limiting (also releases the
    // in-flight reservation as the permit drops)
    permit.complete().await;

    Json(SaveResponse {
        hash: new_hash_str,
        document_id,
        name: req.name,
        archive_name,
    })
    .into_response()
}

/// Create a new empty file in the blob store.
///
/// Creates appropriate empty content based on the file extension,
/// adds it as a blob, and creates a tag for it.
pub(super) async fn new_file_handler(
    State(state): State<AppState>,
    Json(req): Json<NewFileRequest>,
) -> Response {
    tracing::info!("[routes] new_file_handler: name={}", req.name);

    if req.name.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "File name cannot be empty").into_response();
    }

    // Create appropriate empty content based on file type
    let mode = detect_mode(&req.name);
    let content = match mode {
        ContentMode::Rich => b"{}".to_vec(), // Empty PM JSON
        ContentMode::Markdown
        | ContentMode::Plain
        | ContentMode::Raw
        | ContentMode::Binary
        | ContentMode::Media(_) => b"".to_vec(),
    };

    // Add blob to store
    let add_result = state.store.blobs().add_bytes(content).await;
    let outcome = match add_result {
        Ok(outcome) => outcome,
        Err(err) => {
            tracing::error!("[routes] Failed to add blob: {}", err);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to create file").into_response();
        }
    };
    let hash = outcome.hash;
    let hash_str = hash.to_string();

    // Set tag
    let tag = iroh_blobs::api::Tag::from(req.name.clone());
    if let Err(err) = state.store.tags().set(tag, hash).await {
        tracing::error!("[routes] Failed to set tag: {}", err);
        return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to set file name").into_response();
    }

    // Set metadata tags: created + modified
    let now_str = crate::tags::now_unix().to_string();
    // `created_fresh` gates the origin stamp below: a legacy file
    // (created before origin stamping existed) keeps an unknown
    // origin on overwrite instead of claiming this node (Codex
    // review, PR #5191).
    let created_fresh = match state
        .tag_store
        .set_if_absent(
            &state.tag_store.global,
            req.name.as_bytes(),
            b"created",
            Some(now_str.as_bytes()),
            b"",
        )
        .await
    {
        Ok(fresh) => fresh,
        Err(err) => {
            tracing::warn!("[routes] Failed to set created tag: {:#}", err);
            false
        }
    };

    // Stamp this node as the file's origin (#4973) only on a genuine
    // first write (`created` was absent): the first writer stays the
    // origin, and legacy files keep an unknown origin.
    if created_fresh
        && let Err(err) = state
            .tag_store
            .set_if_absent(
                &state.tag_store.global,
                req.name.as_bytes(),
                b"origin",
                Some(state.node_id.as_bytes()),
                b"",
            )
            .await
    {
        tracing::warn!("[routes] Failed to set origin tag: {:#}", err);
    }
    if let Err(err) = state
        .tag_store
        .set_singleton(
            &state.tag_store.global,
            req.name.as_bytes(),
            b"modified",
            Some(now_str.as_bytes()),
            b"",
        )
        .await
    {
        tracing::warn!("[routes] Failed to set modified tag: {:#}", err);
    }

    tracing::info!(
        "[routes] New file created: name={}, hash={}",
        req.name,
        hash_str
    );

    Json(NewFileResponse {
        hash: hash_str,
        name: req.name,
    })
    .into_response()
}

/// Whether a multipart error is the request-body length limit firing
/// (as opposed to malformed multipart data).
///
/// This must classify via `MultipartError::status()`, NOT by matching on
/// the error's `Display`: in axum 0.8, `MultipartError`'s `Display` is the
/// constant string "Error parsing `multipart/form-data` request" for every
/// error, so a substring match can never fire (#751). axum derives the
/// status from the underlying multer error and returns
/// `413 PAYLOAD_TOO_LARGE` exactly for the length-limit cases
/// (`FieldSizeExceeded` / `StreamSizeExceeded` / a wrapped
/// `LengthLimitError`).
pub(super) fn is_body_limit_error(err: &axum::extract::multipart::MultipartError) -> bool {
    err.status() == StatusCode::PAYLOAD_TOO_LARGE
}

/// The clear `413` response for an upload that exceeds the size limits.
pub(super) fn image_too_large_response() -> Response {
    (
        StatusCode::PAYLOAD_TOO_LARGE,
        format!(
            "Image too large: the upload exceeds the maximum image size of {MAX_IMAGE_SIZE} bytes (10 MiB)"
        ),
    )
        .into_response()
}

/// Upload an image file via multipart form data.
///
/// Accepts a single file field, validates the MIME type against allowed
/// image types, stores the blob, creates a named tag, and sets metadata.
/// The hash `name` currently points at, if the name is taken (#1549).
pub(super) async fn upload_name_owner(
    store: &iroh_blobs::api::Store,
    name: &str,
) -> Result<Option<iroh_blobs::Hash>, String> {
    store
        .tags()
        .get(name)
        .await
        .map(|info| info.map(|tag| tag.hash))
        .map_err(|err| err.to_string())
}

/// Choose the stored name for an upload of `hash` intended as
/// `intended` (#1549): the intended name itself when it is free or
/// already names this exact blob (an idempotent re-upload of identical
/// bytes keeps its name), otherwise `stem-N.ext` with the smallest
/// N >= 1 that is free or already names this blob — so a later upload
/// never silently un-names an earlier, different image.
pub(super) async fn uniquify_upload_name(
    store: &iroh_blobs::api::Store,
    intended: &str,
    hash: iroh_blobs::Hash,
) -> Result<String, String> {
    match upload_name_owner(store, intended).await? {
        None => return Ok(intended.to_owned()),
        Some(owner) if owner == hash => return Ok(intended.to_owned()),
        Some(_) => {}
    }
    let (stem, ext) = match intended.rfind('.') {
        Some(dot) if dot > 0 => (&intended[..dot], &intended[dot..]),
        _ => (intended, ""),
    };
    for n in 1.. {
        let candidate = format!("{stem}-{n}{ext}");
        match upload_name_owner(store, &candidate).await? {
            None => return Ok(candidate),
            Some(owner) if owner == hash => return Ok(candidate),
            Some(_) => {}
        }
    }
    unreachable!("the counter always finds a free name")
}

pub(super) async fn upload_handler(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Response {
    // Read the first field from the multipart body
    let field = match multipart.next_field().await {
        Ok(Some(field)) => field,
        Ok(None) => {
            return (StatusCode::BAD_REQUEST, "No file field in upload").into_response();
        }
        Err(err) => {
            tracing::error!("[routes] Failed to read multipart field: {}", err);
            if is_body_limit_error(&err) {
                return image_too_large_response();
            }
            return (
                StatusCode::BAD_REQUEST,
                format!("Invalid multipart data: {err}"),
            )
                .into_response();
        }
    };

    // Validate content type
    let content_type = field
        .content_type()
        .unwrap_or("application/octet-stream")
        .to_owned();
    if !ALLOWED_IMAGE_TYPES.contains(&content_type.as_str()) {
        return (
            StatusCode::BAD_REQUEST,
            format!("Unsupported image type: {content_type}"),
        )
            .into_response();
    }

    // Get filename from the field, or generate one. A supplied name is
    // reconciled with the validated content type (#908) so it cannot
    // masquerade as a different type than the bytes were checked as.
    let ext = mime_to_extension(&content_type);
    let filename = match field.file_name().map(ToOwned::to_owned) {
        Some(name) if !name.trim().is_empty() => reconcile_upload_filename(&name, ext),
        _ => {
            let ts = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            format!("paste-{ts}.{ext}")
        }
    };

    // Read the file bytes
    let data = match field.bytes().await {
        Ok(bytes) => bytes,
        Err(err) => {
            tracing::error!("[routes] Failed to read upload bytes: {}", err);
            if is_body_limit_error(&err) {
                return image_too_large_response();
            }
            return (StatusCode::BAD_REQUEST, "Failed to read file data").into_response();
        }
    };

    // The route body limit leaves room for the multipart envelope, so the
    // file itself must be checked against the advertised file limit here —
    // with a clear size error, not a generic multipart/body failure (#716).
    if data.len() > MAX_IMAGE_SIZE {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "Image too large: {} bytes exceeds the maximum image size of {} bytes (10 MiB)",
                data.len(),
                MAX_IMAGE_SIZE
            ),
        )
            .into_response();
    }

    if data.is_empty() {
        return (StatusCode::BAD_REQUEST, "Uploaded file is empty").into_response();
    }

    // #5342: the declared Content-Type above is caller-asserted; verify
    // the bytes themselves carry that type's signature before storing,
    // so the stored name/type metadata cannot claim a format the
    // content does not have. A mismatch stores and tags nothing.
    if !image_bytes_match_declared_type(&content_type, &data) {
        return (
            StatusCode::BAD_REQUEST,
            format!("Uploaded bytes do not match the declared image type: {content_type}"),
        )
            .into_response();
    }

    tracing::info!(
        "[routes] upload_handler: name={}, content_type={}, size={}",
        filename,
        content_type,
        data.len()
    );

    // Store blob
    let outcome = match state.store.blobs().add_bytes(data.to_vec()).await {
        Ok(outcome) => outcome,
        Err(err) => {
            tracing::error!("[routes] Failed to add upload blob: {}", err);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to store file").into_response();
        }
    };
    let hash = outcome.hash;
    let hash_str = hash.to_string();

    // #2363: Keep the check, unconditional tag set and filename-keyed
    // metadata writes in one critical section across concurrent uploads.
    // This only coordinates handlers sharing this AppState, not independent
    // states/processes or other code that writes blob tags directly.
    let _upload_name_guard = state.upload_name_lock.lock().await;

    // #1549: resolve the stored name before tagging anything. The
    // intended name is kept when free or already naming this blob; a
    // name owned by a different blob is uniquified (stem-N.ext), and
    // the nameless paste-{ts} fallback passes through the same
    // uniquifier, so two uploads can never land on the same stored
    // name. The tag, the filename-keyed metadata, and the response
    // below all use the stored name.
    let filename = match uniquify_upload_name(&state.store, &filename, hash).await {
        Ok(name) => name,
        Err(err) => {
            tracing::error!("[routes] Failed to resolve upload name: {}", err);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to resolve file name",
            )
                .into_response();
        }
    };

    // Set named tag
    let tag = iroh_blobs::api::Tag::from(filename.clone());
    if let Err(err) = state.store.tags().set(tag, hash).await {
        tracing::error!("[routes] Failed to set upload tag: {}", err);
        return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to set file name").into_response();
    }

    // Set metadata tags (created, modified, content-type)
    let now_str = crate::tags::now_unix().to_string();
    // `created_fresh` gates the origin stamp below: a legacy file
    // (created before origin stamping existed) keeps an unknown
    // origin on overwrite instead of claiming this node (Codex
    // review, PR #5191).
    let created_fresh = match state
        .tag_store
        .set_if_absent(
            &state.tag_store.global,
            filename.as_bytes(),
            b"created",
            Some(now_str.as_bytes()),
            b"",
        )
        .await
    {
        Ok(fresh) => fresh,
        Err(err) => {
            tracing::warn!("[routes] Failed to set created tag: {:#}", err);
            false
        }
    };

    // Stamp this node as the file's origin (#4973) only on a genuine
    // first write (`created` was absent): the first writer stays the
    // origin, and legacy files keep an unknown origin.
    if created_fresh
        && let Err(err) = state
            .tag_store
            .set_if_absent(
                &state.tag_store.global,
                filename.as_bytes(),
                b"origin",
                Some(state.node_id.as_bytes()),
                b"",
            )
            .await
    {
        tracing::warn!("[routes] Failed to set origin tag: {:#}", err);
    }
    if let Err(err) = state
        .tag_store
        .set_singleton(
            &state.tag_store.global,
            filename.as_bytes(),
            b"modified",
            Some(now_str.as_bytes()),
            b"",
        )
        .await
    {
        tracing::warn!("[routes] Failed to set modified tag: {:#}", err);
    }
    if let Err(err) = state
        .tag_store
        .set_singleton(
            &state.tag_store.global,
            filename.as_bytes(),
            b"content-type",
            Some(content_type.as_bytes()),
            b"",
        )
        .await
    {
        tracing::warn!("[routes] Failed to set content-type tag: {:#}", err);
    }

    let url = format!(
        "/blob/{hash_str}?filename={}",
        urlencoding::encode(&filename)
    );

    Json(UploadResponse {
        hash: hash_str,
        name: filename,
        url,
    })
    .into_response()
}

/// List all uploaded images.
///
/// Scans blob tags for entries with image content-type metadata.
/// Returns a JSON array of `ImageEntry` objects.
pub(super) async fn images_list_handler(State(state): State<AppState>) -> Response {
    use futures_lite::StreamExt;

    let mut images: Vec<ImageEntry> = Vec::new();

    // List all tags (named blobs) and filter to those with image content-types
    let mut tags_stream = match state.store.tags().list().await {
        Ok(s) => s,
        Err(err) => {
            tracing::error!("[routes] Failed to list tags for images: {}", err);
            return Json(images).into_response();
        }
    };
    while let Some(Ok(tag_info)) = tags_stream.next().await {
        let name = String::from_utf8_lossy(tag_info.name.as_ref()).to_string();
        let hash_str = tag_info.hash.to_string();

        // Skip internal tags
        if crate::tags::is_internal_tag(&name) {
            continue;
        }

        // Check content-type metadata via tag store
        let ct_tags = state
            .tag_store
            .get_by_key(&state.tag_store.global, name.as_bytes(), b"content-type")
            .await
            .unwrap_or_default();

        let ct = ct_tags
            .first()
            .and_then(|t| t.value.as_ref())
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if !ALLOWED_IMAGE_TYPES.contains(&ct) {
            continue;
        }

        let url = format!("/blob/{hash_str}?filename={}", urlencoding::encode(&name));

        images.push(ImageEntry {
            hash: hash_str,
            name,
            url,
            size: blob_size(&state.store, tag_info.hash).await,
        });
    }

    Json(images).into_response()
}

/// Rename a file by changing its tag name.
///
/// If the target name already exists, archives it first under
/// `{new_name}.archive.{timestamp}`. Optionally archives the original name
/// as `{name}.archive.{timestamp}` (when `archive` is true) or simply
/// deletes it (when `archive` is false).
pub(super) async fn rename_handler(
    State(state): State<AppState>,
    Json(req): Json<RenameRequest>,
) -> Response {
    tracing::info!(
        "[routes] rename_handler: name={}, new_name={}, archive={}",
        req.name,
        req.new_name,
        req.archive
    );
    let new_name = req.new_name.trim().to_owned();
    let ops = FileOps::new(&state.store, &state.tag_store);
    match ops.rename(&req.name, &new_name, req.archive).await {
        Ok(outcome) => {
            tracing::info!(
                "[routes] File renamed: {} -> {}, hash={}, archived_original={:?}, archived_replaced={:?}",
                req.name,
                new_name,
                outcome.hash,
                outcome.archived_original,
                outcome.archived_replaced
            );
            Json(RenameResponse {
                name: new_name,
                hash: outcome.hash.to_string(),
                archived_original: outcome.archived_original,
                archived_replaced: outcome.archived_replaced,
            })
            .into_response()
        }
        Err(FileOpError::EmptyName) => {
            (StatusCode::BAD_REQUEST, "New name cannot be empty").into_response()
        }
        Err(err @ FileOpError::ReservedArchiveName(_)) => {
            (StatusCode::BAD_REQUEST, err.to_string()).into_response()
        }
        Err(FileOpError::SameName) => (
            StatusCode::BAD_REQUEST,
            "New name must differ from current name",
        )
            .into_response(),
        Err(FileOpError::NotFound(_)) => (StatusCode::NOT_FOUND, "File not found").into_response(),
        Err(FileOpError::InvalidName(_)) => (
            StatusCode::BAD_REQUEST,
            "New name must be a safe relative path without padding",
        )
            .into_response(),
        Err(err) => {
            tracing::error!("[routes] rename failed: {err}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to rename file").into_response()
        }
    }
}

/// Copy a file by creating a new tag pointing to the same content hash.
///
/// If the destination name already exists, the existing file at that name
/// is archived as `{new_name}.archive.{timestamp}` before the copy.
pub(super) async fn copy_handler(
    State(state): State<AppState>,
    Json(req): Json<CopyRequest>,
) -> Response {
    tracing::info!(
        "[routes] copy_handler: name={}, new_name={}",
        req.name,
        req.new_name
    );
    let new_name = req.new_name.trim().to_owned();
    let ops = FileOps::new(&state.store, &state.tag_store);
    match ops.copy(&req.name, &new_name).await {
        Ok(outcome) => {
            tracing::info!(
                "[routes] File copied: {} -> {}, hash={}",
                req.name,
                new_name,
                outcome.hash
            );
            Json(CopyResponse {
                name: new_name,
                hash: outcome.hash.to_string(),
            })
            .into_response()
        }
        Err(FileOpError::EmptyName) => {
            (StatusCode::BAD_REQUEST, "New name cannot be empty").into_response()
        }
        Err(err @ FileOpError::ReservedArchiveName(_)) => {
            (StatusCode::BAD_REQUEST, err.to_string()).into_response()
        }
        Err(FileOpError::SameName) => (
            StatusCode::BAD_REQUEST,
            "New name must differ from current name",
        )
            .into_response(),
        Err(FileOpError::NotFound(_)) => {
            (StatusCode::NOT_FOUND, "Source file not found").into_response()
        }
        Err(FileOpError::InvalidName(_)) => (
            StatusCode::BAD_REQUEST,
            "New name must be a safe relative path without padding",
        )
            .into_response(),
        Err(err) => {
            tracing::error!("[routes] copy failed: {err}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to copy file").into_response()
        }
    }
}

/// Download the current editor content in the requested format.
///
/// Supports two formats:
/// - `raw`: Converts `ProseMirror` JSON to the native file format (markdown, plain text, etc.)
/// - `json`: Returns the `ProseMirror` JSON document as-is
///
/// For downloading the original stored blob, use `GET /blob/:hash` directly.
pub(super) async fn download_handler(Json(req): Json<DownloadRequest>) -> Response {
    tracing::info!(
        "[routes] download_handler: name={}, format={}",
        req.name,
        req.format
    );

    match req.format.as_str() {
        "raw" => {
            // Convert PM doc to native format
            let bytes = match convert_doc_to_bytes(&req.name, &req.doc) {
                Ok(b) => b,
                Err(err) => {
                    return (StatusCode::BAD_REQUEST, err).into_response();
                }
            };
            let content_type = get_content_type(&req.name);
            let filename_encoded = urlencoding::encode(&req.name);

            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, content_type)
                .header(
                    header::CONTENT_DISPOSITION,
                    format!("attachment; filename=\"{filename_encoded}\""),
                )
                .body(Body::from(bytes))
                .unwrap_or_else(|_| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Failed to build response",
                    )
                        .into_response()
                })
        }
        "json" => {
            // Return PM JSON as-is
            let json_bytes = serde_json::to_vec_pretty(&req.doc).unwrap_or_default();
            let json_name = if req.name.to_lowercase().ends_with(".pm.json") {
                req.name.clone()
            } else {
                format!("{}.pm.json", req.name)
            };
            let filename_encoded = urlencoding::encode(&json_name);

            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "application/json")
                .header(
                    header::CONTENT_DISPOSITION,
                    format!("attachment; filename=\"{filename_encoded}\""),
                )
                .body(Body::from(json_bytes))
                .unwrap_or_else(|_| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Failed to build response",
                    )
                        .into_response()
                })
        }
        _ => (
            StatusCode::BAD_REQUEST,
            "Invalid format. Use 'raw' or 'json'",
        )
            .into_response(),
    }
}

/// Request body for soft-deleting or hard-deleting a file.
pub(super) async fn delete_handler(
    State(state): State<AppState>,
    Json(req): Json<DeleteRequest>,
) -> Response {
    tracing::info!("[routes] delete_handler: name={}", req.name);

    if req.name.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "File name cannot be empty").into_response();
    }

    // Verify the file exists
    match state.store.tags().get(&req.name).await {
        Ok(Some(_)) => {}
        Ok(None) => return (StatusCode::NOT_FOUND, "File not found").into_response(),
        Err(err) => {
            tracing::error!("[routes] Failed to look up tag: {}", err);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to look up file").into_response();
        }
    }

    // Add "deleted" tag with timestamp
    let now_str = crate::tags::now_unix().to_string();
    if let Err(err) = state
        .tag_store
        .set_singleton(
            &state.tag_store.global,
            req.name.as_bytes(),
            b"deleted",
            Some(now_str.as_bytes()),
            b"",
        )
        .await
    {
        tracing::error!("[routes] Failed to set deleted tag: {}", err);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to mark file as deleted",
        )
            .into_response();
    }

    tracing::info!("[routes] File soft-deleted: name={}", req.name);

    Json(DeleteResponse {
        name: req.name,
        hard: false,
    })
    .into_response()
}

/// Restore a soft-deleted file by removing the "deleted" tag.
pub(super) async fn restore_handler(
    State(state): State<AppState>,
    Json(req): Json<DeleteRequest>,
) -> Response {
    tracing::info!("[routes] restore_handler: name={}", req.name);

    if req.name.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "File name cannot be empty").into_response();
    }

    // Remove the "deleted" tag (all values for this key)
    if let Err(err) = state
        .tag_store
        .del_by_key(&state.tag_store.global, req.name.as_bytes(), b"deleted")
        .await
    {
        tracing::error!("[routes] Failed to remove deleted tag: {}", err);
        return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to restore file").into_response();
    }

    tracing::info!("[routes] File restored: name={}", req.name);

    Json(DeleteResponse {
        name: req.name,
        hard: false,
    })
    .into_response()
}

/// Hard-delete a file (admin only).
///
/// Removes the blob tag, all archive tags for this file, and all metadata tags.
/// The blob data itself may be garbage-collected later by iroh-blobs.
pub(super) async fn hard_delete_handler(
    State(state): State<AppState>,
    Json(req): Json<DeleteRequest>,
) -> Response {
    tracing::info!("[routes] hard_delete_handler: name={}", req.name);
    let ops = FileOps::new(&state.store, &state.tag_store);
    match ops.delete(&req.name).await {
        Ok(outcome) => {
            tracing::info!(
                "[routes] File hard-deleted: name={} (metadata={}, archives={})",
                req.name,
                outcome.metadata_removed,
                outcome.archives_removed
            );
            Json(DeleteResponse {
                name: req.name,
                hard: true,
            })
            .into_response()
        }
        Err(FileOpError::EmptyName) => {
            (StatusCode::BAD_REQUEST, "File name cannot be empty").into_response()
        }
        Err(err) => {
            tracing::error!("[routes] hard delete failed: {err}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to delete file").into_response()
        }
    }
}

/// Convert a `ProseMirror` document JSON to bytes in the appropriate file format.
///
/// The format is determined by the file name extension:
/// - `.pm.json` → `ProseMirror` JSON (serialized as-is)
/// - `.md` → Markdown (via `prosemirror_to_markdown`)
/// - `.txt` → Plain text (loss-free recursive extraction; see
///   `extract_plain_text` for the per-block rendering policy)
/// - Other text files → Raw text (extracted from `code_block` nodes)
pub(super) fn convert_doc_to_bytes(name: &str, doc: &serde_json::Value) -> Result<Vec<u8>, String> {
    let mode = detect_mode(name);

    match mode {
        ContentMode::Rich => {
            // .pm.json files: store the ProseMirror JSON directly
            serde_json::to_vec_pretty(doc).map_err(|e| format!("Failed to serialize JSON: {e}"))
        }
        ContentMode::Markdown => {
            // .md files: convert PM doc to markdown
            let markdown = prosemirror_to_markdown(doc)
                .map_err(|e| format!("Failed to convert to markdown: {e}"))?;
            Ok(markdown.into_bytes())
        }
        ContentMode::Plain => {
            // .txt files: loss-free plain-text extraction (#1306)
            let text = extract_plain_text(doc);
            Ok(text.into_bytes())
        }
        ContentMode::Raw => {
            // Code/config files: extract text from code_block nodes
            let text = extract_raw_text(doc);
            Ok(text.into_bytes())
        }
        ContentMode::Binary | ContentMode::Media(_) => {
            Err("Cannot save binary/media files from editor".to_owned())
        }
    }
}

/// Extract plain text from a `ProseMirror` doc — loss-free (#1306).
///
/// Plain-text rendering policy (also documented on `convert_doc_to_bytes`):
/// every block contributes its text, one block per line, joined with
/// `"\n"`:
///
/// - `paragraph` / `heading`: their inline text (see
///   `extract_inline_text`); byte-identical to the pre-#1306 output for
///   docs containing only paragraphs and headings.
/// - Lists (`bullet_list` / `ordered_list` / `task_list`): each item
///   contributes its blocks, one item per line for the usual
///   single-paragraph item; nested lists recurse the same way.
/// - `blockquote`: its blocks, recursively.
/// - `code_block`: its raw text verbatim (internal newlines preserved).
/// - `table`: each cell contributes one line, row-major (rows in order,
///   cells in order within a row); a multi-block cell joins its blocks
///   with a single space so the one-line-per-cell shape holds.
/// - `horizontal_rule`: no text — contributes one empty line, acting as
///   a separator between the blocks around it.
/// - Any other block type: recursed into rather than dropped, so new
///   node types degrade to their text instead of vanishing.
///
/// Inline content keeps `text` and `hard_break` (as a newline) and
/// recurses into any other inline node's `content` rather than
/// dropping it. The pre-#1306 version handled only top-level
/// paragraphs/headings and silently deleted everything else on save.
///
/// Line endings: plain-mode load (`plain_text_to_prosemirror`) strips
/// the CR of CRLF endings and this join emits LF only, so a CRLF file
/// is normalized to LF on save - a deliberate, documented choice
/// (#1307), pinned by the round-trip tests below; the file's
/// trailing-newline state is preserved through the load/save pair.
/// Byte-exact CRLF round-trips remain available in Raw mode.
///
/// Inline atoms carry no `content`, so recursion alone loses them.
/// Pinned representations (#1491): an `image` contributes its `alt`
/// text verbatim when non-empty, otherwise its `src` verbatim when
/// non-empty, otherwise nothing; a `front_matter` block contributes
/// its `text` attr verbatim (#5639). Known remaining gap: `footnote_reference`
/// atoms are not yet rendered — they contribute nothing; that is
/// explicitly out of scope for the #1491 fix.
pub(super) fn extract_plain_text(doc: &serde_json::Value) -> String {
    let mut lines = Vec::new();
    if let Some(content) = doc.get("content").and_then(|c| c.as_array()) {
        collect_plain_text_lines(content, &mut lines);
    }
    lines.join("\n")
}

/// Append the plain-text lines for a list of block nodes to `lines`.
pub(super) fn collect_plain_text_lines(nodes: &[serde_json::Value], lines: &mut Vec<String>) {
    for node in nodes {
        let node_type = node.get("type").and_then(|t| t.as_str()).unwrap_or("");
        let content = node.get("content").and_then(|c| c.as_array());
        match node_type {
            "paragraph" | "heading" => lines.push(extract_inline_text(node)),
            "code_block" => lines.push(extract_inline_text(node)),
            "horizontal_rule" => lines.push(String::new()),
            "table" => {
                if let Some(rows) = content {
                    for row in rows {
                        if let Some(cells) = row.get("content").and_then(|c| c.as_array()) {
                            for cell in cells {
                                let mut cell_lines = Vec::new();
                                if let Some(blocks) = cell.get("content").and_then(|c| c.as_array())
                                {
                                    collect_plain_text_lines(blocks, &mut cell_lines);
                                }
                                lines.push(cell_lines.join(" "));
                            }
                        }
                    }
                }
            }
            // Front matter (#1369) carries no PM child content: its
            // payload is the `text` attr, fences included, so the
            // catch-all below (which needs a content array or a
            // top-level text field) would drop the whole block - the
            // block-level twin of the #1491 image-atom loss (#5639).
            // Render it verbatim as one block entry: the authored
            // text's trailing newline is the block terminator the
            // line join re-adds.
            "front_matter" => {
                if let Some(text) = node
                    .get("attrs")
                    .and_then(|attrs| attrs.get("text"))
                    .and_then(|text| text.as_str())
                    && !text.is_empty()
                {
                    lines.push(text.strip_suffix("\n").unwrap_or(text).to_owned());
                }
            }
            // Lists, list items, blockquote, table rows/cells reached
            // outside a table, and every other container block: recurse
            // into the block content so nothing is dropped.
            _ => {
                if let Some(blocks) = content {
                    if blocks.iter().any(|b| {
                        b.get("text").is_some()
                            || b.get("type").and_then(|t| t.as_str()) == Some("hard_break")
                    }) {
                        lines.push(extract_inline_text(node));
                    } else {
                        collect_plain_text_lines(blocks, lines);
                    }
                } else if let Some(text) = node.get("text").and_then(|t| t.as_str()) {
                    lines.push(text.to_owned());
                }
            }
        }
    }
}

/// Extract the inline text of one node: `text` verbatim, `hard_break`
/// as a newline, and any other inline node recursed into (its own
/// inline content contributes its text instead of being dropped).
/// The `image` atom has no content — its payload is `attrs`, rendered
/// per the pinned policy on `extract_plain_text` (#1491).
pub(super) fn extract_inline_text(node: &serde_json::Value) -> String {
    let mut out = String::new();
    if let Some(content) = node.get("content").and_then(|c| c.as_array()) {
        for inline in content {
            let inline_type = inline.get("type").and_then(|t| t.as_str()).unwrap_or("");
            match inline_type {
                "text" => {
                    if let Some(text) = inline.get("text").and_then(|t| t.as_str()) {
                        out.push_str(text);
                    }
                }
                "hard_break" => out.push('\n'),
                "image" => {
                    let attrs = inline.get("attrs");
                    let alt = attrs
                        .and_then(|a| a.get("alt"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if !alt.is_empty() {
                        out.push_str(alt);
                    } else if let Some(src) =
                        attrs.and_then(|a| a.get("src")).and_then(|v| v.as_str())
                    {
                        out.push_str(src);
                    }
                }
                _ => out.push_str(&extract_inline_text(inline)),
            }
        }
    }
    out
}

/// Extract raw text from a `ProseMirror` doc with `code_block` nodes.
///
/// Used for code and config files where the content is a single `code_block`.
///
/// Text nodes within one `code_block` are concatenated with NO separator:
/// the editor and collab layer split a block's text into multiple nodes
/// (at cursor/selection boundaries, marks, or peer edit points), and those
/// splits are not line breaks — joining them with "\n" inserted spurious
/// lines on Raw-mode save (#932). A single "\n" separates distinct
/// `code_block`s, matching how the blocks were authored.
pub(super) fn extract_raw_text(doc: &serde_json::Value) -> String {
    let mut blocks = Vec::new();

    if let Some(content) = doc.get("content").and_then(|c| c.as_array()) {
        for node in content {
            let node_type = node.get("type").and_then(|t| t.as_str()).unwrap_or("");
            if node_type == "code_block"
                && let Some(inline_content) = node.get("content").and_then(|c| c.as_array())
            {
                let mut block = String::new();
                for inline in inline_content {
                    if let Some(text) = inline.get("text").and_then(|t| t.as_str()) {
                        block.push_str(text);
                    }
                }
                blocks.push(block);
            }
        }
    }

    blocks.join("\n")
}

/// Archive the original tag by creating a timestamped copy.
///
/// If a tag with the given name exists and points to the expected hash,
/// creates a new tag `{name}.archive.{timestamp}` pointing to the same hash.
pub(super) async fn archive_original_tag(
    store: &iroh_blobs::api::Store,
    tag_store: &TagStore,
    name: &str,
    expected_hash: &str,
) -> Option<String> {
    use futures_lite::StreamExt;

    let expected: iroh_blobs::Hash = expected_hash.parse().ok()?;

    // Find the current tag
    let mut tags = store.tags().list().await.ok()?;
    let mut found = false;
    while let Some(Ok(tag_info)) = tags.next().await {
        let tag_name = String::from_utf8_lossy(tag_info.name.as_ref()).to_string();
        if tag_name == name && tag_info.hash == expected {
            found = true;
            break;
        }
    }

    if !found {
        return None;
    }

    // Create archive tag with timestamp
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Bump past an existing archive tag so two archives within the same
    // second never overwrite each other (#1612).
    // Claim (resolve + set) in one critical section: resolving a free
    // name first and setting it afterwards was check-then-act, so
    // concurrent same-second archives could collide (#2462).
    match claim_archive_name(store, name, timestamp, expected).await {
        Ok(archive_name) => {
            // The archive alias keeps the source's recorded provenance
            // (#5383), so listings report its real origin and origin
            // filtering finds it.
            if let Err(err) = snapshot_provenance_tags(tag_store, name, &archive_name).await {
                tracing::warn!(
                    "[routes] Failed to snapshot provenance {name} -> {archive_name}: {err:#}"
                );
            }
            tracing::info!(
                "[routes] Archived original: {} -> {}",
                archive_name,
                expected_hash
            );
            Some(archive_name)
        }
        Err(err) => {
            tracing::error!("[routes] Failed to create archive tag: {}", err);
            None
        }
    }
}
