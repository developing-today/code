//! OCSP (RFC 6960): delegated responder certificates, response
//! signing, inspection and verification — the protocol core behind
//! the `fleet cert ocsp` verb group (spec §§4.3, 6.1; #13722).
//!
//! Scope discipline (spec §4.3): OCSP material exists only behind a
//! functional responder. This module *is* the responder core — it
//! produces real, signed OCSP responses from an issuer bundle and
//! verifies them against the crate's chain code
//! ([`crate::chain`]). It embeds no OCSP URL anywhere and starts no
//! network service; AIA publication stays with the (separate,
//! unbuilt) publish machinery, and an HTTP front door is an
//! operational deployment of this core, not a different protocol.
//!
//! Two signer shapes, both RFC 6960 §3.2-authorized:
//! - **direct**: the issuer key itself signs; the responderID is
//!   the issuer.
//! - **delegated**: a responder certificate issued by that issuer
//!   under the `ocsp-responder` leaf profile (EKU
//!   id-kp-OCSPSigning only, plus the §3.2 NoCheck extension)
//!   signs; verification proves the delegation against the issuer
//!   certificate with the chain code before trusting the status.
//!
//! Signing keys load only from explicit signer sources (§24.3),
//! exactly like every other signing operation in the crate.
//!
//! CertID hashes: SHA-1 is the RFC 6960 baseline every client
//! accepts; SHA-256 is selectable. The SHA-1 below is a small
//! self-contained FIPS 180-1 implementation with known-answer
//! tests — it is an *identifier* hash here (issuer name/key
//! fingerprints inside CertID and responderID), never a signature;
//! the fleet lock carries no `sha1` crate and one was not added
//! for an identifier role.

use crate::algorithm::AlgorithmId;
use crate::chain::{CertInfo, Check, parse_cert};
use crate::engine::{CaSigner, load_signer_key};
use crate::error::{CertError, Result};
use crate::model::SignerSource;
use crate::timeparse::Rfc3339;
use sha2::{Digest, Sha256};

// ---------------------------------------------------------------- SHA-1
// FIPS 180-1, identifier hashing only (see module docs).

fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for block in msg.chunks_exact(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                block[4 * i],
                block[4 * i + 1],
                block[4 * i + 2],
                block[4 * i + 3],
            ]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A827999u32),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let tmp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = tmp;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }
    let mut out = [0u8; 20];
    for (i, word) in h.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

// ---------------------------------------------------------------- DER write

fn der_len(out: &mut Vec<u8>, len: usize) {
    if len < 0x80 {
        out.push(len as u8);
    } else {
        let bytes = len.to_be_bytes();
        let first = bytes
            .iter()
            .position(|&b| b != 0)
            .unwrap_or(bytes.len() - 1);
        let n = &bytes[first..];
        out.push(0x80 | n.len() as u8);
        out.extend_from_slice(n);
    }
}

fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(content.len() + 8);
    out.push(tag);
    der_len(&mut out, content.len());
    out.extend_from_slice(content);
    out
}

fn seq(parts: &[u8]) -> Vec<u8> {
    tlv(0x30, parts)
}

fn oid_der(oid: &[u64]) -> Vec<u8> {
    let mut body = vec![(40 * oid[0] + oid[1]) as u8];
    for &v in &oid[2..] {
        let mut stack = vec![(v & 0x7f) as u8];
        let mut rest = v >> 7;
        while rest > 0 {
            stack.push(((rest & 0x7f) as u8) | 0x80);
            rest >>= 7;
        }
        stack.reverse();
        body.extend_from_slice(&stack);
    }
    tlv(0x06, &body)
}

fn oid_string(oid: &[u64]) -> String {
    oid.iter()
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(".")
}

/// DER INTEGER from big-endian magnitude bytes (leading zeros are
/// stripped; a 0x00 is prepended when the high bit would make the
/// value read negative).
fn integer_der(magnitude: &[u8]) -> Vec<u8> {
    let mut bytes = magnitude;
    while bytes.len() > 1 && bytes[0] == 0 {
        bytes = &bytes[1..];
    }
    let mut content = Vec::with_capacity(bytes.len() + 1);
    if bytes.is_empty() || bytes[0] & 0x80 != 0 {
        content.push(0);
    }
    content.extend_from_slice(bytes);
    tlv(0x02, &content)
}

/// Normalize a serial to magnitude bytes (no sign padding).
fn serial_magnitude(serial: &[u8]) -> Vec<u8> {
    let mut s = serial;
    while s.len() > 1 && s[0] == 0 {
        s = &s[1..];
    }
    if s.is_empty() { vec![0] } else { s.to_vec() }
}

fn generalized_time_der(t: &Rfc3339) -> Vec<u8> {
    let dt = t.0.to_offset(time::UtcOffset::UTC);
    let text = format!(
        "{:04}{:02}{:02}{:02}{:02}{:02}Z",
        dt.year(),
        u8::from(dt.month()),
        dt.day(),
        dt.hour(),
        dt.minute(),
        dt.second()
    );
    tlv(0x18, text.as_bytes())
}

fn algorithm_identifier(oid: &[u64], params_null: bool) -> Vec<u8> {
    let mut body = oid_der(oid);
    if params_null {
        body.extend_from_slice(&[0x05, 0x00]);
    }
    seq(&body)
}

// ---------------------------------------------------------------- DER read

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Reader { buf, pos: 0 }
    }
    fn done(&self) -> bool {
        self.pos >= self.buf.len()
    }
    /// Read one TLV: (tag, content, full encoding).
    fn read(&mut self) -> Result<(u8, &'a [u8], &'a [u8])> {
        let start = self.pos;
        if self.pos + 2 > self.buf.len() {
            return Err(CertError::manifest("OCSP DER: truncated TLV"));
        }
        let tag = self.buf[self.pos];
        self.pos += 1;
        let first = self.buf[self.pos];
        self.pos += 1;
        let len = if first < 0x80 {
            first as usize
        } else {
            let n = (first & 0x7f) as usize;
            if n == 0 || n > 4 || self.pos + n > self.buf.len() {
                return Err(CertError::manifest("OCSP DER: bad length"));
            }
            let mut len = 0usize;
            for k in 0..n {
                len = (len << 8) | self.buf[self.pos + k] as usize;
            }
            self.pos += n;
            len
        };
        if self.pos + len > self.buf.len() {
            return Err(CertError::manifest("OCSP DER: content overruns buffer"));
        }
        let content = &self.buf[self.pos..self.pos + len];
        self.pos += len;
        Ok((tag, content, &self.buf[start..self.pos]))
    }
    fn expect(&mut self, tag: u8, what: &str) -> Result<(&'a [u8], &'a [u8])> {
        let (t, content, full) = self.read()?;
        if t != tag {
            return Err(CertError::manifest(format!(
                "OCSP DER: expected {what} (tag 0x{tag:02x}), found 0x{t:02x}"
            )));
        }
        Ok((content, full))
    }
}

