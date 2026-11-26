//! OIDC (e.g. Google) login as an account pointer (#13710 slice 3; design
//! of record: #5684 synthesis §5 and the #5885 feasibility record).
//!
//! The relying party runs the authorization-code flow with PKCE (S256),
//! then validates the ID Token: signature against the provider's JWKS
//! (RS256 or ES256 only), issuer, audience (and `azp` when there are
//! several audiences), expiry, issue time and the nonce it sent. A valid
//! token yields an [`AuthenticatedAccount`] for `(iss, sub)` — the account
//! pointer and nothing more. It never authorises an ID operation by itself
//! and cannot replace a binding (#5885 invariants 2 and 5); those need the
//! client-held ID key (see [`crate::login_binding`]).
//!
//! The network legs (redirect, token endpoint POST, JWKS fetch) stay with
//! the caller; this module builds and checks the values that cross them.

use std::fmt;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::RngExt as _;
use ring::signature::{self, RsaPublicKeyComponents, UnparsedPublicKey};
use serde_json::Value as Json;
use sha2::{Digest as _, Sha256};
use zeroize::Zeroizing;

use crate::json_unique::parse_json_without_duplicate_keys;
use crate::login_binding::{AuthAccount, AuthenticatedAccount, BindingError};

/// The PKCE code-challenge method this module uses.
pub const PKCE_METHOD: &str = "S256";

/// Largest ID Token accepted, in bytes.
pub const MAX_ID_TOKEN_BYTES: usize = 16 * 1024;

/// Typed OIDC login errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OidcError {
    /// The token is not a well-formed compact JWS.
    Malformed(&'static str),
    /// The header names an algorithm other than RS256 or ES256, or a
    /// critical extension.
    UnsupportedAlgorithm,
    /// No JWKS key matches the token's `kid` and algorithm.
    UnknownKey,
    /// The signature does not verify.
    BadSignature,
    /// A claim check failed.
    Claim(&'static str),
    /// The JWKS document is malformed.
    Jwks(&'static str),
    /// The resulting account pointer is invalid.
    Account(BindingError),
}

impl fmt::Display for OidcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(what) => write!(f, "malformed ID Token: {what}"),
            Self::UnsupportedAlgorithm => f.write_str("unsupported ID Token algorithm"),
            Self::UnknownKey => f.write_str("no matching provider key"),
            Self::BadSignature => f.write_str("ID Token signature does not verify"),
            Self::Claim(which) => write!(f, "ID Token claim `{which}` rejected"),
            Self::Jwks(what) => write!(f, "malformed JWKS: {what}"),
            Self::Account(err) => write!(f, "invalid account: {err}"),
        }
    }
}

impl std::error::Error for OidcError {}

/// A random, URL-safe token for `state` and `nonce` (32 random bytes).
pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// A PKCE code verifier, kept by the client until the token exchange.
pub struct Pkce {
    verifier: Zeroizing<String>,
}

impl fmt::Debug for Pkce {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Pkce { .. }")
    }
}

impl Pkce {
    /// A fresh verifier (43 characters from 32 random bytes).
    pub fn generate() -> Self {
        Self {
            verifier: Zeroizing::new(random_token()),
        }
    }

    /// The verifier, sent only in the token request.
    pub fn verifier(&self) -> &str {
        &self.verifier
    }

    /// `BASE64URL(SHA-256(verifier))`, sent in the authorization request.
    pub fn challenge(&self) -> String {
        URL_SAFE_NO_PAD.encode(Sha256::digest(self.verifier.as_bytes()))
    }
}

/// The parameters of an authorization-code + PKCE request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizationRequest {
    /// The provider's authorization endpoint.
    pub endpoint: String,
    /// This relying party's client id.
    pub client_id: String,
    /// The registered redirect URI.
    pub redirect_uri: String,
    /// CSRF `state`, checked on the redirect.
    pub state: String,
    /// The `nonce` the ID Token must carry back.
    pub nonce: String,
    /// The PKCE code challenge.
    pub code_challenge: String,
}

impl AuthorizationRequest {
    /// A request with fresh `state` and `nonce` for `pkce`.
    pub fn new(endpoint: &str, client_id: &str, redirect_uri: &str, pkce: &Pkce) -> Self {
        Self {
            endpoint: endpoint.to_owned(),
            client_id: client_id.to_owned(),
            redirect_uri: redirect_uri.to_owned(),
            state: random_token(),
            nonce: random_token(),
            code_challenge: pkce.challenge(),
        }
    }

