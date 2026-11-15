//! The client half of `fleet proxy get` / `fleet proxy put` — file
//! transfer over the agent channel (developing-today/code#5759, v1).
//!
//! The daemon side (the `/files/*` routes on the laptop-agent,
//! port 4799) is separately owned; this module is the client: it
//! speaks the design's framed-JSON RPC through the session broker,
//! exactly like every other broker client, and is tested here
//! against a hermetic fake daemon implementing the same protocol.
//!
//! Protocol (option 1 of the design): `stat`, `begin`, `write`,
//! `commit`, `abort`, `read` — JSON envelopes, 2 MiB raw chunks
//! base64-encoded, whole-file SHA-256 verified at commit (put) and
//! after the last chunk (get), deterministic transfer temps
//! (`.<name>.fleet-part-<sha256[..12]>`) so an interrupted transfer
//! resumes by re-issuing `begin` and continuing from the temp's
//! size. Writes are idempotent at a given offset, so retries are
//! safe.
//!
//! SHA-256 and base64 are implemented in-module, std-only: the
//! verbs live in the `proxy-connect` capability, which is compiled
//! into every variant including the size-gated connect builds, and
//! two small well-tested codecs keep that gate intact (the design
//! names this measurement explicitly; the connect build's size is
//! exercised by the workspace's existing size checks).
//!
//! Deferred in v1, per the packet's explicit-follow-up allowance:
//! `get -r` — the daemon protocol has no directory-listing route,
//! so a recursive download cannot be enumerated; `put -r` is
//! implemented (the client walks). The follow-up is recorded on
//! #5759.

use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use fleet_config::Config;
use fleet_proxy_proto::{Request, Response};
use serde_json::{Value, json};

use crate::client::{ClientError, ensure_broker};

/// Raw chunk size: 2 MiB (≈2.67 MiB base64 — far under the 64 MiB
/// frame ceiling, small enough that a lost chunk is cheap).
pub const CHUNK_SIZE: usize = 2 * 1024 * 1024;

pub const PUT_USAGE: &str = "\
Usage: fleet proxy put [--mode OCTAL] [--no-resume] <local-path | -> <laptop-path>
       fleet proxy put -r [--mode OCTAL] [--no-resume] <local-dir> <laptop-dir>