fn oid_to_string(content: &[u8]) -> Result<String> {
    if content.is_empty() {
        return Err(CertError::manifest("OCSP DER: empty OID"));
    }
    let mut parts = vec![(content[0] / 40) as u64, (content[0] % 40) as u64];
    let mut cur: u64 = 0;
    for &b in &content[1..] {
        cur = (cur << 7) | (b & 0x7f) as u64;
        if b & 0x80 == 0 {
            parts.push(cur);
            cur = 0;
        }
    }
    Ok(oid_string(&parts))
}

fn parse_generalized_time(content: &[u8]) -> Result<Rfc3339> {
    let text = std::str::from_utf8(content)
        .map_err(|_| CertError::manifest("OCSP DER: GeneralizedTime is not UTF-8"))?;
    let digits: String = text.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.len() != 14 {
        return Err(CertError::manifest(format!(
            "OCSP DER: unsupported GeneralizedTime '{text}'"
        )));
    }
    let rfc = format!(
        "{}-{}-{}T{}:{}:{}Z",
        &digits[0..4],
        &digits[4..6],
        &digits[6..8],
        &digits[8..10],
        &digits[10..12],
        &digits[12..14]
    );
    Rfc3339::parse(&rfc)
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn unhex(text: &str) -> Result<Vec<u8>> {
    let text = text.trim().trim_start_matches("0x");
    if !text.len().is_multiple_of(2) {
        return Err(CertError::usage(format!("odd-length hex serial '{text}'")));
    }
    let mut out = Vec::with_capacity(text.len() / 2);
    let bytes = text.as_bytes();
    for pair in bytes.chunks_exact(2) {
        let s = std::str::from_utf8(pair).unwrap_or("");
        let v = u8::from_str_radix(s, 16)
            .map_err(|_| CertError::usage(format!("invalid hex serial '{text}'")))?;
        out.push(v);
    }
    Ok(out)
}

// ---------------------------------------------------------------- types

/// The CertID hash algorithm (RFC 6960 §3.2). SHA-1 is the
/// interoperability baseline; SHA-256 is selectable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OcspHash {
    Sha1,
    Sha256,
}

impl OcspHash {
    pub fn parse(input: &str) -> Result<Self> {
        match input.trim().to_ascii_lowercase().as_str() {
            "sha1" | "sha-1" => Ok(OcspHash::Sha1),
            "sha256" | "sha-256" => Ok(OcspHash::Sha256),
            other => Err(CertError::usage(format!(
                "unknown OCSP CertID hash '{other}' (sha1 | sha256)"
            ))),
        }
    }
    pub fn oid(&self) -> &'static [u64] {
        match self {
            OcspHash::Sha1 => &[1, 3, 14, 3, 2, 26],
            OcspHash::Sha256 => &[2, 16, 840, 1, 101, 3, 4, 2, 1],
        }
    }
    pub fn oid_str(&self) -> String {
        oid_string(self.oid())
    }
    pub fn name(&self) -> &'static str {
        match self {
            OcspHash::Sha1 => "sha1",
            OcspHash::Sha256 => "sha256",
        }
    }
    fn digest(&self, data: &[u8]) -> Vec<u8> {
        match self {
            OcspHash::Sha1 => sha1(data).to_vec(),
            OcspHash::Sha256 => Sha256::digest(data).to_vec(),
        }
    }
    fn from_oid(oid: &str) -> Option<Self> {
        match oid {
            "1.3.14.3.2.26" => Some(OcspHash::Sha1),
            "2.16.840.1.101.3.4.2.1" => Some(OcspHash::Sha256),
            _ => None,
        }
    }
}

/// The status a response asserts for one certificate (RFC 6960
/// §3.2). `reason` is a CRLReason code (RFC 5280 §5.3.1); code 7
/// is unassigned and refused at parse/sign time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OcspStatus {
    Good,
    Revoked {
        revocation_time: Rfc3339,
        reason: Option<u8>,
    },
    Unknown,
}

impl OcspStatus {
    pub fn label(&self) -> String {
        match self {
            OcspStatus::Good => "good".to_string(),
            OcspStatus::Revoked { reason, .. } => match reason {
                Some(r) => format!("revoked (reason {r})"),
                None => "revoked".to_string(),
            },
            OcspStatus::Unknown => "unknown".to_string(),
        }
    }
}

/// CRLReason names (RFC 5280 §5.3.1) for the CLI.
pub fn parse_reason(input: &str) -> Result<u8> {
    let named: &[(&str, u8)] = &[
        ("unspecified", 0),
        ("key-compromise", 1),
        ("ca-compromise", 2),
        ("affiliation-changed", 3),
        ("superseded", 4),
        ("cessation-of-operation", 5),
        ("certificate-hold", 6),
        ("remove-from-crl", 8),
        ("privilege-withdrawn", 9),
        ("aa-compromise", 10),
    ];
    let lower = input.trim().to_ascii_lowercase().replace('_', "-");
    for (name, code) in named {
        if *name == lower {
            return Ok(*code);
        }
    }
    match lower.parse::<u8>() {
        Ok(code) if code != 7 && code <= 10 => Ok(code),
        _ => Err(CertError::usage(format!(
            "unknown revocation reason '{input}' (name or code 0-10, excluding 7)"
        ))),
    }
}

