//! How a caller proves who it is: an account credential, a session, a public
//! key it signed with, or the admin token. Also the one-time codes that prove
//! an email address.
//!
//! Sessions, codes and replay records live in memory beside the world, so a
//! restart signs everyone out and forgets pending codes. None of it is
//! journaled, and no session token or code is kept in a form that can be used
//! after its time is up.

use std::collections::HashMap;

use anyhow::{Context as _, Result};
use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey};
use rand::RngExt as _;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq as _;

use crate::directory::{Refusal, normalize_key};
use crate::world::hex_encode;

/// How long a session lasts after it is opened.
pub const SESSION_TTL_MS: u64 = 12 * 60 * 60 * 1000;
const CODE_TTL_MS: u64 = 15 * 60 * 1000;
const CODE_RESEND_MS: u64 = 60 * 1000;
const CODE_ATTEMPTS: u32 = 5;
/// How far a signed request's timestamp may be from the server's clock.
pub const SIGNED_WINDOW_MS: u64 = 5 * 60 * 1000;
const SIGNED_NONCE_LIMIT: usize = 4096;
const SIGNED_DOMAIN: &str = "id-request-v1";
const CODE_DOMAIN: &str = "id-code-v1";

/// Who a session stands for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Principal {
    /// A signed-in account, by ID.
    Account(String),
    /// The admin token, opened as a session.
    Admin,
}

/// What a confirmation code was sent for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// Attach the address to the account that asked.
    Confirm {
        /// The account that asked.
        account: String,
    },
    /// Sign in as the account the address belongs to.
    SignIn {
        /// The account the address belongs to.
        account: String,
    },
}

/// Everything a request presented to prove who it is. Each field is checked
/// by the transport or the directory before it counts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Caller {
    /// The admin token was presented and checked.
    pub admin: bool,
    /// An account credential, `acct.<id>.<secret>`.
    pub credential: Option<String>,
    /// A session token, `sess.<hex>`.
    pub session: Option<String>,
    /// A public key the request was signed with, as lower-case hex.
    pub key: Option<String>,
}

impl Caller {
    /// A caller from one secret: a session token, an account credential, or
    /// nothing when empty. The admin token is checked by the transport.
    #[must_use]
    pub fn from_secret(secret: &str) -> Self {
        if secret.is_empty() {
            Self::default()
        } else if secret.starts_with("sess.") {
            Self {
                session: Some(secret.to_owned()),
                ..Self::default()
            }
        } else {
            Self {
                credential: Some(secret.to_owned()),
                ..Self::default()
            }
        }
    }
}

#[derive(Debug)]
struct PendingCode {
    purpose: Purpose,
    digest: [u8; 32],
    expires_at: u64,
    sent_at: u64,
    attempts: u32,
}

#[derive(Debug)]
struct SessionRecord {
    principal: Principal,
    expires_at: u64,
}

/// Sessions and pending email codes for one world.
#[derive(Debug, Default)]
pub struct DirectoryAuth {
    /// Keyed by the SHA-256 of the token, so memory never holds a usable token.
    sessions: HashMap<[u8; 32], SessionRecord>,
    codes: HashMap<String, PendingCode>,
}

impl DirectoryAuth {
    /// Open a session for a principal and return its token, shown once.
    pub fn open_session(&mut self, principal: Principal, now: u64) -> String {
        self.sessions.retain(|_, record| record.expires_at > now);
        let mut secret = [0_u8; 32];
        rand::rng().fill(&mut secret);
        let token = format!("sess.{}", hex_encode(&secret));
        self.sessions.insert(
            Sha256::digest(token.as_bytes()).into(),
            SessionRecord {
                principal,
                expires_at: now + SESSION_TTL_MS,
            },
        );
        token
    }

    /// Who a session token stands for, if it is current.
    pub fn principal(&mut self, token: &str, now: u64) -> Option<Principal> {
        let digest: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        let record = self.sessions.get(&digest)?;
        if record.expires_at <= now {
            self.sessions.remove(&digest);
            return None;
        }
        Some(record.principal.clone())
    }

    /// End a session. Unknown tokens are ignored.
    pub fn close_session(&mut self, token: &str) {
        let digest: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        self.sessions.remove(&digest);
    }

    /// Make a six-digit code for an address and remember its digest. Returns
    /// the code, which is sent by mail and never stored.
    ///
    /// # Errors
    ///
    /// Fails with [`Refusal::RateLimited`] if a code for this address was sent
    /// less than a minute ago.
    pub fn issue_code(&mut self, purpose: Purpose, address: &str, now: u64) -> Result<String> {
        if let Some(pending) = self.codes.get(address)
            && pending.sent_at + CODE_RESEND_MS > now
        {
            return Err(Refusal::RateLimited(
                "a code was sent a moment ago; wait a minute before asking again".to_owned(),
            )
            .into());
        }
        let code = format!("{:06}", rand::rng().random_range(0..1_000_000_u32));
        self.codes.insert(
            address.to_owned(),
            PendingCode {
                purpose,
                digest: code_digest(address, &code),
                expires_at: now + CODE_TTL_MS,
                sent_at: now,
                attempts: 0,
            },
        );
        Ok(code)
    }

