//! test-rp — a minimal relying party for the Slice 3 loop proof
//! (#13409, step 3). It completes the full authorization-code +
//! PKCE loop against a RUNNING idp-provider over real HTTP:
//!
//!   discovery -> JWKS -> POST /authorize (devtoday local door)
//!   -> redirect with code -> POST /token -> verify the ID token
//!   against the JWKS key -> GET /userinfo with the access token.
//!
//! Configuration (environment):
//!   RP_ISSUER        default http://127.0.0.1:8911
//!   RP_CLIENT_ID     default test-rp
//!   RP_REDIRECT_URI  default http://127.0.0.1:8912/callback
//!   RP_LOGIN_ID      the local-door identity to present
//!   RP_CREDENTIAL    its credential (never printed)
//!   RP_MODE          "owner" (default) expects the full loop to
//!                    succeed; "deny" expects /authorize to refuse
//!                    (403) — used for the non-allowlisted attempt.
//!
//! Prints one JSON object with the verdict and timings. The loop
//! latency reported (`loop_ms`) is measured inside this process from
//! the discovery fetch to the userinfo response, so it excludes
//! process startup.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64URL, Engine as _};
use idp_provider::{b64url, registry::random_b64url};
use rsa::pkcs1v15::{Signature, VerifyingKey};
use rsa::signature::Verifier;
use rsa::{BigUint, RsaPublicKey};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Instant;

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

