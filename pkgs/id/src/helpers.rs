//! Helper functions for command parsing and output formatting.
//!
//! This module provides utilities used across the CLI and REPL commands:
//!
//! - **Spec parsing**: Parse `source:dest` path specifications
//! - **Match printing**: Format search results for CLI and REPL output
//! - **Match logic**: Determine match quality for search operations
//!
//! # Path Specifications
//!
//! Many commands support a `source:destination` syntax for renaming:
//!
//! ```rust
//! use id::helpers::parse_put_spec;
//!
//! // Simple path (no rename)
//! let (path, name) = parse_put_spec("file.txt");
//! assert_eq!(path, "file.txt");
//! assert!(name.is_none());
//!
//! // Path with rename
//! let (path, name) = parse_put_spec("local.txt:remote.txt");
//! assert_eq!(path, "local.txt");
//! assert_eq!(name, Some("remote.txt"));
//! ```
//!
//! # Output Formats
//!
//! Search results can be displayed in three formats:
//!
//! - **tag**: Each match with its originating query (default)
//! - **group**: Matches grouped under their query headers
//! - **union**: Deduplicated matches by hash

use crate::protocol::{FindMatch, MatchKind, TaggedMatch};

/// Parses a put specification into path and optional name.
///
/// The put spec format is `path[:name]` where:
/// - `path` is the source file path
/// - `name` is the optional tag name (defaults to filename)
///
/// # Arguments
///
/// * `spec` - The specification string to parse
///
/// # Returns
///
/// A tuple of `(path, Option<name>)`.
///
/// # Examples
///
/// ```rust
/// use id::helpers::parse_put_spec;
///
/// // Simple path
/// let (path, name) = parse_put_spec("file.txt");
/// assert_eq!(path, "file.txt");
/// assert!(name.is_none());
///
/// // Path with custom name
/// let (path, name) = parse_put_spec("./data/config.json:app-config");
/// assert_eq!(path, "./data/config.json");
/// assert_eq!(name, Some("app-config"));
///
/// // Empty name is treated as None
/// let (path, name) = parse_put_spec("file.txt:");
/// assert_eq!(path, "file.txt");
/// assert!(name.is_none());
///
/// // Multiple colons: first colon is the separator
/// let (path, name) = parse_put_spec("path:name:extra");
/// assert_eq!(path, "path");
/// assert_eq!(name, Some("name:extra"));
/// ```
pub fn parse_put_spec(spec: &str) -> (&str, Option<&str>) {
    if let Some(pos) = spec.find(':') {
        let path = &spec[..pos];
        let name = &spec[pos + 1..];
        if name.is_empty() {
            (path, None)
        } else {
            (path, Some(name))
        }
    } else {
        (spec, None)
    }
}

/// Parses a get specification into source and optional output path.
///
/// The get spec format is `source[:output]` where:
/// - `source` is the tag name or hash to retrieve
/// - `output` is the optional output path (`-` for stdout)
///
/// # Arguments
///
/// * `spec` - The specification string to parse
///
/// # Returns
///
/// A tuple of `(source, Option<output>)`.
///
/// # Examples
///
/// ```rust
/// use id::helpers::parse_get_spec;
///
/// // Simple source
/// let (source, output) = parse_get_spec("config.json");
/// assert_eq!(source, "config.json");
/// assert!(output.is_none());
///
/// // Source with output path
/// let (source, output) = parse_get_spec("config.json:local-config.json");
/// assert_eq!(source, "config.json");
/// assert_eq!(output, Some("local-config.json"));
///
/// // Output to stdout
/// let (source, output) = parse_get_spec("config.json:-");
/// assert_eq!(source, "config.json");
/// assert_eq!(output, Some("-"));
/// ```
pub fn parse_get_spec(spec: &str) -> (&str, Option<&str>) {
    // Same logic as put spec for now
    parse_put_spec(spec)
}

