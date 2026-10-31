//! REPL input preprocessing for shell-like features.
//!
//! This module handles preprocessing of REPL input lines before they are
//! executed as commands. It provides shell-like features that make the
//! REPL more powerful and familiar to command-line users.
//!
//! # Supported Features
//!
//! ## Command Substitution
//!
//! Both `$(...)` and backtick styles are supported:
//!
//! ```text
//! get $(echo filename)     # Execute 'echo filename', use output
//! get `echo filename`      # Same, with backticks
//! get $(echo $(cat list))  # Nested substitution works
//! ```
//!
//! ## Here-String (`<<<`)
//!
//! Inline content for the `put` command:
//!
//! ```text
//! put - name <<< 'literal content'     # Single-quoted
//! put - name <<< "some content"        # Double-quoted
//! put - name <<< unquoted_content      # Unquoted (rest of line)
//! ```
//!
//! ## Heredoc (`<<DELIMITER`)
//!
//! Multi-line input for the `put` command:
//!
//! ```text
//! put - name <<EOF
//! line 1
//! line 2
//! EOF
//! ```
//!
//! ## Pipe Operator (`|>`)
//!
//! Pipe shell command output to a REPL command:
//!
//! ```text
//! echo "hello world" |> put - greeting
//! cat file.txt |> put - backup
//! ```
//!
//! # Quoting Contract
//!
//! Every operator scan in this module — heredoc (`<<`), here-string
//! (`<<<`), `$(` and its closing paren, backticks, and the pipe
//! operator (`|>`) — skips single- and double-quoted spans:
//!
//! - A quoted span is literal until its closing quote; operator
//!   characters inside it are text, not operators.
//! - The scanner does NOT interpret backslash escapes: a quote
//!   character preceded by `\` still opens or closes a span.
//! - An unterminated quote makes the rest of the line quoted, so no
//!   operators are recognized in it.
//! - Quote state tracks the original input only. Captured substitution
//!   output is data, never source: it is appended to the result without
//!   being rescanned, so an apostrophe in a captured filename cannot
//!   open a quote span and hide later substitutions.
//!
//! Full shell grammar is out of scope. The contract exists so quoted
//! arguments containing operator characters (`get "a<<b"`,
//! `get $(echo ')')`) preprocess the way a shell user expects.
//!
//! # Content Markers
//!
//! When preprocessing detects inline content (from `$()`, here-strings, or pipes),
//! it replaces the `-` placeholder with a special marker: `__STDIN_CONTENT__:data`.
//! The REPL runner recognizes this marker and passes the data to the put command.
//!
//! # Processing Flow
//!
//! ```text
//! Raw Input
//!     │
//!     ▼
//! ┌─────────────────────────────────────┐
//! │         preprocess_repl_line        │
//! │  1. Check for heredoc (<<DELIM)     │
//! │  2. Process here-string (<<<)       │
//! │  3. Process $(...) substitution     │
//! │  4. Process `...` substitution      │
//! │  5. Process |> pipe operator        │
//! └─────────────────────────────────────┘
//!     │
//!     ▼
//! ┌─────────────────────────────────────┐
//! │             ReplInput               │
//! │  - Empty: whitespace only           │
//! │  - Ready(line): execute this        │
//! │  - NeedMore: heredoc, read more     │
//! └─────────────────────────────────────┘
//! ```

use anyhow::{Result, bail};
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;

/// Result of preprocessing a REPL input line.
///
/// After preprocessing, a line can be in one of three states:
///
/// - **Empty**: The line was whitespace-only; skip it
/// - **Ready**: The line is ready to execute (possibly modified)
/// - **`NeedMore`**: We're starting a heredoc; read more lines until delimiter
#[derive(Debug)]
pub enum ReplInput {
    /// Line is ready to execute (possibly preprocessed).
    ///
    /// The string may contain `__STDIN_CONTENT__:data` markers for inline content.
    Ready(String),

