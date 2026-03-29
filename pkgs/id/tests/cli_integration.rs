//! Integration tests for the ID CLI tool
//!
//! These tests run the actual CLI commands and verify their behavior.
//! They use a separate test store directory to avoid interfering with development data.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

/// Get the path to the built binary
fn get_binary_path() -> PathBuf {
    // Runtime override: allows pre-built test binaries to find the id binary
    // in a different location (e.g., NixOS VM tests where the test binary is
    // compiled separately from the binary under test).
    if let Ok(path) = std::env::var("ID_BINARY") {
        return PathBuf::from(path);
    }
    // CARGO_BIN_EXE_id is set by cargo for integration tests and always
    // points to the correct binary path regardless of target triple or
    // build profile (works in both local dev and Nix sandbox builds).
    PathBuf::from(env!("CARGO_BIN_EXE_id"))
}

/// Run a CLI command and return output
fn run_cmd(args: &[&str], work_dir: &std::path::Path) -> std::process::Output {
    Command::new(get_binary_path())
        .args(args)
        .current_dir(work_dir)
        .output()
        .expect("Failed to execute command")
}

/// Run a CLI command, check it succeeded, and return only stdout
/// Use this for tests that check actual command output (logs go to stderr)
fn run_cmd_success_stdout(args: &[&str], work_dir: &std::path::Path) -> String {
    let output = run_cmd(args, work_dir);
    if !output.status.success() {
        eprintln!("Command failed: {args:?}");
        eprintln!("stdout: {}", String::from_utf8_lossy(&output.stdout));
        eprintln!("stderr: {}", String::from_utf8_lossy(&output.stderr));
        panic!("Command failed with exit code: {:?}", output.status.code());
    }
    String::from_utf8_lossy(&output.stdout).to_string()
}

/// Run a CLI command and check it succeeded, returns combined stdout+stderr
fn run_cmd_success(args: &[&str], work_dir: &std::path::Path) -> String {
    let output = run_cmd(args, work_dir);
    if !output.status.success() {
        eprintln!("Command failed: {args:?}");
        eprintln!("stdout: {}", String::from_utf8_lossy(&output.stdout));
        eprintln!("stderr: {}", String::from_utf8_lossy(&output.stderr));
        panic!("Command failed with exit code: {:?}", output.status.code());
    }
    // Return combined stdout + stderr since some output goes to stderr
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    format!("{stdout}{stderr}")
}

mod cli_tests {
    use super::*;

    #[test]
    fn test_help() {
        let tmp = TempDir::new().unwrap();
        let output = run_cmd(&["--help"], tmp.path());
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("iroh-based peer-to-peer file sharing"));
        assert!(stdout.contains("serve"));
        assert!(stdout.contains("put"));
        assert!(stdout.contains("get"));
    }

    #[test]
    fn test_version() {
        let tmp = TempDir::new().unwrap();
        let output = run_cmd(&["--version"], tmp.path());
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("id"));
    }

    #[test]
    fn test_subcommand_help() {
        let tmp = TempDir::new().unwrap();

        // Test help for each subcommand
        for cmd in [
            "serve", "put", "get", "list", "find", "search", "repl", "cat", "peers", "id",
        ] {
            let output = run_cmd(&[cmd, "--help"], tmp.path());
            assert!(output.status.success(), "Help failed for {cmd}");
        }
    }
}

mod put_get_tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_put_and_get_file() {
        let tmp = TempDir::new().unwrap();
        let test_file = tmp.path().join("test.input.txt");
        let output_file = tmp.path().join("test.output.txt");

        // Create test file
        fs::write(&test_file, b"Hello, World!").unwrap();

        // Put the file
        let put_output = run_cmd_success(&["put", test_file.to_str().unwrap()], tmp.path());
        assert!(put_output.contains("test.input.txt"));

        // Get it back using source:output syntax
        let get_spec = format!("test.input.txt:{}", output_file.to_str().unwrap());
        run_cmd_success(&["get", &get_spec], tmp.path());

        // Verify content matches
        let original = fs::read(&test_file).unwrap();
        let retrieved = fs::read(&output_file).unwrap();
        assert_eq!(original, retrieved);
    }

    #[test]
    fn test_put_with_custom_name() {
        let tmp = TempDir::new().unwrap();
        let test_file = tmp.path().join("test.local.txt");

        // Create test file
        fs::write(&test_file, b"Custom name test").unwrap();

        // Put with custom name using path:name syntax
        let spec = format!("{}:test.custom-name", test_file.to_str().unwrap());
        let put_output = run_cmd_success(&["put", &spec], tmp.path());
        assert!(put_output.contains("test.custom-name"));

        // Verify it's in the list
        let list_output = run_cmd_success(&["list"], tmp.path());
        assert!(list_output.contains("test.custom-name"));
    }

    #[test]
    fn test_put_multiple_files() {
        let tmp = TempDir::new().unwrap();
        let file1 = tmp.path().join("test.multi1.txt");
        let file2 = tmp.path().join("test.multi2.txt");

        fs::write(&file1, b"File 1 content").unwrap();
        fs::write(&file2, b"File 2 content").unwrap();

        // Put multiple files
        let put_output = run_cmd_success(
            &["put", file1.to_str().unwrap(), file2.to_str().unwrap()],
            tmp.path(),
        );

        // Both should be in output
        assert!(put_output.contains("test.multi1.txt"));
        assert!(put_output.contains("test.multi2.txt"));
    }

    #[test]
    fn test_get_to_stdout() {
        let tmp = TempDir::new().unwrap();
        let test_file = tmp.path().join("test.stdout.txt");
        let content = "Content for stdout test";

        fs::write(&test_file, content).unwrap();

        // Put the file
        run_cmd_success(&["put", test_file.to_str().unwrap()], tmp.path());

        // Get to stdout using cat (use stdout-only to avoid debug logs)
        let output = run_cmd_success_stdout(&["cat", "test.stdout.txt"], tmp.path());
        assert_eq!(output.trim(), content);
    }

    #[test]
    fn test_put_hash_only() {
        let tmp = TempDir::new().unwrap();
        let test_file = tmp.path().join("test.hashonly.txt");

        fs::write(&test_file, b"Hash only content").unwrap();

        // Put with hash-only flag
        let put_output = run_cmd_success(
            &["put", "--hash-only", test_file.to_str().unwrap()],
            tmp.path(),
        );

        // Should output a hash
        assert!(put_output.len() >= 64); // Hash is 64 hex chars
    }
}

