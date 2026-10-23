//! CLI command handler for metadata tag operations.
//!
//! Provides `id tag {set,del,list,search}` subcommands that mirror the REPL
//! tag commands 1:1 — both call the same `run_*` functions in this module
//! through the [`MetaRequester`] trait.
//!
//! # Connection strategy
//!
//! [`MetaClient::open`] connects to a running local `id serve` over the meta
//! protocol if there is one. Otherwise the stores are opened directly
//! ([`crate::local::LocalNode`]) and the request is answered in-process by the
//! same handler, so tags are identical whether or not a server is running.
//!
//! # Binary values
//!
//! Tag values are arbitrary bytes. Input: a positional UTF-8 value, or
//! `--value-hex <HEX>` / `--value-file <PATH>` (`-` for stdin) for raw bytes.
//! Output (`list`/`search`): UTF-8 values print as text; non-UTF-8 values print
//! as `<binary N bytes>`, or as `0x<hex>` with `--hex`, or as lossy text with
//! `--binary`. Long values truncate at [`crate::tags::TAG_DISPLAY_MAX_BYTES`]
//! on a character boundary unless `--no-truncate` is given.
//!
//! # Examples
//!
//! ```bash
//! id tag set README.md priority high
//! id tag set README.md checksum --value-hex deadbeef
//! id tag del README.md priority high   # exact value
//! id tag del README.md priority        # every value of the key
//! id tag list
//! id tag list README.md --hex
//! id tag search priority high
//! ```

use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::cli::TagCommand;
use crate::meta_client::{MetaClient, MetaRequester, expect_ok};
use crate::protocol::{MetaRequest, MetaResponse, WireTag};
use crate::tags::TAG_DISPLAY_MAX_BYTES;

/// Options controlling how tag values are displayed.
#[derive(Debug, Clone, Default)]
pub struct TagDisplayOptions {
    /// Show non-UTF-8 values as `0x<hex>`.
    pub hex: bool,
    /// Show non-UTF-8 values as lossy text instead of a `<binary N bytes>` placeholder.
    pub binary: bool,
    /// Don't truncate long values.
    pub no_truncate: bool,
}

/// Lowercase hex encoding of `bytes`.
pub fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Decode a hex string (case-insensitive, optional `0x` prefix, whitespace ignored).
pub fn hex_decode(input: &str) -> Result<Vec<u8>> {
    let cleaned: String = input
        .trim()
        .trim_start_matches("0x")
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    if !cleaned.len().is_multiple_of(2) {
        bail!("hex value must have an even number of digits");
    }
    cleaned
        .as_bytes()
        .chunks(2)
        .map(|pair| {
            let hi = (pair[0] as char).to_digit(16);
            let lo = (pair[1] as char).to_digit(16);
            match (hi, lo) {
                (Some(h), Some(l)) => u8::try_from(h * 16 + l).context("hex digit out of range"),
                _ => bail!("invalid hex digit in {input:?}"),
            }
        })
        .collect()
}

/// Truncate `s` to at most `max` bytes **on a character boundary**, appending `…`.
pub fn truncate_on_char_boundary(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_owned();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

impl TagDisplayOptions {
    /// Format a tag value for display according to these options.
    pub fn format_value(&self, value: &[u8]) -> String {
        let max = TAG_DISPLAY_MAX_BYTES;
        match std::str::from_utf8(value) {
            Ok(text) => {
                if self.no_truncate {
                    text.to_owned()
                } else {
                    truncate_on_char_boundary(text, max)
                }
            }
            Err(_) if self.hex => {
                if self.no_truncate || value.len() <= max / 2 {
                    format!("0x{}", hex_encode(value))
                } else {
                    format!("0x{}…", hex_encode(&value[..max / 2]))
                }
            }
            Err(_) if self.binary => {
                let lossy = String::from_utf8_lossy(value);
                if self.no_truncate {
                    lossy.into_owned()
                } else {
                    truncate_on_char_boundary(&lossy, max)
                }
            }
            Err(_) => format!("<binary {} bytes>", value.len()),
        }
    }
}

/// Execute a metadata tag subcommand.
///
/// Opens a [`MetaClient`] (server if running, else local stores), runs the
/// subcommand, and closes the client.
pub async fn cmd_tag(cmd: TagCommand) -> Result<()> {
    let mut client = MetaClient::open().await?;
    let result = run_command(&mut client, cmd).await;
    let closed = client.close().await;
    result?;
    closed
}

/// Run a parsed tag subcommand against any [`MetaRequester`] (CLI and REPL).
pub async fn run_command(client: &mut impl MetaRequester, cmd: TagCommand) -> Result<()> {
    match cmd {
        TagCommand::Set {
            file,
            key,
            value,
            value_hex,
            value_file,
        } => {
            let bytes = resolve_value(value, value_hex, value_file.as_deref())?;
            run_set(
                client,
                &file,
                &key,
                bytes.as_deref(),
                &TagDisplayOptions::default(),
            )
            .await
        }
        TagCommand::Del {
            file,
            key,
            value,
            value_hex,
        } => {
            let bytes = resolve_value(value, value_hex, None)?;
            run_del(client, &file, &key, bytes.as_deref()).await
        }
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
            run_list(client, file.as_deref(), &opts).await
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
            run_search(client, &query.join(" "), &opts).await
        }
    }
}

