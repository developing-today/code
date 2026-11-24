//! Provider registry config: the static client registry, the single
//! account row, its linked identities, the deny-by-default allowlist,
//! and the local-door credential hash.
//!
//! This is the provider's *config*, loaded from a JSON file whose
//! path is configured at launch. The real file lives on-host with
//! the provider (never in the repo): it carries the owner's credential
//! HASH (PBKDF2-HMAC-SHA256 — the plaintext credential is never
//! stored anywhere in the provider) and the audit DB location. The
//! repo ships `config.example.json` with the hash field empty; an
//! empty hash verifies nothing, so the door stays shut until the
//! on-host config is written.
//!
//! Federated doors (#13411/#13412/#13414) are configured here too:
//! endpoints and client ids are config, but a door's client SECRET
//! is never in this file — the config names the environment variable
//! the launcher fills from the vault by name.

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

#[cfg(test)]
mod saml_registration_tests {
    use super::*;
    use proptest::prelude::*;

    fn load_sps(sps: serde_json::Value) -> Result<ProviderConfig, String> {
        let config = serde_json::json!({
            "account": {"account_id": "owner", "roles": []},
            "saml_sps": sps
        });
        let path = std::env::temp_dir().join(format!("idp-saml-{}.json", random_b64url(16)));
        std::fs::write(&path, config.to_string()).unwrap();
        let result = ProviderConfig::load(&path);
        std::fs::remove_file(path).unwrap();
        result
    }

    fn sp(entity_id: &str, acs: &str) -> serde_json::Value {
        serde_json::json!({"entity_id": entity_id, "acs_urls": [acs]})
    }

    #[test]
    fn saml_registry_oracle_rejects_invalid_trust_configuration() {
        // Oracle: static registration contract, design §5 + HTTPS/loopback policy.
        for bad in [
            sp("", "https://sp.example/acs"),
            sp("  ", "https://sp.example/acs"),
            serde_json::json!({"entity_id": "urn:test:sp", "acs_urls": []}),
            sp("urn:test:sp", "/acs"),
            sp("urn:test:sp", "http://sp.example/acs"),
            sp("urn:test:sp", "http://localhost.example/acs"),
            sp("urn:test:sp", "http://[2001:db8::1]/acs"),
            sp("urn:test:sp", "ftp://sp.example/acs"),
            sp("urn:test:sp", "https://user:password@sp.example/acs"),
            sp("urn:test:sp", "https://sp.example/acs#fragment"),
        ] {
            assert!(
                load_sps(serde_json::json!([bad])).is_err(),
                "accepted {bad}"
            );
        }
        let entry = sp("urn:test:sp", "https://sp.example/acs");
        assert!(load_sps(serde_json::json!([entry, entry])).is_err());
        for acs in [
            "http://localhost:8080/acs",
            "http://127.0.0.2/acs",
            "http://[::1]/acs",
        ] {
            assert!(load_sps(serde_json::json!([sp("local", acs)])).is_ok());
        }
    }

    #[test]
    fn saml_registry_oracle_defaults_and_exact_matching() {
        // Oracle: design §§3/5; legacy configs remain OIDC-only.
        let legacy: ProviderConfig = serde_json::from_value(serde_json::json!({
            "account": {"account_id": "owner", "roles": []}
        }))
        .unwrap();
        assert!(legacy.saml_sps.is_empty());
        let config = load_sps(serde_json::json!([
            sp("urn:test:sp", "https://sp.example/acs"),
            {"entity_id": "other", "acs_urls": ["http://[::1]:8080/acs"],
             "released_attributes": []}
        ]))
        .unwrap();
        let registered = config.saml_sp("urn:test:sp").unwrap();
        assert_eq!(registered.released_attributes, SAML_ATTRIBUTES);
        assert!(!registered.require_signed_requests);
        assert!(registered.request_signing_certificate.is_none());
        assert!(registered.accepts_acs("https://sp.example/acs"));
        assert!(!registered.accepts_acs("https://sp.example/acs/"));
        assert!(!registered.accepts_acs("https://SP.example/acs"));
        assert!(config.saml_sp("URN:test:sp").is_none());
        assert!(config.saml_sp("unknown").is_none());
        assert!(config
            .saml_sp("other")
            .unwrap()
            .released_attributes
            .is_empty());
    }

