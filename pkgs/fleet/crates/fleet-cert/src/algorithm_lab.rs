//! Per-algorithm measurement and environment probes: the
//! `algorithms benchmark` and `algorithms doctor` verbs (spec §6.1;
//! #13729 under #9157).
//!
//! Benchmarks (spec §§9.2, 26.2): performance is measured, never
//! assumed. For each registered algorithm this times key generation,
//! certificate signing (building and signing one self-signed CA
//! certificate, TBS encoding included) and signature verification
//! (the crate's chain verifier over that certificate), on the current
//! host, and reports min / p50 / p95 / p99 / max / mean in
//! microseconds with the sample count. A report is evidence about one
//! host and one build; it is never a CI baseline or a threshold.
//!
//! Doctor (spec §9.3): reveals what this binary can actually do per
//! algorithm, beyond `fleet cert doctor`'s one-line self-test:
//!
//! - `engine-self-test` — generate, self-sign, verify;
//! - `key-reload` — serialize the key as PKCS#8, reload it through the
//!   same loader a stored CA key goes through, sign with the reloaded
//!   key and verify against the reloaded key's certificate (the path a
//!   real issuer takes on its second signing);
//! - `openssl-verify` — an independent implementation (§11.2) verifies
//!   the self-signed certificate. Skipped when `openssl` is absent,
//!   and for ML-DSA-44 when the local openssl predates ML-DSA (3.5+).
//!
//! Nothing here reads or writes keys outside a private temp file that
//! holds only a public certificate (for the openssl probe).

use crate::algorithm::{self, AlgorithmId, AlgorithmInfo};
use crate::chain::{parse_cert, verify_signature};
use crate::engine::{self, MlDsa44Signer};
use crate::error::{CertError, Result};
use rcgen::{
    BasicConstraints, Certificate, CertificateParams, DnType, IsCa, KeyPair, KeyUsagePurpose,
    PKCS_ECDSA_P256_SHA256, PKCS_ED25519, PKCS_RSA_SHA256, SigningKey,
};
use serde::Serialize;
use std::time::Instant;

/// The default sample count per operation. RSA key generation is
/// slow (hundreds of milliseconds for 4096-bit keys), so the default
/// keeps a full run short; `--iterations` raises it.
pub const DEFAULT_ITERATIONS: usize = 10;

/// The largest sample count a single run accepts.
pub const MAX_ITERATIONS: usize = 1000;

/// One operation's timing distribution, in microseconds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Timing {
    pub samples: usize,
    pub min_us: u64,
    pub p50_us: u64,
    pub p95_us: u64,
    pub p99_us: u64,
    pub max_us: u64,
    pub mean_us: u64,
}

/// The benchmark result for one algorithm.
#[derive(Debug, Clone, Serialize)]
pub struct AlgorithmBenchmark {
    pub algorithm: &'static str,
    pub keygen: Timing,
    pub sign: Timing,
    pub verify: Timing,
    /// Encoded sizes, which drive bundle and handshake size.
    pub private_key_pkcs8_bytes: usize,
    pub certificate_der_bytes: usize,
    pub signature_bytes: usize,
}

/// What the numbers were measured on.
#[derive(Debug, Clone, Serialize)]
pub struct HostInfo {
    pub os: &'static str,
    pub arch: &'static str,
    pub cpus: usize,
    pub backend: &'static str,
    pub debug_build: bool,
}

pub fn host_info() -> HostInfo {
    HostInfo {
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        cpus: std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1),
        backend: "rcgen 0.14 + aws-lc-rs",
        debug_build: cfg!(debug_assertions),
    }
}

/// Nearest-rank percentile over an ascending slice (`p` in 0..=100).
/// An empty slice has no percentile and yields 0.
pub fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let p = p.clamp(0.0, 100.0);
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

/// Summarize raw microsecond samples.
pub fn summarize(mut samples: Vec<u64>) -> Timing {
    samples.sort_unstable();
    let n = samples.len();
    let sum: u128 = samples.iter().map(|&s| s as u128).sum();
    Timing {
        samples: n,
        min_us: samples.first().copied().unwrap_or(0),
        p50_us: percentile(&samples, 50.0),
        p95_us: percentile(&samples, 95.0),
        p99_us: percentile(&samples, 99.0),
        max_us: samples.last().copied().unwrap_or(0),
        mean_us: if n == 0 { 0 } else { (sum / n as u128) as u64 },
    }
}