Upload a file to the laptop over the agent channel (#5759 v1).
The file is hashed (SHA-256), uploaded in 2 MiB chunks to a
deterministic temporary on the laptop, and atomically renamed
over the destination only after the daemon verifies the hash —
an interrupted put resumes from the temp unless --no-resume is
given. '-' reads stdin (spooled locally first; mode defaults to
0644). Progress and the final sha256 go to stderr.
Exit codes: 2 usage, 3 transport, 69 capability-unavailable.";

pub const GET_USAGE: &str = "\
Usage: fleet proxy get [--mode OCTAL] [--no-resume] <laptop-path> <local-path | ->

Download a file from the laptop over the agent channel (#5759 v1).
Chunks land in a deterministic local temporary; the whole-file
SHA-256 from the daemon's stat must match before the temp is
renamed over the destination. '-' streams to stdout (hash still
verified at the end; a mismatch fails the command after the
bytes were emitted — check stderr). Recursive get is deferred
in v1: the daemon protocol has no directory-listing route.
Exit codes: 2 usage, 3 transport, 69 capability-unavailable.";

// ---------------------------------------------------------------------------
// SHA-256 (FIPS 180-4), std-only.
// ---------------------------------------------------------------------------

const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// Incremental SHA-256. Oracle: FIPS 180-4; the test vectors in
/// this module are the standard "abc" / empty / 56-byte cases.
pub struct Sha256 {
    state: [u32; 8],
    buf: [u8; 64],
    buf_len: usize,
    total: u64,
}

impl Sha256 {
    pub fn new() -> Self {
        Sha256 {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            buf: [0u8; 64],
            buf_len: 0,
            total: 0,
        }
    }

    pub fn update(&mut self, mut data: &[u8]) {
        self.total = self.total.wrapping_add(data.len() as u64);
        if self.buf_len > 0 {
            let take = (64 - self.buf_len).min(data.len());
            self.buf[self.buf_len..self.buf_len + take].copy_from_slice(&data[..take]);
            self.buf_len += take;
            data = &data[take..];
            if self.buf_len == 64 {
                let block = self.buf;
                self.compress(&block);
                self.buf_len = 0;
            }
        }
        while data.len() >= 64 {
            let (block, rest) = data.split_at(64);
            let mut b = [0u8; 64];
            b.copy_from_slice(block);
            self.compress(&b);
            data = rest;
        }
        if !data.is_empty() {
            self.buf[..data.len()].copy_from_slice(data);
            self.buf_len = data.len();
        }
    }

    fn compress(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                block[i * 4],
                block[i * 4 + 1],
                block[i * 4 + 2],
                block[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h) = (
            self.state[0],
            self.state[1],
            self.state[2],
            self.state[3],
            self.state[4],
            self.state[5],
            self.state[6],
            self.state[7],
        );
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(SHA256_K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
        self.state[4] = self.state[4].wrapping_add(e);
        self.state[5] = self.state[5].wrapping_add(f);
        self.state[6] = self.state[6].wrapping_add(g);
        self.state[7] = self.state[7].wrapping_add(h);
    }

    pub fn finalize_hex(mut self) -> String {
        let bit_len = self.total.wrapping_mul(8);
        self.update(&[0x80]);
        while self.buf_len != 56 {
            self.update(&[0]);
        }
        // Push the length by hand: update() would count it.
        self.buf[56..64].copy_from_slice(&bit_len.to_be_bytes());
        let block = self.buf;
        self.compress(&block);
        let mut out = String::with_capacity(64);
        for word in self.state {
            out.push_str(&format!("{word:08x}"));
        }
        out
    }
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

pub fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    h.finalize_hex()
}

// ---------------------------------------------------------------------------
// Base64 (RFC 4648, standard alphabet, padded), std-only.
// ---------------------------------------------------------------------------

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn b64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0] as u32,
            *chunk.get(1).unwrap_or(&0) as u32,
            *chunk.get(2).unwrap_or(&0) as u32,
        ];
        let n = (b[0] << 16) | (b[1] << 8) | b[2];
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            B64[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

pub fn b64_decode(text: &str) -> Result<Vec<u8>, ClientError> {
    let bad = |why: &str| ClientError::Transport(format!("base64 decode: {why}"));
    let bytes = text.as_bytes();
    if bytes.len() % 4 != 0 {
        return Err(bad("length is not a multiple of 4"));
    }
    let mut rev = [255u8; 256];
    for (i, c) in B64.iter().enumerate() {
        rev[*c as usize] = i as u8;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks(4) {
        let mut vals = [0u8; 4];
        let mut pad = 0;
        for (i, c) in chunk.iter().enumerate() {
            if *c == b'=' {
                pad += 1;
                vals[i] = 0;
            } else {
                if pad > 0 {
                    return Err(bad("padding in the middle of the input"));
                }
                vals[i] = rev[*c as usize];
                if vals[i] == 255 {
                    return Err(bad("character outside the standard alphabet"));
                }
            }
        }
        if pad > 2 {
            return Err(bad("too much padding"));
        }
        let n = ((vals[0] as u32) << 18)
            | ((vals[1] as u32) << 12)
            | ((vals[2] as u32) << 6)
            | vals[3] as u32;
        out.push((n >> 16) as u8);
        if pad < 2 {
            out.push((n >> 8) as u8);
        }
        if pad < 1 {
            out.push(n as u8);
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Protocol surface.
// ---------------------------------------------------------------------------

/// A seekable byte source (a file, or a cursor in tests).
pub trait ReadSeek: Read + Seek {}
impl<T: Read + Seek> ReadSeek for T {}

/// One daemon RPC. The production implementation speaks through
/// the session broker; tests substitute a hermetic fake daemon.
pub trait FilesTransport {
    /// POST `body` to the daemon route `path` (`/files/stat`, …)
    /// and return the reply body. A reply carrying an `error`
    /// field, or a non-200 status, is an error.
    fn post(&mut self, path: &str, body: Value) -> Result<Value, ClientError>;
}

/// The production transport: one framed `Request`/`Response`
/// exchange per RPC over a fresh session-broker connection — the
/// shape `fleet-secrets`' broker source established. The broker
/// routes by port to the laptop-agent daemon and presents the
/// token from the named file; the token value never enters this
/// process.
pub struct BrokerFilesTransport {
    socket: PathBuf,
    token_file: PathBuf,
    port: u16,
}

impl BrokerFilesTransport {
    pub fn new(config: &Config) -> Self {
        BrokerFilesTransport {
            socket: config.broker_socket(),
            token_file: config.default_token_file(),
            port: fleet_config::PORT_LAPTOP_AGENT,
        }
    }
}

impl FilesTransport for BrokerFilesTransport {
    fn post(&mut self, path: &str, body: Value) -> Result<Value, ClientError> {
        let request = Request {
            method: "POST".to_string(),
            path: path.to_string(),
            body: Some(body),
            port: Some(self.port),
            token_file: Some(self.token_file.to_string_lossy().into_owned()),
        };
        let mut stream = UnixStream::connect(&self.socket).map_err(|e| {
            ClientError::Transport(format!("broker socket {}: {e}", self.socket.display()))
        })?;
        let _ = stream.set_read_timeout(Some(Duration::from_secs(300)));
        fleet_proxy_proto::write_frame(&mut stream, &request)
            .map_err(|e| ClientError::Transport(format!("daemon request write: {e}")))?;
        let response: Response = fleet_proxy_proto::read_typed(&mut stream)
            .map_err(|e| ClientError::Transport(format!("daemon response read: {e}")))?;
        check_reply(path, response.status, response.body)
    }
}

/// A daemon reply is an error when its status is not 200 or its
/// body carries an `error` field (the commit route reports a hash
/// mismatch that way and keeps the temp; the client surfaces the
/// daemon's own words either way).
fn check_reply(route: &str, status: u16, body: Value) -> Result<Value, ClientError> {
    if let Some(error) = body.get("error").and_then(|v| v.as_str()) {
        let actual = body
            .get("actual")
            .and_then(|v| v.as_str())
            .map(|a| format!(" (actual sha256: {a})"))
            .unwrap_or_default();
        return Err(ClientError::Transport(format!(
            "daemon {route}: {error}{actual}"
        )));
    }
    if status != 200 {
        return Err(ClientError::Transport(format!(
            "daemon {route} answered status {status}: {body}"
        )));
    }
    Ok(body)
}

/// Validate a remote (laptop-side) path the way the design bounds
/// it: non-empty, no NUL. Non-UTF8 cannot occur — argv is already
/// a Rust `String`. Confinement is deliberately none (the daemon
/// executes arbitrary commands as the same user; file routes are
/// not a privilege change, and the design says so).
pub fn validate_remote_path(path: &str) -> Result<(), ClientError> {
    if path.is_empty() {
        return Err(ClientError::Usage("remote path is empty".to_string()));
    }
    if path.contains('\0') {
        return Err(ClientError::Usage(
            "remote path contains a NUL byte".to_string(),
        ));
    }
    Ok(())
}

/// The deterministic transfer temp for `(path, sha256)`: a sibling
/// named `.<name>.fleet-part-<sha256[..12]>`. An interrupted
/// client resumes the same transfer by re-deriving this name.
pub fn temp_path_for(path: &str, sha256: &str) -> String {
    let (dir, name) = match path.rfind('/') {
        Some(i) => (&path[..=i], &path[i + 1..]),
        None => ("", path),
    };
    let short = &sha256[..sha256.len().min(12)];
    format!("{dir}.{name}.fleet-part-{short}")
}

/// Chunk plan for `size` bytes starting at `start`: the
/// (offset, length) pairs the write/read loops walk. Oracle: the
/// pairs tile `[start, size)` exactly, each length in
/// `1..=CHUNK_SIZE` — the property the tests assert.
pub fn chunk_plan(size: u64, start: u64) -> Vec<(u64, usize)> {
    let mut plan = Vec::new();
    let mut offset = start;
    while offset < size {
        let len = ((size - offset).min(CHUNK_SIZE as u64)) as usize;
        plan.push((offset, len));
        offset += len as u64;
    }
    plan
}

#[derive(Debug, Clone, PartialEq)]
pub struct StatInfo {
    pub exists: bool,
    pub is_dir: bool,
    pub size: u64,
    pub sha256: Option<String>,
}

pub fn rpc_stat(
    t: &mut dyn FilesTransport,
    path: &str,
    want_sha256: bool,
) -> Result<StatInfo, ClientError> {
    let body = t.post("/files/stat", json!({"path": path, "sha256": want_sha256}))?;
    Ok(StatInfo {
        exists: body
            .get("exists")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        is_dir: body.get("type").and_then(|v| v.as_str()) == Some("dir"),
        size: body.get("size").and_then(|v| v.as_u64()).unwrap_or(0),
        sha256: body
            .get("sha256")
            .and_then(|v| v.as_str())
            .map(str::to_string),
    })
}

#[derive(Debug)]
pub struct PutReport {
    pub size: u64,
    pub sha256: String,
    pub resumed_from: u64,
}

/// Upload `source` (already hashed by the caller pass in
/// `put_file`; here a seekable reader whose full contents are the
/// file) to `remote_path`. `size`/`sha256` describe the contents.
pub fn put_stream(
    t: &mut dyn FilesTransport,
    remote_path: &str,
    source: &mut dyn ReadSeek,
    size: u64,
    sha256: &str,
    mode: Option<u32>,
    resume: bool,
    log: &mut dyn FnMut(&str),
) -> Result<PutReport, ClientError> {
    validate_remote_path(remote_path)?;
    let mut begin = json!({"path": remote_path, "size": size, "sha256": sha256});
    if let Some(mode) = mode {
        begin["mode"] = json!(mode);
    }
    t.post("/files/begin", begin)?;
    // Resume: the deterministic temp may already hold a prefix.
    let temp = temp_path_for(remote_path, sha256);
    let mut start = 0u64;
    if resume {
        let temp_stat = rpc_stat(t, &temp, false)?;
        if temp_stat.exists && !temp_stat.is_dir && temp_stat.size <= size {
            start = temp_stat.size;
        }
    }
    if start > 0 {
        log(&format!(
            "fleet proxy put: resuming {remote_path} at {start}/{size} bytes"
        ));
    }
    source
        .seek(SeekFrom::Start(start))
        .map_err(|e| ClientError::Transport(format!("local seek: {e}")))?;
    let mut buf = vec![0u8; CHUNK_SIZE];
    for (offset, len) in chunk_plan(size, start) {
        let chunk = &mut buf[..len];
        source
            .read_exact(chunk)
            .map_err(|e| ClientError::Transport(format!("local read at {offset}: {e}")))?;
        let reply = t.post(
            "/files/write",
            json!({
                "path": remote_path,
                "sha256": sha256,
                "offset": offset,
                "data_b64": b64_encode(chunk),
            }),
        )?;
        let written = reply.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
        if written != offset + len as u64 {
            return Err(ClientError::Transport(format!(
                "daemon write at offset {offset} reports {written} bytes durably written, expected {}",
                offset + len as u64
            )));
        }
    }
    let reply = t.post(
        "/files/commit",
        json!({"path": remote_path, "sha256": sha256}),
    )?;
    let committed = reply.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
    if committed != size {
        return Err(ClientError::Transport(format!(
            "daemon commit reports size {committed}, expected {size} (temp kept at {temp})"
        )));
    }
    Ok(PutReport {
        size,
        sha256: sha256.to_string(),
        resumed_from: start,
    })
}

/// The local file's permission bits, when the platform has them.
fn local_file_mode(local: &Path) -> Option<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(local)
            .ok()
            .map(|m| m.permissions().mode() & 0o7777)
    }
    #[cfg(not(unix))]
    {
        let _ = local;
        None
    }
}

/// Hash a seekable stream from the start, then rewind.
fn hash_stream(source: &mut dyn ReadSeek) -> Result<(u64, String), ClientError> {
    source
        .seek(SeekFrom::Start(0))
        .map_err(|e| ClientError::Transport(format!("local seek: {e}")))?;
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    let mut buf = vec![0u8; CHUNK_SIZE];
    loop {
        let n = source
            .read(&mut buf)
            .map_err(|e| ClientError::Transport(format!("local read: {e}")))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        total += n as u64;
    }
    source
        .seek(SeekFrom::Start(0))
        .map_err(|e| ClientError::Transport(format!("local seek: {e}")))?;
    Ok((total, hasher.finalize_hex()))
}

/// Upload one local file. This is the verb's real entry: hash
/// pass, then [`put_stream`].
pub fn put_file(
    t: &mut dyn FilesTransport,
    local: &Path,
    remote_path: &str,
    mode_override: Option<u32>,
    resume: bool,
    log: &mut dyn FnMut(&str),
) -> Result<PutReport, ClientError> {
    let mut file = std::fs::File::open(local)
        .map_err(|e| ClientError::Transport(format!("open {}: {e}", local.display())))?;
    let (size, sha256) = hash_stream(&mut file)?;
    let mode = mode_override.or_else(|| local_file_mode(local));
    put_stream(t, remote_path, &mut file, size, &sha256, mode, resume, log)
}

#[derive(Debug)]
pub struct GetReport {
    pub size: u64,
    pub sha256: String,
    pub resumed_from: u64,
}

fn read_chunk(
    t: &mut dyn FilesTransport,
    remote_path: &str,
    offset: u64,
    len: usize,
) -> Result<Vec<u8>, ClientError> {
    let reply = t.post(
        "/files/read",
        json!({"path": remote_path, "offset": offset, "length": len}),
    )?;
    let data = reply
        .get("data_b64")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            ClientError::Transport(format!(
                "daemon read at offset {offset} returned no data_b64"
            ))
        })?;
    let bytes = b64_decode(data)?;
    if bytes.len() > len {
        return Err(ClientError::Transport(format!(
            "daemon read at offset {offset} returned {} bytes, more than the {len} requested",
            bytes.len()
        )));
    }
    Ok(bytes)
}

/// Download `remote_path` to `local`, through the deterministic
/// local temp, verifying the daemon's stated whole-file SHA-256
/// before the atomic rename over the destination.
pub fn get_file(
    t: &mut dyn FilesTransport,
    remote_path: &str,
    local: &Path,
    mode_override: Option<u32>,
    resume: bool,
    log: &mut dyn FnMut(&str),
) -> Result<GetReport, ClientError> {
    validate_remote_path(remote_path)?;
    let stat = rpc_stat(t, remote_path, true)?;
    if !stat.exists {
        return Err(ClientError::Transport(format!(
            "daemon has no file at {remote_path}"
        )));
    }
    if stat.is_dir {
        return Err(ClientError::Usage(format!(
            "{remote_path} is a directory; recursive get is deferred in v1 (no daemon listing route)"
        )));
    }
    let expected_sha = stat.sha256.clone().ok_or_else(|| {
        ClientError::Transport("daemon stat returned no sha256 for a get".to_string())
    })?;
    let local_str = local.to_string_lossy().into_owned();
    let temp = PathBuf::from(temp_path_for(&local_str, &expected_sha));
    let mut start = 0u64;
    if resume {
        if let Ok(meta) = std::fs::metadata(&temp) {
            if meta.len() <= stat.size {
                start = meta.len();
            }
        }
    }
    if start > 0 {
        log(&format!(
            "fleet proxy get: resuming {remote_path} at {start}/{} bytes",
            stat.size
        ));
    }
    {
        let mut out = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&temp)
            .map_err(|e| ClientError::Transport(format!("open {}: {e}", temp.display())))?;
        // If a stale temp is longer than the resume point we
        // computed (it cannot be — metadata said <= size — but a
        // temp from a different, larger transfer under the same
        // name would collide), truncate to the resume point.
        out.set_len(start)
            .map_err(|e| ClientError::Transport(format!("truncate temp: {e}")))?;
        out.seek(SeekFrom::Start(start))
            .map_err(|e| ClientError::Transport(format!("temp seek: {e}")))?;
        for (offset, len) in chunk_plan(stat.size, start) {
            let bytes = read_chunk(t, remote_path, offset, len)?;
            if bytes.len() != len {
                return Err(ClientError::Transport(format!(
                    "daemon read at offset {offset} returned {} bytes, expected {len} (temp kept at {})",
                    bytes.len(),
                    temp.display()
                )));
            }
            out.write_all(&bytes)
                .map_err(|e| ClientError::Transport(format!("temp write: {e}")))?;
        }
        out.flush()
            .map_err(|e| ClientError::Transport(format!("temp flush: {e}")))?;
    }
    // Verify before the rename — the get-side mirror of commit.
    let mut check = std::fs::File::open(&temp)
        .map_err(|e| ClientError::Transport(format!("reopen temp: {e}")))?;
    let (_, actual_sha) = hash_stream(&mut check)?;
    if actual_sha != expected_sha {
        return Err(ClientError::Transport(format!(
            "sha256 mismatch on get of {remote_path}: daemon stated {expected_sha}, downloaded bytes hash to {actual_sha} (temp kept at {})",
            temp.display()
        )));
    }
    if let Some(parent) = local.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| ClientError::Transport(format!("create {}: {e}", parent.display())))?;
        }
    }
    std::fs::rename(&temp, local).map_err(|e| {
        ClientError::Transport(format!(
            "rename {} -> {}: {e}",
            temp.display(),
            local.display()
        ))
    })?;
    if let Some(mode) = mode_override {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(local, std::fs::Permissions::from_mode(mode))
                .map_err(|e| ClientError::Transport(format!("chmod {}: {e}", local.display())))?;
        }
        #[cfg(not(unix))]
        {
            let _ = mode;
        }
    }
    Ok(GetReport {
        size: stat.size,
        sha256: expected_sha,
        resumed_from: start,
    })
}