    #[test]
    fn saml_registry_oracle_certificate_and_release_policy() {
        // Oracle: signed requests need a valid configured trust cert;
        // the release set may only narrow the design's two attributes.
        let cert = include_str!("../../idp-pki/trust/saml-signing.crt.pem");
        let mut entry = sp("urn:test:sp", "https://sp.example/acs");
        entry["require_signed_requests"] = true.into();
        assert!(load_sps(serde_json::json!([entry])).is_err());
        entry["request_signing_certificate"] = cert.into();
        entry["acs_urls"] =
            serde_json::json!(["https://sp.example/acs", "https://sp.example/alternate"]);
        entry["released_attributes"] = serde_json::json!(["roles"]);
        let config = load_sps(serde_json::json!([entry])).unwrap();
        let registered = config.saml_sp("urn:test:sp").unwrap();
        assert_eq!(registered.entity_id, "urn:test:sp");
        assert_eq!(
            registered.acs_urls,
            ["https://sp.example/acs", "https://sp.example/alternate"]
        );
        assert_eq!(
            registered.request_signing_certificate.as_deref(),
            Some(cert)
        );
        assert_eq!(registered.released_attributes, ["roles"]);
        assert!(registered.require_signed_requests);
        entry["request_signing_certificate"] = "-----BEGIN CERTIFICATE-----\nbogus".into();
        assert!(load_sps(serde_json::json!([entry])).is_err());
        entry["request_signing_certificate"] = cert.into();
        entry["released_attributes"] = serde_json::json!(["email"]);
        assert!(load_sps(serde_json::json!([entry])).is_err());
        entry["released_attributes"] = serde_json::json!(["roles", "roles"]);
        assert!(load_sps(serde_json::json!([entry])).is_err());
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        #[test]
        fn saml_registry_property_oracle_http_only_loopback(
            octets in any::<[u8; 4]>(), port in 1u16..=65535, path in "[a-z]{1,24}"
        ) {
            // Oracle: std's IP loopback classification, independent of URL policy.
            let ip = std::net::Ipv4Addr::from(octets);
            let http = format!("http://{ip}:{port}/{path}");
            prop_assert_eq!(validate_acs_url(&http).is_ok(), ip.is_loopback());
            prop_assert!(validate_acs_url(&format!("https://{ip}:{port}/{path}")).is_ok());
        }

        #[test]
        fn saml_registry_property_oracle_uniqueness_and_exact_acs(
            entity in "[a-zA-Z0-9:_-]{1,80}", path in "[a-z]{1,24}"
        ) {
            // Oracle: set uniqueness and literal equality (no URL normalization).
            let url = format!("https://sp.example/{path}");
            let sp: SamlSp = serde_json::from_value(sp(&entity, &url)).unwrap();
            prop_assert!(validate_saml_registrations(std::slice::from_ref(&sp)).is_ok());
            prop_assert!(sp.accepts_acs(&url));
            prop_assert!(!sp.accepts_acs(&format!("{url}/")));
            prop_assert!(validate_saml_registrations(&[sp.clone(), sp]).is_err());
        }
    }
}

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