fn elapsed_us(start: Instant) -> u64 {
    start.elapsed().as_micros().min(u64::MAX as u128) as u64
}

fn self_test_params(cn: &str) -> CertificateParams {
    let mut params = CertificateParams::new(Vec::<String>::new()).expect("empty SAN list");
    params.distinguished_name.push(DnType::CommonName, cn);
    params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let now = crate::timeparse::Rfc3339::now();
    params.not_before = now.0;
    params.not_after = now.0 + time::Duration::days(1);
    params
}

fn self_signed(params: &CertificateParams, key: &KeyPair) -> Result<Certificate> {
    Ok(params.self_signed(key)?)
}

/// Check iterations against the accepted range.
pub fn check_iterations(iterations: usize) -> Result<usize> {
    if iterations == 0 || iterations > MAX_ITERATIONS {
        return Err(CertError::usage(format!(
            "--iterations must be between 1 and {MAX_ITERATIONS}"
        )));
    }
    Ok(iterations)
}

/// Benchmark one algorithm: `iterations` timed samples per operation.
/// Every signed certificate is verified, so a broken engine fails the
/// run instead of reporting a fast wrong answer.
pub fn benchmark_algorithm(algo: AlgorithmId, iterations: usize) -> Result<AlgorithmBenchmark> {
    let iterations = check_iterations(iterations)?;
    if !algo.is_supported() {
        return Err(CertError::engine(format!(
            "{} is not supported by this build's engine",
            algo.name()
        )));
    }
    let mut keygen = Vec::with_capacity(iterations);
    let mut key = None;
    for _ in 0..iterations {
        let start = Instant::now();
        let k = engine::generate_key(algo)?;
        keygen.push(elapsed_us(start));
        key = Some(k);
    }
    let key = key.expect("iterations >= 1");
    let params = self_test_params("fleet cert algorithms benchmark");
    let mut sign = Vec::with_capacity(iterations);
    let mut cert = None;
    for _ in 0..iterations {
        let start = Instant::now();
        let c = self_signed(&params, &key)?;
        sign.push(elapsed_us(start));
        cert = Some(c);
    }
    let cert = cert.expect("iterations >= 1");
    let info = parse_cert(cert.der())?;
    let mut verify = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let start = Instant::now();
        let ok = verify_signature(&info, &info);
        verify.push(elapsed_us(start));
        if !ok {
            return Err(CertError::engine(format!(
                "{} self-signed certificate failed verification during the benchmark",
                algo.name()
            )));
        }
    }
    Ok(AlgorithmBenchmark {
        algorithm: algo.name(),
        keygen: summarize(keygen),
        sign: summarize(sign),
        verify: summarize(verify),
        private_key_pkcs8_bytes: key.serialize_der().len(),
        certificate_der_bytes: cert.der().len(),
        signature_bytes: info.signature().len(),
    })
}

// ---------------------------------------------------------------- doctor

/// One probe's outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProbeStatus {
    Ok,
    Fail,
    Skipped,
}

#[derive(Debug, Clone, Serialize)]
pub struct Probe {
    pub probe: &'static str,
    pub status: ProbeStatus,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AlgorithmDoctor {
    pub algorithm: &'static str,
    pub probes: Vec<Probe>,
}

impl AlgorithmDoctor {
    pub fn ok(&self) -> bool {
        self.probes.iter().all(|p| p.status != ProbeStatus::Fail)
    }
}

/// What the local openssl can do, read once per doctor run.
#[derive(Debug, Clone, Default, Serialize)]
pub struct OpensslEnv {
    pub version: Option<String>,
    pub ml_dsa: bool,
}

impl OpensslEnv {
    pub fn detect() -> Self {
        let version = std::process::Command::new("openssl")
            .arg("version")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
        let ml_dsa = version.is_some()
            && std::process::Command::new("openssl")
                .args(["list", "-signature-algorithms"])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).contains("ML-DSA"))
                .unwrap_or(false);
        OpensslEnv { version, ml_dsa }
    }
}

