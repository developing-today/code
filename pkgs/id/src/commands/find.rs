//! Find and search command handlers for locating blobs by pattern matching.
//!
//! This module provides fuzzy search capabilities for the blob store, allowing
//! users to locate files by partial name or hash matches. It supports both
//! local and remote searching.
//!
//! # Commands
//!
//! - **find**: Search and retrieve matching files (outputs content by default)
//! - **search**: Search and list matches (metadata only, optionally retrieve)
//! - **show/view**: Find and output content to stdout (cat over find)
//! - **peek**: Preview files with head/tail display
//!
//! # Match Types
//!
//! The search algorithm recognizes three types of matches, in priority order:
//!
//! 1. **Exact**: Query exactly matches the name or hash
//! 2. **Prefix**: Name or hash starts with the query
//! 3. **Contains**: Name or hash contains the query anywhere
//!
//! # Output Formats
//!
//! Both commands support multiple output formats via `--format`:
//!
//! - **tag**: Shows query tag with each match
//! - **group**: Groups matches by query
//! - **union**: Default format, shows all matches with query suffix
//!
//! # Filtering and Limiting
//!
//! - `--first N`: Return only the first N matches
//! - `--last N`: Return only the last N matches
//! - `--count`: Print count instead of matches
//! - `--exclude PATTERN`: Exclude matches containing pattern (repeatable)
//!
//! # Architecture
//!
//! ```text
//! ┌────────────────────────────────────────────────────────────────┐
//! │                    cmd_find / cmd_search                       │
//! │            (CLI handlers with output formatting)               │
//! └────────────────────────────────────────────────────────────────┘
//!                               │
//!                               ▼
//! ┌────────────────────────────────────────────────────────────────┐
//! │                     cmd_find_matches                           │
//! │     (core search logic, works locally or via remote node)      │
//! └────────────────────────────────────────────────────────────────┘
//!                    │                        │
//!                    ▼                        ▼
//!            ┌──────────────┐         ┌──────────────┐
//!            │ Local Store  │         │ Remote Node  │
//!            │ (tags list)  │         │ (MetaRequest)│
//!            └──────────────┘         └──────────────┘
//! ```
//!
//! # Examples
//!
//! Find files matching "config":
//! ```bash
//! id find config
//! ```
//!
//! Search for multiple patterns, output all to a directory:
//! ```bash
//! id search "*.json" "*.toml" --all --dir ./configs
//! ```
//!
//! Search on a remote node:
//! ```bash
//! id find config --node <NODE_ID>
//! ```

use anyhow::{Result, bail};
use futures_lite::StreamExt;
use iroh::{
    address_lookup::{DnsAddressLookup, PkarrPublisher},
    endpoint::{Endpoint, RelayMode},
};
use iroh_base::EndpointId;

use crate::{
    CLIENT_KEY_FILE, FindMatch, META_ALPN, MetaRequest, MetaResponse, TaggedMatch, cmd_get_one,
    cmd_get_one_remote, load_or_create_keypair, match_kind, open_store, print_match_cli,
    print_matches_cli,
};

/// Options for filtering and limiting search results.
#[derive(Debug, Clone, Default)]
pub struct SearchOptions {
    /// Return only the first N matches.
    pub first: Option<usize>,
    /// Return only the last N matches.
    pub last: Option<usize>,
    /// Print count instead of matches.
    pub count: bool,
    /// Exclude matches where name or hash contains any of these patterns.
    pub exclude: Vec<String>,
}

impl SearchOptions {
    /// Creates a new `SearchOptions` with the given parameters.
    pub const fn new(
        first: Option<usize>,
        last: Option<usize>,
        count: bool,
        exclude: Vec<String>,
    ) -> Self {
        Self {
            first,
            last,
            count,
            exclude,
        }
    }

    /// Checks if a match should be excluded based on the exclude patterns.
    pub fn should_exclude(&self, name: &str, hash_str: &str) -> bool {
        let name_lower = name.to_lowercase();
        let hash_lower = hash_str.to_lowercase();
        for pattern in &self.exclude {
            let pattern_lower = pattern.to_lowercase();
            if name_lower.contains(&pattern_lower) || hash_lower.contains(&pattern_lower) {
                return true;
            }
        }
        false
    }

    /// Applies filtering and limiting to a list of matches.
    pub fn apply(&self, matches: Vec<TaggedMatch>) -> Vec<TaggedMatch> {
        // First, apply exclusions
        let filtered: Vec<TaggedMatch> = matches
            .into_iter()
            .filter(|m| !self.should_exclude(&m.name, &m.hash.to_string()))
            .collect();

        // Then apply first/last limiting

        if let Some(n) = self.first {
            filtered.into_iter().take(n).collect()
        } else if let Some(n) = self.last {
            let len = filtered.len();
            if n >= len {
                filtered
            } else {
                filtered.into_iter().skip(len - n).collect()
            }
        } else {
            filtered
        }
    }
}

