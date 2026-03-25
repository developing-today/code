//! REPL runner - main interactive loop and command dispatch.
//!
//! This module contains the main REPL loop ([`run_repl`]) and command
//! dispatch logic. It handles user input, preprocessing, command parsing,
//! and execution.
//!
//! # Main Loop
//!
//! The REPL loop:
//!
//! 1. Displays prompt (`> `)
//! 2. Reads input line
//! 3. Handles special cases (Ctrl+C, Ctrl+D, shell escape)
//! 4. Preprocesses the line (see [`input`](super::input) module)
//! 5. Dispatches to appropriate command handler
//! 6. Displays result or error
//! 7. Repeats until quit
//!
//! # Exit Conditions
//!
//! The REPL exits when:
//! - User types `quit`, `exit`, or `q`
//! - User presses Ctrl+D (EOF)
//! - User presses Ctrl+C twice in a row
//!
//! # Command Dispatch
//!
//! Commands are parsed as whitespace-separated tokens and matched against
//! known patterns. The first token is the command name, with optional
//! `@NODE_ID` as second token for remote targeting.
//!
//! # Error Handling
//!
//! Errors from command execution are caught and displayed to the user,
//! but don't terminate the REPL. This allows the user to try again or
//! correct their input.

use anyhow::Result;
use rustyline::{DefaultEditor, error::ReadlineError};
use std::io::Write;

use super::{ReplInput, continue_heredoc, preprocess_repl_line};
use crate::{
    FindMatch, MatchKind, PeekOptions, ReplContext, SearchOptions, is_node_id, print_match_repl,
};

/// Run the interactive REPL.
///
/// This is the main entry point for the REPL. It creates a [`ReplContext`],
/// sets up readline, and runs the main input loop.
///
/// # Arguments
///
/// * `target_node` - Optional remote node ID to connect to.
///   If provided, connects to that remote peer for all operations.
///
/// # Features
///
/// - **Command history**: Uses rustyline for readline functionality
/// - **Shell escape**: Lines starting with `!` are executed as shell commands
/// - **Graceful exit**: Ctrl+C once warns, twice exits; Ctrl+D exits immediately
/// - **Error recovery**: Command errors are displayed but don't exit the REPL
///
/// # Example Session
///
/// ```text
/// $ id repl
/// id repl (local-serve)
/// commands: list, put, get, cat, gethash, help, quit
/// input: $(...), `...`, |>, <<<, <<EOF supported
/// > list
/// abc123...  config.json
/// def456...  data.txt
/// > cat config.json
/// {"key": "value"}
/// > !ls -la
/// total 16
/// drwxr-xr-x  3 user user 4096 Jan  1 12:00 .
/// > quit
/// $
/// ```
///
/// # Errors
///
/// Returns an error if:
/// - Context creation fails (see [`ReplContext::new`])
/// - Readline initialization fails
/// - Context shutdown fails
pub async fn run_repl(target_node: Option<String>) -> Result<()> {
    let mut ctx = ReplContext::new(target_node).await?;
    println!("id repl ({})", ctx.mode_str());
    println!("commands: list, put, get, cat, gethash, peers, help, quit");
    println!("input: $(...), `...`, |>, <<<, <<EOF supported");

    let mut rl = DefaultEditor::new()?;
    let mut ctrl_c_count = 0u8;

    loop {
        match rl.readline("> ") {
            Ok(raw_line) => {
                ctrl_c_count = 0; // Reset on any input
                let raw_line = raw_line.trim();
                if raw_line.is_empty() {
                    continue;
                }
                let _ = rl.add_history_entry(raw_line);

                // Shell escape: !command (no preprocessing)
                if let Some(cmd) = raw_line.strip_prefix('!') {
                    let cmd = cmd.trim();
                    if !cmd.is_empty() {
                        let status = std::process::Command::new("sh").arg("-c").arg(cmd).status();
                        match status {
                            Ok(s) if !s.success() => {
                                if let Some(code) = s.code() {
                                    println!("exit: {code}");
                                }
                            }
                            Err(e) => println!("error: {e}"),
                            _ => {}
                        }
                    }
                    continue;
                }

                // Preprocess the line (handle $(), ``, |>, <<<, <<)
                let line = match preprocess_repl_line(raw_line) {
                    Ok(ReplInput::Empty) => continue,
                    Ok(ReplInput::Ready(line)) => line,
                    Ok(ReplInput::NeedMore {
                        delimiter,
                        mut lines,
                        original_line,
                    }) => {
                        // Heredoc mode - read until delimiter
                        match continue_heredoc(&mut rl, &delimiter, &mut lines) {
                            Ok(Some(content)) => {
                                // Replace - with content marker in original line
                                original_line
                                    .replace(" - ", &format!(" __STDIN_CONTENT__:{content} "))
                                    .replace(" -$", &format!(" __STDIN_CONTENT__:{content}"))
                            }
                            Ok(None) => continue, // Cancelled
                            Err(e) => {
                                println!("error: {e}");
                                continue;
                            }
                        }
                    }
                    Err(e) => {
                        println!("error: {e}");
                        continue;
                    }
                };

                // Execute command and handle result
                let result = execute_repl_command(&mut ctx, &mut rl, &line).await;

                // Check for quit signal
                if matches!(result, Ok(ReplAction::Quit)) {
                    break;
                }

                if let Err(e) = result {
                    println!("error: {e}");
                }
            }
            Err(ReadlineError::Interrupted) => {
                ctrl_c_count += 1;
                if ctrl_c_count >= 2 {
                    println!("^C");
                    break;
                }
                println!("^C (press Ctrl+C again, Ctrl+D, or type 'quit' to exit)");
            }
            Err(ReadlineError::Eof) => {
                break;
            }
            Err(e) => {
                println!("readline error: {e}");
                break;
            }
        }
    }

    ctx.shutdown().await?;
    Ok(())
}

