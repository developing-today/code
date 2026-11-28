//! `id world announce` and `id world lookup`: publish a world
//! announcement to pkarr relays and read one back (#10706, addendum
//! doc/2026-11-25T03-10-00Z_design_id10706_announcement_pkarr).
//!
//! `announce` signs with the world key held in `--key-file` (an
//! owner-only file, created on first use like the node key) and
//! publishes through [`id_sync::announcement_relay::RelaySinks`]. With
//! `--watch` it stays up and republishes on the heartbeat the
//! [`id_sync::announcement_pkarr::Publisher`] keeps. `lookup` runs the
//! resolver's pkarr step against the relays and prints what it found.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use ed25519_dalek::SigningKey;
use serde_json::{Value, json};

use id_core::hex::hex_encode;
use id_sync::announcement::{Announcement, Occupancy, SignedAnnouncement, Subject, Visibility};
use id_sync::announcement_pkarr::{Publisher, z32_encode};
use id_sync::announcement_relay::{DEVELOPING_TODAY_RELAY, PublishReport, RelaySinks, SinkOutcome};
use id_sync::bridge_http::ReqwestHttp;

use crate::cli::{WorldAnnounceArgs, WorldLookupArgs};
use crate::load_or_create_keypair;

fn parse_key(text: &str, what: &str) -> Result<[u8; 32]> {
    let text = text.trim();
    if text.len() != 64 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("{what} must be 64 hex characters");
    }
    let mut out = [0_u8; 32];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16)
            .with_context(|| format!("{what} is not hex"))?;
    }
    Ok(out)
}

fn sinks(relays: Vec<String>) -> Result<RelaySinks> {
    if relays.is_empty() {
        RelaySinks::new([DEVELOPING_TODAY_RELAY])
    } else {
        RelaySinks::new(relays)
    }
}

fn now_us() -> Result<u64> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?;
    u64::try_from(elapsed.as_micros()).context("system clock is out of range")
}

fn occupancy(text: &str) -> Result<Occupancy> {
    Ok(match text {
        "has_room" => Occupancy::HasRoom,
        "near_full" => Occupancy::NearFull,
        "full" => Occupancy::Full,
        other => bail!("unknown occupancy {other}"),
    })
}

fn outcome_json(outcome: &SinkOutcome) -> Value {
    match outcome {
        SinkOutcome::Accepted => json!({ "outcome": "accepted" }),
        SinkOutcome::Stale => json!({ "outcome": "stale" }),
        SinkOutcome::Refused { status, message } => {
            json!({ "outcome": "refused", "status": status, "message": message })
        }
        SinkOutcome::Unreachable { error } => json!({ "outcome": "unreachable", "error": error }),
    }
}

/// The JSON line one publish prints.
fn report_json(world_key: &[u8; 32], seq: u64, expires_at: u64, report: &PublishReport) -> Value {
    json!({
        "world_id": hex_encode(world_key),
        "world_z32": z32_encode(world_key),
        "seq": seq,
        "expires_at": expires_at,
        "published": report.succeeded(),
        "contested": report.contested(),
        "relays": report
            .outcomes
            .iter()
            .map(|(base, outcome)| {
                let mut entry = outcome_json(outcome);
                entry["relay"] = json!(base);
                entry
            })
            .collect::<Vec<_>>(),
    })
}

/// The JSON `lookup` prints for a resolved announcement.
fn announcement_json(signed: &SignedAnnouncement) -> Value {
    let a = &signed.announcement;
    let subject = match &a.subject {
        Subject::Endpoint { node_id, relay } => {
            json!({ "endpoint": { "node_id": hex_encode(node_id), "relay": relay } })
        }
        Subject::Program { content_hash } => {
            json!({ "program": { "content_hash": hex_encode(content_hash) } })
        }
    };
    json!({
        "world_id": hex_encode(&a.world_id),
        "name": a.name,
        "description": a.description,
        "subject": subject,
        "occupancy": serde_json::to_value(a.occupancy).unwrap_or(Value::Null),
        "tags": a.tags,
        "visibility": serde_json::to_value(a.visibility).unwrap_or(Value::Null),
        "issued_at": a.issued_at,
        "expires_at": a.expires_at,
        "seq": a.seq,
    })
}

