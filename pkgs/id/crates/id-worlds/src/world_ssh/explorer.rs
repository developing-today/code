//! World-explorer presentation for the SSH shell: the command-mode
//! line parser, the line buffer, the records/documents renderers,
//! and the read-only explorer session. Split out of `world_ssh.rs`
//! to keep that module under the production line limit (#1411, #16255).

use crate::directory_view::{
    DirectoryAction, DirectoryOutcome, HELP, Line, parse_line, render_text,
};

use super::{Caller, Login, ResolveError, WorldHub, terminal_text};

const MAX_WORLD_TERMINAL_LINE: usize = 1024;
pub(super) const MAX_RECORDS_TERMINAL_BYTES: usize = 384 * 1024;
/// Total rendered bytes a single `/records` drain may produce across all its
/// pages. Each page is additionally bounded by [`MAX_RECORDS_TERMINAL_BYTES`].
pub(super) const MAX_RECORDS_TOTAL_BYTES: usize = 1024 * 1024;
pub(super) const MAX_DOCUMENT_LIST_BYTES: usize = 32 * 1024;
pub(super) const MAX_DOCUMENT_LIST_ITEMS: usize = 512;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum WorldCommand {
    Chat(String),
    Input(String),
    Records {
        prefix: Option<String>,
    },
    RecordsContinue {
        prefix: Option<String>,
        after: String,
    },
    Documents {
        prefix: Option<String>,
    },
    DocumentsContinue {
        prefix: Option<String>,
        after: String,
    },
    Document(String),
    Stats,
    Logs {
        seconds: Option<u64>,
    },
    Caps,
    Anchor {
        target: String,
        relation: Option<String>,
    },
    AnchorConfirm {
        proposal: u64,
    },
    AnchorList,
    Talk {
        target: String,
    },
    Anchors {
        target: String,
    },
    Resolve {
        thread: String,
        into: Option<String>,
    },
    Queue,
    Flush,
    Help,
    Message(String),
    Quit,
}

pub(super) fn world_command(line: &str) -> Option<WorldCommand> {
    use crate::world_line::{WorldLine, parse_line};
    Some(match parse_line(line)? {
        WorldLine::Chat(text) => WorldCommand::Chat(text),
        WorldLine::Input(hex) => WorldCommand::Input(hex),
        WorldLine::Records { prefix } => WorldCommand::Records { prefix },
        WorldLine::Documents { prefix } => WorldCommand::Documents { prefix },
        WorldLine::Document(name) => WorldCommand::Document(name),
        WorldLine::Stats => WorldCommand::Stats,
        WorldLine::Logs { seconds } => WorldCommand::Logs { seconds },
        WorldLine::Caps => WorldCommand::Caps,
        WorldLine::Anchor { target, relation } => WorldCommand::Anchor { target, relation },
        WorldLine::AnchorConfirm { proposal } => WorldCommand::AnchorConfirm { proposal },
        WorldLine::AnchorList => WorldCommand::AnchorList,
        WorldLine::Talk { target } => WorldCommand::Talk { target },
        WorldLine::Anchors { target } => WorldCommand::Anchors { target },
        WorldLine::Resolve { thread, into } => WorldCommand::Resolve { thread, into },
        WorldLine::Queue => WorldCommand::Queue,
        WorldLine::Flush => WorldCommand::Flush,
        WorldLine::Help => WorldCommand::Help,
        WorldLine::Quit => WorldCommand::Quit,
        WorldLine::Message(message) => WorldCommand::Message(message),
    })
}

#[derive(Default)]
pub(super) struct WorldLineBuffer {
    line: Vec<u8>,
    overflow: bool,
    after_cr: bool,
    escape: WorldEscape,
}

#[derive(Default)]
enum WorldEscape {
    #[default]
    None,
    Start,
    Sequence,
    Osc,
    OscEscape,
    String,
    StringEscape,
}