/// Who signs a response, and where its key comes from (§24.3: the
/// signer source is always explicit).
#[derive(Debug, Clone)]
pub enum OcspSigner {
    /// The issuer key signs directly; responderID = the issuer.
    Direct {
        issuer_record_algorithm: AlgorithmId,
        key_file: String,
        source: SignerSource,
    },
    /// A delegated responder certificate (profile
    /// `ocsp-responder`) signs; its key comes from `source`.
    Delegated {
        cert: CertInfo,
        key_file: String,
        source: SignerSource,
    },
}

impl OcspSigner {
    /// The certificate whose key signs (also the responderID
    /// source and the certificate embedded in the response).
    pub fn cert<'i>(&'i self, issuer: &'i CertInfo) -> &'i CertInfo {
        match self {
            OcspSigner::Direct { .. } => issuer,
            OcspSigner::Delegated { cert, .. } => cert,
        }
    }
    fn key_algorithm(&self, issuer: &CertInfo) -> AlgorithmId {
        match self {
            OcspSigner::Direct {
                issuer_record_algorithm,
                ..
            } => *issuer_record_algorithm,
            OcspSigner::Delegated { cert, .. } => cert
                .subject_algorithm
                .unwrap_or(issuer.subject_algorithm.unwrap_or(AlgorithmId::P256)),
        }
    }
    fn load(&self) -> Result<CaSigner> {
        match self {
            OcspSigner::Direct {
                issuer_record_algorithm,
                key_file,
                source,
            } => load_signer_key(source, key_file, *issuer_record_algorithm),
            OcspSigner::Delegated {
                cert,
                key_file,
                source,
            } => {
                let algo = cert.subject_algorithm.ok_or_else(|| {
                    CertError::engine("responder certificate has an unregistered key algorithm")
                })?;
                load_signer_key(source, key_file, algo)
            }
        }
    }
}

/// One `ocsp sign` request, fully resolved by the caller.
#[derive(Debug, Clone)]
pub struct OcspSignRequest {
    /// The CA that issued the leaf the response is about.
    pub issuer: CertInfo,
    /// The leaf's serial (magnitude bytes).
    pub leaf_serial: Vec<u8>,
    pub status: OcspStatus,
    pub hash: OcspHash,
    pub produced_at: Rfc3339,
    pub this_update: Rfc3339,
    pub next_update: Option<Rfc3339>,
    pub signer: OcspSigner,
}

// ---------------------------------------------------------------- cert helpers

/// The subject public key bytes (the BIT STRING payload) of a
/// certificate — what CertID's issuerKeyHash and the responderID
/// byKey hash are taken over (RFC 6960 §3.2: the public key,
/// excluding tag, length and algorithm).
pub fn public_key_bytes(cert_der: &[u8]) -> Result<Vec<u8>> {
    let (_, cert) = x509_parser::prelude::parse_x509_certificate(cert_der)
        .map_err(|e| CertError::manifest(format!("certificate parse failed: {e}")))?;
    Ok(cert
        .tbs_certificate
        .subject_pki
        .subject_public_key
        .data
        .to_vec())
}

/// Does this certificate's EKU extension (2.5.29.37) grant OCSP
/// signing (1.3.6.1.5.5.7.3.9)? No EKU extension at all returns
/// false — a delegated responder must carry the EKU explicitly.
pub fn cert_has_ocsp_signing_eku(cert_der: &[u8]) -> Result<bool> {
    let (_, cert) = x509_parser::prelude::parse_x509_certificate(cert_der)
        .map_err(|e| CertError::manifest(format!("certificate parse failed: {e}")))?;
    for ext in cert.tbs_certificate.extensions() {
        if ext.oid.to_id_string() == "2.5.29.37" {
            let mut r = Reader::new(ext.value);
            let (content, _) = r.expect(0x30, "EKU sequence")?;
            let mut inner = Reader::new(content);
            while !inner.done() {
                let (tag, oid_content, _) = inner.read()?;
                if tag == 0x06 && oid_to_string(oid_content)? == "1.3.6.1.5.5.7.3.9" {
                    return Ok(true);
                }
            }
            return Ok(false);
        }
    }
    Ok(false)
}

// ---------------------------------------------------------------- signing

fn signature_algorithm_for(algo: AlgorithmId) -> (&'static [u64], bool) {
    // (OID, params-are-NULL). ECDSA and EdDSA/ML-DSA algorithm
    // identifiers carry absent parameters; RSA carries NULL.
    match algo {
        AlgorithmId::Ed25519 => (&[1, 3, 101, 112], false),
        AlgorithmId::P256 => (&[1, 2, 840, 10045, 4, 3, 2], false),
        AlgorithmId::Rsa2048 | AlgorithmId::Rsa4096 => (&[1, 2, 840, 113549, 1, 1, 11], true),
        AlgorithmId::MlDsa44 => (&[2, 16, 840, 1, 101, 3, 4, 3, 17], false),
    }
}

fn sign_bytes(signer: &CaSigner, msg: &[u8]) -> Result<Vec<u8>> {
    use rcgen::SigningKey;
    let sig = match signer {
        CaSigner::Classical(key) => key.sign(msg),
        CaSigner::MlDsa(signer) => signer.sign(msg),
    };
    sig.map_err(|e| CertError::engine(format!("OCSP response signing failed: {e}")))
}

fn cert_id_der(issuer: &CertInfo, leaf_serial: &[u8], hash: OcspHash) -> Result<Vec<u8>> {
    let name_hash = hash.digest(&issuer.subject_name_der);
    let key_hash = hash.digest(&public_key_bytes(&issuer.der)?);
    let mut body = algorithm_identifier(hash.oid(), true);
    body.extend_from_slice(&tlv(0x04, &name_hash));
    body.extend_from_slice(&tlv(0x04, &key_hash));
    body.extend_from_slice(&integer_der(&serial_magnitude(leaf_serial)));
    Ok(seq(&body))
}