fn probe(name: &'static str, result: std::result::Result<String, String>) -> Probe {
    match result {
        Ok(detail) => Probe {
            probe: name,
            status: ProbeStatus::Ok,
            detail,
        },
        Err(detail) => Probe {
            probe: name,
            status: ProbeStatus::Fail,
            detail,
        },
    }
}

fn skipped(name: &'static str, detail: impl Into<String>) -> Probe {
    Probe {
        probe: name,
        status: ProbeStatus::Skipped,
        detail: detail.into(),
    }
}

/// Generate, self-sign and verify. Returns the key and certificate
/// for the later probes.
fn engine_probe(algo: AlgorithmId) -> std::result::Result<(KeyPair, Certificate), String> {
    let key = engine::generate_key(algo).map_err(|e| format!("key generation failed: {e}"))?;
    let cert = self_signed(&self_test_params("fleet cert algorithms doctor"), &key)
        .map_err(|e| format!("self-signing failed: {e}"))?;
    let info = parse_cert(cert.der()).map_err(|e| format!("certificate did not parse: {e}"))?;
    if !verify_signature(&info, &info) {
        return Err("self-signed certificate failed signature verification".to_string());
    }
    Ok((key, cert))
}

/// The engine's one-line self-test, as `fleet cert doctor` reports it.
pub fn engine_self_test(algo: AlgorithmId) -> bool {
    engine_probe(algo).is_ok()
}

fn sign_and_verify(
    params: &CertificateParams,
    signer: &impl SigningKey,
) -> std::result::Result<(), String> {
    let cert = params
        .self_signed(signer)
        .map_err(|e| format!("signing with the reloaded key failed: {e}"))?;
    let info = parse_cert(cert.der()).map_err(|e| format!("certificate did not parse: {e}"))?;
    if !verify_signature(&info, &info) {
        return Err("certificate signed by the reloaded key failed verification".to_string());
    }
    Ok(())
}

fn reload_probe(algo: AlgorithmId, key: &KeyPair) -> std::result::Result<String, String> {
    let der = key.serialize_der();
    let params = self_test_params("fleet cert algorithms doctor reload");
    let sign_alg: &'static rcgen::SignatureAlgorithm = match algo {
        AlgorithmId::MlDsa44 => {
            let signer = MlDsa44Signer::from_pkcs8(&der)
                .map_err(|e| format!("PKCS#8 reload failed: {e}"))?;
            sign_and_verify(&params, &signer)?;
            return Ok(format!("PKCS#8 ({} bytes) reloads and signs", der.len()));
        }
        AlgorithmId::Ed25519 => &PKCS_ED25519,
        AlgorithmId::P256 => &PKCS_ECDSA_P256_SHA256,
        AlgorithmId::Rsa4096 | AlgorithmId::Rsa2048 => &PKCS_RSA_SHA256,
    };
    let reloaded = KeyPair::from_pkcs8_der_and_sign_algo(
        &rustls_pki_types::PrivatePkcs8KeyDer::from(der.clone()),
        sign_alg,
    )
    .map_err(|e| format!("PKCS#8 reload failed: {e}"))?;
    sign_and_verify(&params, &reloaded)?;
    Ok(format!("PKCS#8 ({} bytes) reloads and signs", der.len()))
}

fn openssl_probe(algo: AlgorithmId, cert: &Certificate, env: &OpensslEnv) -> Probe {
    const NAME: &str = "openssl-verify";
    let Some(version) = &env.version else {
        return skipped(NAME, "openssl not found");
    };
    if algo == AlgorithmId::MlDsa44 && !env.ml_dsa {
        return skipped(
            NAME,
            format!("{version} has no ML-DSA (needs OpenSSL 3.5+)"),
        );
    }
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!(
        "fleet-cert-doctor-{}-{}-{nonce}.pem",
        std::process::id(),
        algo.name()
    ));
    if let Err(e) = std::fs::write(&path, cert.pem()) {
        return skipped(NAME, format!("could not write a temp certificate: {e}"));
    }
    let out = std::process::Command::new("openssl")
        .arg("verify")
        .arg("-CAfile")
        .arg(&path)
        .arg(&path)
        .output();
    let _ = std::fs::remove_file(&path);
    match out {
        Ok(o) if o.status.success() => Probe {
            probe: NAME,
            status: ProbeStatus::Ok,
            detail: format!("{version} verifies the self-signed certificate"),
        },
        Ok(o) => Probe {
            probe: NAME,
            status: ProbeStatus::Fail,
            detail: format!(
                "{version} rejects the certificate: {}",
                String::from_utf8_lossy(&o.stderr)
                    .lines()
                    .chain(String::from_utf8_lossy(&o.stdout).lines())
                    .find(|l| l.contains("error"))
                    .unwrap_or("verify failed")
                    .trim()
            ),
        },
        Err(e) => skipped(NAME, format!("openssl did not run: {e}")),
    }
}

