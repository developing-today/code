//! `id claim`: set, clear, list, and search tag claims on any subject (P6.7c).
//!
//! This module parses the arguments, acts on the event store, and prints. The rules are the
//! library's ([`crate::local_events`], [`crate::tag_claims`]).
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

use crate::cli::{ClaimCommand, ClaimScope};
use crate::commands::doc::{open_events, print_action, print_rows};
use crate::commands::group::hex;
use crate::groups::log::hash_from_hex;
use crate::tag_claims::{Claim, TagScope};

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
            let scope = match scope {
                ClaimScope::Shared => TagScope::Shared,
                ClaimScope::Local => TagScope::Local,
            };
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
        } => {
            let subject = hash_from_hex(&subject)?;
            let scope = match scope {
                ClaimScope::Shared => TagScope::Shared,
                ClaimScope::Local => TagScope::Local,
            };
            let moved = events.set_tag_scope(subject, &name, scope)?;
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
            let rows: Vec<Vec<(&str, Value)>> = events
                .review_tags()?
                .iter()
                .map(claim_fields)
                .collect();
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