fn status_der(status: &OcspStatus) -> Vec<u8> {
    match status {
        OcspStatus::Good => tlv(0x80, &[]),
        OcspStatus::Revoked {
            revocation_time,
            reason,
        } => {
            // RevokedInfo is IMPLICIT-tagged: [1] replaces the
            // SEQUENCE tag, the fields are the content directly.
            let mut info = generalized_time_der(revocation_time);
            if let Some(reason) = reason {
                let enumerated = tlv(0x0a, &[*reason]);
                info.extend_from_slice(&tlv(0xa0, &enumerated));
            }
            tlv(0xa1, &info)
        }
        // UnknownInfo is an empty SEQUENCE, IMPLICIT-tagged:
        // [2] with empty content.
        OcspStatus::Unknown => tlv(0xa2, &[]),
    }
}

/// Sign one OCSP response. Returns the full OCSPResponse DER
/// (responseStatus `successful`, a single SingleResponse).
pub fn sign_response(req: &OcspSignRequest) -> Result<Vec<u8>> {
    if let OcspStatus::Revoked { reason, .. } = req.status {
        if let Some(r) = reason {
            if r == 7 || r > 10 {
                return Err(CertError::usage(format!(
                    "revocation reason {r} is not an assigned CRLReason code"
                )));
            }
        }
    }
    let signer_cert = req.signer.cert(&req.issuer);
    let signer_key = req.signer.load()?;
    let key_algo = req.signer.key_algorithm(&req.issuer);

    // SingleResponse.
    let mut single = cert_id_der(&req.issuer, &req.leaf_serial, req.hash)?;
    single.extend_from_slice(&status_der(&req.status));
    single.extend_from_slice(&generalized_time_der(&req.this_update));
    if let Some(next) = &req.next_update {
        single.extend_from_slice(&tlv(0xa0, &generalized_time_der(next)));
    }
    let single = seq(&single);

    // ResponseData (TBS). responderID byKey [2] EXPLICIT OCTET
    // STRING holding the SHA-1 of the signer's public key,
    // whatever the CertID hash (§3.2; ResponderID is EXPLICIT-
    // tagged, unlike CertStatus).
    let responder_id = tlv(
        0xa2,
        &tlv(0x04, &sha1(&public_key_bytes(&signer_cert.der)?)),
    );
    let mut tbs_body = responder_id;
    tbs_body.extend_from_slice(&generalized_time_der(&req.produced_at));
    tbs_body.extend_from_slice(&seq(&single));
    let tbs = seq(&tbs_body);

    let signature = sign_bytes(&signer_key, &tbs)?;
    let (sig_oid, params_null) = signature_algorithm_for(key_algo);
    let mut basic_body = tbs;
    basic_body.extend_from_slice(&algorithm_identifier(sig_oid, params_null));
    let mut sig_bits = vec![0u8];
    sig_bits.extend_from_slice(&signature);
    basic_body.extend_from_slice(&tlv(0x03, &sig_bits));
    // certs [0]: the signing certificate, so a verifier can
    // identify and authorize the responder from the response
    // alone (plus the issuer it already trusts).
    // certs [0] EXPLICIT SEQUENCE OF Certificate: EXPLICIT
    // tagging wraps the base type's full encoding, i.e. the
    // SEQUENCE TLV holding the certificates.
    basic_body.extend_from_slice(&tlv(0xa0, &seq(&signer_cert.der)));
    let basic = seq(&basic_body);

    let mut response_bytes = oid_der(&[1, 3, 6, 1, 5, 5, 7, 48, 1, 1]);
    response_bytes.extend_from_slice(&tlv(0x04, &basic));
    let mut out = tlv(0x0a, &[0]);
    out.extend_from_slice(&tlv(0xa0, &seq(&response_bytes)));
    Ok(seq(&out))
}

// ---------------------------------------------------------------- parsing

/// One parsed SingleResponse.
#[derive(Debug, Clone)]
pub struct OcspSingleInfo {
    pub hash: Option<OcspHash>,
    pub hash_algorithm_oid: String,
    pub issuer_name_hash: String,
    pub issuer_key_hash: String,
    pub serial_hex: String,
    pub status: OcspStatus,
    pub this_update: Rfc3339,
    pub next_update: Option<Rfc3339>,
}

/// A parsed OCSPResponse, with the internals verification needs.
#[derive(Debug, Clone)]
pub struct OcspResponseInfo {
    /// responseStatus == successful (0).
    pub successful: bool,
    pub produced_at: Rfc3339,
    /// responderID byKey hash (hex) when the byKey form is used.
    pub responder_key_hash: Option<String>,
    pub signature_algorithm_oid: String,
    /// Certificates embedded in the response (DER).
    pub embedded_certs: Vec<Vec<u8>>,
    pub singles: Vec<OcspSingleInfo>,
    tbs: Vec<u8>,
    signature: Vec<u8>,
}

/// Parse an OCSPResponse DER blob.
pub fn parse_response(der: &[u8]) -> Result<OcspResponseInfo> {
    let mut top = Reader::new(der);
    let (resp_content, _) = top.expect(0x30, "OCSPResponse sequence")?;
    let mut r = Reader::new(resp_content);
    let (status_content, _) = r.expect(0x0a, "responseStatus enumerated")?;
    let successful = status_content == [0];
    if !successful {
        return Ok(OcspResponseInfo {
            successful: false,
            produced_at: Rfc3339::now(),
            responder_key_hash: None,
            signature_algorithm_oid: String::new(),
            embedded_certs: Vec::new(),
            singles: Vec::new(),
            tbs: Vec::new(),
            signature: Vec::new(),
        });
    }
    let (rb_wrap, _) = r.expect(0xa0, "responseBytes [0]")?;
    let mut rb_reader = Reader::new(rb_wrap);
    let (rb_content, _) = rb_reader.expect(0x30, "ResponseBytes sequence")?;
    let mut rb = Reader::new(rb_content);
    let (type_content, _) = rb.expect(0x06, "responseType OID")?;
    let type_oid = oid_to_string(type_content)?;
    if type_oid != "1.3.6.1.5.5.7.48.1.1" {
        return Err(CertError::manifest(format!(
            "OCSP response type {type_oid} is not id-pkix-ocsp-basic"
        )));
    }
    let (basic_der, _) = rb.expect(0x04, "response OCTET STRING")?;
    parse_basic_response(basic_der)
}

