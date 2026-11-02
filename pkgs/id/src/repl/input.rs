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
//! (`<<<`), `$(` and its closing paren, the opening backtick, and the pipe
//! operator (`|>`) — skips single- and double-quoted spans:
//!
//! - A quoted span is literal until its closing quote; operator
//!   characters inside it are text, not operators.
//! - The scanner does NOT interpret backslash escapes: a quote
//!   character preceded by `\` still opens or closes a span.
//! - An unterminated quote makes the rest of the line quoted, so no
//!   operators are recognized in it.
//! - The closing backtick is not quote-aware. The next backtick that is
//!   not inside captured output closes the substitution, even when that
//!   backtick lies inside quotes. The substitution body is shell code.
//! - Quote state tracks the original input only. Captured substitution
//!   output is data, never source: it is appended to the result without
//!   being rescanned, so an apostrophe in a captured filename cannot
//!   open a quote span and hide later substitutions.
//!
//! Full shell grammar is out of scope. The contract exists so quoted
//! arguments containing operator characters (`get "a<<b"`,
//! `get $(echo ')')`) preprocess the way a shell user expects.
//!
//! # Rescan Boundary
//!
//! Content a stage inserts into the line is a protected span that
//! later stages never rescan:
//!
//! - Here-string content — single-quoted, double-quoted, or
//!   unquoted — is stored literally: a `$(...)` or backtick span
//!   inside it is text, not a substitution (#2281).
//! - `$(...)` output is data, not source: the backtick stage never
//!   rescans it, so backticks in captured output stay literal.
//!
//! Substitutions written in the typed line itself, outside any
//! inserted content, still execute exactly as documented above.
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
/// Whether `cmd` ends with the `-` content placeholder in trailing
/// position — a lone `-` as the final token. The supported form puts
/// the placeholder before the stored name (`put - name ...`); a
/// trailing one was historically "handled" by a literal `" -$"`
/// replace, which is not an end anchor and never matched, so the
/// content addressed to it was silently dropped or misrouted
/// (#2280). All three input forms (here-string, pipe, heredoc)
/// reject the trailing form with the error below instead.
fn has_trailing_placeholder(cmd: &str) -> bool {
    let t = cmd.trim_end();
    t == "-" || t.ends_with(" -")
}

/// `cmd` with every here-string (`<<<` plus its content) removed.
/// Here-string content is literal data, not command syntax: a `-`
/// payload (`<<< -`) is not a trailing placeholder, and a quoted
/// payload is not an operator. Source-level validation that inspects
/// a command containing a here-string must look at the command with
/// the data excluded, or it rejects lines the substitution stages
/// handle correctly (Codex review, PR #3096). An unquoted payload
/// consumes the rest of the line; an unterminated quote drops the
/// remainder here too — the here-string stage itself reports that
/// error when it runs.
fn without_here_strings(cmd: &str) -> String {
    let mut result = cmd.to_owned();
    while let Some(pos) = find_unquoted_from(&result, "<<<", &[], 0) {
        let before = result[..pos].to_owned();
        let after = result[pos + 3..].trim_start().to_owned();
        let rest = match after.chars().next() {
            Some(quote @ ('\'' | '"')) => match after[1..].find(quote) {
                Some(end) => after[1 + end + 1..].to_owned(),
                None => String::new(),
            },
            _ => String::new(),
        };
        result = format!("{before}{rest}");
    }
    result
}

fn trailing_placeholder_error() -> anyhow::Error {
    anyhow::anyhow!(
        "unsupported trailing '-' content placeholder: place '-' before the name (put - name ...)"
    )
}

