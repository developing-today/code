//! `id keyset recovery`: guardian recovery through a relay file (#2382).
//!
//! The relay file is a [`RecoveryRelay`]: it keeps every record it is
//! handed and attests when it first saw each one. Until the directory
//! serves the relay over the network, the devices and guardians taking
//! part share the file itself. The flow is the design's: a new device
//! requests, guardians approve, any surviving key may cancel, and after
//! the delay the requesting device completes the rotation.

use std::path::Path;

use anyhow::{Context, Result, bail, ensure};
use ed25519_dalek::SigningKey;

use super::keyset::render_state;
use crate::cli::RecoveryCommand;
use crate::{CLIENT_KEY_FILE, KEY_FILE};
use id_core::hex::hex_encode;
use id_directory::keyset_file::{CLIENT_KEYSET_FILE, KEYSET_FILE, KeysetFile};
use id_directory::recovery::{AnnexEntry, AnnexRecord, notify_targets};
use id_directory::recovery_flow::{self, Progress};
use id_directory::recovery_relay::{RecoveryRelay, RelayedRecord};
use id_directory::recovery_view::{RelayView, Status};
use id_store::store::load_or_create_keypair;

/// Run an `id keyset recovery` subcommand.
///
/// # Errors
///
/// Fails if a key, keyset or the relay file cannot be read or written,
/// or the step is not one this key may take.
pub async fn cmd_recovery(command: RecoveryCommand) -> Result<()> {
    let now = unix_now()?;
    match command {
        RecoveryCommand::Publish { relay, client } => {
            let key = local_key(client).await?;
            let file = local_keyset(client, &key)?;
            let mut relay = RecoveryRelay::open(relay)?;
            let added = publish(&file, &mut relay, now)?;
            println!(
                "Published the keyset of {} ({added} new records).",
                file.state()?.identity_id
            );
        }
        RecoveryCommand::Request {
            identity,
            relay,
            reason,
            client,
        } => {
            let key = local_key(client).await?;
            let identity = identity.to_ascii_lowercase();
            let mut relay = RecoveryRelay::open(relay)?;
            let head = RelayView::new(&relay, &identity)?.head()?;
            ensure!(
                head.guardian_set.is_some(),
                "{identity} names no guardians, so it cannot be recovered"
            );
            let public = public_hex(&key);
            let request = recovery_flow::request(&key, &public, &head, now, &reason)?;
            relay.relay(RelayedRecord::Annex(request.clone()), now)?;
            println!("Requested the recovery of {identity}.");
            println!("Request ID: {}", request.hash);
            println!(
                "Once enough guardians approve and the delay has passed, this device's key {public} becomes its admin and device key."
            );
            println!("Tell every device and guardian below; any surviving device can cancel:");
            for target in notify_targets(&head) {
                println!("  {target}");
            }
        }
        RecoveryCommand::Approve {
            request,
            relay,
            client,
        } => {
            let key = local_key(client).await?;
            let file = local_keyset(client, &key)?;
            let mut relay = RecoveryRelay::open(relay)?;
            publish(&file, &mut relay, now)?;
            let guardian = file.state()?;
            ensure!(
                guardian.is_admin(&public_hex(&key)),
                "this key is not an active admin key of {}, so it cannot approve for it",
                guardian.identity_id
            );
            let request = request_record(&relay, &request)?;
            let identity = request.identity_id().to_owned();
            let head = RelayView::new(&relay, &identity)?.head()?;
            ensure!(
                head.guardian_set
                    .as_ref()
                    .is_some_and(|set| set.guardians.contains(&guardian.identity_id)),
                "{} is not a guardian of {identity}",
                guardian.identity_id
            );
            let approval = recovery_flow::approve(&key, &guardian.identity_id, &request, now)?;
            relay.relay(RelayedRecord::Annex(approval), now)?;
            println!(
                "Approved request {} as guardian {}.",
                request.hash, guardian.identity_id
            );
            print_status(&relay, &identity, &request.hash, now)?;
        }
        RecoveryCommand::Cancel {
            request,
            relay,
            client,
        } => {
            let key = local_key(client).await?;
            let mut relay = RecoveryRelay::open(relay)?;
            let request = request_record(&relay, &request)?;
            let identity = request.identity_id().to_owned();
            let head = RelayView::new(&relay, &identity)?.head()?;
            let public = public_hex(&key);
            ensure!(
                head.is_admin(&public) || head.is_device(&public),
                "this key is not a surviving admin or device key of {identity}"
            );
            let cancellation = recovery_flow::cancel(&key, &request)?;
            relay.relay(RelayedRecord::Annex(cancellation), now)?;
            println!("Cancelled request {}.", request.hash);
        }
        RecoveryCommand::Status { request, relay } => {
            let relay = RecoveryRelay::open(relay)?;
            let request = request_record(&relay, &request)?;
            print_status(&relay, request.identity_id(), &request.hash, now)?;
        }
        RecoveryCommand::Complete {
            request,
            relay,
            client,
        } => {
            let key = local_key(client).await?;
            let mut relay = RecoveryRelay::open(relay)?;
            let request = request_record(&relay, &request)?;
            let identity = request.identity_id().to_owned();
            let rotation = {
                let view = RelayView::new(&relay, &identity)?;
                let Status::Open(Progress::Ready { approval_hashes }) =
                    view.status(&request.hash, now)?
                else {
                    bail!(
                        "the recovery is not ready: {}",
                        render_status(&view.status(&request.hash, now)?, &request.hash, now)
                    );
                };
                recovery_flow::rotate(&key, &request, approval_hashes, view.next_lamport()?)?
            };
            relay.relay(RelayedRecord::Keyset(rotation), now)?;
            let view = RelayView::new(&relay, &identity)?;
            ensure!(
                view.status(&request.hash, now)? == Status::Completed,
                "the relay did not fold the rotation"
            );
            println!("Recovered {identity}.");
            print!("{}", render_state(&view.head()?));
        }
    }
    Ok(())
}