fn parse_basic_response(der: &[u8]) -> Result<OcspResponseInfo> {
    let mut top = Reader::new(der);
    let (basic_content, _) = top.expect(0x30, "BasicOCSPResponse sequence")?;
    let mut r = Reader::new(basic_content);
    let (tbs_content, tbs_full) = r.expect(0x30, "ResponseData sequence")?;
    let tbs = tbs_full.to_vec();
    let (sig_alg_content, _) = r.expect(0x30, "signatureAlgorithm sequence")?;
    let mut sa = Reader::new(sig_alg_content);
    let (sig_oid_content, _) = sa.expect(0x06, "signature algorithm OID")?;
    let signature_algorithm_oid = oid_to_string(sig_oid_content)?;
    let (sig_content, _) = r.expect(0x03, "signature BIT STRING")?;
    if sig_content.is_empty() || sig_content[0] != 0 {
        return Err(CertError::manifest(
            "OCSP signature BIT STRING has a nonzero unused-bit count",
        ));
    }
    let signature = sig_content[1..].to_vec();
    let mut embedded_certs = Vec::new();
    if !r.done() {
        let (certs_wrap, _) = r.expect(0xa0, "certs [0]")?;
        let mut cr = Reader::new(certs_wrap);
        let (certs_content, _) = cr.expect(0x30, "certificates sequence")?;
        let mut list = Reader::new(certs_content);
        while !list.done() {
            let (_, _, full) = list.read()?;
            embedded_certs.push(full.to_vec());
        }
    }

    // ResponseData.
    let mut t = Reader::new(tbs_content);
    let mut responder_key_hash = None;
    // Optional version [0] EXPLICIT INTEGER — present only for
    // non-v1 responses; skip it when present.
    let save = t.pos;
    if let Ok((tag, _, _)) = t.read() {
        if tag != 0xa0 {
            t.pos = save;
        }
    }
    {
        let (tag, content, _) = t.read()?;
        match tag {
            // byName [1] EXPLICIT Name: constructed.
            0xa1 => {
                let _ = content;
            }
            // byKey [2] EXPLICIT OCTET STRING: the key hash is
            // the wrapped OCTET STRING's content.
            0xa2 => {
                let mut inner = Reader::new(content);
                let (hash_content, _) = inner.expect(0x04, "responderID byKey octet string")?;
                responder_key_hash = Some(hex(hash_content));
            }
            other => {
                return Err(CertError::manifest(format!(
                    "OCSP responderID has unexpected tag 0x{other:02x}"
                )));
            }
        }
    }
    let (produced_content, _) = t.expect(0x18, "producedAt GeneralizedTime")?;
    let produced_at = parse_generalized_time(produced_content)?;
    let (singles_content, _) = t.expect(0x30, "responses sequence")?;
    let mut singles = Vec::new();
    let mut sr = Reader::new(singles_content);
    while !sr.done() {
        let (single_content, _) = sr.expect(0x30, "SingleResponse sequence")?;
        singles.push(parse_single(single_content)?);
    }
    Ok(OcspResponseInfo {
        successful: true,
        produced_at,
        responder_key_hash,
        signature_algorithm_oid,
        embedded_certs,
        singles,
        tbs,
        signature,
    })
}

fn parse_single(content: &[u8]) -> Result<OcspSingleInfo> {
    let mut r = Reader::new(content);
    let (cid_content, _) = r.expect(0x30, "CertID sequence")?;
    let mut cid = Reader::new(cid_content);
    let (alg_content, _) = cid.expect(0x30, "hashAlgorithm sequence")?;
    let mut alg = Reader::new(alg_content);
    let (hash_oid_content, _) = alg.expect(0x06, "hash algorithm OID")?;
    let hash_algorithm_oid = oid_to_string(hash_oid_content)?;
    let (name_hash, _) = cid.expect(0x04, "issuerNameHash")?;
    let (key_hash, _) = cid.expect(0x04, "issuerKeyHash")?;
    let (serial_content, _) = cid.expect(0x02, "serialNumber INTEGER")?;

    let (status_tag, status_content, _) = r.read()?;
    let status = match status_tag {
        0x80 => OcspStatus::Good,
        0xa1 => {
            // RevokedInfo, IMPLICIT: the fields are the content.
            let mut ri = Reader::new(status_content);
            let (time_content, _) = ri.expect(0x18, "revocationTime")?;
            let revocation_time = parse_generalized_time(time_content)?;
            let mut reason = None;
            if !ri.done() {
                let (wrap, _) = ri.expect(0xa0, "revocationReason [0]")?;
                let mut wr = Reader::new(wrap);
                let (enum_content, _) = wr.expect(0x0a, "CRLReason enumerated")?;
                reason = enum_content.first().copied();
            }
            OcspStatus::Revoked {
                revocation_time,
                reason,
            }
        }
        0xa2 => OcspStatus::Unknown,
        other => {
            return Err(CertError::manifest(format!(
                "OCSP certStatus has unexpected tag 0x{other:02x}"
            )));
        }
    };
    let (this_content, _) = r.expect(0x18, "thisUpdate")?;
    let this_update = parse_generalized_time(this_content)?;
    // Optional nextUpdate [0] EXPLICIT GeneralizedTime; any
    // further elements are singleExtensions, not modelled.
    let mut next_update = None;
    if !r.done() && r.buf[r.pos] == 0xa0 {
        let (wrap, _) = r.expect(0xa0, "nextUpdate [0]")?;
        let mut wr = Reader::new(wrap);
        let (next_content, _) = wr.expect(0x18, "nextUpdate")?;
        next_update = Some(parse_generalized_time(next_content)?);
    }
    Ok(OcspSingleInfo {
        hash: OcspHash::from_oid(&hash_algorithm_oid),
        hash_algorithm_oid,
        issuer_name_hash: hex(name_hash),
        issuer_key_hash: hex(key_hash),
        serial_hex: hex(&serial_magnitude(serial_content)),
        status,
        this_update,
        next_update,
    })
}

// ---------------------------------------------------------------- verification

