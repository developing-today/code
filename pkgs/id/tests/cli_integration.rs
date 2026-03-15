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
