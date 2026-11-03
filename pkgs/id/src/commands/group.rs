//! `id group`, `id delegate` and `id revoke` (P6.7a and P6.7b): parse the arguments, act on the group
//! log, and print.
//!
//! The rules are the fold's, and the actions are the library's ([`crate::groups::actions`]). This
//! module only turns arguments into calls and results into lines or JSON. A refused action is an
//! error, so the exit status says so. Its event stays in the log, as the author's record.
//!
//! # Output
//!
//! Each action prints one line, `<command> key=value ...`, with `group`, the action's fields,
//! `outcome=applied` and `event=<ID>`. With `--json` it prints one object with the same keys and a
//! `command` key. `list` prints one line (or one array element) per group, with no command word.

use std::path::PathBuf;

use anyhow::{Result, anyhow, bail};
use clap::ValueEnum;
use p2panda_core::Hash;
use serde_json::{Map, Value, json};

use crate::cli::{DelegateArgs, GroupCommand, GroupRemoveMode, GroupTemplate, RevokeArgs};
use crate::groups::actions::{Action, NewGroup};
use crate::groups::log::{FILE_NAME, GroupLog, hash_from_hex, key_from_hex};
use crate::groups::policy::{
    History, Join, OnMemberRemoved, Removal, RemoveMode, RemoveSuccessor, SuccessorKind,
    TransferOpts,
};
use crate::groups::warnings::{Evaluator, Observation, Warning, size_tier};
use crate::groups::{Body, Delegate, GroupView, Key, MemberOp, Outcome};
use crate::invites::Invitation;
use crate::{KEY_FILE, load_or_create_keypair};

/// The log a command reads and appends to: `--groups-log`, or the default file in the data directory.
pub(crate) fn log_path(path: Option<PathBuf>) -> PathBuf {
    path.unwrap_or_else(|| PathBuf::from(FILE_NAME))
}

/// The account this node acts as: its node key, which `id id` prints.
async fn author() -> Result<Key> {
    let key = load_or_create_keypair(KEY_FILE).await?;
    Ok(*key.public().as_bytes())
}

/// An account or ID as its 64 hex characters.
pub(super) fn hex(bytes: &Key) -> String {
    Hash::from_bytes(*bytes).to_hex()
}

/// A value-enum's name, as the CLI spells it.
fn name_of<T: ValueEnum>(value: &T) -> String {
    value
        .to_possible_value()
        .map(|possible| possible.get_name().to_owned())
        .unwrap_or_default()
}

/// A field's value as text: strings without quotes, anything else as JSON.
///
/// Plain-text safety (#1611): a string containing whitespace or control characters is
/// rendered as a JSON string literal — double-quoted, with the whitespace and control
/// bytes escaped (`\n`, `\t`, `\u00XX`, and so on). A plain row is therefore always
/// exactly one physical line, and a stored name or value can never forge extra fields
/// or rows: any `key=value`-looking text inside such a value sits inside the quotes, so
/// a row's real trailing fields (such as `author=` and `event=`) are the only unquoted
/// occurrences. Strings without whitespace or control characters render byte-identically
/// to before, and `--json` output (which goes through [`object`]) is unchanged.
///
/// Format characters (#3111, design #3455 Phase 2): a string containing a Unicode
/// format character (Cf, per the shared [`crate::is_format_char`] set landed by
/// #2839) is quoted the same way, and the format characters themselves are escaped
/// as `\uXXXX` inside the literal — `serde_json` alone would leave them raw, so a
/// bidi override such as `U+202E` could still visually reorder a row's real fields
/// at the terminal even inside quotes. A format character on its own therefore
/// triggers quoting too, exactly like a control character.
fn text(value: &Value) -> String {
    match value {
        Value::String(string)
            if string
                .chars()
                .any(|c| c.is_whitespace() || c.is_control() || crate::is_format_char(c)) =>
        {
            quoted(string)
        }
        Value::String(string) => string.clone(),
        other => other.to_string(),
    }
}

