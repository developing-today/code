//! SSH client for the frame transport (F13.2): `SshTarget` parsing,
//! host-key trust, and [`connect_frames`], which opens an `exec` channel
//! carrying the same length-prefixed JSON frames as the iroh `/id-world/1`
//! stream. Split out of `world_ssh.rs` to keep that module under the
//! production line limit (#1411, split tracked by #13057).
//!
//! The SSH login grants nothing on this channel: as on iroh, the capability
//! or admin secret inside the frames is the only authority.

use std::sync::Arc;

use russh::keys::ssh_key::HashAlg;

use super::WORLD_EXEC_COMMAND;

/// Where `id world join --via ssh` connects: `[user@]host[:port]` (port 22
/// by default; an IPv6 address goes in brackets). The user is only the SSH
/// login: on the frame channel it names no world and grants nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshTarget {
    /// SSH login name (`id` when absent).
    pub user: String,
    /// Host name or address.
    pub host: String,
    /// TCP port.
    pub port: u16,
}

impl std::str::FromStr for SshTarget {
    type Err = anyhow::Error;

    fn from_str(text: &str) -> anyhow::Result<Self> {
        let (user, rest) = text.rsplit_once('@').unwrap_or(("id", text));
        let port = |digits: &str| {
            anyhow::ensure!(
                digits.bytes().all(|byte| byte.is_ascii_digit()),
                "`{digits}` is not a port"
            );
            let port = digits
                .parse::<u16>()
                .map_err(|_| anyhow::anyhow!("`{digits}` is not a port"))?;
            anyhow::ensure!(port != 0, "`{digits}` is not a port");
            Ok(port)
        };
        let (host, port) = if let Some(inner) = rest.strip_prefix('[') {
            let Some((host, tail)) = inner.split_once(']') else {
                anyhow::bail!("`{text}`: missing `]` after the IPv6 address");
            };
            match tail.strip_prefix(':') {
                Some(digits) => (host, port(digits)?),
                None if tail.is_empty() => (host, 22),
                None => anyhow::bail!("`{text}`: expected `:port` after `]`"),
            }
        } else {
            match rest.split_once(':') {
                Some((host, digits)) if !digits.contains(':') => (host, port(digits)?),
                Some(_) => anyhow::bail!("`{text}`: put an IPv6 address in brackets"),
                None => (rest, 22),
            }
        };
        // #3267: whitespace, control characters, and `/` in the user or
        // host are never legitimate — without this check they parse and
        // fail later inside russh as opaque DNS/auth errors, and a
        // newline-bearing host lands verbatim in error messages and the
        // known-hosts lookup key. The bracketed-IPv6 host is held to the
        // same rule: valid IPv6 literals contain none of these.
        let has_bad_chars = |part: &str| {
            part.chars()
                .any(|c| c.is_whitespace() || c.is_control() || c == '/')
        };
        anyhow::ensure!(
            !has_bad_chars(user) && !has_bad_chars(host),
            "`{text}` is not [user@]host[:port]: user and host must not \
             contain whitespace, control characters, or `/`"
        );
        anyhow::ensure!(
            !host.is_empty() && !user.is_empty(),
            "`{text}` is not [user@]host[:port]"
        );
        Ok(Self {
            user: user.to_owned(),
            host: host.to_owned(),
            port,
        })
    }
}

/// How the SSH client decides to trust the host key. There is no
/// trust-on-first-use: an unknown key is refused with its fingerprint.
#[derive(Clone, Debug)]
pub enum HostKeyCheck {
    /// The SHA256 fingerprint `id serve` prints (`SHA256:...`).
    Fingerprint(String),
    /// An OpenSSH `known_hosts` file.
    KnownHosts(std::path::PathBuf),
}

/// The client half of the host-key decision.
#[derive(Debug)]
pub struct HostKeyTrust {
    host: String,
    port: u16,
    check: HostKeyCheck,
    refused: Arc<std::sync::Mutex<Option<String>>>,
}

impl russh::client::Handler for HostKeyTrust {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, russh::Error> {
        let russh::keys::PublicKeyOrCertificate::PublicKey { key, .. } = key else {
            self.refuse("a certificate (not supported)".to_owned());
            return Ok(false);
        };
        let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
        let trusted = match &self.check {
            HostKeyCheck::Fingerprint(expected) => *expected == fingerprint,
            HostKeyCheck::KnownHosts(path) => {
                russh::keys::check_known_hosts_path(&self.host, self.port, key, path)
                    .unwrap_or(false)
            }
        };
        if !trusted {
            self.refuse(fingerprint);
        }
        Ok(trusted)
    }
}

impl HostKeyTrust {
    fn refuse(&self, fingerprint: String) {
        if let Ok(mut refused) = self.refused.lock() {
            *refused = Some(fingerprint);
        }
    }
}

/// Keeps the SSH connection of a frame session open; dropping it disconnects.
pub struct SshConnection {
    _session: russh::client::Handle<HostKeyTrust>,
}

impl std::fmt::Debug for SshConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SshConnection").finish_non_exhaustive()
    }
}

/// Open a frame session over an SSH exec channel. The returned stream
/// carries exactly the bytes an iroh `/id-world/1` stream would: use
/// [`crate::frames::write_frame_to`] and [`crate::frames::read_frame_from`].
///
/// # Errors
///
/// Fails if the host is unreachable, its key is not trusted, or it does not
/// offer the frame transport.
pub async fn connect_frames(
    target: &SshTarget,
    check: HostKeyCheck,
) -> anyhow::Result<(SshConnection, russh::ChannelStream<russh::client::Msg>)> {
    use anyhow::Context as _;

    let refused = Arc::new(std::sync::Mutex::new(None));
    let trust = HostKeyTrust {
        host: target.host.clone(),
        port: target.port,
        check,
        refused: Arc::clone(&refused),
    };
    let config = Arc::new(russh::client::Config::default());
    let mut session =
        match russh::client::connect(config, (target.host.as_str(), target.port), trust).await {
            Ok(session) => session,
            Err(error) => {
                if let Some(key) = refused.lock().ok().and_then(|mut refused| refused.take()) {
                    anyhow::bail!(
                        "the host key of {}:{} ({key}) is not trusted; compare it with the \
                         `world ssh host key` line `id serve` prints, then pass \
                         --ssh-host-key {key} or add the key to known_hosts",
                        target.host,
                        target.port
                    );
                }
                return Err(error).context("connect over ssh");
            }
        };
    // The login grants nothing here: the frames carry their own capability.
    let auth = session
        .authenticate_password(target.user.as_str(), "")
        .await
        .context("ssh login")?;
    anyhow::ensure!(auth.success(), "the ssh server refused the login");
    let mut channel = session
        .channel_open_session()
        .await
        .context("open ssh channel")?;
    channel
        .exec(true, WORLD_EXEC_COMMAND)
        .await
        .context("start the frame transport")?;
    loop {
        match channel.wait().await {
            Some(russh::ChannelMsg::Success) => break,
            Some(russh::ChannelMsg::Failure) | None => anyhow::bail!(
                "the ssh server does not offer the id world frame transport (exec {WORLD_EXEC_COMMAND})"
            ),
            Some(_) => {}
        }
    }
    Ok((SshConnection { _session: session }, channel.into_stream()))
}