/// Copies every record of `file` into `relay`; returns how many were new.
fn publish(file: &KeysetFile, relay: &mut RecoveryRelay, now: u64) -> Result<usize> {
    let mut added = 0;
    for record in file.records() {
        if relay.relay(RelayedRecord::Keyset(record.clone()), now)?.new {
            added += 1;
        }
    }
    Ok(added)
}

/// The recovery request with this ID in `relay`.
fn request_record(relay: &RecoveryRelay, request: &str) -> Result<AnnexRecord> {
    let request = request.to_ascii_lowercase();
    let record = relay
        .annex_record(&request)
        .with_context(|| format!("the relay holds no record {request}"))?;
    ensure!(
        matches!(record.entry, AnnexEntry::RecoveryRequest { .. }),
        "{request} is not a recovery request"
    );
    Ok(record.clone())
}

fn print_status(relay: &RecoveryRelay, identity: &str, request: &str, now: u64) -> Result<()> {
    let status = RelayView::new(relay, identity)?.status(request, now)?;
    println!("{}", render_status(&status, request, now));
    Ok(())
}

/// One line on where a request stands.
#[must_use]
pub fn render_status(status: &Status, request: &str, now: u64) -> String {
    match status {
        Status::Completed => "Complete: the successor keys now hold the identity.".to_owned(),
        Status::Moot => {
            "Moot: the keyset has moved on from the head this request pins, so it can never complete."
                .to_owned()
        }
        Status::Open(Progress::Cancelled { by }) => format!("Cancelled by {by}."),
        Status::Open(Progress::Waiting {
            approvals,
            threshold,
            ready_at,
        }) => format!(
            "Waiting: {approvals} of {threshold} guardian approvals; the delay ends at {ready_at} (unix seconds, {}).",
            until(*ready_at, now)
        ),
        Status::Open(Progress::Ready { .. }) => format!(
            "Ready: complete it on the requesting device with `id keyset recovery complete {request}`."
        ),
    }
}

/// How long from `now` until `at`, in hours and minutes.
fn until(at: u64, now: u64) -> String {
    let Some(left) = at.checked_sub(now).filter(|left| *left > 0) else {
        return "already passed".to_owned();
    };
    format!("in {}h {}m", left / 3600, left % 3600 / 60)
}

async fn local_key(client: bool) -> Result<SigningKey> {
    let secret = load_or_create_keypair(if client { CLIENT_KEY_FILE } else { KEY_FILE }).await?;
    Ok(SigningKey::from_bytes(&secret.to_bytes()))
}

fn local_keyset(client: bool, key: &SigningKey) -> Result<KeysetFile> {
    let path = if client {
        CLIENT_KEYSET_FILE
    } else {
        KEYSET_FILE
    };
    let (file, migrated) = KeysetFile::open_or_migrate(Path::new(path), key)?;
    if migrated {
        println!("Created the keyset for this identity in {path}.");
    }
    Ok(file)
}

fn public_hex(key: &SigningKey) -> String {
    hex_encode(&key.verifying_key().to_bytes())
}

fn unix_now() -> Result<u64> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .context("the clock is before 1970")?
        .as_secs())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn statuses_read_plainly() {
        let waiting = Status::Open(Progress::Waiting {
            approvals: 1,
            threshold: 2,
            ready_at: 10_000,
        });
        assert_eq!(
            render_status(&waiting, "ab", 10_000 - 3_660),
            "Waiting: 1 of 2 guardian approvals; the delay ends at 10000 (unix seconds, in 1h 1m)."
        );
        assert!(render_status(&waiting, "ab", 10_000).contains("already passed"));
        let ready = Status::Open(Progress::Ready {
            approval_hashes: vec![],
        });
        assert!(render_status(&ready, "ab", 0).ends_with("`id keyset recovery complete ab`."));
        assert!(render_status(&Status::Moot, "ab", 0).starts_with("Moot"));
    }
}
