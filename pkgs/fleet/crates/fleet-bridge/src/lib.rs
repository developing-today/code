//! fleet-bridge — the bridge domain of the fleet multicall binary:
//! send and receive through the message bridge's fleet mailboxes
//! (`pkgs/message-bridge/worker/src/fleet.js`, live at
//! `https://bridge.security.cab`).
//!
//! Every fleet agent (a Muse lane, an OpenCode lane, a laptop job)
//! has a mailbox on the bridge named after it, and registers itself
//! with optional groups (`muse`, `code-lanes`, ...). Central — the
//! laptop seat, or anyone holding the bridge's owner token — sends a
//! message or a command to one agent, a group, or every agent; the
//! agent reads its mailbox, does the work, and posts its answer back,
//! which marks the command handled.
//!
//! ```text
//! central:  fleet bridge send --to group:muse --command "run the backup check"
//!           fleet bridge send --to agent:muse-3 "fyi: main is green again"
//!           fleet bridge agents
//!           fleet bridge log --direction outbound
//! agent:    fleet bridge inbox --agent muse-3 --groups muse --commands
//!           fleet bridge inbox --wait 600            (block until something arrives)
//!           fleet bridge post --reply-to 42 "backup check ok"
//!           fleet bridge ack --up-to 42
//! ```
//!
//! Auth, by name through fleet-secrets (values never printed, logged
//! or put in argv): central verbs use the owner token
//! (`message-bridge.access-token`); agent verbs use the fleet token
//! (`message-bridge.fleet-token`), which opens only the agent routes
//! and can't send commands. When the fleet token's secret is missing
//! but the owner token resolves (the laptop seat), the fleet token is
//! derived from it exactly as the bridge does (HMAC-SHA256 over a fixed
//! label). Tokens travel in the URL path, so every error string is
//! scrubbed of them before it is printed.
//!
//! A seat provisioned by bootstrap enrolment (#14370) instead holds a
//! per-agent credential under `bridge.<agent>-fleet-token` — the name
//! the agent profile records, which Settings prefers over the shared
//! default — and a class/name notes token under `bridge.<agent>-token`
//! for the `notes` family (a Bearer header, not a path token).
//! `exchange` is how those credentials get there: it trades the
//! owner's single-use bootstrap token for them and stores the values
//! through fleet-secrets' write path, printing only their names.
//!
//! Exit codes, the workspace convention: 0 ok · 2 usage · 3 transport
//! down / credentials unresolved · 4 the bridge answered with an HTTP
//! error (its JSON error goes to stderr).

use std::io::Read;
use std::time::{Duration, Instant};