/// `id world announce`.
pub async fn announce(args: WorldAnnounceArgs) -> Result<()> {
    let WorldAnnounceArgs {
        key_file,
        node,
        relay_hint,
        name,
        description,
        tags,
        listed,
        occupancy: occupancy_text,
        relays,
        lifetime,
        watch,
    } = args;
    let secret = load_or_create_keypair(&key_file).await?;
    let key = SigningKey::from_bytes(&secret.to_bytes());
    let world_key = key.verifying_key().to_bytes();
    let fields = Announcement {
        world_id: world_key,
        name,
        description,
        subject: Subject::Endpoint {
            node_id: parse_key(&node, "the node id")?,
            relay: relay_hint,
        },
        occupancy: occupancy(&occupancy_text)?,
        tags,
        visibility: if listed {
            Visibility::Listed
        } else {
            Visibility::Unlisted
        },
        issued_at: 0,
        expires_at: 1,
        seq: 0,
    };
    let sinks = sinks(relays)?;
    let mut publisher = Publisher::new(key, lifetime)?;
    let mut http = ReqwestHttp::new()?;
    loop {
        let item = publisher.publish(&fields, now_us()?)?;
        let report = sinks.publish(&mut http, &world_key, &item).await;
        let expires_at = item.timestamp_us / 1_000_000 + lifetime;
        println!(
            "{}",
            report_json(&world_key, item.timestamp_us, expires_at, &report)
        );
        if !watch {
            if report.succeeded() {
                return Ok(());
            }
            bail!("no pkarr relay accepted the announcement");
        }
        let next = publisher
            .next_heartbeat_at()
            .context("a publish always sets the next heartbeat")?;
        let now = now_us()? / 1_000_000;
        tokio::select! {
            () = tokio::time::sleep(Duration::from_secs(next.saturating_sub(now))) => {}
            _ = tokio::signal::ctrl_c() => return Ok(()),
        }
    }
}

/// `id world lookup`.
pub async fn lookup(args: WorldLookupArgs) -> Result<()> {
    let WorldLookupArgs { world_id, relays } = args;
    let world_key = parse_key(&world_id, "the world id")?;
    let sinks = sinks(relays)?;
    let mut http = ReqwestHttp::new()?;
    let now = now_us()? / 1_000_000;
    let signed = sinks.fetch(&mut http, &world_key, now).await?;
    println!("{}", announcement_json(&signed));
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn keys_parse_only_as_64_hex_characters() {
        assert_eq!(parse_key(&"ab".repeat(32), "k").unwrap(), [0xab; 32]);
        assert!(parse_key("ab", "k").is_err());
        assert!(parse_key(&"zz".repeat(32), "k").is_err());
        assert!(parse_key(&format!("+{}", "a".repeat(63)), "k").is_err());
    }

    #[test]
    fn no_relay_flag_means_our_relay() {
        assert_eq!(sinks(Vec::new()).unwrap().bases(), [DEVELOPING_TODAY_RELAY]);
        assert_eq!(
            sinks(vec!["https://relay.example.net/".to_owned()])
                .unwrap()
                .bases(),
            ["https://relay.example.net"]
        );
    }

    #[test]
    fn occupancy_names_match_the_wire_names() {
        for (text, value) in [
            ("has_room", Occupancy::HasRoom),
            ("near_full", Occupancy::NearFull),
            ("full", Occupancy::Full),
        ] {
            assert_eq!(occupancy(text).unwrap(), value);
            assert_eq!(serde_json::to_value(value).unwrap(), json!(text));
        }
        assert!(occupancy("busy").is_err());
    }

    #[test]
    fn a_report_names_every_relay_and_the_verdict() {
        let report = PublishReport {
            outcomes: vec![
                ("https://a.example".to_owned(), SinkOutcome::Accepted),
                (
                    "https://b.example".to_owned(),
                    SinkOutcome::Unreachable {
                        error: "refused".to_owned(),
                    },
                ),
            ],
        };
        let value = report_json(&[7; 32], 5, 9, &report);
        assert_eq!(value["published"], json!(true));
        assert_eq!(value["contested"], json!(false));
        assert_eq!(value["relays"][0]["relay"], json!("https://a.example"));
        assert_eq!(value["relays"][1]["outcome"], json!("unreachable"));
        assert_eq!(value["world_z32"].as_str().unwrap().len(), 52);
    }
}