/// Action returned by command execution to control the REPL loop.
///
/// Commands return this enum to indicate whether the REPL should
/// continue running or exit.
#[derive(Debug)]
pub enum ReplAction {
    /// Continue the REPL loop (default for most commands).
    Continue,
    /// Exit the REPL (returned by quit/exit commands).
    Quit,
}

/// Execute a single REPL command.
///
/// This function parses the command line and dispatches to the appropriate
/// handler method on [`ReplContext`].
///
/// # Command Format
///
/// Commands follow this general format:
/// ```text
/// <command> [@NODE_ID] [arguments...]
/// ```
///
/// Where `@NODE_ID` is an optional remote target (64 hex chars prefixed with @).
///
/// # Supported Commands
///
/// ## Storage Commands
/// - `list`, `ls`: List stored files
/// - `put <FILE> [NAME]`: Store a file
/// - `get <NAME> [OUTPUT]`: Retrieve a file
/// - `cat <NAME>`: Print file to stdout
/// - `gethash <HASH> <OUTPUT>`: Retrieve by hash
/// - `delete`, `rm <NAME>`: Delete a file
/// - `rename <FROM> <TO>`: Rename a file
/// - `copy`, `cp <FROM> <TO>`: Copy a file
///
/// ## Discovery Commands
/// - `peers`: List discovered peers
///
/// ## Search Commands
/// - `find <QUERY>...`: Find and output matches
/// - `search <QUERY>...`: List matches
///
/// ## Control Commands
/// - `help`, `?`: Show help
/// - `quit`, `exit`, `q`: Exit REPL
///
/// # Returns
///
/// - `Ok(ReplAction::Continue)` for most commands
/// - `Ok(ReplAction::Quit)` for quit/exit commands
/// - `Err(...)` on command execution failure
async fn execute_repl_command(
    ctx: &mut ReplContext,
    rl: &mut DefaultEditor,
    line: &str,
) -> Result<ReplAction> {
    // Special handling for __STDIN_CONTENT__: marker
    if line.contains("__STDIN_CONTENT__:") {
        return handle_stdin_content(ctx, line).await;
    }

    let parts: Vec<&str> = line.split_whitespace().collect();

    // Check for @NODE_ID prefix on commands
    let (target_node, cmd_parts) = parse_target_node(&parts);

    match (target_node, cmd_parts.as_slice()) {
        // Commands with @NODE_ID target
        (Some(node), ["list" | "ls"]) => {
            ctx.list_on_node(node).await?;
            Ok(ReplAction::Continue)
        }
        (Some(node), ["put", path]) => {
            ctx.put_on_node(node, path, None).await?;
            Ok(ReplAction::Continue)
        }
        (Some(node), ["put", path, name]) => {
            ctx.put_on_node(node, path, Some(name)).await?;
            Ok(ReplAction::Continue)
        }
        (Some(node), ["get", name]) => {
            ctx.get_on_node(node, name, None).await?;
            Ok(ReplAction::Continue)
        }
        (Some(node), ["get", name, output]) => {
            ctx.get_on_node(node, name, Some(output)).await?;
            Ok(ReplAction::Continue)
        }
        (Some(node), ["cat", name]) => {
            ctx.get_on_node(node, name, Some("-")).await?;
            Ok(ReplAction::Continue)
        }
        (Some(node), ["delete" | "rm", name]) => {
            ctx.delete_on_node(node, name).await?;
            Ok(ReplAction::Continue)
        }
        (Some(node), ["peers"]) => {
            ctx.peers_on_node(node).await?;
            Ok(ReplAction::Continue)
        }
        (Some(_node), _) => {
            println!("@NODE_ID not supported for this command");
            Ok(ReplAction::Continue)
        }

        // Regular commands (no @NODE_ID)
        (None, ["quit" | "exit" | "q"]) => Ok(ReplAction::Quit),
        (None, ["help" | "?"]) => {
            print_help();
            Ok(ReplAction::Continue)
        }
        (None, ["list" | "ls"]) => {
            ctx.list().await?;
            Ok(ReplAction::Continue)
        }
        (None, ["put" | "in", path]) => {
            ctx.put(path, None).await?;
            Ok(ReplAction::Continue)
        }
        (None, ["put" | "in", path, name]) => {
            ctx.put(path, Some(name)).await?;
            Ok(ReplAction::Continue)
        }
        (None, ["get", name]) => {
            ctx.get(name, None).await?;
            Ok(ReplAction::Continue)
        }
        (None, ["get", name, output]) => {
            ctx.get(name, Some(output)).await?;
            Ok(ReplAction::Continue)
        }
        (None, ["cat" | "output" | "out", name]) => {
            ctx.get(name, Some("-")).await?;
            Ok(ReplAction::Continue)
        }
        (None, ["gethash", hash, output]) => {
            ctx.gethash(hash, output).await?;
            Ok(ReplAction::Continue)
        }
        (None, ["delete" | "rm", name]) => {
            ctx.delete(name).await?;
            Ok(ReplAction::Continue)
        }
        (None, ["rename", from, to]) => {
            ctx.rename(from, to).await?;
            Ok(ReplAction::Continue)
        }
        (None, ["copy" | "cp", from, to]) => {
            ctx.copy(from, to).await?;
            Ok(ReplAction::Continue)
        }
        (None, ["peers"]) => {
            ctx.peers().await?;
            Ok(ReplAction::Continue)
        }
        (None, ["find", rest @ ..]) => {
            handle_find_command(ctx, rl, rest).await?;
            Ok(ReplAction::Continue)
        }
        (None, ["search", rest @ ..]) => {
            handle_search_command(ctx, rest).await?;
            Ok(ReplAction::Continue)
        }
        (None, ["tag", "set", subject, key]) => {
            ctx.set_tag(subject, key, None).await?;
            Ok(ReplAction::Continue)
        }
        (None, ["tag", "set", subject, key, value]) => {
            ctx.set_tag(subject, key, Some(value)).await?;
            Ok(ReplAction::Continue)
        }
        (None, ["tag", "del" | "rm", subject, key]) => {
            ctx.del_tag(subject, key, None).await?;
            Ok(ReplAction::Continue)
        }
        (None, ["tag", "del" | "rm", subject, key, value]) => {
            ctx.del_tag(subject, key, Some(value)).await?;
            Ok(ReplAction::Continue)
        }
        (None, ["tags"]) => {
            ctx.get_tags(None).await?;
            Ok(ReplAction::Continue)
        }
        (None, ["tags", subject]) => {
            ctx.get_tags(Some(subject)).await?;
            Ok(ReplAction::Continue)
        }
        (None, ["tag", "search", key]) => {
            ctx.search_tags(Some(key), None).await?;
            Ok(ReplAction::Continue)
        }
        (None, ["tag", "search", key, value]) => {
            ctx.search_tags(Some(key), Some(value)).await?;
            Ok(ReplAction::Continue)
        }
        (None, ["show" | "view", rest @ ..]) => {
            handle_show_command(ctx, rest).await?;
            Ok(ReplAction::Continue)
        }
        (None, ["peek", rest @ ..]) => {
            handle_peek_command(ctx, rest).await?;
            Ok(ReplAction::Continue)
        }
        _ => {
            println!("unknown command: {line}");
            println!("type 'help' for available commands");
            Ok(ReplAction::Continue)
        }
    }
}

