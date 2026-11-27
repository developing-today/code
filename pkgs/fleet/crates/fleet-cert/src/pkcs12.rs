//! PKCS#12 import/export for a single certificate and its matching key.
//!
//! Import/export exposes one identity per archive: the selected private key
//! and its matching leaf certificate. Additional certificate entries are
//! treated as the CA chain and preserved in order; an archive without that
//! pair, with a mismatched pair, or with the leaf repeated in the chain is
//! rejected. PKCS#12 permits broader SafeBag contents, but this API does not
//! expose unrelated key or non-certificate bags. Passwords are mandatory,
//! provided by the caller, and never stored in a bundle.

use crate::bundle::sha256_hex;
use crate::error::{CertError, Result};
use openssl::pkcs12::Pkcs12;
use openssl::pkey::PKey;
use openssl::stack::Stack;
use openssl::x509::X509;

/// Contents selected from a PKCS#12 archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Imported {
    /// DER-encoded X.509 certificate paired with the archive's private key.
    pub certificate_der: Vec<u8>,
    /// Unencrypted DER-encoded PKCS#8 private key. Callers must store it
    /// using the bundle's private-artifact permissions.
    pub private_key_pkcs8_der: Vec<u8>,
    /// Additional certificates, in archive order, excluding the paired leaf.
    pub chain_der: Vec<Vec<u8>>,
}

/// Export a single matching certificate and PKCS#8 key, with optional
/// additional certificates, as a password-protected PKCS#12 archive.
pub fn export(
    certificate_der: &[u8],
    private_key_pkcs8_der: &[u8],
    chain_der: &[Vec<u8>],
    password: &str,
) -> Result<Vec<u8>> {
    require_password(password)?;
    let certificate = X509::from_der(certificate_der)
        .map_err(|e| CertError::validation(format!("invalid certificate DER: {e}")))?;
    if certificate.to_der().map_err(engine_error)? != certificate_der {
        return Err(CertError::validation(
            "certificate DER has trailing or non-canonical bytes",
        ));
    }
    let key = PKey::private_key_from_pkcs8(private_key_pkcs8_der)
        .map_err(|e| CertError::validation(format!("invalid PKCS#8 private key: {e}")))?;
    if !certificate
        .public_key()
        .map_err(engine_error)?
        .public_eq(&key)
    {
        return Err(CertError::validation(
            "private key does not match the certificate public key",
        ));
    }

    let mut authorities = Stack::new().map_err(engine_error)?;
    for (index, der) in chain_der.iter().enumerate() {
        let ca = X509::from_der(der).map_err(|e| {
            CertError::validation(format!(
                "invalid chain certificate DER at index {index}: {e}"
            ))
        })?;
        if ca.to_der().map_err(engine_error)? != *der {
            return Err(CertError::validation(format!(
                "chain certificate DER at index {index} has trailing or non-canonical bytes"
            )));
        }
        if ca.to_der().map_err(engine_error)? == certificate_der {
            return Err(CertError::validation(
                "the leaf certificate must not also appear in the CA chain",
            ));
        }
        authorities
            .push(ca)
            .map_err(|_| CertError::validation("PKCS#12 CA chain is too large"))?;
    }

    let mut builder = Pkcs12::builder();
    builder
        .name("fleet-cert")
        .pkey(&key)
        .cert(&certificate)
        .ca(authorities);
    builder
        .build2(password)
        .and_then(|pfx| pfx.to_der())
        .map_err(engine_error)
}