/// Run every probe for one algorithm. Probes after a failed engine
/// probe are skipped: there is no key or certificate to test.
pub fn doctor_algorithm(algo: AlgorithmId, env: &OpensslEnv) -> AlgorithmDoctor {
    let mut probes = Vec::new();
    if !algo.is_supported() {
        probes.push(skipped(
            "engine-self-test",
            "not supported by this build's engine",
        ));
        return AlgorithmDoctor {
            algorithm: algo.name(),
            probes,
        };
    }
    match engine_probe(algo) {
        Ok((key, cert)) => {
            probes.push(probe(
                "engine-self-test",
                Ok("generate, self-sign and verify succeed".to_string()),
            ));
            probes.push(probe("key-reload", reload_probe(algo, &key)));
            probes.push(openssl_probe(algo, &cert, env));
        }
        Err(detail) => {
            probes.push(probe("engine-self-test", Err(detail)));
            probes.push(skipped("key-reload", "engine self-test failed"));
            probes.push(skipped("openssl-verify", "engine self-test failed"));
        }
    }
    AlgorithmDoctor {
        algorithm: algo.name(),
        probes,
    }
}

/// Resolve an optional comma-separated algorithm selection; none
/// means every registered algorithm, in registry order.
pub fn select_algorithms(selection: Option<&str>) -> Result<(Vec<AlgorithmInfo>, Vec<String>)> {
    match selection {
        None => Ok((algorithm::registry(), Vec::new())),
        Some(csv) => {
            let (algos, warnings) = algorithm::parse_algorithm_list(csv)?;
            let mut seen = std::collections::BTreeSet::new();
            let infos = algos
                .into_iter()
                .filter(|a| seen.insert(*a))
                .map(|a| a.info())
                .collect();
            Ok((infos, warnings))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_is_nearest_rank() {
        let s: Vec<u64> = (1..=100).collect();
        assert_eq!(percentile(&s, 50.0), 50);
        assert_eq!(percentile(&s, 95.0), 95);
        assert_eq!(percentile(&s, 99.0), 99);
        assert_eq!(percentile(&s, 100.0), 100);
        assert_eq!(percentile(&s, 0.0), 1);
        assert_eq!(percentile(&[7], 99.0), 7);
        assert_eq!(percentile(&[], 50.0), 0);
        // Four samples: p50 is the 2nd, p95 and p99 the 4th.
        let four = [10, 20, 30, 40];
        assert_eq!(percentile(&four, 50.0), 20);
        assert_eq!(percentile(&four, 95.0), 40);
    }

    #[test]
    fn summarize_orders_and_averages() {
        let t = summarize(vec![30, 10, 20, 40]);
        assert_eq!(
            t,
            Timing {
                samples: 4,
                min_us: 10,
                p50_us: 20,
                p95_us: 40,
                p99_us: 40,
                max_us: 40,
                mean_us: 25,
            }
        );
        assert_eq!(summarize(Vec::new()).samples, 0);
    }

    #[test]
    fn iterations_are_bounded() {
        assert!(check_iterations(0).is_err());
        assert!(check_iterations(MAX_ITERATIONS + 1).is_err());
        assert_eq!(check_iterations(3).unwrap(), 3);
    }

    #[test]
    fn selection_dedupes_and_defaults_to_the_registry() {
        let (all, _) = select_algorithms(None).unwrap();
        assert_eq!(all.len(), algorithm::registry().len());
        let (two, _) = select_algorithms(Some("ed25519,p256,ed25519")).unwrap();
        assert_eq!(
            two.iter().map(|i| i.id).collect::<Vec<_>>(),
            vec![AlgorithmId::Ed25519, AlgorithmId::P256]
        );
        assert!(select_algorithms(Some("dsa-1024")).is_err());
    }
}