/// Handle commands containing the `__STDIN_CONTENT__:` marker.
///
/// This function processes commands where content was inlined via
/// preprocessing (from `$()`, `<<<`, or `|>`). It extracts the content
/// and name, then calls the appropriate command handler.
///
/// # Marker Format
///
/// The marker format is: `__STDIN_CONTENT__:<content> <name>`
///
/// For example, `put __STDIN_CONTENT__:hello world greeting` becomes
/// a put command with content "hello world" and name "greeting".
async fn handle_stdin_content(ctx: &mut ReplContext, line: &str) -> Result<ReplAction> {
    if let Some(start) = line.find("__STDIN_CONTENT__:") {
        let before = line[..start].trim();
        let after_marker = &line[start + 18..]; // 18 = len("__STDIN_CONTENT__:")

        // Find the last whitespace-separated token (the name)
        let after_trimmed = after_marker.trim();
        if let Some(last_space) = after_trimmed.rfind(' ') {
            let content = &after_trimmed[..last_space];
            let name = &after_trimmed[last_space + 1..];

            if before == "put" {
                let content_marker = format!("__STDIN_CONTENT__:{content}");
                ctx.put(&content_marker, Some(name)).await?;
            } else {
                println!("unknown command with content: {before}");
            }
        } else {
            println!("error: content requires a name (e.g., put $(cmd) name.txt)");
        }
    }
    Ok(ReplAction::Continue)
}

/// Parse `@NODE_ID` prefix from command parts.
///
/// Checks if the second token is a valid `@NODE_ID` (@ followed by 64 hex chars).
/// If so, returns the node ID and remaining parts; otherwise returns None and
/// original parts.
///
/// # Examples
///
/// ```rust,ignore
/// // With @NODE_ID
/// let (node, parts) = parse_target_node(&["list", "@abc123..."]);
/// assert!(node.is_some());
/// assert_eq!(parts, vec!["list"]);
///
/// // Without @NODE_ID
/// let (node, parts) = parse_target_node(&["list"]);
/// assert!(node.is_none());
/// assert_eq!(parts, vec!["list"]);
/// ```
fn parse_target_node<'a>(parts: &[&'a str]) -> (Option<&'a str>, Vec<&'a str>) {
    if parts.len() >= 2
        && let Some(node_str) = parts[1].strip_prefix('@')
        && is_node_id(node_str)
    {
        let mut new_parts = vec![parts[0]];
        new_parts.extend(&parts[2..]);
        return (Some(node_str), new_parts);
    }
    (None, parts.to_vec())
}

/// Print the REPL help message.
///
/// Displays all available commands, their syntax, and usage examples
/// for remote targeting and input methods.
fn print_help() {
    println!("commands:");
    println!("  list                   - List all stored files");
    println!("  put <FILE> [NAME]      - Store file (NAME defaults to filename)");
    println!("  get <NAME> [OUTPUT]    - Retrieve file (OUTPUT defaults to NAME, - for stdout)");
    println!("  cat <NAME>             - Print file to stdout");
    println!("  gethash <HASH> <OUTPUT> - Retrieve by hash (- for stdout)");
    println!("  delete <NAME>          - Delete a file (alias: rm)");
    println!("  rename <FROM> <TO>     - Rename a file");
    println!("  copy <FROM> <TO>       - Copy a file (alias: cp)");
    println!("  peers                  - List discovered peers");
    println!("  find <QUERY>...        - Find & output matches");
    println!("  search <QUERY>...      - List matches");
    println!("  tags [FILE]            - List all tags (or tags for FILE)");
    println!("  tag set <FILE> <KEY> [VALUE] - Set a metadata tag");
    println!("  tag del <FILE> <KEY> [VALUE] - Delete a metadata tag");
    println!("  tag search <KEY> [VALUE]     - Search tags by key/value");
    println!("  show <QUERY>...        - Find & cat to stdout (alias: view)");
    println!("  peek <QUERY>...        - Preview with head/tail display");
    println!("  !<cmd>                 - Run shell command");
    println!("  help                   - Show this help");
    println!("  quit                   - Exit repl");
    println!();
    println!("search/find flags:");
    println!("  --name                 - Prefer name matches over hash matches");
    println!("  --all                  - Output all matches");
    println!("  --first [N]            - Return first N matches (default 1)");
    println!("  --last [N]             - Return last N matches (default 1)");
    println!("  --count                - Print match count only");
    println!("  --exclude PAT          - Exclude matches containing PAT (repeatable)");
    println!("  --dir <DIR>            - Save matches to directory");
    println!("  --file, >FILE          - Save to file");
    println!();
    println!("peek flags:");
    println!("  --all                  - Peek all matches (excerpt of each)");
    println!("  --lines N, -n N        - Lines to show from head/tail (default 5)");
    println!("  --head-only            - Show only head lines");
    println!("  --tail-only            - Show only tail lines");
    println!("  --chars                - Count by characters instead of lines");
    println!("  --words                - Count by words instead of lines");
    println!("  --quiet, -q            - No header banner");
    println!("  -o FILE                - Output to file");
    println!();
    println!("remote targeting:");
    println!("  list @NODE_ID          - List files on remote node");
    println!("  put @NODE_ID FILE      - Store file on remote node");
    println!("  get @NODE_ID NAME      - Get file from remote node");
    println!("  cat @NODE_ID NAME      - Print remote file to stdout");
    println!("  delete @NODE_ID NAME   - Delete file on remote node");
    println!("  peers @NODE_ID         - List peers known to remote node");
    println!();
    println!("input methods:");
    println!("  put $(cmd) name        - Store output of command");
    println!("  put `cmd` name         - Store output of command (alt)");
    println!("  cmd |> put - name      - Pipe command output to put");
    println!("  put - name <<< 'text'  - Store literal text");
    println!("  put - name <<EOF       - Start heredoc (end with EOF)");
}

