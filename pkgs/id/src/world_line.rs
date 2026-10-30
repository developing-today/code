//! One line grammar for the world's three surfaces.
//!
//! The SSH command mode (`cmd:` login), the `id world join` stdin pump, and
//! the `id world` CLI subcommands all speak the same verbs: chat, records,
//! stats, logs, caps and quit. This module is the single parser that maps one
//! line to one [`WorldLine`] action, so a sentence works identically as an SSH
//! command, a piped script line, and a CLI one-off.

/// What one input line asks the world for.
///
/// Each surface maps a [`WorldLine`] to its own execution: SSH turns it into a
/// session frame, the pump into the same frame over stdin, and the CLI into a
/// `WorldCommand` subcommand.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorldLine {
    /// A plain chat line.
    Chat(String),
    /// `/input <hex>`: an opaque participant input, hex-encoded.
    Input(String),
    /// `/records [prefix]`: read the world's structured records.
    Records { prefix: Option<String> },
    /// `/documents [prefix]`: list public documents.
    Documents { prefix: Option<String> },
    /// `/document <name>`: read one public document.
    Document(String),
    /// `/stats`: the world's module, capability and profile state.
    Stats,
    /// `/logs [seconds]`: tail the world's events.
    Logs { seconds: Option<u64> },
    /// `/caps`: the world's capability report.
    Caps,
    /// `/help`: list the commands.
    Help,
    /// `/quit`: end the session.
    Quit,
    /// A command that did not parse; the message is shown to the user.
    Message(String),
}

/// Parse one line (already trimmed) into a [`WorldLine`].
///
/// An empty line returns `None`. Any other line that does not start with `/`
/// is chat. Unknown `/` commands become [`WorldLine::Message`].
#[must_use]
pub fn parse_line(line: &str) -> Option<WorldLine> {
    if line.is_empty() {
        return None;
    }
    match line {
        "/help" => return Some(WorldLine::Help),
        "/quit" => return Some(WorldLine::Quit),
        "/stats" => return Some(WorldLine::Stats),
        "/caps" => return Some(WorldLine::Caps),
        _ => {}
    }
    if line == "/records" || line.starts_with("/records ") {
        let Some(prefix) = line.strip_prefix("/records") else {
            return Some(WorldLine::Message("usage: /records [prefix]".to_owned()));
        };
        let prefix = prefix.trim();
        return Some(WorldLine::Records {
            prefix: (!prefix.is_empty()).then(|| prefix.to_owned()),
        });
    }
    if line == "/documents" || line.starts_with("/documents ") {
        let Some(prefix) = line.strip_prefix("/documents") else {
            return Some(WorldLine::Message("usage: /documents [prefix]".to_owned()));
        };
        let prefix = prefix.trim();
        return Some(WorldLine::Documents {
            prefix: (!prefix.is_empty()).then(|| prefix.to_owned()),
        });
    }
    if line == "/document" {
        return Some(WorldLine::Message("usage: /document <name>".to_owned()));
    }
    if let Some(rest) = line.strip_prefix("/document ") {
        let name = rest.trim();
        if !name.is_empty() && name == rest {
            return Some(WorldLine::Document(name.to_owned()));
        }
        return Some(WorldLine::Message("usage: /document <name>".to_owned()));
    }
    if line == "/logs" || line.starts_with("/logs ") {
        let rest = line.strip_prefix("/logs").unwrap_or_default().trim();
        if rest.is_empty() {
            return Some(WorldLine::Logs { seconds: None });
        }
        if let Ok(seconds) = rest.parse::<u64>() {
            return Some(WorldLine::Logs {
                seconds: Some(seconds),
            });
        }
        return Some(WorldLine::Message("usage: /logs [seconds]".to_owned()));
    }
    if line == "/input" {
        return Some(WorldLine::Message("usage: /input <hex>".to_owned()));
    }
    if let Some(hex) = line.strip_prefix("/input ") {
        if !hex.is_empty() && hex.len() % 2 == 0 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Some(WorldLine::Input(hex.to_ascii_lowercase()));
        }
        return Some(WorldLine::Message("usage: /input <hex>".to_owned()));
    }
    if line.starts_with('/') {
        return Some(WorldLine::Message("unknown command; type /help".to_owned()));
    }
    Some(WorldLine::Chat(line.to_owned()))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_plain_lines() {
        assert_eq!(parse_line(""), None);
        assert_eq!(
            parse_line("hi there"),
            Some(WorldLine::Chat("hi there".into()))
        );
        assert_eq!(
            parse_line("  padded  "),
            Some(WorldLine::Chat("  padded  ".into())),
            "the caller trims before parsing"
        );
    }

    #[test]
    fn the_shared_verbs_parse_identically() {
        assert_eq!(parse_line("/help"), Some(WorldLine::Help));
        assert_eq!(parse_line("/quit"), Some(WorldLine::Quit));
        assert_eq!(parse_line("/stats"), Some(WorldLine::Stats));
        assert_eq!(parse_line("/caps"), Some(WorldLine::Caps));
        assert_eq!(parse_line("/logs"), Some(WorldLine::Logs { seconds: None }));
        assert_eq!(
            parse_line("/logs 5"),
            Some(WorldLine::Logs { seconds: Some(5) })
        );
        assert_eq!(
            parse_line("/records"),
            Some(WorldLine::Records { prefix: None })
        );
        assert_eq!(
            parse_line("/records abc"),
            Some(WorldLine::Records {
                prefix: Some("abc".into())
            })
        );
        assert_eq!(
            parse_line("/documents notes"),
            Some(WorldLine::Documents {
                prefix: Some("notes".into())
            })
        );
        assert_eq!(
            parse_line("/document notes.md"),
            Some(WorldLine::Document("notes.md".into()))
        );
        assert_eq!(
            parse_line("/input 00aF"),
            Some(WorldLine::Input("00af".into()))
        );
    }

    #[test]
    fn glued_and_bare_verbs_do_not_parse_as_commands() {
        // #1507: every verb needs its boundary — exact, or the verb, a space,
        // and its argument. A glued argument is not the command.
        assert_eq!(
            parse_line("/logs5"),
            Some(WorldLine::Message("unknown command; type /help".into()))
        );
        assert_eq!(
            parse_line("/logs55"),
            Some(WorldLine::Message("unknown command; type /help".into()))
        );
        assert_eq!(
            parse_line("/logs\t5"),
            Some(WorldLine::Message("unknown command; type /help".into()))
        );
        // A bare verb that takes an argument answers with its usage, like
        // the trailing-space form already did — not "unknown command".
        assert_eq!(
            parse_line("/document"),
            Some(WorldLine::Message("usage: /document <name>".into()))
        );
        assert_eq!(
            parse_line("/input"),
            Some(WorldLine::Message("usage: /input <hex>".into()))
        );
    }

    #[test]
    fn unknown_and_malformed_commands_become_messages() {
        assert_eq!(
            parse_line("/nonsense"),
            Some(WorldLine::Message("unknown command; type /help".into()))
        );
        assert_eq!(
            parse_line("/input nope"),
            Some(WorldLine::Message("usage: /input <hex>".into()))
        );
        assert_eq!(
            parse_line("/logs soon"),
            Some(WorldLine::Message("usage: /logs [seconds]".into()))
        );
    }
}