// Here-string content is removed before pipes are processed. In particular,
// unquoted content consumes the rest of the line, including any `|>`.
fn source_pipe_position(line: &str) -> Option<usize> {
    let mut from = 0;
    loop {
        let pipe = find_unquoted_from(line, "|>", &[], from);
        let here = find_unquoted_from(line, "<<<", &[], from);
        match (pipe, here) {
            (Some(p), Some(h)) if p < h => return Some(p),
            (Some(p), None) => return Some(p),
            (_, Some(h)) => {
                let after = &line[h + 3..];
                let content = after.trim_start();
                let quote = content.chars().next()?;
                if quote != '\'' && quote != '"' {
                    return None;
                }
                let end = content[1..].find(quote)?;
                from = line.len() - content.len() + 1 + end + 1;
            }
            (None, None) => return None,
        }
    }
}

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
                if has_trailing_placeholder(&original_line) {
                    return Err(trailing_placeholder_error());
                }
                return Ok(ReplInput::NeedMore {
                    delimiter,
                    lines: Vec::new(),
                    original_line,
                });
            }
        }
    }

    // Validate the pipe's trailing placeholder on the SOURCE line,
    // before any stage that executes shell code: the $(...) and
    // backtick stages below run substitutions while expanding the
    // left side, so reaching the pipe stage's own check (after those
    // stages) would already have executed `echo $(touch ...)`
    // before reporting the placeholder error -- and a failing
    // substitution would mask the diagnostic with its own error
    // (Codex review, PR #3096).
    if let Some(pos) = source_pipe_position(line) {
        let right = line[pos + 2..].trim();
        // The right side may carry a here-string whose payload is
        // literal data: `echo hi |> put - name <<< -` stores `-`
        // under `name` (the payload fills the mid-line placeholder),
        // so the trailing check must exclude here-string content or
        // it rejects a supported line (Codex review, PR #3096).
        if has_trailing_placeholder(&without_here_strings(right)) {
            return Err(trailing_placeholder_error());
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

        if rest.trim().is_empty() && has_trailing_placeholder(before.trim()) {
            return Err(trailing_placeholder_error());
        }
        // Replace - with content marker in the command
        let before_str = before.trim();
        let new_before = before_str.replace(" - ", &format!(" __STDIN_CONTENT__:{content} "));
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

        if has_trailing_placeholder(&right) {
            return Err(trailing_placeholder_error());
        }

        // Execute left side as shell command
        let output = shell_capture(&left)?;

        // Replace - in right side with stdin content marker
        let mut new_result = right.replace(" - ", &format!(" __STDIN_CONTENT__:{output} "));

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
    fn test_here_string_pipe_is_literal_content() {
        for line in [
            "put - name <<< literal |> -",
            "put - name <<< 'literal |> -'",
            "put - name <<< \"literal |> -\"",
        ] {
            match preprocess_repl_line(line).unwrap() {
                ReplInput::Ready(s) => assert_eq!(s, "put __STDIN_CONTENT__:literal |> - name"),
                _ => panic!("expected Ready"),
            }
        }
    }

    #[test]
    fn test_pipe_after_quoted_here_string_rejected_before_substitution() {
        for quote in ['\'', '"'] {
            let line = format!("echo $(exit 3) <<< {quote}literal |> -{quote} |> put name -");
            let error = preprocess_repl_line(&line).unwrap_err().to_string();
            assert!(
                error.contains("trailing '-' content placeholder"),
                "{error}"
            );
        }
    }

    #[test]
    fn test_pipe_rhs_here_string_dash_payload_is_literal() {
        // Codex review, PR #3096: a real pipe before a here-string
        // whose payload is `-` — the payload is data for the mid-line
        // placeholder, not a trailing placeholder.
        for line in [
            "echo hi |> put - name <<< -",
            "echo hi |> put - name <<< '-'",
            "echo hi |> put - name <<< \"-\"",
        ] {
            match preprocess_repl_line(line).unwrap() {
                ReplInput::Ready(s) => {
                    assert_eq!(s, "put __STDIN_CONTENT__:- name", "{line}")
                }
                _ => panic!("expected Ready for {line}"),
            }
        }
    }

    #[test]
    fn test_pipe_rhs_trailing_dash_with_here_string_still_rejected() {
        // Excluding the here-string payload must not weaken the
        // rejection: a trailing placeholder in the command itself is
        // still an error, raised before the left side executes.
        let result = preprocess_repl_line("exit 3 |> put name - <<< payload");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("trailing '-' content placeholder")
        );
    }

    #[test]
    fn test_preprocess_here_string_trailing_dash_rejected() {
        // #2280: the trailing form was silently dropped by a dead
        // literal " -$" replace; it is rejected consistently now.
        let result = preprocess_repl_line("put name - <<< 'payload'");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("trailing '-' content placeholder")
        );
    }

    #[test]
    fn test_preprocess_pipe_trailing_dash_rejected_before_execution() {
        // The left side would fail if it ran; the placeholder error
        // must win, proving the rejection precedes pipe execution.
        let result = preprocess_repl_line("exit 3 |> put name -");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("trailing '-' content placeholder")
        );
    }

    #[test]
    fn test_preprocess_pipe_trailing_dash_rejected_before_substitution() {
        // The substitution on the left would create a file if it ran;
        // the placeholder error must win and the file must not exist:
        // rejection precedes every stage that executes shell code
        // (Codex review, PR #3096).
        let marker =
            std::env::temp_dir().join(format!("mcpx-repl-side-effect-{}", std::process::id()));
        let _ = std::fs::remove_file(&marker);
        let line = format!("echo $(touch {}) |> put name -", marker.display());
        let result = preprocess_repl_line(&line);
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("trailing '-' content placeholder")
        );
        assert!(
            !marker.exists(),
            "substitution executed before the placeholder rejection"
        );
        // A failing substitution must not mask the diagnostic either.
        let result = preprocess_repl_line("echo $(exit 3) |> put name -");
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("trailing '-' content placeholder")
        );
    }

    #[test]
    fn test_preprocess_pipe_midline_placeholder_substitutes() {
        match preprocess_repl_line("echo hi |> put - name").unwrap() {
            ReplInput::Ready(s) => assert_eq!(s, "put __STDIN_CONTENT__:hi name"),
            _ => panic!("expected Ready"),
        }
    }

    #[test]
    fn test_preprocess_pipe_without_placeholder_appends() {
        match preprocess_repl_line("echo hi |> put name").unwrap() {
            ReplInput::Ready(s) => assert_eq!(s, "put name __STDIN_CONTENT__:hi"),
            _ => panic!("expected Ready"),
        }
    }

    #[test]
    fn test_preprocess_heredoc_trailing_dash_rejected() {
        // Rejected at the start line, before any body is collected.
        let result = preprocess_repl_line("put name - <<EOF");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("trailing '-' content placeholder")
        );
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

    #[test]
    fn test_preprocess_here_string_single_quoted_substitution_is_literal() {
        // #2281 regression: single-quoted here-string content is
        // documented literal; the $(...) stage must not rescan the
        // content the here-string stage inserted and execute it.
        match preprocess_repl_line("put - name <<< 'price $(echo SUBSTITUTED) end'").unwrap() {
            ReplInput::Ready(s) => {
                assert_eq!(
                    s,
                    "put __STDIN_CONTENT__:price $(echo SUBSTITUTED) end name"
                );
            }
            other => panic!("expected Ready, got {other:?}"),
        }
    }

    #[test]
    fn test_preprocess_here_string_single_quoted_backticks_are_literal() {
        // #2281 regression: same boundary for the backtick stage.
        match preprocess_repl_line("put - name <<< 'a `echo TICK` b'").unwrap() {
            ReplInput::Ready(s) => {
                assert_eq!(s, "put __STDIN_CONTENT__:a `echo TICK` b name");
            }
            other => panic!("expected Ready, got {other:?}"),
        }
    }

    #[test]
    fn test_preprocess_substitution_output_backticks_are_not_rescanned() {
        // #2281 regression: $(...) output containing backticks is a
        // protected span; the backtick stage must not execute it.
        match preprocess_repl_line("get $(printf '`echo TICK`')").unwrap() {
            ReplInput::Ready(s) => assert_eq!(s, "get `echo TICK`"),
            other => panic!("expected Ready, got {other:?}"),
        }
    }

    #[test]
    fn test_preprocess_here_string_double_quoted_substitution_is_literal() {
        // #2281 pinned semantics: double-quoted here-string content is
        // literal too — inserted content is never rescanned, whatever
        // quote form carried it.
        match preprocess_repl_line("put - name <<< \"price $(echo SUBSTITUTED) end\"").unwrap() {
            ReplInput::Ready(s) => {
                assert_eq!(
                    s,
                    "put __STDIN_CONTENT__:price $(echo SUBSTITUTED) end name"
                );
            }
            other => panic!("expected Ready, got {other:?}"),
        }
    }

    #[test]
    fn test_preprocess_here_string_unquoted_substitution_is_literal() {
        // #2281 pinned semantics: unquoted here-string content is
        // likewise inserted as a protected span and never rescanned.
        match preprocess_repl_line("put - name <<< price_$(echo SUBSTITUTED)_end").unwrap() {
            ReplInput::Ready(s) => {
                assert_eq!(
                    s,
                    "put __STDIN_CONTENT__:price_$(echo SUBSTITUTED)_end name"
                );
            }
            other => panic!("expected Ready, got {other:?}"),
        }
    }

    #[test]
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