/// Parsed arguments for find/search commands.
///
/// This struct holds the parsed options for find and search commands,
/// including queries, flags, and output configuration.
struct FindArgs<'a> {
    /// Search queries (patterns to match)
    queries: Vec<&'a str>,
    /// Prefer name matches over hash matches
    prefer_name: bool,
    /// Output all matches (not just first)
    all: bool,
    /// Explicit output filename (from `>filename`)
    output_file: Option<&'a str>,
    /// Directory to save files to (from `--dir`)
    dir: Option<&'a str>,
    /// Save to file instead of stdout
    to_file: bool,
    /// Output format: "tag", "group", or "union"
    format: &'a str,
    /// Return only first N matches
    first: Option<usize>,
    /// Return only last N matches
    last: Option<usize>,
    /// Print count instead of matches
    count: bool,
    /// Exclude patterns (names/hashes containing these are excluded)
    exclude: Vec<&'a str>,
}

impl FindArgs<'_> {
    /// Convert to `SearchOptions` for filtering.
    fn to_search_options(&self) -> SearchOptions {
        SearchOptions::new(
            self.first,
            self.last,
            self.count,
            self.exclude.iter().map(ToString::to_string).collect(),
        )
    }
}

/// Parse find/search command arguments from tokens.
///
/// Supports:
/// - Multiple queries (non-flag arguments)
/// - `--name`: Prefer name matches
/// - `--all`, `--out`, `--export`, `--save`, `--full`: Output all matches
/// - `--file`: Save to file
/// - `>filename`: Save to specific file
/// - `--dir <path>`: Save all to directory
/// - `--format <fmt>`: Set output format
/// - `--tag`, `--group`, `--union`: Format shortcuts
/// - `--first [N]`: Return only first N matches (default 1)
/// - `--last [N]`: Return only last N matches (default 1)
/// - `--count`: Print count instead of matches
/// - `--exclude PATTERN`: Exclude matches containing pattern (repeatable)
fn parse_find_args<'a>(rest: &[&'a str], default_format: &'a str) -> FindArgs<'a> {
    let mut args = FindArgs {
        queries: Vec::new(),
        prefer_name: false,
        all: false,
        output_file: None,
        dir: None,
        to_file: false,
        format: default_format,
        first: None,
        last: None,
        count: false,
        exclude: Vec::new(),
    };

    let mut i = 0;
    while i < rest.len() {
        let arg = rest[i];
        if arg == "--name" {
            args.prefer_name = true;
        } else if arg == "--all"
            || arg == "--out"
            || arg == "--export"
            || arg == "--save"
            || arg == "--full"
        {
            args.all = true;
        } else if arg == "--file" {
            args.to_file = true;
        } else if let Some(path) = arg.strip_prefix('>') {
            args.output_file = Some(path);
            args.to_file = true;
        } else if arg == "--dir" {
            if i + 1 < rest.len() {
                args.dir = Some(rest[i + 1]);
                i += 1;
            }
        } else if arg == "--format" {
            if i + 1 < rest.len() {
                args.format = rest[i + 1];
                i += 1;
            }
        } else if arg == "--tag" {
            args.format = "tag";
        } else if arg == "--group" {
            args.format = "group";
        } else if arg == "--union" {
            args.format = "union";
        } else if arg == "--first" {
            // --first with optional number argument
            if i + 1 < rest.len() && !rest[i + 1].starts_with('-') {
                if let Ok(n) = rest[i + 1].parse::<usize>() {
                    args.first = Some(n);
                    i += 1;
                } else {
                    args.first = Some(1); // default to 1
                }
            } else {
                args.first = Some(1); // default to 1
            }
        } else if arg == "--last" {
            // --last with optional number argument
            if i + 1 < rest.len() && !rest[i + 1].starts_with('-') {
                if let Ok(n) = rest[i + 1].parse::<usize>() {
                    args.last = Some(n);
                    i += 1;
                } else {
                    args.last = Some(1); // default to 1
                }
            } else {
                args.last = Some(1); // default to 1
            }
        } else if arg == "--count" {
            args.count = true;
        } else if arg == "--exclude" {
            if i + 1 < rest.len() {
                args.exclude.push(rest[i + 1]);
                i += 1;
            }
        } else if !arg.starts_with('-') {
            args.queries.push(arg);
        }
        i += 1;
    }
    args
}

