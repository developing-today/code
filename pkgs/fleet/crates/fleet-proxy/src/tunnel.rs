//! The reach layer: an HTTP CONNECT tunnel through the tailnet
//! egress proxy to the laptop, byte-for-byte the construction
//! laptop-proxy-connect.py and laptop-agent.py's `tunnel_socket`
//! have always used.
//!
//! The proxy credential comes from the `HTTPS_PROXY` environment
//! variable at runtime and is sent only inside the CONNECT request
//! itself — never in an argument list, never logged (the flaw in
//! the retired socat-based ProxyCommand, code#948). The proxy's
//! port is NOT the URL's port: the tunnel proxy listens on a fixed
//! port (fleet-config's egress port, default 3130) on the host the
//! URL names — again, exactly as the Python does it.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// Handshake timeout for opening a tunnel (the Python's 60 s).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(60);
/// Read/write timeouts once a tunnel carries HTTP (the Python
/// broker's HTTPConnection timeout is 660 s).
pub const TUNNEL_READ_TIMEOUT: Duration = Duration::from_secs(660);
pub const TUNNEL_WRITE_TIMEOUT: Duration = Duration::from_secs(60);

/// The egress proxy endpoint, parsed from `HTTPS_PROXY`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Egress {
    /// Hostname of the egress proxy (from the URL).
    pub host: String,
    /// The tunnel port on that host (config, not the URL's port).
    pub port: u16,
    /// `Proxy-Authorization` value ("Basic …"). Always present:
    /// the Python sends the header even with empty credentials.
    pub authorization: String,
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            if let Ok(v) = u8::from_str_radix(hex, 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn base64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0] as u32,
            *chunk.get(1).unwrap_or(&0) as u32,
            *chunk.get(2).unwrap_or(&0) as u32,
        ];
        let n = (b[0] << 16) | (b[1] << 8) | b[2];
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// Parse the egress endpoint from the `HTTPS_PROXY` env var. The
/// error strings name shapes only — never the URL (it carries the
/// credential).
pub fn egress_from_env(proxy_port: u16) -> Result<Egress, String> {
    let raw = std::env::var("HTTPS_PROXY")
        .map_err(|_| "HTTPS_PROXY must be set (runtime proxy env)".to_string())?;
    egress_from_url(&raw, proxy_port)
}

/// [`egress_from_env`] with the URL supplied (tests, diagnostics).
pub fn egress_from_url(raw: &str, proxy_port: u16) -> Result<Egress, String> {
    let after_scheme = raw.split_once("://").map(|(_, rest)| rest).unwrap_or(raw);
    let authority = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(after_scheme);
    // Userinfo ends at the LAST '@' (passwords may contain one,
    // percent-encoded or not — the Python splits the same way via
    // urlparse).
    let (userinfo, hostport) = match authority.rfind('@') {
        Some(at) => (Some(&authority[..at]), &authority[at + 1..]),
        None => (None, authority),
    };
    let host = if let Some(rest) = hostport.strip_prefix('[') {
        // [v6 literal]:port — keep the literal, drop the rest.
        rest.split(']').next().unwrap_or(rest)
    } else {
        hostport.split(':').next().unwrap_or(hostport)
    };
    if host.is_empty() {
        return Err("HTTPS_PROXY has no host".to_string());
    }
    let (user, password) = match userinfo {
        Some(ui) => match ui.split_once(':') {
            Some((u, p)) => (percent_decode(u), percent_decode(p)),
            None => (percent_decode(ui), String::new()),
        },
        None => (String::new(), String::new()),
    };
    Ok(Egress {
        host: host.to_string(),
        port: proxy_port,
        authorization: format!(
            "Basic {}",
            base64_encode(format!("{user}:{password}").as_bytes())
        ),
    })
}

/// Open a tunnel to `target_host:target_port`: either a direct TCP
/// connection (a broker running on the target host itself — and the
/// hermetic-test mode), or a CONNECT through the egress proxy.
///
/// The returned stream has the HTTP timeouts applied. On the
/// CONNECT path the response headers are consumed byte-wise: the
/// tunnel may already carry payload bytes behind them, and
/// over-reading into a buffered reader would eat them (the
/// Python's warning, kept).
pub fn connect_tunnel(
    egress: Option<&Egress>,
    target_host: &str,
    target_port: u16,
) -> std::io::Result<TcpStream> {
    connect_tunnel_with_timeout(egress, target_host, target_port, true)
}

/// Open a raw stream tunnel without the HTTP-shaped idle timeouts.
/// The CONNECT handshake still uses [`CONNECT_TIMEOUT`]; only the
/// established byte stream is unbounded.
pub fn connect_raw_tunnel(
    egress: Option<&Egress>,
    target_host: &str,
    target_port: u16,
) -> std::io::Result<TcpStream> {
    connect_tunnel_with_timeout(egress, target_host, target_port, false)
}