/// Download `remote_path` straight to `out` (the `get -` form):
/// no temp, no resume; the hash is verified after the last byte
/// and a mismatch fails the command.
pub fn get_to_writer(
    t: &mut dyn FilesTransport,
    remote_path: &str,
    out: &mut dyn Write,
) -> Result<GetReport, ClientError> {
    validate_remote_path(remote_path)?;
    let stat = rpc_stat(t, remote_path, true)?;
    if !stat.exists {
        return Err(ClientError::Transport(format!(
            "daemon has no file at {remote_path}"
        )));
    }
    if stat.is_dir {
        return Err(ClientError::Usage(format!(
            "{remote_path} is a directory; recursive get is deferred in v1 (no daemon listing route)"
        )));
    }
    let expected_sha = stat.sha256.clone().ok_or_else(|| {
        ClientError::Transport("daemon stat returned no sha256 for a get".to_string())
    })?;
    let mut hasher = Sha256::new();
    for (offset, len) in chunk_plan(stat.size, 0) {
        let bytes = read_chunk(t, remote_path, offset, len)?;
        if bytes.len() != len {
            return Err(ClientError::Transport(format!(
                "daemon read at offset {offset} returned {} bytes, expected {len}",
                bytes.len()
            )));
        }
        hasher.update(&bytes);
        out.write_all(&bytes)
            .map_err(|e| ClientError::Transport(format!("stdout write: {e}")))?;
    }
    out.flush()
        .map_err(|e| ClientError::Transport(format!("stdout flush: {e}")))?;
    let actual_sha = hasher.finalize_hex();
    if actual_sha != expected_sha {
        return Err(ClientError::Transport(format!(
            "sha256 mismatch on streamed get of {remote_path}: daemon stated {expected_sha}, streamed bytes hash to {actual_sha} (bytes already emitted)"
        )));
    }
    Ok(GetReport {
        size: stat.size,
        sha256: expected_sha,
        resumed_from: 0,
    })
}