/// Handle the `find` command in the REPL.
///
/// Searches for blobs matching the queries and outputs their content.
/// If multiple matches are found and `--all` is not set, presents an
/// interactive selection prompt.
///
/// # Behavior
///
/// - Single match: Output immediately
/// - Multiple matches: Show numbered list, prompt for selection
/// - `--all` flag: Output all matches without prompting
/// - `--count` flag: Just print count
/// - `--first N` / `--last N`: Limit results
/// - `--exclude PATTERN`: Filter out matches
async fn handle_find_command(
    ctx: &mut ReplContext,
    rl: &mut DefaultEditor,
    rest: &[&str],
) -> Result<()> {
    let args = parse_find_args(rest, "union");

    if args.queries.is_empty() {
        println!(
            "usage: find <query>... [--name] [--all] [--first [N]] [--last [N]] [--count] [--exclude PAT] [--dir <dir>] [--file] [>filename]"
        );
        return Ok(());
    }

    // Collect matches for all queries
    let all_matches = collect_matches(ctx, &args.queries, args.prefer_name).await;

    // Apply filtering and limiting via SearchOptions
    let search_opts = args.to_search_options();
    let filtered_matches = apply_search_options(&all_matches, &search_opts);

    if filtered_matches.is_empty() {
        println!("no matches found for: {}", args.queries.join(", "));
        return Ok(());
    }

    // --count mode: just print the count
    if args.count {
        println!("{}", filtered_matches.len());
        return Ok(());
    }

    // --all mode: output all matches
    if args.all {
        return output_all_matches_filtered(ctx, &filtered_matches, args.dir, args.format).await;
    }

    // Single match
    if filtered_matches.len() == 1 {
        let (_, m) = &filtered_matches[0];
        let output = if args.to_file {
            args.output_file.unwrap_or(&m.name)
        } else {
            "-"
        };
        return ctx.get(&m.name, Some(output)).await;
    }

    // Multiple matches - interactive selection
    select_and_output_matches_filtered(
        ctx,
        rl,
        &filtered_matches,
        args.dir,
        args.output_file,
        args.to_file,
        args.format,
    )
    .await
}

/// Handle the `search` command in the REPL.
///
/// Lists blobs matching the queries without outputting content by default.
/// With `--all` or `--file`, also retrieves the matching files.
async fn handle_search_command(ctx: &mut ReplContext, rest: &[&str]) -> Result<()> {
    let args = parse_find_args(rest, "union");

    if args.queries.is_empty() {
        println!(
            "usage: search <query>... [--name] [--all] [--first [N]] [--last [N]] [--count] [--exclude PAT] [--dir <dir>] [--file] [>filename]"
        );
        return Ok(());
    }

    // Collect matches for all queries
    let all_matches = collect_matches(ctx, &args.queries, args.prefer_name).await;

    // Apply filtering and limiting via SearchOptions
    let search_opts = args.to_search_options();
    let filtered_matches = apply_search_options(&all_matches, &search_opts);

    if filtered_matches.is_empty() {
        println!("no matches found for: {}", args.queries.join(", "));
        return Ok(());
    }

    // --count mode: just print the count
    if args.count {
        println!("{}", filtered_matches.len());
        return Ok(());
    }

    // --all mode: output all matches to files
    if args.all {
        return output_all_matches_filtered(ctx, &filtered_matches, args.dir, args.format).await;
    }

    // Default: list matches
    for (query, m) in &filtered_matches {
        print_match_repl(query, m, args.format);
    }

    // If --file or >filename, also output first match to file
    if args.to_file {
        let (_, m) = &filtered_matches[0];
        let output = args.output_file.unwrap_or(&m.name);
        ctx.get(&m.name, Some(output)).await
    } else {
        Ok(())
    }
}

/// Handle the `show` command in the REPL (find + cat to stdout).
///
/// Searches for blobs matching the queries and outputs their content
/// to stdout (or file with -o).
async fn handle_show_command(ctx: &mut ReplContext, rest: &[&str]) -> Result<()> {
    let args = parse_find_args(rest, "union");

    if args.queries.is_empty() {
        println!(
            "usage: show <query>... [--all] [--first [N]] [--last [N]] [--exclude PAT] [-o FILE]"
        );
        return Ok(());
    }

    // Collect matches for all queries
    let all_matches = collect_matches(ctx, &args.queries, args.prefer_name).await;

    // Apply filtering and limiting
    let search_opts = args.to_search_options();
    let filtered_matches = apply_search_options(&all_matches, &search_opts);

    if filtered_matches.is_empty() {
        println!("no matches found for: {}", args.queries.join(", "));
        return Ok(());
    }

    // Determine output destination
    let output = args.output_file.unwrap_or("-");

    if args.all {
        // Output all matches
        let mut seen = std::collections::HashSet::new();
        for (_, m) in &filtered_matches {
            let key = format!("{}:{}", m.hash, m.name);
            if seen.insert(key)
                && let Err(e) = ctx.get(&m.name, Some(output)).await
            {
                println!("error: {e}");
            }
        }
    } else {
        // Output first match only
        let (_, m) = &filtered_matches[0];
        ctx.get(&m.name, Some(output)).await?;
    }

    Ok(())
}