/// Import the selected key/certificate pair and preserve its CA chain.
/// Archives without a key or matching leaf certificate are rejected.
pub fn import(bytes: &[u8], password: &str) -> Result<Imported> {
    require_password(password)?;
    let archive = Pkcs12::from_der(bytes)
        .map_err(|e| CertError::validation(format!("invalid PKCS#12 archive: {e}")))?;
    if archive.to_der().map_err(engine_error)? != bytes {
        return Err(CertError::validation(
            "PKCS#12 archive has trailing or non-canonical bytes",
        ));
    }
    let parsed = archive.parse2(password).map_err(|_| {
        CertError::validation("PKCS#12 password is invalid or archive is malformed")
    })?;
    let key = parsed
        .pkey
        .ok_or_else(|| CertError::validation("PKCS#12 archive contains no private key"))?;
    let certificate = parsed.cert.ok_or_else(|| {
        CertError::validation("PKCS#12 archive contains no matching leaf certificate")
    })?;
    if !certificate
        .public_key()
        .map_err(engine_error)?
        .public_eq(&key)
    {
        return Err(CertError::validation(
            "PKCS#12 private key does not match its leaf certificate",
        ));
    }
    let chain_der = parsed
        .ca
        .map(|chain| {
            chain
                .iter()
                .map(|cert| cert.to_der().map_err(engine_error))
                .collect::<Result<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_default();
    let certificate_der = certificate.to_der().map_err(engine_error)?;
    if chain_der.iter().any(|der| der == &certificate_der) {
        return Err(CertError::validation(
            "PKCS#12 archive ambiguously repeats the leaf in its CA chain",
        ));
    }
    let private_key_pkcs8_der = key.private_key_to_pkcs8().map_err(engine_error)?;
    Ok(Imported {
        certificate_der,
        private_key_pkcs8_der,
        chain_der,
    })
}

/// Metadata from verifying a PKCS#12 identity archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verified {
    /// SHA-256 fingerprint of the selected leaf certificate DER.
    pub certificate_sha256: String,
    /// Number of additional CA certificates in archive order.
    pub chain_certificates: usize,
}

/// Verify password, archive structure, selected identity pairing and chain.
/// Unlike [`import`], this never returns private-key bytes.
pub fn verify(bytes: &[u8], password: &str) -> Result<Verified> {
    let imported = import(bytes, password)?;
    Ok(Verified {
        certificate_sha256: sha256_hex(&imported.certificate_der),
        chain_certificates: imported.chain_der.len(),
    })
}

fn require_password(password: &str) -> Result<()> {
    if password.is_empty() {
        return Err(CertError::usage(
            "PKCS#12 requires a non-empty password; supply it through a protected password file",
        ));
    }
    Ok(())
}

fn engine_error(error: openssl::error::ErrorStack) -> CertError {
    CertError::engine(format!("OpenSSL PKCS#12 operation failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use openssl::hash::MessageDigest;
    use openssl::rsa::Rsa;
    use openssl::x509::X509NameBuilder;

    fn fixture() -> (Vec<u8>, Vec<u8>) {
        let key = PKey::from_rsa(Rsa::generate(2048).expect("RSA key")).expect("PKey");
        let mut name = X509NameBuilder::new().expect("name builder");
        name.append_entry_by_text("CN", "pkcs12.test").expect("CN");
        let name = name.build();
        let mut builder = X509::builder().expect("certificate builder");
        builder.set_version(2).expect("version");
        builder
            .set_serial_number(
                &openssl::asn1::Asn1Integer::from_bn(
                    &openssl::bn::BigNum::from_u32(1).expect("serial"),
                )
                .expect("serial integer"),
            )
            .expect("serial number");
        builder.set_subject_name(&name).expect("subject");
        builder.set_issuer_name(&name).expect("issuer");
        builder.set_pubkey(&key).expect("public key");
        builder
            .set_not_before(&openssl::asn1::Asn1Time::days_from_now(0).expect("not before"))
            .expect("not before");
        builder
            .set_not_after(&openssl::asn1::Asn1Time::days_from_now(1).expect("not after"))
            .expect("not after");
        builder.sign(&key, MessageDigest::sha256()).expect("sign");
        (
            builder.build().to_der().expect("certificate DER"),
            key.private_key_to_pkcs8().expect("key DER"),
        )
    }

    #[test]
    fn round_trip_preserves_certificate_key_and_multi_certificate_chain() {
        let (certificate, key) = fixture();
        let (chain_certificate, _) = fixture();
        let archive = export(
            &certificate,
            &key,
            &[chain_certificate.clone()],
            "correct horse battery staple",
        )
        .expect("export PKCS#12");
        let imported = import(&archive, "correct horse battery staple").expect("import PKCS#12");
        assert_eq!(imported.certificate_der, certificate);
        assert_eq!(imported.private_key_pkcs8_der, key);
        assert_eq!(imported.chain_der, [chain_certificate]);
        let verified = verify(&archive, "correct horse battery staple").expect("verify PKCS#12");
        assert_eq!(verified.chain_certificates, 1);
        assert_eq!(verified.certificate_sha256, sha256_hex(&certificate));
    }

    #[test]
    fn rejects_empty_or_wrong_password() {
        let (certificate, key) = fixture();
        assert!(export(&certificate, &key, &[], "").is_err());
        let archive = export(&certificate, &key, &[], "secret").expect("export");
        assert!(import(&archive, "wrong").is_err());
        assert!(verify(&archive, "").is_err());
    }

    #[test]
    fn rejects_invalid_archives_and_key_certificate_mismatch() {
        assert!(import(b"not a PKCS#12 archive", "secret").is_err());
        let (certificate, _) = fixture();
        let (_, other_key) = fixture();
        assert!(export(&certificate, &other_key, &[], "secret").is_err());
    }

    #[test]
    fn rejects_trailing_bytes() {
        let (certificate, key) = fixture();
        let mut archive = export(&certificate, &key, &[], "secret").expect("export");
        archive.push(0);
        assert!(import(&archive, "secret").is_err());
    }
}
