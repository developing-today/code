//! The four transport legs, in ladder order: the warm broker socket,
//! the Cloudflare tunnel, Tailscale CONNECT, and SSH.
//!
//! Each leg maps its failures onto the ladder's three classes: absent
//! (not configured here), unsent (failed before the request was written)
//! and sent (failed after). See [`crate::ladder`] for why that split
//! decides whether a request may fall through.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use fleet_proxy::http::{HttpConn, HttpError};
use fleet_proxy::tunnel::{connect_cf_tunnel, connect_tunnel, egress_from_env, egress_from_env_url_port, Egress};

use crate::ladder::{Ladder, Leg, LegError, Reply, Request};
use crate::{broker_socket, read_token, token_file, CF_ACCESS_SECRET, TARGET_HOST, TARGET_PORT};

/// The proxy port the Python tailnet CONNECT dials (`PROXY_PORT`).
pub const TAILNET_PROXY_PORT: u16 = 3130;

/// The broker's framed protocol: an 8-byte big-endian length, then the
/// JSON message; the reply is framed the same way.
pub struct BrokerLeg {
    pub socket: PathBuf,
}

impl Leg for BrokerLeg {
    fn name(&self) -> &'static str {
        "broker"
    }

    fn call(&self, req: &Request) -> Result<Reply, LegError> {
        if !self.socket.exists() {
            return Err(LegError::Absent(format!(
                "no broker socket at {}",
                self.socket.display()
            )));
        }
        let mut s = UnixStream::connect(&self.socket)
            .map_err(|e| LegError::Unsent(format!("broker connect: {}", e.kind())))?;
        let msg = serde_json::json!({
            "method": req.method,
            "path": req.path,
            "body": req.body,
            "trip": req.trip,
        });
        let payload = serde_json::to_vec(&msg).unwrap_or_default();
        let mut frame = (payload.len() as u64).to_be_bytes().to_vec();
        frame.extend_from_slice(&payload);
        s.write_all(&frame)
            .map_err(|e| LegError::Sent(format!("broker write: {}", e.kind())))?;
        let mut header = [0u8; 8];
        s.read_exact(&mut header)
            .map_err(|e| LegError::Sent(format!("broker read: {}", e.kind())))?;
        let n = u64::from_be_bytes(header);
        let mut data = vec![0u8; n as usize];
        s.read_exact(&mut data)
            .map_err(|e| LegError::Sent(format!("broker read mid-reply: {}", e.kind())))?;
        let obj: serde_json::Value = serde_json::from_slice(&data)
            .map_err(|e| LegError::Sent(format!("broker reply not JSON: {e}")))?;
        let status = obj
            .get("status")
            .and_then(|v| v.as_u64())
            .and_then(|v| u16::try_from(v).ok())
            .ok_or_else(|| LegError::Sent("broker reply has no status".into()))?;
        let body = serde_json::to_vec(obj.get("body").unwrap_or(&serde_json::Value::Null))
            .unwrap_or_default();
        Ok(Reply { status, body })
    }
}

/// One HTTP exchange over an established connection. Failures after
/// the connection exists are `Sent`: the request may have reached the
/// daemon, so the ladder decides replay by idempotence.
fn exchange(
    conn: Result<HttpConn, std::io::Error>,
    req: &Request,
    token: &str,
    host_header: &str,
    extra: &[(String, String)],
) -> Result<Reply, LegError> {
    let mut conn = conn.map_err(|e| LegError::Unsent(e.to_string()))?;
    let body = req
        .body
        .as_ref()
        .map(|b| serde_json::to_vec(b).unwrap_or_default());
    let mut headers = extra.to_vec();
    headers.push(("X-Trip-Id".to_string(), req.trip.clone()));
    match conn.request_with_extra_headers(
        req.method,
        &req.path,
        host_header,
        token,
        body.as_deref(),
        &headers,
    ) {
        Ok((status, body)) => Ok(Reply { status, body }),
        Err(HttpError::Io(e)) => Err(LegError::Sent(e.to_string())),
        Err(e) => Err(LegError::Sent(e.to_string())),
    }
}

/// The Cloudflare leg. Its Access service token is a fleet secret,
/// resolved when the leg is tried (so a healthy broker never pays for
/// it); the value never leaves this process except in the headers.
///
/// Resolution mirrors the Python client (code#5702 lineage): a success
/// is cached for the life of the process, a failure is cached for
/// 60 s — the resolver chain can take ~2 minutes to fail (broker
/// retries, then the Bitwarden subprocess), and no caller should pay
/// that more than once a minute. The chain's own error is surfaced in
/// the Absent message: SecretError is value-free by design, and "the
/// secret is unavailable" without its reason is undiagnosable (the
/// 2026-11-17 parity pass on #10796 burned its CF budget on exactly
/// that opacity).
pub struct CfLeg {
    pub token: String,
    pub config: fleet_config::Config,
    pub cf_host: String,
    pub cache: Mutex<CfCache>,
}

#[derive(Default)]
pub struct CfCache {
    ready: Option<Vec<(String, String)>>,
    failed_at: Option<Instant>,
}

