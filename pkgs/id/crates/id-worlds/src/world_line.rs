//! One line grammar for the world's three surfaces.
//!
//! The SSH command mode (`cmd:` login), the `id world join` stdin pump, and
//! the `id world` CLI subcommands all speak the same verbs: chat, records,
//! stats, logs, caps, the object-discussion flow verbs of #5873 (anchor,
//! talk, anchors, resolve, queue, flush) and quit. This module is the single parser that maps one
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
    /// `/anchor <target> [type]`: anchor the current conversation to an
    /// object (#5873 section 3). The target is an `id://` reference or a
    /// friendly-name phrase; the optional type is the anchor relation
    /// (about, anchored-to-part, derived-from, talk-for), taken from the
    /// final word when it names one.
    Anchor {
        /// The object the conversation anchors to.
        target: String,
        /// The anchor relation, when the line named one.
        relation: Option<String>,
    },
    /// `/anchor confirm <n>`: confirm the proposed anchor numbered `n`
    /// (#5873 sections 3 and 5.4).
    AnchorConfirm {
        /// The proposal number, as listed by `/anchor list`.
        proposal: u64,
    },
    /// `/anchor list`: list this conversation's anchors and their state.
    AnchorList,
    /// `/talk <target>`: open (or create, per #5871) the object's talk page.
    Talk {
        /// The object whose talk page opens.
        target: String,
    },
    /// `/anchors <target>`: list conversations anchored to an object.
    Anchors {
        /// The object whose anchored conversations are listed.
        target: String,
    },
    /// `/resolve <thread> [into <target>]`: resolve a thread, optionally
    /// into an edit of the target (#5874's mechanics).
    Resolve {
        /// The thread being resolved.
        thread: String,
        /// The edit target, when the line named one after `into`.
        into: Option<String>,
    },
    /// `/queue`: inspect the offline outbox (#5873 section 6).
    Queue,
    /// `/flush`: drain the offline outbox (#5873 section 6).
    Flush,
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
///
/// The verb boundary is uniform for every verb: the verb itself, or the
/// verb followed by an ASCII space and its argument. A tab is not a
/// separator (#4492) — `/help<TAB>me` and `/logs<TAB>5` are both unknown
/// commands — while repeated spaces still parse because the argument
/// remainder is trimmed.
#[must_use]
pub fn parse_line(line: &str) -> Option<WorldLine> {
    if line.is_empty() {
        return None;
    }
    // Split on the ASCII space only, not on whitespace: the bare verbs use
    // the same literal-space boundary as the argument verbs below.
    match line.split(' ').next() {
        Some("/help") => return Some(WorldLine::Help),
        Some("/quit") => return Some(WorldLine::Quit),
        Some("/stats") => return Some(WorldLine::Stats),
        Some("/caps") => return Some(WorldLine::Caps),
        Some("/queue") => return Some(WorldLine::Queue),
        Some("/flush") => return Some(WorldLine::Flush),
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
        // The separator is ASCII spaces, not exactly one space: siblings
        // (/records, /documents, /logs) trim theirs, and a name with
        // interior spaces ("my file.txt") is still one name. A tab is not
        // a separator — the verb boundary is a literal space (#4492).
        let name = rest.trim();
        if !name.is_empty() {
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
    // The object-discussion flow verbs (#5873 section 3). /anchors is
    // checked before /anchor: the names share a prefix, and the verb
    // boundary rule (exact, or verb + space + argument) is what keeps
    // /anchors from parsing as /anchor with a glued argument.
    if line == "/anchors" {
        return Some(WorldLine::Message("usage: /anchors <target>".to_owned()));
    }
    if let Some(rest) = line.strip_prefix("/anchors ") {
        let target = rest.trim();
        if !target.is_empty() {
            return Some(WorldLine::Anchors {
                target: target.to_owned(),
            });
        }
        return Some(WorldLine::Message("usage: /anchors <target>".to_owned()));
    }
    if line == "/anchor" {
        return Some(WorldLine::Message(
            "usage: /anchor <target> [type] | /anchor confirm <n> | /anchor list".to_owned(),
        ));
    }
    if let Some(rest) = line.strip_prefix("/anchor ") {
        let rest = rest.trim();
        if rest == "list" {
            return Some(WorldLine::AnchorList);
        }
        if rest == "confirm" {
            return Some(WorldLine::Message("usage: /anchor confirm <n>".to_owned()));
        }
        if let Some(number) = rest.strip_prefix("confirm ") {
            if let Ok(proposal) = number.trim().parse::<u64>() {
                return Some(WorldLine::AnchorConfirm { proposal });
            }
            return Some(WorldLine::Message("usage: /anchor confirm <n>".to_owned()));
        }
        // The optional [type] is the anchor relation, and it is the final
        // word when it names one; a friendly-name target keeps its spaces
        // otherwise, exactly as /document names do.
        const RELATIONS: [&str; 4] = ["about", "anchored-to-part", "derived-from", "talk-for"];
        if let Some((head, last)) = rest.rsplit_once(' ') {
            let head = head.trim();
            if !head.is_empty() && RELATIONS.contains(&last) {
                return Some(WorldLine::Anchor {
                    target: head.to_owned(),
                    relation: Some(last.to_owned()),
                });
            }
        }
        return Some(WorldLine::Anchor {
            target: rest.to_owned(),
            relation: None,
        });
    }
    if line == "/talk" {
        return Some(WorldLine::Message("usage: /talk <target>".to_owned()));
    }
    if let Some(rest) = line.strip_prefix("/talk ") {
        let target = rest.trim();
        if !target.is_empty() {
            return Some(WorldLine::Talk {
                target: target.to_owned(),
            });
        }
        return Some(WorldLine::Message("usage: /talk <target>".to_owned()));
    }
    if line == "/resolve" {
        return Some(WorldLine::Message(
            "usage: /resolve <thread> [into <target>]".to_owned(),
        ));
    }
    if let Some(rest) = line.strip_prefix("/resolve ") {
        let rest = rest.trim();
        if let Some((thread, target)) = rest.split_once(" into ") {
            let thread = thread.trim();
            let target = target.trim();
            if !thread.is_empty() && !target.is_empty() {
                return Some(WorldLine::Resolve {
                    thread: thread.to_owned(),
                    into: Some(target.to_owned()),
                });
            }
            return Some(WorldLine::Message(
                "usage: /resolve <thread> [into <target>]".to_owned(),
            ));
        }
        if !rest.is_empty() {
            return Some(WorldLine::Resolve {
                thread: rest.to_owned(),
                into: None,
            });
        }
        return Some(WorldLine::Message(
            "usage: /resolve <thread> [into <target>]".to_owned(),
        ));
    }
    if line == "/input" {
        return Some(WorldLine::Message("usage: /input <hex>".to_owned()));
    }
    if let Some(hex) = line.strip_prefix("/input ") {
        let hex = hex.trim();
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
    fn bare_verbs_ignore_space_separated_arguments() {
        assert_eq!(parse_line("/help me"), Some(WorldLine::Help));
        assert_eq!(parse_line("/quit now"), Some(WorldLine::Quit));
        assert_eq!(parse_line("/stats all"), Some(WorldLine::Stats));
        assert_eq!(parse_line("/caps list"), Some(WorldLine::Caps));
        assert_eq!(
            parse_line("/nonsense arg"),
            Some(WorldLine::Message("unknown command; type /help".into()))
        );
        assert_eq!(
            parse_line("/helpme"),
            Some(WorldLine::Message("unknown command; type /help".into()))
        );
    }

    #[test]
    fn extra_separator_whitespace_still_parses() {
        // #1946: the separator after /document and /input is one or more
        // ASCII spaces, not exactly one space — siblings trim theirs.
        assert_eq!(
            parse_line("/document  report.txt"),
            Some(WorldLine::Document("report.txt".into()))
        );
        assert_eq!(
            parse_line("/input  ab"),
            Some(WorldLine::Input("ab".into()))
        );
        // A name with interior spaces is still one name.
        assert_eq!(
            parse_line("/document my file.txt"),
            Some(WorldLine::Document("my file.txt".into()))
        );
        // Extra separator whitespace does not rescue a bad argument.
        assert_eq!(
            parse_line("/input  zz"),
            Some(WorldLine::Message("usage: /input <hex>".into()))
        );
        assert_eq!(
            parse_line("/input  abc"),
            Some(WorldLine::Message("usage: /input <hex>".into()))
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
    fn test_tab_is_not_a_verb_boundary_for_any_verb_class() {
        // #4492: one boundary rule — a tab separates nothing, for bare
        // verbs exactly as for argument verbs (#1507 pinned /logs<TAB>5).
        for line in ["/help\tme", "/stats\tall", "/quit\tnow", "/caps\tlist"] {
            assert_eq!(
                parse_line(line),
                Some(WorldLine::Message("unknown command; type /help".into())),
                "{line:?}"
            );
        }
        for line in [
            "/logs\t5",
            "/records\tfoo",
            "/documents\tfoo",
            "/document\treport.txt",
            "/input\tab",
        ] {
            assert_eq!(
                parse_line(line),
                Some(WorldLine::Message("unknown command; type /help".into())),
                "{line:?}"
            );
        }
    }

    #[test]
    fn test_space_boundary_still_parses_for_bare_and_argument_verbs() {
        assert_eq!(parse_line("/help me"), Some(WorldLine::Help));
        assert_eq!(parse_line("/help  me"), Some(WorldLine::Help));
        assert_eq!(
            parse_line("/logs 5"),
            Some(WorldLine::Logs { seconds: Some(5) })
        );
        assert_eq!(
            parse_line("/logs  5"),
            Some(WorldLine::Logs { seconds: Some(5) })
        );
    }

    #[test]
    fn the_flow_verbs_parse() {
        // #5873 section 3: the object-discussion verbs, in the shared
        // grammar every surface consumes.
        assert_eq!(
            parse_line("/anchor id://abc"),
            Some(WorldLine::Anchor {
                target: "id://abc".into(),
                relation: None
            })
        );
        assert_eq!(
            parse_line("/anchor id://abc#pa1~b~r1~p~blk anchored-to-part"),
            Some(WorldLine::Anchor {
                target: "id://abc#pa1~b~r1~p~blk".into(),
                relation: Some("anchored-to-part".into())
            })
        );
        // A friendly-name target keeps its spaces; a trailing relation
        // word is the type, and only then.
        assert_eq!(
            parse_line("/anchor the design doc about"),
            Some(WorldLine::Anchor {
                target: "the design doc".into(),
                relation: Some("about".into())
            })
        );
        assert_eq!(
            parse_line("/anchor the design doc"),
            Some(WorldLine::Anchor {
                target: "the design doc".into(),
                relation: None
            })
        );
        assert_eq!(
            parse_line("/anchor confirm 2"),
            Some(WorldLine::AnchorConfirm { proposal: 2 })
        );
        assert_eq!(parse_line("/anchor list"), Some(WorldLine::AnchorList));
        assert_eq!(
            parse_line("/talk id://abc"),
            Some(WorldLine::Talk {
                target: "id://abc".into()
            })
        );
        assert_eq!(
            parse_line("/anchors the design doc"),
            Some(WorldLine::Anchors {
                target: "the design doc".into()
            })
        );
        assert_eq!(
            parse_line("/resolve thread-9"),
            Some(WorldLine::Resolve {
                thread: "thread-9".into(),
                into: None
            })
        );
        assert_eq!(
            parse_line("/resolve thread-9 into id://abc"),
            Some(WorldLine::Resolve {
                thread: "thread-9".into(),
                into: Some("id://abc".into())
            })
        );
        assert_eq!(parse_line("/queue"), Some(WorldLine::Queue));
        assert_eq!(parse_line("/flush"), Some(WorldLine::Flush));
    }

    #[test]
    fn the_flow_verbs_keep_the_verb_boundary() {
        // /anchors is not /anchor with a glued argument, and a tab is not
        // a separator for the new verbs either (#4492's one boundary rule).
        assert_eq!(
            parse_line("/anchorx id://abc"),
            Some(WorldLine::Message("unknown command; type /help".into()))
        );
        assert_eq!(
            parse_line("/talk\tid://abc"),
            Some(WorldLine::Message("unknown command; type /help".into()))
        );
        assert_eq!(
            parse_line("/queue\tnow"),
            Some(WorldLine::Message("unknown command; type /help".into()))
        );
        // Bare argument verbs answer with their usage, like /document.
        assert_eq!(
            parse_line("/anchor"),
            Some(WorldLine::Message(
                "usage: /anchor <target> [type] | /anchor confirm <n> | /anchor list".into()
            ))
        );
        assert_eq!(
            parse_line("/anchor confirm later"),
            Some(WorldLine::Message("usage: /anchor confirm <n>".into()))
        );
        assert_eq!(
            parse_line("/anchors"),
            Some(WorldLine::Message("usage: /anchors <target>".into()))
        );
        assert_eq!(
            parse_line("/talk"),
            Some(WorldLine::Message("usage: /talk <target>".into()))
        );
        assert_eq!(
            parse_line("/resolve"),
            Some(WorldLine::Message(
                "usage: /resolve <thread> [into <target>]".into()
            ))
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