use fleet_config::{
    Config, DEFAULT_BRIDGE_FLEET_TOKEN_NAME, DEFAULT_BRIDGE_OWNER_TOKEN_NAME, DEFAULT_BRIDGE_URL,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub const EXIT_OK: i32 = 0;
pub const EXIT_USAGE: i32 = 2;
pub const EXIT_TRANSPORT: i32 = 3;
pub const EXIT_HTTP: i32 = 4;

const FLEET_TOKEN_LABEL: &str = "message-bridge/fleet-token/v1";
const DEFAULT_INTERVAL_SECS: u64 = 15;
const MAX_WAIT_SECS: u64 = 24 * 3600;

/// The task-it-or-queue-it rule for instructions (design §4.6,
/// #14372). This exact text is carried in three places that must stay
/// verbatim-consistent: here (the work verb's help), the fleet book
/// (pkgs/fleet/book/reference/bridge.md), and the woken-agent text the
/// intake ladder hands out with a pushed command
/// (fleet-agent/src/next.rs). Edit all three together.
pub const WORK_RULE: &str = "\
Task it or queue it. An instruction is a work item: do the work now, in \
this session, and reply — or queue it as a repo-task/v1 packet whose \
context_refs carry the bridge surface, the message id and the batch, and \
reply with the task id. Record the outcome with fleet bridge work: \
--state acted for work done in-session, --state queued --task <id> for \
queued work, --state done when the work is finished. An instruction \
that is neither acted on nor queued is a defect: it stays at received, \
where the owner can see it stuck in the pipeline.";

pub const USAGE: &str = "\
usage: fleet bridge <verb> [options]

Send and receive through the message bridge's fleet mailboxes.

central (owner token):
  send --to <targets> [--command] [--sender NAME] [--limit N] <text | ->
        targets: agent:<name> (or a bare name), group:<name>, all;
        repeat --to or separate with commas. A name may be a glob
        (agent:f*); --limit caps how many of the matches receive it.
        The answer is the match report: matched, delivered, the
        recipients, and how many matched agents remain.
  send --broaden <batch> [--limit N]
        deliver a pattern send's original text to the matched agents
        its limit left over — the same batch, never a duplicate.
  offer --to <targets> --slots N [--command] [--sender NAME]
        [--ttl SECS] <text | ->
        post the text as a claim offer instead of delivering it: the
        next N eligible agents to claim it are delivered a row on the
        offer's batch (#14371).
  offers                       every offer with its claims
  agents                       registered agents, groups, unread counts
  log [--agent A] [--direction inbound|outbound] [--kind message|command]
      [--since-id N] [--before-id N] [--limit N] [--order asc|desc]
  pipeline [--kind K] [--sender S] [--receiver R] [--surface S]
      [--batch B] [--unread] [--work-state W] [--since-id N]
      [--before-id N] [--limit N] [--order asc|desc]
        the unified owner feed over connector messages, fleet
        mailboxes and notes, real senders shown
  directory                    the unified agents list: fleet registry
        and note identities, each row marked by surface
  credentials                  per-agent credential state (active or
        revoked, live scopes; never a hash)
  revoke (--agent A | --identity I)
        revoke an agent's fleet credentials, or a notes identity's
        tokens (the owner's kill switch, #14368)
  mint-enroll --agent A [--kind K] [--groups g1,g2]
      [--notes-identity I] [--ttl SECS]
        mint a bootstrap enroll token bound to the agent (and kind,
        groups, notes identity); the value is printed once, in this
        answer only — hand it to the new seat's enroll --bootstrap
  enroll-tokens                every minted bootstrap token's state
        (pending, used, expired, revoked, with its closing times;
        never a hash or a value)
  revoke-enroll (--agent A | --token-hash H)
        revoke unused bootstrap tokens: an agent's pending ones, or
        one token by its stored hash (a token value is never needed,
        and never travels)
  wake-hooks                   every wake hook (address, kind, host;
        never the token or the path)
  wake-hooks --set -           set one hook from a JSON object on
        stdin: {address, url, token?} — the token, when there is
        one, travels on stdin only, never as an argument
  wake-hooks --remove --address A
        remove the hook for one address
  mint-credential --agent A [--scopes s1,s2] [--addr service:<name>]
        mint a per-agent credential directly; the value is printed
        once, in this answer only

agent (fleet token):
  hello [--agent A] [--groups g1,g2]          register / refresh groups
  inbox [--agent A] [--groups g1,g2] [--commands | --messages] [--all]
        [--since-id N] [--limit N] [--take] [--wait SECS] [--interval SECS]
        unread by default; --all includes read ones; --take marks what it
        returns as read; --wait polls until something arrives or SECS pass.
  post [--agent A] [--reply-to ID] [--to ADDR] [--instruction | --kind K] <text | ->
        message to central by default; with --reply-to, answers that row
        (and marks it read). --to sends to any address instead: drew,
        claude, chatgpt, connectors, agent:<name> (or a bare name),
        group:<name>, agents, all; repeat or separate with commas.
        --instruction (or --kind instruction) makes it an instruction,
        which needs --to.
  ack [--agent A] (--up-to ID | ID...)        mark messages read
  work [--agent A] --state acted|queued|done [--task ID] ID...
        record the work-item lifecycle of your own instructions:
        acted (handled in-session), queued (became queue task --task),
        done. States move only forward, from received.

provisioning (single-use bootstrap token, minted by the owner):
  exchange --agent A --bootstrap TOKEN
        trade the bootstrap token for this agent's own credentials
        and store them in the vault by name (bridge.<agent>-fleet-token
        and, when the mint carried a notes identity,
        bridge.<agent>-token). The values are never printed.

notes (the calling identity's notes token, as a Bearer header):
  notes send --to CLASS/NAME [--kind note|instruction] [--reply-to ID] <text | ->
        a note to a class/name identity (a * segment wildcards);
        --kind instruction makes it a work item, which lands in the
        recipient's inbox without a sender. --reply-to ID answers a
        note in your inbox — routed to its real sender, whom an
        instruction never shows you — and may replace --to.
  notes inbox                  this identity's pending notes
  notes ack (--up-to ID | ID...)   acknowledge notes

  health                       the bridge's /healthz (no token)

The agent defaults to $FLEET_AGENT, then [bridge] agent; groups to
$FLEET_AGENT_GROUPS, then [bridge] groups. The bridge URL is
$FLEET_BRIDGE_URL, then [bridge] url, then https://bridge.security.cab.
Output is the bridge's JSON. Exit: 0 ok, 2 usage, 3 transport or
credentials, 4 the bridge refused the request.";

/// The trailing block of USAGE, shared by every per-verb help text.
/// A test pins USAGE to end with this, so the two cannot drift.
const USAGE_FOOTER: &str = "\
The agent defaults to $FLEET_AGENT, then [bridge] agent; groups to
$FLEET_AGENT_GROUPS, then [bridge] groups. The bridge URL is
$FLEET_BRIDGE_URL, then [bridge] url, then https://bridge.security.cab.
Output is the bridge's JSON. Exit: 0 ok, 2 usage, 3 transport or
credentials, 4 the bridge refused the request.";

/// Per-verb usage for `fleet bridge <verb> --help` (#13197). The
/// domain's status line advertises per-verb help, so each verb answers
/// with its own section of USAGE plus the shared footer. Unknown verbs
/// return None and the caller falls back to USAGE, so a help request
/// never prints nothing.
pub fn verb_usage(verb: &str) -> Option<String> {
    let section = match verb {
        "send" => {
            "\
usage: fleet bridge send --to <targets> [--command] [--sender NAME] [--limit N] <text | ->
       fleet bridge send --broaden <batch> [--limit N]

central (owner token). targets: agent:<name> (or a bare name),
group:<name>, all; repeat --to or separate with commas. A name may be
a glob (agent:f*); --limit caps how many of the matches receive it,
and the answer is the match report (matched, delivered, recipients,
remaining). --broaden delivers a pattern send's original text to the
matched agents its limit left over, on the same batch. --command
marks it a command (default: a plain message). '-' reads the text
from stdin."
        }
        "offer" => {
            "\
usage: fleet bridge offer --to <targets> --slots N [--command] [--sender NAME] [--ttl SECS] <text | ->

central (owner token). Post the text as a claim offer (#14371)
instead of delivering it: the next N eligible agents to claim it are
delivered a row on the offer's batch, and their replies thread onto
it. targets are the send grammar (names, globs, groups, all);
--slots is how many claims land. --ttl bounds how long the offer
stands open, in seconds."
        }
        "offers" => {
            "\
usage: fleet bridge offers

central (owner token). Every offer with its claims: who claimed,
when, and the row each claim delivered."
        }
        "agents" => {
            "\
usage: fleet bridge agents

central (owner token). Registered agents, groups, unread counts."
        }
        "log" => {
            "\
usage: fleet bridge log [--agent A] [--direction inbound|outbound] [--kind message|command]
      [--since-id N] [--before-id N] [--limit N] [--order asc|desc]

central (owner token). The central message log, filtered."
        }
        "pipeline" => {
            "\
usage: fleet bridge pipeline [--kind K] [--sender S] [--receiver R] [--surface S]
      [--batch B] [--unread] [--work-state W] [--since-id N]
      [--before-id N] [--limit N] [--order asc|desc]

central (owner token). The unified owner feed over connector
messages, fleet mailboxes and notes (#14369), real senders shown:
kind is message, instruction, command or note; sender and receiver
name an address or its bare name; surface is connector, fleet or
notes. --work-state is passed through for the work-item lifecycle
(#14372)."
        }
        "directory" => {
            "\
usage: fleet bridge directory

central (owner token). The unified agents list (#14369): fleet
registry rows (groups, first/last seen, unread counts) and note
identities (scopes, signal counter), each row marked by surface.
Notes identities have no presence; the row says so."
        }
        "credentials" => {
            "\
usage: fleet bridge credentials

central (owner token). Per-agent credential state (#14368): one row
per agent that has held a credential — active or revoked, the scopes
its live credentials carry, mint and revocation times. State only:
no hash or value is ever shown."
        }
        "revoke" => {
            "\
usage: fleet bridge revoke (--agent A | --identity I)

central (owner token). Revoke every live fleet credential of an
agent (--agent), or every notes token of an identity (--identity):
the per-agent kill switch (#14368). Prints how many were revoked."
        }
        "mint-enroll" => {
            "\
usage: fleet bridge mint-enroll --agent A [--kind K] [--groups g1,g2]
      [--notes-identity I] [--ttl SECS]

central (owner token). Mint a bootstrap enroll token (#14368) bound
at mint to the agent's name, kind (default: agent), groups and class
notes identity, expiring after --ttl seconds (60 to 604800, default
86400). The answer carries the token value once: it is stored
hash-only and can never be shown again. Hand the value to the new
seat: fleet agent enroll --name A --kind K --bootstrap <value>."
        }
        "enroll-tokens" => {
            "\
usage: fleet bridge enroll-tokens

central (owner token). Every minted bootstrap token's state
(#14376): pending, used by the exchange, expired, or revoked, each
with the times that closed it. State only: no hash or value is ever
shown — the value existed once, in the mint answer."
        }
        "revoke-enroll" => {
            "\
usage: fleet bridge revoke-enroll (--agent A | --token-hash H)

central (owner token). Revoke unused bootstrap tokens (#14376):
--agent takes back every still-pending token minted for that agent
(the console's form — its list shows no hash); --token-hash revokes
the one token stored under that SHA-256 hash. The hash is not a
credential — it cannot enroll anything — so it may travel where the
token value never may. By hash, a used or already-revoked token
answers {revoked: false}; by agent, the answer counts what was
taken back."
        }
        "wake-hooks" => {
            "\
usage: fleet bridge wake-hooks [--set - | --remove --address A]

central (owner token). The bridge's wake hooks (#14544): with no
flag, every hook — address, kind, host, and whether it holds a
token; never the token or the URL's path. --set reads one hook as a
JSON object {address, url, token?} from stdin (so a hook token
never travels as an argument) and sets it; --remove takes the hook
for one address back. Both post to the wake-hooks API the retired
bridge page used; the PM console's Agents screen drives this verb."
        }
        "mint-credential" => {
            "\
usage: fleet bridge mint-credential --agent A [--scopes s1,s2] [--addr service:<name>]

central (owner token). Mint a per-agent credential directly
(#14368): scopes default to inbox + send; with --addr the credential
is a service credential bound to that address (its own name only),
and service mints default to send + read. The answer carries the
credential value once: it is stored hash-only and can never be shown
again."
        }
        "hello" => {
            "\
usage: fleet bridge hello [--agent A] [--groups g1,g2]

agent (fleet token). Register / refresh groups."
        }
        "inbox" => {
            "\
usage: fleet bridge inbox [--agent A] [--groups g1,g2] [--commands | --messages] [--all]
        [--since-id N] [--limit N] [--take] [--wait SECS] [--interval SECS]

agent (fleet token). Unread by default; --all includes read ones;
--take marks what it returns as read; --wait polls until something
arrives or SECS pass."
        }
        "post" => {
            "\
usage: fleet bridge post [--agent A] [--reply-to ID] [--to ADDR] [--instruction | --kind K] <text | ->

agent (fleet token). Message to central by default; with --reply-to,
answers that row (and marks it read). --to sends to any address
instead: drew, claude, chatgpt, connectors, agent:<name> (or a bare
name), group:<name>, agents, all; repeat or separate with commas.
--instruction (or --kind instruction) makes it an instruction, which
needs --to."
        }
        "ack" => {
            "\
usage: fleet bridge ack [--agent A] (--up-to ID | ID...)

agent (fleet token). Mark messages read."
        }
        "work" => {
            return Some(format!(
                "\
usage: fleet bridge work [--agent A] --state acted|queued|done [--task ID] ID...

agent (fleet token). Record the work-item lifecycle of instructions in
your own mailbox: acted for work handled in-session, queued for work
that became a fleet queue task (give its id with --task), done when the
work is finished. States move only forward, from received; a row can
only be marked by the agent it was sent to.

{WORK_RULE}

{USAGE_FOOTER}"
            ));
        }
        "exchange" => {
            "\
usage: fleet bridge exchange --agent A --bootstrap TOKEN

provisioning (single-use bootstrap token, minted by the owner and
bound to this agent's name, kind and groups). Trades the token for
the agent's own fleet credential (and notes token, when the mint
carried a notes identity) and stores them in the vault by name:
bridge.<agent>-fleet-token, bridge.<agent>-token. The credential
values are never printed. A refused exchange stores nothing."
        }
        "notes" => {
            "\
usage: fleet bridge notes <send | inbox | ack | get | watch> [options]

The class/name notes surface, authenticated by this identity's own
notes token (Bearer header, resolved by vault name). `get` alone needs
no token.

  notes send --to CLASS/NAME [--kind note|instruction] [--reply-to ID] <text | ->
        a * segment in --to wildcards. --kind instruction makes the
        note a work item; it lands in the recipient's inbox without
        a sender. --reply-to ID answers a note in your inbox and is
        routed to its real sender; it may replace --to.
  notes inbox                  this identity's pending notes
  notes ack (--up-to ID | ID...)   acknowledge notes
  notes get CLASS/NAME         the path's public wake signal: a bare
        UUID that changes when a note is delivered, and also rotates
        on its own every 10-40 minutes, so a change is a reason to
        look, never proof of a note
  notes watch [--wait SECS] [--interval SECS]
        block until this identity has pending notes, then print them
        (the inbox answer, plus how it woke). Polls the public signal
        every --interval (default 15) and reads the inbox only when
        the signal changes; gives up after --wait (default and most
        86400) with an empty answer"
        }
        "health" => {
            "\
usage: fleet bridge health

The bridge's /healthz (no token)."
        }
        _ => return None,
    };
    Some(format!("{section}\n\n{USAGE_FOOTER}"))
}

#[derive(Debug, PartialEq)]
pub enum BridgeError {
    Usage(String),
    Transport(String),
    Http { status: u16, message: String },
}

impl BridgeError {
    pub fn exit_code(&self) -> i32 {
        match self {
            BridgeError::Usage(_) => EXIT_USAGE,
            BridgeError::Transport(_) => EXIT_TRANSPORT,
            BridgeError::Http { .. } => EXIT_HTTP,
        }
    }
}

impl std::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BridgeError::Usage(m) => write!(f, "fleet bridge: {m}"),
            BridgeError::Transport(m) => write!(f, "fleet bridge: {m}"),
            BridgeError::Http { status, message } => {
                write!(f, "fleet bridge: the bridge answered {status}: {message}")
            }
        }
    }
}

type R<T> = Result<T, BridgeError>;

fn usage(msg: impl Into<String>) -> BridgeError {
    BridgeError::Usage(msg.into())
}

// ---- tokens ----

/// Resolves a secret by name; the real one is the fleet-secrets chain.
pub trait Secrets {
    fn resolve(&self, name: &str) -> Result<String, String>;
    /// Stores a value under a name (the write side `exchange` needs
    /// for a freshly minted credential). The default refuses: only
    /// the real chain can write, through fleet-secrets' bws store.
    fn store(&self, _name: &str, _value: &str) -> Result<(), String> {
        Err("storing secrets is not supported by this source".to_string())
    }
}

/// A [`Secrets`] seen through a name scope (#10358 D11): reads and
/// writes both map the caller's name to its full stored name first,
/// so a scoped bridge reads what it wrote. [`Scope::none`] passes
/// every name through unchanged.
///
/// [`Scope::none`]: fleet_secrets::Scope::none
pub struct ScopedSecrets<S> {
    scope: fleet_secrets::Scope,
    inner: S,
}

impl<S: Secrets> ScopedSecrets<S> {
    pub fn new(scope: fleet_secrets::Scope, inner: S) -> ScopedSecrets<S> {
        ScopedSecrets { scope, inner }
    }

    fn full(&self, name: &str) -> Result<String, String> {
        self.scope.to_full(name).map_err(|e| e.to_string())
    }
}

impl<S: Secrets> Secrets for ScopedSecrets<S> {
    fn resolve(&self, name: &str) -> Result<String, String> {
        self.inner.resolve(&self.full(name)?)
    }

    fn store(&self, name: &str, value: &str) -> Result<(), String> {
        self.inner.store(&self.full(name)?, value)
    }
}

/// The real chain, taking full stored names; [`run`] wraps it in the
/// instance's scope.
struct ChainSecrets<'a>(&'a Config);

impl Secrets for ChainSecrets<'_> {
    fn resolve(&self, name: &str) -> Result<String, String> {
        fleet_secrets::Resolver::default_chain(self.0)
            .resolve(name)
            .map(|v| v.expose().to_string())
            .map_err(|e| e.to_string())
    }

    fn store(&self, name: &str, value: &str) -> Result<(), String> {
        let store = fleet_secrets::BwsStore::from_config(self.0).ok_or_else(|| {
            format!(
                "no bws project is configured for secret writes: set ${} or [secrets] bws_project_id in fleet.toml",
                fleet_secrets::BwsStore::PROJECT_ID_ENV
            )
        })?;
        store.store(name, value).map_err(|e| e.to_string())
    }
}

/// HMAC-SHA256 (RFC 2104) over SHA-256.
pub fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut k = [0u8; BLOCK];
    if key.len() > BLOCK {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(k.map(|b| b ^ 0x36));
    inner.update(msg);
    let inner = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(k.map(|b| b ^ 0x5c));
    outer.update(inner);
    let mut out = [0u8; 32];
    out.copy_from_slice(&outer.finalize());
    out
}

/// Base64url without padding.
pub fn b64url(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut s = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        s.push(A[(n >> 18) as usize & 63] as char);
        s.push(A[(n >> 12) as usize & 63] as char);
        if c.len() > 1 {
            s.push(A[(n >> 6) as usize & 63] as char);
        }
        if c.len() > 2 {
            s.push(A[n as usize & 63] as char);
        }
    }
    s
}

/// The fleet token the bridge derives from its access token.
pub fn derive_fleet_token(access_token: &str) -> String {
    b64url(&hmac_sha256(
        access_token.as_bytes(),
        FLEET_TOKEN_LABEL.as_bytes(),
    ))
}

// ---- settings ----

/// Where and as whom: resolved from flags, then env, then fleet.toml.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub url: String,
    pub agent: Option<String>,
    pub groups: Option<Vec<String>>,
    pub owner_token_name: String,
    pub fleet_token_name: String,
    /// The class/name notes token's vault name, when one can be
    /// named: [bridge] config, the agent profile, or the
    /// `bridge.<agent>-token` convention. None when no agent is
    /// known and nothing names one — the notes verbs say so.
    pub notes_token_name: Option<String>,
}

fn non_blank(v: Option<String>) -> Option<String> {
    v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn split_list(s: &str) -> Vec<String> {
    s.split(',')
        .map(|g| g.trim().to_string())
        .filter(|g| !g.is_empty())
        .collect()
}

/// What the agent profile contributes to settings: the agent's name
/// and the credential vault names `fleet agent enroll --bootstrap`
/// recorded there (#14370; enroll.rs owns the format). Read
/// leniently — a missing or malformed profile contributes nothing,
/// exactly like a seat that never enrolled.
#[derive(Debug, Clone, Default, PartialEq)]
struct ProfileNames {
    agent: Option<String>,
    fleet_token_name: Option<String>,
    notes_token_name: Option<String>,
}

fn profile_path(env: &dyn Fn(&str) -> Option<String>) -> Option<std::path::PathBuf> {
    if let Some(p) = non_blank(env("FLEET_AGENT_PROFILE")) {
        return Some(std::path::PathBuf::from(p));
    }
    if let Some(config) = non_blank(env("XDG_CONFIG_HOME")) {
        return Some(
            std::path::PathBuf::from(config)
                .join("fleet")
                .join("agent.json"),
        );
    }
    non_blank(env("HOME")).map(|h| std::path::PathBuf::from(h).join(".config/fleet/agent.json"))
}

fn read_profile_names(env: &dyn Fn(&str) -> Option<String>) -> ProfileNames {
    let Some(path) = profile_path(env) else {
        return ProfileNames::default();
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return ProfileNames::default();
    };
    let Ok(v) = serde_json::from_str::<Value>(&text) else {
        return ProfileNames::default();
    };
    let get = |k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    ProfileNames {
        agent: get("name"),
        fleet_token_name: get("fleet_token_name"),
        notes_token_name: get("notes_token_name"),
    }
}

impl Settings {
    pub fn resolve(config: &Config, env: &dyn Fn(&str) -> Option<String>) -> R<Settings> {
        let b = &config.bridge;
        let url = non_blank(env("FLEET_BRIDGE_URL"))
            .or_else(|| b.url.clone())
            .unwrap_or_else(|| DEFAULT_BRIDGE_URL.to_string());
        let url = check_url(&url)?;
        let profile = read_profile_names(env);
        let agent = non_blank(env("FLEET_AGENT"))
            .or_else(|| b.agent.clone())
            .or_else(|| profile.agent.clone());
        Ok(Settings {
            url,
            groups: non_blank(env("FLEET_AGENT_GROUPS"))
                .map(|s| split_list(&s))
                .or_else(|| b.groups.clone()),
            owner_token_name: b
                .owner_token_name
                .clone()
                .unwrap_or_else(|| DEFAULT_BRIDGE_OWNER_TOKEN_NAME.to_string()),
            fleet_token_name: b
                .fleet_token_name
                .clone()
                .or_else(|| profile.fleet_token_name.clone())
                .unwrap_or_else(|| DEFAULT_BRIDGE_FLEET_TOKEN_NAME.to_string()),
            notes_token_name: b
                .notes_token_name
                .clone()
                .or_else(|| profile.notes_token_name.clone())
                .or_else(|| agent.as_ref().map(|a| format!("bridge.{a}-token"))),
            agent,
        })
    }
}

/// https anywhere; plain http only to this machine (tokens ride the path).
pub fn check_url(raw: &str) -> R<String> {
    let url = raw.trim().trim_end_matches('/').to_string();
    let rest = if let Some(r) = url.strip_prefix("https://") {
        r
    } else if let Some(r) = url.strip_prefix("http://") {
        let host = r.split(['/', ':']).next().unwrap_or("");
        let host = if r.starts_with('[') {
            r.split(']').next().map(|h| &h[1..]).unwrap_or("")
        } else {
            host
        };
        if !matches!(host, "127.0.0.1" | "localhost" | "::1") {
            return Err(usage(
                "the bridge URL must be https (plain http only to localhost)",
            ));
        }
        r
    } else {
        return Err(usage("the bridge URL must start with https://"));
    };
    if rest.is_empty() || rest.contains(['?', '#', ' ']) || rest.contains('/') {
        return Err(usage(
            "the bridge URL must be a bare origin, like https://bridge.example",
        ));
    }
    Ok(url)
}

// ---- HTTP ----

/// Percent-encodes a query value (RFC 3986 unreserved kept).
pub fn qenc(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            o.push(b as char);
        } else {
            o.push_str(&format!("%{b:02X}"));
        }
    }
    o
}

fn query(pairs: &[(&str, Option<String>)]) -> String {
    let parts: Vec<String> = pairs
        .iter()
        .filter_map(|(k, v)| v.as_ref().map(|v| format!("{k}={}", qenc(v))))
        .collect();
    if parts.is_empty() {
        String::new()
    } else {
        format!("?{}", parts.join("&"))
    }
}

/// The headers every bridge request carries, plus the Bearer token
/// when a route authenticates by header (the notes routes). Generic
/// over the builder's typestate: `get` starts WithoutBody, `post`
/// starts WithBody, and headers are settable in either.
fn with_auth<State>(
    req: ureq::RequestBuilder<State>,
    bearer: Option<&str>,
) -> ureq::RequestBuilder<State> {
    let req = req
        .header("Accept", "application/json")
        .header("User-Agent", "fleet-bridge");
    match bearer {
        Some(t) => req.header("Authorization", format!("Bearer {t}")),
        None => req,
    }
}

pub struct Client {
    agent: ureq::Agent,
    base: String,
    secrets_to_scrub: Vec<String>,
}

impl Client {
    pub fn new(base: &str) -> Client {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(60)))
            .timeout_connect(Some(Duration::from_secs(15)))
            .http_status_as_error(false)
            .tls_config(
                ureq::tls::TlsConfig::builder()
                    .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                    .build(),
            )
            .build()
            .into();
        Client {
            agent,
            base: base.to_string(),
            secrets_to_scrub: Vec::new(),
        }
    }

    fn scrub(&self, s: &str) -> String {
        let mut out = s.to_string();
        for t in &self.secrets_to_scrub {
            if !t.is_empty() {
                out = out.replace(t.as_str(), "<token>");
            }
        }
        out
    }

    /// One request; `path` already carries the token. Returns the JSON body.
    fn call(&self, method: &str, path: &str, body: Option<&Value>) -> R<Value> {
        self.call_req(method, path, body, None)
    }

    /// One request authenticated by an `Authorization: Bearer` header
    /// instead of a path token (the notes routes work that way). The
    /// token joins the scrub list, like a path token.
    fn call_bearer(
        &mut self,
        method: &str,
        path: &str,
        body: Option<&Value>,
        token: &str,
    ) -> R<Value> {
        if !self.secrets_to_scrub.iter().any(|t| t == token) {
            self.secrets_to_scrub.push(token.to_string());
        }
        self.call_req(method, path, body, Some(token))
    }

    /// One unauthenticated GET whose answer is plain text (the notes
    /// public signal), trimmed.
    fn get_text(&self, path: &str) -> R<String> {
        let url = format!("{}{}", self.base, path);
        let mut resp = self
            .agent
            .get(&url)
            .header("Accept", "text/plain")
            .header("User-Agent", "fleet-bridge")
            .call()
            .map_err(|e| {
                BridgeError::Transport(format!(
                    "GET to the bridge failed: {}",
                    self.scrub(&e.to_string())
                ))
            })?;
        let status = resp.status().as_u16();
        let text = resp
            .body_mut()
            .read_to_string()
            .map_err(|e| BridgeError::Transport(format!("reading the bridge's answer: {e}")))?;
        if !(200..300).contains(&status) {
            return Err(BridgeError::Http {
                status,
                message: self.scrub(text.trim()).chars().take(300).collect(),
            });
        }
        Ok(text.trim().to_string())
    }

    fn call_req(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
        bearer: Option<&str>,
    ) -> R<Value> {
        let url = format!("{}{}", self.base, path);
        let result = match body {
            None => with_auth(self.agent.get(&url), bearer).call(),
            Some(b) => with_auth(self.agent.post(&url), bearer)
                .header("Content-Type", "application/json")
                .send(b.to_string().as_bytes()),
        };
        let mut resp = result.map_err(|e| {
            BridgeError::Transport(format!(
                "{method} to the bridge failed: {}",
                self.scrub(&e.to_string())
            ))
        })?;
        let status = resp.status().as_u16();
        let text = resp
            .body_mut()
            .read_to_string()
            .map_err(|e| BridgeError::Transport(format!("reading the bridge's answer: {e}")))?;
        let parsed: Option<Value> = serde_json::from_str(&text).ok();
        if !(200..300).contains(&status) {
            let message = match parsed.as_ref().and_then(|v| v.get("error")) {
                Some(Value::String(e)) => e.clone(),
                _ if status == 404 => {
                    "not found (a wrong token, an old bridge without fleet mailboxes, or a wrong URL)"
                        .to_string()
                }
                _ => self.scrub(text.trim()).chars().take(300).collect(),
            };
            return Err(BridgeError::Http { status, message });
        }
        parsed.ok_or_else(|| BridgeError::Transport("the bridge's answer was not JSON".to_string()))
    }
}

// ---- argument parsing ----

#[derive(Debug, Default)]
struct Args {
    flags: Vec<(String, Option<String>)>,
    positional: Vec<String>,
}

/// Flags that take a value; everything else starting with -- is a switch.
const VALUE_FLAGS: &[&str] = &[
    "--to",
    "--sender",
    "--receiver",
    "--surface",
    "--batch",
    "--work-state",
    "--agent",
    "--identity",
    "--groups",
    "--direction",
    "--kind",
    "--since-id",
    "--before-id",
    "--limit",
    "--order",
    "--wait",
    "--interval",
    "--reply-to",
    "--up-to",
    "--broaden",
    "--slots",
    "--state",
    "--task",
    "--bootstrap",
    "--ttl",
    "--scopes",
    "--notes-identity",
    "--token-hash",
    "--addr",
    "--address",
];

fn parse(args: &[String], switches: &[&str], values: &[&str]) -> R<Args> {
    let mut out = Args::default();
    let mut it = args.iter();
    let mut only_positional = false;
    while let Some(a) = it.next() {
        if only_positional || !a.starts_with("--") || a == "-" {
            out.positional.push(a.clone());
            continue;
        }
        if a == "--" {
            only_positional = true;
            continue;
        }
        let (name, inline) = match a.split_once('=') {
            Some((n, v)) => (n.to_string(), Some(v.to_string())),
            None => (a.clone(), None),
        };
        if values.contains(&name.as_str()) {
            debug_assert!(VALUE_FLAGS.contains(&name.as_str()));
            let v = match inline {
                Some(v) => v,
                None => it
                    .next()
                    .cloned()
                    .ok_or_else(|| usage(format!("{name} needs a value")))?,
            };
            out.flags.push((name, Some(v)));
        } else if switches.contains(&name.as_str()) {
            if inline.is_some() {
                return Err(usage(format!("{name} takes no value")));
            }
            out.flags.push((name, None));
        } else {
            return Err(usage(format!("unknown option {name}")));
        }
    }
    Ok(out)
}

impl Args {
    fn has(&self, name: &str) -> bool {
        self.flags.iter().any(|(n, _)| n == name)
    }
    fn last(&self, name: &str) -> Option<String> {
        self.flags
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .and_then(|(_, v)| v.clone())
    }
    fn all(&self, name: &str) -> Vec<String> {
        self.flags
            .iter()
            .filter(|(n, _)| n == name)
            .filter_map(|(_, v)| v.clone())
            .collect()
    }
    fn uint(&self, name: &str) -> R<Option<u64>> {
        match self.last(name) {
            None => Ok(None),
            Some(v) => v
                .trim()
                .parse::<u64>()
                .map(Some)
                .map_err(|_| usage(format!("{name} must be a whole number"))),
        }
    }
    fn no_positional(&self, verb: &str) -> R<()> {
        if let Some(p) = self.positional.first() {
            return Err(usage(format!("{verb} takes no argument '{p}'")));
        }
        Ok(())
    }
}

/// The message text: the positional words, or stdin for a lone `-`.
fn text_of(a: &Args, stdin: &mut dyn Read) -> R<String> {
    let text = if a.positional.len() == 1 && a.positional[0] == "-" {
        let mut s = String::new();
        stdin
            .read_to_string(&mut s)
            .map_err(|e| usage(format!("reading stdin: {e}")))?;
        s
    } else {
        a.positional.join(" ")
    };
    if text.trim().is_empty() {
        return Err(usage("give the message text (or - to read it from stdin)"));
    }
    Ok(text)
}

fn agent_of(a: &Args, s: &Settings) -> R<String> {
    a.last("--agent")
        .or_else(|| s.agent.clone())
        .map(|n| n.trim().to_lowercase())
        .filter(|n| !n.is_empty())
        .ok_or_else(|| usage("name the agent: --agent NAME, $FLEET_AGENT or [bridge] agent"))
}

fn groups_of(a: &Args, s: &Settings) -> Option<Vec<String>> {
    match a.last("--groups") {
        Some(g) => Some(split_list(&g)),
        None => s.groups.clone(),
    }
}

// ---- verbs ----

/// What a run needs from the outside world, so tests can supply it.
pub struct Ctx<'a> {
    pub settings: Settings,
    pub secrets: &'a dyn Secrets,
    pub stdin: &'a mut dyn Read,
    pub sleep: &'a dyn Fn(Duration),
}

fn owner_token(ctx: &Ctx) -> R<String> {
    ctx.secrets
        .resolve(&ctx.settings.owner_token_name)
        .map_err(|e| {
            BridgeError::Transport(format!(
                "owner token '{}' unresolved: {e} (central verbs need it)",
                ctx.settings.owner_token_name
            ))
        })
}

fn fleet_token(ctx: &Ctx) -> R<String> {
    match ctx.secrets.resolve(&ctx.settings.fleet_token_name) {
        Ok(t) if !t.trim().is_empty() => Ok(t.trim().to_string()),
        fleet_err => match ctx.secrets.resolve(&ctx.settings.owner_token_name) {
            Ok(owner) if !owner.trim().is_empty() => Ok(derive_fleet_token(owner.trim())),
            _ => Err(BridgeError::Transport(format!(
                "fleet token '{}' unresolved{} and no owner token '{}' to derive it from",
                ctx.settings.fleet_token_name,
                match fleet_err {
                    Err(e) => format!(" ({e})"),
                    Ok(_) => " (empty)".to_string(),
                },
                ctx.settings.owner_token_name
            ))),
        },
    }
}

fn client_with(ctx: &Ctx, token: &str) -> Client {
    let mut c = Client::new(&ctx.settings.url);
    c.secrets_to_scrub.push(token.to_string());
    c
}

/// The calling identity's notes token, by vault name (the notes
/// verbs authenticate with it as a Bearer header).
fn notes_token(ctx: &Ctx) -> R<String> {
    let name = ctx.settings.notes_token_name.as_deref().ok_or_else(|| {
        usage(
            "no notes token can be named: set [bridge] notes_token_name or agent in fleet.toml, or enrol this seat (fleet agent enroll --bootstrap)",
        )
    })?;
    ctx.secrets.resolve(name).map_err(|e| {
        BridgeError::Transport(format!(
            "notes token '{name}' unresolved: {e} (the notes verbs need it)"
        ))
    })
}

/// `fleet bridge exchange` (#14370, design §4.5/§4.7): trade a
/// single-use bootstrap token — minted by the owner, bound to this
/// agent's name, kind and groups — for the agent's own credentials.
/// The exchange answer carries the credential values exactly once;
/// they go straight into the vault under their names
/// (`bridge.<agent>-fleet-token`, `bridge.<agent>-token`) and are
/// never printed, so this verb's output is names and metadata only.
/// A refused exchange (wrong, used, expired or revoked token, or a
/// name that is not the minted name) stores nothing.
fn verb_exchange(ctx: &mut Ctx, rest: &[String]) -> R<Value> {
    let a = parse(rest, &[], &["--agent", "--bootstrap"])?;
    a.no_positional("exchange")?;
    let agent = agent_of(&a, &ctx.settings)?;
    let bootstrap = a.last("--bootstrap").ok_or_else(|| {
        usage("exchange needs --bootstrap <the single-use token the console minted for this agent>")
    })?;
    let mut client = Client::new(&ctx.settings.url);
    client.secrets_to_scrub.push(bootstrap.clone());
    let out = client
        .call(
            "POST",
            "/enroll/exchange",
            Some(&json!({ "bootstrap_token": bootstrap, "agent": agent })),
        )
        .map_err(|e| match e {
            BridgeError::Http { status: 404, .. } => BridgeError::Http {
                status: 404,
                message: "the bridge refused the exchange: the bootstrap token is wrong, \
                          already used, expired or revoked, or was minted for a different agent name"
                    .to_string(),
            },
            other => other,
        })?;
    let agent = out
        .get("agent")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            BridgeError::Transport("the bridge's exchange answer named no agent".to_string())
        })?
        .to_string();
    let fleet_value = out
        .get("fleet_token")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| {
            BridgeError::Transport(
                "the bridge's exchange answer carried no fleet credential".to_string(),
            )
        })?;
    let fleet_name = format!("bridge.{agent}-fleet-token");
    ctx.secrets.store(&fleet_name, fleet_value).map_err(|e| {
        BridgeError::Transport(format!(
            "the exchange minted {agent}'s credentials but storing the fleet credential as '{fleet_name}' failed: {e}. \
             The bootstrap token is burned; mint a fresh one and re-run the exchange."
        ))
    })?;
    let mut notes_name = None;
    if let Some(notes_value) = out.get("notes_token").and_then(Value::as_str) {
        let name = format!("bridge.{agent}-token");
        ctx.secrets.store(&name, notes_value).map_err(|e| {
            BridgeError::Transport(format!(
                "the exchange minted {agent}'s credentials and stored '{fleet_name}', but storing the notes token as '{name}' failed: {e}. \
                 The bootstrap token is burned; mint a fresh one and re-run the exchange."
            ))
        })?;
        notes_name = Some(name);
    }
    Ok(json!({
        "agent": agent,
        "kind": out.get("kind").cloned().unwrap_or(Value::Null),
        "groups": out.get("groups").cloned().unwrap_or(json!([])),
        "notes_identity": out.get("notes_identity").cloned().unwrap_or(Value::Null),
        "fleet_token_name": fleet_name,
        "notes_token_name": notes_name,
        "url": ctx.settings.url,
        "stored": true,
    }))
}

