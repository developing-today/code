//! Provider registry config: the static client registry, the single
//! account row, its linked identities, the deny-by-default allowlist,
//! and the local-door credential hash.
//!
//! This is the provider's *config*, loaded from a JSON file whose
//! path is configured at launch. The real file lives on-host with the
//! provider (never in the repo): it carries the owner's credential
//! HASH (PBKDF2-HMAC-SHA256 — the plaintext credential is never
//! stored anywhere in the provider) and the audit DB location. The
//! repo ships `config.example.json` with the hash field empty; an
//! empty hash verifies nothing, so the door stays shut until the
//! on-host config is written.

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// The stand-up door for this slice: the design's *devtoday local*
/// identity (§3 of the settled design). The federated doors
/// (GitHub/Google/Microsoft) land as their own slices and link into
/// the same account row.
pub const LOCAL_DOOR_PROVIDER: &str = "devtoday-local";

#[derive(Clone, Debug, Deserialize)]
pub struct Client {
    pub client_id: String,
    #[serde(default)]
    pub name: String,
    pub redirect_uris: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Identity {
    pub provider: String,
    pub subject: String,
}

impl Identity {
    /// Allowlist key form: `provider:subject`.
    pub fn key(&self) -> String {
        format!("{}:{}", self.provider, self.subject)
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Account {
    pub account_id: String,
    #[serde(default)]
    pub display_name: String,
    /// The account's #1253 role set (owner / orchestrator / agent /
    /// read-only). Roles live on the account, not the door.
    pub roles: Vec<String>,
    /// Linked external identities (linking history is retained for
    /// the life of the account; rows are never hard-deleted here).
    #[serde(default)]
    pub identities: Vec<Identity>,
    /// PBKDF2-HMAC-SHA256 hash of the local-door credential, in the
    /// form `pbkdf2-sha256$<iterations>$<salt b64url>$<hash b64url>`.
    /// Empty string = local door credential not set (verifies
    /// nothing).
    #[serde(default)]
    pub local_credential_hash: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ProviderConfig {
    pub account: Account,
    /// Deny-by-default allowlist of identity keys (`provider:subject`)
    /// holding only the owner's account IDs.
    #[serde(default)]
    pub allowlist: Vec<String>,
    #[serde(default)]
    pub clients: Vec<Client>,
    /// Audit DB path; when absent the audit log is in-memory only.
    #[serde(default)]
    pub audit_db_path: Option<PathBuf>,
}

impl ProviderConfig {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read provider config {}: {e}", path.display()))?;
        serde_json::from_str(&text)
            .map_err(|e| format!("failed to parse provider config {}: {e}", path.display()))
    }

    pub fn client(&self, client_id: &str) -> Option<&Client> {
        self.clients.iter().find(|c| c.client_id == client_id)
    }

    pub fn is_allowlisted(&self, identity_key: &str) -> bool {
        self.allowlist.iter().any(|k| k == identity_key)
    }

    /// Is `provider:subject` a linked identity of the account row?
    pub fn is_linked(&self, provider: &str, subject: &str) -> bool {
        self.account
            .identities
            .iter()
            .any(|i| i.provider == provider && i.subject == subject)
    }
}

// ---------- small helpers (time, randomness, credential hashing) ----------

pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Random bytes from the OS (`/dev/urandom`); used for authorization
/// codes, token ids, and credential salts.
pub fn random_bytes(n: usize) -> Vec<u8> {
    let mut buf = vec![0u8; n];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut buf))
        .is_err()
    {
        // Last-resort fallback: time/pid mixing. Only reached if the
        // OS RNG is unavailable, which would be a broken host.
        let seed = format!("{}:{}", now_unix(), std::process::id());
        let mut h = Sha256::digest(seed.as_bytes()).to_vec();
        while h.len() < n {
            let next = Sha256::digest(&h).to_vec();
            h.extend_from_slice(&next);
        }
        buf.copy_from_slice(&h[..n]);
    }
    buf
}

pub fn random_b64url(nbytes: usize) -> String {
    crate::b64url(&random_bytes(nbytes))
}

fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    let mut block = [0u8; 64];
    if key.len() > 64 {
        let hashed = Sha256::digest(key);
        block[..32].copy_from_slice(&hashed);
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    for i in 0..64 {
        ipad[i] ^= block[i];
        opad[i] ^= block[i];
    }
    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(msg);
    let inner_hash = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner_hash);
    outer.finalize().into()
}

/// PBKDF2-HMAC-SHA256, 32-byte output.
pub fn pbkdf2_sha256(password: &[u8], salt: &[u8], iterations: u32) -> [u8; 32] {
    let mut u = {
        let mut msg = salt.to_vec();
        msg.extend_from_slice(&1u32.to_be_bytes());
        hmac_sha256(password, &msg)
    };
    let mut out = u;
    for _ in 1..iterations.max(1) {
        u = hmac_sha256(password, &u);
        for i in 0..32 {
            out[i] ^= u[i];
        }
    }
    out
}

pub const CREDENTIAL_HASH_ITERATIONS: u32 = 210_000;

/// Hash a local-door credential for storage in the on-host provider
/// config. The plaintext is never stored.
pub fn hash_credential(credential: &str) -> String {
    let salt = random_bytes(16);
    let hash = pbkdf2_sha256(credential.as_bytes(), &salt, CREDENTIAL_HASH_ITERATIONS);
    format!(
        "pbkdf2-sha256${}${}${}",
        CREDENTIAL_HASH_ITERATIONS,
        crate::b64url(&salt),
        crate::b64url(&hash)
    )
}

/// Verify a presented credential against a stored hash. Empty or
/// malformed stored hashes verify nothing (door stays shut).
pub fn verify_credential(credential: &str, stored: &str) -> bool {
    let parts: Vec<&str> = stored.split('$').collect();
    if parts.len() != 4 || parts[0] != "pbkdf2-sha256" {
        return false;
    }
    let iterations: u32 = match parts[1].parse() {
        Ok(n) if n > 0 => n,
        _ => return false,
    };
    let decode =
        |s: &str| base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, s).ok();
    let (salt, expected) = match (decode(parts[2]), decode(parts[3])) {
        (Some(s), Some(e)) if e.len() == 32 => (s, e),
        _ => return false,
    };
    let actual = pbkdf2_sha256(credential.as_bytes(), &salt, iterations);
    // Constant-time comparison.
    let mut diff = 0u8;
    for i in 0..32 {
        diff |= actual[i] ^ expected[i];
    }
    diff == 0
}
