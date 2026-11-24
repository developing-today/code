//! SAML metadata for the minimal provider.
//!
//! SAML types and XML signature operations stay behind this module so the
//! provider's OIDC surface does not depend on the protocol library.

use axum::{
    extract::State,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use rsa::{pkcs1::EncodeRsaPrivateKey, RsaPrivateKey};
use samael::{
    crypto::{CertificateDer, Crypto, CryptoProvider},
    signature::{DigestAlgorithm, Signature},
    traits::ToXml,
};

const HTTP_POST: &str = "urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST";
const HTTP_REDIRECT: &str = "urn:oasis:names:tc:SAML:2.0:bindings:HTTP-Redirect";
const PERSISTENT_NAME_ID: &str = "urn:oasis:names:tc:SAML:2.0:nameid-format:persistent";

/// A validated key/certificate pair used to sign IdP metadata.
#[derive(Clone)]
pub struct MetadataSigner {
    issuer: String,
    private_key_der: Vec<u8>,
    certificate_der: Vec<u8>,
}

impl MetadataSigner {
    pub fn new(
        issuer: impl Into<String>,
        key: &RsaPrivateKey,
        certificate_der: Vec<u8>,
    ) -> Result<Self, String> {
        if certificate_der.is_empty() {
            return Err("SAML signing certificate is empty".into());
        }
        let private_key_der = key
            .to_pkcs1_der()
            .map_err(|e| format!("SAML RSA key encoding failed: {e}"))?
            .as_bytes()
            .to_vec();
        let certificate_pem = format!(
            "-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----\n",
            B64.encode(&certificate_der)
        );
        let public_key = crate::jwk_from_cert_pem(&certificate_pem)?;
        let (modulus, exponent) =
            crate::signing::TokenSigner::new(key.clone(), "saml".into()).public_components();
        if crate::b64url(&modulus) != public_key.n || crate::b64url(&exponent) != public_key.e {
            return Err("SAML signing key does not match its published certificate".into());
        }
        Ok(Self {
            issuer: issuer.into().trim_end_matches('/').to_string(),
            private_key_der,
            certificate_der,
        })
    }

    /// Build an IdP metadata document advertising the profile selected in
    /// the SAML design record and sign the root EntityDescriptor with XML-DSig.
    pub fn metadata_xml(&self) -> Result<String, String> {
        let issuer = xml_escape(&self.issuer);
        let cert = B64.encode(&self.certificate_der);
        let id = format!("_{}", crate::registry::random_b64url(18));
        let mut signature_template =
            Signature::template(&id, &CertificateDer::from(self.certificate_der.clone()));
        for reference in &mut signature_template.signed_info.reference {
            reference.digest_method.algorithm = DigestAlgorithm::Sha256;
        }
        let signature = signature_template
            .to_string()
            .map_err(|e| format!("SAML metadata signature template failed: {e:?}"))?;
        let unsigned = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
             <md:EntityDescriptor xmlns:md=\"urn:oasis:names:tc:SAML:2.0:metadata\" xmlns:ds=\"http://www.w3.org/2000/09/xmldsig#\" ID=\"{id}\" entityID=\"{issuer}\">\
               {signature}\
               <md:IDPSSODescriptor protocolSupportEnumeration=\"urn:oasis:names:tc:SAML:2.0:protocol\" WantAuthnRequestsSigned=\"false\">\
                 <md:KeyDescriptor use=\"signing\"><ds:KeyInfo><ds:X509Data><ds:X509Certificate>{cert}</ds:X509Certificate></ds:X509Data></ds:KeyInfo></md:KeyDescriptor>\
                 <md:NameIDFormat>{PERSISTENT_NAME_ID}</md:NameIDFormat>\
                 <md:SingleSignOnService Binding=\"{HTTP_REDIRECT}\" Location=\"{issuer}/saml/sso\"/>\
                 <md:SingleSignOnService Binding=\"{HTTP_POST}\" Location=\"{issuer}/saml/sso\"/>\
               </md:IDPSSODescriptor>\
             </md:EntityDescriptor>"
        );
        Crypto::sign_xml(unsigned.as_bytes(), &self.private_key_der)
            .map_err(|e| format!("SAML metadata signing failed: {e}"))
    }
}

