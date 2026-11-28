//! The agent kinds this fleet knows how to enrol.
//!
//! A kind says three things about an agent: which bridge groups it
//! joins (so "all Muse lanes" is one address), how it reaches the fleet
//! from where it runs, and what to tell it at enrolment. Adding a kind
//! is adding a row here; no verb changes.

use serde_json::{Value, json};

/// How an agent of this kind reaches the fleet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seat {
    /// It has a shell and runs the fleet binary itself.
    Binary,
    /// It has no shell of its own: it speaks the bridge's fleet routes
    /// over HTTPS (`scripts/fleet/bridge-mailbox.py` is the reference
    /// client), and gets its work from its mailbox.
    Http,
    /// It is a chat assistant with the Message Bridge connector
    /// attached. It has a channel address, not a mailbox.
    Connector,
}

impl Seat {
    pub fn id(self) -> &'static str {
        match self {
            Seat::Binary => "binary",
            Seat::Http => "http",
            Seat::Connector => "connector",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Kind {
    pub name: &'static str,
    pub summary: &'static str,
    /// Bridge groups every agent of this kind joins.
    pub groups: &'static [&'static str],
    pub seat: Seat,
    /// The fixed bridge address of a connector kind.
    pub address: Option<&'static str>,
    /// What is particular to enrolling this kind.
    pub note: &'static str,
}

pub const KINDS: &[Kind] = &[
    Kind {
        name: "muse",
        summary: "A Muse lane or a persistent Muse agent",
        groups: &["muse"],
        seat: Seat::Binary,
        address: None,
        note: "A fresh Muse gets a bridge-capable static build from scripts/muse/bootstrap.sh. \
               Where the installed binary predates `fleet agent`, run the same commands on the \
               laptop install: ~/tools/laptop-ssh.sh 'fleet agent next --agent <lane>'.",
    },
    Kind {
        name: "opencode",
        summary: "An OpenCode lane",
        groups: &["opencode"],
        seat: Seat::Binary,
        address: None,
        note: "Runs on the laptop seat, so every leg is local.",
    },
    Kind {
        name: "openchamber",
        summary: "An OpenChamber session or lane",
        groups: &["opencode", "openchamber"],
        seat: Seat::Binary,
        address: None,
        note: "Joins the opencode group too, so a send to group:opencode reaches both pipelines.",
    },
    Kind {
        name: "openclaw",
        summary: "An OpenClaw agent",
        groups: &["openclaw"],
        seat: Seat::Binary,
        address: None,
        note: "",
    },
    Kind {
        name: "claude-code",
        summary: "A Claude Code session with a shell, cloud or local",
        groups: &["claude-code"],
        seat: Seat::Binary,
        address: None,
        note: "A cloud session has no tailnet: messaging and the priority rung work over HTTPS; \
               builds go through `fleet shell`.",
    },
    Kind {
        name: "codex",
        summary: "A Codex CLI or cloud agent",
        groups: &["codex"],
        seat: Seat::Binary,
        address: None,
        note: "",
    },
    Kind {
        name: "gemini",
        summary: "A Gemini CLI agent",
        groups: &["gemini"],
        seat: Seat::Binary,
        address: None,
        note: "",
    },
    Kind {
        name: "antigravity",
        summary: "An Antigravity agent",
        groups: &["antigravity"],
        seat: Seat::Binary,
        address: None,
        note: "",
    },
    Kind {
        name: "grok",
        summary: "A Grok bot",
        groups: &["grok"],
        seat: Seat::Binary,
        address: None,
        note: "A bot with no shell enrols as kind `http` under the same name and groups.",
    },
    Kind {
        name: "dot",
        summary: "The Dot assistant",
        groups: &["dot"],
        seat: Seat::Http,
        address: None,
        note: "pkgs/dot-bridge carries its Slack and ChatGPT transports; the mailbox is the \
               same store.",
    },
    Kind {
        name: "duet",
        summary: "The Duet assistant",
        groups: &["duet"],
        seat: Seat::Http,
        address: None,
        note: "",
    },
    Kind {
        name: "claude",
        summary: "Claude, through the Message Bridge connector",
        groups: &[],
        seat: Seat::Connector,
        address: Some("claude"),
        note: "Set a wake hook for channel:claude on the bridge page so a send starts a session.",
    },
    Kind {
        name: "chatgpt",
        summary: "ChatGPT, through the Message Bridge connector",
        groups: &[],
        seat: Seat::Connector,
        address: Some("chatgpt"),
        note: "Woken by its MCP Events subscription.",
    },
    Kind {
        name: "laptop",
        summary: "A job or service on the fleet laptop",
        groups: &["laptop"],
        seat: Seat::Binary,
        address: None,
        note: "",
    },
    Kind {
        name: "http",
        summary: "Anything that can make an HTTPS request and nothing else",
        groups: &[],
        seat: Seat::Http,
        address: None,
        note: "",
    },
    Kind {
        name: "generic",
        summary: "Any other agent with a shell",
        groups: &[],
        seat: Seat::Binary,
        address: None,
        note: "",
    },
];

pub fn kind(name: &str) -> Option<&'static Kind> {
    KINDS.iter().find(|k| k.name == name)
}

pub fn names() -> Vec<&'static str> {
    KINDS.iter().map(|k| k.name).collect()
}

/// `fleet agent kinds`: the table as JSON.
pub fn table() -> Value {
    json!({
        "kinds": KINDS
            .iter()
            .map(|k| {
                json!({
                    "kind": k.name,
                    "summary": k.summary,
                    "groups": k.groups,
                    "seat": k.seat.id(),
                    "address": k.address,
                    "note": k.note,
                })
            })
            .collect::<Vec<_>>(),
        "seats": {
            "binary": "runs the fleet binary itself: fleet agent enroll, then fleet agent next",
            "http": "no shell: speaks the bridge's fleet routes over HTTPS; work arrives in its mailbox",
            "connector": "a chat assistant with the Message Bridge connector; addressed by channel",
        },
    })
}
