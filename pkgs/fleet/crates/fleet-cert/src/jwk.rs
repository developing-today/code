//! Strict public-key JWK conversion for bundle key material.
//!
//! Supported subset is intentionally public-only Ed25519 (`OKP`/`Ed25519`)
//! and uncompressed P-256 (`EC`/`P-256`). Private JWK parameters, RSA, ML-DSA,
//! unknown members, and non-canonical/lossy points are rejected rather than
//! silently discarded. Inputs and outputs use RFC 7517 JSON with RFC 4648
//! base64url without padding.

use crate::algorithm::AlgorithmId;
use crate::engine::inspect_key;
use crate::error::{CertError, Result};
use serde_json::{Value, json};

fn enc(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let a = chunk[0] as u32;
        let b = chunk.get(1).copied().unwrap_or(0) as u32;
        let c = chunk.get(2).copied().unwrap_or(0) as u32;
        out.push(TABLE[(a >> 2) as usize] as char);
        out.push(TABLE[(((a & 3) << 4) | (b >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[(((b & 15) << 2) | (c >> 6)) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(TABLE[(c & 63) as usize] as char);
        }
    }
    out
}

fn dec(text: &str) -> Result<Vec<u8>> {
    if text.contains('=')
        || text
            .bytes()
            .any(|b| !(b.is_ascii_alphanumeric() || b == b'-' || b == b'_'))
        || text.len() % 4 == 1
    {
        return Err(CertError::validation(
            "JWK member is not canonical base64url without padding",
        ));
    }
    let val = |b: u8| -> u8 {
        match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'-' => 62,
            _ => 63,
        }
    };
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    for chunk in bytes.chunks(4) {
        let a = val(chunk[0]) as u32;
        let b = val(chunk[1]) as u32;
        let c = chunk.get(2).map(|x| val(*x) as u32).unwrap_or(0);
        let d = chunk.get(3).map(|x| val(*x) as u32).unwrap_or(0);
        out.push(((a << 2) | (b >> 4)) as u8);
        if chunk.len() > 2 {
            out.push(((b << 4) | (c >> 2)) as u8);
        }
        if chunk.len() > 3 {
            out.push(((c << 6) | d) as u8);
        }
    }
    if enc(&out) != text {
        return Err(CertError::validation("non-canonical base64url encoding"));
    }
    Ok(out)
}

/// Export a private PKCS#8 key as a public-only JWK. Private bytes never
/// appear in the returned JSON.
pub fn public_from_pkcs8(algo: AlgorithmId, pkcs8: &[u8]) -> Result<Value> {
    let raw = inspect_key(algo, pkcs8)?;
    match algo {
        AlgorithmId::Ed25519 if raw.len() == 32 => {
            Ok(json!({"kty":"OKP","crv":"Ed25519","x":enc(&raw)}))
        }
        AlgorithmId::P256 if raw.len() == 65 && raw[0] == 4 => {
            Ok(json!({"kty":"EC","crv":"P-256","x":enc(&raw[1..33]),"y":enc(&raw[33..65])}))
        }
        AlgorithmId::Ed25519 | AlgorithmId::P256 => Err(CertError::validation(
            "public key has a non-canonical encoding",
        )),
        _ => Err(CertError::usage(
            "JWK conversion supports public Ed25519 and P-256 only; private JWK, RSA, and ML-DSA are unsupported",
        )),
    }
}

/// Validate a public-only supported JWK and return its canonical raw public key
/// bytes (Ed25519 32-byte key or SEC1 uncompressed P-256 point).
pub fn public_to_raw(value: &Value) -> Result<(AlgorithmId, Vec<u8>)> {
    let obj = value
        .as_object()
        .ok_or_else(|| CertError::validation("JWK must be a JSON object"))?;
    let string = |key: &str| {
        obj.get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| CertError::validation(format!("JWK requires string member {key}")))
    };
    let kty = string("kty")?;
    let crv = string("crv")?;
    let mut expected: Vec<&str> = match (kty, crv) {
        ("OKP", "Ed25519") => vec!["kty", "crv", "x"],
        ("EC", "P-256") => vec!["kty", "crv", "x", "y"],
        _ => {
            return Err(CertError::usage(
                "JWK supports only public Ed25519 (OKP) and P-256 (EC)",
            ));
        }
    };
    expected.sort_unstable();
    let mut actual: Vec<&str> = obj.keys().map(String::as_str).collect();
    actual.sort_unstable();
    if actual != expected {
        return Err(CertError::validation(
            "JWK has private, unknown, or missing members; only the exact public-key subset is accepted",
        ));
    }
    match (kty, crv) {
        ("OKP", "Ed25519") => {
            let x = dec(string("x")?)?;
            if x.len() != 32 {
                return Err(CertError::validation(
                    "Ed25519 JWK x must decode to exactly 32 bytes",
                ));
            }
            Ok((AlgorithmId::Ed25519, x))
        }
        ("EC", "P-256") => {
            let x = dec(string("x")?)?;
            let y = dec(string("y")?)?;
            if x.len() != 32 || y.len() != 32 {
                return Err(CertError::validation(
                    "P-256 JWK coordinates must each decode to exactly 32 bytes",
                ));
            }
            let mut point = Vec::with_capacity(65);
            point.push(4);
            point.extend(x);
            point.extend(y);
            Ok((AlgorithmId::P256, point))
        }
        _ => unreachable!(),
    }
}

/// Verify a JWK round-trip against the canonical public key extracted from PKCS#8.
pub fn verify_public_roundtrip(algo: AlgorithmId, pkcs8: &[u8], jwk: &Value) -> Result<()> {
    let expected = inspect_key(algo, pkcs8)?;
    let (got_algo, got) = public_to_raw(jwk)?;
    if got_algo != algo || got != expected {
        return Err(CertError::validation(
            "JWK public key does not match canonical DER key material",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::generate_key;

    #[test]
    fn ed25519_and_p256_public_jwk_round_trip_matches_der_key() {
        for algo in [AlgorithmId::Ed25519, AlgorithmId::P256] {
            let key = generate_key(algo).unwrap();
            let der = key.serialize_der();
            let jwk = public_from_pkcs8(algo, &der).unwrap();
            verify_public_roundtrip(algo, &der, &jwk).unwrap();
            assert_eq!(
                public_to_raw(&jwk).unwrap().1,
                inspect_key(algo, &der).unwrap()
            );
            assert!(!jwk.as_object().unwrap().contains_key("d"));
        }
    }

    #[test]
    fn rejects_private_unknown_lossy_and_unsupported_jwks() {
        let private = json!({"kty":"OKP","crv":"Ed25519","x":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","d":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"});
        assert!(public_to_raw(&private).is_err());
        let unknown = json!({"kty":"OKP","crv":"Ed25519","x":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","use":"sig"});
        assert!(public_to_raw(&unknown).is_err());
        let short = json!({"kty":"OKP","crv":"Ed25519","x":"AA"});
        assert!(public_to_raw(&short).is_err());
        let rsa = json!({"kty":"RSA","n":"AA","e":"AQAB"});
        assert!(public_to_raw(&rsa).is_err());
        assert!(dec("AA==").is_err());
        assert!(dec("AB").is_err());
    }
}