/// `fleet bridge notes <send|inbox|ack>` — the class/name notes
/// family (#13907's surface, with #14370's kind/reply_to): the
/// calling identity's own token authenticates as a Bearer header,
/// resolved by vault name like every other bridge token.
fn verb_notes(ctx: &mut Ctx, rest: &[String]) -> R<Value> {
    let Some((sub, nrest)) = rest.split_first() else {
        return Err(usage(
            "notes needs a verb: send, inbox, ack, get or watch (try: fleet bridge notes --help)",
        ));
    };
    match sub.as_str() {
        "send" => {
            let a = parse(nrest, &[], &["--to", "--kind", "--reply-to"])?;
            let kind = a.last("--kind").unwrap_or_else(|| "note".to_string());
            if !matches!(kind.as_str(), "note" | "instruction") {
                return Err(usage(format!(
                    "--kind is note or instruction, not '{kind}'"
                )));
            }
            let reply_to = a.uint("--reply-to")?;
            let to = a.last("--to");
            if to.is_none() && reply_to.is_none() {
                return Err(usage(
                    "notes send needs --to CLASS/NAME (a * segment wildcards), or --reply-to ID naming a note in your inbox",
                ));
            }
            let body = text_of(&a, ctx.stdin)?;
            let mut payload = json!({ "note": body, "kind": kind });
            if let Some(to) = to {
                payload["to"] = json!(to);
            }
            if let Some(id) = reply_to {
                payload["reply_to"] = json!(id);
            }
            let token = notes_token(ctx)?;
            let mut client = Client::new(&ctx.settings.url);
            client.call_bearer("POST", "/notes/send", Some(&payload), &token)
        }
        "inbox" => {
            let a = parse(nrest, &[], &[])?;
            a.no_positional("notes inbox")?;
            let token = notes_token(ctx)?;
            let mut client = Client::new(&ctx.settings.url);
            client.call_bearer("GET", "/notes/inbox", None, &token)
        }
        "ack" => {
            let a = parse(nrest, &[], &["--up-to"])?;
            let ids: Vec<u64> = a
                .positional
                .iter()
                .map(|p| {
                    p.parse::<u64>()
                        .ok()
                        .filter(|n| *n > 0)
                        .ok_or_else(|| usage(format!("note id '{p}' is not a positive number")))
                })
                .collect::<R<_>>()?;
            let up_to = a.uint("--up-to")?;
            if up_to.is_none() && ids.is_empty() {
                return Err(usage("notes ack needs --up-to ID or one or more IDs"));
            }
            let mut payload = json!({});
            if let Some(u) = up_to {
                payload["up_to_id"] = json!(u);
            }
            if !ids.is_empty() {
                payload["ids"] = json!(ids);
            }
            let token = notes_token(ctx)?;
            let mut client = Client::new(&ctx.settings.url);
            client.call_bearer("POST", "/notes/ack", Some(&payload), &token)
        }
        "get" => {
            let a = parse(nrest, &[], &[])?;
            let [identity] = a.positional.as_slice() else {
                return Err(usage("notes get needs one CLASS/NAME"));
            };
            let identity = signal_identity(identity)?;
            let client = Client::new(&ctx.settings.url);
            let signal = client.get_text(&format!("/{identity}"))?;
            Ok(json!({ "identity": identity, "signal": signal }))
        }
        "watch" => {
            let a = parse(nrest, &[], &["--wait", "--interval"])?;
            a.no_positional("notes watch")?;
            let wait = a.uint("--wait")?.unwrap_or(MAX_WAIT_SECS);
            if wait > MAX_WAIT_SECS {
                return Err(usage(format!("--wait is at most {MAX_WAIT_SECS} seconds")));
            }
            let interval = a
                .uint("--interval")?
                .unwrap_or(DEFAULT_INTERVAL_SECS)
                .max(1);
            let token = notes_token(ctx)?;
            let mut client = Client::new(&ctx.settings.url);
            notes_watch(ctx, &mut client, &token, wait, interval)
        }
        other => Err(usage(format!(
            "unknown notes verb '{other}' (send, inbox, ack, get or watch)"
        ))),
    }
}