pub async fn metadata(State(state): State<crate::AppState>) -> Response {
    let Some(signer) = state.saml_metadata else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "SAML signing is not configured",
        )
            .into_response();
    };
    match signer.metadata_xml() {
        Ok(xml) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/samlmetadata+xml")],
            xml,
        )
            .into_response(),
        Err(error) => {
            eprintln!("idp-provider: SAML metadata signing failed: {error}");
            (StatusCode::INTERNAL_SERVER_ERROR, "metadata unavailable").into_response()
        }
    }
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsa::pkcs1::DecodeRsaPrivateKey;
    use samael::{
        crypto::{Crypto, CryptoProvider},
        idp::{IdentityProvider, KeyType, Rsa},
    };
    use tower::ServiceExt;

    #[test]
    fn signed_metadata_advertises_the_configured_idp_profile() {
        // The oracle is samael's XML-DSig verifier: metadata must verify with
        // the matching test certificate and expose the settled bindings.
        let idp = IdentityProvider::generate_new(KeyType::Rsa(Rsa::Rsa2048)).unwrap();
        let cert = idp
            .create_certificate(&samael::idp::CertificateParams {
                common_name: "test-idp",
                issuer_name: "test-idp",
                days_until_expiration: 1,
            })
            .unwrap();
        let key = RsaPrivateKey::from_pkcs1_der(&idp.export_private_key_der().unwrap()).unwrap();
        let signer =
            MetadataSigner::new("https://idp.example", &key, cert.der_data().to_vec()).unwrap();
        let signed = signer.metadata_xml().unwrap();
        Crypto::verify_signed_xml(&signed, &cert, Some("ID")).unwrap();
        assert!(signed.contains(PERSISTENT_NAME_ID));
        assert!(signed.contains(HTTP_REDIRECT));
        assert!(signed.contains(HTTP_POST));
        assert!(signed.contains("https://idp.example/saml/sso"));
        assert!(signed.contains("http://www.w3.org/2001/04/xmlenc#sha256"));
        assert!(!signed.contains("http://www.w3.org/2000/09/xmldsig#sha1"));
    }

    #[test]
    fn metadata_escapes_the_issuer_before_signing() {
        let idp = IdentityProvider::generate_new(KeyType::Rsa(Rsa::Rsa2048)).unwrap();
        let cert = idp
            .create_certificate(&samael::idp::CertificateParams {
                common_name: "test-idp",
                issuer_name: "test-idp",
                days_until_expiration: 1,
            })
            .unwrap();
        let key = RsaPrivateKey::from_pkcs1_der(&idp.export_private_key_der().unwrap()).unwrap();
        let signer = MetadataSigner::new(
            "https://idp.example/?a=1&b=2",
            &key,
            cert.der_data().to_vec(),
        )
        .unwrap();
        let signed = signer.metadata_xml().unwrap();
        assert!(signed.contains("a=1&amp;b=2"));
        Crypto::verify_signed_xml(&signed, &cert, Some("ID")).unwrap();
    }

    #[tokio::test]
    async fn metadata_endpoint_returns_signed_metadata() {
        let idp = IdentityProvider::generate_new(KeyType::Rsa(Rsa::Rsa2048)).unwrap();
        let cert = idp
            .create_certificate(&samael::idp::CertificateParams {
                common_name: "test-idp",
                issuer_name: "test-idp",
                days_until_expiration: 1,
            })
            .unwrap();
        let key = RsaPrivateKey::from_pkcs1_der(&idp.export_private_key_der().unwrap()).unwrap();
        let cert_pem = format!(
            "-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----\n",
            B64.encode(cert.der_data())
        );
        let jwks = crate::jwks_from_cert_pem(&cert_pem).unwrap();
        let signer =
            MetadataSigner::new("https://idp.example", &key, cert.der_data().to_vec()).unwrap();
        let app = crate::build_app_with_saml_metadata(
            crate::Config {
                issuer: "https://idp.example".into(),
                bind: "127.0.0.1:0".into(),
                oidc_cert_path: Default::default(),
                provider_config_path: None,
                signing_key_path: None,
                oidc_cert_paths: vec![],
                signing_key_paths: vec![],
                oidc_active_kid: None,
                audit_db_path: None,
                identity_db_path: None,
            },
            jwks,
            None,
            Some(std::sync::Arc::new(signer)),
        );
        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/saml/metadata")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "application/samlmetadata+xml"
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let xml = String::from_utf8(body.to_vec()).unwrap();
        Crypto::verify_signed_xml(&xml, &cert, Some("ID")).unwrap();
        assert!(xml.contains("https://idp.example/saml/sso"));
    }
}
