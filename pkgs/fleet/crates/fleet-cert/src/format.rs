//! Certificate and PKCS#8 key container conversion (spec §6.1 `format`):
//! PEM and DER inputs are validated before any output is produced. Certificate
//! conversion remains X.509-only; private-key conversion requires an explicit
//! registry algorithm and preserves the exact PKCS#8 bytes.

use crate::algorithm::AlgorithmId;
use crate::bundle::{pem_decode_all, pem_encode, sha256_hex};
use crate::error::{CertError, Result};

pub const CERT_LABEL: &str = "CERTIFICATE";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Pem,
    Der,
}

impl Encoding {
    pub fn parse(input: &str) -> Result<Self> {
        match input {
            "pem" => Ok(Encoding::Pem),
            "der" => Ok(Encoding::Der),
            other => Err(CertError::usage(format!(
                "unknown format {other}; use pem or der"
            ))),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Encoding::Pem => "pem",
            Encoding::Der => "der",
        }
    }
}

/// PEM when the input starts with a PEM BEGIN line, DER otherwise.
pub fn detect(bytes: &[u8]) -> Encoding {
    let trimmed = bytes
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .map(|i| &bytes[i..])
        .unwrap_or(&[]);
    if trimmed.starts_with(b"-----BEGIN ") {
        Encoding::Pem
    } else {
        Encoding::Der
    }
}

/// Extract the one certificate DER from `bytes`, parse it as X.509
/// and reject trailing data.
pub fn certificate_der(bytes: &[u8]) -> Result<Vec<u8>> {
    let der = match detect(bytes) {
        Encoding::Der => bytes.to_vec(),
        Encoding::Pem => {
            let text = std::str::from_utf8(bytes)
                .map_err(|_| CertError::validation("PEM input is not UTF-8"))?;
            let trimmed = text.trim();
            let blocks = pem_decode_all(trimmed)?;
            if blocks.len() != 1 || blocks[0].0 != CERT_LABEL {
                return Err(CertError::validation(format!(
                    "expected exactly one {CERT_LABEL} block, found {} PEM blocks",
                    blocks.len()
                )));
            }
            let begin = format!("-----BEGIN {CERT_LABEL}-----");
            let end = format!("-----END {CERT_LABEL}-----");
            if !trimmed.starts_with(&begin) || !trimmed.ends_with(&end) {
                return Err(CertError::validation(
                    "unexpected text or PEM block outside the certificate",
                ));
            }
            blocks.into_iter().next().expect("one PEM block").1
        }
    };
    let (rest, _) = x509_parser::parse_x509_certificate(&der)
        .map_err(|e| CertError::validation(format!("not an X.509 certificate: {e:?}")))?;
    if !rest.is_empty() {
        return Err(CertError::validation(
            "trailing bytes after the certificate",
        ));
    }
    Ok(der)
}

/// Convert a certificate to `to`. The output is the same certificate
/// bytes in the other container.
pub fn convert(bytes: &[u8], to: Encoding) -> Result<Vec<u8>> {
    let der = certificate_der(bytes)?;
    Ok(match to {
        Encoding::Pem => pem_encode(CERT_LABEL, &der).into_bytes(),
        Encoding::Der => der,
    })
}

/// PKCS#8 private-key PEM label. Legacy algorithm-specific PEM labels and
/// encrypted private keys are intentionally not accepted by this path.
pub const PRIVATE_KEY_LABEL: &str = "PRIVATE KEY";