/// Resolve the mutually exclusive value inputs into raw bytes.
fn resolve_value(
    text: Option<String>,
    hex: Option<String>,
    file: Option<&Path>,
) -> Result<Option<Vec<u8>>> {
    match (text, hex, file) {
        (None, None, None) => Ok(None),
        (Some(t), None, None) => Ok(Some(t.into_bytes())),
        (None, Some(h), None) => hex_decode(&h).map(Some),
        (None, None, Some(p)) => {
            let bytes = if p == Path::new("-") {
                use std::io::Read;
                let mut buf = Vec::new();
                std::io::stdin().read_to_end(&mut buf)?;
                buf
            } else {
                std::fs::read(p).with_context(|| format!("reading {}", p.display()))?
            };
            Ok(Some(bytes))
        }
        _ => bail!("give at most one of VALUE, --value-hex and --value-file"),
    }
}

/// Set a metadata tag on a file.
pub async fn run_set(
    client: &mut impl MetaRequester,
    subject: &str,
    key: &str,
    value: Option<&[u8]>,
    opts: &TagDisplayOptions,
) -> Result<()> {
    let resp = expect_ok(
        client
            .request(MetaRequest::SetTag {
                subject: subject.as_bytes().to_vec(),
                key: key.as_bytes().to_vec(),
                value: value.map(<[u8]>::to_vec),
            })
            .await?,
    )?;
    match resp {
        MetaResponse::SetTag { success: true } => {
            match value {
                Some(v) => println!("tag set: {subject} [{key}={}]", opts.format_value(v)),
                None => println!("tag set: {subject} [{key}]"),
            }
            Ok(())
        }
        MetaResponse::SetTag { success: false } => bail!("failed to set tag"),
        _ => bail!("unexpected response"),
    }
}

/// Delete tags from a file.
///
/// With a value, only that exact `(key, value)` tag is deleted. Without one,
/// **every** tag with this key is deleted.
pub async fn run_del(
    client: &mut impl MetaRequester,
    subject: &str,
    key: &str,
    value: Option<&[u8]>,
) -> Result<()> {
    let resp = expect_ok(
        client
            .request(MetaRequest::DelTag {
                subject: subject.as_bytes().to_vec(),
                key: key.as_bytes().to_vec(),
                value: value.map(<[u8]>::to_vec),
            })
            .await?,
    )?;
    match resp {
        MetaResponse::DelTag {
            success: true,
            deleted,
        } => {
            println!("tag deleted: {subject} [{key}] ({deleted} removed)");
            Ok(())
        }
        MetaResponse::DelTag { success: false, .. } => {
            bail!("no matching tag: {subject} [{key}]")
        }
        _ => bail!("unexpected response"),
    }
}

/// List metadata tags (all or for one file).
pub async fn run_list(
    client: &mut impl MetaRequester,
    subject: Option<&str>,
    opts: &TagDisplayOptions,
) -> Result<()> {
    let resp = expect_ok(
        client
            .request(MetaRequest::GetTags {
                subject: subject.map(|s| s.as_bytes().to_vec()),
            })
            .await?,
    )?;
    let MetaResponse::GetTags { tags } = resp else {
        bail!("unexpected response");
    };
    if tags.is_empty() {
        match subject {
            Some(s) => println!("(no tags for {s})"),
            None => println!("(no tags)"),
        }
    } else {
        print_tags(&tags, opts);
    }
    Ok(())
}

/// Search metadata tags with the structured query syntax.
pub async fn run_search(
    client: &mut impl MetaRequester,
    query: &str,
    opts: &TagDisplayOptions,
) -> Result<()> {
    let resp = expect_ok(
        client
            .request(MetaRequest::SearchTags {
                query: query.to_owned(),
            })
            .await?,
    )?;
    let MetaResponse::SearchTags { tags } = resp else {
        bail!("unexpected response");
    };
    if tags.is_empty() {
        println!("(no matching tags)");
    } else {
        print_tags(&tags, opts);
    }
    Ok(())
}

/// Add `name`/`file` auto-tags to every blob name that lacks them.
pub async fn run_migrate(client: &mut impl MetaRequester) -> Result<()> {
    let resp = expect_ok(client.request(MetaRequest::MigrateTags).await?)?;
    match resp {
        MetaResponse::MigrateTags { migrated } => {
            println!("migrated {migrated} file(s)");
            Ok(())
        }
        _ => bail!("unexpected response"),
    }
}