/// One upstream federated door. `name` is also the identity
/// provider key: identities from this door are recorded as
/// `<name>:<subject>`.
#[derive(Clone, Debug, Deserialize)]
pub struct UpstreamDoor {
    /// "github" | "google" | "microsoft" (any name works; it keys
    /// the door's routes, allowlist entries, and identity rows).
    pub name: String,
    /// "oidc" (Google/Microsoft shape: ID token verified against
    /// the upstream JWKS) or "github" (code exchange + user fetch,
    /// subject = the numeric user id).
    pub kind: String,
    #[serde(default)]
    pub display_name: String,
    pub client_id: String,
    /// Name of the environment variable holding the door's client
    /// secret. The launcher fills it from the vault by name (e.g.
    /// vault `idp.github.client-secret` -> `IDP_GITHUB_CLIENT_SECRET`);
    /// the secret itself is never in this config file or the repo.
    pub client_secret_env: String,
    pub authorize_url: String,
    pub token_url: String,
    /// OIDC doors: the upstream JWKS URI ID tokens are verified
    /// against.
    #[serde(default)]
    pub jwks_uri: Option<String>,
    /// GitHub door: the user endpoint (default
    /// `https://api.github.com/user`).
    #[serde(default)]
    pub userinfo_url: Option<String>,
    /// OIDC doors: the expected `iss` of upstream ID tokens.
    #[serde(default)]
    pub issuer: Option<String>,
    /// OIDC doors: which claim names the recorded subject
    /// (default `sub`; Microsoft records `oid`, falling back to
    /// `sub` when absent).
    #[serde(default)]
    pub subject_claim: Option<String>,
    /// Scopes requested at the upstream (OIDC default `openid`;
    /// GitHub default `read:user`).
    #[serde(default)]
    pub scopes: Vec<String>,
}

/// The attributes a SAML SP may receive. The design's default release
/// set is exactly these two (§3); a registration may narrow it but
/// never add an attribute the provider does not vouch for.
pub const SAML_ATTRIBUTES: [&str; 2] = ["roles", "displayName"];

fn default_saml_attributes() -> Vec<String> {
    SAML_ATTRIBUTES
        .iter()
        .map(|attr| (*attr).to_string())
        .collect()
}

/// One statically registered SAML service provider (design §5). Trust
/// is a reviewed config change: nothing here fetches metadata, and no
/// field accepts a metadata URL.
#[derive(Clone, Debug, Deserialize)]
pub struct SamlSp {
    /// The SP's entityID, matched exactly (opaque identifier, not
    /// required to be a URL).
    pub entity_id: String,
    /// Assertion Consumer Service URLs; at least one. The response
    /// Recipient must equal one of these exactly.
    pub acs_urls: Vec<String>,
    /// Optional SP certificate for verifying signed AuthnRequests
    /// (PEM). Required when `require_signed_requests` is set.
    #[serde(default)]
    pub request_signing_certificate: Option<String>,
    /// Attributes released to this SP; defaults to the design's
    /// `roles` + `displayName` set.
    #[serde(default = "default_saml_attributes")]
    pub released_attributes: Vec<String>,
    /// Reject unsigned AuthnRequests from this SP.
    #[serde(default)]
    pub require_signed_requests: bool,
}

impl SamlSp {
    pub fn accepts_acs(&self, acs: &str) -> bool {
        self.acs_urls.iter().any(|u| u == acs)
    }
}

fn loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<std::net::IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false)
}

/// Validate one ACS URL: absolute HTTP(S), no credentials, no
/// fragment, HTTPS unless the host is loopback (the local development
/// carve-out; everything reachable off-host is TLS-only).
fn validate_acs_url(url: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("invalid ACS URL {url:?}: {e}"))?;
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(format!("ACS URL {url:?} must not carry credentials"));
    }
    if parsed.fragment().is_some() {
        return Err(format!("ACS URL {url:?} must not carry a fragment"));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| format!("ACS URL {url:?} has no host"))?;
    match parsed.scheme() {
        "https" => Ok(()),
        "http" if loopback_host(host) => Ok(()),
        "http" => Err(format!(
            "ACS URL {url:?} is HTTP; only loopback HTTP is allowed"
        )),
        scheme => Err(format!("ACS URL {url:?} must be http(s), not {scheme}")),
    }
}