    /// The URL to send the user agent to.
    pub fn url(&self) -> String {
        let params = [
            ("response_type", "code"),
            ("client_id", self.client_id.as_str()),
            ("redirect_uri", self.redirect_uri.as_str()),
            ("scope", "openid"),
            ("state", self.state.as_str()),
            ("nonce", self.nonce.as_str()),
            ("code_challenge", self.code_challenge.as_str()),
            ("code_challenge_method", PKCE_METHOD),
        ];
        let separator = if self.endpoint.contains('?') {
            '&'
        } else {
            '?'
        };
        let mut url = self.endpoint.clone();
        url.push(separator);
        url.push_str(&form_encode(&params));
        url
    }
}

/// The `application/x-www-form-urlencoded` body of the token request.
pub fn token_request_body(code: &str, redirect_uri: &str, client_id: &str, pkce: &Pkce) -> String {
    form_encode(&[
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("client_id", client_id),
        ("code_verifier", pkce.verifier()),
    ])
}

/// One provider signing key.
#[derive(Debug, Clone, PartialEq, Eq)]
enum KeyMaterial {
    /// RSA public key: big-endian modulus and exponent.
    Rsa { n: Vec<u8>, e: Vec<u8> },
    /// P-256 public key as an uncompressed SEC1 point.
    P256 { point: Vec<u8> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Jwk {
    kid: String,
    material: KeyMaterial,
}

/// A provider's JSON Web Key Set, restricted to the RSA and P-256 keys
/// this module verifies with.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Jwks {
    keys: Vec<Jwk>,
}

impl Jwks {
    /// Parses a JWKS document. Keys of other types, keys without a
    /// `kid`, and keys marked for encryption (`use: enc`) are skipped.
    ///
    /// # Errors
    ///
    /// [`OidcError::Jwks`] when the document is not a key set or a
    /// supported key is malformed.
    pub fn parse(json: &str) -> Result<Self, OidcError> {
        let doc =
            parse_json_without_duplicate_keys(json).map_err(|_err| OidcError::Jwks("not JSON"))?;
        let keys = doc
            .get("keys")
            .and_then(Json::as_array)
            .ok_or(OidcError::Jwks("no keys array"))?;
        let mut out = Vec::new();
        for key in keys {
            let Some(kid) = key.get("kid").and_then(Json::as_str) else {
                continue;
            };
            if key.get("use").and_then(Json::as_str) == Some("enc") {
                continue;
            }
            let material = match key.get("kty").and_then(Json::as_str) {
                Some("RSA") => KeyMaterial::Rsa {
                    n: b64_field(key, "n")?,
                    e: b64_field(key, "e")?,
                },
                Some("EC") if key.get("crv").and_then(Json::as_str) == Some("P-256") => {
                    let x = b64_field(key, "x")?;
                    let y = b64_field(key, "y")?;
                    if x.len() != 32 || y.len() != 32 {
                        return Err(OidcError::Jwks("P-256 coordinate is not 32 bytes"));
                    }
                    let mut point = Vec::with_capacity(65);
                    point.push(0x04);
                    point.extend_from_slice(&x);
                    point.extend_from_slice(&y);
                    KeyMaterial::P256 { point }
                }
                _ => continue,
            };
            out.push(Jwk {
                kid: kid.to_owned(),
                material,
            });
        }
        Ok(Self { keys: out })
    }

    /// How many usable keys the set holds.
    pub const fn len(&self) -> usize {
        self.keys.len()
    }