/// The worker's reserved first segments: these paths are routes, not
/// a class's public signal (notes.js RESERVED_SIGNAL_CLASSES).
const RESERVED_SIGNAL_CLASSES: &[&str] = &[
    ".well-known",
    "api",
    "authorize",
    "enroll",
    "fleet",
    "healthz",
    "mcp",
    "notes",
    "p",
    "register",
    "token",
];

/// A `class/name` whose public signal can be read: two segments of
/// 1-64 `a-z 0-9 . _ -` starting with a letter or digit (the worker's
/// NOTE_SEGMENT), no wildcard, and not a reserved route.
fn signal_identity(raw: &str) -> R<String> {
    let lower = raw.trim().to_ascii_lowercase();
    let segment_ok = |seg: &str| {
        let b = seg.as_bytes();
        !b.is_empty()
            && b.len() <= 64
            && b[0].is_ascii_alphanumeric()
            && b.iter()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
    };
    match lower.split('/').collect::<Vec<_>>().as_slice() {
        [class, name] if segment_ok(class) && segment_ok(name) => {
            if RESERVED_SIGNAL_CLASSES.contains(class) {
                Err(usage(format!(
                    "'{class}' is a bridge route, not a notes class"
                )))
            } else {
                Ok(lower)
            }
        }
        _ => Err(usage(format!(
            "'{raw}' is not CLASS/NAME (two segments of a-z 0-9 . _ -, no wildcard)"
        ))),
    }
}