/// Find files matching queries and output their content.
///
/// This is the main handler for the `id find` command. It searches for blobs
/// matching the given queries and outputs their content. By default, it outputs
/// to stdout; with `--file` or `--dir`, it saves to files.
///
/// # Behavior
///
/// - **Single match**: Outputs directly to stdout or file
/// - **Multiple matches**: Lists all matches with details, uses first match
/// - **`--all` mode**: Outputs all matching files (deduplicated by hash+name)
///
/// # Arguments
///
/// * `queries` - One or more search patterns to match against names/hashes
/// * `prefer_name` - If true, prioritize name matches over hash matches
/// * `to_stdout` - If true, always output content to stdout
/// * `all` - If true, output all matches (not just the first)
/// * `dir` - Optional directory to save all matching files
/// * `format` - Output format: "tag", "group", or "union"
/// * `options` - Search options for filtering and limiting
/// * `node` - Optional remote node ID to search on
/// * `no_relay` - If true, disable relay servers for remote connections
///
/// # Errors
///
/// Returns an error if no matches are found for any query.
///
/// # Example
///
/// ```bash
/// id find config              # Find and output first match
/// id find config --all        # Output all matches to stdout
/// id find "*.json" --dir ./   # Save all JSON files to current directory
/// id find --first 3 config    # First 3 matches
/// id find --count config      # Count matches
/// ```
pub async fn cmd_find(
    queries: Vec<String>,
    prefer_name: bool,
    to_stdout: bool,
    all: bool,
    dir: Option<String>,
    format: &str,
    options: SearchOptions,
    node: Option<String>,
    no_relay: bool,
) -> Result<()> {
    // Collect matches for all queries
    let mut all_matches: Vec<TaggedMatch> = Vec::new();
    for query in &queries {
        let matches = cmd_find_matches(query, prefer_name, node.clone(), no_relay).await?;
        for m in matches {
            all_matches.push(TaggedMatch {
                query: query.clone(),
                hash: m.hash,
                name: m.name,
                kind: m.kind,
                is_hash_match: m.is_hash_match,
            });
        }
    }

    // Apply filtering and limiting
    let all_matches = options.apply(all_matches);

    if all_matches.is_empty() {
        bail!("no matches found for: {}", queries.join(", "));
    }

    // --count mode: just print the count
    if options.count {
        println!("{}", all_matches.len());
        return Ok(());
    }

    // --all mode: output all matches
    if all {
        if let Some(ref dir_path) = dir {
            std::fs::create_dir_all(dir_path)?;
            // Deduplicate by hash+name for file output
            let mut seen = std::collections::HashSet::new();
            for m in &all_matches {
                let key = format!("{}:{}", m.hash, m.name);
                if seen.insert(key) {
                    let output_path = format!("{}/{}", dir_path, m.name);
                    if let Some(ref node_str) = node {
                        let node_id: EndpointId = node_str.parse()?;
                        cmd_get_one_remote(node_id, &m.name, &output_path, no_relay).await?;
                    } else {
                        cmd_get_one(&m.name, &output_path, false, false).await?;
                    }
                    print_match_cli(m, format);
                }
            }
        } else {
            // Output all to stdout (concatenated)
            let mut seen = std::collections::HashSet::new();
            for m in &all_matches {
                let key = format!("{}:{}", m.hash, m.name);
                if seen.insert(key) {
                    if let Some(ref node_str) = node {
                        let node_id: EndpointId = node_str.parse()?;
                        cmd_get_one_remote(node_id, &m.name, "-", no_relay).await?;
                    } else {
                        cmd_get_one(&m.name, "-", false, false).await?;
                    }
                }
            }
        }
        return Ok(());
    }

    // Single match or first match mode
    if all_matches.len() == 1 {
        let m = &all_matches[0];
        let output = if to_stdout { "-" } else { &m.name };
        if let Some(node_str) = node {
            let node_id: EndpointId = node_str.parse()?;
            cmd_get_one_remote(node_id, &m.name, output, no_relay).await?;
        } else {
            cmd_get_one(&m.name, output, false, false).await?;
        }
    } else {
        // Multiple matches - print them and use first one
        eprintln!("found {} matches (using first):", all_matches.len());
        print_matches_cli(&all_matches, format);
        let m = &all_matches[0];
        let output = if to_stdout { "-" } else { &m.name };
        if let Some(node_str) = node {
            let node_id: EndpointId = node_str.parse()?;
            cmd_get_one_remote(node_id, &m.name, output, no_relay).await?;
        } else {
            cmd_get_one(&m.name, output, false, false).await?;
        }
    }
    Ok(())
}