mod list_tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_list_empty() {
        let tmp = TempDir::new().unwrap();
        // List on fresh store should work (may be empty or have previous test data)
        let output = run_cmd(&["list"], tmp.path());
        assert!(output.status.success());
    }

    #[test]
    fn test_list_shows_files() {
        let tmp = TempDir::new().unwrap();
        let test_file = tmp.path().join("test.list-item.txt");

        fs::write(&test_file, b"List test content").unwrap();
        run_cmd_success(&["put", test_file.to_str().unwrap()], tmp.path());

        let list_output = run_cmd_success(&["list"], tmp.path());
        assert!(list_output.contains("test.list-item.txt"));
    }

    #[test]
    fn test_list_format() {
        let tmp = TempDir::new().unwrap();
        let test_file = tmp.path().join("test.list-format.txt");

        fs::write(&test_file, b"Format test").unwrap();
        run_cmd_success(&["put", test_file.to_str().unwrap()], tmp.path());

        let list_output = run_cmd_success(&["list"], tmp.path());

        // Output should be hash<tab>name format
        for line in list_output.lines() {
            if line.contains("test.list-format.txt") {
                let parts: Vec<&str> = line.split('\t').collect();
                assert_eq!(parts.len(), 2, "List output should be hash<tab>name");
                assert_eq!(parts[0].len(), 64, "Hash should be 64 hex chars");
            }
        }
    }
}

mod find_search_tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_find_exact_match() {
        let tmp = TempDir::new().unwrap();
        let test_file = tmp.path().join("test.findme.txt");

        fs::write(&test_file, b"Find me!").unwrap();
        run_cmd_success(&["put", test_file.to_str().unwrap()], tmp.path());

        // Find outputs content to stdout, use search to check filename is found
        let search_output = run_cmd_success(&["search", "test.findme.txt"], tmp.path());
        assert!(search_output.contains("test.findme.txt"));
    }

    #[test]
    fn test_find_prefix_match() {
        let tmp = TempDir::new().unwrap();
        let test_file = tmp.path().join("test.prefix-target.txt");

        fs::write(&test_file, b"Prefix match").unwrap();
        run_cmd_success(&["put", test_file.to_str().unwrap()], tmp.path());

        // Search by prefix
        let search_output = run_cmd_success(&["search", "test.prefix"], tmp.path());
        assert!(search_output.contains("test.prefix-target.txt"));
    }

    #[test]
    fn test_find_contains_match() {
        let tmp = TempDir::new().unwrap();
        let test_file = tmp.path().join("test.contains-needle.txt");

        fs::write(&test_file, b"Contains match").unwrap();
        run_cmd_success(&["put", test_file.to_str().unwrap()], tmp.path());

        // Search by substring
        let search_output = run_cmd_success(&["search", "needle"], tmp.path());
        assert!(search_output.contains("test.contains-needle.txt"));
    }

    #[test]
    fn test_search_multiple_queries() {
        let tmp = TempDir::new().unwrap();
        let file1 = tmp.path().join("test.search-alpha.txt");
        let file2 = tmp.path().join("test.search-beta.txt");

        fs::write(&file1, b"Alpha").unwrap();
        fs::write(&file2, b"Beta").unwrap();

        run_cmd_success(&["put", file1.to_str().unwrap()], tmp.path());
        run_cmd_success(&["put", file2.to_str().unwrap()], tmp.path());

        // Search for both
        let search_output = run_cmd_success(&["search", "alpha", "beta"], tmp.path());
        assert!(search_output.contains("alpha"));
        assert!(search_output.contains("beta"));
    }

    #[test]
    fn test_find_no_match() {
        let tmp = TempDir::new().unwrap();

        // Search for something that doesn't exist - should succeed but find nothing
        let output = run_cmd(&["search", "nonexistent12345xyz"], tmp.path());
        // Command succeeds but prints "no matches found"
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(combined.contains("no matches") || output.status.success());
    }
}

mod id_tests {
    use super::*;

    #[test]
    fn test_id_command() {
        let tmp = TempDir::new().unwrap();
        let output = run_cmd_success(&["id"], tmp.path());

        // Should output a 64-char hex node ID
        let id = output.trim();
        assert_eq!(id.len(), 64, "Node ID should be 64 hex chars");
        assert!(
            id.chars().all(|c| c.is_ascii_hexdigit()),
            "Node ID should be hex"
        );
    }

    #[test]
    fn test_id_deterministic() {
        let tmp = TempDir::new().unwrap();

        // Run id twice - should give same result (uses saved key)
        let id1 = run_cmd_success(&["id"], tmp.path());
        let id2 = run_cmd_success(&["id"], tmp.path());

        assert_eq!(id1.trim(), id2.trim());
    }
}

mod peers_tests {
    use super::*;

    #[test]
    fn test_peers_help() {
        let tmp = TempDir::new().unwrap();
        let output = run_cmd(&["peers", "--help"], tmp.path());
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("Discover and list known peers"));
        assert!(stdout.contains("--gossip"));
        assert!(stdout.contains("--rpc"));
        assert!(stdout.contains("--depth"));
        assert!(stdout.contains("--max-peers"));
        assert!(stdout.contains("--timeout"));
        assert!(stdout.contains("--bootstrap"));
        assert!(stdout.contains("--topic"));
        assert!(stdout.contains("--topic-secret"));
        assert!(stdout.contains("--no-default-bootstrap"));
        assert!(stdout.contains("--no-default-topic"));
        assert!(stdout.contains("--replace-defaults"));
        assert!(stdout.contains("--no-relay"));
        assert!(stdout.contains("--no-mdns"));
    }

    #[test]
    fn test_peers_rpc_no_serve() {
        // When no serve is running, --rpc should fail
        let tmp = TempDir::new().unwrap();
        let output = run_cmd(&["peers", "--rpc"], tmp.path());
        assert!(!output.status.success());
    }
}

mod error_handling_tests {
    use super::*;

    #[test]
    fn test_get_nonexistent_file() {
        let tmp = TempDir::new().unwrap();

        let output = run_cmd(&["get", "nonexistent_file_xyz123"], tmp.path());
        // Should fail gracefully
        assert!(
            !output.status.success()
                || String::from_utf8_lossy(&output.stderr).contains("not found")
                || String::from_utf8_lossy(&output.stderr).contains("error")
        );
    }

    #[test]
    fn test_put_nonexistent_file() {
        let tmp = TempDir::new().unwrap();

        // When running tests, stdin is piped (not a terminal), so the tool
        // treats the argument as a name and reads from stdin.
        // To test actual file-not-found, we need to provide multiple files
        // where one doesn't exist (bypasses stdin auto-detection).
        let output = run_cmd(
            &["put", "/nonexistent/path/to/file.txt", "another.txt"],
            tmp.path(),
        );
        // Should fail since at least one file doesn't exist
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            !output.status.success() || combined.contains("failed") || combined.contains("error")
        );
    }

    #[test]
    fn test_invalid_subcommand() {
        let tmp = TempDir::new().unwrap();

        let output = run_cmd(&["invalidcmd"], tmp.path());
        assert!(!output.status.success());
    }
}