/// `notes watch`: read the inbox once (notes may already be waiting,
/// and its answer names this token's identity), then poll that
/// identity's public signal and read the inbox again only when the
/// signal changes. The signal also rotates on its own, so a change
/// with an empty inbox just resets the baseline. Returns the first
/// non-empty inbox answer, or the last (empty) one when --wait runs
/// out, with `woke` (pending | signal | null) and `waited` seconds.
fn notes_watch(ctx: &Ctx, client: &mut Client, token: &str, wait: u64, interval: u64) -> R<Value> {
    let started = Instant::now();
    let mut slept = 0u64;
    let finish = |mut inbox: Value, woke: Option<&str>, waited: u64| {
        inbox["woke"] = json!(woke);
        inbox["waited"] = json!(waited);
        inbox
    };
    let has_notes = |v: &Value| {
        v.get("notes")
            .and_then(Value::as_array)
            .is_some_and(|n| !n.is_empty())
    };
    let inbox = client.call_bearer("GET", "/notes/inbox", None, token)?;
    if has_notes(&inbox) {
        return Ok(finish(inbox, Some("pending"), 0));
    }
    let identity = inbox
        .get("identity")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            BridgeError::Transport("the bridge's inbox answer named no identity".to_string())
        })
        .and_then(signal_identity)?;
    let signal_path = format!("/{identity}");
    let mut last = client.get_text(&signal_path)?;
    let mut latest = inbox;
    loop {
        let elapsed = started.elapsed().as_secs().max(slept);
        if elapsed >= wait {
            return Ok(finish(latest, None, elapsed));
        }
        let step = interval.min(wait - elapsed);
        (ctx.sleep)(Duration::from_secs(step));
        slept += step;
        let now = client.get_text(&signal_path)?;
        if now != last {
            last = now;
            latest = client.call_bearer("GET", "/notes/inbox", None, token)?;
            if has_notes(&latest) {
                let waited = started.elapsed().as_secs().max(slept);
                return Ok(finish(latest, Some("signal"), waited));
            }
        }
    }
}