/// Search for files matching queries and list the results.
///
/// This is the main handler for the `id search` command. Unlike `find`, it
/// defaults to listing matches (metadata only) without outputting file content.
/// Use `--all` to also retrieve the matching files.
///
/// # Behavior
///
/// - **Default**: Lists all matches with hash, name, match type, and query
/// - **`--all` mode**: Lists matches and also outputs file content
/// - **`--file`**: After listing, save the first match to a file
///
/// # Arguments
///
/// * `queries` - One or more search patterns to match against names/hashes
/// * `prefer_name` - If true, prioritize name matches over hash matches
/// * `all` - If true, also output file content for all matches
/// * `dir` - Optional directory to save all matching files
/// * `format` - Output format: "tag", "group", or "union"
/// * `options` - Search options for filtering and limiting
/// * `node` - Optional remote node ID to search on
/// * `no_relay` - If true, disable relay servers for remote connections
///
/// # Example
///
/// ```bash
/// id search config            # List all matches
/// id search config --all      # List and output all matches
/// id search "*.json" --dir ./ # List and save all JSON files
/// id search --count config    # Count matches
/// ```
pub async fn cmd_search(
    queries: Vec<String>,
    prefer_name: bool,
    all: bool,
    dir: Option<String>,
    format: &str,
    options: SearchOptions,
    node: Option<String>,
    no_relay: bool,
) -> Result<()> {
    // Collect matches for all queries
    let mut all_matches: Vec<TaggedMatch> = Vec::new();
    for query in &queries {
        let matches = cmd_find_matches(query, prefer_name, node.clone(), no_relay).await?;
        for m in matches {
            all_matches.push(TaggedMatch {
                query: query.clone(),
                hash: m.hash,
                name: m.name,
                kind: m.kind,
                is_hash_match: m.is_hash_match,
            });
        }
    }

    // Apply filtering and limiting
    let all_matches = options.apply(all_matches);

    if all_matches.is_empty() {
        println!("no matches found for: {}", queries.join(", "));
        return Ok(());
    }

    // --count mode: just print the count
    if options.count {
        println!("{}", all_matches.len());
        return Ok(());
    }

    // --all mode: output all files (like find --all)
    if all {
        if let Some(ref dir_path) = dir {
            std::fs::create_dir_all(dir_path)?;
            let mut seen = std::collections::HashSet::new();
            for m in &all_matches {
                let key = format!("{}:{}", m.hash, m.name);
                if seen.insert(key) {
                    let output_path = format!("{}/{}", dir_path, m.name);
                    if let Some(ref node_str) = node {
                        let node_id: EndpointId = node_str.parse()?;
                        cmd_get_one_remote(node_id, &m.name, &output_path, no_relay).await?;
                    } else {
                        cmd_get_one(&m.name, &output_path, false, false).await?;
                    }
                    print_match_cli(m, format);
                }
            }
        } else {
            // Output all to stdout (concatenated)
            let mut seen = std::collections::HashSet::new();
            for m in &all_matches {
                let key = format!("{}:{}", m.hash, m.name);
                if seen.insert(key) {
                    if let Some(ref node_str) = node {
                        let node_id: EndpointId = node_str.parse()?;
                        cmd_get_one_remote(node_id, &m.name, "-", no_relay).await?;
                    } else {
                        cmd_get_one(&m.name, "-", false, false).await?;
                    }
                }
            }
        }
        return Ok(());
    }

    // Default: just list matches
    print_matches_cli(&all_matches, format);
    Ok(())
}

/// Show file content by searching for matches (cat over find).
///
/// This is the handler for the `id show` and `id view` commands. It finds
/// files matching the query and outputs their content to stdout (or a file).
///
/// # Arguments
///
/// * `queries` - Search patterns to match against names/hashes
/// * `prefer_name` - If true, prioritize name matches over hash matches
/// * `all` - If true, output all matches (not just the first)
/// * `output` - Output destination (None = stdout)
/// * `options` - Search options for filtering and limiting
/// * `node` - Optional remote node ID to search on
/// * `no_relay` - If true, disable relay servers for remote connections
///
/// # Example
///
/// ```bash
/// id show config              # Show first match
/// id show --all config        # Show all matches
/// id show -o out.txt config   # Write to file
/// ```
pub async fn cmd_show(
    queries: Vec<String>,
    prefer_name: bool,
    all: bool,
    output: Option<String>,
    options: SearchOptions,
    node: Option<String>,
    no_relay: bool,
) -> Result<()> {
    // Collect matches for all queries
    let mut all_matches: Vec<TaggedMatch> = Vec::new();
    for query in &queries {
        let matches = cmd_find_matches(query, prefer_name, node.clone(), no_relay).await?;
        for m in matches {
            all_matches.push(TaggedMatch {
                query: query.clone(),
                hash: m.hash,
                name: m.name,
                kind: m.kind,
                is_hash_match: m.is_hash_match,
            });
        }
    }

    // Apply filtering and limiting
    let all_matches = options.apply(all_matches);

    if all_matches.is_empty() {
        bail!("no matches found for: {}", queries.join(", "));
    }

    let out_path = output.as_deref().unwrap_or("-");

    if all {
        // Output all matches
        let mut seen = std::collections::HashSet::new();
        for m in &all_matches {
            let key = format!("{}:{}", m.hash, m.name);
            if seen.insert(key) {
                if let Some(ref node_str) = node {
                    let node_id: EndpointId = node_str.parse()?;
                    cmd_get_one_remote(node_id, &m.name, out_path, no_relay).await?;
                } else {
                    cmd_get_one(&m.name, out_path, false, false).await?;
                }
            }
        }
    } else {
        // Output first match only
        let m = &all_matches[0];
        if let Some(ref node_str) = node {
            let node_id: EndpointId = node_str.parse()?;
            cmd_get_one_remote(node_id, &m.name, out_path, no_relay).await?;
        } else {
            cmd_get_one(&m.name, out_path, false, false).await?;
        }
    }

    Ok(())
}

/// Options for the peek command.
#[derive(Debug, Clone)]
pub struct PeekOptions {
    /// Number of lines to show from head and tail.
    pub lines: usize,
    /// Show only head lines (no tail).
    pub head_only: bool,
    /// Show only tail lines (no head).
    pub tail_only: bool,
    /// Count by characters instead of lines.
    pub chars: bool,
    /// Count by words instead of lines.
    pub words: bool,
    /// Quiet mode: no header banner.
    pub quiet: bool,
}