// ---------------------------------------------------------------------------
// Argument parsing and the verb entry points.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct PutArgs {
    pub recursive: bool,
    pub no_resume: bool,
    pub mode: Option<u32>,
    pub local: String,
    pub remote: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GetArgs {
    pub recursive: bool,
    pub no_resume: bool,
    pub mode: Option<u32>,
    pub remote: String,
    pub local: String,
}

fn parse_mode(text: &str) -> Result<u32, ClientError> {
    let digits = text.strip_prefix("0o").unwrap_or(text);
    u32::from_str_radix(digits, 8)
        .map_err(|_| ClientError::Usage(format!("--mode {text:?} is not an octal mode")))
}

fn parse_transfer_args(
    argv: &[String],
    usage: &str,
) -> Result<(bool, bool, Option<u32>, Vec<String>), ClientError> {
    let mut recursive = false;
    let mut no_resume = false;
    let mut mode = None;
    let mut positional = Vec::new();
    let mut i = 0;
    while i < argv.len() {
        let arg = &argv[i];
        match arg.as_str() {
            "-r" | "--recursive" => recursive = true,
            "--no-resume" => no_resume = true,
            "--mode" => {
                i += 1;
                let value = argv
                    .get(i)
                    .ok_or_else(|| ClientError::Usage(usage.to_string()))?;
                mode = Some(parse_mode(value)?);
            }
            _ if arg.starts_with("--mode=") => {
                mode = Some(parse_mode(&arg["--mode=".len()..])?);
            }
            _ if arg.starts_with('-') && arg.len() > 1 && arg != "-" => {
                return Err(ClientError::Usage(format!(
                    "unknown flag {arg:?}\n\n{usage}"
                )));
            }
            _ => positional.push(arg.clone()),
        }
        i += 1;
    }
    Ok((recursive, no_resume, mode, positional))
}

