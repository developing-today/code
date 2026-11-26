//! `id hosted`: push to and pull from a hosted ID instance over the
//! signed-object bridge (#15486).
//!
//! The protocol is the library's ([`id_sync::bridge_http`]): every leg
//! carries a fresh ID-key proof, `push` sends each author's events from
//! the hosted next sequence on, and `pull` reads past the local heads
//! until the hosted store reports nothing more. This module loads the
//! key, opens the local event store, and prints.
//!
//! # Output
//!
//! `heads` prints `head author=<HEX> next_seq=<N> head=<ID|->` per
//! author. `push` and `pull` print one line, `<push|pull> events=<N>
//! calls=<N> accepted=<N> duplicate=<N> waiting=<N> forks=<N>
//! refused=<N>`, then `refused <reason>: <message>` per refusal. `--json`
//! prints one object instead. Any refusal makes the command fail after
//! printing, so a script never mistakes a partial transfer for a
//! complete one.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use ed25519_dalek::SigningKey;
use serde_json::json;

use crate::cli::{HostedCommand, HostedTarget};
use crate::commands::writer_set::open_store;
use crate::{KEY_FILE, load_or_create_keypair};
use id_sync::bridge::Head;
use id_sync::bridge_http::{BridgeHttpClient, ReqwestHttp, TransferReport, pull_all, push_all};

/// Loads the ID key the binding names. A missing key file is an error,
/// never a fresh key: a new key could not prove the binding.
async fn load_id_key(path: &Path) -> Result<SigningKey> {
    if !path.exists() {
        bail!(
            "no ID key at {}: your hosted binding names this node's key, so run from the node's \
             directory or pass --key-file",
            path.display()
        );
    }
    let path_text = path
        .to_str()
        .with_context(|| format!("the key file path {} is not UTF-8", path.display()))?;
    let secret = load_or_create_keypair(path_text).await?;
    Ok(SigningKey::from_bytes(&secret.to_bytes()))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn heads_json(heads: &[Head]) -> serde_json::Value {
    json!(
        heads
            .iter()
            .map(|head| json!({ "author": head.author, "next_seq": head.next_seq, "head": head.head }))
            .collect::<Vec<_>>()
    )
}

fn print_heads(heads: &[Head], json: bool) {
    if json {
        println!("{}", heads_json(heads));
        return;
    }
    for head in heads {
        println!(
            "head author={} next_seq={} head={}",
            head.author,
            head.next_seq,
            head.head.as_deref().unwrap_or("-")
        );
    }
}

/// Prints a transfer, then fails if the hosted store or the local store
/// refused anything.
fn finish_transfer(action: &str, report: &TransferReport, json: bool) -> Result<()> {
    if json {
        println!(
            "{}",
            json!({
                "action": action,
                "events": report.events,
                "calls": report.calls,
                "accepted": report.accepted,
                "duplicate": report.duplicate,
                "waiting": report.waiting,
                "forks": report.forks,
                "refused": report.refused,
                "heads": heads_json(&report.heads),
            })
        );
    } else {
        println!(
            "{action} events={} calls={} accepted={} duplicate={} waiting={} forks={} refused={}",
            report.events,
            report.calls,
            report.accepted,
            report.duplicate,
            report.waiting,
            report.forks,
            report.refused.len()
        );
        for refusal in &report.refused {
            println!("refused {refusal}");
        }
    }
    if !report.refused.is_empty() {
        bail!("{action}: {} event(s) were refused", report.refused.len());
    }
    Ok(())
}

/// Runs one `id hosted` action against `event_store` (or the default
/// events directory).
pub async fn cmd_hosted(
    event_store: Option<PathBuf>,
    json: bool,
    command: HostedCommand,
) -> Result<()> {
    let HostedTarget {
        url,
        binding,
        key_file,
    } = command.target().clone();
    let key_path = key_file.unwrap_or_else(|| PathBuf::from(KEY_FILE));
    let key = load_id_key(&key_path).await?;
    let client = BridgeHttpClient::new(binding.clone(), key, url.clone());
    let mut http = ReqwestHttp::new()?;
    match command {
        HostedCommand::Heads { .. } => {
            let heads = client
                .heads(&mut http)
                .await
                .with_context(|| format!("read the heads of binding {binding} at {url}"))?;
            print_heads(&heads, json);
            Ok(())
        }
        HostedCommand::Push { .. } => {
            let store = open_store(event_store)?;
            let report = push_all(&client, &mut http, &store)
                .await
                .with_context(|| format!("push to binding {binding} at {url}"))?;
            finish_transfer("push", &report, json)
        }
        HostedCommand::Pull { .. } => {
            let mut store = open_store(event_store)?;
            let report = pull_all(&client, &mut http, &mut store, now_ms())
                .await
                .with_context(|| format!("pull from binding {binding} at {url}"))?;
            finish_transfer("pull", &report, json)
        }
    }
}