impl WorldLineBuffer {
    pub(super) fn push(&mut self, packet: &[u8]) -> Vec<WorldCommand> {
        let mut commands = Vec::new();
        for &byte in packet {
            match self.escape {
                WorldEscape::Start => {
                    self.escape = match byte {
                        b'[' | b'O' => WorldEscape::Sequence,
                        b']' => WorldEscape::Osc,
                        b'P' | b'_' | b'^' => WorldEscape::String,
                        _ => WorldEscape::None,
                    };
                    continue;
                }
                WorldEscape::Sequence => {
                    if (0x40..=0x7e).contains(&byte) {
                        self.escape = WorldEscape::None;
                    }
                    continue;
                }
                WorldEscape::Osc => {
                    if byte == 0x07 {
                        self.escape = WorldEscape::None;
                    } else if byte == 0x1b {
                        self.escape = WorldEscape::OscEscape;
                    }
                    continue;
                }
                WorldEscape::OscEscape => {
                    self.escape = if byte == b'\\' {
                        WorldEscape::None
                    } else {
                        WorldEscape::Osc
                    };
                    continue;
                }
                WorldEscape::String => {
                    if byte == 0x1b {
                        self.escape = WorldEscape::StringEscape;
                    }
                    continue;
                }
                WorldEscape::StringEscape => {
                    self.escape = if byte == b'\\' {
                        WorldEscape::None
                    } else {
                        WorldEscape::String
                    };
                    continue;
                }
                WorldEscape::None => {}
            }
            if byte == 0x1b {
                self.escape = WorldEscape::Start;
                continue;
            }
            let after_cr = std::mem::take(&mut self.after_cr);
            if byte == b'\n' && after_cr {
                continue;
            }
            if byte == b'\r' || byte == b'\n' {
                self.after_cr = byte == b'\r';
                if !self.overflow {
                    let bytes = std::mem::take(&mut self.line);
                    let line = String::from_utf8_lossy(&bytes);
                    if let Some(command) = world_command(&line) {
                        commands.push(command);
                    }
                } else {
                    self.line.clear();
                }
                self.overflow = false;
                continue;
            }
            if byte == 0x03 || (byte == 0x04 && self.line.is_empty()) {
                commands.push(WorldCommand::Quit);
            } else if matches!(byte, 0x7f | 0x08) {
                self.line.pop();
            } else if (0x20..=0x7e).contains(&byte) || byte >= 0x80 {
                if self.line.len() < MAX_WORLD_TERMINAL_LINE && !self.overflow {
                    self.line.push(byte);
                } else {
                    self.overflow = true;
                    self.line.clear();
                }
            }
        }
        commands
    }
}

/// Render a drained documents listing exactly as the shell-local
/// `list_public_documents` did: header, one name per line (or `(none)`),
/// the truncation marker when the drain stopped early, and the footer.
pub(super) fn render_documents_listing(names: &[String], truncated: bool) -> String {
    let mut output = String::from("public named files:\n");
    if names.is_empty() {
        output.push_str("(none)\n");
    } else {
        for name in names {
            output.push_str(name);
            output.push('\n');
        }
    }
    if truncated {
        output.push_str("[listing truncated]\n");
    }
    output.push_str("View UTF-8 .txt/.md/.markdown files with /document <name>.\n");
    output
}

pub(super) fn render_records_value(
    records: Option<&serde_json::Map<String, serde_json::Value>>,
    sequence: u64,
    next_after: Option<&str>,
) -> String {
    let mut output = format!("records at sequence {sequence}:\n");
    if let Some(records) = records {
        for (key, value) in records {
            let key = serde_json::to_string(key).unwrap_or_else(|_| "\"?\"".to_owned());
            let value = serde_json::to_string(value).unwrap_or_else(|_| "null".to_owned());
            let line = format!("{key} = {value}\n");
            if output.len() + line.len() + 32 > MAX_RECORDS_TERMINAL_BYTES {
                output.push_str("[page display truncated]\n");
                break;
            }
            output.push_str(&line);
        }
    }
    if next_after.is_some() {
        output.push_str("more records; fetching next page\n");
    }
    output
}

/// What a records page's `next_after` means for the drain, judged
/// against `produced_by` — the cursor that produced this page (the
/// empty string before the first page, whose request sends
/// `after: null`). Factored out of the `records` frame arm so the
/// contract can be unit-tested without a live session (#6615).
#[derive(Debug, PartialEq, Eq)]
pub(super) enum RecordsCursor {
    /// No cursor (absent or null): the listing is complete.
    End,
    /// A cursor that strictly advances: fetch the next page after it.
    Advance(String),
    /// A present, non-string cursor: a malformed page, never a clean
    /// end-of-pages.
    Malformed,
    /// A cursor that does not advance past `produced_by` (a repeat,
    /// a step backwards, or the empty cursor at the start): following
    /// it would re-request the page just rendered.
    Stalled,
}