    /// Whether the set holds no usable key.
    pub const fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    fn find(&self, kid: &str, alg: Alg) -> Option<&KeyMaterial> {
        self.keys
            .iter()
            .filter(|key| key.kid == kid)
            .map(|key| &key.material)
            .find(|material| {
                matches!(
                    (alg, material),
                    (Alg::Rs256, KeyMaterial::Rsa { .. }) | (Alg::Es256, KeyMaterial::P256 { .. })
                )
            })
    }
}

/// What a relying party requires of an ID Token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdTokenPolicy {
    /// The exact expected `iss`.
    pub issuer: String,
    /// This relying party's client id, which `aud` must contain.
    pub client_id: String,
    /// Allowed clock skew, seconds.
    pub leeway_secs: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Alg {
    Rs256,
    Es256,
}

/// Validates an ID Token and returns the authenticated account pointer.
///
/// # Errors
///
/// [`OidcError`] for the first failed check. Every check runs before
/// anything is returned; there is no partial acceptance.
pub fn validate_id_token(
    token: &str,
    jwks: &Jwks,
    policy: &IdTokenPolicy,
    expected_nonce: &str,
    now: u64,
) -> Result<AuthenticatedAccount, OidcError> {
    if token.len() > MAX_ID_TOKEN_BYTES {
        return Err(OidcError::Malformed("too large"));
    }
    let mut parts = token.split('.');
    let (Some(header_b64), Some(claims_b64), Some(sig_b64), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(OidcError::Malformed("not three segments"));
    };
    let header = decode_json(header_b64, "header")?;
    if header.get("crit").is_some() {
        return Err(OidcError::UnsupportedAlgorithm);
    }
    let alg = match header.get("alg").and_then(Json::as_str) {
        Some("RS256") => Alg::Rs256,
        Some("ES256") => Alg::Es256,
        _ => return Err(OidcError::UnsupportedAlgorithm),
    };
    let kid = header
        .get("kid")
        .and_then(Json::as_str)
        .ok_or(OidcError::UnknownKey)?;
    let key = jwks.find(kid, alg).ok_or(OidcError::UnknownKey)?;
    let signature = URL_SAFE_NO_PAD
        .decode(sig_b64)
        .map_err(|_err| OidcError::Malformed("signature encoding"))?;
    let signing_input = &token.as_bytes()[..header_b64.len() + 1 + claims_b64.len()];
    let verified = match key {
        KeyMaterial::Rsa { n, e } => RsaPublicKeyComponents { n, e }
            .verify(
                &signature::RSA_PKCS1_2048_8192_SHA256,
                signing_input,
                &signature,
            )
            .is_ok(),
        KeyMaterial::P256 { point } => {
            UnparsedPublicKey::new(&signature::ECDSA_P256_SHA256_FIXED, point)
                .verify(signing_input, &signature)
                .is_ok()
        }
    };
    if !verified {
        return Err(OidcError::BadSignature);
    }

    let claims = decode_json(claims_b64, "claims")?;
    let issuer = claims
        .get("iss")
        .and_then(Json::as_str)
        .ok_or(OidcError::Claim("iss"))?;
    if issuer != policy.issuer {
        return Err(OidcError::Claim("iss"));
    }
    check_audience(&claims, &policy.client_id)?;
    let exp = claims
        .get("exp")
        .and_then(Json::as_u64)
        .ok_or(OidcError::Claim("exp"))?;
    if exp.saturating_add(policy.leeway_secs) <= now {
        return Err(OidcError::Claim("exp"));
    }
    let iat = claims
        .get("iat")
        .and_then(Json::as_u64)
        .ok_or(OidcError::Claim("iat"))?;
    if iat > now.saturating_add(policy.leeway_secs) {
        return Err(OidcError::Claim("iat"));
    }
    let nonce = claims
        .get("nonce")
        .and_then(Json::as_str)
        .ok_or(OidcError::Claim("nonce"))?;
    if !constant_time_eq(nonce.as_bytes(), expected_nonce.as_bytes()) {
        return Err(OidcError::Claim("nonce"));
    }
    let subject = claims
        .get("sub")
        .and_then(Json::as_str)
        .ok_or(OidcError::Claim("sub"))?;
    let account = AuthAccount::oidc(issuer, subject).map_err(OidcError::Account)?;
    Ok(AuthenticatedAccount::from_verified_login(account))
}

fn check_audience(claims: &Json, client_id: &str) -> Result<(), OidcError> {
    match claims.get("aud") {
        Some(Json::String(aud)) if aud == client_id => Ok(()),
        Some(Json::Array(auds)) => {
            if !auds.iter().any(|aud| aud.as_str() == Some(client_id)) {
                return Err(OidcError::Claim("aud"));
            }
            if auds.len() > 1 && claims.get("azp").and_then(Json::as_str) != Some(client_id) {
                return Err(OidcError::Claim("azp"));
            }
            Ok(())
        }
        _ => Err(OidcError::Claim("aud")),
    }
}

fn decode_json(segment: &str, what: &'static str) -> Result<Json, OidcError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(segment)
        .map_err(|_err| OidcError::Malformed(what))?;
    // Duplicate keys are refused: a repeated claim such as `sub` or
    // `aud` would let two readers of the same signed bytes disagree.
    let text = std::str::from_utf8(&bytes).map_err(|_err| OidcError::Malformed(what))?;
    let value =
        parse_json_without_duplicate_keys(text).map_err(|_err| OidcError::Malformed(what))?;
    if value.is_object() {
        Ok(value)
    } else {
        Err(OidcError::Malformed(what))
    }
}

fn b64_field(key: &Json, field: &'static str) -> Result<Vec<u8>, OidcError> {
    let text = key
        .get(field)
        .and_then(Json::as_str)
        .ok_or(OidcError::Jwks(field))?;
    URL_SAFE_NO_PAD
        .decode(text)
        .map_err(|_err| OidcError::Jwks(field))
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn form_encode(params: &[(&str, &str)]) -> String {
    let mut out = String::new();
    for (index, (key, value)) in params.iter().enumerate() {
        if index > 0 {
            out.push('&');
        }
        percent_encode(key, &mut out);
        out.push('=');
        percent_encode(value, &mut out);
    }
    out
}

fn percent_encode(text: &str, out: &mut String) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push('%');
            out.push(char::from(HEX[usize::from(byte >> 4)]));
            out.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use ring::rand::SystemRandom;
    use ring::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair as _};