impl Default for PeekOptions {
    fn default() -> Self {
        Self {
            lines: 5,
            head_only: false,
            tail_only: false,
            chars: false,
            words: false,
            quiet: false,
        }
    }
}

/// Preview file content with head/tail display.
///
/// This is the handler for the `id peek` command. It shows a preview of
/// matching files with configurable head and tail lines.
///
/// # Arguments
///
/// * `queries` - Search patterns to match against names/hashes
/// * `prefer_name` - If true, prioritize name matches over hash matches
/// * `all` - If true, peek all matches (not just the first per query)
/// * `output` - Output destination (None = stdout)
/// * `peek_opts` - Peek-specific options (lines, `head_only`, etc.)
/// * `search_opts` - Search options for filtering and limiting
/// * `node` - Optional remote node ID to search on
/// * `no_relay` - If true, disable relay servers for remote connections
///
/// # Example
///
/// ```bash
/// id peek readme              # Preview readme with 5 head + 5 tail
/// id peek --lines 10 readme   # 10 head + 10 tail lines
/// id peek --head-only readme  # Only head lines
/// ```
pub async fn cmd_peek(
    queries: Vec<String>,
    prefer_name: bool,
    all: bool,
    output: Option<String>,
    peek_opts: PeekOptions,
    search_opts: SearchOptions,
    node: Option<String>,
    no_relay: bool,
) -> Result<()> {
    // Collect matches for all queries
    let mut all_matches: Vec<TaggedMatch> = Vec::new();
    for query in &queries {
        let matches = cmd_find_matches(query, prefer_name, node.clone(), no_relay).await?;
        for m in matches {
            all_matches.push(TaggedMatch {
                query: query.clone(),
                hash: m.hash,
                name: m.name,
                kind: m.kind,
                is_hash_match: m.is_hash_match,
            });
        }
    }

    // Apply filtering and limiting
    let all_matches = search_opts.apply(all_matches);

    if all_matches.is_empty() {
        bail!("no matches found for: {}", queries.join(", "));
    }

    // Determine which matches to process (deduplicated)
    let mut seen = std::collections::HashSet::new();
    let matches_to_peek: Vec<&TaggedMatch> = if all {
        all_matches
            .iter()
            .filter(|m| seen.insert(format!("{}:{}", m.hash, m.name)))
            .collect()
    } else {
        // Just first match per unique hash+name
        all_matches
            .iter()
            .filter(|m| seen.insert(format!("{}:{}", m.hash, m.name)))
            .take(1)
            .collect()
    };

    // Use a writer for output
    let mut out: Box<dyn std::io::Write> = if let Some(ref path) = output {
        Box::new(std::fs::File::create(path)?)
    } else {
        Box::new(std::io::stdout())
    };

    for (idx, m) in matches_to_peek.iter().enumerate() {
        if idx > 0 {
            writeln!(out)?;
        }

        // Fetch content to a temp buffer
        let content = fetch_content_to_string(m, node.clone(), no_relay).await?;

        // Print the peek
        print_peek(
            &mut out,
            &m.name,
            &m.hash.to_string(),
            &content,
            &peek_opts,
            matches_to_peek.len(),
        )?;
    }

    Ok(())
}

/// Fetches blob content as a string.
async fn fetch_content_to_string(
    m: &TaggedMatch,
    node: Option<String>,
    no_relay: bool,
) -> Result<String> {
    use tempfile::NamedTempFile;

    // Create a temp file to fetch into
    let temp_file = NamedTempFile::new()?;
    let temp_path = temp_file.path().to_string_lossy().to_string();

    if let Some(ref node_str) = node {
        let node_id: EndpointId = node_str.parse()?;
        cmd_get_one_remote(node_id, &m.name, &temp_path, no_relay).await?;
    } else {
        cmd_get_one(&m.name, &temp_path, false, false).await?;
    }

    // Read content
    let content = std::fs::read_to_string(&temp_path)?;

    Ok(content)
}

/// Prints a peek preview of content.
fn print_peek(
    out: &mut dyn std::io::Write,
    name: &str,
    hash: &str,
    content: &str,
    opts: &PeekOptions,
    total_files: usize,
) -> Result<()> {
    if opts.chars {
        print_peek_chars(out, name, hash, content, opts, total_files)
    } else if opts.words {
        print_peek_words(out, name, hash, content, opts, total_files)
    } else {
        print_peek_lines(out, name, hash, content, opts, total_files)
    }
}