pub(super) fn records_cursor(next_after: Option<&serde_json::Value>, produced_by: &str) -> RecordsCursor {
    match next_after {
        None | Some(serde_json::Value::Null) => RecordsCursor::End,
        Some(serde_json::Value::String(cursor)) if cursor.as_str() > produced_by => {
            RecordsCursor::Advance(cursor.clone())
        }
        Some(serde_json::Value::String(_)) => RecordsCursor::Stalled,
        Some(_) => RecordsCursor::Malformed,
    }
}

#[cfg(test)]
pub(super) fn render_records(
    records: &crate::world::Records,
    sequence: u64,
    next_after: Option<&str>,
) -> String {
    let value = serde_json::to_value(records).ok();
    render_records_value(
        value.as_ref().and_then(serde_json::Value::as_object),
        sequence,
        next_after,
    )
}

/// One explorer login: the world, the caller it acts as, and the secret it
/// leases the world with. Sign-in changes the caller for the rest of the login.
pub(super) struct Explorer {
    hub: WorldHub,
    world: Option<String>,
    token: Option<String>,
    caller: Caller,
}

impl Explorer {
    pub(super) fn new(hub: WorldHub, login: &Login, secret: &str) -> Self {
        let mut caller = if hub.admin_ok(secret) {
            Caller {
                admin: true,
                ..Caller::default()
            }
        } else {
            Caller::from_secret(secret)
        };
        caller.key.clone_from(&login.key);
        Self {
            hub,
            world: login.world.clone(),
            token: (!secret.is_empty()).then(|| secret.to_owned()),
            caller,
        }
    }

    pub(super) async fn act(&self, action: DirectoryAction) -> anyhow::Result<DirectoryOutcome> {
        let lease = self
            .hub
            .lease(self.world.as_deref(), self.token.as_deref())
            .await
            .map_err(|error| match error {
                ResolveError::Busy | ResolveError::TooManyWorlds => {
                    anyhow::anyhow!("the server is busy; try again")
                }
                // #1067 F12.10: surface the quota refusal verbatim so a
                // caller at their storage limit is not told "no such
                // world" for a world that exists.
                ResolveError::StorageQuota => anyhow::anyhow!(error.message()),
                _ => anyhow::anyhow!("no such world"),
            })?;
        lease.service().directory(self.caller.clone(), action).await
    }

    /// Run one typed line. Returns the text to show, and whether to close.
    pub(super) async fn execute(&mut self, line: &str) -> (String, bool) {
        let action = match parse_line(line) {
            Ok(Line::Blank) => return (String::new(), false),
            Ok(Line::Help) => return (terminal_text(&format!("{HELP}\n")), false),
            Ok(Line::Quit) => return (terminal_text("bye\n"), true),
            Ok(Line::Action(action)) => action,
            Err(error) => return (terminal_text(&format!("error: {error:#}\n")), false),
        };
        let signs_out = matches!(action, DirectoryAction::SignOut);
        let mailed_to = match &action {
            DirectoryAction::AddEmail { address } | DirectoryAction::SignInEmail { address } => {
                Some(address.clone())
            }
            _ => None,
        };
        let outcome = match self.act(action).await {
            Ok(outcome) => outcome,
            Err(error) => return (terminal_text(&format!("error: {error:#}\n")), false),
        };
        let mut text = String::new();
        if let Some(credential) = outcome.credential {
            let _ = writeln!(text, "credential (shown once): {credential}");
            self.become_caller(Caller {
                credential: Some(credential),
                ..Caller::default()
            });
        }
        if let Some(token) = outcome.session {
            let _ = writeln!(text, "session (shown once): {token}");
            self.become_caller(Caller {
                session: Some(token),
                ..Caller::default()
            });
        }
        if signs_out {
            self.become_caller(Caller::default());
        }
        if let (true, Some(address)) = (outcome.mailed, mailed_to) {
            let _ = writeln!(text, "a code was mailed to {address}");
        }
        text.push_str(&render_text(&outcome.view));
        (terminal_text(&text), false)
    }

    /// Switch to acting as another caller, keeping only this login's key. The
    /// admin token does not carry over: a new identity is not the admin.
    fn become_caller(&mut self, caller: Caller) {
        self.caller = Caller {
            key: self.caller.key.clone(),
            ..caller
        };
    }
}