/// Extract and validate one PKCS#8 private key, returning its exact DER
/// bytes. The algorithm is explicit because a private-key container does not
/// by itself select fleet-cert's policy/registry identity.
pub fn private_key_der(bytes: &[u8], algorithm: AlgorithmId) -> Result<Vec<u8>> {
    let der = match detect(bytes) {
        Encoding::Der => bytes.to_vec(),
        Encoding::Pem => {
            let text = std::str::from_utf8(bytes)
                .map_err(|_| CertError::validation("private-key PEM input is not UTF-8"))?;
            let trimmed = text.trim();
            let blocks = pem_decode_all(trimmed)?;
            if blocks.len() != 1 || blocks[0].0 != PRIVATE_KEY_LABEL {
                return Err(CertError::validation(format!(
                    "expected exactly one {PRIVATE_KEY_LABEL} PKCS#8 block, found {} PEM blocks",
                    blocks.len()
                )));
            }
            let begin = format!("-----BEGIN {PRIVATE_KEY_LABEL}-----");
            let end = format!("-----END {PRIVATE_KEY_LABEL}-----");
            if !trimmed.starts_with(&begin) || !trimmed.ends_with(&end) {
                return Err(CertError::validation(
                    "unexpected text or PEM block outside the private key",
                ));
            }
            blocks.into_iter().next().expect("one PEM block").1
        }
    };

    // The outer DER sequence must consume the entire input. Key parsers are
    // not relied on to reject bytes after the PKCS#8 object.
    let (sequence, trailing) = der_tlv(&der, 0x30, "PKCS#8 sequence")?;
    if !trailing.is_empty() {
        return Err(CertError::validation(
            "trailing bytes after the PKCS#8 private key",
        ));
    }
    if sequence.is_empty() {
        return Err(CertError::validation("empty PKCS#8 private key"));
    }

    let public_key = crate::engine::inspect_key(algorithm, &der).map_err(|error| {
        CertError::validation(format!(
            "input is not a valid {} PKCS#8 private key: {error}",
            algorithm.name()
        ))
    })?;
    if matches!(algorithm, AlgorithmId::Rsa2048 | AlgorithmId::Rsa4096) {
        let actual_bits = rsa_public_key_bits(&public_key)?;
        let expected_bits = match algorithm {
            AlgorithmId::Rsa2048 => 2048,
            AlgorithmId::Rsa4096 => 4096,
            _ => unreachable!("RSA-only branch"),
        };
        if actual_bits != expected_bits {
            return Err(CertError::validation(format!(
                "RSA key size mismatch: algorithm {} requires {expected_bits} bits, found {actual_bits}",
                algorithm.name()
            )));
        }
    }
    Ok(der)
}

/// Convert one validated PKCS#8 private key to PEM or DER.
pub fn convert_private_key(bytes: &[u8], to: Encoding, algorithm: AlgorithmId) -> Result<Vec<u8>> {
    let der = private_key_der(bytes, algorithm)?;
    Ok(match to {
        Encoding::Pem => pem_encode(PRIVATE_KEY_LABEL, &der).into_bytes(),
        Encoding::Der => der,
    })
}

/// Summary of a verified private-key container. It deliberately contains no
/// key bytes or private-key digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedPrivateKey {
    pub input: Encoding,
    pub algorithm: AlgorithmId,
    pub der_len: usize,
}

/// Validate a private key and prove PEM and DER round trips preserve its exact
/// PKCS#8 bytes.
pub fn verify_private_key(bytes: &[u8], algorithm: AlgorithmId) -> Result<VerifiedPrivateKey> {
    let input = detect(bytes);
    let der = private_key_der(bytes, algorithm)?;
    let pem = convert_private_key(&der, Encoding::Pem, algorithm)?;
    if private_key_der(&pem, algorithm)? != der {
        return Err(CertError::validation(
            "PEM round trip changed the PKCS#8 private key",
        ));
    }
    if convert_private_key(&pem, Encoding::Der, algorithm)? != der {
        return Err(CertError::validation(
            "DER round trip changed the PKCS#8 private key",
        ));
    }
    Ok(VerifiedPrivateKey {
        input,
        algorithm,
        der_len: der.len(),
    })
}

/// Parse one DER TLV, rejecting indefinite, truncated, and non-minimal lengths.
fn der_tlv<'a>(bytes: &'a [u8], tag: u8, what: &str) -> Result<(&'a [u8], &'a [u8])> {
    if bytes.len() < 2 || bytes[0] != tag {
        return Err(CertError::validation(format!(
            "malformed {what}: expected DER tag 0x{tag:02x}"
        )));
    }
    let first = bytes[1];
    let (header_len, content_len) = if first & 0x80 == 0 {
        (2usize, first as usize)
    } else {
        let count = (first & 0x7f) as usize;
        if count == 0 || count > std::mem::size_of::<usize>() || bytes.len() < 2 + count {
            return Err(CertError::validation(format!(
                "malformed {what}: invalid DER length"
            )));
        }
        if bytes[2] == 0 {
            return Err(CertError::validation(format!(
                "malformed {what}: non-minimal DER length"
            )));
        }
        let mut length = 0usize;
        for byte in &bytes[2..2 + count] {
            length = length
                .checked_mul(256)
                .and_then(|value| value.checked_add(*byte as usize))
                .ok_or_else(|| {
                    CertError::validation(format!("malformed {what}: DER length overflow"))
                })?;
        }
        if length < 128 {
            return Err(CertError::validation(format!(
                "malformed {what}: non-minimal DER length"
            )));
        }
        (2 + count, length)
    };
    let end = header_len
        .checked_add(content_len)
        .ok_or_else(|| CertError::validation(format!("malformed {what}: DER length overflow")))?;
    if end > bytes.len() {
        return Err(CertError::validation(format!(
            "malformed {what}: truncated DER value"
        )));
    }
    Ok((&bytes[header_len..end], &bytes[end..]))
}