impl CfLeg {
    fn headers(&self) -> Result<Vec<(String, String)>, LegError> {
        let mut cache = self.cache.lock().unwrap();
        if let Some(headers) = &cache.ready {
            return Ok(headers.clone());
        }
        if let Some(at) = cache.failed_at {
            if at.elapsed() < Duration::from_secs(60) {
                return Err(LegError::Absent(format!(
                    "secret {CF_ACCESS_SECRET} unavailable (failure cached <60s)"
                )));
            }
        }
        let resolver = fleet_secrets::Resolver::default_chain(&self.config);
        let resolved = resolver.resolve(CF_ACCESS_SECRET).map_err(|e| {
            LegError::Absent(format!("secret {CF_ACCESS_SECRET} unavailable: {e}"))
        });
        let secret = match resolved {
            Ok(secret) => secret,
            Err(e) => {
                cache.failed_at = Some(Instant::now());
                return Err(e);
            }
        };
        match cf_access_headers(secret.expose()) {
            Some(headers) => {
                cache.ready = Some(headers.clone());
                cache.failed_at = None;
                Ok(headers)
            }
            None => {
                cache.failed_at = Some(Instant::now());
                Err(LegError::Absent(format!(
                    "secret {CF_ACCESS_SECRET} is not a service token"
                )))
            }
        }
    }
}

impl Leg for CfLeg {
    fn name(&self) -> &'static str {
        "cloudflare"
    }

    fn call(&self, req: &Request) -> Result<Reply, LegError> {
        let egress: Egress = egress_from_env_url_port()
            .map_err(|e| LegError::Absent(e))?;
        let headers = self.headers()?;
        let conn = connect_cf_tunnel(&egress, &self.cf_host)
            .and_then(HttpConn::from_tls);
        exchange(conn, req, &self.token, &self.cf_host, &headers)
    }
}

/// `CF-Access-*` headers from the secret's JSON (`client_id`,
/// `client_secret`). None when the value is not a service token.
pub fn cf_access_headers(json_text: &str) -> Option<Vec<(String, String)>> {
    let value: serde_json::Value = serde_json::from_str(json_text).ok()?;
    let id = value.get("client_id")?.as_str()?;
    let secret = value.get("client_secret")?.as_str()?;
    if id.is_empty() || secret.is_empty() {
        return None;
    }
    Some(vec![
        ("CF-Access-Client-Id".to_string(), id.to_string()),
        ("CF-Access-Client-Secret".to_string(), secret.to_string()),
    ])
}

/// Tailscale: a CONNECT through the tailnet egress proxy to the
/// laptop's daemon port.
pub struct TailscaleLeg {
    pub token: String,
}

impl Leg for TailscaleLeg {
    fn name(&self) -> &'static str {
        "tailscale"
    }

    fn call(&self, req: &Request) -> Result<Reply, LegError> {
        let egress = egress_from_env(TAILNET_PROXY_PORT).map_err(LegError::Absent)?;
        let conn = connect_tunnel(Some(&egress), TARGET_HOST, TARGET_PORT).and_then(HttpConn::new);
        let host = format!("{TARGET_HOST}:{TARGET_PORT}");
        exchange(conn, req, &self.token, &host, &[])
    }
}

/// SSH: a named stub. The real leg is `~/tools/laptop-ssh-real.sh`,
/// which is absent from the sandbox, so its semantics are unverified
/// and nothing is attempted here.
pub struct SshLeg;

impl Leg for SshLeg {
    fn name(&self) -> &'static str {
        "ssh"
    }

    fn call(&self, _req: &Request) -> Result<Reply, LegError> {
        Err(LegError::Absent(
            "ssh leg not implemented (laptop-ssh-real.sh not verified)".into(),
        ))
    }
}

/// The ladder for this process: broker, Cloudflare, Tailscale, SSH.
pub fn ladder(config: &fleet_config::Config) -> Result<Ladder, String> {
    let token = read_token(&token_file())?;
    let legs: Vec<Box<dyn Leg>> = vec![
        Box::new(BrokerLeg {
            socket: broker_socket(),
        }),
        Box::new(CfLeg {
            token: token.clone(),
            config: config.clone(),
            cf_host: config.cf_host().to_string(),
            cache: Mutex::new(CfCache::default()),
        }),
        Box::new(TailscaleLeg {
            token: token.clone(),
        }),
        Box::new(SshLeg),
    ];
    Ok(Ladder::new(legs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cf_headers_come_from_a_service_token_only() {
        let h = cf_access_headers(r#"{"client_id":"id.access","client_secret":"s"}"#).unwrap();
        assert_eq!(h[0], ("CF-Access-Client-Id".to_string(), "id.access".to_string()));
        assert_eq!(h[1].0, "CF-Access-Client-Secret");
        assert!(cf_access_headers(r#"{"client_id":"id"}"#).is_none());
        assert!(cf_access_headers("not json").is_none());
        assert!(cf_access_headers(r#"{"client_id":"","client_secret":"s"}"#).is_none());
    }

    #[test]
    fn a_missing_broker_socket_is_absent_not_sent() {
        let leg = BrokerLeg {
            socket: PathBuf::from("/nonexistent/laptop-agent-test/broker.sock"),
        };
        let req = Request {
            method: "GET",
            path: "/health".into(),
            body: None,
            trip: "t".into(),
            idempotent: true,
            direct_only: false,
        };
        assert!(matches!(leg.call(&req), Err(LegError::Absent(_))));
    }

    #[test]
    fn ssh_leg_is_an_absent_stub() {
        let req = Request {
            method: "GET",
            path: "/health".into(),
            body: None,
            trip: "t".into(),
            idempotent: true,
            direct_only: false,
        };
        assert!(matches!(SshLeg.call(&req), Err(LegError::Absent(_))));
    }
}
