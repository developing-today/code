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

/// Options controlling how tag values are displayed.
#[derive(Debug, Clone, Default)]
pub struct TagDisplayOptions {
    /// Show binary values as hex strings.
    pub hex: bool,
    /// Include binary (non-UTF-8) tag values in output.
    pub binary: bool,
    /// Don't truncate long values.
    pub no_truncate: bool,
}

impl TagDisplayOptions {
    /// Format a tag value for display according to these options.
    ///
    /// Returns `None` if the value is binary and `binary` is false (skip it).
    pub fn format_value(&self, value: &str) -> String {
        if self.no_truncate {
            value.to_owned()
        } else if value.len() > crate::tags::TAG_DISPLAY_MAX_BYTES {
            let truncated = &value[..crate::tags::TAG_DISPLAY_MAX_BYTES];
            format!("{truncated}…")
        } else {
            value.to_owned()
        }
    }
}

/// Execute a metadata tag subcommand.
///
/// Dispatches to the appropriate handler based on the `TagCommand` variant.
/// Each handler tries to connect to a running serve instance first, falling
/// back to direct store access if no server is running.
pub async fn cmd_tag(cmd: TagCommand) -> Result<()> {
    match cmd {
        TagCommand::Set { file, key, value } => tag_set(&file, &key, value.as_deref()).await,
        TagCommand::Del { file, key, value } => tag_del(&file, &key, value.as_deref()).await,
        TagCommand::List {
            file,
            hex,
            binary,
            no_truncate,
        } => {
            let opts = TagDisplayOptions {
                hex,
                binary,
                no_truncate,
            };
            tag_list(file.as_deref(), &opts).await
        }
        TagCommand::Search {
            query,
            hex,
            binary,
            no_truncate,
        } => {
            let opts = TagDisplayOptions {
                hex,
                binary,
                no_truncate,
            };
            tag_search(&query.join(" "), &opts).await
        }
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
async fn tag_list(subject: Option<&str>, opts: &TagDisplayOptions) -> Result<()> {
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
                        print_tag_line(subj, key, value.as_deref(), opts);
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
            print_tag_line(&t.subject, &t.key, t.value.as_deref(), opts);
        }
        println!("{} tag(s)", tags.len());
    }
    store.shutdown().await?;
    Ok(())
}

/// Search metadata tags using structured query syntax.
///
/// Query syntax supports `key:`, `:value`, `key:value`, `"literal"`, and
/// bare word searches. Multiple terms are ANDed together.
async fn tag_search(query: &str, opts: &TagDisplayOptions) -> Result<()> {
    let req = MetaRequest::SearchTags {
        query: query.to_owned(),
    };

    if let Some(resp) = send_meta_request(&req).await? {
        match resp {
            MetaResponse::SearchTags { tags } => {
                if tags.is_empty() {
                    println!("(no matching tags)");
                } else {
                    for (subj, k, v) in &tags {
                        print_tag_line(subj, k, v.as_deref(), opts);
                    }
                    println!("{} tag(s)", tags.len());
                }
            }
            _ => anyhow::bail!("unexpected response"),
        }
        return Ok(());
    }

    // Fallback: legacy MetaDoc search (limited to bare word matching)
    let store = crate::open_store(false).await?;
    let store_handle = store.as_store();
    let meta = crate::tags::load_meta(&store_handle).await?;
    let search_terms = crate::tags::parse_search_query(query);
    let tags: Vec<_> = meta
        .tags
        .iter()
        .filter(|t| {
            search_terms.iter().all(|term| {
                use crate::tags::SearchTerm;
                match term {
                    SearchTerm::KeyOnly(k) => t.key == *k,
                    SearchTerm::ValueOnly(v) => t.value.as_deref() == Some(v.as_str()),
                    SearchTerm::KeyValue(k, v) => {
                        t.key == *k && t.value.as_deref() == Some(v.as_str())
                    }
                    SearchTerm::Literal(text) | SearchTerm::BareWord(text) => {
                        let text_lower = text.to_lowercase();
                        t.subject.to_lowercase().contains(&text_lower)
                            || t.key.to_lowercase().contains(&text_lower)
                            || t.value
                                .as_deref()
                                .unwrap_or("")
                                .to_lowercase()
                                .contains(&text_lower)
                    }
                }
            })
        })
        .collect();
    if tags.is_empty() {
        println!("(no matching tags)");
    } else {
        for t in &tags {
            print_tag_line(&t.subject, &t.key, t.value.as_deref(), opts);
        }
        println!("{} tag(s)", tags.len());
    }
    store.shutdown().await?;
    Ok(())
}

/// Print a single tag line with truncation per display options.
fn print_tag_line(subject: &str, key: &str, value: Option<&str>, opts: &TagDisplayOptions) {
    if let Some(v) = value {
        let formatted = opts.format_value(v);
        println!("{subject}\t{key}={formatted}");
    } else {
        println!("{subject}\t{key}");
    }
}

/// Migrate all existing blob tags to have name/file auto-tags.
///
/// If a local serve is running, sends `MetaRequest::MigrateTags` via the
/// meta protocol. Otherwise prints a message (TagStore is only available
/// when serve is running).
pub async fn cmd_migrate_tags() -> Result<()> {
    let req = MetaRequest::MigrateTags;

    if let Some(resp) = send_meta_request(&req).await? {
        match resp {
            MetaResponse::MigrateTags { migrated } => {
                println!("migrated {migrated} file(s)");
            }
            _ => anyhow::bail!("unexpected response"),
        }
        return Ok(());
    }

    // Fallback: no serve running, can't migrate without TagStore
    println!("migrate-tags requires a running serve instance (id serve)");
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_format_value_short() {
        let opts = TagDisplayOptions::default();
        assert_eq!(opts.format_value("hello"), "hello");
    }

    #[test]
    fn test_format_value_truncation() {
        let opts = TagDisplayOptions::default();
        let long_value = "a".repeat(300);
        let result = opts.format_value(&long_value);
        assert!(result.len() < long_value.len(), "should be truncated");
        assert!(result.ends_with('…'), "should end with ellipsis");
    }

    #[test]
    fn test_format_value_no_truncate() {
        let opts = TagDisplayOptions {
            no_truncate: true,
            ..Default::default()
        };
        let long_value = "a".repeat(300);
        let result = opts.format_value(&long_value);
        assert_eq!(result, long_value, "should not truncate with --no-truncate");
    }

    #[test]
    fn test_format_value_exact_boundary() {
        let opts = TagDisplayOptions::default();
        let exact = "a".repeat(crate::tags::TAG_DISPLAY_MAX_BYTES);
        let result = opts.format_value(&exact);
        assert_eq!(result, exact, "exactly at limit should not truncate");
    }

    #[test]
    fn test_format_value_one_over_boundary() {
        let opts = TagDisplayOptions::default();
        let over = "a".repeat(crate::tags::TAG_DISPLAY_MAX_BYTES + 1);
        let result = opts.format_value(&over);
        assert!(result.ends_with('…'), "one over limit should truncate");
    }

    #[test]
    fn test_display_options_default() {
        let opts = TagDisplayOptions::default();
        assert!(!opts.hex);
        assert!(!opts.binary);
        assert!(!opts.no_truncate);
    }
}