fn validate_saml_registrations(sps: &[SamlSp]) -> Result<(), String> {
    let mut seen = std::collections::HashSet::new();
    for sp in sps {
        if sp.entity_id.trim().is_empty() {
            return Err("SAML SP registration has an empty entityID".into());
        }
        if !seen.insert(sp.entity_id.as_str()) {
            return Err(format!("duplicate SAML entityID {:?}", sp.entity_id));
        }
        if sp.acs_urls.is_empty() {
            return Err(format!("SAML SP {:?} has no ACS URLs", sp.entity_id));
        }
        let mut acs_seen = std::collections::HashSet::new();
        for acs in &sp.acs_urls {
            validate_acs_url(acs).map_err(|e| format!("SAML SP {:?}: {e}", sp.entity_id))?;
            if !acs_seen.insert(acs.as_str()) {
                return Err(format!(
                    "SAML SP {:?} repeats ACS URL {acs:?}",
                    sp.entity_id
                ));
            }
        }
        if let Some(cert) = &sp.request_signing_certificate {
            use der::DecodePem;
            if x509_cert::Certificate::from_pem(cert.as_bytes()).is_err() {
                return Err(format!(
                    "SAML SP {:?} request-signing certificate is not a valid PEM X.509 certificate",
                    sp.entity_id
                ));
            }
        }
        if sp.require_signed_requests && sp.request_signing_certificate.is_none() {
            return Err(format!(
                "SAML SP {:?} requires signed requests but has no certificate",
                sp.entity_id
            ));
        }
        let mut attr_seen = std::collections::HashSet::new();
        for attr in &sp.released_attributes {
            if !SAML_ATTRIBUTES.contains(&attr.as_str()) {
                return Err(format!(
                    "SAML SP {:?} releases unknown attribute {attr:?}",
                    sp.entity_id
                ));
            }
            if !attr_seen.insert(attr.as_str()) {
                return Err(format!(
                    "SAML SP {:?} repeats attribute {attr:?}",
                    sp.entity_id
                ));
            }
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize)]
pub struct ProviderConfig {
    pub account: Account,
    /// Static SAML trust registry; absent in legacy OIDC configs.
    #[serde(default)]
    pub saml_sps: Vec<SamlSp>,
    /// Deny-by-default allowlist of identity keys (`provider:subject`)
    /// holding only the owner's account IDs.
    #[serde(default)]
    pub allowlist: Vec<String>,
    #[serde(default)]
    pub clients: Vec<Client>,
    /// Federated doors, in login-form order.
    #[serde(default)]
    pub doors: Vec<UpstreamDoor>,
    /// Audit DB path; when absent the audit log is in-memory only.
    #[serde(default)]
    pub audit_db_path: Option<PathBuf>,
    /// Identity store DB path (#13415); when absent the link state
    /// is in-memory only (tests, ephemeral runs).
    #[serde(default)]
    pub identity_db_path: Option<PathBuf>,
}

impl ProviderConfig {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read provider config {}: {e}", path.display()))?;
        let config: Self = serde_json::from_str(&text)
            .map_err(|e| format!("failed to parse provider config {}: {e}", path.display()))?;
        validate_saml_registrations(&config.saml_sps)
            .map_err(|e| format!("invalid provider config {}: {e}", path.display()))?;
        Ok(config)
    }

    pub fn saml_sp(&self, entity_id: &str) -> Option<&SamlSp> {
        self.saml_sps.iter().find(|sp| sp.entity_id == entity_id)
    }

    pub fn client(&self, client_id: &str) -> Option<&Client> {
        self.clients.iter().find(|c| c.client_id == client_id)
    }

    pub fn is_allowlisted(&self, identity_key: &str) -> bool {
        self.allowlist.iter().any(|k| k == identity_key)
    }
}

// ---------- small helpers (time, randomness, credential hashing) ----------

pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Percent-encode a query component (unreserved characters pass
/// through, everything else becomes `%XX`).
pub fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
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