/// Print peek by lines.
fn print_peek_lines(
    out: &mut dyn std::io::Write,
    name: &str,
    hash: &str,
    content: &str,
    opts: &PeekOptions,
    total_files: usize,
) -> Result<()> {
    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len();
    let n = opts.lines;

    // Print header if not quiet
    if !opts.quiet {
        writeln!(out, "─── {name} ───")?;
        writeln!(
            out,
            "hash: {}  lines: {}  files: {}",
            &hash[..12],
            total_lines,
            total_files
        )?;
        writeln!(out, "───────────────────────────────────────")?;
    }

    // If small enough, show all
    if total_lines <= n * 2 {
        for line in &lines {
            writeln!(out, "{line}")?;
        }
    } else if opts.head_only {
        // Show only head
        for line in lines.iter().take(n) {
            writeln!(out, "{line}")?;
        }
        if total_lines > n && !opts.quiet {
            writeln!(out, "... ({} more lines)", total_lines - n)?;
        }
    } else if opts.tail_only {
        // Show only tail
        if total_lines > n && !opts.quiet {
            writeln!(out, "... ({} lines above)", total_lines - n)?;
        }
        for line in lines.iter().skip(total_lines.saturating_sub(n)) {
            writeln!(out, "{line}")?;
        }
    } else {
        // Show head + tail
        for line in lines.iter().take(n) {
            writeln!(out, "{line}")?;
        }
        writeln!(out, "...")?;
        writeln!(
            out,
            "... ({} lines omitted)",
            total_lines.saturating_sub(n * 2)
        )?;
        writeln!(out, "...")?;
        for line in lines.iter().skip(total_lines.saturating_sub(n)) {
            writeln!(out, "{line}")?;
        }
    }

    Ok(())
}

/// Print peek by characters.
fn print_peek_chars(
    out: &mut dyn std::io::Write,
    name: &str,
    hash: &str,
    content: &str,
    opts: &PeekOptions,
    total_files: usize,
) -> Result<()> {
    let total_chars = content.chars().count();
    let n = opts.lines; // reuse lines as char count

    if !opts.quiet {
        writeln!(out, "─── {name} ───")?;
        writeln!(
            out,
            "hash: {}  chars: {}  files: {}",
            &hash[..12],
            total_chars,
            total_files
        )?;
        writeln!(out, "───────────────────────────────────────")?;
    }

    if total_chars <= n * 2 {
        write!(out, "{content}")?;
    } else if opts.head_only {
        let head: String = content.chars().take(n).collect();
        write!(out, "{head}")?;
        if !opts.quiet {
            writeln!(out, "\n... ({} more chars)", total_chars - n)?;
        }
    } else if opts.tail_only {
        if !opts.quiet {
            writeln!(out, "... ({} chars above)", total_chars - n)?;
        }
        let tail: String = content
            .chars()
            .skip(total_chars.saturating_sub(n))
            .collect();
        write!(out, "{tail}")?;
    } else {
        let head: String = content.chars().take(n).collect();
        let tail: String = content
            .chars()
            .skip(total_chars.saturating_sub(n))
            .collect();
        write!(out, "{head}")?;
        writeln!(
            out,
            "\n... ({} chars omitted)",
            total_chars.saturating_sub(n * 2)
        )?;
        write!(out, "{tail}")?;
    }
    writeln!(out)?;

    Ok(())
}

/// Print peek by words.
fn print_peek_words(
    out: &mut dyn std::io::Write,
    name: &str,
    hash: &str,
    content: &str,
    opts: &PeekOptions,
    total_files: usize,
) -> Result<()> {
    let words: Vec<&str> = content.split_whitespace().collect();
    let total_words = words.len();
    let n = opts.lines; // reuse lines as word count

    if !opts.quiet {
        writeln!(out, "─── {name} ───")?;
        writeln!(
            out,
            "hash: {}  words: {}  files: {}",
            &hash[..12],
            total_words,
            total_files
        )?;
        writeln!(out, "───────────────────────────────────────")?;
    }

    if total_words <= n * 2 {
        writeln!(out, "{}", words.join(" "))?;
    } else if opts.head_only {
        let head: Vec<&str> = words.iter().take(n).copied().collect();
        writeln!(out, "{}", head.join(" "))?;
        if !opts.quiet {
            writeln!(out, "... ({} more words)", total_words - n)?;
        }
    } else if opts.tail_only {
        if !opts.quiet {
            writeln!(out, "... ({} words above)", total_words - n)?;
        }
        let tail: Vec<&str> = words
            .iter()
            .skip(total_words.saturating_sub(n))
            .copied()
            .collect();
        writeln!(out, "{}", tail.join(" "))?;
    } else {
        let head: Vec<&str> = words.iter().take(n).copied().collect();
        let tail: Vec<&str> = words
            .iter()
            .skip(total_words.saturating_sub(n))
            .copied()
            .collect();
        writeln!(out, "{}", head.join(" "))?;
        writeln!(
            out,
            "... ({} words omitted)",
            total_words.saturating_sub(n * 2)
        )?;
        writeln!(out, "{}", tail.join(" "))?;
    }

    Ok(())
}

