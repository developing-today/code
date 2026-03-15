//! Helper functions for command parsing and formatting

use crate::protocol::{FindMatch, MatchKind, TaggedMatch};

/// Parse a put spec like "path:name" into (path, optional_name)
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

/// Parse a get spec like "source:output" into (source, optional_output)
pub fn parse_get_spec(spec: &str) -> (&str, Option<&str>) {
    // Same logic as put spec for now
    parse_put_spec(spec)
}

/// Print a single match in CLI format
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

/// Print multiple matches in CLI format
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

/// Print a single match in REPL format (simpler)
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

/// Local match_kind helper (duplicated from lib for use in commands)
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
            query: "test".to_string(),
            hash: Hash::from_bytes(hash_bytes),
            name: "file.txt".to_string(),
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
                query: "q1".to_string(),
                hash: Hash::from_bytes(hash_bytes),
                name: "file1.txt".to_string(),
                kind: MatchKind::Exact,
                is_hash_match: false,
            },
            TaggedMatch {
                query: "q1".to_string(),
                hash: Hash::from_bytes([1u8; 32]),
                name: "file2.txt".to_string(),
                kind: MatchKind::Prefix,
                is_hash_match: false,
            },
            TaggedMatch {
                query: "q2".to_string(),
                hash: Hash::from_bytes([2u8; 32]),
                name: "file3.txt".to_string(),
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
            name: "file.txt".to_string(),
            kind: MatchKind::Exact,
            is_hash_match: false,
        };

        // Just verify no panic
        print_match_repl("query", &m, "tag");
        print_match_repl("query", &m, "group");
        print_match_repl("query", &m, "union");
    }
}
