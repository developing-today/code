//! `id claim`: set, clear, list, and search tag claims on any subject (P6.7c).
//!
//! This module parses the arguments, acts on the event store, and prints. The rules are the
//! library's ([`id_events::local_events`], [`id_store::tag_claims`]).
//!
//! # Output
//!
//! `set` prints `set subject=<ID> name=<NAME> [value=<VALUE>] event=<ID>`, where `event` is the
//! claim's ID. `clear` prints `clear subject=<ID> name=<NAME> claims=<N> event=<ID>`. `list` and
//! `search` print one line per live claim, `subject= name= [value=] author= event=`, with no command
//! word, or one JSON array. The `event` of a row is the claim's ID.

use std::path::PathBuf;

use anyhow::Result;
use p2panda_core::Hash;
use serde_json::{Value, json};

use anyhow::bail;

use crate::cli::{ClaimCommand, ClaimScope};
use crate::commands::doc::{open_events, print_action, print_rows};
use crate::commands::group::hex;
use id_events::local_claims::{scope_prefix_error, split_scope_prefix};
use id_groups::groups::log::hash_from_hex;
use id_store::tag_claims::{Claim, TagScope};

/// Prints a `set` line for a claim just recorded.
pub(super) fn print_set(json: bool, subject: Hash, name: &str, value: Option<&str>, event: Hash) {
    let mut fields = vec![("subject", json!(subject.to_hex())), ("name", json!(name))];
    if let Some(value) = value {
        fields.push(("value", json!(value)));
    }
    fields.push(("event", json!(event.to_hex())));
    print_action(json, "set", fields);
}

/// The fields of one live claim, as the list and search rows show it.
fn claim_fields(claim: &Claim) -> Vec<(&'static str, Value)> {
    let mut fields = vec![
        ("subject", json!(claim.subject.to_hex())),
        ("name", json!(claim.name)),
    ];
    if let Some(value) = &claim.value {
        fields.push(("value", json!(value)));
    }
    fields.push(("author", json!(hex(&claim.author))));
    fields.push(("scope", json!(claim.scope.as_str())));
    fields.push(("event", json!(claim.id.to_hex())));
    fields
}