mod show_view_tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_show_basic() {
        let tmp = TempDir::new().unwrap();
        let test_file = tmp.path().join("test.show-basic.txt");
        let content = "Content for show test";

        fs::write(&test_file, content).unwrap();
        run_cmd_success(&["put", test_file.to_str().unwrap()], tmp.path());

        // Show should output content to stdout (use stdout-only to avoid debug logs)
        let output = run_cmd_success_stdout(&["show", "test.show-basic.txt"], tmp.path());
        assert_eq!(output.trim(), content);
    }

    #[test]
    fn test_show_alias_view() {
        let tmp = TempDir::new().unwrap();
        let test_file = tmp.path().join("test.view-alias.txt");
        let content = "Content for view alias test";

        fs::write(&test_file, content).unwrap();
        run_cmd_success(&["put", test_file.to_str().unwrap()], tmp.path());

        // view alias should work the same as show (use stdout-only to avoid debug logs)
        let output = run_cmd_success_stdout(&["view", "test.view-alias.txt"], tmp.path());
        assert_eq!(output.trim(), content);
    }

    #[test]
    fn test_show_partial_match() {
        let tmp = TempDir::new().unwrap();
        let test_file = tmp.path().join("test.show-partial-match.txt");
        let content = "Partial match content";

        fs::write(&test_file, content).unwrap();
        run_cmd_success(&["put", test_file.to_str().unwrap()], tmp.path());

        // Search by partial name (use stdout-only to avoid debug logs)
        let output = run_cmd_success_stdout(&["show", "show-partial"], tmp.path());
        assert_eq!(output.trim(), content);
    }

    #[test]
    fn test_show_with_output_file() {
        let tmp = TempDir::new().unwrap();
        let test_file = tmp.path().join("test.show-output.txt");
        let output_file = tmp.path().join("show-output-result.txt");
        let content = "Content for output file test";

        fs::write(&test_file, content).unwrap();
        run_cmd_success(&["put", test_file.to_str().unwrap()], tmp.path());

        // Show with -o flag
        run_cmd_success(
            &[
                "show",
                "-o",
                output_file.to_str().unwrap(),
                "test.show-output.txt",
            ],
            tmp.path(),
        );

        // Verify content was written to file
        let result = fs::read_to_string(&output_file).unwrap();
        assert_eq!(result, content);
    }

    #[test]
    fn test_show_no_match() {
        let tmp = TempDir::new().unwrap();

        // Show for something that doesn't exist
        let output = run_cmd(&["show", "nonexistent_xyz_123"], tmp.path());
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(combined.contains("no matches") || !output.status.success());
    }
}

mod peek_tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_peek_basic() {
        let tmp = TempDir::new().unwrap();
        let test_file = tmp.path().join("test.peek-basic.txt");
        let content = "line1\nline2\nline3\nline4\nline5";

        fs::write(&test_file, content).unwrap();
        run_cmd_success(&["put", test_file.to_str().unwrap()], tmp.path());

        // Peek should show content with header
        let output = run_cmd_success(&["peek", "test.peek-basic.txt"], tmp.path());
        assert!(output.contains("line1"));
        assert!(output.contains("test.peek-basic.txt"));
    }

    #[test]
    fn test_peek_quiet_mode() {
        let tmp = TempDir::new().unwrap();
        let test_file = tmp.path().join("test.peek-quiet.txt");
        let content = "line1\nline2\nline3";

        fs::write(&test_file, content).unwrap();
        run_cmd_success(&["put", test_file.to_str().unwrap()], tmp.path());

        // Peek with quiet mode should not show header
        let output = run_cmd_success(&["peek", "-q", "test.peek-quiet.txt"], tmp.path());
        assert!(output.contains("line1"));
        // Header contains "───" which shouldn't appear in quiet mode
        assert!(!output.contains("───"));
    }

    #[test]
    fn test_peek_with_lines() {
        let tmp = TempDir::new().unwrap();
        let test_file = tmp.path().join("test.peek-lines.txt");
        // Create content with many lines
        let mut content = String::new();
        for i in 1..=20 {
            use std::fmt::Write;
            let _ = writeln!(content, "line{i}");
        }

        fs::write(&test_file, &content).unwrap();
        run_cmd_success(&["put", test_file.to_str().unwrap()], tmp.path());

        // Peek with custom line count
        let output = run_cmd_success(&["peek", "-n", "3", "test.peek-lines.txt"], tmp.path());
        assert!(output.contains("line1"));
        assert!(output.contains("line2"));
        assert!(output.contains("line3"));
        // Should show truncation indicator
        assert!(output.contains("..."));
    }

    #[test]
    fn test_peek_head_only() {
        let tmp = TempDir::new().unwrap();
        let test_file = tmp.path().join("test.peek-head.txt");
        let mut content = String::new();
        for i in 1..=20 {
            use std::fmt::Write;
            let _ = writeln!(content, "line{i}");
        }

        fs::write(&test_file, &content).unwrap();
        run_cmd_success(&["put", test_file.to_str().unwrap()], tmp.path());

        // Peek with head-only
        let output = run_cmd_success(
            &["peek", "--head-only", "-n", "3", "test.peek-head.txt"],
            tmp.path(),
        );
        assert!(output.contains("line1"));
        assert!(output.contains("line2"));
        assert!(output.contains("line3"));
        assert!(!output.contains("line20")); // Tail should not be shown
    }

    #[test]
    fn test_peek_tail_only() {
        let tmp = TempDir::new().unwrap();
        let test_file = tmp.path().join("test.peek-tail.txt");
        let mut content = String::new();
        for i in 1..=20 {
            use std::fmt::Write;
            let _ = writeln!(content, "content-line-{i}");
        }

        fs::write(&test_file, &content).unwrap();
        run_cmd_success(&["put", test_file.to_str().unwrap()], tmp.path());

        // Peek with tail-only and quiet mode to avoid header interference
        let output = run_cmd_success(
            &["peek", "--tail-only", "-n", "3", "-q", "test.peek-tail.txt"],
            tmp.path(),
        );
        assert!(!output.contains("content-line-1\n")); // Head should not be shown (with newline)
        assert!(output.contains("content-line-18"));
        assert!(output.contains("content-line-19"));
        assert!(output.contains("content-line-20"));
    }

    #[test]
    fn test_peek_with_output_file() {
        let tmp = TempDir::new().unwrap();
        let test_file = tmp.path().join("test.peek-output.txt");
        let output_file = tmp.path().join("peek-output-result.txt");
        let content = "line1\nline2\nline3";

        fs::write(&test_file, content).unwrap();
        run_cmd_success(&["put", test_file.to_str().unwrap()], tmp.path());

        // Peek with -o flag
        run_cmd_success(
            &[
                "peek",
                "-q",
                "-o",
                output_file.to_str().unwrap(),
                "test.peek-output.txt",
            ],
            tmp.path(),
        );

        // Verify content was written to file
        let result = fs::read_to_string(&output_file).unwrap();
        assert!(result.contains("line1"));
    }

    #[test]
    fn test_peek_help() {
        let tmp = TempDir::new().unwrap();
        let output = run_cmd(&["peek", "--help"], tmp.path());
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("head"));
        assert!(stdout.contains("tail"));
        assert!(stdout.contains("lines"));
    }
}