    /// Need more input - heredoc mode.
    ///
    /// The REPL should continue reading lines until the delimiter is found,
    /// then join them and substitute into the original line.
    NeedMore {
        /// The heredoc delimiter (e.g., "EOF")
        delimiter: String,
        /// Lines collected so far (initially empty)
        lines: Vec<String>,
        /// The original command line (before `<<DELIM`)
        original_line: String,
    },

    /// Empty or whitespace-only input; skip.
    Empty,
}

/// Execute a shell command and capture its stdout.
///
/// This function runs a command through `/bin/sh -c` and returns the
/// trimmed output. Used internally for `$()` and backtick substitution.
///
/// # Arguments
///
/// * `cmd` - Shell command to execute
///
/// # Returns
///
/// The command's stdout with leading/trailing whitespace trimmed.
///
/// # Errors
///
/// Returns an error if:
/// - The shell command cannot be spawned
/// - The command exits with a non-zero status
///
/// # Example
///
/// ```rust,ignore
/// let output = shell_capture("echo hello")?;
/// assert_eq!(output, "hello");
/// ```
pub fn shell_capture(cmd: &str) -> Result<String> {
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .output()
        .map_err(|e| anyhow::anyhow!("failed to execute shell command: {e}"))?;
    if !output.status.success() {
        bail!(
            "command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Find the first occurrence of `needle` in `haystack` that lies
/// outside single- and double-quoted spans, returning its byte index.
///
/// Implements the module's quoting contract: a quoted span is literal
/// until its closing quote, backslash escapes are not interpreted, and
/// an unterminated quote makes the rest of the line quoted (so no
/// occurrence is found in it).
fn find_unquoted(haystack: &str, needle: &str) -> Option<usize> {
    find_unquoted_from(haystack, needle, &[], 0)
}

/// Whether byte index `idx` lies inside a protected span. Protected
/// spans are text an earlier preprocessing stage inserted (captured
/// command output, here-string content): data, not source. Their
/// bytes never toggle quote state and never begin an operator match,
/// in this or any later stage — an apostrophe in captured output
/// (`O'Reilly`) must not read as source quoting and hide a genuine
/// operator behind it (#1585, Codex review).
fn is_protected(protected: &[(usize, usize)], idx: usize) -> bool {
    protected.iter().any(|&(a, b)| idx >= a && idx < b)
}

/// `find_unquoted`, searching from byte offset `from` (a point the
/// caller knows to be unquoted in the source) and treating protected
/// spans as inert.
fn find_unquoted_from(
    haystack: &str,
    needle: &str,
    protected: &[(usize, usize)],
    from: usize,
) -> Option<usize> {
    let mut quote: Option<char> = None;
    for (rel, c) in haystack[from..].char_indices() {
        let i = from + rel;
        if is_protected(protected, i) {
            continue;
        }
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
            }
            None => {
                if c == '\'' || c == '"' {
                    quote = Some(c);
                } else if haystack[i..].starts_with(needle) {
                    return Some(i);
                }
            }
        }
    }
    None
}

/// The protected spans of the content markers a stage inserted into
/// `result`, located by their literal marker text in insertion order.
fn content_spans(result: &str, contents: &[String]) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut cursor = 0;
    for content in contents {
        let marker = format!("__STDIN_CONTENT__:{content}");
        if let Some(pos) = result[cursor..].find(&marker) {
            let start = cursor + pos;
            spans.push((start, start + marker.len()));
            cursor = start + marker.len();
        }
    }
    spans
}

/// Carry the protected spans that lie inside a verbatim-copied source
/// chunk into the rebuilt string: a chunk copied from
/// `src_from..src_from + len` to `dst_base` shifts its spans by the
/// same offset. Spans outside the chunk (consumed by a substitution,
/// or not yet copied) do not carry.
fn remap_spans(
    protected: &[(usize, usize)],
    src_from: usize,
    len: usize,
    dst_base: usize,
    out: &mut Vec<(usize, usize)>,
) {
    for &(a, b) in protected {
        if a >= src_from && b <= src_from + len {
            out.push((dst_base + (a - src_from), dst_base + (b - src_from)));
        }
    }
}