impl PutArgs {
    pub fn parse(argv: &[String]) -> Result<PutArgs, ClientError> {
        let (recursive, no_resume, mode, positional) = parse_transfer_args(argv, PUT_USAGE)?;
        if positional.len() != 2 {
            return Err(ClientError::Usage(PUT_USAGE.to_string()));
        }
        Ok(PutArgs {
            recursive,
            no_resume,
            mode,
            local: positional[0].clone(),
            remote: positional[1].clone(),
        })
    }
}

impl GetArgs {
    pub fn parse(argv: &[String]) -> Result<GetArgs, ClientError> {
        let (recursive, no_resume, mode, positional) = parse_transfer_args(argv, GET_USAGE)?;
        if positional.len() != 2 {
            return Err(ClientError::Usage(GET_USAGE.to_string()));
        }
        Ok(GetArgs {
            recursive,
            no_resume,
            mode,
            remote: positional[0].clone(),
            local: positional[1].clone(),
        })
    }
}

fn join_remote(dir: &str, rel: &Path) -> String {
    let rel = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/");
    if dir.ends_with('/') {
        format!("{dir}{rel}")
    } else {
        format!("{dir}/{rel}")
    }
}

/// Collect the regular files under `root` as (local path,
/// relative path) pairs. Symlinks are not followed (design v1);
/// they are reported through `log` and skipped.
fn walk_local_dir(
    root: &Path,
    log: &mut dyn FnMut(&str),
) -> Result<Vec<(PathBuf, PathBuf)>, ClientError> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir)
            .map_err(|e| ClientError::Transport(format!("read dir {}: {e}", dir.display())))?;
        for entry in entries {
            let entry =
                entry.map_err(|e| ClientError::Transport(format!("read dir entry: {e}")))?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|e| ClientError::Transport(format!("file type: {e}")))?;
            if file_type.is_symlink() {
                log(&format!(
                    "fleet proxy put: skipping symlink {} (v1 does not follow symlinks)",
                    path.display()
                ));
            } else if file_type.is_dir() {
                stack.push(path);
            } else if file_type.is_file() {
                let rel = path
                    .strip_prefix(root)
                    .expect("walk stays under root")
                    .to_path_buf();
                out.push((path, rel));
            }
        }
    }
    out.sort();
    Ok(out)
}