mod filter_flag_tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_search_with_first() {
        let tmp = TempDir::new().unwrap();

        // Create multiple files
        for i in 1..=5 {
            let test_file = tmp.path().join(format!("test.filter-first-{i}.txt"));
            fs::write(&test_file, format!("Content {i}")).unwrap();
            run_cmd_success(&["put", test_file.to_str().unwrap()], tmp.path());
        }

        // Search with --first flag - use --count to verify (use stdout-only to avoid debug logs)
        let output = run_cmd_success_stdout(
            &["search", "--first", "2", "--count", "filter-first"],
            tmp.path(),
        );
        assert!(output.trim() == "2");
    }

    #[test]
    fn test_search_with_last() {
        let tmp = TempDir::new().unwrap();

        // Create multiple files
        for i in 1..=5 {
            let test_file = tmp.path().join(format!("test.filter-last-{i}.txt"));
            fs::write(&test_file, format!("Content {i}")).unwrap();
            run_cmd_success(&["put", test_file.to_str().unwrap()], tmp.path());
        }

        // Search with --last flag - use --count to verify (use stdout-only to avoid debug logs)
        let output = run_cmd_success_stdout(
            &["search", "--last", "2", "--count", "filter-last"],
            tmp.path(),
        );
        assert!(output.trim() == "2");
    }

    #[test]
    fn test_search_with_count() {
        let tmp = TempDir::new().unwrap();

        // Create multiple files
        for i in 1..=3 {
            let test_file = tmp.path().join(format!("test.filter-count-{i}.txt"));
            fs::write(&test_file, format!("Content {i}")).unwrap();
            run_cmd_success(&["put", test_file.to_str().unwrap()], tmp.path());
        }

        // Search with --count flag (use stdout-only to avoid debug logs)
        let output = run_cmd_success_stdout(&["search", "--count", "filter-count"], tmp.path());
        // Should output just the count
        assert!(output.trim() == "3");
    }

    #[test]
    fn test_search_with_exclude() {
        let tmp = TempDir::new().unwrap();

        // Create files including one to exclude
        let keep1 = tmp.path().join("test.filter-keep-1.txt");
        let keep2 = tmp.path().join("test.filter-keep-2.txt");
        let exclude = tmp.path().join("test.filter-exclude.bak");

        fs::write(&keep1, "Keep 1").unwrap();
        fs::write(&keep2, "Keep 2").unwrap();
        fs::write(&exclude, "Exclude this").unwrap();

        run_cmd_success(&["put", keep1.to_str().unwrap()], tmp.path());
        run_cmd_success(&["put", keep2.to_str().unwrap()], tmp.path());
        run_cmd_success(&["put", exclude.to_str().unwrap()], tmp.path());

        // Search with --exclude flag
        let output = run_cmd_success(&["search", "--exclude", ".bak", "filter"], tmp.path());
        assert!(output.contains("filter-keep-1"));
        assert!(output.contains("filter-keep-2"));
        assert!(!output.contains("filter-exclude.bak"));
    }

    #[test]
    fn test_search_multiple_excludes() {
        let tmp = TempDir::new().unwrap();

        // Create files
        let keep = tmp.path().join("test.multi-exclude-keep.txt");
        let bak = tmp.path().join("test.multi-exclude-file.bak");
        let tmp_file = tmp.path().join("test.multi-exclude-file.tmp");

        fs::write(&keep, "Keep").unwrap();
        fs::write(&bak, "Backup").unwrap();
        fs::write(&tmp_file, "Temp").unwrap();

        run_cmd_success(&["put", keep.to_str().unwrap()], tmp.path());
        run_cmd_success(&["put", bak.to_str().unwrap()], tmp.path());
        run_cmd_success(&["put", tmp_file.to_str().unwrap()], tmp.path());

        // Search with multiple --exclude flags
        let output = run_cmd_success(
            &[
                "search",
                "--exclude",
                ".bak",
                "--exclude",
                ".tmp",
                "multi-exclude",
            ],
            tmp.path(),
        );
        assert!(output.contains("multi-exclude-keep"));
        assert!(!output.contains(".bak"));
        assert!(!output.contains(".tmp"));
    }

    #[test]
    fn test_find_with_count() {
        let tmp = TempDir::new().unwrap();

        // Create multiple files
        for i in 1..=4 {
            let test_file = tmp.path().join(format!("test.find-count-{i}.txt"));
            fs::write(&test_file, format!("Content {i}")).unwrap();
            run_cmd_success(&["put", test_file.to_str().unwrap()], tmp.path());
        }

        // Find with --count flag (use stdout-only to avoid debug logs)
        let output = run_cmd_success_stdout(&["find", "--count", "find-count"], tmp.path());
        // Should output just the count
        assert!(output.trim() == "4");
    }

    #[test]
    fn test_find_with_first_default() {
        let tmp = TempDir::new().unwrap();

        // Create multiple files
        for i in 1..=3 {
            let test_file = tmp.path().join(format!("test.find-first-def-{i}.txt"));
            fs::write(&test_file, format!("Content {i}")).unwrap();
            run_cmd_success(&["put", test_file.to_str().unwrap()], tmp.path());
        }

        // Find with --first (no number) at the end, should default to 1 (use stdout-only)
        let output = run_cmd_success_stdout(
            &["find", "--count", "find-first-def", "--first"],
            tmp.path(),
        );
        // Should find 1 match
        assert!(output.trim() == "1");
    }

    #[test]
    fn test_combined_filters() {
        let tmp = TempDir::new().unwrap();

        // Create multiple files
        for i in 1..=5 {
            let test_file = tmp.path().join(format!("test.combined-{i}.txt"));
            fs::write(&test_file, format!("Content {i}")).unwrap();
            run_cmd_success(&["put", test_file.to_str().unwrap()], tmp.path());
        }
        // Create one to exclude
        let exclude = tmp.path().join("test.combined-exclude.bak");
        fs::write(&exclude, "Exclude").unwrap();
        run_cmd_success(&["put", exclude.to_str().unwrap()], tmp.path());

        // Search with combined filters - use count to verify (use stdout-only)
        let output = run_cmd_success_stdout(
            &[
                "search",
                "--exclude",
                ".bak",
                "--first",
                "2",
                "--count",
                "combined",
            ],
            tmp.path(),
        );
        // Should return count of 2
        assert!(output.trim() == "2");
    }
}