/// Preprocess a REPL input line, handling shell-like features.
///
/// This function transforms the input line by:
///
/// 1. Detecting heredoc start (`<<DELIM`) and returning `NeedMore`
/// 2. Processing here-strings (`<<<`) and substituting content
/// 3. Processing `$(...)` command substitution
/// 4. Processing backtick command substitution
/// 5. Processing `|>` pipe operator
///
/// # Arguments
///
/// * `line` - The raw input line from the REPL
///
/// # Returns
///
/// - `ReplInput::Empty` if the line is whitespace-only
/// - `ReplInput::Ready(processed)` if ready to execute
/// - `ReplInput::NeedMore { ... }` if starting a heredoc
///
/// # Content Markers
///
/// When the `put` command has inline content (from `$()`, `<<<`, or `|>`),
/// the `-` placeholder is replaced with `__STDIN_CONTENT__:content`. This
/// marker is recognized by the REPL runner.
///
/// # Errors
///
/// Returns an error if:
/// - Unterminated `$(...)` or backticks
/// - Unterminated quotes in here-string
/// - Shell command execution fails
///
/// # Example
///
/// ```rust,ignore
/// // Simple command passes through
/// assert!(matches!(
///     preprocess_repl_line("list")?,
///     ReplInput::Ready(s) if s == "list"
/// ));
///
/// // Here-string becomes content marker
/// assert!(matches!(
///     preprocess_repl_line("put - name <<< 'hello'")?,
///     ReplInput::Ready(s) if s.contains("__STDIN_CONTENT__:hello")
/// ));
///
/// // Heredoc starts NeedMore mode
/// assert!(matches!(
///     preprocess_repl_line("put - name <<EOF")?,
///     ReplInput::NeedMore { delimiter, .. } if delimiter == "EOF"
/// ));
/// ```
pub fn preprocess_repl_line(line: &str) -> Result<ReplInput> {
    let line = line.trim();
    if line.is_empty() {
        return Ok(ReplInput::Empty);
    }

    // Check for heredoc: put - name <<EOF
    if let Some(heredoc_start) = find_unquoted(line, "<<") {
        let after = &line[heredoc_start + 2..];
        // Check it's not <<< (here-string)
        if !after.starts_with('<') {
            let delimiter = after.trim().to_owned();
            if !delimiter.is_empty() {
                let original_line = line[..heredoc_start].trim().to_owned();
                return Ok(ReplInput::NeedMore {
                    delimiter,
                    lines: Vec::new(),
                    original_line,
                });
            }
        }
    }

    let mut result = line.to_owned();

    // Protected spans of stage-inserted text, in `result` coordinates.
    // Every later stage searches with them (see is_protected).
    let mut protected: Vec<(usize, usize)> = Vec::new();

    // Process here-string: <<< 'content' or <<< "content" or <<< content
    let mut here_contents: Vec<String> = Vec::new();
    while let Some(pos) = find_unquoted_from(&result, "<<<", &protected, 0) {
        let before = &result[..pos];
        let after = &result[pos + 3..].trim_start();

        // Extract the content (quoted or unquoted)
        let (content, rest) = if let Some(after_quote) = after.strip_prefix('\'') {
            // Single-quoted
            if let Some(end) = after_quote.find('\'') {
                (&after_quote[..end], &after_quote[end + 1..])
            } else {
                bail!("unterminated single quote in here-string");
            }
        } else if let Some(after_quote) = after.strip_prefix('"') {
            // Double-quoted
            if let Some(end) = after_quote.find('"') {
                (&after_quote[..end], &after_quote[end + 1..])
            } else {
                bail!("unterminated double quote in here-string");
            }
        } else {
            // Unquoted - take until end (rest of line is content)
            (after.trim(), "")
        };

        // Replace - with content marker in the command
        let before_str = before.trim();
        let new_before = before_str
            .replace(" - ", &format!(" __STDIN_CONTENT__:{content} "))
            .replace(" -$", &format!(" __STDIN_CONTENT__:{content}"));
        here_contents.push(content.to_owned());
        result = format!("{new_before}{rest}");
        protected = content_spans(&result, &here_contents);
    }

    // Process $(...) command substitution in a single pass over the
    // source text. Quote state tracks the original input only: each
    // search resumes right after the previous substitution's closing
    // paren (a point where the source is known to be unquoted), and
    // captured output is appended to `substituted` without ever being
    // rescanned -- an unmatched quote in captured output (an apostrophe
    // in a filename) must not read as source quoting and hide a later
    // substitution (#1585, Codex review). Text inserted by earlier
    // stages (here-string content) is protected the same way, and this
    // stage's own insertions are recorded as protected spans for the
    // stages after it: source/output separation holds across stages,
    // not only within one pass.
    {
        let source = std::mem::take(&mut result);
        let mut substituted = String::with_capacity(source.len());
        let mut new_protected: Vec<(usize, usize)> = Vec::new();
        let mut pos = 0;
        while let Some(start) = find_unquoted_from(&source, "$(", &protected, pos) {
            let mut depth = 1;
            let mut end = start + 2;
            let mut quote: Option<char> = None;
            for (i, c) in source[start + 2..].char_indices() {
                if is_protected(&protected, start + 2 + i) {
                    continue;
                }
                match quote {
                    Some(q) => {
                        if c == q {
                            quote = None;
                        }
                    }
                    None => match c {
                        '\'' | '"' => quote = Some(c),
                        '(' => depth += 1,
                        ')' => {
                            depth -= 1;
                            if depth == 0 {
                                end = start + 2 + i;
                                break;
                            }
                        }
                        _ => {}
                    },
                }
            }
            if depth != 0 {
                bail!("unterminated $(...) in command");
            }
            let cmd = &source[start + 2..end];
            let output = shell_capture(cmd)?;

            // Check if this $() is the first arg to put - if so, treat as content
            let before = format!("{}{}", substituted, &source[pos..start]);
            let before = before.trim();
            let chunk_base = substituted.len();
            substituted.push_str(&source[pos..start]);
            remap_spans(&protected, pos, start - pos, chunk_base, &mut new_protected);
            let span_start = substituted.len();
            if before == "put" || before.ends_with(" put") {
                substituted.push_str("__STDIN_CONTENT__:");
            }
            substituted.push_str(&output);
            new_protected.push((span_start, substituted.len()));
            pos = end + 1;
        }
        let chunk_base = substituted.len();
        substituted.push_str(&source[pos..]);
        remap_spans(
            &protected,
            pos,
            source.len() - pos,
            chunk_base,
            &mut new_protected,
        );
        protected = new_protected;
        result = substituted;
    }

    // Process `...` backtick substitution, with the same single-pass
    // discipline as $(...) above: captured output is never rescanned,
    // so a backtick or quote inside it cannot corrupt the scan. The
    // protected spans from the earlier stages are honoured here too:
    // an apostrophe in $(...) output sitting before a backtick pair
    // must not open a quote span that hides the pair (#1585, Codex
    // review).
    {
        let source = std::mem::take(&mut result);
        let mut substituted = String::with_capacity(source.len());
        let mut new_protected: Vec<(usize, usize)> = Vec::new();
        let mut pos = 0;
        while let Some(start) = find_unquoted_from(&source, "`", &protected, pos) {
            let closing = source[start + 1..]
                .char_indices()
                .find(|&(i, c)| c == '`' && !is_protected(&protected, start + 1 + i))
                .map(|(i, _)| i);
            if let Some(end) = closing {
                let cmd = &source[start + 1..start + 1 + end];
                let output = shell_capture(cmd)?;

                // Check if this `` is the first arg to put - if so, treat as content
                let before = format!("{}{}", substituted, &source[pos..start]);
                let before = before.trim();
                let chunk_base = substituted.len();
                substituted.push_str(&source[pos..start]);
                remap_spans(&protected, pos, start - pos, chunk_base, &mut new_protected);
                let span_start = substituted.len();
                if before == "put" || before.ends_with(" put") {
                    substituted.push_str("__STDIN_CONTENT__:");
                }
                substituted.push_str(&output);
                new_protected.push((span_start, substituted.len()));
                pos = start + 2 + end;
            } else {
                bail!("unterminated backtick in command");
            }
        }
        let chunk_base = substituted.len();
        substituted.push_str(&source[pos..]);
        remap_spans(
            &protected,
            pos,
            source.len() - pos,
            chunk_base,
            &mut new_protected,
        );
        protected = new_protected;
        result = substituted;
    }

    // Process |> pipe operator: echo hello |> put - name
    if let Some(pos) = find_unquoted_from(&result, "|>", &protected, 0) {
        let left = result[..pos].trim().to_owned();
        let right = result[pos + 2..].trim().to_owned();

        // Execute left side as shell command
        let output = shell_capture(&left)?;

        // Replace - in right side with stdin content marker
        let mut new_result = right
            .replace(" - ", &format!(" __STDIN_CONTENT__:{output} "))
            .replace(" -\n", &format!(" __STDIN_CONTENT__:{output}\n"))
            .replace(" -$", &format!(" __STDIN_CONTENT__:{output}"));

        // If no - found, might be at end
        if !new_result.contains("__STDIN_CONTENT__") {
            // Append content as argument
            new_result = format!("{right} __STDIN_CONTENT__:{output}");
        }
        result = new_result;
    }

    Ok(ReplInput::Ready(result))
}

