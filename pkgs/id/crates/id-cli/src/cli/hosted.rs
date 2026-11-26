//! `id hosted` subcommand definitions: push to and pull from a hosted ID
//! instance over the signed-object bridge (#15486).

use std::path::PathBuf;

use clap::{Args, Subcommand};

/// Which hosted store to talk to, and the key that proves it is yours.
#[derive(Args, Debug, Clone, PartialEq, Eq)]
pub struct HostedTarget {
    /// The hosted instance's front-door origin, for example
    /// `https://id.example`. `http://` is accepted only for a local
    /// development instance on localhost or 127.0.0.1.
    #[arg(long, env = "ID_HOSTED_URL", value_parser = parse_hosted_url)]
    pub url: String,
    /// Your hosted store's binding id: 32 lowercase hex characters.
    #[arg(long, env = "ID_HOSTED_BINDING", value_parser = parse_binding_id)]
    pub binding: String,
    /// The key file holding the ID key your binding names. Defaults to
    /// this node's key file.
    #[arg(long)]
    pub key_file: Option<PathBuf>,
}

/// The actions of `id hosted`.
#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum HostedCommand {
    /// Show the hosted store's chain heads: one row per author.
    Heads {
        /// The hosted store.
        #[command(flatten)]
        target: HostedTarget,
    },
    /// Push every local event the hosted store has not applied, in
    /// bounded batches. Safe to run again: a re-sent event is a
    /// duplicate, never a second copy.
    Push {
        /// The hosted store.
        #[command(flatten)]
        target: HostedTarget,
    },
    /// Pull everything the hosted store holds beyond the local heads
    /// into the local event store.
    Pull {
        /// The hosted store.
        #[command(flatten)]
        target: HostedTarget,
    },
}

impl HostedCommand {
    /// The hosted store this action talks to.
    #[must_use]
    pub const fn target(&self) -> &HostedTarget {
        match self {
            Self::Heads { target } | Self::Push { target } | Self::Pull { target } => target,
        }
    }
}

/// A binding id: exactly 32 lowercase hex characters, as the hosted
/// worker's routing requires.
pub fn parse_binding_id(raw: &str) -> Result<String, String> {
    if raw.len() == 32
        && raw
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        Ok(raw.to_owned())
    } else {
        Err("a binding id is 32 lowercase hex characters".to_owned())
    }
}

/// A front-door origin: `https://`, or `http://` to localhost only, so
/// a proof is never sent in the clear to a remote host.
pub fn parse_hosted_url(raw: &str) -> Result<String, String> {
    let url = raw.trim_end_matches('/');
    if let Some(rest) = url.strip_prefix("https://") {
        if !rest.is_empty() {
            return Ok(url.to_owned());
        }
    } else if let Some(rest) = url.strip_prefix("http://") {
        let local = ["localhost", "127.0.0.1", "[::1]"].iter().any(|host| {
            rest == *host
                || rest.starts_with(&format!("{host}:"))
                || rest.starts_with(&format!("{host}/"))
        });
        if local {
            return Ok(url.to_owned());
        }
        return Err("http:// is only for a local instance; use https://".to_owned());
    }
    Err("the hosted url must start with https://".to_owned())
}
