//! Endpoint tests against the REAL Slice 2 signing certificate
//! published at pkgs/idp-pki/trust/oidc-signing.crt.pem. The expected
//! thumbprint and RSA key are derived from the cert file inside the
//! test — nothing about the key is hardcoded.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use base64::{engine::general_purpose::STANDARD as B64_STD, Engine as _};
use der::Decode;
use idp_provider::{app_from_config, b64url, Config};
use pkcs1::DecodeRsaPublicKey;
use rsa::{traits::PublicKeyParts, RsaPublicKey};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use tower::ServiceExt;
use x509_cert::Certificate;

fn cert_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../idp-pki/trust/oidc-signing.crt.pem")
}

fn cert_der() -> Vec<u8> {
    let pem = std::fs::read_to_string(cert_path()).expect("read signing cert PEM");
    let body: String = pem
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .collect::<Vec<_>>()
        .join("");
    B64_STD.decode(body.trim()).expect("decode cert DER")
}

fn test_config() -> Config {
    Config {
        issuer: "http://127.0.0.1:8911".to_string(),
        bind: "127.0.0.1:0".to_string(),
        oidc_cert_path: cert_path(),
        provider_config_path: None,
        signing_key_path: None,
        audit_db_path: None,
    }
}

async fn get_json(app: axum::Router, uri: &str) -> (StatusCode, serde_json::Value) {
    let resp = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn healthz_ok() {
    let app = app_from_config(test_config()).unwrap();
    let (status, body) = get_json(app, "/healthz").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok");
}

#[tokio::test]
async fn discovery_matches_configured_issuer() {
    let app = app_from_config(test_config()).unwrap();
    let (status, body) = get_json(app, "/.well-known/openid-configuration").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["issuer"], "http://127.0.0.1:8911");
    assert_eq!(
        body["authorization_endpoint"],
        "http://127.0.0.1:8911/authorize"
    );
    assert_eq!(body["token_endpoint"], "http://127.0.0.1:8911/token");
    assert_eq!(body["userinfo_endpoint"], "http://127.0.0.1:8911/userinfo");
    assert_eq!(body["jwks_uri"], "http://127.0.0.1:8911/jwks");
    assert_eq!(
        body["response_types_supported"],
        serde_json::json!(["code"])
    );
    assert_eq!(
        body["grant_types_supported"],
        serde_json::json!(["authorization_code"])
    );
    assert_eq!(
        body["code_challenge_methods_supported"],
        serde_json::json!(["S256"])
    );
    assert_eq!(
        body["subject_types_supported"],
        serde_json::json!(["public"])
    );
    assert_eq!(
        body["id_token_signing_alg_values_supported"],
        serde_json::json!(["RS256"])
    );
    assert_eq!(
        body["scopes_supported"],
        serde_json::json!(["openid", "profile"])
    );
}

#[tokio::test]
async fn jwks_matches_cert_thumbprint_and_key() {
    let der_bytes = cert_der();
    let expected_kid = b64url(&Sha256::digest(&der_bytes));
    let cert = Certificate::from_der(&der_bytes).unwrap();
    let spki = &cert.tbs_certificate.subject_public_key_info;
    let pubkey = RsaPublicKey::from_pkcs1_der(spki.subject_public_key.as_bytes().unwrap()).unwrap();
    let expected_n = b64url(&pubkey.n().to_bytes_be());
    let expected_e = b64url(&pubkey.e().to_bytes_be());

    let app = app_from_config(test_config()).unwrap();
    let (status, body) = get_json(app, "/jwks").await;
    assert_eq!(status, StatusCode::OK);
    let keys = body["keys"].as_array().unwrap();
    assert_eq!(keys.len(), 1);
    let k = &keys[0];
    assert_eq!(k["kty"], "RSA");
    assert_eq!(k["use"], "sig");
    assert_eq!(k["alg"], "RS256");
    assert_eq!(k["kid"], expected_kid);
    assert_eq!(k["x5t#S256"], expected_kid);
    assert_eq!(k["n"], expected_n);
    assert_eq!(k["e"], expected_e);
    // n/e must decode back to the cert's RSA public key components.
    let n_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(k["n"].as_str().unwrap())
        .unwrap();
    assert_eq!(n_bytes, pubkey.n().to_bytes_be());
    let e_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(k["e"].as_str().unwrap())
        .unwrap();
    assert_eq!(e_bytes, pubkey.e().to_bytes_be());
}