/// Continue reading heredoc lines until the delimiter is found.
///
/// This function is called after `preprocess_repl_line` returns `NeedMore`.
/// It reads lines from the user until a line matching the delimiter is found,
/// then returns the collected content.
///
/// # Arguments
///
/// * `rl` - The rustyline editor for reading input
/// * `delimiter` - The heredoc delimiter to look for
/// * `lines` - Mutable vector to collect lines (passed from `NeedMore`)
///
/// # Returns
///
/// - `Ok(Some(content))` when delimiter is found; content is joined lines
/// - `Ok(None)` if user cancels (Ctrl+C) or EOF
/// - `Err(...)` on readline error
///
/// # User Experience
///
/// - Prompts with `.. ` to indicate continuation
/// - Prints hint about delimiter and Ctrl+C
/// - Ctrl+C cancels without error
///
/// # Example Flow
///
/// ```text
/// > put - name <<EOF
/// (heredoc: type 'EOF' on its own line to end, Ctrl+C to cancel)
/// .. line 1
/// .. line 2
/// .. EOF
/// stored: name -> abc123...
/// ```
pub fn continue_heredoc(
    rl: &mut DefaultEditor,
    delimiter: &str,
    lines: &mut Vec<String>,
) -> Result<Option<String>> {
    println!("(heredoc: type '{delimiter}' on its own line to end, Ctrl+C to cancel)");

    loop {
        match rl.readline(".. ") {
            Ok(line) => {
                if line.trim() == delimiter {
                    return Ok(Some(lines.join("\n")));
                }
                lines.push(line);
            }
            Err(ReadlineError::Interrupted) => {
                println!("^C (heredoc cancelled)");
                return Ok(None);
            }
            Err(ReadlineError::Eof) => {
                return Ok(None);
            }
            Err(e) => {
                bail!("readline error: {e}");
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn test_preprocess_empty() {
        assert!(matches!(
            preprocess_repl_line("").unwrap(),
            ReplInput::Empty
        ));
        assert!(matches!(
            preprocess_repl_line("   ").unwrap(),
            ReplInput::Empty
        ));
    }

    #[test]
    fn test_preprocess_whitespace_only() {
        assert!(matches!(
            preprocess_repl_line("   \t  ").unwrap(),
            ReplInput::Empty
        ));
    }

    #[test]
    fn test_preprocess_simple() {
        match preprocess_repl_line("list").unwrap() {
            ReplInput::Ready(s) => assert_eq!(s, "list"),
            _ => panic!("expected Ready"),
        }
    }

    #[test]
    fn test_preprocess_simple_with_args() {
        match preprocess_repl_line("put file.txt").unwrap() {
            ReplInput::Ready(s) => assert_eq!(s, "put file.txt"),
            _ => panic!("expected Ready"),
        }
    }

    #[test]
    fn test_preprocess_heredoc_start() {
        match preprocess_repl_line("put - name <<EOF").unwrap() {
            ReplInput::NeedMore {
                delimiter,
                original_line,
                ..
            } => {
                assert_eq!(delimiter, "EOF");
                assert_eq!(original_line, "put - name");
            }
            _ => panic!("expected NeedMore"),
        }
    }

    #[test]
    fn test_preprocess_heredoc_different_delimiter() {
        match preprocess_repl_line("put - test <<END").unwrap() {
            ReplInput::NeedMore { delimiter, .. } => {
                assert_eq!(delimiter, "END");
            }
            _ => panic!("expected NeedMore"),
        }
    }

    #[test]
    fn test_preprocess_heredoc_with_whitespace() {
        match preprocess_repl_line("put - name <<  MARKER  ").unwrap() {
            ReplInput::NeedMore { delimiter, .. } => {
                assert_eq!(delimiter, "MARKER");
            }
            _ => panic!("expected NeedMore"),
        }
    }

    #[test]
    fn test_preprocess_here_string() {
        match preprocess_repl_line("put - name <<< 'hello'").unwrap() {
            ReplInput::Ready(s) => assert!(s.contains("__STDIN_CONTENT__:hello")),
            _ => panic!("expected Ready"),
        }
    }

    #[test]
    fn test_preprocess_here_string_double_quotes() {
        match preprocess_repl_line("put - name <<< \"hello world\"").unwrap() {
            ReplInput::Ready(s) => assert!(s.contains("__STDIN_CONTENT__:hello world")),
            _ => panic!("expected Ready"),
        }
    }

    #[test]
    fn test_preprocess_here_string_unquoted() {
        match preprocess_repl_line("put - name <<< content").unwrap() {
            ReplInput::Ready(s) => assert!(s.contains("__STDIN_CONTENT__:content")),
            _ => panic!("expected Ready"),
        }
    }

    #[test]
    fn test_preprocess_here_string_unterminated_single() {
        let result = preprocess_repl_line("put - name <<< 'unterminated");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("unterminated single quote")
        );
    }

    #[test]
    fn test_preprocess_here_string_unterminated_double() {
        let result = preprocess_repl_line("put - name <<< \"unterminated");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("unterminated double quote")
        );
    }

    #[test]
    fn test_preprocess_quoted_close_paren_in_substitution() {
        match preprocess_repl_line("get $(echo ')')").unwrap() {
            ReplInput::Ready(s) => assert_eq!(s, "get )"),
            _ => panic!("expected Ready"),
        }
    }

    #[test]
    fn test_preprocess_quoted_open_paren_in_substitution() {
        match preprocess_repl_line("get $(echo '(')").unwrap() {
            ReplInput::Ready(s) => assert_eq!(s, "get ("),
            _ => panic!("expected Ready"),
        }
    }

    #[test]
    fn test_preprocess_double_quoted_close_paren_in_substitution() {
        match preprocess_repl_line("get $(echo \")\")").unwrap() {
            ReplInput::Ready(s) => assert_eq!(s, "get )"),
            _ => panic!("expected Ready"),
        }
    }

    #[test]
    fn test_preprocess_quoted_heredoc_marker_inside_substitution() {
        match preprocess_repl_line("get $(printf 'a<<b')").unwrap() {
            ReplInput::Ready(s) => assert_eq!(s, "get a<<b"),
            _ => panic!("expected Ready"),
        }
    }

    #[test]
    fn test_preprocess_quoted_heredoc_marker_in_argument() {
        match preprocess_repl_line("get \"a<<b\"").unwrap() {
            ReplInput::Ready(s) => assert_eq!(s, "get \"a<<b\""),
            _ => panic!("expected Ready"),
        }
    }

    #[test]
    fn test_preprocess_unterminated_quote_makes_rest_of_line_quoted() {
        match preprocess_repl_line("get \"a<<b").unwrap() {
            ReplInput::Ready(s) => assert_eq!(s, "get \"a<<b"),
            _ => panic!("expected Ready"),
        }
    }

    #[test]
    fn test_preprocess_quoted_backtick_is_not_substitution() {
        match preprocess_repl_line("get 'a`b'").unwrap() {
            ReplInput::Ready(s) => assert_eq!(s, "get 'a`b'"),
            _ => panic!("expected Ready"),
        }
    }

    #[test]
    fn test_preprocess_quoted_here_string_marker_is_not_an_operator() {
        match preprocess_repl_line("get 'a<<<b'").unwrap() {
            ReplInput::Ready(s) => assert_eq!(s, "get 'a<<<b'"),
            _ => panic!("expected Ready"),
        }
    }

    #[test]
    fn test_preprocess_quoted_pipe_is_not_an_operator() {
        match preprocess_repl_line("get 'a|>b'").unwrap() {
            ReplInput::Ready(s) => assert_eq!(s, "get 'a|>b'"),
            _ => panic!("expected Ready"),
        }
    }

    #[test]
    fn test_shell_capture_simple() {
        let result = shell_capture("echo hello").unwrap();
        assert_eq!(result, "hello");
    }

    #[test]
    fn test_shell_capture_with_args() {
        let result = shell_capture("echo -n test").unwrap();
        assert_eq!(result, "test");
    }

    #[test]
    fn test_shell_capture_multiline_output() {
        let result = shell_capture("printf 'line1\\nline2'").unwrap();
        // Result is trimmed, so newlines at end are removed
        assert!(result.contains("line1"));
        assert!(result.contains("line2"));
    }

    #[test]
    fn test_shell_capture_failing_command() {
        let result = shell_capture("exit 1");
        assert!(result.is_err());
    }

    #[test]
    fn test_shell_capture_nonexistent_command() {
        let result = shell_capture("nonexistent_command_12345");
        assert!(result.is_err());
    }

    #[test]
    fn test_preprocess_command_substitution_echo() {
        // Note: This test uses actual shell execution
        match preprocess_repl_line("get $(echo test)").unwrap() {
            ReplInput::Ready(s) => assert_eq!(s, "get test"),
            _ => panic!("expected Ready"),
        }
    }

    #[test]
    #[test]
    fn test_preprocess_substitution_output_quotes_do_not_hide_later_substitutions() {
        // Captured output is data, not source: an unmatched apostrophe in
        // the first substitution's output must not open a quote span that
        // hides the second substitution (#1585, Codex review).
        match preprocess_repl_line("get $(printf \"O'Reilly\")$(printf .txt)").unwrap() {
            ReplInput::Ready(s) => assert_eq!(s, "get O'Reilly.txt"),
            other => panic!("expected Ready, got {other:?}"),
        }
    }

    #[test]
    fn test_preprocess_substitution_output_quotes_do_not_hide_backticks() {
        // Cross-stage (#1585, Codex review): the apostrophe arrives via
        // the $(...) stage, and the backtick stage runs after it. The
        // output is data, not source, so the backtick pair behind it is
        // still a substitution.
        match preprocess_repl_line("get $(printf \"O'Reilly\")`printf .txt`").unwrap() {
            ReplInput::Ready(s) => assert_eq!(s, "get O'Reilly.txt"),
            other => panic!("expected Ready, got {other:?}"),
        }
    }

    #[test]
    fn test_preprocess_here_string_content_quotes_do_not_hide_substitutions() {
        // Cross-stage (#1585, Codex review): the here-string stage runs
        // first and inserts its content into the line; the apostrophe
        // in that content must not open a quote span that hides the
        // $(...) substitution the $(...) stage looks for afterwards.
        match preprocess_repl_line("put - $(printf name) <<< \"O'Reilly\"").unwrap() {
            ReplInput::Ready(s) => {
                assert_eq!(s, "put __STDIN_CONTENT__:O'Reilly name");
            }
            other => panic!("expected Ready, got {other:?}"),
        }
    }

    fn test_preprocess_backtick_substitution() {
        match preprocess_repl_line("get `echo hello`").unwrap() {
            ReplInput::Ready(s) => assert_eq!(s, "get hello"),
            _ => panic!("expected Ready"),
        }
    }

    #[test]
    fn test_preprocess_unterminated_dollar_paren() {
        let result = preprocess_repl_line("get $(echo incomplete");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unterminated"));
    }

    #[test]
    fn test_preprocess_unterminated_backtick() {
        let result = preprocess_repl_line("get `echo incomplete");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("unterminated backtick")
        );
    }

    #[test]
    fn test_preprocess_command_substitution_multibyte() {
        // Regression: the closing-paren scan must use byte offsets
        // (char_indices), not char counts — a multibyte character inside
        // $(...) used to make `end` land inside the character and panic
        // on the slice (issue #1583).
        match preprocess_repl_line("get $(echo café)").unwrap() {
            ReplInput::Ready(s) => assert_eq!(s, "get café"),
            _ => panic!("expected Ready"),
        }
    }

    #[test]
    fn test_preprocess_command_substitution_emoji() {
        match preprocess_repl_line("get $(echo ☕)").unwrap() {
            ReplInput::Ready(s) => assert_eq!(s, "get ☕"),
            _ => panic!("expected Ready"),
        }
    }

    #[test]
    fn test_preprocess_command_substitution_two_multibyte() {
        // Two multibyte characters also exercise the remainder slice
        // (&result[end + 1..]), the second former panic site.
        match preprocess_repl_line("get $(echo éé)").unwrap() {
            ReplInput::Ready(s) => assert_eq!(s, "get éé"),
            _ => panic!("expected Ready"),
        }
    }

    #[test]
    fn test_preprocess_put_command_substitution_multibyte() {
        match preprocess_repl_line("put $(echo café) name").unwrap() {
            ReplInput::Ready(s) => assert!(s.contains("__STDIN_CONTENT__:café")),
            _ => panic!("expected Ready"),
        }
    }

    #[test]
    fn test_preprocess_nested_dollar_paren() {
        // Test nested $() - $(echo $(echo inner))
        match preprocess_repl_line("get $(echo $(echo nested))").unwrap() {
            ReplInput::Ready(s) => assert_eq!(s, "get nested"),
            _ => panic!("expected Ready"),
        }
    }

    #[test]
    fn test_preprocess_pipe_operator() {
        // Test |> operator
        match preprocess_repl_line("echo hello |> put - test").unwrap() {
            ReplInput::Ready(s) => {
                assert!(s.contains("__STDIN_CONTENT__:hello"));
                assert!(s.contains("put"));
                assert!(s.contains("test"));
            }
            _ => panic!("expected Ready"),
        }
    }

    #[test]
    fn test_preprocess_put_with_command_substitution() {
        // When $() is first arg to put, treat as content
        match preprocess_repl_line("put $(echo content) name").unwrap() {
            ReplInput::Ready(s) => assert!(s.contains("__STDIN_CONTENT__:content")),
            _ => panic!("expected Ready"),
        }
    }

    #[test]
    fn test_preprocess_put_with_backtick_content() {
        match preprocess_repl_line("put `echo data` myfile").unwrap() {
            ReplInput::Ready(s) => assert!(s.contains("__STDIN_CONTENT__:data")),
            _ => panic!("expected Ready"),
        }
    }

    #[test]
    fn test_repl_input_enum_variants() {
        // Test Ready variant
        let ready = ReplInput::Ready("test".to_owned());
        assert!(matches!(ready, ReplInput::Ready(_)));

        // Test Empty variant
        let empty = ReplInput::Empty;
        assert!(matches!(empty, ReplInput::Empty));

        // Test NeedMore variant
        let need_more = ReplInput::NeedMore {
            delimiter: "EOF".to_owned(),
            lines: vec!["line1".to_owned()],
            original_line: "put - name".to_owned(),
        };
        match need_more {
            ReplInput::NeedMore {
                delimiter,
                lines,
                original_line,
            } => {
                assert_eq!(delimiter, "EOF");
                assert_eq!(lines.len(), 1);
                assert_eq!(original_line, "put - name");
            }
            _ => panic!("expected NeedMore"),
        }
    }
}