fn push_unicode_escape(rendered: &mut String, c: char) {
    use std::fmt::Write as _;

    let code_point = u32::from(c);
    if code_point <= 0xFFFF {
        // Writing into a String cannot fail.
        let _ = write!(rendered, "\\u{code_point:04x}");
    } else {
        // JSON escapes supplementary scalars as a UTF-16 surrogate pair.
        let scalar = code_point - 0x1_0000;
        let high = 0xD800 + (scalar >> 10);
        let low = 0xDC00 + (scalar & 0x3FF);
        let _ = write!(rendered, "\\u{high:04x}\\u{low:04x}");
    }
}

/// `string` as a double-quoted JSON string literal, with format characters escaped.
///
/// Identical to `serde_json`'s string rendering except that Unicode format
/// characters (Cf) are written as `\uXXXX` instead of raw: the result still parses
/// as the same JSON string, but no raw format character reaches the terminal.
fn quoted(string: &str) -> String {
    let mut rendered = String::with_capacity(string.len() + 2);
    rendered.push('"');
    for c in string.chars() {
        match c {
            '"' => rendered.push_str("\\\""),
            '\\' => rendered.push_str("\\\\"),
            '\u{0008}' => rendered.push_str("\\b"),
            '\u{000c}' => rendered.push_str("\\f"),
            '\n' => rendered.push_str("\\n"),
            '\r' => rendered.push_str("\\r"),
            '\t' => rendered.push_str("\\t"),
            c if crate::is_format_char(c) || (c.is_control() && (c as u32) < 0x20) => {
                push_unicode_escape(&mut rendered, c);
            }
            c => rendered.push(c),
        }
    }
    rendered.push('"');
    rendered
}

/// The fields as `key=value` tokens.
pub(super) fn tokens(fields: &[(&str, Value)]) -> String {
    fields
        .iter()
        .map(|(key, value)| format!("{key}={}", text(value)))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The fields as one JSON object.
pub(super) fn object(fields: &[(&str, Value)]) -> Value {
    let map: Map<String, Value> = fields
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect();
    Value::Object(map)
}

/// Prints one applied action. A refused action is an error, which names the reason and the event.
fn print_action(
    json: bool,
    command: &str,
    fields: Vec<(&str, Value)>,
    action: &Action,
) -> Result<()> {
    if let Outcome::Refused(refusal) = &action.outcome {
        let reason = action
            .outcome
            .reason()
            .map(|reason| reason.to_string())
            .unwrap_or_default();
        bail!(
            "refused: {command} group={} event={} reason={reason} ({refusal:?})",
            action.group.to_hex(),
            action.event.to_hex()
        );
    }
    let mut all = vec![("group", json!(action.group.to_hex()))];
    all.extend(fields);
    all.push(("outcome", json!("applied")));
    all.push(("event", json!(action.event.to_hex())));
    if json {
        let mut with_command = vec![("command", json!(command))];
        with_command.extend(all);
        println!("{}", object(&with_command));
    } else {
        println!("{command} {}", tokens(&all));
    }
    Ok(())
}

const fn join_name(join: Join) -> &'static str {
    match join {
        Join::ManagerAdd => "manager_add",
        Join::Invite => "invite",
        Join::MutualRequest => "mutual_request",
        Join::Open => "open",
    }
}

const fn removed_name(on: OnMemberRemoved) -> &'static str {
    match on {
        OnMemberRemoved::Continue => "continue",
        OnMemberRemoved::Dissolve => "dissolve",
    }
}

const fn removal_name(removal: Removal) -> &'static str {
    match removal {
        Removal::Strong => "strong",
        Removal::Weak => "weak",
    }
}

fn history_name(history: History) -> String {
    match history {
        History::Full => "full".to_owned(),
        History::Recent(epochs) => format!("recent:{epochs}"),
        History::FromJoin => "from_join".to_owned(),
        History::Causal => "causal".to_owned(),
    }
}

