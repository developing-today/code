//! CLI publication commands (#2147 slice 3b): `id publish`,
//! `id unpublish`, `id publications`.
//!
//! These commands drive the publication manager of the RUNNING
//! `id serve` process, through the meta protocol's operator-only
//! publication requests. Two design rules (2026-11-04 decision on
//! #2147) shape everything here:
//!
//! - **Operator-only.** The connection presents this machine's
//!   client keypair ([`create_local_operator_endpoint`]), which
//!   serve designates as its local operator identity; the handler
//!   refuses every other identity, however open the node's write
//!   policy is. Holding a local key file is the credential.
//! - **Never a local fallback.** If no serve is running, the
//!   command fails with an actionable error. It does NOT edit the
//!   persisted registry itself and does NOT open the local stores:
//!   a publication command that reports success without changing
//!   the running server is exactly the failure the design forbids.

use anyhow::{Result, bail};

use crate::META_ALPN;
use crate::commands::client::create_local_operator_endpoint;
use crate::commands::serve::get_serve_info;
use crate::meta_client::{expect_ok, request_over};
use crate::protocol::{MetaRequest, MetaResponse, WirePlacement, WirePublication};

/// Publish `world` on the running serve: at `--listen ADDR` (a
/// dedicated listener) or `--path PATH` (a mount on the primary
/// Explorer listener). Exactly one placement is required.
pub async fn cmd_publish(
    world: &str,
    listen: Option<&str>,
    path: Option<&str>,
    acknowledge_shadow: bool,
) -> Result<()> {
    let placement = placement_from_args(listen, path)?;
    let described = describe_placement(&placement);
    let publication = WirePublication {
        world: world.to_owned(),
        placement,
        acknowledge_shadow,
    };
    match operator_request(MetaRequest::Publish { publication }).await? {
        MetaResponse::Published { warning } => {
            println!("published {world} at {described}");
            if let Some(warning) = warning {
                eprintln!("warning: {warning}");
            }
            Ok(())
        }
        other => bail!("unexpected response from serve: {other:?}"),
    }
}

/// Remove the publication at `--listen ADDR` or `--path PATH` from
/// the running serve.
pub async fn cmd_unpublish(listen: Option<&str>, path: Option<&str>) -> Result<()> {
    let placement = placement_from_args(listen, path)?;
    let described = describe_placement(&placement);
    match operator_request(MetaRequest::Unpublish { placement }).await? {
        MetaResponse::Unpublished { success: true } => {
            println!("unpublished {described}");
            Ok(())
        }
        other => bail!("unexpected response from serve: {other:?}"),
    }
}

/// List the running serve's live publications.
pub async fn cmd_publications() -> Result<()> {
    match operator_request(MetaRequest::ListPublications).await? {
        MetaResponse::Publications { items } => {
            if items.is_empty() {
                println!("no dynamic publications");
            }
            for item in &items {
                let live = match &item.placement {
                    WirePlacement::Listener { .. } => match item.actual_addr {
                        Some(actual) => format!(" (live at {actual})"),
                        None => " (not live)".to_owned(),
                    },
                    WirePlacement::PathMount { .. } => String::new(),
                };
                println!(
                    "{} at {}{}",
                    item.world,
                    describe_placement(&item.placement),
                    live
                );
            }
            Ok(())
        }
        other => bail!("unexpected response from serve: {other:?}"),
    }
}

fn placement_from_args(listen: Option<&str>, path: Option<&str>) -> Result<WirePlacement> {
    match (listen, path) {
        (Some(addr), None) => Ok(WirePlacement::Listener {
            addr: addr
                .parse()
                .map_err(|error| anyhow::anyhow!("invalid --listen address {addr:?}: {error}"))?,
        }),
        (None, Some(path)) => Ok(WirePlacement::PathMount {
            path: path.to_owned(),
        }),
        _ => bail!("exactly one placement is required: --listen ADDR or --path PATH"),
    }
}

fn describe_placement(placement: &WirePlacement) -> String {
    match placement {
        WirePlacement::Listener { addr } => format!("listener {addr}"),
        WirePlacement::PathMount { path } => format!("path {path}"),
    }
}

/// Send one publication request to the running serve, as the
/// operator. Fails (never falls back) when no serve is running.
async fn operator_request(req: MetaRequest) -> Result<MetaResponse> {
    let Some(info) = get_serve_info().await else {
        bail!(
            "no `id serve` is running for this data directory — publication \
             commands drive the running serve process and never edit local \
             state directly; start one with `id serve --web` first"
        );
    };
    let (endpoint, addr) = create_local_operator_endpoint(&info).await?;
    let conn = id_core::connectivity::connect(&endpoint, addr, META_ALPN).await?;
    let response = request_over(&conn, &req).await;
    conn.close(0u32.into(), b"done");
    endpoint.close().await;
    expect_ok(response?)
}