/// Runs one verb; Ok is the JSON to print.
pub fn execute(ctx: &mut Ctx, args: &[String]) -> R<Value> {
    let Some((verb, rest)) = args.split_first() else {
        return Err(usage("missing verb (try: fleet bridge --help)"));
    };
    match verb.as_str() {
        "send" => {
            let a = parse(
                rest,
                &["--command"],
                &["--to", "--sender", "--kind", "--limit", "--broaden"],
            )?;
            let limit = a.uint("--limit")?;
            // A broaden names the pattern send's batch and carries no
            // text or targets of its own: the bridge re-delivers the
            // original body to the matched agents the limit left over.
            if let Some(batch) = a.last("--broaden") {
                a.no_positional("send --broaden")?;
                let token = owner_token(ctx)?;
                let mut payload = json!({ "broaden": batch });
                if let Some(n) = limit {
                    payload["limit"] = json!(n);
                }
                return client_with(ctx, &token).call(
                    "POST",
                    &format!("/api/{token}/fleet/send"),
                    Some(&payload),
                );
            }
            let to: Vec<String> = a.all("--to").iter().flat_map(|t| split_list(t)).collect();
            if to.is_empty() {
                return Err(usage(
                    "send needs --to (agent:NAME, group:NAME or all; agents use post)",
                ));
            }
            let kind = if a.has("--command") {
                "command".to_string()
            } else {
                a.last("--kind").unwrap_or_else(|| "message".to_string())
            };
            let body = text_of(&a, ctx.stdin)?;
            let token = owner_token(ctx)?;
            let mut payload = json!({ "body": body, "to": to, "kind": kind });
            if let Some(s) = a.last("--sender") {
                payload["sender"] = json!(s);
            }
            if let Some(n) = limit {
                payload["limit"] = json!(n);
            }
            client_with(ctx, &token).call(
                "POST",
                &format!("/api/{token}/fleet/send"),
                Some(&payload),
            )
        }
        "offer" => {
            let a = parse(
                rest,
                &["--command"],
                &["--to", "--sender", "--kind", "--slots", "--ttl"],
            )?;
            let to: Vec<String> = a.all("--to").iter().flat_map(|t| split_list(t)).collect();
            if to.is_empty() {
                return Err(usage("offer needs --to (agent:NAME, group:NAME or all)"));
            }
            let Some(slots) = a.uint("--slots")? else {
                return Err(usage("offer needs --slots N (how many claims land)"));
            };
            let kind = if a.has("--command") {
                "command".to_string()
            } else {
                a.last("--kind").unwrap_or_else(|| "message".to_string())
            };
            let body = text_of(&a, ctx.stdin)?;
            let token = owner_token(ctx)?;
            let mut payload = json!({ "body": body, "to": to, "kind": kind, "slots": slots });
            if let Some(s) = a.last("--sender") {
                payload["sender"] = json!(s);
            }
            if let Some(ttl) = a.uint("--ttl")? {
                payload["ttl_seconds"] = json!(ttl);
            }
            client_with(ctx, &token).call(
                "POST",
                &format!("/api/{token}/fleet/offer"),
                Some(&payload),
            )
        }
        "offers" => {
            let a = parse(rest, &[], &[])?;
            a.no_positional("offers")?;
            let token = owner_token(ctx)?;
            client_with(ctx, &token).call("GET", &format!("/api/{token}/fleet/offers"), None)
        }
        "agents" => {
            let a = parse(rest, &[], &[])?;
            a.no_positional("agents")?;
            let token = owner_token(ctx)?;
            client_with(ctx, &token).call("GET", &format!("/api/{token}/fleet/agents"), None)
        }
        "log" => {
            let a = parse(
                rest,
                &[],
                &[
                    "--agent",
                    "--direction",
                    "--kind",
                    "--since-id",
                    "--before-id",
                    "--limit",
                    "--order",
                ],
            )?;
            a.no_positional("log")?;
            let q = query(&[
                ("agent", a.last("--agent")),
                ("direction", a.last("--direction")),
                ("kind", a.last("--kind")),
                ("since_id", a.uint("--since-id")?.map(|n| n.to_string())),
                ("before_id", a.uint("--before-id")?.map(|n| n.to_string())),
                ("limit", a.uint("--limit")?.map(|n| n.to_string())),
                ("order", a.last("--order")),
            ]);
            let token = owner_token(ctx)?;
            client_with(ctx, &token).call("GET", &format!("/api/{token}/fleet/messages{q}"), None)
        }
        "pipeline" => {
            let a = parse(
                rest,
                &["--unread"],
                &[
                    "--kind",
                    "--sender",
                    "--receiver",
                    "--surface",
                    "--batch",
                    "--work-state",
                    "--since-id",
                    "--before-id",
                    "--limit",
                    "--order",
                ],
            )?;
            a.no_positional("pipeline")?;
            let q = query(&[
                ("kind", a.last("--kind")),
                ("sender", a.last("--sender")),
                ("receiver", a.last("--receiver")),
                ("surface", a.last("--surface")),
                ("batch", a.last("--batch")),
                ("unread_only", a.has("--unread").then(|| "true".to_string())),
                ("work_state", a.last("--work-state")),
                ("since_id", a.uint("--since-id")?.map(|n| n.to_string())),
                ("before_id", a.uint("--before-id")?.map(|n| n.to_string())),
                ("limit", a.uint("--limit")?.map(|n| n.to_string())),
                ("order", a.last("--order")),
            ]);
            let token = owner_token(ctx)?;
            client_with(ctx, &token).call("GET", &format!("/api/{token}/pipeline{q}"), None)
        }
        "directory" => {
            let a = parse(rest, &[], &[])?;
            a.no_positional("directory")?;
            let token = owner_token(ctx)?;
            client_with(ctx, &token).call("GET", &format!("/api/{token}/agents"), None)
        }
        "credentials" => {
            let a = parse(rest, &[], &[])?;
            a.no_positional("credentials")?;
            let token = owner_token(ctx)?;
            client_with(ctx, &token).call("GET", &format!("/api/{token}/fleet/credentials"), None)
        }
        "revoke" => {
            let a = parse(rest, &[], &["--agent", "--identity"])?;
            a.no_positional("revoke")?;
            // Checked before any credential is resolved: a usage error
            // is a usage error even on a seat with no tokens.
            let target = match (a.last("--agent"), a.last("--identity")) {
                (Some(agent), None) => ("fleet", agent),
                (None, Some(identity)) => ("notes", identity),
                _ => {
                    return Err(usage("revoke needs exactly one of --agent or --identity"));
                }
            };
            let token = owner_token(ctx)?;
            match target {
                ("fleet", agent) => client_with(ctx, &token).call(
                    "POST",
                    &format!("/api/{token}/fleet/credentials/revoke"),
                    Some(&json!({ "agent": agent })),
                ),
                (_, identity) => client_with(ctx, &token).call(
                    "POST",
                    &format!("/api/{token}/notes-tokens/revoke"),
                    Some(&json!({ "identity": identity })),
                ),
            }
        }
        "mint-enroll" => {
            let a = parse(
                rest,
                &[],
                &["--agent", "--kind", "--groups", "--notes-identity", "--ttl"],
            )?;
            a.no_positional("mint-enroll")?;
            // Checked before any credential is resolved: a usage error
            // is a usage error even on a seat with no tokens.
            let agent = a
                .last("--agent")
                .ok_or_else(|| usage("mint-enroll needs --agent NAME"))?;
            let mut payload = json!({ "agent": agent });
            if let Some(kind) = a.last("--kind") {
                payload["kind"] = json!(kind);
            }
            if let Some(groups) = a.last("--groups") {
                payload["groups"] = json!(split_list(&groups));
            }
            if let Some(identity) = a.last("--notes-identity") {
                payload["notes_identity"] = json!(identity);
            }
            if let Some(ttl) = a.uint("--ttl")? {
                payload["ttl_seconds"] = json!(ttl);
            }
            let token = owner_token(ctx)?;
            client_with(ctx, &token).call(
                "POST",
                &format!("/api/{token}/enroll-tokens"),
                Some(&payload),
            )
        }
        "enroll-tokens" => {
            let a = parse(rest, &[], &[])?;
            a.no_positional("enroll-tokens")?;
            let token = owner_token(ctx)?;
            client_with(ctx, &token).call("GET", &format!("/api/{token}/enroll-tokens"), None)
        }
        "wake-hooks" => {
            let a = parse(rest, &["--set", "--remove"], &["--address"])?;
            // Checked before any credential is resolved, like every
            // usage error here. The three forms are exclusive: list,
            // set (the hook as JSON on stdin, so a hook token never
            // travels as an argument), or remove one address.
            match (a.has("--set"), a.has("--remove")) {
                (false, false) => {
                    a.no_positional("wake-hooks")?;
                    if a.last("--address").is_some() {
                        return Err(usage("wake-hooks: --address goes with --remove"));
                    }
                    let token = owner_token(ctx)?;
                    client_with(ctx, &token).call("GET", &format!("/api/{token}/wake-hooks"), None)
                }
                (true, false) => {
                    if a.last("--address").is_some() {
                        return Err(usage(
                            "wake-hooks --set: the address is in the JSON on stdin",
                        ));
                    }
                    let text = text_of(&a, ctx.stdin).map_err(|_| {
                        usage("wake-hooks --set: give the hook as a JSON object on stdin (fleet bridge wake-hooks --set -)")
                    })?;
                    let payload: Value = serde_json::from_str(&text)
                        .map_err(|e| usage(format!("wake-hooks --set: stdin is not JSON: {e}")))?;
                    if !payload.is_object() {
                        return Err(usage(
                            "wake-hooks --set: stdin must be a JSON object {address, url, token?}",
                        ));
                    }
                    let token = owner_token(ctx)?;
                    client_with(ctx, &token).call(
                        "POST",
                        &format!("/api/{token}/wake-hooks"),
                        Some(&payload),
                    )
                }
                (false, true) => {
                    a.no_positional("wake-hooks --remove")?;
                    let address = a
                        .last("--address")
                        .ok_or_else(|| usage("wake-hooks --remove needs --address A"))?;
                    let token = owner_token(ctx)?;
                    client_with(ctx, &token).call(
                        "POST",
                        &format!("/api/{token}/wake-hooks"),
                        Some(&json!({ "address": address })),
                    )
                }
                (true, true) => Err(usage("wake-hooks: --set and --remove are different acts")),
            }
        }
        "revoke-enroll" => {
            let a = parse(rest, &[], &["--agent", "--token-hash"])?;
            a.no_positional("revoke-enroll")?;
            // Exactly one form, checked before any credential is
            // resolved, like every usage error here. The hash is not
            // a credential (it cannot enroll anything), so it may
            // ride in an argument where a token value never may.
            let body = match (a.last("--agent"), a.last("--token-hash")) {
                (Some(agent), None) => json!({ "agent": agent }),
                (None, Some(hash)) => json!({ "token_hash": hash }),
                _ => {
                    return Err(usage(
                        "revoke-enroll needs exactly one of --agent or --token-hash",
                    ));
                }
            };
            let token = owner_token(ctx)?;
            client_with(ctx, &token).call(
                "POST",
                &format!("/api/{token}/enroll-tokens/revoke"),
                Some(&body),
            )
        }
        "mint-credential" => {
            let a = parse(rest, &[], &["--agent", "--scopes", "--addr"])?;
            a.no_positional("mint-credential")?;
            let agent = a
                .last("--agent")
                .ok_or_else(|| usage("mint-credential needs --agent NAME"))?;
            let mut payload = json!({ "agent": agent });
            if let Some(scopes) = a.last("--scopes") {
                payload["scopes"] = json!(split_list(&scopes));
            }
            if let Some(addr) = a.last("--addr") {
                payload["addr"] = json!(addr);
            }
            let token = owner_token(ctx)?;
            client_with(ctx, &token).call(
                "POST",
                &format!("/api/{token}/fleet/credentials/mint"),
                Some(&payload),
            )
        }
        "hello" => {
            let a = parse(rest, &[], &["--agent", "--groups"])?;
            a.no_positional("hello")?;
            let agent = agent_of(&a, &ctx.settings)?;
            let mut payload = json!({ "agent": agent });
            if let Some(g) = groups_of(&a, &ctx.settings) {
                payload["groups"] = json!(g);
            }
            let token = fleet_token(ctx)?;
            client_with(ctx, &token).call("POST", &format!("/fleet/{token}/hello"), Some(&payload))
        }
        "inbox" => {
            let a = parse(
                rest,
                &["--commands", "--messages", "--all", "--take"],
                &[
                    "--agent",
                    "--groups",
                    "--since-id",
                    "--limit",
                    "--wait",
                    "--interval",
                ],
            )?;
            a.no_positional("inbox")?;
            if a.has("--commands") && a.has("--messages") {
                return Err(usage("--commands and --messages exclude each other"));
            }
            let agent = agent_of(&a, &ctx.settings)?;
            let kind = if a.has("--commands") {
                Some("command".to_string())
            } else if a.has("--messages") {
                Some("message".to_string())
            } else {
                None
            };
            let wait = a.uint("--wait")?.unwrap_or(0);
            if wait > MAX_WAIT_SECS {
                return Err(usage(format!("--wait is at most {MAX_WAIT_SECS} seconds")));
            }
            let interval = a
                .uint("--interval")?
                .unwrap_or(DEFAULT_INTERVAL_SECS)
                .max(1);
            let q = query(&[
                ("agent", Some(agent)),
                ("groups", groups_of(&a, &ctx.settings).map(|g| g.join(","))),
                ("kind", kind),
                ("since_id", a.uint("--since-id")?.map(|n| n.to_string())),
                ("limit", a.uint("--limit")?.map(|n| n.to_string())),
                ("unread_only", a.has("--all").then(|| "false".to_string())),
                ("mark_read", a.has("--take").then(|| "true".to_string())),
            ]);
            let token = fleet_token(ctx)?;
            let client = client_with(ctx, &token);
            let path = format!("/fleet/{token}/inbox{q}");
            let started = Instant::now();
            loop {
                let out = client.call("GET", &path, None)?;
                let empty = out
                    .get("messages")
                    .and_then(Value::as_array)
                    .is_none_or(|m| m.is_empty());
                let elapsed = started.elapsed().as_secs();
                if !empty || elapsed >= wait {
                    return Ok(out);
                }
                (ctx.sleep)(Duration::from_secs(interval.min(wait - elapsed)));
            }
        }
        "post" => {
            let a = parse(
                rest,
                &["--instruction", "--command"],
                &["--agent", "--reply-to", "--to", "--kind"],
            )?;
            let agent = agent_of(&a, &ctx.settings)?;
            let to: Vec<String> = a.all("--to").iter().flat_map(|t| split_list(t)).collect();
            let kind = if a.has("--instruction") || a.has("--command") {
                Some("instruction".to_string())
            } else {
                a.last("--kind")
            };
            if let Some(k) = &kind
                && !matches!(k.as_str(), "message" | "instruction" | "command")
            {
                return Err(usage(format!(
                    "--kind is message or instruction, not '{k}'"
                )));
            }
            if kind.is_some() && to.is_empty() {
                return Err(usage(
                    "an instruction needs --to (central and replies take messages only)",
                ));
            }
            let body = text_of(&a, ctx.stdin)?;
            let mut payload = json!({ "agent": agent, "body": body });
            if !to.is_empty() {
                payload["to"] = json!(to);
            }
            if let Some(k) = kind {
                payload["kind"] = json!(k);
            }
            if let Some(id) = a.uint("--reply-to")? {
                payload["reply_to"] = json!(id);
            }
            let token = fleet_token(ctx)?;
            client_with(ctx, &token).call("POST", &format!("/fleet/{token}/send"), Some(&payload))
        }
        "ack" => {
            let a = parse(rest, &[], &["--agent", "--up-to"])?;
            let agent = agent_of(&a, &ctx.settings)?;
            let ids: Vec<u64> =
                a.positional
                    .iter()
                    .map(|p| {
                        p.parse::<u64>().ok().filter(|n| *n > 0).ok_or_else(|| {
                            usage(format!("message id '{p}' is not a positive number"))
                        })
                    })
                    .collect::<R<_>>()?;
            let up_to = a.uint("--up-to")?;
            if up_to.is_none() && ids.is_empty() {
                return Err(usage("ack needs --up-to ID or one or more IDs"));
            }
            let mut payload = json!({ "agent": agent });
            if let Some(u) = up_to {
                payload["up_to_id"] = json!(u);
            }
            if !ids.is_empty() {
                payload["ids"] = json!(ids);
            }
            let token = fleet_token(ctx)?;
            client_with(ctx, &token).call("POST", &format!("/fleet/{token}/ack"), Some(&payload))
        }
        "work" => {
            let a = parse(rest, &[], &["--agent", "--state", "--task"])?;
            let agent = agent_of(&a, &ctx.settings)?;
            let state = a
                .last("--state")
                .ok_or_else(|| usage("work needs --state acted, queued or done"))?;
            if !matches!(state.as_str(), "acted" | "queued" | "done") {
                return Err(usage(format!(
                    "--state is acted, queued or done, not '{state}'"
                )));
            }
            let task = a.uint("--task")?;
            if state == "queued" && task.is_none() {
                return Err(usage(
                    "queued work needs --task: the fleet queue task id it became",
                ));
            }
            let ids: Vec<u64> =
                a.positional
                    .iter()
                    .map(|p| {
                        p.parse::<u64>().ok().filter(|n| *n > 0).ok_or_else(|| {
                            usage(format!("message id '{p}' is not a positive number"))
                        })
                    })
                    .collect::<R<_>>()?;
            if ids.is_empty() {
                return Err(usage("work needs at least one message ID"));
            }
            let token = fleet_token(ctx)?;
            let client = client_with(ctx, &token);
            let mut marked = Vec::new();
            for id in &ids {
                let mut payload = json!({ "agent": &agent, "id": id, "state": &state });
                if let Some(t) = task {
                    payload["task_id"] = json!(t);
                }
                marked.push(client.call(
                    "POST",
                    &format!("/fleet/{token}/work"),
                    Some(&payload),
                )?);
            }
            if marked.len() == 1 {
                Ok(marked.pop().unwrap())
            } else {
                Ok(json!({ "marked": marked }))
            }
        }
        "exchange" => verb_exchange(ctx, rest),
        "notes" => verb_notes(ctx, rest),
        "health" => {
            let a = parse(rest, &[], &[])?;
            a.no_positional("health")?;
            Client::new(&ctx.settings.url).call("GET", "/healthz", None)
        }
        other => Err(usage(format!(
            "unknown verb '{other}' (try: fleet bridge --help)"
        ))),
    }
}