fn verify_signature_with(cert_der: &[u8], sig_oid: &str, msg: &[u8], sig: &[u8]) -> bool {
    use aws_lc_rs::signature::{
        ECDSA_P256_SHA256_ASN1, ED25519, ML_DSA_44, RSA_PKCS1_2048_8192_SHA256, UnparsedPublicKey,
    };
    let key = match public_key_bytes(cert_der) {
        Ok(k) => k,
        Err(_) => return false,
    };
    let ok = match sig_oid {
        "1.3.101.112" => UnparsedPublicKey::new(&ED25519, &key)
            .verify(msg, sig)
            .is_ok(),
        "1.2.840.10045.4.3.2" => UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, &key)
            .verify(msg, sig)
            .is_ok(),
        "1.2.840.113549.1.1.11" => UnparsedPublicKey::new(&RSA_PKCS1_2048_8192_SHA256, &key)
            .verify(msg, sig)
            .is_ok(),
        "2.16.840.1.101.3.4.3.17" => UnparsedPublicKey::new(&ML_DSA_44, &key)
            .verify(msg, sig)
            .is_ok(),
        _ => false,
    };
    ok
}

fn secs_between(a: Rfc3339, b: Rfc3339) -> i64 {
    (b.0 - a.0).whole_seconds()
}

/// Clock-skew allowance for producedAt/thisUpdate (§4.3 status
/// freshness is a deployment policy; five minutes is the crate's
/// verification default).
pub const FRESHNESS_SKEW_SECONDS: i64 = 300;

/// Verify a response about `leaf_serial` from `issuer` at time
/// `at`, check by check (the same `Check` shape the chain
/// validator reports). The oracle is the crate chain code:
/// signature verification is aws-lc-rs, and a delegated responder
/// is authorized by proving the issuer signed its certificate
/// ([`crate::chain::verify_signature`]) and that the certificate
/// carries the OCSP-signing EKU.
pub fn verify_response(
    der: &[u8],
    issuer: &CertInfo,
    leaf_serial: &[u8],
    at: Rfc3339,
) -> Result<Vec<Check>> {
    let mut checks: Vec<Check> = Vec::new();
    let mut push = |name: &str, ok: bool, detail: String| {
        checks.push(Check {
            name: name.to_string(),
            ok,
            detail,
        });
    };
    let info = match parse_response(der) {
        Ok(info) if info.successful => {
            push(
                "parse",
                true,
                "OCSPResponse parses; responseStatus successful".to_string(),
            );
            info
        }
        Ok(_) => {
            push(
                "parse",
                false,
                "responseStatus is not successful".to_string(),
            );
            return Ok(checks);
        }
        Err(e) => {
            push("parse", false, format!("response does not parse: {e}"));
            return Ok(checks);
        }
    };
    if info.singles.len() != 1 {
        push(
            "parse",
            false,
            format!(
                "expected exactly one SingleResponse, found {}",
                info.singles.len()
            ),
        );
        return Ok(checks);
    }
    let single = &info.singles[0];

    // --- identify the signer: the embedded certificate (or the
    // issuer itself) whose key verifies the response signature.
    let mut candidates: Vec<Vec<u8>> = info.embedded_certs.clone();
    candidates.push(issuer.der.clone());
    let signer_der = candidates.iter().find(|der| {
        verify_signature_with(
            der,
            &info.signature_algorithm_oid,
            &info.tbs,
            &info.signature,
        )
    });
    let signer_der = match signer_der {
        Some(d) => d.clone(),
        None => {
            push(
                "signature",
                false,
                "no candidate key verifies the response signature".to_string(),
            );
            return Ok(checks);
        }
    };
    let signer = parse_cert(&signer_der)?;
    push(
        "signature",
        true,
        format!(
            "response signature verifies under {} ({})",
            signer.subject_dn, info.signature_algorithm_oid
        ),
    );

    // --- authorize the signer.
    if signer.der == issuer.der {
        push(
            "responder-authorized",
            true,
            "signed directly by the issuer".to_string(),
        );
    } else {
        let delegated_sig = crate::chain::verify_signature(issuer, &signer);
        let eku = cert_has_ocsp_signing_eku(&signer.der).unwrap_or(false);
        let in_window =
            info.produced_at >= signer.not_before && info.produced_at <= signer.not_after;
        push(
            "responder-authorized",
            delegated_sig && eku && in_window,
            format!(
                "delegated responder {}: issuer signature {}, OCSP-signing EKU {}, producedAt within responder validity {}",
                signer.subject_dn,
                if delegated_sig { "verifies" } else { "FAILS" },
                if eku { "present" } else { "MISSING" },
                if in_window { "yes" } else { "NO" }
            ),
        );
    }

    // --- CertID binding to (issuer, leaf serial).
    match single.hash {
        Some(hash) => {
            let name_ok = single.issuer_name_hash == hex(&hash.digest(&issuer.subject_name_der));
            let key_ok = match public_key_bytes(&issuer.der) {
                Ok(kb) => single.issuer_key_hash == hex(&hash.digest(&kb)),
                Err(_) => false,
            };
            let serial_ok = single.serial_hex == hex(&serial_magnitude(leaf_serial));
            push(
                "cert-id",
                name_ok && key_ok && serial_ok,
                format!(
                    "CertID ({}) issuer name hash {}, key hash {}, serial {}",
                    hash.name(),
                    if name_ok { "match" } else { "MISMATCH" },
                    if key_ok { "match" } else { "MISMATCH" },
                    if serial_ok { "match" } else { "MISMATCH" }
                ),
            );
        }
        None => {
            push(
                "cert-id",
                false,
                format!(
                    "unsupported CertID hash algorithm {}",
                    single.hash_algorithm_oid
                ),
            );
        }
    }

    // --- freshness at `at`.
    let this_age = secs_between(single.this_update, at);
    let future_skew = secs_between(at, single.this_update);
    let mut fresh = this_age >= 0 || future_skew <= FRESHNESS_SKEW_SECONDS;
    let mut detail = format!("thisUpdate {}", single.this_update);
    if future_skew > FRESHNESS_SKEW_SECONDS {
        fresh = false;
        detail.push_str(&format!(
            " is {future_skew}s in the future (skew allowance {FRESHNESS_SKEW_SECONDS}s)"
        ));
    }
    match single.next_update {
        Some(next) => {
            detail.push_str(&format!(", nextUpdate {next}"));
            if at > next {
                fresh = false;
                detail.push_str(" — EXPIRED at the verification time");
            }
        }
        None => detail.push_str(", no nextUpdate (status ages unbounded)"),
    }
    if secs_between(info.produced_at, at) < -FRESHNESS_SKEW_SECONDS {
        fresh = false;
        detail.push_str("; producedAt is beyond the skew allowance in the future");
    }
    push("freshness", fresh, detail);

    // --- the signed status itself (the payload of the response).
    push("status", true, format!("status: {}", single.status.label()));
    Ok(checks)
}