/// Get matching entries for a query, either locally or from a remote node.
///
/// This is the core search function used by both `cmd_find` and `cmd_search`.
/// It performs case-insensitive matching against both tag names and blob hashes.
///
/// # Search Algorithm
///
/// For each stored blob:
/// 1. Convert name to lowercase
/// 2. Check if query matches name (exact → prefix → contains)
/// 3. If no name match, check if query matches hash string
/// 4. Collect all matches with their match type and source (name vs hash)
///
/// Results are sorted by:
/// 1. Match kind (Exact > Prefix > Contains)
/// 2. Match source (name vs hash, controlled by `prefer_name`)
///
/// # Arguments
///
/// * `query` - The search pattern (case-insensitive)
/// * `prefer_name` - If true, name matches sort before hash matches
/// * `node` - Optional remote node ID; if None, search locally
/// * `no_relay` - If true, disable relay servers for remote connections
///
/// # Returns
///
/// A vector of [`FindMatch`] entries, sorted by relevance. Each entry contains:
/// - `hash`: The blob's content hash
/// - `name`: The blob's tag name
/// - `kind`: Match type (Exact, Prefix, or Contains)
/// - `is_hash_match`: Whether the query matched the hash (vs name)
///
/// # Remote Protocol
///
/// When `node` is Some, sends `MetaRequest::Find { query, prefer_name }`
/// and receives `MetaResponse::Find { matches }`.
pub async fn cmd_find_matches(
    query: &str,
    prefer_name: bool,
    node: Option<String>,
    no_relay: bool,
) -> Result<Vec<FindMatch>> {
    if let Some(node_str) = node {
        let node_id: EndpointId = node_str.parse()?;
        let client_key = load_or_create_keypair(CLIENT_KEY_FILE).await?;
        let mut builder = Endpoint::builder()
            .secret_key(client_key)
            .address_lookup(PkarrPublisher::n0_dns())
            .address_lookup(DnsAddressLookup::n0_dns());
        if no_relay {
            builder = builder.relay_mode(RelayMode::Disabled);
        }
        let endpoint = builder.bind().await?;

        let meta_conn = endpoint.connect(node_id, META_ALPN).await?;
        let (mut send, mut recv) = meta_conn.open_bi().await?;
        let req = postcard::to_allocvec(&MetaRequest::Find {
            query: query.to_owned(),
            prefer_name,
        })?;
        send.write_all(&req).await?;
        send.finish()?;
        let resp_buf = recv.read_to_end(64 * 1024).await?;
        let resp: MetaResponse = postcard::from_bytes(&resp_buf)?;
        meta_conn.close(0u32.into(), b"done");

        match resp {
            MetaResponse::Find { matches } => Ok(matches),
            _ => bail!("unexpected response"),
        }
    } else {
        // Local search
        let store = open_store(false).await?;
        let store_handle = store.as_store();
        let mut matches = Vec::new();
        let query_lower = query.to_lowercase();

        if let Ok(mut list) = store_handle.tags().list().await {
            while let Some(item) = list.next().await {
                if let Ok(item) = item {
                    let name = String::from_utf8_lossy(item.name.as_ref()).to_string();
                    let hash_str = item.hash.to_string();
                    let name_lower = name.to_lowercase();

                    if let Some(kind) = match_kind(&name_lower, &query_lower) {
                        matches.push(FindMatch {
                            hash: item.hash,
                            name: name.clone(),
                            kind,
                            is_hash_match: false,
                        });
                    } else if let Some(kind) = match_kind(&hash_str, &query_lower) {
                        matches.push(FindMatch {
                            hash: item.hash,
                            name,
                            kind,
                            is_hash_match: true,
                        });
                    }
                }
            }
        }

        matches.sort_by(|a, b| match a.kind.cmp(&b.kind) {
            std::cmp::Ordering::Equal => {
                if prefer_name {
                    a.is_hash_match.cmp(&b.is_hash_match)
                } else {
                    b.is_hash_match.cmp(&a.is_hash_match)
                }
            }
            other => other,
        });

        store.shutdown().await?;
        Ok(matches)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::MatchKind;
    use iroh_blobs::Hash;

    #[test]
    fn test_match_kind_exact() {
        assert_eq!(match_kind("hello", "hello"), Some(MatchKind::Exact));
    }

    #[test]
    fn test_match_kind_prefix() {
        assert_eq!(match_kind("hello world", "hello"), Some(MatchKind::Prefix));
    }

    #[test]
    fn test_match_kind_contains() {
        assert_eq!(
            match_kind("say hello to me", "hello"),
            Some(MatchKind::Contains)
        );
    }

    #[test]
    fn test_match_kind_no_match() {
        assert_eq!(match_kind("goodbye", "hello"), None);
    }

    #[test]
    fn test_search_options_exclude() {
        let opts = SearchOptions::new(None, None, false, vec![".bak".to_owned()]);
        assert!(opts.should_exclude("file.bak", "abc123"));
        assert!(!opts.should_exclude("file.txt", "abc123"));
    }

    #[test]
    fn test_search_options_exclude_hash() {
        let opts = SearchOptions::new(None, None, false, vec!["abc".to_owned()]);
        assert!(opts.should_exclude("file.txt", "abc123def"));
        assert!(!opts.should_exclude("file.txt", "xyz789"));
    }

    #[test]
    fn test_search_options_first() {
        let opts = SearchOptions::new(Some(2), None, false, vec![]);
        let hash = Hash::from_bytes([0u8; 32]);
        let matches = vec![
            TaggedMatch {
                query: "q".to_owned(),
                hash,
                name: "a.txt".to_owned(),
                kind: MatchKind::Exact,
                is_hash_match: false,
            },
            TaggedMatch {
                query: "q".to_owned(),
                hash,
                name: "b.txt".to_owned(),
                kind: MatchKind::Exact,
                is_hash_match: false,
            },
            TaggedMatch {
                query: "q".to_owned(),
                hash,
                name: "c.txt".to_owned(),
                kind: MatchKind::Exact,
                is_hash_match: false,
            },
        ];
        let result = opts.apply(matches);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "a.txt");
        assert_eq!(result[1].name, "b.txt");
    }

    #[test]
    fn test_search_options_last() {
        let opts = SearchOptions::new(None, Some(2), false, vec![]);
        let hash = Hash::from_bytes([0u8; 32]);
        let matches = vec![
            TaggedMatch {
                query: "q".to_owned(),
                hash,
                name: "a.txt".to_owned(),
                kind: MatchKind::Exact,
                is_hash_match: false,
            },
            TaggedMatch {
                query: "q".to_owned(),
                hash,
                name: "b.txt".to_owned(),
                kind: MatchKind::Exact,
                is_hash_match: false,
            },
            TaggedMatch {
                query: "q".to_owned(),
                hash,
                name: "c.txt".to_owned(),
                kind: MatchKind::Exact,
                is_hash_match: false,
            },
        ];
        let result = opts.apply(matches);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "b.txt");
        assert_eq!(result[1].name, "c.txt");
    }

    #[test]
    fn test_search_options_combined() {
        // Exclude + first
        let opts = SearchOptions::new(Some(1), None, false, vec![".bak".to_owned()]);
        let hash = Hash::from_bytes([0u8; 32]);
        let matches = vec![
            TaggedMatch {
                query: "q".to_owned(),
                hash,
                name: "a.bak".to_owned(),
                kind: MatchKind::Exact,
                is_hash_match: false,
            },
            TaggedMatch {
                query: "q".to_owned(),
                hash,
                name: "b.txt".to_owned(),
                kind: MatchKind::Exact,
                is_hash_match: false,
            },
            TaggedMatch {
                query: "q".to_owned(),
                hash,
                name: "c.txt".to_owned(),
                kind: MatchKind::Exact,
                is_hash_match: false,
            },
        ];
        let result = opts.apply(matches);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "b.txt");
    }

    #[test]
    fn test_search_options_default() {
        let opts = SearchOptions::default();
        assert!(opts.first.is_none());
        assert!(opts.last.is_none());
        assert!(!opts.count);
        assert!(opts.exclude.is_empty());
    }

    #[test]
    fn test_search_options_exclude_case_insensitive() {
        let opts = SearchOptions::new(None, None, false, vec!["BAK".to_owned()]);
        // Should exclude .bak even though pattern is uppercase
        assert!(opts.should_exclude("file.bak", "abc123"));
        assert!(opts.should_exclude("FILE.BAK", "abc123"));
    }

    #[test]
    fn test_search_options_multiple_excludes() {
        let opts = SearchOptions::new(
            None,
            None,
            false,
            vec![".bak".to_owned(), ".tmp".to_owned(), "test".to_owned()],
        );
        assert!(opts.should_exclude("file.bak", "abc123"));
        assert!(opts.should_exclude("file.tmp", "abc123"));
        assert!(opts.should_exclude("test_file.txt", "abc123"));
        assert!(!opts.should_exclude("config.json", "xyz789"));
    }

    #[test]
    fn test_search_options_last_greater_than_len() {
        // When last > matches.len(), should return all
        let opts = SearchOptions::new(None, Some(10), false, vec![]);
        let hash = Hash::from_bytes([0u8; 32]);
        let matches = vec![
            TaggedMatch {
                query: "q".to_owned(),
                hash,
                name: "a.txt".to_owned(),
                kind: MatchKind::Exact,
                is_hash_match: false,
            },
            TaggedMatch {
                query: "q".to_owned(),
                hash,
                name: "b.txt".to_owned(),
                kind: MatchKind::Exact,
                is_hash_match: false,
            },
        ];
        let result = opts.apply(matches);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_search_options_first_zero() {
        let opts = SearchOptions::new(Some(0), None, false, vec![]);
        let hash = Hash::from_bytes([0u8; 32]);
        let matches = vec![TaggedMatch {
            query: "q".to_owned(),
            hash,
            name: "a.txt".to_owned(),
            kind: MatchKind::Exact,
            is_hash_match: false,
        }];
        let result = opts.apply(matches);
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn test_search_options_empty_matches() {
        let opts = SearchOptions::new(Some(5), None, false, vec![]);
        let matches: Vec<TaggedMatch> = vec![];
        let result = opts.apply(matches);
        assert!(result.is_empty());
    }

    // Tests for PeekOptions
    #[test]
    fn test_peek_options_default() {
        let opts = PeekOptions::default();
        assert_eq!(opts.lines, 5);
        assert!(!opts.head_only);
        assert!(!opts.tail_only);
        assert!(!opts.chars);
        assert!(!opts.words);
        assert!(!opts.quiet);
    }

    #[test]
    fn test_peek_options_custom() {
        let opts = PeekOptions {
            lines: 10,
            head_only: true,
            tail_only: false,
            chars: false,
            words: false,
            quiet: true,
        };
        assert_eq!(opts.lines, 10);
        assert!(opts.head_only);
        assert!(opts.quiet);
    }

    // Test print_peek_lines helper
    #[test]
    fn test_print_peek_lines_small_file() {
        let opts = PeekOptions {
            lines: 5,
            head_only: false,
            tail_only: false,
            chars: false,
            words: false,
            quiet: true,
        };
        let mut output = Vec::new();
        let content = "line1\nline2\nline3";
        print_peek_lines(&mut output, "test.txt", "abcdef123456", content, &opts, 1).unwrap();
        let result = String::from_utf8(output).unwrap();
        assert!(result.contains("line1"));
        assert!(result.contains("line2"));
        assert!(result.contains("line3"));
        assert!(!result.contains("...")); // No truncation for small file
    }

    #[test]
    fn test_print_peek_lines_large_file() {
        let opts = PeekOptions {
            lines: 2,
            head_only: false,
            tail_only: false,
            chars: false,
            words: false,
            quiet: true,
        };
        let mut output = Vec::new();
        let content = "line1\nline2\nline3\nline4\nline5\nline6\nline7\nline8";
        print_peek_lines(&mut output, "test.txt", "abcdef123456", content, &opts, 1).unwrap();
        let result = String::from_utf8(output).unwrap();
        assert!(result.contains("line1"));
        assert!(result.contains("line2"));
        assert!(result.contains("line7"));
        assert!(result.contains("line8"));
        assert!(result.contains("...")); // Should show truncation
    }

    #[test]
    fn test_print_peek_lines_head_only() {
        let opts = PeekOptions {
            lines: 2,
            head_only: true,
            tail_only: false,
            chars: false,
            words: false,
            quiet: true,
        };
        let mut output = Vec::new();
        let content = "line1\nline2\nline3\nline4\nline5";
        print_peek_lines(&mut output, "test.txt", "abcdef123456", content, &opts, 1).unwrap();
        let result = String::from_utf8(output).unwrap();
        assert!(result.contains("line1"));
        assert!(result.contains("line2"));
        assert!(!result.contains("line5")); // Should not show tail
    }

    #[test]
    fn test_print_peek_lines_tail_only() {
        let opts = PeekOptions {
            lines: 2,
            head_only: false,
            tail_only: true,
            chars: false,
            words: false,
            quiet: true,
        };
        let mut output = Vec::new();
        let content = "line1\nline2\nline3\nline4\nline5";
        print_peek_lines(&mut output, "test.txt", "abcdef123456", content, &opts, 1).unwrap();
        let result = String::from_utf8(output).unwrap();
        assert!(!result.contains("line1")); // Should not show head
        assert!(result.contains("line4"));
        assert!(result.contains("line5"));
    }

    #[test]
    fn test_print_peek_lines_with_header() {
        let opts = PeekOptions {
            lines: 2,
            head_only: false,
            tail_only: false,
            chars: false,
            words: false,
            quiet: false, // Show header
        };
        let mut output = Vec::new();
        let content = "line1\nline2\nline3";
        print_peek_lines(&mut output, "test.txt", "abcdef123456", content, &opts, 1).unwrap();
        let result = String::from_utf8(output).unwrap();
        assert!(result.contains("test.txt")); // Header should include filename
        assert!(result.contains("abcdef123456")); // Header should include hash prefix
        assert!(result.contains("lines:")); // Header should show line count
    }

    // Test print_peek_chars helper
    #[test]
    fn test_print_peek_chars_small_content() {
        let opts = PeekOptions {
            lines: 100, // Used as char count
            head_only: false,
            tail_only: false,
            chars: true,
            words: false,
            quiet: true,
        };
        let mut output = Vec::new();
        let content = "Hello world";
        print_peek_chars(&mut output, "test.txt", "abcdef123456", content, &opts, 1).unwrap();
        let result = String::from_utf8(output).unwrap();
        assert!(result.contains("Hello world"));
        assert!(!result.contains("omitted")); // No truncation
    }

    #[test]
    fn test_print_peek_chars_large_content() {
        let opts = PeekOptions {
            lines: 5, // Used as char count
            head_only: false,
            tail_only: false,
            chars: true,
            words: false,
            quiet: true,
        };
        let mut output = Vec::new();
        let content = "Hello beautiful world!";
        print_peek_chars(&mut output, "test.txt", "abcdef123456", content, &opts, 1).unwrap();
        let result = String::from_utf8(output).unwrap();
        assert!(result.contains("Hello")); // First 5 chars
        assert!(result.contains("orld!")); // Last 5 chars
        assert!(result.contains("omitted")); // Should show truncation
    }

    // Test print_peek_words helper
    #[test]
    fn test_print_peek_words_small_content() {
        let opts = PeekOptions {
            lines: 10, // Used as word count
            head_only: false,
            tail_only: false,
            chars: false,
            words: true,
            quiet: true,
        };
        let mut output = Vec::new();
        let content = "Hello beautiful world";
        print_peek_words(&mut output, "test.txt", "abcdef123456", content, &opts, 1).unwrap();
        let result = String::from_utf8(output).unwrap();
        assert!(result.contains("Hello beautiful world"));
        assert!(!result.contains("omitted")); // No truncation
    }

    #[test]
    fn test_print_peek_words_large_content() {
        let opts = PeekOptions {
            lines: 2, // Used as word count
            head_only: false,
            tail_only: false,
            chars: false,
            words: true,
            quiet: true,
        };
        let mut output = Vec::new();
        let content = "one two three four five six seven eight";
        print_peek_words(&mut output, "test.txt", "abcdef123456", content, &opts, 1).unwrap();
        let result = String::from_utf8(output).unwrap();
        assert!(result.contains("one two")); // First 2 words
        assert!(result.contains("seven eight")); // Last 2 words
        assert!(result.contains("omitted")); // Should show truncation
    }
}