/// Prints a single tagged match in CLI format.
///
/// The output format depends on the `format` argument:
///
/// - `"group"`: Prints query header followed by indented match
/// - `"union"`: Prints hash and name only (no query info)
/// - `"tag"` (default): Prints hash, name, and query in parentheses
///
/// # Arguments
///
/// * `m` - The tagged match to print
/// * `format` - Output format: "tag", "group", or "union"
///
/// # Output Examples
///
/// ```text
/// # tag format (default)
/// abc123...  config.json  (config)
///
/// # group format
/// [config]
///   abc123...  config.json
///
/// # union format
/// abc123...  config.json
/// ```
pub fn print_match_cli(m: &TaggedMatch, format: &str) {
    match format {
        "group" => {
            println!("[{}]", m.query);
            println!("  {}\t{}", m.hash, m.name);
        }
        "union" => {
            println!("{}\t{}", m.hash, m.name);
        }
        _ => {
            // "tag" format (default)
            println!("{}\t{}\t({})", m.hash, m.name, m.query);
        }
    }
}

/// Prints multiple tagged matches in CLI format.
///
/// Handles batching and deduplication based on the format:
///
/// - `"group"`: Groups consecutive matches by query with headers
/// - `"union"`: Deduplicates by hash, showing each blob once
/// - `"tag"` (default): Shows each match with its query
///
/// # Arguments
///
/// * `matches` - Slice of tagged matches to print
/// * `format` - Output format: "tag", "group", or "union"
///
/// # Example
///
/// ```rust,ignore
/// use id::helpers::print_matches_cli;
///
/// // Print results grouped by query
/// print_matches_cli(&matches, "group");
/// ```
pub fn print_matches_cli(matches: &[TaggedMatch], format: &str) {
    match format {
        "group" => {
            // Group by query
            let mut current_query: Option<&str> = None;
            for m in matches {
                if current_query != Some(&m.query) {
                    if current_query.is_some() {
                        println!(); // Blank line between groups
                    }
                    println!("[{}]", m.query);
                    current_query = Some(&m.query);
                }
                println!("  {}\t{}", m.hash, m.name);
            }
        }
        "union" => {
            // Deduplicated by hash
            let mut seen = std::collections::HashSet::new();
            for m in matches {
                if seen.insert(m.hash) {
                    println!("{}\t{}", m.hash, m.name);
                }
            }
        }
        _ => {
            // "tag" format (default)
            for m in matches {
                println!("{}\t{}\t({})", m.hash, m.name, m.query);
            }
        }
    }
}

/// Prints a single match in REPL format with match kind details.
///
/// REPL output includes more detail than CLI output, showing the
/// match kind (exact/prefix/contains) for debugging.
///
/// # Arguments
///
/// * `query` - The search query that produced this match
/// * `m` - The match to print
/// * `format` - Output format: "tag", "group", or "union"
///
/// # Output Example
///
/// ```text
/// # tag format
///   abc123...  config.json  [exact, config]
///
/// # group/union format
///   abc123...  config.json
/// ```
pub fn print_match_repl(query: &str, m: &FindMatch, format: &str) {
    let kind_str = match m.kind {
        MatchKind::Exact => "exact",
        MatchKind::Prefix => "prefix",
        MatchKind::Contains => "contains",
    };
    match format {
        "group" | "union" => {
            println!("  {}\t{}", m.hash, m.name);
        }
        _ => {
            println!("  {}\t{}\t[{}, {}]", m.hash, m.name, kind_str, query);
        }
    }
}