    use super::*;

    const ISSUER: &str = "https://accounts.google.com";
    const CLIENT: &str = "test-client.apps.example";
    const NOW: u64 = 1_800_000_100;

    // RS256 fixture: one token signed offline with a throwaway 2048-bit
    // key whose private half was destroyed; only the public modulus is
    // kept. Claims: iss/aud/azp as above, sub 110169484474386276334,
    // nonce "fixture-nonce", iat 1800000000, exp 1800003600.
    const RS256_TOKEN: &str = include_str!("../test-data/oidc_rs256_fixture.jwt");
    const RS256_N: &str = include_str!("../test-data/oidc_rs256_fixture.n");

    fn policy() -> IdTokenPolicy {
        IdTokenPolicy {
            issuer: ISSUER.to_owned(),
            client_id: CLIENT.to_owned(),
            leeway_secs: 60,
        }
    }

    fn rsa_jwks() -> Jwks {
        Jwks::parse(&format!(
            r#"{{"keys":[{{"kty":"RSA","kid":"test-rsa-1","use":"sig","alg":"RS256","n":"{}","e":"AQAB"}}]}}"#,
            RS256_N.trim()
        ))
        .unwrap()
    }

    struct EcIssuer {
        pair: EcdsaKeyPair,
        rng: SystemRandom,
    }

    impl EcIssuer {
        fn new() -> Self {
            let rng = SystemRandom::new();
            let pkcs8 =
                EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
            let pair =
                EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng)
                    .unwrap();
            Self { pair, rng }
        }

        fn jwks(&self) -> Jwks {
            let point = self.pair.public_key().as_ref();
            Jwks::parse(&format!(
                r#"{{"keys":[{{"kty":"EC","crv":"P-256","kid":"ec-1","x":"{}","y":"{}"}}]}}"#,
                URL_SAFE_NO_PAD.encode(&point[1..33]),
                URL_SAFE_NO_PAD.encode(&point[33..65]),
            ))
            .unwrap()
        }