fn connect_tunnel_with_timeout(
    egress: Option<&Egress>,
    target_host: &str,
    target_port: u16,
    http_timeouts: bool,
) -> std::io::Result<TcpStream> {
    match egress {
        None => {
            let stream = TcpStream::connect((target_host, target_port))?;
            tune(&stream, http_timeouts);
            Ok(stream)
        }
        Some(egress) => {
            let mut stream = TcpStream::connect((egress.host.as_str(), egress.port))?;
            stream.set_read_timeout(Some(CONNECT_TIMEOUT))?;
            let request = format!(
                "CONNECT {target_host}:{target_port} HTTP/1.1\r\n\
                 Host: {target_host}:{target_port}\r\n\
                 Proxy-Authorization: {auth}\r\n\
                 Proxy-Connection: Keep-Alive\r\n\r\n",
                auth = egress.authorization
            );
            stream.write_all(request.as_bytes())?;
            // Byte-wise until the blank line — see the doc comment.
            let mut buf: Vec<u8> = Vec::with_capacity(256);
            let mut byte = [0u8; 1];
            while !buf.ends_with(b"\r\n\r\n") {
                let n = stream.read(&mut byte)?;
                if n == 0 {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "proxy closed during CONNECT",
                    ));
                }
                buf.push(byte[0]);
                if buf.len() > 65536 {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "proxy response too large",
                    ));
                }
            }
            let status_line = String::from_utf8_lossy(&buf);
            let status_line = status_line.split("\r\n").next().unwrap_or("");
            if !status_line.contains(" 200 ") && !status_line.ends_with(" 200") {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::ConnectionRefused,
                    format!("CONNECT refused: {status_line}"),
                ));
            }
            tune(&stream, http_timeouts);
            Ok(stream)
        }
    }
}

fn tune(stream: &TcpStream, http_timeouts: bool) {
    if http_timeouts {
        stream.set_read_timeout(Some(TUNNEL_READ_TIMEOUT)).ok();
        stream.set_write_timeout(Some(TUNNEL_WRITE_TIMEOUT)).ok();
    } else {
        stream.set_read_timeout(None).ok();
        stream.set_write_timeout(None).ok();
    }
    stream.set_nodelay(true).ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn egress_parses_the_runtime_proxy_shape() {
        let e =
            egress_from_url("http://hatch-runtime:s3cret@hatch-egress-proxy:3128", 3130).unwrap();
        assert_eq!(e.host, "hatch-egress-proxy");
        // The port is the tunnel port, NOT the URL's 3128.
        assert_eq!(e.port, 3130);
        assert_eq!(
            e.authorization,
            format!("Basic {}", base64_encode(b"hatch-runtime:s3cret"))
        );
    }

    #[test]
    fn egress_without_userinfo_still_sends_basic() {
        // Python parity: userinfo absent → user/password are empty
        // strings and the header is still sent.
        let e = egress_from_url("http://proxyhost:3128", 3130).unwrap();
        assert_eq!(e.host, "proxyhost");
        assert_eq!(e.authorization, format!("Basic {}", base64_encode(b":")));
    }

    #[test]
    fn egress_percent_decodes_userinfo() {
        let e = egress_from_url("http://user:p%40ss%3Aw@h:3128", 3130).unwrap();
        assert_eq!(
            e.authorization,
            format!("Basic {}", base64_encode("user:p@ss:w".as_bytes()))
        );
    }

    #[test]
    fn egress_rejects_a_hostless_url() {
        assert!(egress_from_url("http://:3128", 3130).is_err());
        assert!(egress_from_url("", 3130).is_err());
    }

    #[test]
    fn raw_direct_tunnel_has_no_http_timeouts() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let peer = std::thread::spawn(move || listener.accept().unwrap());
        let stream = connect_raw_tunnel(None, "127.0.0.1", addr.port()).unwrap();
        let (_peer, _) = peer.join().unwrap();
        assert_eq!(stream.read_timeout().unwrap(), None);
        assert_eq!(stream.write_timeout().unwrap(), None);
    }

    #[test]
    fn raw_connect_tunnel_clears_timeouts_after_handshake() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let proxy = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut byte = [0u8; 1];
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            stream
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                .unwrap();
        });
        let egress = Egress {
            host: "127.0.0.1".to_string(),
            port: addr.port(),
            authorization: "Basic dTpw".to_string(),
        };
        let stream = connect_raw_tunnel(Some(&egress), "laptop", 21118).unwrap();
        proxy.join().unwrap();
        assert_eq!(stream.read_timeout().unwrap(), None);
        assert_eq!(stream.write_timeout().unwrap(), None);
    }
}