/// The fields of a group's policy, as `policy` shows them.
fn policy_fields(view: &GroupView) -> Vec<(&'static str, Value)> {
    let policy = &view.policy;
    vec![
        (
            "cap",
            json!(
                policy
                    .member_cap
                    .map_or_else(|| "none".to_owned(), |cap| cap.to_string())
            ),
        ),
        ("join", json!(join_name(policy.join))),
        ("on_removed", json!(removed_name(policy.on_member_removed))),
        ("removal", json!(removal_name(policy.removal))),
        ("delegation", json!(name_of(&policy.delegation_mode()))),
        ("history", json!(history_name(policy.history))),
    ]
}

/// The size tier and any Evaluator crossing warning for a group (P8.6).
fn evaluate_group(group: Hash, members: usize) -> (usize, Option<String>) {
    let tier = size_tier(members);
    let observation = Observation {
        group,
        client_made: true,
        members,
        readds: &[],
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut evaluator = Evaluator::default();
    let warnings = evaluator.observe(&observation, now);
    let size_warning = warnings
        .iter()
        .filter(|w| matches!(w, Warning::Size { .. }))
        .last();
    let churn_warning = warnings.iter().find(|w| matches!(w, Warning::Churn { .. }));
    let warning = match (size_warning, churn_warning) {
        (Some(size), Some(churn)) => Some(format!("{}; {}", size.message(), churn.message())),
        (Some(size), None) => Some(size.message()),
        (None, Some(churn)) => Some(churn.message()),
        (None, None) => None,
    };
    (tier, warning)
}

/// Runs an `id group` command.
///
/// # Errors
///
/// Fails if an argument is malformed, the log cannot be read or written, or the action is refused.
pub async fn cmd_group(path: Option<PathBuf>, json: bool, command: GroupCommand) -> Result<()> {
    let mut log = GroupLog::open(&log_path(path))?;
    match command {
        GroupCommand::Create {
            template,
            with,
            max,
            admins_on_create,
        } => {
            let (new, fields) = match template {
                GroupTemplate::Normal => {
                    if with.is_some() {
                        bail!(
                            "--with names the other side of a friendship: add --template friendship"
                        );
                    }
                    let fields = vec![
                        ("template", json!("normal")),
                        (
                            "max",
                            json!(max.map_or_else(|| "none".to_owned(), |n| n.to_string())),
                        ),
                    ];
                    (
                        NewGroup::Normal {
                            max,
                            admins_on_create,
                        },
                        fields,
                    )
                }
                GroupTemplate::Friendship => {
                    if max.is_some() {
                        bail!("a friendship has two members, so it takes no --max");
                    }
                    let with =
                        with.ok_or_else(|| anyhow!("--template friendship needs --with ACCOUNT"))?;
                    let with = key_from_hex(&with)?;
                    let fields = vec![
                        ("template", json!("friendship")),
                        ("with", json!(hex(&with))),
                    ];
                    (
                        NewGroup::Friendship {
                            with,
                            admins_on_create,
                        },
                        fields,
                    )
                }
            };
            let action = log.create(author().await?, new)?;
            print_action(json, "create", fields, &action)
        }
        GroupCommand::Policy { group } => {
            let group = hash_from_hex(&group)?;
            let view = log.view(group)?;
            let mut fields = vec![("group", json!(group.to_hex()))];
            fields.extend(policy_fields(&view));
            let (tier, warning) = evaluate_group(group, view.members.len());
            fields.push(("tier", json!(tier)));
            if let Some(warning) = &warning {
                fields.push(("warning", json!(warning)));
            }
            if json {
                let mut with_command = vec![("command", json!("policy"))];
                with_command.extend(fields);
                println!("{}", object(&with_command));
            } else {
                println!("policy {}", tokens(&fields));
            }
            Ok(())
        }
        GroupCommand::Add {
            group,
            member,
            role,
        } => {
            let group = hash_from_hex(&group)?;
            let member = key_from_hex(&member)?;
            let action = log.add(author().await?, group, member, role)?;
            let fields = vec![
                ("member", json!(hex(&member))),
                ("role", json!(name_of(&role))),
            ];
            print_action(json, "add", fields, &action)
        }
        GroupCommand::Remove {
            group,
            member,
            mode,
            successor_kind,
            successor,
        } => {
            let group = hash_from_hex(&group)?;
            let member = key_from_hex(&member)?;
            let selected_mode = mode.unwrap_or_else(|| {
                if successor.is_some() || successor_kind.is_some() {
                    GroupRemoveMode::Transfer
                } else {
                    GroupRemoveMode::ServerDefault
                }
            });
            if selected_mode != GroupRemoveMode::Transfer
                && (successor.is_some() || successor_kind.is_some())
            {
                bail!("--successor and --successor-kind require --mode transfer");
            }
            let successor_key = successor.as_deref().map(key_from_hex).transpose()?;
            let remove_mode = match selected_mode {
                GroupRemoveMode::ServerDefault => RemoveMode::ServerDefault,
                GroupRemoveMode::Revoke => RemoveMode::Revoke,
                GroupRemoveMode::KeepPrior => RemoveMode::KeepPrior,
                GroupRemoveMode::ReassignToGroup => RemoveMode::ReassignToGroup,
                GroupRemoveMode::Transfer => {
                    let kind = successor_kind.unwrap_or(SuccessorKind::Member);
                    let successor = match (kind, successor_key) {
                        (SuccessorKind::Member, Some(key)) => RemoveSuccessor::Member(key),
                        (SuccessorKind::Designated, Some(key)) => RemoveSuccessor::Designated(key),
                        (SuccessorKind::Admin, None) => RemoveSuccessor::Admin,
                        (SuccessorKind::Actor, None) => RemoveSuccessor::Actor,
                        (SuccessorKind::Member | SuccessorKind::Designated, None) => {
                            bail!("--successor is required for a member or designated successor")
                        }
                        (SuccessorKind::Admin | SuccessorKind::Actor, Some(_)) => {
                            bail!("--successor is not used for an admin or actor successor")
                        }
                    };
                    RemoveMode::Transfer(TransferOpts { successor })
                }
            };
            let action = log.remove(author().await?, group, member, remove_mode)?;
            let resolved_mode = log
                .events()
                .iter()
                .find(|event| event.id == action.event)
                .and_then(|event| match &event.body {
                    Body::Member(MemberOp::Remove { mode, .. }) => Some(*mode),
                    _ => None,
                })
                .unwrap_or(remove_mode);
            let mut fields = vec![("member", json!(hex(&member)))];
            match resolved_mode {
                RemoveMode::Revoke => fields.push(("mode", json!("revoke"))),
                RemoveMode::KeepPrior => fields.push(("mode", json!("keep_prior"))),
                RemoveMode::ReassignToGroup => {
                    fields.push(("mode", json!("reassign_to_group")));
                }
                RemoveMode::ServerDefault => fields.push(("mode", json!("server_default"))),
                RemoveMode::Transfer(opts) => {
                    fields.push(("mode", json!("transfer")));
                    fields.push(("successor_kind", json!(name_of(&opts.successor.kind()))));
                    match opts.successor {
                        RemoveSuccessor::Member(key) | RemoveSuccessor::Designated(key) => {
                            fields.push(("successor", json!(hex(&key))));
                        }
                        RemoveSuccessor::Admin | RemoveSuccessor::Actor => {}
                    }
                }
            }
            print_action(json, "remove", fields, &action)
        }
        GroupCommand::SetRole {
            group,
            member,
            role,
        } => {
            let group = hash_from_hex(&group)?;
            let member = key_from_hex(&member)?;
            let action = log.set_role(author().await?, group, member, role)?;
            let fields = vec![
                ("member", json!(hex(&member))),
                ("role", json!(name_of(&role))),
            ];
            print_action(json, "role", fields, &action)
        }
        GroupCommand::List => {
            let projection = log.projection()?;
            let rows: Vec<Vec<(&str, Value)>> = projection
                .groups()
                .map(|(id, view)| {
                    let (tier, warning) = evaluate_group(*id, view.members.len());
                    let mut row = vec![
                        ("group", json!(id.to_hex())),
                        ("active", json!(view.is_active())),
                        ("members", json!(view.members.len())),
                        ("tier", json!(tier)),
                    ];
                    if let Some(warning) = warning {
                        row.push(("warning", json!(warning)));
                    }
                    row
                })
                .collect();
            if json {
                let array: Vec<Value> = rows.iter().map(|row| object(row)).collect();
                println!("{}", Value::Array(array));
            } else {
                for row in &rows {
                    println!("{}", tokens(row));
                }
            }
            Ok(())
        }
        GroupCommand::Join { invite } => {
            Invitation::parse(&invite)?;
            bail!(
                "group join is unavailable: invitation parsing works, but admission and network \
                 acceptance are not implemented yet; no group membership was recorded"
            )
        }
    }
}

/// Runs `id delegate`: sets the mode if asked, then grants the scope.
///
/// # Errors
///
/// Fails if an argument is malformed, the log cannot be read or written, or the action is refused.
pub async fn cmd_delegate(path: Option<PathBuf>, args: DelegateArgs) -> Result<()> {
    let mut log = GroupLog::open(&log_path(path))?;
    let group = hash_from_hex(&args.group)?;
    let grantee = key_from_hex(&args.grantee)?;
    let delegate = Delegate {
        grantee,
        scope: args.scope,
        can_redelegate: args.redelegate,
    };
    let delegated = log.delegate(author().await?, group, delegate, args.mode)?;
    if let (Some(change), Some(mode)) = (&delegated.mode_change, args.mode) {
        print_action(
            args.json,
            "policy",
            vec![("delegation", json!(name_of(&mode)))],
            change,
        )?;
    }
    if let Some(grant) = &delegated.grant {
        let fields = vec![
            ("grantee", json!(hex(&grantee))),
            ("scope", json!(name_of(&args.scope))),
            ("redelegate", json!(args.redelegate)),
        ];
        print_action(args.json, "delegate", fields, grant)?;
    }
    Ok(())
}

/// Runs `id revoke`: vetoes the grant.
///
/// # Errors
///
/// Fails if an argument is malformed, the log cannot be read or written, or the veto is refused.
pub async fn cmd_revoke(path: Option<PathBuf>, args: RevokeArgs) -> Result<()> {
    let mut log = GroupLog::open(&log_path(path))?;
    let group = hash_from_hex(&args.group)?;
    let grant = hash_from_hex(&args.grant)?;
    let action = log.revoke(author().await?, group, grant)?;
    let fields = vec![("grant", json!(grant.to_hex()))];
    print_action(args.json, "revoke", fields, &action)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// A claim-shaped row: the stored name/value sit before the real author/event.
    fn row(name: &str, value: &str) -> String {
        tokens(&[
            ("subject", json!("deadbeef")),
            ("name", json!(name)),
            ("value", json!(value)),
            ("author", json!("cafe")),
            ("event", json!("beef")),
        ])
    }

    #[test]
    fn test_space_free_values_render_byte_identically() {
        assert_eq!(
            row("tag", "plain-value_1"),
            "subject=deadbeef name=tag value=plain-value_1 author=cafe event=beef"
        );
        assert_eq!(text(&json!(42)), "42");
        assert_eq!(text(&json!(true)), "true");
    }

    #[test]
    fn test_space_in_value_is_quoted() {
        assert_eq!(
            row("tag", "a b"),
            "subject=deadbeef name=tag value=\"a b\" author=cafe event=beef"
        );
    }

    #[test]
    fn test_newline_in_value_stays_one_line() {
        let rendered = row("tag", "a\nb");
        assert!(
            !rendered.contains('\n'),
            "row must be one physical line: {rendered}"
        );
        assert!(rendered.contains("value=\"a\\nb\""));
    }

    #[test]
    fn test_newline_in_name_stays_one_line() {
        let rendered = row("a\nb", "v");
        assert!(
            !rendered.contains('\n'),
            "row must be one physical line: {rendered}"
        );
        assert!(rendered.contains("name=\"a\\nb\""));
    }

    #[test]
    fn test_tab_in_value_is_escaped() {
        let rendered = row("tag", "a\tb");
        assert!(
            !rendered.contains('\t'),
            "row must be one physical line: {rendered}"
        );
        assert!(rendered.contains("value=\"a\\tb\""));
    }

    #[test]
    fn test_forged_row_payload_cannot_shadow_real_fields() {
        let rendered = row(
            "tag",
            "x\nsubject=deadbeef name=forged author=deadbeef event=deadbeef",
        );
        assert!(
            !rendered.contains('\n'),
            "row must be one physical line: {rendered}"
        );
        // The real fields are the row's tail; the forged copies sit inside the quoted value.
        assert!(rendered.ends_with(" author=cafe event=beef"));
        assert_eq!(rendered.matches("author=cafe").count(), 1);
        assert!(
            rendered.contains(
                "value=\"x\\nsubject=deadbeef name=forged author=deadbeef event=deadbeef\""
            )
        );
    }

    #[test]
    fn test_format_char_alone_is_quoted_and_escaped() {
        // Cf-only path (#3111): no whitespace and no control character, so the
        // pre-#3111 renderer passed the value through raw and unquoted.
        let rendered = row("tag", "\u{202e}");
        assert_eq!(
            rendered,
            "subject=deadbeef name=tag value=\"\\u202e\" author=cafe event=beef"
        );
        assert!(!rendered.contains('\u{202e}'), "raw Cf must never render");
    }

    #[test]
    fn test_format_char_in_name_is_quoted_and_escaped() {
        let rendered = row("a\u{202e}b", "v");
        assert!(rendered.contains("name=\"a\\u202eb\""));
        assert!(!rendered.contains('\u{202e}'), "raw Cf must never render");
    }

    #[test]
    fn test_every_named_format_char_is_escaped_alone_and_with_a_space() {
        // The full Cf set named in #3111's acceptance: U+202A-202E, U+2066-2069,
        // U+200E/U+200F, U+200B, U+FEFF, U+00AD.
        let named: Vec<char> = (0x202a..=0x202e)
            .chain(0x2066..=0x2069)
            .chain([0x200e, 0x200f, 0x200b, 0xfeff, 0x00ad])
            .map(|cp| char::from_u32(cp).expect("named Cf code point"))
            .collect();
        for c in named {
            assert!(
                crate::is_format_char(c),
                "{c:?} must be in the shared Cf set"
            );
            // Cf-only (unquoted before the fix): quoted, escaped, round-trips.
            let alone = text(&json!(c.to_string()));
            assert_eq!(alone, format!("\"\\u{:04x}\"", c as u32), "Cf-only {c:?}");
            assert!(!alone.contains(c), "raw Cf {c:?} remains in {alone}");
            assert_eq!(
                serde_json::from_str::<String>(&alone).expect("valid JSON literal"),
                c.to_string()
            );
            // Cf-plus-space (quoted before the fix, but serde_json left Cf raw).
            let value = format!("a{c} b");
            let with_space = text(&json!(value));
            assert_eq!(
                with_space,
                format!("\"a\\u{:04x} b\"", c as u32),
                "Cf-plus-space {c:?}"
            );
            assert!(
                !with_space.contains(c),
                "raw Cf {c:?} remains in {with_space}"
            );
            assert_eq!(
                serde_json::from_str::<String>(&with_space).expect("valid JSON literal"),
                value
            );
        }
    }

    #[test]
    fn test_json_output_is_unchanged() {
        assert_eq!(
            object(&[("value", json!("a\nb"))]).to_string(),
            "{\"value\":\"a\\nb\"}"
        );
    }
}