        fn sign(&self, header: &Json, claims: &Json) -> String {
            let input = format!(
                "{}.{}",
                URL_SAFE_NO_PAD.encode(header.to_string()),
                URL_SAFE_NO_PAD.encode(claims.to_string())
            );
            let sig = self.pair.sign(&self.rng, input.as_bytes()).unwrap();
            format!("{input}.{}", URL_SAFE_NO_PAD.encode(sig.as_ref()))
        }
    }

    fn header() -> Json {
        serde_json::json!({"alg": "ES256", "kid": "ec-1", "typ": "JWT"})
    }

    fn claims() -> Json {
        serde_json::json!({
            "iss": ISSUER,
            "aud": CLIENT,
            "sub": "1234",
            "nonce": "n-1",
            "iat": NOW - 10,
            "exp": NOW + 3600,
        })
    }

    #[test]
    fn test_rs256_fixture_validates_to_account_pointer() {
        let session = validate_id_token(
            RS256_TOKEN.trim(),
            &rsa_jwks(),
            &policy(),
            "fixture-nonce",
            NOW,
        )
        .unwrap();
        assert_eq!(
            session.account(),
            &AuthAccount::oidc(ISSUER, "110169484474386276334").unwrap()
        );
    }

    #[test]
    fn test_rs256_tampered_claims_fail_signature() {
        let token = RS256_TOKEN.trim();
        let mut parts: Vec<&str> = token.split('.').collect();
        let forged_claims = URL_SAFE_NO_PAD.encode(
            serde_json::json!({
                "aud": CLIENT, "exp": 1_800_003_600u64, "iat": 1_800_000_000u64,
                "iss": ISSUER, "nonce": "fixture-nonce", "sub": "attacker"
            })
            .to_string(),
        );
        parts[1] = &forged_claims;
        let forged = parts.join(".");
        assert_eq!(
            validate_id_token(&forged, &rsa_jwks(), &policy(), "fixture-nonce", NOW).map(|_| ()),
            Err(OidcError::BadSignature)
        );
    }

    #[test]
    fn test_es256_token_validates() {
        let issuer = EcIssuer::new();
        let token = issuer.sign(&header(), &claims());
        let session = validate_id_token(&token, &issuer.jwks(), &policy(), "n-1", NOW).unwrap();
        assert_eq!(
            session.account(),
            &AuthAccount::oidc(ISSUER, "1234").unwrap()
        );
    }

    #[test]
    fn test_claim_checks_reject() {
        let issuer = EcIssuer::new();
        let jwks = issuer.jwks();
        let check = |claims: Json, nonce: &str| {
            validate_id_token(
                &issuer.sign(&header(), &claims),
                &jwks,
                &policy(),
                nonce,
                NOW,
            )
            .map(|_| ())
        };
        assert_eq!(check(claims(), "other"), Err(OidcError::Claim("nonce")));

        let mut c = claims();
        c["iss"] = Json::from("https://evil.example");
        assert_eq!(check(c, "n-1"), Err(OidcError::Claim("iss")));

        let mut c = claims();
        c["aud"] = Json::from("someone-else");
        assert_eq!(check(c, "n-1"), Err(OidcError::Claim("aud")));

        let mut c = claims();
        c["aud"] = serde_json::json!([CLIENT, "someone-else"]);
        assert_eq!(check(c.clone(), "n-1"), Err(OidcError::Claim("azp")));
        c["azp"] = Json::from(CLIENT);
        assert_eq!(check(c, "n-1"), Ok(()));

        let mut c = claims();
        c["exp"] = Json::from(NOW - 61);
        assert_eq!(check(c, "n-1"), Err(OidcError::Claim("exp")));

        let mut c = claims();
        c["iat"] = Json::from(NOW + 120);
        assert_eq!(check(c, "n-1"), Err(OidcError::Claim("iat")));

        let mut c = claims();
        c.as_object_mut().unwrap().remove("nonce");
        assert_eq!(check(c, "n-1"), Err(OidcError::Claim("nonce")));
    }

    #[test]
    fn test_header_checks_reject() {
        let issuer = EcIssuer::new();
        let jwks = issuer.jwks();
        let run = |header: Json| {
            validate_id_token(
                &issuer.sign(&header, &claims()),
                &jwks,
                &policy(),
                "n-1",
                NOW,
            )
            .map(|_| ())
        };
        assert_eq!(
            run(serde_json::json!({"alg": "none", "kid": "ec-1"})),
            Err(OidcError::UnsupportedAlgorithm)
        );
        assert_eq!(
            run(serde_json::json!({"alg": "HS256", "kid": "ec-1"})),
            Err(OidcError::UnsupportedAlgorithm)
        );
        assert_eq!(
            run(serde_json::json!({"alg": "ES256", "kid": "ec-1", "crit": ["x"]})),
            Err(OidcError::UnsupportedAlgorithm)
        );
        assert_eq!(
            run(serde_json::json!({"alg": "ES256", "kid": "missing"})),
            Err(OidcError::UnknownKey)
        );
        // An RS256 header cannot select the EC key.
        assert_eq!(
            run(serde_json::json!({"alg": "RS256", "kid": "ec-1"})),
            Err(OidcError::UnknownKey)
        );
        // A different issuer's key does not verify.
        let other = EcIssuer::new();
        assert_eq!(
            validate_id_token(
                &other.sign(&header(), &claims()),
                &jwks,
                &policy(),
                "n-1",
                NOW
            )
            .map(|_| ()),
            Err(OidcError::BadSignature)
        );
        assert_eq!(
            validate_id_token("a.b", &jwks, &policy(), "n-1", NOW).map(|_| ()),
            Err(OidcError::Malformed("not three segments"))
        );
    }

    #[test]
    fn test_duplicate_claim_keys_are_refused() {
        let issuer = EcIssuer::new();
        let header_b64 = URL_SAFE_NO_PAD.encode(header().to_string());
        let claims_text = format!(
            r#"{{"iss":"{ISSUER}","aud":"{CLIENT}","sub":"1234","sub":"attacker","nonce":"n-1","iat":{},"exp":{}}}"#,
            NOW - 10,
            NOW + 3600
        );
        let input = format!("{header_b64}.{}", URL_SAFE_NO_PAD.encode(claims_text));
        let sig = issuer.pair.sign(&issuer.rng, input.as_bytes()).unwrap();
        let token = format!("{input}.{}", URL_SAFE_NO_PAD.encode(sig.as_ref()));
        assert_eq!(
            validate_id_token(&token, &issuer.jwks(), &policy(), "n-1", NOW).map(|_| ()),
            Err(OidcError::Malformed("claims"))
        );
    }

    #[test]
    fn test_pkce_and_authorization_url() {
        let pkce = Pkce::generate();
        assert_eq!(pkce.verifier().len(), 43);
        assert_eq!(
            pkce.challenge(),
            URL_SAFE_NO_PAD.encode(Sha256::digest(pkce.verifier().as_bytes()))
        );
        let request = AuthorizationRequest::new(
            "https://accounts.google.com/o/oauth2/v2/auth",
            CLIENT,
            "https://id.example/cb?x=1",
            &pkce,
        );
        let url = request.url();
        assert!(
            url.starts_with("https://accounts.google.com/o/oauth2/v2/auth?response_type=code&")
        );
        assert!(url.contains("redirect_uri=https%3A%2F%2Fid.example%2Fcb%3Fx%3D1"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains(&format!("nonce={}", request.nonce)));
        assert!(!url.contains(pkce.verifier()));
        let body = token_request_body("abc", "https://id.example/cb", CLIENT, &pkce);
        assert!(body.contains(&format!("code_verifier={}", pkce.verifier())));
        assert_ne!(request.state, request.nonce);
    }

    #[test]
    fn invariant_2_oidc_login_alone_cannot_rebind_a_key() {
        use crate::login_binding::{
            BindingRegistry, BindingStatement, CustodyProfile, SignedBinding,
        };
        use ed25519_dalek::SigningKey;

        let issuer = EcIssuer::new();
        let jwks = issuer.jwks();
        let session = validate_id_token(
            &issuer.sign(&header(), &claims()),
            &jwks,
            &policy(),
            "n-1",
            NOW,
        )
        .unwrap();
        let held = SigningKey::from_bytes(&[21; 32]);
        let statement = BindingStatement {
            id_public_key: held.verifying_key().to_bytes(),
            account: session.account().clone(),
            custody: CustodyProfile::UserHeld,
            recovery_commitment: [1; 32],
            generation: 1,
            prev: None,
        };
        let mut registry = BindingRegistry::new();
        registry
            .enrol(
                &session,
                SignedBinding::enrol(statement.clone(), &held).unwrap(),
            )
            .unwrap();

        // A second, equally valid Google login with a new key is refused.
        let again = validate_id_token(
            &issuer.sign(&header(), &claims()),
            &jwks,
            &policy(),
            "n-1",
            NOW,
        )
        .unwrap();
        let other = SigningKey::from_bytes(&[22; 32]);
        let rebind = BindingStatement {
            id_public_key: other.verifying_key().to_bytes(),
            ..statement
        };
        assert_eq!(
            registry
                .enrol(&again, SignedBinding::enrol(rebind, &other).unwrap())
                .map(|_| ()),
            Err(BindingError::AlreadyBound)
        );
    }
}
