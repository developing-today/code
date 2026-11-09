//! `id send` — copy a file from one running server to another (#5345).
//!
//! The command only *asks* the source server to perform the transfer: the
//! source server reads its stored file and pushes it to the destination
//! with its own server identity (see [`crate::transfer`]). The local CLI
//! client key is never the sending identity, and no file bytes pass
//! through this process.
//!
//! ```bash
//! # Copy report.txt from server A to server B under the same name
//! id send A_NODE_ID report.txt --to B_NODE_ID
//!
//! # Copy under a different destination name
//! id send A_NODE_ID report.txt --to B_NODE_ID --as summary.txt
//!
//! # Replace an existing destination name (the old file is archived first)
//! id send A_NODE_ID report.txt --to B_NODE_ID --replace
//! ```

use anyhow::{Result, bail};
use iroh_base::EndpointId;

use crate::is_node_id;
use crate::meta_client::{MetaClient, MetaRequester, expect_ok};
use crate::protocol::{MetaRequest, MetaResponse};

/// Trim a name typed at the CLI surface (FileOps stores names verbatim and
/// refuses padding, so a stray space is a typing slip, as in `id put`).
fn trim_surface_name(name: &str) -> &str {
    name.trim()
}

/// Ask the server `source` to copy its file `name` to the server `to`.
///
/// `dest_name` defaults to `name`. With `replace`, an existing different
/// file at the destination name is archived and replaced; without it, a
/// conflict is an error and nothing is sent.
pub async fn cmd_send(
    source: &str,
    name: &str,
    to: &str,
    dest_name: Option<String>,
    replace: bool,
    no_relay: bool,
) -> Result<()> {
    if !is_node_id(source) {
        bail!("invalid source node ID: must be 64 hex characters");
    }
    if !is_node_id(to) {
        bail!("invalid destination node ID: must be 64 hex characters");
    }
    let source_id: EndpointId = source.parse()?;
    let dest_id: EndpointId = to.parse()?;
    let name = trim_surface_name(name);
    let dest_name = dest_name.as_deref().map(trim_surface_name);

    let mut client = MetaClient::connect_node(source_id, no_relay).await?;
    let resp = client
        .request(MetaRequest::Send {
            name: name.to_owned(),
            dest: dest_id,
            dest_name: dest_name.map(ToOwned::to_owned),
            replace,
        })
        .await;
    let resp = match resp {
        Ok(resp) => resp,
        Err(e) => {
            let _ = client.close().await;
            return Err(e);
        }
    };
    let result = match expect_ok(resp)? {
        MetaResponse::Send {
            hash,
            name: registered,
            dest,
            already_present,
            replaced,
        } => {
            if already_present {
                println!(
                    "already present: {name} -> {dest}:{registered} ({hash}); nothing to send"
                );
            } else if replaced {
                println!(
                    "sent: {name} -> {dest}:{registered} ({hash}); previous file archived on the destination"
                );
            } else {
                println!("sent: {name} -> {dest}:{registered} ({hash})");
            }
            Ok(())
        }
        _ => bail!("unexpected response"),
    };
    client.close().await?;
    result
}