/// The engine returns RSA public keys as DER-encoded PKCS#1 RSAPublicKey.
/// Inspect only the public modulus to distinguish the registry's 2048- and
/// 4096-bit profiles; private material remains inside the validated parser.
fn rsa_public_key_bits(bytes: &[u8]) -> Result<usize> {
    let (sequence, trailing) = der_tlv(bytes, 0x30, "RSA public key")?;
    if !trailing.is_empty() {
        return Err(CertError::validation("trailing bytes after RSA public key"));
    }
    let (modulus, rest) = der_tlv(sequence, 0x02, "RSA modulus")?;
    let (_, rest) = der_tlv(rest, 0x02, "RSA exponent")?;
    if !rest.is_empty() || modulus.is_empty() || modulus[0] & 0x80 != 0 {
        return Err(CertError::validation("malformed RSA public key integers"));
    }
    let significant = if modulus.len() > 1 && modulus[0] == 0 {
        &modulus[1..]
    } else {
        modulus
    };
    let first = *significant
        .first()
        .ok_or_else(|| CertError::validation("empty RSA modulus"))?;
    if first == 0 {
        return Err(CertError::validation("non-canonical RSA modulus"));
    }
    Ok(significant.len() * 8 - first.leading_zeros() as usize)
}

/// Summary of a verified certificate container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verified {
    pub input: Encoding,
    pub der_len: usize,
    pub sha256: String,
}

/// Parse, then round-trip through both encodings and require each
/// to reproduce the same certificate DER.
pub fn verify(bytes: &[u8]) -> Result<Verified> {
    let input = detect(bytes);
    let der = certificate_der(bytes)?;
    let as_pem = convert(&der, Encoding::Pem)?;
    if certificate_der(&as_pem)? != der {
        return Err(CertError::validation(
            "PEM round trip changed the certificate",
        ));
    }
    if convert(&as_pem, Encoding::Der)? != der {
        return Err(CertError::validation(
            "DER round trip changed the certificate",
        ));
    }
    Ok(Verified {
        input,
        der_len: der.len(),
        sha256: sha256_hex(&der),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::algorithm::AlgorithmId;
    use crate::engine::generate_key;
    use rcgen::CertificateParams;

    fn self_signed_der(algo: AlgorithmId) -> Vec<u8> {
        let key = generate_key(algo).expect("key generation");
        let params = CertificateParams::new(vec!["format.test".to_string()]).expect("params");
        let cert = params.self_signed(&key).expect("self-signed");
        cert.der().to_vec()
    }

    #[test]
    fn round_trips_every_registered_algorithm() {
        for algo in [
            AlgorithmId::Ed25519,
            AlgorithmId::P256,
            AlgorithmId::Rsa2048,
            AlgorithmId::Rsa4096,
            AlgorithmId::MlDsa44,
        ] {
            let der = self_signed_der(algo);
            let pem = convert(&der, Encoding::Pem).expect("der to pem");
            assert!(
                pem.starts_with(b"-----BEGIN CERTIFICATE-----\n"),
                "{algo:?}"
            );
            assert_eq!(
                convert(&pem, Encoding::Der).expect("pem to der"),
                der,
                "{algo:?}"
            );
            let verified = verify(&der).expect("verify der");
            assert_eq!(verified.input, Encoding::Der);
            assert_eq!(verified.der_len, der.len());
        }
    }

    #[test]
    fn rejects_non_certificate_input() {
        assert!(certificate_der(b"not a certificate").is_err());
        let pem = pem_encode("PRIVATE KEY", b"\x30\x00");
        assert!(certificate_der(pem.trim_end().as_bytes()).is_err());
    }

    #[test]
    fn rejects_trailing_bytes_after_pem_certificate() {
        let der = self_signed_der(AlgorithmId::P256);
        let mut pem = String::from_utf8(convert(&der, Encoding::Pem).expect("pem")).expect("utf8");
        pem.push_str("-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n");
        assert!(certificate_der(pem.trim_end().as_bytes()).is_err());
    }

    #[test]
    fn rejects_other_pem_blocks_and_surrounding_text() {
        let der = self_signed_der(AlgorithmId::P256);
        let pem = String::from_utf8(convert(&der, Encoding::Pem).expect("pem")).expect("utf8");
        let key = pem_encode("PRIVATE KEY", b"key material");
        assert!(certificate_der(format!("{pem}{key}").as_bytes()).is_err());
        assert!(certificate_der(format!("ignored\n{pem}").as_bytes()).is_err());
        assert!(certificate_der(format!("{pem}ignored").as_bytes()).is_err());
    }

    #[test]
    fn parses_format_names() {
        assert_eq!(Encoding::parse("pem").expect("pem"), Encoding::Pem);
        assert!(Encoding::parse("p7c").is_err());
    }
}