/// Runs an `id claim` command.
///
/// # Errors
///
/// Fails if an argument is malformed, the store cannot be read or written, or the library refuses
/// the action. A clear with no live claim to clear is refused.
pub async fn cmd_claim(path: Option<PathBuf>, json: bool, command: ClaimCommand) -> Result<()> {
    let mut events = open_events(path).await?;
    match command {
        ClaimCommand::Set {
            subject,
            name,
            value,
            scope,
        } => {
            let subject = hash_from_hex(&subject)?;
            let (scope, name) = resolve_claim_scope(&name, scope)?;
            let claim = events.set_tag_scoped(subject, &name, value.as_deref(), scope)?;
            let mut fields = vec![
                ("subject", json!(subject.to_hex())),
                ("name", json!(name)),
                ("scope", json!(scope.as_str())),
                ("event", json!(claim.to_hex())),
            ];
            if let Some(value) = value {
                fields.push(("value", json!(value)));
            }
            print_action(json, "set", fields);
        }
        ClaimCommand::Clear { subject, name } => {
            let subject = hash_from_hex(&subject)?;
            let (count, event) = events.clear_tag(subject, &name)?;
            if count > 0 {
                eprintln!(
                    "note: clearing stops future replication; it cannot unsend copies peers already hold"
                );
            }
            print_action(
                json,
                "clear",
                vec![
                    ("subject", json!(subject.to_hex())),
                    ("name", json!(name)),
                    ("claims", json!(count)),
                    ("event", json!(event.to_hex())),
                ],
            );
        }
        ClaimCommand::Search { name, value } => {
            let rows: Vec<Vec<(&str, Value)>> = events
                .search(&name, value.as_deref())?
                .iter()
                .flat_map(|hit| hit.claims.iter())
                .map(claim_fields)
                .collect();
            print_rows(json, &rows);
        }
        ClaimCommand::Scope {
            subject,
            name,
            scope,
            yes,
        } => {
            let subject = hash_from_hex(&subject)?;
            let scope = match scope {
                ClaimScope::Shared => TagScope::Shared,
                ClaimScope::Local => TagScope::Local,
            };
            if scope == TagScope::Shared && !yes {
                let would_publish = events
                    .tags_on(subject)?
                    .iter()
                    .any(|claim| claim.name == name && claim.scope == TagScope::Local);
                if would_publish {
                    bail!(
                        "moving '{name}' to shared publishes this node's local claim to peers,                          and publishing cannot be undone; re-run with --yes to confirm"
                    );
                }
            }
            let moved = events.set_tag_scope(subject, &name, scope)?;
            if moved > 0 {
                match scope {
                    TagScope::Local => eprintln!(
                        "note: moving a claim to local stops future replication;                          it cannot unsend copies peers already hold"
                    ),
                    TagScope::Shared => eprintln!(
                        "note: published to peers; this cannot be undone, and a later                          move or clear cannot unsend the copies peers receive"
                    ),
                }
            }
            print_action(
                json,
                "scope",
                vec![
                    ("subject", json!(subject.to_hex())),
                    ("name", json!(name)),
                    ("scope", json!(scope.as_str())),
                    ("moved", json!(moved)),
                ],
            );
        }
        ClaimCommand::Review => {
            let rows: Vec<Vec<(&str, Value)>> =
                events.review_tags()?.iter().map(claim_fields).collect();
            print_rows(json, &rows);
        }
        ClaimCommand::List { subject } => {
            let subject = hash_from_hex(&subject)?;
            let rows: Vec<Vec<(&str, Value)>> =
                events.tags_on(subject)?.iter().map(claim_fields).collect();
            print_rows(json, &rows);
        }
    }
    Ok(())
}

/// Resolves a `claim set` scope and the name to record (#4974, decision D3):
/// an explicit `--scope` wins, and the name is then used exactly as written,
/// prefix included; with no `--scope`, a `local:` name prefix is shorthand
/// for local and the prefix is stripped. No flag and no prefix is the shared
/// default, as before scopes existed.
///
/// # Errors
///
/// Fails on a bare `local:` prefix with no name after it.
fn resolve_claim_scope(name: &str, scope: Option<ClaimScope>) -> Result<(TagScope, String)> {
    match scope {
        Some(ClaimScope::Shared) => Ok((TagScope::Shared, name.to_owned())),
        Some(ClaimScope::Local) => Ok((TagScope::Local, name.to_owned())),
        None => {
            if let Some(error) = scope_prefix_error(name) {
                bail!("{error}");
            }
            let (scope, bare) = split_scope_prefix(name);
            Ok((scope, bare.to_owned()))
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    // Oracle: #4974 design record decision D3 — an explicit `--scope` wins
    // over the `local:` name prefix; the prefix is shorthand only when
    // `--scope` is absent.
    #[test]
    fn explicit_scope_wins_over_the_local_prefix() {
        assert_eq!(
            resolve_claim_scope("local:city", Some(ClaimScope::Shared)).unwrap(),
            (TagScope::Shared, "local:city".to_owned())
        );
        assert_eq!(
            resolve_claim_scope("local:city", Some(ClaimScope::Local)).unwrap(),
            (TagScope::Local, "local:city".to_owned())
        );
    }

    #[test]
    fn the_prefix_is_shorthand_only_when_scope_is_absent() {
        assert_eq!(
            resolve_claim_scope("local:city", None).unwrap(),
            (TagScope::Local, "city".to_owned())
        );
        assert_eq!(
            resolve_claim_scope("city", None).unwrap(),
            (TagScope::Shared, "city".to_owned())
        );
        assert!(resolve_claim_scope("local:", None).is_err());
    }
}