// ---------------------------------------------------------------- synthetic e2e

/// The acceptance run for `fleet cert ocsp test`: build a
/// synthetic hierarchy in `dir` (Ed25519+P-256 roots, a P-256
/// issuer, a server leaf, a delegated responder), then serve and
/// verify good and revoked statuses locally, plus the negative
/// cases (tampered signature, wrong serial, stale response).
/// Every step is a named check; the oracle is the crate's own
/// chain code.
pub fn synthetic_e2e(dir: &std::path::Path) -> Result<Vec<Check>> {
    use crate::bundle::load_bundle;
    use crate::graph::CrossSigningMode;
    use crate::model::{
        IssuerCreateRequest, IssuerPurpose, LeafIssueRequest, RootCreateRequest, SignerSource,
    };
    use crate::timeparse::CalendarDuration;

    let mut checks: Vec<Check> = Vec::new();
    let mut push = |name: &str, ok: bool, detail: String| {
        checks.push(Check {
            name: name.to_string(),
            ok,
            detail,
        });
    };
    let nb = Rfc3339::parse("2026-01-01T00:00:00Z")?;
    // The verification instant sits inside the responder
    // profile's real 30-day window (2026-01-01 + 30d) on purpose:
    // the delegation check must pass with production lifetimes.
    let at = Rfc3339::parse("2026-01-15T00:00:00Z")?;
    let day = CalendarDuration::days(1);

    let root_dir = dir.join("roots");
    crate::engine::create_root_set(
        &RootCreateRequest {
            organization: "OCSP Synthetic Org".into(),
            country: "US".into(),
            algorithms: vec![AlgorithmId::Ed25519, AlgorithmId::P256],
            generation: 1,
            partition: "A".into(),
            cross_signing: CrossSigningMode::Both,
            not_before: nb,
            not_after: Rfc3339::parse("2046-01-01T00:00:00Z")?,
            allow_legacy_root_strength: false,
            private_store: None,
        },
        &root_dir,
    )?;
    let issuer_dir = dir.join("issuers");
    crate::engine::create_issuer_set(
        &IssuerCreateRequest {
            root_bundle: root_dir.clone(),
            root_signer: SignerSource {
                key_dir: root_dir.join("private/keys"),
            },
            name: "Base".into(),
            purpose: IssuerPurpose::General,
            ou: None,
            slots: vec![(AlgorithmId::P256, "a".to_string())],
            cross_signing: CrossSigningMode::RootsOnly,
            not_before: nb,
            not_after: Rfc3339::parse("2031-01-01T00:00:00Z")?,
            allow_legacy_root_strength: false,
            permitted_dns: vec![],
            private_store: None,
        },
        &issuer_dir,
    )?;
    let leaf_dir = dir.join("leaf");
    crate::engine::issue_leaf(
        &LeafIssueRequest {
            issuer_bundle: issuer_dir.clone(),
            issuer_signer: SignerSource {
                key_dir: issuer_dir.join("private/keys"),
            },
            profile: "server".into(),
            name: "www".into(),
            common_name: None,
            dns_names: vec!["www.example.test".into()],
            key_algorithm: None,
            issuer_algorithm: Some(AlgorithmId::P256),
            issuer_partition: None,
            not_before: nb,
            not_after: CalendarDuration::days(730).add_to(nb)?,
            private_store: None,
        },
        &leaf_dir,
    )?;
    let responder_dir = dir.join("responder");
    crate::engine::issue_leaf(
        &LeafIssueRequest {
            issuer_bundle: issuer_dir.clone(),
            issuer_signer: SignerSource {
                key_dir: issuer_dir.join("private/keys"),
            },
            profile: "ocsp-responder".into(),
            name: "Responder".into(),
            common_name: Some("OCSP Responder".into()),
            dns_names: vec![],
            key_algorithm: None,
            issuer_algorithm: Some(AlgorithmId::P256),
            issuer_partition: None,
            not_before: nb,
            not_after: CalendarDuration::days(30).add_to(nb)?,
            private_store: None,
        },
        &responder_dir,
    )?;
    push(
        "hierarchy",
        true,
        "synthetic root set + P-256 issuer + server leaf + delegated responder built".to_string(),
    );

    let cert_of =
        |bundle: &crate::bundle::Bundle, record: &crate::model::CertRecord| -> Result<CertInfo> {
            let pem = crate::bundle::read_artifact(bundle, &record.cert_path)?;
            let blocks = crate::bundle::pem_decode_all(
                std::str::from_utf8(&pem)
                    .map_err(|_| CertError::manifest("bundle cert artifact is not UTF-8 PEM"))?,
            )?;
            let der = blocks
                .into_iter()
                .find(|(label, _)| label == "CERTIFICATE")
                .map(|(_, der)| der)
                .ok_or_else(|| CertError::manifest("no CERTIFICATE block in bundle artifact"))?;
            parse_cert(&der)
        };
    let issuer_bundle = load_bundle(&issuer_dir)?;
    let issuer_record = issuer_bundle
        .manifest
        .certificates
        .iter()
        .find(|r| r.role == "issuer" && r.primary && r.algorithm == AlgorithmId::P256)
        .ok_or_else(|| CertError::engine("synthetic issuer has no primary P-256 record"))?;
    let issuer = cert_of(&issuer_bundle, &issuer_record)?;
    let leaf_bundle = load_bundle(&leaf_dir)?;
    let leaf_record = &leaf_bundle.manifest.certificates[0];
    let leaf = cert_of(&leaf_bundle, &leaf_record)?;
    let responder_bundle = load_bundle(&responder_dir)?;
    let responder_record = &responder_bundle.manifest.certificates[0];
    let responder = cert_of(&responder_bundle, &responder_record)?;
    let leaf_serial = unhex(&leaf.serial_hex)?;
    let eku_ok = cert_has_ocsp_signing_eku(&responder.der)?;
    push(
        "responder-profile",
        eku_ok,
        format!(
            "responder certificate {} carries the OCSP-signing EKU: {eku_ok}",
            responder.subject_dn
        ),
    );

    let delegated = OcspSigner::Delegated {
        cert: responder.clone(),
        key_file: "leaf.pk8".to_string(),
        source: SignerSource {
            key_dir: responder_dir.join("private/keys"),
        },
    };
    let base_req = |status: OcspStatus, signer: OcspSigner| OcspSignRequest {
        issuer: issuer.clone(),
        leaf_serial: leaf_serial.clone(),
        status,
        hash: OcspHash::Sha1,
        produced_at: at,
        this_update: at,
        next_update: Some(day.add_to(at).unwrap()),
        signer,
    };
    let all_ok = |cs: &[Check]| cs.iter().all(|c| c.ok);

    let good = sign_response(&base_req(OcspStatus::Good, delegated.clone()))?;
    let vc = verify_response(&good, &issuer, &leaf_serial, at)?;
    push(
        "good-delegated",
        all_ok(&vc),
        "delegated good response verifies end to end".to_string(),
    );

    let revoked = sign_response(&base_req(
        OcspStatus::Revoked {
            revocation_time: Rfc3339(at.0 - time::Duration::days(1)),
            reason: Some(4),
        },
        delegated.clone(),
    ))?;
    let vc = verify_response(&revoked, &issuer, &leaf_serial, at)?;
    let status_named = vc
        .iter()
        .find(|c| c.name == "status")
        .map(|c| c.detail.contains("revoked"))
        .unwrap_or(false);
    push(
        "revoked-delegated",
        all_ok(&vc) && status_named,
        "delegated revoked response verifies; status reads revoked".to_string(),
    );

    let direct = sign_response(&base_req(
        OcspStatus::Good,
        OcspSigner::Direct {
            issuer_record_algorithm: AlgorithmId::P256,
            key_file: "p256-a.pk8".to_string(),
            source: SignerSource {
                key_dir: issuer_dir.join("private/keys"),
            },
        },
    ))?;
    let vc = verify_response(&direct, &issuer, &leaf_serial, at)?;
    push(
        "good-direct",
        all_ok(&vc),
        "issuer-signed (direct) good response verifies".to_string(),
    );

    let mut tampered = good.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 0x01;
    let vc = verify_response(&tampered, &issuer, &leaf_serial, at)?;
    push(
        "tampered-fails",
        !all_ok(&vc),
        "a one-byte tamper in the signature is rejected".to_string(),
    );

    let mut wrong_serial = leaf_serial.clone();
    let n = wrong_serial.len() - 1;
    wrong_serial[n] = wrong_serial[n].wrapping_add(1);
    let vc = verify_response(&good, &issuer, &wrong_serial, at)?;
    let cert_id_failed = vc
        .iter()
        .find(|c| c.name == "cert-id")
        .map(|c| !c.ok)
        .unwrap_or(false);
    push(
        "wrong-serial-fails",
        cert_id_failed,
        "verification against a different leaf serial fails the CertID check".to_string(),
    );

    let stale_at = CalendarDuration::days(30).add_to(at)?;
    let vc = verify_response(&good, &issuer, &leaf_serial, stale_at)?;
    let freshness_failed = vc
        .iter()
        .find(|c| c.name == "freshness")
        .map(|c| !c.ok)
        .unwrap_or(false);
    push(
        "stale-fails",
        freshness_failed,
        "the same response verified 30 days later fails freshness (nextUpdate passed)".to_string(),
    );
    Ok(checks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha1_fips_vectors() {
        assert_eq!(hex(&sha1(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(
            hex(&sha1(b"abc")),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert_eq!(
            hex(&sha1(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
        let long = vec![b'a'; 1_000_000];
        assert_eq!(
            hex(&sha1(&long)),
            "34aa973cd4c4daa4f61eeb2bdbad27316534016f"
        );
    }

    #[test]
    fn oid_round_trip() {
        for oid in [
            vec![1, 3, 6, 1, 5, 5, 7, 48, 1, 1],
            vec![2, 16, 840, 1, 101, 3, 4, 2, 1],
            vec![1, 2, 840, 10045, 4, 3, 2],
            vec![2, 16, 840, 1, 101, 3, 4, 3, 17],
        ] {
            let der = oid_der(&oid);
            let mut r = Reader::new(&der);
            let (tag, content, _) = r.read().unwrap();
            assert_eq!(tag, 0x06);
            assert_eq!(oid_to_string(content).unwrap(), oid_string(&oid));
        }
    }

    #[test]
    fn integer_edges() {
        // High bit set: sign padding appears.
        let der = integer_der(&[0x80]);
        assert_eq!(der, vec![0x02, 0x02, 0x00, 0x80]);
        // Leading zeros stripped.
        let der = integer_der(&[0x00, 0x00, 0x01]);
        assert_eq!(der, vec![0x02, 0x01, 0x01]);
        assert_eq!(serial_magnitude(&[0, 0, 5]), vec![5]);
        assert_eq!(serial_magnitude(&[]), vec![0]);
    }

    #[test]
    fn reasons_parse() {
        assert_eq!(parse_reason("superseded").unwrap(), 4);
        assert_eq!(parse_reason("key-compromise").unwrap(), 1);
        assert_eq!(parse_reason("6").unwrap(), 6);
        assert!(parse_reason("7").is_err());
        assert!(parse_reason("nonsense").is_err());
    }

    #[test]
    fn generalized_time_round_trip() {
        let t = Rfc3339::parse("2026-06-01T12:34:56Z").unwrap();
        let der = generalized_time_der(&t);
        let mut r = Reader::new(&der);
        let (tag, content, _) = r.read().unwrap();
        assert_eq!(tag, 0x18);
        assert_eq!(parse_generalized_time(content).unwrap(), t);
    }
}