/// Determines the match quality of a needle in a haystack.
///
/// This is a local helper duplicating the logic from the protocol's `match_kind`
/// for use in local command implementations without requiring protocol access.
///
/// # Arguments
///
/// * `haystack` - The string to search in
/// * `needle` - The string to search for
///
/// # Returns
///
/// - `Some(MatchKind::Exact)` if strings are equal
/// - `Some(MatchKind::Prefix)` if haystack starts with needle
/// - `Some(MatchKind::Contains)` if haystack contains needle
/// - `None` if no match
///
/// # Note
///
/// Matching is case-sensitive. Callers should lowercase both strings
/// for case-insensitive matching.
///
/// # Examples
///
/// ```rust
/// use id::helpers::match_kind;
/// use id::protocol::MatchKind;
///
/// assert_eq!(match_kind("hello", "hello"), Some(MatchKind::Exact));
/// assert_eq!(match_kind("hello world", "hello"), Some(MatchKind::Prefix));
/// assert_eq!(match_kind("say hello", "hello"), Some(MatchKind::Contains));
/// assert_eq!(match_kind("goodbye", "hello"), None);
///
/// // Case-sensitive
/// assert_eq!(match_kind("Hello", "hello"), None);
/// ```
pub fn match_kind(haystack: &str, needle: &str) -> Option<MatchKind> {
    if haystack == needle {
        Some(MatchKind::Exact)
    } else if haystack.starts_with(needle) {
        Some(MatchKind::Prefix)
    } else if haystack.contains(needle) {
        Some(MatchKind::Contains)
    } else {
        None
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use iroh_blobs::Hash;

    #[test]
    fn test_parse_put_spec_simple() {
        let (path, name) = parse_put_spec("file.txt");
        assert_eq!(path, "file.txt");
        assert!(name.is_none());
    }

    #[test]
    fn test_parse_put_spec_with_name() {
        let (path, name) = parse_put_spec("local.txt:remote.txt");
        assert_eq!(path, "local.txt");
        assert_eq!(name, Some("remote.txt"));
    }

    #[test]
    fn test_parse_put_spec_empty_name() {
        let (path, name) = parse_put_spec("file.txt:");
        assert_eq!(path, "file.txt");
        assert!(name.is_none());
    }

    #[test]
    fn test_parse_put_spec_multiple_colons() {
        let (path, name) = parse_put_spec("path:name:extra");
        assert_eq!(path, "path");
        assert_eq!(name, Some("name:extra"));
    }

    #[test]
    fn test_parse_get_spec_simple() {
        let (source, output) = parse_get_spec("file.txt");
        assert_eq!(source, "file.txt");
        assert!(output.is_none());
    }

    #[test]
    fn test_parse_get_spec_with_output() {
        let (source, output) = parse_get_spec("source.txt:output.txt");
        assert_eq!(source, "source.txt");
        assert_eq!(output, Some("output.txt"));
    }

    #[test]
    fn test_parse_get_spec_stdout() {
        let (source, output) = parse_get_spec("file.txt:-");
        assert_eq!(source, "file.txt");
        assert_eq!(output, Some("-"));
    }

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
        assert_eq!(match_kind("say hello", "hello"), Some(MatchKind::Contains));
    }

    #[test]
    fn test_match_kind_none() {
        assert_eq!(match_kind("goodbye", "hello"), None);
    }

    #[test]
    fn test_match_kind_case_sensitive() {
        assert_eq!(match_kind("Hello", "hello"), None);
        assert_eq!(match_kind("hello", "Hello"), None);
    }

    #[test]
    fn test_match_kind_empty_needle() {
        // Empty string: starts_with("") is true, so returns Prefix
        assert_eq!(match_kind("hello", ""), Some(MatchKind::Prefix));
    }

    #[test]
    fn test_print_match_cli_formats() {
        let hash_bytes = [0u8; 32];
        let m = TaggedMatch {
            query: "test".to_owned(),
            hash: Hash::from_bytes(hash_bytes),
            name: "file.txt".to_owned(),
            kind: MatchKind::Exact,
            is_hash_match: false,
        };

        // Just verify no panic - actual output goes to stdout
        print_match_cli(&m, "tag");
        print_match_cli(&m, "group");
        print_match_cli(&m, "union");
    }

    #[test]
    fn test_print_matches_cli_group_format() {
        let hash_bytes = [0u8; 32];
        let matches = vec![
            TaggedMatch {
                query: "q1".to_owned(),
                hash: Hash::from_bytes(hash_bytes),
                name: "file1.txt".to_owned(),
                kind: MatchKind::Exact,
                is_hash_match: false,
            },
            TaggedMatch {
                query: "q1".to_owned(),
                hash: Hash::from_bytes([1u8; 32]),
                name: "file2.txt".to_owned(),
                kind: MatchKind::Prefix,
                is_hash_match: false,
            },
            TaggedMatch {
                query: "q2".to_owned(),
                hash: Hash::from_bytes([2u8; 32]),
                name: "file3.txt".to_owned(),
                kind: MatchKind::Contains,
                is_hash_match: true,
            },
        ];

        // Just verify no panic
        print_matches_cli(&matches, "group");
        print_matches_cli(&matches, "union");
        print_matches_cli(&matches, "tag");
    }

    #[test]
    fn test_print_match_repl_formats() {
        let hash_bytes = [0u8; 32];
        let m = FindMatch {
            hash: Hash::from_bytes(hash_bytes),
            name: "file.txt".to_owned(),
            kind: MatchKind::Exact,
            is_hash_match: false,
        };

        // Just verify no panic
        print_match_repl("query", &m, "tag");
        print_match_repl("query", &m, "group");
        print_match_repl("query", &m, "union");
    }
}
