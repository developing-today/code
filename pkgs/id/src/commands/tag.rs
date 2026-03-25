//! CLI command handler for metadata tag operations.
//!
//! Provides `id tag {set,del,list,search}` subcommands that mirror the REPL
//! tag commands with 1:1 feature parity.
//!
//! # Connection strategy
//!
//! 1. If a local `id serve` is running (detected via lock file), connect via
//!    the meta protocol and use the server's `TagStore` (iroh-docs backed).
//! 2. Otherwise, fall back to direct store access with the legacy `MetaDoc`
//!    blob-based metadata system.
//!
//! # Examples
//!
//! ```bash
//! id tag set README.md priority high
//! id tag del README.md priority high
//! id tag list
//! id tag list README.md
//! id tag search priority
//! id tag search priority high
//! ```

use anyhow::Result;

use crate::META_ALPN;
use crate::cli::TagCommand;
use crate::commands::client::create_local_client_endpoint;
use crate::commands::serve::get_serve_info;
use crate::protocol::{MetaRequest, MetaResponse};

/// Execute a metadata tag subcommand.
///
/// Dispatches to the appropriate handler based on the `TagCommand` variant.
/// Each handler tries to connect to a running serve instance first, falling
/// back to direct store access if no server is running.
pub async fn cmd_tag(cmd: TagCommand) -> Result<()> {
    match cmd {
        TagCommand::Set { file, key, value } => tag_set(&file, &key, value.as_deref()).await,
        TagCommand::Del { file, key, value } => tag_del(&file, &key, value.as_deref()).await,
        TagCommand::List { file } => tag_list(file.as_deref()).await,
        TagCommand::Search { key, value } => tag_search(&key, value.as_deref()).await,
    }
}

/// Helper: send a `MetaRequest` to the running serve and return the response.
///
/// Returns `None` if no serve is running.
async fn send_meta_request(req: &MetaRequest) -> Result<Option<MetaResponse>> {
    let Some(serve_info) = get_serve_info().await else {
        return Ok(None);
    };
    let (endpoint, endpoint_addr) = create_local_client_endpoint(&serve_info).await?;
    let conn = endpoint.connect(endpoint_addr, META_ALPN).await?;
    let (mut send, mut recv) = conn.open_bi().await?;
    let req_bytes = postcard::to_allocvec(req)?;
    send.write_all(&req_bytes).await?;
    send.finish()?;
    let resp_buf = recv.read_to_end(1024 * 1024).await?;
    let resp: MetaResponse = postcard::from_bytes(&resp_buf)?;
    conn.close(0u32.into(), b"done");
    endpoint.close().await;
    Ok(Some(resp))
}

/// Set a metadata tag on a file.
async fn tag_set(subject: &str, key: &str, value: Option<&str>) -> Result<()> {
    let req = MetaRequest::SetTag {
        subject: subject.to_owned(),
        key: key.to_owned(),
        value: value.map(ToOwned::to_owned),
    };

    if let Some(resp) = send_meta_request(&req).await? {
        match resp {
            MetaResponse::SetTag { success } => {
                if success {
                    if let Some(v) = value {
                        println!("tag set: {subject} [{key}={v}]");
                    } else {
                        println!("tag set: {subject} [{key}]");
                    }
                } else {
                    println!("failed to set tag (server has no TagStore)");
                }
            }
            _ => anyhow::bail!("unexpected response"),
        }
        return Ok(());
    }

    // Fallback: direct store access with legacy MetaDoc
    let store = crate::open_store(false).await?;
    let store_handle = store.as_store();
    let mut meta = crate::tags::load_meta(&store_handle).await?;
    let now = crate::tags::now_unix();
    crate::tags::add_tag(&mut meta, subject, key, value, None);
    crate::tags::save_meta(&store_handle, &meta).await?;
    if let Some(v) = value {
        println!("tag set: {subject} [{key}={v}] (legacy, ts={now})");
    } else {
        println!("tag set: {subject} [{key}] (legacy, ts={now})");
    }
    store.shutdown().await?;
    Ok(())
}