    /// Check a code for an address. A code is used up by a correct answer, by
    /// its expiry, or by too many wrong answers.
    ///
    /// # Errors
    ///
    /// Fails if no code is pending, it has expired or run out of attempts, or
    /// the answer is wrong.
    pub fn check_code(&mut self, address: &str, code: &str, now: u64) -> Result<Purpose> {
        let refused =
            || anyhow::Error::from(Refusal::Unauthenticated("no matching code".to_owned()));
        let pending = self.codes.get_mut(address).ok_or_else(refused)?;
        if pending.expires_at <= now || pending.attempts >= CODE_ATTEMPTS {
            self.codes.remove(address);
            return Err(refused());
        }
        let digest = code_digest(address, code.trim());
        if !bool::from(digest.ct_eq(&pending.digest)) {
            pending.attempts += 1;
            return Err(refused());
        }
        let pending = self.codes.remove(address).context("code vanished")?;
        Ok(pending.purpose)
    }
}

fn code_digest(address: &str, code: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(CODE_DOMAIN.as_bytes());
    hasher.update(b"\n");
    hasher.update(address.as_bytes());
    hasher.update(b"\n");
    hasher.update(code.as_bytes());
    hasher.finalize().into()
}

/// Remembers signed requests for their window, so a captured one cannot be
/// replayed inside it.
#[derive(Debug, Default)]
pub struct ReplayGuard {
    seen: HashMap<String, u64>,
}

impl ReplayGuard {
    /// Record a request's key and nonce.
    ///
    /// # Errors
    ///
    /// Fails if the same key and nonce were already used in the window, or if
    /// too many requests are being remembered.
    pub fn accept(&mut self, key: &str, nonce: &str, at: u64, now: u64) -> Result<()> {
        self.seen
            .retain(|_, seen_at| *seen_at + SIGNED_WINDOW_MS > now);
        let id = format!("{key}:{nonce}");
        if self.seen.contains_key(&id) {
            return Err(Refusal::Unauthenticated("request was already used".to_owned()).into());
        }
        if self.seen.len() >= SIGNED_NONCE_LIMIT {
            return Err(Refusal::RateLimited(
                "too many signed requests; try again shortly".to_owned(),
            )
            .into());
        }
        self.seen.insert(id, at);
        Ok(())
    }
}

/// The headers a signed request carries.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
pub struct Signed {
    /// The public key, as lower-case hex.
    pub key: String,
    /// When the request was made, in Unix milliseconds.
    pub at: u64,
    /// A random value, unique per request.
    pub nonce: String,
    /// Hex Ed25519 signature over the canonical message.
    pub signature: String,
}

/// The bytes a request signs: its method, its path and query, its time, its
/// nonce, and the hash of its body.
#[must_use]
pub fn signed_message(method: &str, target: &str, at: u64, nonce: &str, body: &[u8]) -> String {
    format!(
        "{SIGNED_DOMAIN}\n{method}\n{target}\n{at}\n{nonce}\n{}",
        hex_encode(&Sha256::digest(body))
    )
}

/// Sign a request as a client holding `key` would.
#[must_use]
pub fn sign_request(
    key: &SigningKey,
    method: &str,
    target: &str,
    body: &[u8],
    at: u64,
    nonce: &str,
) -> Signed {
    let message = signed_message(method, target, at, nonce, body);
    Signed {
        key: hex_encode(&key.verifying_key().to_bytes()),
        at,
        nonce: nonce.to_owned(),
        signature: hex_encode(&key.sign(message.as_bytes()).to_bytes()),
    }
}

/// Check a signed request and return the key that made it.
///
/// # Errors
///
/// Fails if the time is outside the window, the key or signature is malformed
/// or does not verify, or the request was already used.
pub fn verify_signed(
    replays: &mut ReplayGuard,
    signed: &Signed,
    method: &str,
    target: &str,
    body: &[u8],
    now: u64,
) -> Result<String> {
    let refused = || {
        anyhow::Error::from(Refusal::Unauthenticated(
            "signature does not verify".to_owned(),
        ))
    };
    let window_ok = signed.at.abs_diff(now) <= SIGNED_WINDOW_MS;
    if !window_ok {
        return Err(Refusal::Unauthenticated(
            "signed request is outside its time window".to_owned(),
        )
        .into());
    }
    if signed.nonce.is_empty()
        || signed.nonce.len() > 64
        || !signed
            .nonce
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err(refused());
    }
    let key = normalize_key(&signed.key).ok().ok_or_else(refused)?;
    let raw: [u8; 32] = hex_bytes(&key).ok_or_else(refused)?;
    let verifying = VerifyingKey::from_bytes(&raw).ok().ok_or_else(refused)?;
    let raw_signature: [u8; 64] = hex_bytes(&signed.signature).ok_or_else(refused)?;
    let message = signed_message(method, target, signed.at, &signed.nonce, body);
    verifying
        .verify(message.as_bytes(), &Signature::from_bytes(&raw_signature))
        .ok()
        .ok_or_else(refused)?;
    replays.accept(&key, &signed.nonce, signed.at, now)?;
    Ok(key)
}

