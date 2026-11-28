//! `id writer-sets`: show, expand, and discover the writer sets stored in
//! the event store (writer-set spec §§4–5; issue #6690).
//!
//! This module parses the arguments, acts on the event store, and prints.
//! The rules are the library's ([`crate::event_store`]): a stored set is read
//! back from its retained body and checked against its publishing header,
//! the projection is [`crate::idset::view::expand_set`] over snapshots the
//! store builds from its resolved group state, and discovery is
//! subject-first — filter by the set's scope, each hit carrying the set and
//! the writers it expands to.
//!
//! # Output
//!
//! `show` prints `show event=<ID> set=<ID> version=<N> owner=<KEY>
//! [scope=<SCOPE>] entries=<N> excludes=<N>`. `expand` prints `expand
//! event=<ID> public=<BOOL> authors=<HEX,HEX,...> retro=<BOOL>`. `discover`
//! prints one row per stored set, `event= set= version= [scope=] public=
//! authors=`, or one JSON array with `--json`.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Result, bail};
use p2panda_core::Hash;
use serde_json::{Value, json};

use crate::cli::WriterSetCommand;
use crate::commands::doc::{print_action, print_rows};
use crate::commands::group::hex;
use crate::event_store::{DiscoveredWriterSet, EventStore};
use crate::groups::log::hash_from_hex;
use crate::idset::WriterSet;
use crate::idset::view::ExpandedWriters;
use crate::local_events::DIR_NAME;
use crate::waiting::WaitingLimits;

/// Opens the event store the command reads, at `path` or the default
/// events directory.
fn open_store(path: Option<PathBuf>) -> Result<EventStore> {
    let dir = path.unwrap_or_else(|| PathBuf::from(DIR_NAME));
    std::fs::create_dir_all(&dir)
        .map_err(|err| anyhow::anyhow!("create {}: {err}", dir.display()))?;
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0);
    Ok(EventStore::open(&dir, WaitingLimits::default(), now_ms)?)
}

/// The resolution snapshot a query pins: the `--resolution` hash when one is
/// given, else the zero hash, which names the store's current resolved
/// state.
fn resolution(pin: Option<String>) -> Result<Hash> {
    match pin {
        Some(text) => hash_from_hex(&text),
        None => Ok(Hash::from([0; 32])),
    }
}

/// The author keys an expansion admits explicitly, as hex, in key order.
fn author_values(expanded: &ExpandedWriters) -> Value {
    json!(
        expanded
            .explicit_authors()
            .iter()
            .map(|author| hex(author))
            .collect::<Vec<_>>()
    )
}

/// The fields of one stored set, as `show` prints them.
fn set_fields(event: Hash, set: &WriterSet) -> Vec<(&'static str, Value)> {
    let mut fields = vec![
        ("event", json!(event.to_hex())),
        ("set", json!(set.set_id.to_hex())),
        ("version", json!(set.version)),
        ("owner", json!(hex(&set.owner))),
    ];
    if let Some(scope) = &set.scope {
        fields.push(("scope", json!(scope)));
    }
    fields.push(("entries", json!(set.entries.len())));
    fields.push(("excludes", json!(set.excludes.0.len())));
    fields
}

/// The fields of one discovery hit, as `discover` rows print them.
fn hit_fields(hit: &DiscoveredWriterSet) -> Vec<(&'static str, Value)> {
    let mut fields = vec![
        ("event", json!(hit.event_id.to_hex())),
        ("set", json!(hit.set.set_id.to_hex())),
        ("version", json!(hit.set.version)),
    ];
    if let Some(scope) = &hit.set.scope {
        fields.push(("scope", json!(scope)));
    }
    fields.push(("public", json!(hit.expanded.is_public())));
    fields.push(("authors", author_values(&hit.expanded)));
    fields
}

/// Runs an `id writer-sets` command.
///
/// # Errors
///
/// Fails if an argument is malformed, the store cannot be opened or read,
/// or the named event is not a stored writer set.
pub async fn cmd_writer_sets(
    path: Option<PathBuf>,
    json: bool,
    command: WriterSetCommand,
) -> Result<()> {
    let store = open_store(path)?;
    match command {
        WriterSetCommand::Show { event } => {
            let event = hash_from_hex(&event)?;
            let Some(set) = store.stored_writer_set(&event)? else {
                bail!("no stored writer set for event {event}");
            };
            print_action(json, "show", set_fields(event, &set));
        }
        WriterSetCommand::Expand {
            event,
            resolution: pin,
            retro,
        } => {
            let event = hash_from_hex(&event)?;
            let expanded = store.expand_stored_writer_set(&event, &resolution(pin)?, retro)?;
            print_action(
                json,
                "expand",
                vec![
                    ("event", json!(event.to_hex())),
                    ("public", json!(expanded.is_public())),
                    ("authors", author_values(&expanded)),
                    ("retro", json!(retro)),
                ],
            );
        }
        WriterSetCommand::Discover {
            scope,
            resolution: pin,
        } => {
            let hits = store.discover_writer_sets(scope.as_deref(), &resolution(pin)?)?;
            let rows: Vec<Vec<(&str, Value)>> = hits.iter().map(hit_fields).collect();
            print_rows(json, &rows);
        }
    }
    Ok(())
}