/// Migrate all existing blob names to have name/file auto-tags.
pub async fn cmd_migrate_tags() -> Result<()> {
    let mut client = MetaClient::open().await?;
    let result = run_migrate(&mut client).await;
    let closed = client.close().await;
    result?;
    closed
}

fn print_tags(tags: &[WireTag], opts: &TagDisplayOptions) {
    for t in tags {
        print_tag_line(&t.subject, &t.key, t.value.as_deref(), opts);
    }
    println!("{} tag(s)", tags.len());
}

/// Print a single tag line with display options applied.
fn print_tag_line(subject: &[u8], key: &[u8], value: Option<&[u8]>, opts: &TagDisplayOptions) {
    let subject = String::from_utf8_lossy(subject);
    let key = String::from_utf8_lossy(key);
    match value {
        Some(v) => println!("{subject}\t{key}={}", opts.format_value(v)),
        None => println!("{subject}\t{key}"),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn format_value_short_text_unchanged() {
        let opts = TagDisplayOptions::default();
        assert_eq!(opts.format_value(b"hello"), "hello");
    }

    #[test]
    fn format_value_truncates_long_ascii() {
        let opts = TagDisplayOptions::default();
        let long = "a".repeat(TAG_DISPLAY_MAX_BYTES + 10);
        let out = opts.format_value(long.as_bytes());
        assert!(out.ends_with('…'));
        assert_eq!(out.len(), TAG_DISPLAY_MAX_BYTES + '…'.len_utf8());
    }

    #[test]
    fn format_value_never_splits_a_multibyte_char() {
        // 255 ASCII bytes then a 2-byte char straddling the 256-byte limit:
        // slicing at byte 256 used to panic.
        let opts = TagDisplayOptions::default();
        let s = format!("{}é{}", "a".repeat(TAG_DISPLAY_MAX_BYTES - 1), "tail");
        let out = opts.format_value(s.as_bytes());
        assert!(out.ends_with('…'));
        assert!(out.starts_with(&"a".repeat(TAG_DISPLAY_MAX_BYTES - 1)));
        assert!(!out.contains('é'), "char straddling the limit is dropped");
    }

    #[test]
    fn format_value_no_truncate_keeps_everything() {
        let opts = TagDisplayOptions {
            no_truncate: true,
            ..Default::default()
        };
        let long = "é".repeat(TAG_DISPLAY_MAX_BYTES);
        assert_eq!(opts.format_value(long.as_bytes()), long);
    }

    #[test]
    fn binary_values_are_summarised_by_default() {
        let opts = TagDisplayOptions::default();
        assert_eq!(opts.format_value(&[0xff, 0xfe, 0x00]), "<binary 3 bytes>");
    }

    #[test]
    fn hex_flag_renders_binary_as_hex() {
        let opts = TagDisplayOptions {
            hex: true,
            ..Default::default()
        };
        assert_eq!(opts.format_value(&[0xff, 0xfe, 0x00]), "0xfffe00");
    }

    #[test]
    fn hex_flag_truncates_long_binary() {
        let opts = TagDisplayOptions {
            hex: true,
            ..Default::default()
        };
        let data = vec![0xffu8; TAG_DISPLAY_MAX_BYTES];
        let out = opts.format_value(&data);
        assert!(out.starts_with("0xffff"));
        assert!(out.ends_with('…'));
        let no_trunc = TagDisplayOptions {
            hex: true,
            no_truncate: true,
            ..Default::default()
        };
        assert_eq!(no_trunc.format_value(&data).len(), 2 + data.len() * 2);
    }

    #[test]
    fn binary_flag_renders_lossy_text() {
        let opts = TagDisplayOptions {
            binary: true,
            ..Default::default()
        };
        assert_eq!(opts.format_value(&[b'a', 0xff, b'b']), "a\u{fffd}b");
    }

    #[test]
    fn hex_roundtrip() {
        let data = [0u8, 1, 0xab, 0xff];
        assert_eq!(hex_decode(&hex_encode(&data)).unwrap(), data);
        assert_eq!(hex_decode("0xAB cd").unwrap(), vec![0xab, 0xcd]);
    }

    #[test]
    fn hex_decode_rejects_bad_input() {
        assert!(hex_decode("abc").is_err(), "odd length");
        assert!(hex_decode("zz").is_err(), "not hex");
    }

    #[test]
    fn resolve_value_rejects_conflicting_inputs() {
        assert!(resolve_value(Some("a".into()), Some("00".into()), None).is_err());
        assert_eq!(resolve_value(None, None, None).unwrap(), None);
        assert_eq!(
            resolve_value(None, Some("6869".into()), None).unwrap(),
            Some(b"hi".to_vec())
        );
    }

    #[test]
    fn display_options_default() {
        let opts = TagDisplayOptions::default();
        assert!(!opts.hex);
        assert!(!opts.binary);
        assert!(!opts.no_truncate);
    }
}
