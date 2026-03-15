//! REPL input preprocessing - handles shell substitution, heredocs, etc.

use anyhow::{bail, Result};
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

/// Result of preprocessing a REPL line
#[derive(Debug)]
pub enum ReplInput {
    /// Ready to execute with this line (possibly modified)
    Ready(String),
    /// Need more input - heredoc mode with delimiter
    NeedMore {
        delimiter: String,
        lines: Vec<String>,
        original_line: String,
    },
    /// Empty/whitespace only
    Empty,
}

/// Execute a shell command and return its stdout
pub fn shell_capture(cmd: &str) -> Result<String> {
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .output()
        .map_err(|e| anyhow::anyhow!("failed to execute shell command: {}", e))?;
    if !output.status.success() {
        bail!(
            "command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Preprocess a REPL line, handling:
/// - $(...) and `...` command substitution
/// - <<< here-string
/// - <<DELIM heredoc start
/// - |> pipe operator (cmd |> put - name)
pub fn preprocess_repl_line(line: &str) -> Result<ReplInput> {
    let line = line.trim();
    if line.is_empty() {
        return Ok(ReplInput::Empty);
    }

    // Check for heredoc: put - name <<EOF
    if let Some(heredoc_start) = line.find("<<") {
        let after = &line[heredoc_start + 2..];
        // Check it's not <<< (here-string)
        if !after.starts_with('<') {
            let delimiter = after.trim().to_string();
            if !delimiter.is_empty() {
                let original_line = line[..heredoc_start].trim().to_string();
                return Ok(ReplInput::NeedMore {
                    delimiter,
                    lines: Vec::new(),
                    original_line,
                });
            }
        }
    }

    let mut result = line.to_string();

    // Process here-string: <<< 'content' or <<< "content" or <<< content
    while let Some(pos) = result.find("<<<") {
        let before = &result[..pos];
        let after = &result[pos + 3..].trim_start();

        // Extract the content (quoted or unquoted)
        let (content, rest) = if after.starts_with('\'') {
            // Single-quoted
            if let Some(end) = after[1..].find('\'') {
                (&after[1..end + 1], &after[end + 2..])
            } else {
                bail!("unterminated single quote in here-string");
            }
        } else if after.starts_with('"') {
            // Double-quoted
            if let Some(end) = after[1..].find('"') {
                (&after[1..end + 1], &after[end + 2..])
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
            .replace(" - ", &format!(" __STDIN_CONTENT__:{} ", content))
            .replace(" -$", &format!(" __STDIN_CONTENT__:{}", content));
        result = format!("{}{}", new_before, rest);
    }

    // Process $(...) command substitution
    while let Some(start) = result.find("$(") {
        let mut depth = 1;
        let mut end = start + 2;
        for (i, c) in result[start + 2..].chars().enumerate() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = start + 2 + i;
                        break;
                    }
                }
                _ => {}
            }
        }
        if depth != 0 {
            bail!("unterminated $(...) in command");
        }
        let cmd = &result[start + 2..end];
        let output = shell_capture(cmd)?;

        // Check if this $() is the first arg to put - if so, treat as content
        let before = result[..start].trim();
        if before == "put" || before.ends_with(" put") {
            result = format!(
                "{}__STDIN_CONTENT__:{}{}",
                &result[..start],
                output,
                &result[end + 1..]
            );
        } else {
            result = format!("{}{}{}", &result[..start], output, &result[end + 1..]);
        }
    }

    // Process `...` backtick substitution
    while let Some(start) = result.find('`') {
        if let Some(end) = result[start + 1..].find('`') {
            let cmd = &result[start + 1..start + 1 + end];
            let output = shell_capture(cmd)?;

            // Check if this `` is the first arg to put - if so, treat as content
            let before = result[..start].trim();
            if before == "put" || before.ends_with(" put") {
                result = format!(
                    "{}__STDIN_CONTENT__:{}{}",
                    &result[..start],
                    output,
                    &result[start + 2 + end..]
                );
            } else {
                result = format!(
                    "{}{}{}",
                    &result[..start],
                    output,
                    &result[start + 2 + end..]
                );
            }
        } else {
            bail!("unterminated backtick in command");
        }
    }

    // Process |> pipe operator: echo hello |> put - name
    if let Some(pos) = result.find("|>") {
        let left = result[..pos].trim().to_string();
        let right = result[pos + 2..].trim().to_string();

        // Execute left side as shell command
        let output = shell_capture(&left)?;

        // Replace - in right side with stdin content marker
        let mut new_result = right
            .replace(" - ", &format!(" __STDIN_CONTENT__:{} ", output))
            .replace(" -\n", &format!(" __STDIN_CONTENT__:{}\n", output))
            .replace(" -$", &format!(" __STDIN_CONTENT__:{}", output));

        // If no - found, might be at end
        if !new_result.contains("__STDIN_CONTENT__") {
            // Append content as argument
            new_result = format!("{} __STDIN_CONTENT__:{}", right, output);
        }
        result = new_result;
    }

    Ok(ReplInput::Ready(result))
}

/// Continue reading heredoc lines until delimiter is found
pub fn continue_heredoc(
    rl: &mut DefaultEditor,
    delimiter: &str,
    lines: &mut Vec<String>,
) -> Result<Option<String>> {
    println!(
        "(heredoc: type '{}' on its own line to end, Ctrl+C to cancel)",
        delimiter
    );

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
                bail!("readline error: {}", e);
            }
        }
    }
}

#[cfg(test)]
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
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("unterminated single quote"));
    }

    #[test]
    fn test_preprocess_here_string_unterminated_double() {
        let result = preprocess_repl_line("put - name <<< \"unterminated");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("unterminated double quote"));
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
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("unterminated backtick"));
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
        let ready = ReplInput::Ready("test".to_string());
        assert!(matches!(ready, ReplInput::Ready(_)));

        // Test Empty variant
        let empty = ReplInput::Empty;
        assert!(matches!(empty, ReplInput::Empty));

        // Test NeedMore variant
        let need_more = ReplInput::NeedMore {
            delimiter: "EOF".to_string(),
            lines: vec!["line1".to_string()],
            original_line: "put - name".to_string(),
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