fn url_encode(s: &str) -> String {
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

fn form_body(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", url_encode(k), url_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Minimal HTTP/1.1 client (loopback only): returns
/// (status, location header, body).
fn http(
    method: &str,
    url: &str,
    body: Option<&str>,
    bearer: Option<&str>,
) -> Result<(u16, Option<String>, String), String> {
    let rest = url
        .strip_prefix("http://")
        .ok_or("only http:// URLs are supported")?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let mut stream =
        TcpStream::connect(authority).map_err(|e| format!("connect {authority}: {e}"))?;
    let mut req = format!("{method} {path} HTTP/1.1\r\nhost: {authority}\r\nconnection: close\r\n");
    if let Some(b) = bearer {
        req.push_str(&format!("authorization: Bearer {b}\r\n"));
    }
    let payload = body.unwrap_or("");
    if body.is_some() {
        req.push_str("content-type: application/x-www-form-urlencoded\r\n");
        req.push_str(&format!("content-length: {}\r\n", payload.len()));
    }
    req.push_str("\r\n");
    req.push_str(payload);
    stream
        .write_all(req.as_bytes())
        .map_err(|e| format!("write request: {e}"))?;
    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .map_err(|e| format!("read response: {e}"))?;
    let raw = String::from_utf8_lossy(&raw).to_string();
    let (head, body) = raw
        .split_once("\r\n\r\n")
        .ok_or("malformed HTTP response")?;
    let mut lines = head.lines();
    let status: u16 = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .ok_or("no status line")?;
    let mut location = None;
    let mut chunked = false;
    for line in lines {
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("location:") {
            // The value is case-sensitive (it carries the code) —
            // slice it from the ORIGINAL line, not the lowercased one.
            location = Some(line["location:".len()..].trim().to_string());
        }
        if lower.starts_with("transfer-encoding:") && lower.contains("chunked") {
            chunked = true;
        }
    }
    let body = if chunked {
        dechunk(body)
    } else {
        body.to_string()
    };
    Ok((status, location, body))
}

fn dechunk(body: &str) -> String {
    let mut out = String::new();
    let mut rest = body;
    loop {
        let Some(nl) = rest.find("\r\n") else { break };
        let size = usize::from_str_radix(rest[..nl].trim(), 16).unwrap_or(0);
        if size == 0 {
            break;
        }
        let start = nl + 2;
        out.push_str(&rest[start..start + size]);
        rest = &rest[start + size + 2..];
    }
    out
}

fn main() {
    let started = Instant::now();
    let issuer = env_or("RP_ISSUER", "http://127.0.0.1:8911")
        .trim_end_matches('/')
        .to_string();
    let client_id = env_or("RP_CLIENT_ID", "test-rp");
    let redirect_uri = env_or("RP_REDIRECT_URI", "http://127.0.0.1:8912/callback");
    let login_id = env_or("RP_LOGIN_ID", "drew");
    let credential = std::env::var("RP_CREDENTIAL").unwrap_or_default();
    let mode = env_or("RP_MODE", "owner");

    let run = |fail: &str| -> ! {
        println!(
            "{}",
            serde_json::json!({"rp": "test-rp", "mode": mode, "ok": false, "error": fail})
        );
        std::process::exit(1);
    };

    // 1. Discovery.
    let (status, _l, body) = http(
        "GET",
        &format!("{issuer}/.well-known/openid-configuration"),
        None,
        None,
    )
    .unwrap_or_else(|e| run(&e));
    if status != 200 {
        run(&format!("discovery status {status}"));
    }
    let discovery: serde_json::Value =
        serde_json::from_str(&body).unwrap_or_else(|_| run("discovery is not JSON"));
    if discovery["issuer"] != issuer {
        run("discovery issuer mismatch");
    }
    let authz_ep = discovery["authorization_endpoint"].as_str().unwrap_or("");
    let token_ep = discovery["token_endpoint"].as_str().unwrap_or("");
    let userinfo_ep = discovery["userinfo_endpoint"].as_str().unwrap_or("");
    let jwks_uri = discovery["jwks_uri"].as_str().unwrap_or("");

    // 2. JWKS -> RSA public key.
    let (status, _l, body) = http("GET", jwks_uri, None, None).unwrap_or_else(|e| run(&e));
    if status != 200 {
        run(&format!("jwks status {status}"));
    }
    let jwks: serde_json::Value =
        serde_json::from_str(&body).unwrap_or_else(|_| run("jwks is not JSON"));
    let jwk = &jwks["keys"][0];
    let kid = jwk["kid"].as_str().unwrap_or("").to_string();
    let decode = |s: &str| B64URL.decode(s).unwrap_or_default();
    let pubkey = RsaPublicKey::new(
        BigUint::from_bytes_be(&decode(jwk["n"].as_str().unwrap_or(""))),
        BigUint::from_bytes_be(&decode(jwk["e"].as_str().unwrap_or(""))),
    )
    .unwrap_or_else(|_| run("JWKS key is not a valid RSA public key"));

    // 3. Authorize (local door) with PKCE.
    let verifier = random_b64url(32);
    let challenge = b64url(&Sha256::digest(verifier.as_bytes()));
    let form = form_body(&[
        ("response_type", "code"),
        ("client_id", &client_id),
        ("redirect_uri", &redirect_uri),
        ("scope", "openid profile"),
        ("state", "proof-state"),
        ("nonce", "proof-nonce"),
        ("code_challenge", &challenge),
        ("code_challenge_method", "S256"),
        ("login_id", &login_id),
        ("credential", &credential),
    ]);
    let (status, location, _body) =
        http("POST", authz_ep, Some(&form), None).unwrap_or_else(|e| run(&e));
    if mode == "deny" {
        // The proof's negative case: refusal is the success here.
        println!(
            "{}",
            serde_json::json!({
                "rp": "test-rp", "mode": "deny", "ok": status == 403,
                "authorize_status": status, "login_id": login_id,
                "loop_ms": started.elapsed().as_millis() as u64,
            })
        );
        std::process::exit(if status == 403 { 0 } else { 1 });
    }
    if status != 302 {
        run(&format!("authorize status {status} (expected 302)"));
    }
    let location = location.unwrap_or_else(|| run("authorize redirect has no Location"));
    let code = location
        .split("code=")
        .nth(1)
        .and_then(|s| s.split('&').next())
        .unwrap_or_else(|| run("redirect carries no code"))
        .to_string();
    if !location.contains("state=proof-state") {
        run("redirect state mismatch");
    }

    // 4. Token exchange.
    let form = form_body(&[
        ("grant_type", "authorization_code"),
        ("code", &code),
        ("redirect_uri", &redirect_uri),
        ("client_id", &client_id),
        ("code_verifier", &verifier),
    ]);
    let (status, _l, body) = http("POST", token_ep, Some(&form), None).unwrap_or_else(|e| run(&e));
    if status != 200 {
        run(&format!("token status {status}: {body}"));
    }
    let tokens: serde_json::Value =
        serde_json::from_str(&body).unwrap_or_else(|_| run("token response is not JSON"));
    let id_token = tokens["id_token"]
        .as_str()
        .unwrap_or_else(|| run("no id_token"));
    let access_token = tokens["access_token"]
        .as_str()
        .unwrap_or_else(|| run("no access_token"));

    // 5. Verify the ID token against the JWKS key.
    let parts: Vec<&str> = id_token.split('.').collect();
    if parts.len() != 3 {
        run("id_token is not a compact JWS");
    }
    let header: serde_json::Value =
        serde_json::from_slice(&decode(parts[0])).unwrap_or_else(|_| run("bad jwt header"));
    if header["alg"] != "RS256" || header["kid"] != kid {
        run("id_token header alg/kid does not match JWKS");
    }
    let signature = rsa::pkcs1v15::Signature::try_from(decode(parts[2]).as_slice())
        .unwrap_or_else(|_| run("bad signature encoding"));
    VerifyingKey::<Sha256>::new(pubkey)
        .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &signature)
        .unwrap_or_else(|_| run("ID TOKEN SIGNATURE VERIFICATION FAILED"));
    let claims: serde_json::Value =
        serde_json::from_slice(&decode(parts[1])).unwrap_or_else(|_| run("bad claims"));
    if claims["iss"] != issuer || claims["aud"] != client_id || claims["nonce"] != "proof-nonce" {
        run("id_token iss/aud/nonce mismatch");
    }

    // 6. Userinfo with the access token.
    let (status, _l, body) =
        http("GET", userinfo_ep, None, Some(access_token)).unwrap_or_else(|e| run(&e));
    if status != 200 {
        run(&format!("userinfo status {status}"));
    }
    let userinfo: serde_json::Value =
        serde_json::from_str(&body).unwrap_or_else(|_| run("userinfo is not JSON"));
    if userinfo["sub"] != claims["sub"] {
        run("userinfo sub does not match id_token sub");
    }

    println!(
        "{}",
        serde_json::json!({
            "rp": "test-rp", "mode": "owner", "ok": true,
            "kid": kid,
            "sub": claims["sub"],
            "roles": claims["roles"],
            "userinfo_roles": userinfo["roles"],
            "expires_in": tokens["expires_in"],
            "loop_ms": started.elapsed().as_millis() as u64,
        })
    );
}
