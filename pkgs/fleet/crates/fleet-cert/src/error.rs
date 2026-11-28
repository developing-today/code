//! The crate's error type. Every failure the CLI can produce is one of
//! these; `exit_code` maps them onto the fleet binary's contract
//! (0 ok · 2 usage · 1 operational failure · 69 unavailable).

use std::fmt;

#[derive(Debug)]
pub enum CertError {
    /// Bad flags, bad values, contradictory options — the caller's fault.
    Usage(String),
    /// A spec policy gate refused the operation (legacy algorithm
    /// without its opt-in, a missing explicit signer source, a
    /// forbidden cross-sign edge, a PQ-policy violation, ...).
    Policy(String),
    /// The underlying engine (rcgen / aws-lc-rs) refused.
    Engine(String),
    /// A bundle or manifest is missing, malformed, or fails its own
    /// checksums.
    Manifest(String),
    /// Chain building or validation failed; the message carries the
    /// per-check detail.
    Validation(String),
    /// Filesystem failure.
    Io(std::io::Error),
}

impl CertError {
    pub fn usage(msg: impl Into<String>) -> Self {
        CertError::Usage(msg.into())
    }
    pub fn policy(msg: impl Into<String>) -> Self {
        CertError::Policy(msg.into())
    }
    pub fn engine(msg: impl Into<String>) -> Self {
        CertError::Engine(msg.into())
    }
    pub fn manifest(msg: impl Into<String>) -> Self {
        CertError::Manifest(msg.into())
    }
    pub fn validation(msg: impl Into<String>) -> Self {
        CertError::Validation(msg.into())
    }

    pub fn exit_code(&self) -> i32 {
        match self {
            CertError::Usage(_) => 2,
            _ => 1,
        }
    }
}

impl fmt::Display for CertError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CertError::Usage(m) => write!(f, "usage: {m}"),
            CertError::Policy(m) => write!(f, "policy: {m}"),
            CertError::Engine(m) => write!(f, "engine: {m}"),
            CertError::Manifest(m) => write!(f, "manifest: {m}"),
            CertError::Validation(m) => write!(f, "validation: {m}"),
            CertError::Io(e) => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for CertError {}

impl From<std::io::Error> for CertError {
    fn from(e: std::io::Error) -> Self {
        CertError::Io(e)
    }
}

impl From<rcgen::Error> for CertError {
    fn from(e: rcgen::Error) -> Self {
        CertError::Engine(e.to_string())
    }
}

impl From<serde_json::Error> for CertError {
    fn from(e: serde_json::Error) -> Self {
        CertError::Manifest(format!("json: {e}"))
    }
}

pub type Result<T> = std::result::Result<T, CertError>;