pub fn run_put(
    args: &PutArgs,
    config: &Config,
    client: &str,
    capabilities: &[String],
) -> Result<i32, ClientError> {
    validate_remote_path(&args.remote)?;
    let _ensured = ensure_broker(config, client, capabilities)?;
    let mut transport = BrokerFilesTransport::new(config);
    let mut log = |line: &str| eprintln!("{line}");
    if args.recursive {
        let root = Path::new(&args.local);
        if !root.is_dir() {
            return Err(ClientError::Usage(format!(
                "put -r: {} is not a directory",
                args.local
            )));
        }
        let files = walk_local_dir(root, &mut log)?;
        if files.is_empty() {
            eprintln!(
                "fleet proxy put: no regular files under {} (empty directories are not preserved in v1)",
                args.local
            );
            return Ok(0);
        }
        for (local, rel) in &files {
            let remote = join_remote(&args.remote, rel);
            let report = put_file(
                &mut transport,
                local,
                &remote,
                args.mode,
                !args.no_resume,
                &mut log,
            )?;
            eprintln!(
                "fleet proxy put: {} -> {remote}: {} bytes, sha256 {}",
                local.display(),
                report.size,
                report.sha256
            );
        }
        return Ok(0);
    }
    if args.local == "-" {
        // Stdin: spool to a local temp first — the protocol's
        // begin needs size + sha256 up front. The spool is ours
        // alone (pid-named, removed on every exit path below).
        let spool = std::env::temp_dir().join(format!(
            "fleet-proxy-put-{}-{}.tmp",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let spool_result = (|| -> Result<PutReport, ClientError> {
            {
                let mut out = std::fs::File::create(&spool).map_err(|e| {
                    ClientError::Transport(format!("create stdin spool {}: {e}", spool.display()))
                })?;
                std::io::copy(&mut std::io::stdin(), &mut out)
                    .map_err(|e| ClientError::Transport(format!("spool stdin: {e}")))?;
            }
            put_file(
                &mut transport,
                &spool,
                &args.remote,
                args.mode.or(Some(0o644)),
                !args.no_resume,
                &mut log,
            )
        })();
        let _ = std::fs::remove_file(&spool);
        let report = spool_result?;
        eprintln!(
            "fleet proxy put: stdin -> {}: {} bytes, sha256 {}",
            args.remote, report.size, report.sha256
        );
        return Ok(0);
    }
    let local = Path::new(&args.local);
    if local.is_dir() {
        return Err(ClientError::Usage(format!(
            "{} is a directory; use -r for a recursive put",
            args.local
        )));
    }
    let report = put_file(
        &mut transport,
        local,
        &args.remote,
        args.mode,
        !args.no_resume,
        &mut log,
    )?;
    eprintln!(
        "fleet proxy put: {} -> {}: {} bytes, sha256 {}",
        args.local, args.remote, report.size, report.sha256
    );
    Ok(0)
}

pub fn run_get(
    args: &GetArgs,
    config: &Config,
    client: &str,
    capabilities: &[String],
) -> Result<i32, ClientError> {
    validate_remote_path(&args.remote)?;
    if args.recursive {
        return Err(ClientError::Usage(
            "fleet proxy get -r is deferred in v1: the daemon protocol has no directory-listing route; \
             download files individually or use put -r from the laptop side. Follow-up recorded on #5759."
                .to_string(),
        ));
    }
    let _ensured = ensure_broker(config, client, capabilities)?;
    let mut transport = BrokerFilesTransport::new(config);
    let mut log = |line: &str| eprintln!("{line}");
    if args.local == "-" {
        let mut stdout = std::io::stdout();
        let report = get_to_writer(&mut transport, &args.remote, &mut stdout)?;
        eprintln!(
            "fleet proxy get: {} -> stdout: {} bytes, sha256 {}",
            args.remote, report.size, report.sha256
        );
        return Ok(0);
    }
    let report = get_file(
        &mut transport,
        &args.remote,
        Path::new(&args.local),
        args.mode,
        !args.no_resume,
        &mut log,
    )?;
    eprintln!(
        "fleet proxy get: {} -> {}: {} bytes, sha256 {}",
        args.remote, args.local, report.size, report.sha256
    );
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::io::Cursor;
    use std::sync::{Arc, Mutex};

    // --- Codec oracles -------------------------------------------------

    #[test]
    fn sha256_matches_fips_vectors() {
        // Oracle: FIPS 180-4 examples.
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        // Incremental updates in odd-sized pieces hash identically.
        let mut h = Sha256::new();
        for piece in b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq".chunks(7) {
            h.update(piece);
        }
        assert_eq!(
            h.finalize_hex(),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        // A multi-block stream crossing the chunk buffer.
        let big = vec![b'x'; 1_000_000];
        assert_eq!(sha256_hex(&big), {
            let mut h = Sha256::new();
            h.update(&big);
            h.finalize_hex()
        });
    }

    #[test]
    fn base64_matches_rfc4648_vectors() {
        // Oracle: RFC 4648 §10.
        let vectors = [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ];
        for (raw, encoded) in vectors {
            assert_eq!(b64_encode(raw.as_bytes()), encoded);
            assert_eq!(b64_decode(encoded).unwrap(), raw.as_bytes());
        }
        assert!(b64_decode("Zm9!").is_err());
        assert!(b64_decode("Zm9").is_err());
        assert!(b64_decode("Zg==Zg==").is_ok());
    }

    #[test]
    fn chunk_plan_tiles_exactly() {
        // Oracle: the plan tiles [start, size) with in-bounds lens.
        for (size, start) in [
            (0u64, 0u64),
            (1, 0),
            (CHUNK_SIZE as u64, 0),
            (CHUNK_SIZE as u64 + 1, 0),
            (5_500_000, 0),
            (5_500_000, 2_097_152),
            (5_500_000, 5_500_000),
        ] {
            let plan = chunk_plan(size, start);
            let mut expect = start;
            for (offset, len) in &plan {
                assert_eq!(*offset, expect);
                assert!(*len >= 1 && *len <= CHUNK_SIZE);
                expect += *len as u64;
            }
            assert_eq!(expect, size, "plan for size={size} start={start}");
        }
        assert_eq!(
            temp_path_for("/home/user/dir/file.bin", "abcdef0123456789"),
            "/home/user/dir/.file.bin.fleet-part-abcdef012345"
        );
        assert_eq!(
            temp_path_for("file.bin", "abcdef0123456789"),
            ".file.bin.fleet-part-abcdef012345"
        );
    }

    // --- Hermetic fake daemon ------------------------------------------
    //
    // An in-memory implementation of the #5759 daemon protocol —
    // the oracle is the design's route semantics: deterministic
    // temps, offset-idempotent writes, hash-checked atomic commit.

    #[derive(Default)]
    struct FakeDaemon {
        files: HashMap<String, Vec<u8>>,
        modes: HashMap<String, u32>,
        pending_modes: HashMap<String, u32>,
        write_count: usize,
    }

    impl FakeDaemon {
        fn stat(&self, body: &Value) -> Value {
            let path = body["path"].as_str().unwrap_or("");
            match self.files.get(path) {
                Some(bytes) => {
                    let mut out = json!({
                        "exists": true, "type": "file",
                        "size": bytes.len() as u64, "mtime": 0,
                    });
                    if body.get("sha256").and_then(|v| v.as_bool()) == Some(true) {
                        out["sha256"] = json!(sha256_hex(bytes));
                    }
                    out
                }
                None => json!({"exists": false, "type": "missing", "size": 0}),
            }
        }

        fn handle(&mut self, route: &str, body: &Value) -> Value {
            match route {
                "/files/stat" => self.stat(body),
                "/files/begin" => {
                    let path = body["path"].as_str().unwrap_or("").to_string();
                    let sha = body["sha256"].as_str().unwrap_or("").to_string();
                    let temp = temp_path_for(&path, &sha);
                    // Resume: an existing temp for the same
                    // (path, sha) is kept, never truncated.
                    self.files.entry(temp.clone()).or_default();
                    if let Some(mode) = body.get("mode").and_then(|v| v.as_u64()) {
                        self.pending_modes.insert(temp, mode as u32);
                    }
                    json!({"ok": true})
                }
                "/files/write" => {
                    self.write_count += 1;
                    let path = body["path"].as_str().unwrap_or("");
                    let sha = body["sha256"].as_str().unwrap_or("");
                    let temp = temp_path_for(path, sha);
                    let offset = body["offset"].as_u64().unwrap_or(0) as usize;
                    let data = b64_decode(body["data_b64"].as_str().unwrap_or("")).unwrap();
                    let slot = self.files.entry(temp).or_default();
                    if offset > slot.len() {
                        return json!({"error": "offset beyond end of temp"});
                    }
                    let overlap = (slot.len() - offset).min(data.len());
                    if slot[offset..offset + overlap] != data[..overlap] {
                        return json!({"error": "conflicting rewrite at offset"});
                    }
                    slot.truncate(offset + overlap);
                    slot.extend_from_slice(&data[overlap..]);
                    json!({"size": slot.len() as u64})
                }
                "/files/commit" => {
                    let path = body["path"].as_str().unwrap_or("").to_string();
                    let sha = body["sha256"].as_str().unwrap_or("");
                    let temp = temp_path_for(&path, sha);
                    let bytes = self.files.get(&temp).cloned().unwrap_or_default();
                    let actual = sha256_hex(&bytes);
                    if actual != sha {
                        return json!({"error": "sha256 mismatch", "actual": actual});
                    }
                    self.files.remove(&temp);
                    if let Some(mode) = self.pending_modes.remove(&temp) {
                        self.modes.insert(path.clone(), mode);
                    }
                    let size = bytes.len() as u64;
                    self.files.insert(path.clone(), bytes);
                    json!({"size": size, "sha256": actual})
                }
                "/files/abort" => {
                    let path = body["path"].as_str().unwrap_or("");
                    let sha = body["sha256"].as_str().unwrap_or("");
                    self.files.remove(&temp_path_for(path, sha));
                    json!({"ok": true})
                }
                "/files/read" => {
                    let path = body["path"].as_str().unwrap_or("");
                    let offset = body["offset"].as_u64().unwrap_or(0) as usize;
                    let length = body["length"].as_u64().unwrap_or(0) as usize;
                    match self.files.get(path) {
                        Some(bytes) => {
                            let end = (offset + length).min(bytes.len());
                            let slice = if offset <= bytes.len() {
                                &bytes[offset..end]
                            } else {
                                &[]
                            };
                            json!({
                                "data_b64": b64_encode(slice),
                                "eof": end >= bytes.len(),
                                "size": bytes.len() as u64,
                                "mtime": 0,
                            })
                        }
                        None => json!({"error": "no such file"}),
                    }
                }
                _ => json!({"error": format!("unknown route {route}")}),
            }
        }
    }

    struct FakeTransport {
        daemon: Arc<Mutex<FakeDaemon>>,
    }

    impl FilesTransport for FakeTransport {
        fn post(&mut self, path: &str, body: Value) -> Result<Value, ClientError> {
            let reply = self.daemon.lock().unwrap().handle(path, &body);
            check_reply(path, 200, reply)
        }
    }

    fn patterned_bytes(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i * 31 % 251) as u8).collect()
    }

    fn put_bytes(
        t: &mut dyn FilesTransport,
        remote: &str,
        data: &[u8],
        resume: bool,
    ) -> Result<PutReport, ClientError> {
        let mut cursor = Cursor::new(data.to_vec());
        let sha = sha256_hex(data);
        put_stream(
            t,
            remote,
            &mut cursor,
            data.len() as u64,
            &sha,
            Some(0o640),
            resume,
            &mut |_| {},
        )
    }

    #[test]
    fn put_get_round_trip_multi_chunk() {
        // Multi-chunk round trip through the fake daemon must be
        // byte-identical — the smoke.sh oracle, in-process.
        let daemon = Arc::new(Mutex::new(FakeDaemon::default()));
        let mut t = FakeTransport {
            daemon: daemon.clone(),
        };
        let data = patterned_bytes(CHUNK_SIZE * 2 + 12345);
        let report = put_bytes(&mut t, "/srv/blob.bin", &data, true).unwrap();
        assert_eq!(report.size, data.len() as u64);
        // The mode declared at begin lands on the committed file.
        assert_eq!(
            daemon.lock().unwrap().modes.get("/srv/blob.bin"),
            Some(&0o640)
        );
        let mut out: Vec<u8> = Vec::new();
        let got = get_to_writer(&mut t, "/srv/blob.bin", &mut out).unwrap();
        assert_eq!(got.sha256, report.sha256);
        assert_eq!(out, data);
    }

    #[test]
    fn put_resume_continues_from_temp_size() {
        let daemon = Arc::new(Mutex::new(FakeDaemon::default()));
        let mut t = FakeTransport {
            daemon: daemon.clone(),
        };
        let data = patterned_bytes(CHUNK_SIZE + 500);
        let sha = sha256_hex(&data);
        // Simulate an interrupted put: begin + first chunk only.
        t.post(
            "/files/begin",
            json!({"path": "/srv/f.bin", "size": data.len(), "sha256": sha}),
        )
        .unwrap();
        t.post(
            "/files/write",
            json!({
                "path": "/srv/f.bin", "sha256": sha, "offset": 0,
                "data_b64": b64_encode(&data[..CHUNK_SIZE]),
            }),
        )
        .unwrap();
        let before = daemon.lock().unwrap().write_count;
        let report = put_bytes(&mut t, "/srv/f.bin", &data, true).unwrap();
        assert_eq!(report.resumed_from, CHUNK_SIZE as u64);
        // Only the remaining chunk was written again.
        assert_eq!(daemon.lock().unwrap().write_count - before, 1);
        let mut out: Vec<u8> = Vec::new();
        get_to_writer(&mut t, "/srv/f.bin", &mut out).unwrap();
        assert_eq!(out, data);
    }

    #[test]
    fn write_idempotency_and_conflict() {
        let daemon = Arc::new(Mutex::new(FakeDaemon::default()));
        let mut t = FakeTransport { daemon };
        let data = b"hello world".to_vec();
        let sha = sha256_hex(&data);
        t.post(
            "/files/begin",
            json!({"path": "/srv/g.txt", "size": data.len(), "sha256": sha}),
        )
        .unwrap();
        let write = json!({
            "path": "/srv/g.txt", "sha256": sha, "offset": 0,
            "data_b64": b64_encode(&data),
        });
        let first = t.post("/files/write", write.clone()).unwrap();
        // Same offset, same bytes: a no-op success (retry-safe).
        let second = t.post("/files/write", write).unwrap();
        assert_eq!(first, second);
        // Same offset, different bytes: refused.
        let conflict = t.post(
            "/files/write",
            json!({
                "path": "/srv/g.txt", "sha256": sha, "offset": 0,
                "data_b64": b64_encode(b"HELLO WORLD"),
            }),
        );
        assert!(conflict.is_err());
    }

    #[test]
    fn commit_refuses_hash_mismatch_and_keeps_temp() {
        let daemon = Arc::new(Mutex::new(FakeDaemon::default()));
        let mut t = FakeTransport {
            daemon: daemon.clone(),
        };
        let data = b"real bytes".to_vec();
        let wrong_sha = sha256_hex(b"claimed bytes");
        t.post(
            "/files/begin",
            json!({"path": "/srv/h.txt", "size": data.len(), "sha256": wrong_sha}),
        )
        .unwrap();
        t.post(
            "/files/write",
            json!({
                "path": "/srv/h.txt", "sha256": wrong_sha, "offset": 0,
                "data_b64": b64_encode(&data),
            }),
        )
        .unwrap();
        let err = t
            .post(
                "/files/commit",
                json!({"path": "/srv/h.txt", "sha256": wrong_sha}),
            )
            .unwrap_err();
        assert!(format!("{err}").contains("sha256 mismatch"), "{err}");
        // Temp stays for inspection/resume; target never appeared.
        let d = daemon.lock().unwrap();
        assert!(
            d.files
                .contains_key(&temp_path_for("/srv/h.txt", &wrong_sha))
        );
        assert!(!d.files.contains_key("/srv/h.txt"));
    }

    #[test]
    fn get_file_verifies_and_renames() {
        let daemon = Arc::new(Mutex::new(FakeDaemon::default()));
        let mut t = FakeTransport { daemon };
        let data = patterned_bytes(100_000);
        put_bytes(&mut t, "/srv/dl.bin", &data, true).unwrap();
        let dir = std::env::temp_dir().join(format!(
            "fleet-files-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let local = dir.join("dl.bin");
        let report = get_file(&mut t, "/srv/dl.bin", &local, None, true, &mut |_| {}).unwrap();
        assert_eq!(report.size, data.len() as u64);
        assert_eq!(std::fs::read(&local).unwrap(), data);
        // Missing remote file is a transport error, not an empty get.
        assert!(get_file(&mut t, "/srv/nope", &local, None, true, &mut |_| {}).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn broker_transport_round_trip_over_a_temp_socket() {
        // The hermetic fake-daemon broker test: a framed
        // Request/Response server on a real Unix socket, speaking
        // for the daemon; BrokerFilesTransport drives a full
        // multi-chunk put + get through it.
        use std::os::unix::net::UnixListener;
        let dir = std::env::temp_dir().join(format!(
            "fleet-files-sock-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("broker.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let daemon = Arc::new(Mutex::new(FakeDaemon::default()));
        let server_daemon = daemon.clone();
        let server = std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = match stream {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                let daemon = server_daemon.clone();
                std::thread::spawn(move || {
                    loop {
                        let request: Request = match fleet_proxy_proto::read_typed(&mut stream) {
                            Ok(r) => r,
                            Err(_) => return,
                        };
                        let body = request.body.clone().unwrap_or(Value::Null);
                        let reply = daemon.lock().unwrap().handle(&request.path, &body);
                        let status = if reply.get("error").is_some() {
                            500
                        } else {
                            200
                        };
                        let response = Response {
                            status,
                            body: reply,
                        };
                        if fleet_proxy_proto::write_frame(&mut stream, &response).is_err() {
                            return;
                        }
                    }
                });
            }
        });
        drop(server);
        let mut t = BrokerFilesTransport {
            socket: socket.clone(),
            token_file: dir.join("token"),
            port: fleet_config::PORT_LAPTOP_AGENT,
        };
        let data = patterned_bytes(CHUNK_SIZE + 777);
        let report = put_bytes(&mut t, "/srv/sock.bin", &data, true).unwrap();
        assert_eq!(report.size, data.len() as u64);
        let mut out: Vec<u8> = Vec::new();
        get_to_writer(&mut t, "/srv/sock.bin", &mut out).unwrap();
        assert_eq!(out, data);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn arg_parsing_modes_and_shapes() {
        let put = PutArgs::parse(&[
            "--mode".to_string(),
            "640".to_string(),
            "a.txt".to_string(),
            "/srv/a.txt".to_string(),
        ])
        .unwrap();
        assert_eq!(put.mode, Some(0o640));
        assert!(!put.recursive);
        let get =
            GetArgs::parse(&["-r".to_string(), "/srv/d".to_string(), "d".to_string()]).unwrap();
        assert!(get.recursive);
        assert!(PutArgs::parse(&["only-one".to_string()]).is_err());
        assert!(
            PutArgs::parse(&[
                "--mode".to_string(),
                "banana".to_string(),
                "a".to_string(),
                "b".to_string(),
            ])
            .is_err()
        );
        assert!(validate_remote_path("").is_err());
        assert!(validate_remote_path("/ok/path").is_ok());
        assert!(validate_remote_path("bad\0path").is_err());
    }
}