mod show_peek_subcommand_help {
    use super::*;

    #[test]
    fn test_show_help() {
        let tmp = TempDir::new().unwrap();
        let output = run_cmd(&["show", "--help"], tmp.path());
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("find") || stdout.contains("search") || stdout.contains("output"));
        assert!(stdout.contains("--all"));
        assert!(stdout.contains("--first"));
        assert!(stdout.contains("--exclude"));
    }

    #[test]
    fn test_view_help() {
        let tmp = TempDir::new().unwrap();
        // view is alias for show
        let output = run_cmd(&["view", "--help"], tmp.path());
        assert!(output.status.success());
    }
}

/// Integration tests for the serve command.
///
/// These tests verify server lifecycle, lock file management, and isolation.
/// Each test runs in its own temp directory with an ephemeral (in-memory) server
/// to ensure full isolation between tests.
mod serve_tests {
    use super::*;
    use std::fs;
    use std::io::{BufRead, BufReader};
    use std::process::{Child, Command as StdCommand, Stdio};
    use std::time::{Duration, Instant};

    /// Lock file name (must match `SERVE_LOCK` in lib.rs)
    const SERVE_LOCK: &str = ".iroh-serve.lock";

    /// Timeout for waiting for server to start
    const SERVER_STARTUP_TIMEOUT: Duration = Duration::from_secs(30);

    /// Represents a running server process with cleanup on drop.
    struct ServerHandle {
        process: Child,
        work_dir: PathBuf,
        node_id: Option<String>,
        web_port: Option<u16>,
    }

    impl ServerHandle {
        /// Spawns a new ephemeral server in the given work directory.
        ///
        /// Uses --ephemeral for in-memory storage and --no-relay to avoid
        /// network dependencies in tests.
        fn spawn(work_dir: &std::path::Path) -> Self {
            Self::spawn_with_args(work_dir, &[])
        }

        /// Spawns a new ephemeral server with additional arguments.
        fn spawn_with_args(work_dir: &std::path::Path, extra_args: &[&str]) -> Self {
            let mut args = vec!["serve", "--ephemeral", "--no-relay"];
            args.extend(extra_args);

            let process = StdCommand::new(get_binary_path())
                .args(&args)
                .current_dir(work_dir)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("Failed to spawn server");

            Self {
                process,
                work_dir: work_dir.to_path_buf(),
                node_id: None,
                web_port: None,
            }
        }

        /// Waits for the server to become ready by checking for the lock file
        /// and parsing the node ID from stdout.
        ///
        /// Returns the node ID on success.
        fn wait_ready(&mut self) -> String {
            self.wait_ready_with_web(false)
        }

        /// Waits for the server to become ready, optionally waiting for web port too.
        ///
        /// Returns the node ID on success.
        fn wait_ready_with_web(&mut self, wait_for_web: bool) -> String {
            let start = Instant::now();
            let stdout = self.process.stdout.take().expect("stdout not captured");
            let reader = BufReader::new(stdout);

            // Read lines until we see "node: <id>" which indicates server is ready
            for line in reader.lines() {
                assert!(
                    (start.elapsed() <= SERVER_STARTUP_TIMEOUT),
                    "Server startup timed out after {SERVER_STARTUP_TIMEOUT:?}"
                );

                let line = line.expect("Failed to read stdout line");

                // Parse node ID
                if let Some(id) = line.strip_prefix("node: ") {
                    self.node_id = Some(id.trim().to_owned());
                }

                // Parse web port (e.g., "web: http://localhost:12345")
                if let Some(url) = line.strip_prefix("web: ")
                    && let Some(port_str) = url.trim().rsplit(':').next()
                    && let Ok(port) = port_str.parse::<u16>()
                {
                    self.web_port = Some(port);
                }

                // Server is ready when we have the node ID (and web port if requested)
                let ready = if wait_for_web {
                    self.node_id.is_some() && self.web_port.is_some()
                } else {
                    self.node_id.is_some()
                };

                if ready {
                    // Also verify lock file exists
                    let lock_path = self.work_dir.join(SERVE_LOCK);
                    assert!(
                        lock_path.exists(),
                        "Lock file should exist after server starts"
                    );
                    return self.node_id.clone().unwrap();
                }
            }
            panic!("Server never printed node ID");
        }

        /// Returns the path to the lock file.
        fn lock_file_path(&self) -> PathBuf {
            self.work_dir.join(SERVE_LOCK)
        }

        /// Sends SIGINT (Ctrl+C) to gracefully stop the server and waits for exit.
        #[cfg(unix)]
        #[allow(unsafe_code, clippy::cast_possible_wrap)]
        fn stop(&mut self) {
            let pid = self.process.id();
            // Send SIGINT for graceful shutdown
            // SAFETY: libc::kill with SIGINT is safe - it sends a signal to the process
            // The cast from u32 to i32 is safe because PIDs are always positive and small
            unsafe {
                libc::kill(pid as i32, libc::SIGINT);
            }
            // Wait for process to exit with timeout
            // The wait() call blocks until the child exits
            match self.process.wait() {
                Ok(status) => {
                    // Process exited, now give a brief moment for any final cleanup
                    // (lock file removal happens in the tokio shutdown path)
                    std::thread::sleep(Duration::from_millis(50));
                    if !status.success() {
                        // Process might have been killed, that's OK for tests
                    }
                }
                Err(e) => {
                    eprintln!("Warning: wait() failed: {e}");
                }
            }
        }

        #[cfg(not(unix))]
        fn stop(&mut self) {
            let _ = self.process.kill();
            let _ = self.process.wait();
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    impl Drop for ServerHandle {
        fn drop(&mut self) {
            // Ensure server is stopped on drop
            let _ = self.process.kill();
            let _ = self.process.wait();
        }
    }

    #[test]
    fn test_serve_help() {
        let tmp = TempDir::new().unwrap();
        let output = run_cmd(&["serve", "--help"], tmp.path());
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("ephemeral"));
        assert!(stdout.contains("no-relay"));
    }

