//! `id doc`: publish a file as a document, and list a document's events, frontier and history
//! (P6.7c): parse the arguments, act on the event store, and print.
//!
//! The rules are the library's ([`crate::local_events`]). This module only turns arguments into
//! calls and results into lines or JSON. The store is the stand-in for sync, as the group log is.
//!
//! # Output
//!
//! `publish` prints one line, `publish document=<ID> content=<HASH> size=<N> media_type=<TYPE>`,
//! then one `set` line for each `--tag`. The events, frontier and history print one line per row,
//! with no command word, and `kind=` (or the head's fields) first. With `--json` each action prints
//! one object with a `command` key, and each list prints one array.

use std::path::PathBuf;

use anyhow::{Context, Result};
use p2panda_core::{Hash, SigningKey};
use serde_json::{Value, json};

use crate::cli::DocCommand;
use crate::commands::claim::print_set;
use crate::commands::group::{hex, object, tokens};
use crate::groups::log::hash_from_hex;
use crate::local_events::{DIR_NAME, Entry, LocalEvents};
use crate::{KEY_FILE, load_or_create_keypair};

/// Opens the event store that the command reads and appends to, signed as this node. The store is
/// `--event-store`, or the default directory in the data directory.
///
/// # Errors
///
/// Fails if the node key cannot be read or created, or the store cannot be opened.
pub(super) async fn open_events(path: Option<PathBuf>) -> Result<LocalEvents> {
    let secret = load_or_create_keypair(KEY_FILE).await?;
    let dir = path.unwrap_or_else(|| PathBuf::from(DIR_NAME));
    LocalEvents::open(&dir, SigningKey::from_bytes(&secret.to_bytes()))
}

/// Prints one action: a line `command key=value ...`, or one JSON object with `command` first.
pub(super) fn print_action(json: bool, command: &str, fields: Vec<(&str, Value)>) {
    if json {
        let mut with_command = vec![("command", json!(command))];
        with_command.extend(fields);
        println!("{}", object(&with_command));
    } else {
        println!("{command} {}", tokens(&fields));
    }
}

/// Prints rows: one line each, or one JSON array with `--json`.
pub(super) fn print_rows(json: bool, rows: &[Vec<(&str, Value)>]) {
    if json {
        let array: Vec<Value> = rows.iter().map(|row| object(row)).collect();
        println!("{}", Value::Array(array));
    } else {
        for row in rows {
            println!("{}", tokens(row));
        }
    }
}

/// Splits a `--tag` into its name and optional value: `name=value`, or `name` for a bare tag.
fn parse_tag(text: &str) -> (String, Option<String>) {
    match text.split_once('=') {
        Some((name, value)) => (name.to_owned(), Some(value.to_owned())),
        None => (text.to_owned(), None),
    }
}

/// The fields of one event in a document's history, as the CLI lists it.
fn entry_fields(entry: &Entry) -> Vec<(&'static str, Value)> {
    let mut fields = vec![
        ("kind", json!(entry.kind())),
        ("event", json!(entry.id().to_hex())),
        ("author", json!(hex(&entry.author()))),
    ];
    match entry {
        Entry::Document { document, .. } => fields.extend([
            ("content", json!(document.content.to_hex())),
            ("size", json!(document.size)),
            ("media_type", json!(document.media_type)),
        ]),
        Entry::Set {
            subject,
            name,
            value,
            ..
        } => {
            fields.push(("subject", json!(subject.to_hex())));
            fields.push(("name", json!(name)));
            if let Some(value) = value {
                fields.push(("value", json!(value)));
            }
        }
        Entry::Clear { claims, .. } => {
            let named: Vec<String> = claims.iter().map(Hash::to_hex).collect();
            fields.push(("claims", json!(named.join(","))));
        }
        Entry::Remove { document, .. } => fields.push(("document", json!(document.to_hex()))),
        Entry::NoteCreate { content, .. } => fields.extend([
            ("content", json!(content.content.to_hex())),
            ("size", json!(content.size)),
            ("media_type", json!(content.media_type)),
        ]),
        Entry::NoteRevise {
            note,
            content,
            parents,
            ..
        } => {
            fields.push(("note", json!(note.to_hex())));
            fields.push(("content", json!(content.content.to_hex())));
            let named: Vec<String> = parents.iter().map(Hash::to_hex).collect();
            fields.push(("parents", json!(named.join(","))));
        }
    }
    fields
}

/// Runs an `id doc` command.
///
/// # Errors
///
/// Fails if an argument is malformed, the file cannot be read, the store cannot be read or written,
/// or the library refuses the action. A refused publish records nothing.
pub async fn cmd_doc(path: Option<PathBuf>, json: bool, command: DocCommand) -> Result<()> {
    let mut events = open_events(path).await?;
    match command {
        DocCommand::Publish {
            file,
            media_type,
            tags,
        } => {
            let content =
                std::fs::read(&file).with_context(|| format!("read {}", file.display()))?;
            let tags: Vec<(String, Option<String>)> =
                tags.iter().map(|text| parse_tag(text)).collect();
            let published = events.publish(&media_type, &content, &tags)?;
            let fields = vec![
                ("document", json!(published.id.to_hex())),
                ("content", json!(published.document.content.to_hex())),
                ("size", json!(published.document.size)),
                ("media_type", json!(published.document.media_type)),
            ];
            print_action(json, "publish", fields);
            for (claim, (name, value)) in published.claims.iter().zip(&tags) {
                print_set(json, published.id, name, value.as_deref(), *claim);
            }
        }
        DocCommand::Events { document } => {
            let document = hash_from_hex(&document)?;
            let rows: Vec<Vec<(&str, Value)>> = events
                .events_on(document)?
                .iter()
                .map(entry_fields)
                .collect();
            print_rows(json, &rows);
        }
        DocCommand::History { document } => {
            let document = hash_from_hex(&document)?;
            let rows: Vec<Vec<(&str, Value)>> =
                events.history(document)?.iter().map(entry_fields).collect();
            print_rows(json, &rows);
        }
        DocCommand::Frontier => {
            let rows: Vec<Vec<(&str, Value)>> = events
                .frontier()?
                .iter()
                .map(|head| {
                    vec![
                        ("event", json!(head.event.to_hex())),
                        ("author", json!(hex(&head.author))),
                        ("seq", json!(head.seq)),
                    ]
                })
                .collect();
            print_rows(json, &rows);
        }
    }
    Ok(())
}