/// The domain entry point: `fleet bridge <verb> ...`.
pub fn run(config: &Config, args: &[String]) -> i32 {
    // Help wins over every other argument (the #10838 convention):
    // `fleet bridge <verb> --help` prints the verb's own usage (#13197);
    // bare or top-level help prints the whole-domain USAGE.
    if args.is_empty() {
        println!("{USAGE}");
        return EXIT_USAGE;
    }
    if matches!(args[0].as_str(), "-h" | "--help" | "help") {
        println!("{USAGE}");
        return EXIT_OK;
    }
    if args.iter().skip(1).any(|a| a == "--help" || a == "-h") {
        match verb_usage(&args[0]) {
            Some(text) => println!("{text}"),
            None => println!("{USAGE}"),
        }
        return EXIT_OK;
    }
    let env = |k: &str| std::env::var(k).ok();
    let settings = match Settings::resolve(config, &env) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            return e.exit_code();
        }
    };
    let scope = match fleet_secrets::Scope::from_config(config) {
        Ok(scope) => scope,
        Err(e) => {
            eprintln!("fleet bridge: secrets prefix: {e}");
            return EXIT_USAGE;
        }
    };
    let secrets = ScopedSecrets::new(scope, ChainSecrets(config));
    let mut stdin = std::io::stdin();
    let sleep = |d: Duration| std::thread::sleep(d);
    let mut ctx = Ctx {
        settings,
        secrets: &secrets,
        stdin: &mut stdin,
        sleep: &sleep,
    };
    match execute(&mut ctx, args) {
        Ok(v) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&v).unwrap_or_else(|_| v.to_string())
            );
            EXIT_OK
        }
        Err(e) => {
            eprintln!("{e}");
            e.exit_code()
        }
    }
}

#[cfg(test)]
mod tests;