/// Delete a metadata tag from a file.
async fn tag_del(subject: &str, key: &str, value: Option<&str>) -> Result<()> {
    let req = MetaRequest::DelTag {
        subject: subject.to_owned(),
        key: key.to_owned(),
        value: value.map(ToOwned::to_owned),
    };

    if let Some(resp) = send_meta_request(&req).await? {
        match resp {
            MetaResponse::DelTag { success } => {
                if success {
                    println!("tag deleted: {subject} [{key}]");
                } else {
                    println!("failed to delete tag");
                }
            }
            _ => anyhow::bail!("unexpected response"),
        }
        return Ok(());
    }

    // Fallback: legacy MetaDoc — not supported for delete in local mode
    println!("tag delete not supported in local mode (no TagStore)");
    Ok(())
}

/// List metadata tags (all or for a specific file).
async fn tag_list(subject: Option<&str>) -> Result<()> {
    let req = MetaRequest::GetTags {
        subject: subject.map(ToOwned::to_owned),
    };

    if let Some(resp) = send_meta_request(&req).await? {
        match resp {
            MetaResponse::GetTags { tags } => {
                if tags.is_empty() {
                    if let Some(s) = subject {
                        println!("(no tags for {s})");
                    } else {
                        println!("(no tags)");
                    }
                } else {
                    for (subj, key, value) in &tags {
                        if let Some(v) = value {
                            println!("{subj}\t{key}={v}");
                        } else {
                            println!("{subj}\t{key}");
                        }
                    }
                    println!("{} tag(s)", tags.len());
                }
            }
            _ => anyhow::bail!("unexpected response"),
        }
        return Ok(());
    }

    // Fallback: legacy MetaDoc
    let store = crate::open_store(false).await?;
    let store_handle = store.as_store();
    let meta = crate::tags::load_meta(&store_handle).await?;
    let tags: Vec<_> = if let Some(subj) = subject {
        meta.tags.iter().filter(|t| t.subject == subj).collect()
    } else {
        meta.tags.iter().collect()
    };
    if tags.is_empty() {
        if let Some(s) = subject {
            println!("(no tags for {s})");
        } else {
            println!("(no tags)");
        }
    } else {
        for t in &tags {
            if let Some(v) = &t.value {
                println!("{}\t{}={v}", t.subject, t.key);
            } else {
                println!("{}\t{}", t.subject, t.key);
            }
        }
        println!("{} tag(s)", tags.len());
    }
    store.shutdown().await?;
    Ok(())
}

/// Search metadata tags by key and/or value.
async fn tag_search(key: &str, value: Option<&str>) -> Result<()> {
    let req = MetaRequest::SearchTags {
        key: Some(key.to_owned()),
        value: value.map(ToOwned::to_owned),
    };

    if let Some(resp) = send_meta_request(&req).await? {
        match resp {
            MetaResponse::SearchTags { tags } => {
                if tags.is_empty() {
                    println!("(no matching tags)");
                } else {
                    for (subj, k, v) in &tags {
                        if let Some(val) = v {
                            println!("{subj}\t{k}={val}");
                        } else {
                            println!("{subj}\t{k}");
                        }
                    }
                    println!("{} tag(s)", tags.len());
                }
            }
            _ => anyhow::bail!("unexpected response"),
        }
        return Ok(());
    }

    // Fallback: legacy MetaDoc search
    let store = crate::open_store(false).await?;
    let store_handle = store.as_store();
    let meta = crate::tags::load_meta(&store_handle).await?;
    let tags: Vec<_> = meta
        .tags
        .iter()
        .filter(|t| {
            let key_match = t.key == key;
            let value_match = value.is_none_or(|v| t.value.as_deref() == Some(v));
            key_match && value_match
        })
        .collect();
    if tags.is_empty() {
        println!("(no matching tags)");
    } else {
        for t in &tags {
            if let Some(v) = &t.value {
                println!("{}\t{}={v}", t.subject, t.key);
            } else {
                println!("{}\t{}", t.subject, t.key);
            }
        }
        println!("{} tag(s)", tags.len());
    }
    store.shutdown().await?;
    Ok(())
}