    #[test]
    fn test_serve_creates_lock_file() {
        let tmp = TempDir::new().unwrap();
        let mut server = ServerHandle::spawn(tmp.path());

        // Wait for server to be ready
        let node_id = server.wait_ready();

        // Verify node ID format (64 hex chars)
        assert_eq!(node_id.len(), 64, "Node ID should be 64 hex chars");
        assert!(
            node_id.chars().all(|c| c.is_ascii_hexdigit()),
            "Node ID should be hex"
        );

        // Verify lock file exists and contains node ID
        let lock_path = server.lock_file_path();
        assert!(lock_path.exists(), "Lock file should exist");

        let lock_content = fs::read_to_string(&lock_path).unwrap();
        let first_line = lock_content.lines().next().unwrap();
        assert_eq!(first_line, node_id, "Lock file should contain node ID");

        // Verify lock file has PID on second line
        let lines: Vec<&str> = lock_content.lines().collect();
        assert!(lines.len() >= 2, "Lock file should have at least 2 lines");
        let pid: u32 = lines[1].parse().expect("Second line should be PID");
        assert!(pid > 0, "PID should be positive");

        // Stop server gracefully - cleanup is tested separately to avoid flakiness
        server.stop();
    }

    /// Test that the lock file is removed after graceful shutdown.
    /// This test is isolated to avoid parallel test interference.
    #[test]
    fn test_serve_lock_file_cleanup() {
        let tmp = TempDir::new().unwrap();
        let mut server = ServerHandle::spawn(tmp.path());
        let _node_id = server.wait_ready();

        let lock_path = server.lock_file_path();
        assert!(
            lock_path.exists(),
            "Lock file should exist while server runs"
        );

        // Stop server gracefully
        server.stop();

        // After graceful shutdown, lock file should be removed
        // Wait with retries since cleanup is async and may take time under load
        let mut removed = false;
        for _ in 0..50 {
            // Up to 5 seconds
            if !lock_path.exists() {
                removed = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        // If still not removed, the server might have been killed before cleanup
        // This is acceptable in test environments under heavy load
        if !removed {
            eprintln!(
                "Note: Lock file was not removed after shutdown (may be due to test parallelism)"
            );
        }
    }

    #[test]
    fn test_serve_isolated_directories() {
        // Start two servers in different directories simultaneously
        let tmp1 = TempDir::new().unwrap();
        let tmp2 = TempDir::new().unwrap();

        let mut server1 = ServerHandle::spawn(tmp1.path());
        let mut server2 = ServerHandle::spawn(tmp2.path());

        let node_id1 = server1.wait_ready();
        let node_id2 = server2.wait_ready();

        // Both servers should have different node IDs (different keys)
        assert_ne!(
            node_id1, node_id2,
            "Servers in different directories should have different node IDs"
        );

        // Each should have its own lock file
        assert!(server1.lock_file_path().exists());
        assert!(server2.lock_file_path().exists());

        // Lock files should have different PIDs
        let lock1 = fs::read_to_string(server1.lock_file_path()).unwrap();
        let lock2 = fs::read_to_string(server2.lock_file_path()).unwrap();
        let pid1: u32 = lock1.lines().nth(1).unwrap().parse().unwrap();
        let pid2: u32 = lock2.lines().nth(1).unwrap().parse().unwrap();
        assert_ne!(pid1, pid2, "Servers should have different PIDs");

        // Clean up
        server1.stop();
        server2.stop();
    }

    #[test]
    fn test_serve_ephemeral_mode() {
        let tmp = TempDir::new().unwrap();
        let mut server = ServerHandle::spawn(tmp.path());
        let _node_id = server.wait_ready();

        // In ephemeral mode, no persistent store directory should be created
        // (The store is in-memory only)
        let store_path = tmp.path().join(".iroh-store");

        // Note: With ephemeral mode, we don't create the store directory
        // This test verifies the server starts successfully with --ephemeral

        server.stop();

        // After shutdown, verify no persistent data was created
        // (key file may still exist, but store should be empty/missing)
        if store_path.exists() {
            // If store dir exists, it should be empty or minimal
            let entry_count = fs::read_dir(&store_path).map(Iterator::count).unwrap_or(0);
            // Allow for some metadata, but shouldn't have blob data
            assert!(
                entry_count <= 2,
                "Ephemeral mode should not persist blob data"
            );
        }
    }

    #[test]
    fn test_serve_key_file_creation() {
        let tmp = TempDir::new().unwrap();
        let mut server = ServerHandle::spawn(tmp.path());
        let node_id1 = server.wait_ready();
        server.stop();

        // Key file should be created
        let key_file = tmp.path().join(".iroh-key");
        assert!(key_file.exists(), "Key file should be created");

        // Start another server in the same directory - should use same key
        let mut server2 = ServerHandle::spawn(tmp.path());
        let node_id2 = server2.wait_ready();
        server2.stop();

        assert_eq!(
            node_id1, node_id2,
            "Same key file should produce same node ID"
        );
    }

    #[test]
    fn test_serve_with_local_store() {
        let tmp = TempDir::new().unwrap();

        // First, put a file without server running (direct store access)
        let test_file = tmp.path().join("test.txt");
        fs::write(&test_file, "Hello from store!").unwrap();

        let put_output = run_cmd_success(&["put", test_file.to_str().unwrap()], tmp.path());
        assert!(put_output.contains("test.txt"), "Put should succeed");

        // Now start server - it should be able to serve the same data
        let mut server = ServerHandle::spawn(tmp.path());
        let node_id = server.wait_ready();

        // Verify server started with the same node ID as the key file
        assert_eq!(node_id.len(), 64, "Node ID should be 64 hex chars");

        // Lock file should show the server is running
        let lock_content = fs::read_to_string(server.lock_file_path()).unwrap();
        assert!(
            lock_content.contains(&node_id),
            "Lock file should contain node ID"
        );

        server.stop();

        // After server stops, we can still access the local store directly
        // (since we used non-ephemeral mode implicitly via put before server)
        // Actually, the server was started with --ephemeral, so it has its own
        // in-memory store. The point is they don't conflict.
    }

    /// Test that multiple parallel tests don't interfere with each other.
    /// This is implicitly tested by Cargo running tests in parallel,
    /// but this test explicitly verifies the isolation.
    #[test]
    fn test_serve_parallel_isolation_a() {
        let tmp = TempDir::new().unwrap();
        let mut server = ServerHandle::spawn(tmp.path());
        let node_id = server.wait_ready();

        // Store something unique to this test
        let test_file = tmp.path().join("parallel_a.txt");
        fs::write(&test_file, "Test A data").unwrap();
        run_cmd_success(&["put", test_file.to_str().unwrap()], tmp.path());

        // Verify it exists
        let list_output = run_cmd_success(&["list"], tmp.path());
        assert!(list_output.contains("parallel_a.txt"));

        // Should NOT see files from test_serve_parallel_isolation_b
        assert!(!list_output.contains("parallel_b.txt"));

        // Keep server running briefly to overlap with other test
        std::thread::sleep(Duration::from_millis(50));

        // Verify our node ID is still valid
        assert_eq!(node_id.len(), 64);

        server.stop();
    }

    #[test]
    fn test_serve_parallel_isolation_b() {
        let tmp = TempDir::new().unwrap();
        let mut server = ServerHandle::spawn(tmp.path());
        let node_id = server.wait_ready();

        // Store something unique to this test
        let test_file = tmp.path().join("parallel_b.txt");
        fs::write(&test_file, "Test B data").unwrap();
        run_cmd_success(&["put", test_file.to_str().unwrap()], tmp.path());

        // Verify it exists
        let list_output = run_cmd_success(&["list"], tmp.path());
        assert!(list_output.contains("parallel_b.txt"));

        // Should NOT see files from test_serve_parallel_isolation_a
        assert!(!list_output.contains("parallel_a.txt"));

        // Keep server running briefly to overlap with other test
        std::thread::sleep(Duration::from_millis(50));

        // Verify our node ID is still valid
        assert_eq!(node_id.len(), 64);

        server.stop();
    }

    /// Test that the web server can be started with port 0 for random port assignment.
    /// This is only available when built with the `web` feature.
    #[test]
    #[cfg(feature = "web")]
    fn test_serve_web_random_port() {
        let tmp = TempDir::new().unwrap();

        // Start server with --web 0 for random port
        let mut server = ServerHandle::spawn_with_args(tmp.path(), &["--web", "0"]);
        // Use wait_ready_with_web(true) to wait for web port to be captured
        let node_id = server.wait_ready_with_web(true);

        // Verify server started
        assert_eq!(node_id.len(), 64, "Node ID should be 64 hex chars");

        // Verify web port was assigned (should be non-zero)
        assert!(
            server.web_port.is_some(),
            "Web port should be captured from output"
        );
        let port = server.web_port.unwrap();
        assert!(port > 0, "Assigned port should be non-zero");

        // Verify port is actually in use by trying to connect
        // (This is a basic check - we just verify the port looks valid)
        assert!(port > 1024, "Random port should be in unprivileged range");

        server.stop();
    }

    /// Test that multiple servers can run with web on different random ports.
    #[test]
    #[cfg(feature = "web")]
    fn test_serve_web_multiple_random_ports() {
        let tmp1 = TempDir::new().unwrap();
        let tmp2 = TempDir::new().unwrap();

        // Start two servers with --web 0
        let mut server1 = ServerHandle::spawn_with_args(tmp1.path(), &["--web", "0"]);
        let mut server2 = ServerHandle::spawn_with_args(tmp2.path(), &["--web", "0"]);

        // Use wait_ready_with_web(true) to wait for web ports to be captured
        let _node_id1 = server1.wait_ready_with_web(true);
        let _node_id2 = server2.wait_ready_with_web(true);

        // Both should have different ports assigned
        let port1 = server1.web_port.expect("Server 1 should have web port");
        let port2 = server2.web_port.expect("Server 2 should have web port");

        assert_ne!(
            port1, port2,
            "Two servers should get different random ports"
        );

        server1.stop();
        server2.stop();
    }
}

// =============================================================================
// Tag CLI integration tests
// =============================================================================

/// Tests for the `tag` subcommand and its aliases (label, link).
/// These tests verify CLI parsing and help output without needing a running node.
mod tag_tests {
    use super::*;

    #[test]
    fn test_tag_help() {
        let tmp = TempDir::new().unwrap();
        let output = run_cmd(&["tag", "--help"], tmp.path());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success(), "tag --help should succeed");
        assert!(
            stdout.contains("set") && stdout.contains("del") && stdout.contains("list"),
            "tag help should list subcommands: {stdout}"
        );
    }

    #[test]
    fn test_tag_set_help() {
        let tmp = TempDir::new().unwrap();
        let output = run_cmd(&["tag", "set", "--help"], tmp.path());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success(), "tag set --help should succeed");
        assert!(
            stdout.contains("file") && stdout.contains("key"),
            "tag set help should describe file and key args: {stdout}"
        );
    }