/// Handle the `peek` command in the REPL (preview with head/tail).
///
/// Searches for blobs and shows a preview with configurable head/tail lines.
async fn handle_peek_command(ctx: &mut ReplContext, rest: &[&str]) -> Result<()> {
    let (args, peek_opts) = parse_peek_args(rest);

    if args.queries.is_empty() {
        println!(
            "usage: peek <query>... [--all] [--lines N] [--head-only] [--tail-only] [--chars] [--words] [--quiet] [-o FILE]"
        );
        return Ok(());
    }

    // Collect matches for all queries
    let all_matches = collect_matches(ctx, &args.queries, args.prefer_name).await;

    // Apply filtering and limiting
    let search_opts = args.to_search_options();
    let filtered_matches = apply_search_options(&all_matches, &search_opts);

    if filtered_matches.is_empty() {
        println!("no matches found for: {}", args.queries.join(", "));
        return Ok(());
    }

    // Determine which matches to peek (deduplicated)
    let mut seen = std::collections::HashSet::new();
    let matches_to_peek: Vec<&(String, FindMatch)> = if args.all {
        filtered_matches
            .iter()
            .filter(|(_, m)| seen.insert(format!("{}:{}", m.hash, m.name)))
            .collect()
    } else {
        // Just first unique match
        filtered_matches
            .iter()
            .filter(|(_, m)| seen.insert(format!("{}:{}", m.hash, m.name)))
            .take(1)
            .collect()
    };

    // Output destination
    let mut out: Box<dyn Write> = if let Some(path) = args.output_file {
        Box::new(std::fs::File::create(path)?)
    } else {
        Box::new(std::io::stdout())
    };

    for (idx, (_, m)) in matches_to_peek.iter().enumerate() {
        if idx > 0 {
            writeln!(out)?;
        }

        // Fetch content to string (via get to temp file)
        let content = fetch_content_for_peek(ctx, m).await?;

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

/// Collect matches for multiple queries.
///
/// Executes find for each query and collects all results into a single
/// vector. Errors for individual queries are printed but don't stop
/// processing of other queries.
async fn collect_matches(
    ctx: &mut ReplContext,
    queries: &[&str],
    prefer_name: bool,
) -> Vec<(String, FindMatch)> {
    let mut all_matches = Vec::new();
    for query in queries {
        match ctx.find(query, prefer_name).await {
            Ok(matches) => {
                for m in matches {
                    all_matches.push((String::from(*query), m));
                }
            }
            Err(e) => {
                println!("error searching for '{query}': {e}");
            }
        }
    }
    all_matches
}

/// Apply `SearchOptions` to filter and limit matches.
fn apply_search_options(
    matches: &[(String, FindMatch)],
    opts: &SearchOptions,
) -> Vec<(String, FindMatch)> {
    // First, apply exclusions
    let filtered: Vec<(String, FindMatch)> = matches
        .iter()
        .filter(|(_, m)| !opts.should_exclude(&m.name, &m.hash.to_string()))
        .cloned()
        .collect();

    // Then apply first/last limiting
    if let Some(n) = opts.first {
        filtered.into_iter().take(n).collect()
    } else if let Some(n) = opts.last {
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

/// Output all filtered matches (for `--all` mode).
async fn output_all_matches_filtered(
    ctx: &mut ReplContext,
    filtered_matches: &[(String, FindMatch)],
    dir: Option<&str>,
    format: &str,
) -> Result<()> {
    if let Some(dir_path) = dir {
        if let Err(e) = std::fs::create_dir_all(dir_path) {
            println!("error creating directory: {e}");
            return Ok(());
        }
        let mut seen = std::collections::HashSet::new();
        for (query, m) in filtered_matches {
            let key = format!("{}:{}", m.hash, m.name);
            if seen.insert(key) {
                let output_path = format!("{}/{}", dir_path, m.name);
                if let Err(e) = ctx.get(&m.name, Some(&output_path)).await {
                    println!("error: {e}");
                } else {
                    print_match_repl(query, m, format);
                }
            }
        }
    } else {
        // Output all to stdout
        let mut seen = std::collections::HashSet::new();
        for (_, m) in filtered_matches {
            let key = format!("{}:{}", m.hash, m.name);
            if seen.insert(key)
                && let Err(e) = ctx.get(&m.name, Some("-")).await
            {
                println!("error: {e}");
            }
        }
    }
    Ok(())
}

/// Interactive selection and output of filtered matches.
#[allow(clippy::if_same_then_else)] // Both branches return Ok(()), but have different side effects
async fn select_and_output_matches_filtered(
    ctx: &mut ReplContext,
    rl: &mut DefaultEditor,
    filtered_matches: &[(String, FindMatch)],
    dir: Option<&str>,
    output_file: Option<&str>,
    to_file: bool,
    format: &str,
) -> Result<()> {
    // Print numbered list
    println!("found {} matches:", filtered_matches.len());
    for (i, (query, m)) in filtered_matches.iter().enumerate() {
        let kind_str = match m.kind {
            MatchKind::Exact => "exact",
            MatchKind::Prefix => "prefix",
            MatchKind::Contains => "contains",
        };
        let match_type = if m.is_hash_match { "hash" } else { "name" };
        match format {
            "tag" => println!(
                "[{}]\t{}\t{}\t{}\t({} {})",
                i + 1,
                query,
                m.hash,
                m.name,
                kind_str,
                match_type
            ),
            "group" => println!(
                "[{}]\t{}\t{}\t({} {})",
                i + 1,
                m.hash,
                m.name,
                kind_str,
                match_type
            ),
            _ => println!(
                "[{}]\t{}\t{}\t({} {}) [{}]",
                i + 1,
                m.hash,
                m.name,
                kind_str,
                match_type,
                query
            ),
        }
    }
    println!("select numbers (e.g., '1 3 5' or '1,2,3') or enter to cancel:");

    if let Ok(sel) = rl.readline("? ") {
        let sel = sel.trim();
        if sel.is_empty() {
            println!("cancelled");
            return Ok(());
        }

        // Parse selection
        let selections: Vec<usize> = sel
            .split([',', ' '])
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.trim().parse::<usize>().ok())
            .filter(|&n| n >= 1 && n <= filtered_matches.len())
            .collect();

        if selections.is_empty() {
            println!("invalid selection");
            return Ok(());
        }

        // Output based on mode
        if let Some(dir_path) = dir {
            if let Err(e) = std::fs::create_dir_all(dir_path) {
                println!("error creating directory: {e}");
                return Ok(());
            }
            for n in &selections {
                let (_, m) = &filtered_matches[n - 1];
                let output_path = format!("{}/{}", dir_path, m.name);
                if let Err(e) = ctx.get(&m.name, Some(&output_path)).await {
                    println!("error: {e}");
                }
                if let Err(e) = ctx.get(&m.name, Some("-")).await {
                    println!("error: {e}");
                }
            }
        } else if to_file {
            for n in &selections {
                let (_, m) = &filtered_matches[n - 1];
                let output = output_file.unwrap_or(&m.name);
                if let Err(e) = ctx.get(&m.name, Some(output)).await {
                    println!("error: {e}");
                }
            }
        } else {
            for n in &selections {
                let (_, m) = &filtered_matches[n - 1];
                if let Err(e) = ctx.get(&m.name, Some("-")).await {
                    println!("error: {e}");
                }
            }
        }
    } else {
        println!("cancelled");
    }
    Ok(())
}

/// Parse peek command arguments.
fn parse_peek_args<'a>(rest: &[&'a str]) -> (FindArgs<'a>, PeekOptions) {
    let mut find_args = FindArgs {
        queries: Vec::new(),
        prefer_name: false,
        all: false,
        output_file: None,
        dir: None,
        to_file: false,
        format: "union",
        first: None,
        last: None,
        count: false,
        exclude: Vec::new(),
    };
    let mut peek_opts = PeekOptions::default();

    let mut i = 0;
    while i < rest.len() {
        let arg = rest[i];
        if arg == "--name" {
            find_args.prefer_name = true;
        } else if arg == "--all" {
            find_args.all = true;
        } else if arg == "-o" || arg == "--output" {
            if i + 1 < rest.len() {
                find_args.output_file = Some(rest[i + 1]);
                i += 1;
            }
        } else if arg == "--first" {
            if i + 1 < rest.len() && !rest[i + 1].starts_with('-') {
                if let Ok(n) = rest[i + 1].parse::<usize>() {
                    find_args.first = Some(n);
                    i += 1;
                } else {
                    find_args.first = Some(1);
                }
            } else {
                find_args.first = Some(1);
            }
        } else if arg == "--last" {
            if i + 1 < rest.len() && !rest[i + 1].starts_with('-') {
                if let Ok(n) = rest[i + 1].parse::<usize>() {
                    find_args.last = Some(n);
                    i += 1;
                } else {
                    find_args.last = Some(1);
                }
            } else {
                find_args.last = Some(1);
            }
        } else if arg == "--exclude" {
            if i + 1 < rest.len() {
                find_args.exclude.push(rest[i + 1]);
                i += 1;
            }
        } else if arg == "--lines" || arg == "-n" {
            if i + 1 < rest.len()
                && let Ok(n) = rest[i + 1].parse::<usize>()
            {
                peek_opts.lines = n;
                i += 1;
            }
        } else if arg == "--head-only" || arg == "--head" {
            peek_opts.head_only = true;
        } else if arg == "--tail-only" || arg == "--tail" {
            peek_opts.tail_only = true;
        } else if arg == "--chars" {
            peek_opts.chars = true;
        } else if arg == "--words" {
            peek_opts.words = true;
        } else if arg == "--quiet" || arg == "-q" {
            peek_opts.quiet = true;
        } else if !arg.starts_with('-') {
            find_args.queries.push(arg);
        }
        i += 1;
    }

    (find_args, peek_opts)
}

/// Fetch content for peek preview.
async fn fetch_content_for_peek(ctx: &mut ReplContext, m: &FindMatch) -> Result<String> {
    use tempfile::NamedTempFile;

    // Create a temp file to fetch into
    let temp_file = NamedTempFile::new()?;
    let temp_path = temp_file.path().to_string_lossy().to_string();

    ctx.get(&m.name, Some(&temp_path)).await?;

    // Read content
    let content = std::fs::read_to_string(&temp_path)?;

    Ok(content)
}

/// Print peek preview.
fn print_peek(
    out: &mut dyn Write,
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
    out: &mut dyn Write,
    name: &str,
    hash: &str,
    content: &str,
    opts: &PeekOptions,
    total_files: usize,
) -> Result<()> {
    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len();
    let n = opts.lines;
    let hash_short = if hash.len() >= 12 { &hash[..12] } else { hash };

    // Print header if not quiet
    if !opts.quiet {
        writeln!(out, "─── {name} ───")?;
        writeln!(
            out,
            "hash: {hash_short}  lines: {total_lines}  files: {total_files}"
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
    out: &mut dyn Write,
    name: &str,
    hash: &str,
    content: &str,
    opts: &PeekOptions,
    total_files: usize,
) -> Result<()> {
    let total_chars = content.chars().count();
    let n = opts.lines; // reuse lines as char count
    let hash_short = if hash.len() >= 12 { &hash[..12] } else { hash };

    if !opts.quiet {
        writeln!(out, "─── {name} ───")?;
        writeln!(
            out,
            "hash: {hash_short}  chars: {total_chars}  files: {total_files}"
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
    out: &mut dyn Write,
    name: &str,
    hash: &str,
    content: &str,
    opts: &PeekOptions,
    total_files: usize,
) -> Result<()> {
    let words: Vec<&str> = content.split_whitespace().collect();
    let total_words = words.len();
    let n = opts.lines; // reuse lines as word count
    let hash_short = if hash.len() >= 12 { &hash[..12] } else { hash };

    if !opts.quiet {
        writeln!(out, "─── {name} ───")?;
        writeln!(
            out,
            "hash: {hash_short}  words: {total_words}  files: {total_files}"
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_target_node_with_node() {
        let parts = vec![
            "list",
            "@0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        ];
        let (node, cmd_parts) = parse_target_node(&parts);
        assert!(node.is_some());
        assert_eq!(cmd_parts, vec!["list"]);
    }

    #[test]
    fn test_parse_target_node_without_node() {
        let parts = vec!["list"];
        let (node, cmd_parts) = parse_target_node(&parts);
        assert!(node.is_none());
        assert_eq!(cmd_parts, vec!["list"]);
    }

    #[test]
    fn test_parse_target_node_invalid_node() {
        let parts = vec!["list", "@invalid"];
        let (node, cmd_parts) = parse_target_node(&parts);
        assert!(node.is_none());
        assert_eq!(cmd_parts, vec!["list", "@invalid"]);
    }

    #[test]
    fn test_parse_find_args_basic() {
        let rest = vec!["query1", "query2"];
        let args = parse_find_args(&rest, "union");
        assert_eq!(args.queries, vec!["query1", "query2"]);
        assert!(!args.prefer_name);
        assert!(!args.all);
        assert_eq!(args.format, "union");
    }

    #[test]
    fn test_parse_find_args_with_flags() {
        let rest = vec!["query", "--name", "--all", "--format", "tag"];
        let args = parse_find_args(&rest, "union");
        assert_eq!(args.queries, vec!["query"]);
        assert!(args.prefer_name);
        assert!(args.all);
        assert_eq!(args.format, "tag");
    }

    #[test]
    fn test_parse_find_args_with_output_file() {
        let rest = vec!["query", ">output.txt"];
        let args = parse_find_args(&rest, "union");
        assert_eq!(args.queries, vec!["query"]);
        assert!(args.to_file);
        assert_eq!(args.output_file, Some("output.txt"));
    }

    #[test]
    fn test_parse_find_args_with_dir() {
        let rest = vec!["query", "--dir", "/tmp/out"];
        let args = parse_find_args(&rest, "union");
        assert_eq!(args.queries, vec!["query"]);
        assert_eq!(args.dir, Some("/tmp/out"));
    }

    #[test]
    fn test_parse_find_args_shorthand_formats() {
        let rest = vec!["query", "--tag"];
        let args = parse_find_args(&rest, "union");
        assert_eq!(args.format, "tag");

        let rest = vec!["query", "--group"];
        let args = parse_find_args(&rest, "union");
        assert_eq!(args.format, "group");

        let rest = vec!["query", "--union"];
        let args = parse_find_args(&rest, "tag");
        assert_eq!(args.format, "union");
    }

    #[test]
    fn test_parse_find_args_first_with_number() {
        let rest = vec!["query", "--first", "5"];
        let args = parse_find_args(&rest, "union");
        assert_eq!(args.first, Some(5));
        assert_eq!(args.last, None);
    }

    #[test]
    fn test_parse_find_args_first_without_number() {
        let rest = vec!["query", "--first"];
        let args = parse_find_args(&rest, "union");
        assert_eq!(args.first, Some(1)); // defaults to 1
    }

    #[test]
    fn test_parse_find_args_last_with_number() {
        let rest = vec!["query", "--last", "3"];
        let args = parse_find_args(&rest, "union");
        assert_eq!(args.last, Some(3));
        assert_eq!(args.first, None);
    }

    #[test]
    fn test_parse_find_args_count() {
        let rest = vec!["query", "--count"];
        let args = parse_find_args(&rest, "union");
        assert!(args.count);
    }

    #[test]
    fn test_parse_find_args_exclude() {
        let rest = vec!["query", "--exclude", ".bak", "--exclude", ".tmp"];
        let args = parse_find_args(&rest, "union");
        assert_eq!(args.exclude, vec![".bak", ".tmp"]);
    }

    #[test]
    fn test_parse_find_args_combined_filters() {
        let rest = vec!["query", "--first", "10", "--exclude", ".bak", "--name"];
        let args = parse_find_args(&rest, "union");
        assert_eq!(args.first, Some(10));
        assert_eq!(args.exclude, vec![".bak"]);
        assert!(args.prefer_name);
    }

    #[test]
    fn test_parse_peek_args_basic() {
        let rest = vec!["readme"];
        let (find_args, peek_opts) = parse_peek_args(&rest);
        assert_eq!(find_args.queries, vec!["readme"]);
        assert_eq!(peek_opts.lines, 5); // default
        assert!(!peek_opts.head_only);
        assert!(!peek_opts.tail_only);
    }

    #[test]
    fn test_parse_peek_args_with_lines() {
        let rest = vec!["readme", "--lines", "10"];
        let (_, peek_opts) = parse_peek_args(&rest);
        assert_eq!(peek_opts.lines, 10);
    }

    #[test]
    fn test_parse_peek_args_head_only() {
        let rest = vec!["readme", "--head-only"];
        let (_, peek_opts) = parse_peek_args(&rest);
        assert!(peek_opts.head_only);
        assert!(!peek_opts.tail_only);
    }

    #[test]
    fn test_parse_peek_args_tail_only() {
        let rest = vec!["readme", "--tail-only"];
        let (_, peek_opts) = parse_peek_args(&rest);
        assert!(!peek_opts.head_only);
        assert!(peek_opts.tail_only);
    }

    #[test]
    fn test_parse_peek_args_chars() {
        let rest = vec!["readme", "--chars"];
        let (_, peek_opts) = parse_peek_args(&rest);
        assert!(peek_opts.chars);
        assert!(!peek_opts.words);
    }

    #[test]
    fn test_parse_peek_args_words() {
        let rest = vec!["readme", "--words"];
        let (_, peek_opts) = parse_peek_args(&rest);
        assert!(!peek_opts.chars);
        assert!(peek_opts.words);
    }

    #[test]
    fn test_parse_peek_args_quiet() {
        let rest = vec!["readme", "-q"];
        let (_, peek_opts) = parse_peek_args(&rest);
        assert!(peek_opts.quiet);
    }

    #[test]
    fn test_parse_peek_args_output_file() {
        let rest = vec!["readme", "-o", "out.txt"];
        let (find_args, _) = parse_peek_args(&rest);
        assert_eq!(find_args.output_file, Some("out.txt"));
    }

    #[test]
    fn test_find_args_to_search_options() {
        let rest = vec!["query", "--first", "5", "--exclude", ".bak"];
        let args = parse_find_args(&rest, "union");
        let opts = args.to_search_options();
        assert_eq!(opts.first, Some(5));
        assert_eq!(opts.exclude, vec![".bak".to_owned()]);
        assert!(!opts.count);
    }
}
