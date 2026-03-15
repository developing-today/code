//! Integration tests for the ID CLI tool
//!
//! These tests run the actual CLI commands and verify their behavior.
//! They use a separate test store directory to avoid interfering with development data.

use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

/// Get the path to the built binary
fn get_binary_path() -> PathBuf {
    // Use CARGO_MANIFEST_DIR to get absolute path to binary
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let manifest_path = PathBuf::from(manifest_dir);

    // Try debug build first, then release
    let debug_path = manifest_path.join("target/debug/id");
    if debug_path.exists() {
        return debug_path;
    }
    manifest_path.join("target/release/id")
}

/// Run a CLI command and return output
fn run_cmd(args: &[&str], work_dir: &std::path::Path) -> std::process::Output {
    Command::new(get_binary_path())
        .args(args)
        .current_dir(work_dir)
        .output()
        .expect("Failed to execute command")
}

/// Run a CLI command and return stdout as string
fn run_cmd_stdout(args: &[&str], work_dir: &std::path::Path) -> String {
    let output = run_cmd(args, work_dir);
    String::from_utf8_lossy(&output.stdout).to_string()
}

/// Run a CLI command and check it succeeded, returns combined stdout+stderr
fn run_cmd_success(args: &[&str], work_dir: &std::path::Path) -> String {
    let output = run_cmd(args, work_dir);
    if !output.status.success() {
        eprintln!("Command failed: {:?}", args);
        eprintln!("stdout: {}", String::from_utf8_lossy(&output.stdout));
        eprintln!("stderr: {}", String::from_utf8_lossy(&output.stderr));
        panic!("Command failed with exit code: {:?}", output.status.code());
    }
    // Return combined stdout + stderr since some output goes to stderr
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    format!("{}{}", stdout, stderr)
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
            "serve", "put", "get", "list", "find", "search", "repl", "cat",
        ] {
            let output = run_cmd(&[cmd, "--help"], tmp.path());
            assert!(output.status.success(), "Help failed for {}", cmd);
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

        // Get to stdout using cat
        let output = run_cmd_success(&["cat", "test.stdout.txt"], tmp.path());
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

        // Show should output content to stdout
        let output = run_cmd_success(&["show", "test.show-basic.txt"], tmp.path());
        assert_eq!(output.trim(), content);
    }

    #[test]
    fn test_show_alias_view() {
        let tmp = TempDir::new().unwrap();
        let test_file = tmp.path().join("test.view-alias.txt");
        let content = "Content for view alias test";

        fs::write(&test_file, content).unwrap();
        run_cmd_success(&["put", test_file.to_str().unwrap()], tmp.path());

        // view alias should work the same as show
        let output = run_cmd_success(&["view", "test.view-alias.txt"], tmp.path());
        assert_eq!(output.trim(), content);
    }

    #[test]
    fn test_show_partial_match() {
        let tmp = TempDir::new().unwrap();
        let test_file = tmp.path().join("test.show-partial-match.txt");
        let content = "Partial match content";

        fs::write(&test_file, content).unwrap();
        run_cmd_success(&["put", test_file.to_str().unwrap()], tmp.path());

        // Search by partial name
        let output = run_cmd_success(&["show", "show-partial"], tmp.path());
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
            content.push_str(&format!("line{}\n", i));
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
            content.push_str(&format!("line{}\n", i));
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
            content.push_str(&format!("content-line-{}\n", i));
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
            let test_file = tmp.path().join(format!("test.filter-first-{}.txt", i));
            fs::write(&test_file, format!("Content {}", i)).unwrap();
            run_cmd_success(&["put", test_file.to_str().unwrap()], tmp.path());
        }

        // Search with --first flag - use --count to verify
        let output = run_cmd_success(
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
            let test_file = tmp.path().join(format!("test.filter-last-{}.txt", i));
            fs::write(&test_file, format!("Content {}", i)).unwrap();
            run_cmd_success(&["put", test_file.to_str().unwrap()], tmp.path());
        }

        // Search with --last flag - use --count to verify
        let output = run_cmd_success(
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
            let test_file = tmp.path().join(format!("test.filter-count-{}.txt", i));
            fs::write(&test_file, format!("Content {}", i)).unwrap();
            run_cmd_success(&["put", test_file.to_str().unwrap()], tmp.path());
        }

        // Search with --count flag
        let output = run_cmd_success(&["search", "--count", "filter-count"], tmp.path());
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
            let test_file = tmp.path().join(format!("test.find-count-{}.txt", i));
            fs::write(&test_file, format!("Content {}", i)).unwrap();
            run_cmd_success(&["put", test_file.to_str().unwrap()], tmp.path());
        }

        // Find with --count flag
        let output = run_cmd_success(&["find", "--count", "find-count"], tmp.path());
        // Should output just the count
        assert!(output.trim() == "4");
    }

    #[test]
    fn test_find_with_first_default() {
        let tmp = TempDir::new().unwrap();

        // Create multiple files
        for i in 1..=3 {
            let test_file = tmp.path().join(format!("test.find-first-def-{}.txt", i));
            fs::write(&test_file, format!("Content {}", i)).unwrap();
            run_cmd_success(&["put", test_file.to_str().unwrap()], tmp.path());
        }

        // Find with --first (no number) at the end, should default to 1
        let output = run_cmd_success(
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
            let test_file = tmp.path().join(format!("test.combined-{}.txt", i));
            fs::write(&test_file, format!("Content {}", i)).unwrap();
            run_cmd_success(&["put", test_file.to_str().unwrap()], tmp.path());
        }
        // Create one to exclude
        let exclude = tmp.path().join("test.combined-exclude.bak");
        fs::write(&exclude, "Exclude").unwrap();
        run_cmd_success(&["put", exclude.to_str().unwrap()], tmp.path());

        // Search with combined filters - use count to verify
        let output = run_cmd_success(
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