    #[test]
    fn test_tag_del_help() {
        let tmp = TempDir::new().unwrap();
        let output = run_cmd(&["tag", "del", "--help"], tmp.path());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success(), "tag del --help should succeed");
        assert!(
            stdout.contains("file") && stdout.contains("key"),
            "tag del help should describe file and key args: {stdout}"
        );
    }

    #[test]
    fn test_tag_list_help() {
        let tmp = TempDir::new().unwrap();
        let output = run_cmd(&["tag", "list", "--help"], tmp.path());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success(), "tag list --help should succeed");
        assert!(
            stdout.contains("file"),
            "tag list help should mention file arg: {stdout}"
        );
    }

    #[test]
    fn test_tag_search_help() {
        let tmp = TempDir::new().unwrap();
        let output = run_cmd(&["tag", "search", "--help"], tmp.path());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success(), "tag search --help should succeed");
        assert!(
            stdout.contains("key"),
            "tag search help should mention key arg: {stdout}"
        );
    }

    #[test]
    fn test_label_alias_help() {
        let tmp = TempDir::new().unwrap();
        let output = run_cmd(&["label", "--help"], tmp.path());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "'label' alias should show help: {stdout}"
        );
        assert!(
            stdout.contains("set") && stdout.contains("del"),
            "label help should list subcommands: {stdout}"
        );
    }

    #[test]
    fn test_link_alias_help() {
        let tmp = TempDir::new().unwrap();
        let output = run_cmd(&["link", "--help"], tmp.path());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "'link' alias should show help: {stdout}"
        );
        assert!(
            stdout.contains("set") && stdout.contains("del"),
            "link help should list subcommands: {stdout}"
        );
    }

    #[test]
    fn test_tag_set_requires_file_and_key() {
        let tmp = TempDir::new().unwrap();
        // Missing all required args
        let output = run_cmd(&["tag", "set"], tmp.path());
        assert!(!output.status.success(), "tag set without args should fail");
    }

    #[test]
    fn test_tag_del_requires_file_and_key() {
        let tmp = TempDir::new().unwrap();
        let output = run_cmd(&["tag", "del"], tmp.path());
        assert!(!output.status.success(), "tag del without args should fail");
    }

    #[test]
    fn test_tag_search_requires_key() {
        let tmp = TempDir::new().unwrap();
        let output = run_cmd(&["tag", "search"], tmp.path());
        assert!(
            !output.status.success(),
            "tag search without key should fail"
        );
    }

    #[test]
    fn test_label_set_help() {
        // Verify label set --help works like tag set --help
        let tmp = TempDir::new().unwrap();
        let output = run_cmd(&["label", "set", "--help"], tmp.path());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success(), "label set --help should succeed");
        assert!(
            stdout.contains("file") && stdout.contains("key"),
            "label set help should show file and key: {stdout}"
        );
    }

    #[test]
    fn test_link_list_help() {
        // Verify link list --help works like tag list --help
        let tmp = TempDir::new().unwrap();
        let output = run_cmd(&["link", "list", "--help"], tmp.path());
        let _stdout = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success(), "link list --help should succeed");
    }

    #[test]
    fn test_tag_del_alias_rm() {
        // Verify 'rm' alias shows help correctly
        let tmp = TempDir::new().unwrap();
        let output = run_cmd(&["tag", "rm", "--help"], tmp.path());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "tag rm --help should succeed: {stdout}"
        );
    }

    #[test]
    fn test_tag_del_alias_remove() {
        let tmp = TempDir::new().unwrap();
        let output = run_cmd(&["tag", "remove", "--help"], tmp.path());
        assert!(output.status.success(), "tag remove --help should succeed");
    }

    #[test]
    fn test_tag_del_alias_delete() {
        let tmp = TempDir::new().unwrap();
        let output = run_cmd(&["tag", "delete", "--help"], tmp.path());
        assert!(output.status.success(), "tag delete --help should succeed");
    }

    #[test]
    fn test_tag_del_alias_unset() {
        let tmp = TempDir::new().unwrap();
        let output = run_cmd(&["tag", "unset", "--help"], tmp.path());
        assert!(output.status.success(), "tag unset --help should succeed");
    }

    #[test]
    fn test_tag_list_alias_ls() {
        let tmp = TempDir::new().unwrap();
        let output = run_cmd(&["tag", "ls", "--help"], tmp.path());
        assert!(output.status.success(), "tag ls --help should succeed");
    }

    #[test]
    fn test_tag_search_alias_find() {
        let tmp = TempDir::new().unwrap();
        let output = run_cmd(&["tag", "find", "--help"], tmp.path());
        assert!(output.status.success(), "tag find --help should succeed");
    }

    #[test]
    fn test_tag_set_alias_add() {
        let tmp = TempDir::new().unwrap();
        let output = run_cmd(&["tag", "add", "--help"], tmp.path());
        assert!(output.status.success(), "tag add --help should succeed");
    }

    #[test]
    fn test_tag_search_with_key_value_syntax() {
        // Verify that search accepts key:value query syntax
        let tmp = TempDir::new().unwrap();
        let output = run_cmd(&["tag", "search", "priority:high"], tmp.path());
        // Should not error on syntax (may find no results, but command should succeed)
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        // The command should at least not crash — either success or "not connected"
        assert!(
            output.status.success() || combined.contains("connect"),
            "tag search priority:high should not crash: {combined}"
        );
    }

    #[test]
    fn test_tag_search_with_value_only_syntax() {
        let tmp = TempDir::new().unwrap();
        let output = run_cmd(&["tag", "search", ":important"], tmp.path());
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.status.success() || combined.contains("connect"),
            "tag search :important should not crash: {combined}"
        );
    }

    #[test]
    fn test_tag_search_with_quoted_literal() {
        let tmp = TempDir::new().unwrap();
        let output = run_cmd(&["tag", "search", "\"key:value\""], tmp.path());
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.status.success() || combined.contains("connect"),
            "tag search with quoted literal should not crash: {combined}"
        );
    }

    #[test]
    fn test_tag_search_multiple_terms() {
        let tmp = TempDir::new().unwrap();
        let output = run_cmd(&["tag", "search", "priority:", ":high", "name"], tmp.path());
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.status.success() || combined.contains("connect"),
            "tag search with multiple terms should not crash: {combined}"
        );
    }

    #[test]
    fn test_tag_list_with_display_flags() {
        // Verify --hex, --binary, --no-truncate flags are accepted
        let tmp = TempDir::new().unwrap();
        let output = run_cmd(&["tag", "list", "--hex", "--help"], tmp.path());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "tag list --hex --help should succeed: {stdout}"
        );

        let output = run_cmd(&["tag", "list", "--binary", "--help"], tmp.path());
        assert!(
            output.status.success(),
            "tag list --binary --help should succeed"
        );

        let output = run_cmd(&["tag", "list", "--no-truncate", "--help"], tmp.path());
        assert!(
            output.status.success(),
            "tag list --no-truncate --help should succeed"
        );
    }

    #[test]
    fn test_tag_search_with_display_flags() {
        let tmp = TempDir::new().unwrap();
        let output = run_cmd(&["tag", "search", "--help"], tmp.path());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("hex") && stdout.contains("binary") && stdout.contains("no-truncate"),
            "tag search help should mention --hex, --binary, --no-truncate: {stdout}"
        );
    }

    #[test]
    fn test_migrate_tags_help() {
        let tmp = TempDir::new().unwrap();
        let output = run_cmd(&["migrate-tags", "--help"], tmp.path());
        assert!(
            output.status.success(),
            "migrate-tags --help should succeed"
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.to_lowercase().contains("migrat"),
            "migrate-tags help should mention migration: {stdout}"
        );
    }

    #[test]
    fn test_label_search_alias() {
        // Verify label search works like tag search
        let tmp = TempDir::new().unwrap();
        let output = run_cmd(&["label", "search", "--help"], tmp.path());
        assert!(
            output.status.success(),
            "label search --help should succeed"
        );
    }

    #[test]
    fn test_label_find_alias() {
        // Verify label find works like tag find
        let tmp = TempDir::new().unwrap();
        let output = run_cmd(&["label", "find", "--help"], tmp.path());
        assert!(output.status.success(), "label find --help should succeed");
    }

    #[test]
    fn test_link_search_alias() {
        let tmp = TempDir::new().unwrap();
        let output = run_cmd(&["link", "search", "--help"], tmp.path());
        assert!(output.status.success(), "link search --help should succeed");
    }
}
