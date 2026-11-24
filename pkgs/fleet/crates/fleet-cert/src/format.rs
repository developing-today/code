//! Certificate container conversion (spec §6.1 `format`): PEM and DER
//! for a single X.509 certificate. Input is parsed as an X.509
//! certificate before any output is produced, so a conversion never
//! emits bytes that are not a certificate.

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
            let mut certs: Vec<Vec<u8>> = pem_decode_all(text)?
                .into_iter()
                .filter(|(label, _)| label == CERT_LABEL)
                .map(|(_, der)| der)
                .collect();
            if certs.len() != 1 {
                return Err(CertError::validation(format!(
                    "expected exactly one {CERT_LABEL} block, found {}",
                    certs.len()
                )));
            }
            certs.swap_remove(0)
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
    fn parses_format_names() {
        assert_eq!(Encoding::parse("pem").expect("pem"), Encoding::Pem);
        assert!(Encoding::parse("p7c").is_err());
    }
}