fn hex_bytes<const N: usize>(text: &str) -> Option<[u8; N]> {
    if text.len() != N * 2 || !text.is_ascii() {
        return None;
    }
    let mut out = [0_u8; N];
    for (slot, pair) in out.iter_mut().zip(text.as_bytes().chunks(2)) {
        let digits = std::str::from_utf8(pair).ok()?;
        *slot = u8::from_str_radix(digits, 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn now() -> u64 {
        1_000_000_000_000
    }

    #[test]
    fn a_session_lasts_its_ttl_and_closes_on_request() {
        let mut auth = DirectoryAuth::default();
        let token = auth.open_session(Principal::Admin, now());
        assert!(token.starts_with("sess."));
        assert_eq!(auth.principal(&token, now()), Some(Principal::Admin));
        assert_eq!(auth.principal(&token, now() + SESSION_TTL_MS), None);
        let again = auth.open_session(Principal::Account("ab".repeat(32)), now());
        auth.close_session(&again);
        assert_eq!(auth.principal(&again, now()), None);
    }

    #[test]
    fn a_code_is_used_once_and_only_by_its_purpose() {
        let mut auth = DirectoryAuth::default();
        let purpose = Purpose::SignIn {
            account: "ab".repeat(32),
        };
        let code = auth.issue_code(purpose.clone(), "a@b.co", now()).unwrap();
        assert_eq!(auth.check_code("a@b.co", &code, now()).unwrap(), purpose);
        assert!(auth.check_code("a@b.co", &code, now()).is_err());
    }

    #[test]
    fn a_code_is_refused_after_five_wrong_answers_and_after_expiry() {
        let mut auth = DirectoryAuth::default();
        let code = auth
            .issue_code(
                Purpose::Confirm {
                    account: "ab".repeat(32),
                },
                "a@b.co",
                now(),
            )
            .unwrap();
        let wrong = if code == "999999" { "000000" } else { "999999" };
        for _ in 0..CODE_ATTEMPTS {
            assert!(auth.check_code("a@b.co", wrong, now()).is_err());
        }
        assert!(auth.check_code("a@b.co", &code, now()).is_err());

        let code = auth
            .issue_code(
                Purpose::Confirm {
                    account: "ab".repeat(32),
                },
                "a@b.co",
                now() + CODE_RESEND_MS,
            )
            .unwrap();
        assert!(
            auth.check_code("a@b.co", &code, now() + CODE_RESEND_MS + CODE_TTL_MS)
                .is_err()
        );
    }

    #[test]
    fn a_code_cannot_be_asked_for_again_within_a_minute() {
        let mut auth = DirectoryAuth::default();
        let purpose = Purpose::SignIn {
            account: "ab".repeat(32),
        };
        auth.issue_code(purpose.clone(), "a@b.co", now()).unwrap();
        let error = auth.issue_code(purpose, "a@b.co", now() + 1).unwrap_err();
        assert!(matches!(
            error.downcast_ref::<Refusal>(),
            Some(Refusal::RateLimited(_))
        ));
    }

    #[test]
    fn a_signed_request_verifies_once_inside_its_window() {
        let key = SigningKey::from_bytes(&[7; 32]);
        let signed = sign_request(
            &key,
            "GET",
            "/api/world/directory?world=x",
            b"",
            now(),
            "n1",
        );
        let mut replays = ReplayGuard::default();
        let who = verify_signed(
            &mut replays,
            &signed,
            "GET",
            "/api/world/directory?world=x",
            b"",
            now(),
        )
        .unwrap();
        assert_eq!(who, signed.key);
        assert!(
            verify_signed(
                &mut replays,
                &signed,
                "GET",
                "/api/world/directory?world=x",
                b"",
                now()
            )
            .is_err()
        );
    }

    #[test]
    fn a_signed_request_is_bound_to_its_method_target_body_and_time() {
        let key = SigningKey::from_bytes(&[7; 32]);
        let signed = sign_request(&key, "POST", "/api/world/directory", b"{}", now(), "n2");
        for (method, target, body, at) in [
            ("GET", "/api/world/directory", b"{}".as_slice(), now()),
            ("POST", "/api/world/other", b"{}".as_slice(), now()),
            ("POST", "/api/world/directory", b"{ }".as_slice(), now()),
            (
                "POST",
                "/api/world/directory",
                b"{}".as_slice(),
                now() + SIGNED_WINDOW_MS + 1,
            ),
        ] {
            let mut replays = ReplayGuard::default();
            let mut moved = signed.clone();
            moved.at = at;
            assert!(verify_signed(&mut replays, &moved, method, target, body, now()).is_err());
        }
    }
}
