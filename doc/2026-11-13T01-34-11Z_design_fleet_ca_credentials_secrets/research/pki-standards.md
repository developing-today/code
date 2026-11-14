# PKI standards and Rust implementation evidence

**Retrieved:** 2026-11-12. This note records primary-source retrieval performed for the Fleet CA credentials/secrets v0.3 design. It is evidence about standards and library surfaces, not a claim that the repository has already integrated these libraries.

## Standards facts

### X.509 certificates, paths, and revocation

RFC 5280 profiles X.509 v3 certificates and v2 CRLs, defines certificate extensions and a path-validation algorithm, and makes clear that an implementation need not use that exact algorithm so long as it derives the same results. A certificate binds a subject public key to a subject by the issuer's signature; path trust begins at an assured trust-anchor key, not from arbitrary certificates in a presented bundle. [RFC 5280, §§3.1–3.3, 4, 6](https://www.rfc-editor.org/rfc/rfc5280)

Relevant profile details for this design:

* A certificate serial is positive, unique per CA, no more than 20 octets; validity and signature algorithm identifiers are signed fields. [RFC 5280, §§4.1.1–4.1.2](https://www.rfc-editor.org/rfc/rfc5280#section-4.1)
* CA certificates require Basic Constraints and key usage appropriate to certificate signing; leaf key usage/EKU and SAN are policy choices applied by the issuer. AIA and CRL Distribution Points are the standard access-location extensions. [RFC 5280, §§4.2.1.3, 4.2.1.6, 4.2.1.9, 4.2.1.12–13, 4.2.2.1](https://www.rfc-editor.org/rfc/rfc5280#section-4.2)
* Cross-certificates are CA certificates with different issuer and subject names and express a relationship between CAs; self-issued and self-signed certificates are distinct concepts. Trust anchors are inputs to path validation, not simply the top certificate in any chain. [RFC 5280, §§3.2, 6.1.1](https://www.rfc-editor.org/rfc/rfc5280#section-3.2)
* CRLs are signed, time-bounded status snapshots with issuer, thisUpdate/nextUpdate and CRL number; a revoked entry is keyed by serial, and RFC 5280 says entries must remain at least through the scheduled CRL beyond the revoked certificate's validity. CRLs are periodic, so their latency is bounded by the publication cadence. [RFC 5280, §§3.3, 5.1–5.3](https://www.rfc-editor.org/rfc/rfc5280#section-5)

### CSR, CMS kits, OCSP, and wildcard names

* PKCS #10/RFC 2986 CSR contains subject DN, SPKI, optional attributes and a signature over DER-encoded CertificationRequestInfo. A CA verifies proof of possession and independently chooses issuer, serial, validity and signature algorithm; returning a certificate is outside the CSR specification. [RFC 2986, §§1, 3–4.2](https://www.rfc-editor.org/rfc/rfc2986)
* CMS/RFC 5652 SignedData can carry certificates and CRLs, including a certs-only degenerate SignedData package (no signers; eContent omitted and content type id-data). It does not define Fleet-specific trust semantics. [RFC 5652, §§5, 5.1](https://www.rfc-editor.org/rfc/rfc5652#section-5)
* OCSP/RFC 6960 has definitive `good`, `revoked`, and `unknown` states plus unsigned protocol errors such as `unauthorized`, `malformedRequest`, `internalError`, and `tryLater`. `good` alone does not prove that the certificate was issued. A responder key must be the issuing CA, a locally trusted responder, or a directly CA-issued explicitly authorized OCSP signer. [RFC 6960, §§2.2–2.6, 4.2.2.2](https://www.rfc-editor.org/rfc/rfc6960)
* TLS service identities belong in typed SAN entries, not Common Name. RFC 9525 supports wildcards only as the sole `*` occupying the complete left-most DNS label; it matches one label. It also warns that each SAN broadens the impact of key compromise. [RFC 9525, §§1.3, 2, 4.1, 6.3, 7.1, 7.5](https://www.rfc-editor.org/rfc/rfc9525)

## Rust implementation evidence (retrieved from docs.rs)

These are capability boundaries observed in public crate documentation, not exhaustive guarantees about every version or downstream integration. Versions below are the versions displayed by docs.rs on retrieval date.

| Component | Documented capability | Design-relevant gap / limitation |
|---|---|---|
| [`rcgen` 0.14.10](https://docs.rs/rcgen/0.14.10/rcgen/) | Certificate and CSR generation, signed/self-signed certificates, certificate/CRL parameter types, SAN/basic-constraints/name-constraints and custom extensions; has `SigningKey` abstraction and pluggable crypto feature backends (ring/aws-lc-rs). | A certificate construction library is not an issuance policy/state machine, trust-store manager, publication service, immutable artifact store, or operator-approval system. Do not assume a key can be used through an arbitrary remote signer without an explicit adapter and proof that its signing semantics/algorithms match. Its documented signature algorithm list is not evidence for every Fleet-required algorithm. |
| [`x509-cert` 0.3.0](https://docs.rs/x509-cert/0.3.0/x509_cert/) | RustCrypto X.509 syntax types following RFC 5280; modules include certificate builder, certificate, CRL, extensions, names, trust-anchor-related structures, and PKCS #10 request types. | Syntax/building blocks alone do not establish policy-correct issuance, robust path building/validation, service-identity verification, signing-key protection, publication, lifecycle state, or operational authorization. Treat each as separate required capability and test it. |
| [`der` 0.8.3](https://docs.rs/der/0.8.3/der/) | DER encode/decode, canonical-form checks, extensible ASN.1 types, `no_std`/heapless support; documented format integrations include X.509, OCSP and CMS. | It is an encoding layer, not X.509 profile validation, cryptographic signing, path validation, or PKI operations. Generic ASN.1 `Any` makes schema/policy validation the caller's responsibility. |
| [`cms` 0.2.3](https://docs.rs/cms/0.2.3/cms/) | RustCrypto CMS structures for RFC 5652; docs specifically show certificate to certs-only SignedData / `.p7b` conversion. | Documented API does not establish the full production issuance-kit workflow, path verification, signing algorithms for the Fleet matrix, safe external signing, or CMS profile validation. Pin a tested version and add interoperability/oracle tests before use. |
| [`openssl` 0.10.82](https://docs.rs/openssl/0.10.82/openssl/) | Bindings to OpenSSL; documents `x509`, `ocsp`, `cms`, keys, signatures, ASN.1 and CRL-oriented APIs; supports OpenSSL 1.0.2 through 4.x and several LibreSSL versions. | Native OpenSSL and headers are required unless vendored; API availability differs by linked library version and build configuration. That portability/version variability must be locked and tested. Presence of a binding does not itself provide safe Fleet issuance workflow, explicit human authorization, immutable publication, or a key custodian. |
| [`rustls-pki-types` 1.16.0](https://docs.rs/rustls-pki-types/1.16.0/rustls_pki_types/) | Shared DER/PEM types for certificates, CRLs, CSRs and private keys. Explicitly says it does **not** create certificates or keys and points users to rcgen for creation. Private key types intentionally do not implement `Clone`. | Representation and parsing only; not a CA implementation, validator, issuance engine or publication system. |

### Explicit implementation guardrails inferred for design (design recommendations, not standard mandates)

1. Put certificate profile construction behind a typed, versioned profile layer. Derive emitted DER and extensions deterministically from the approved request and immutable profile; independently parse and validate the result before activation/publication.
2. Define a signing-provider contract (`key_id`, public SPKI, algorithm capability, `sign(algorithm, exact_tbs_der)`, verification) and bind it to a *named, authorized signing source*. Do not fall back to generating a software key, a different algorithm, a different issuer or a different key if the configured signer is absent or rejects a request.
3. Keep crypto/ASN.1 adapters behind a narrow interface. Select one backend per supported algorithm via explicit configuration; reject unsupported algorithm, parameter, key custody, CMS feature or OCSP mode with a typed error. No silent backend/algorithm substitution.
4. Treat OCSP as unavailable unless the separate responder, signing authorization, status database, freshness policy and response validation are implemented. A library's OCSP ASN.1 API is not evidence that these operational controls exist.
5. Validate wildcards before CSR acceptance: only `*.` as a full left-most label, no wildcard IP SAN, no embedded/multiple wildcard, no public-suffix or out-of-scope wildcard under policy, and no Common Name fallback. The CA chooses the final SAN set; CSR attributes are requests, never authority.

## Evidence log

Direct web retrieval with `webfetch` on 2026-11-12:

* RFC Editor primary texts: RFC 5280, RFC 6960, RFC 2986, RFC 5652, RFC 9525.
* docs.rs rendered API/crate documentation: rcgen 0.14.10, x509-cert 0.3.0, der 0.8.3, cms 0.2.3, openssl 0.10.82, rustls-pki-types 1.16.0.

Claims above are limited to the sections/pages cited. Library docs are mutable; production dependency selection should pin versions and re-check API, feature, algorithm, license and test evidence at implementation time.
