//! `fleet cert algorithms benchmark | doctor` (#13729, spec §6.1).
//!
//! Oracles: the registry (every registered algorithm is measured and
//! probed), the timing invariants of a distribution (min ≤ p50 ≤ p95
//! ≤ p99 ≤ max, sample counts equal the request), known encoded sizes
//! of the registered algorithms (Ed25519 signatures are 64 bytes,
//! ML-DSA-44 signatures 2420 bytes per FIPS 204, RSA signatures the
//! modulus length), and the system `openssl` binary when present.

use fleet_cert::algorithm::{self, AlgorithmId};
use fleet_cert::algorithm_lab::{
    self, OpensslEnv, ProbeStatus, benchmark_algorithm, doctor_algorithm, select_algorithms,
};

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[test]
fn benchmark_measures_every_registered_algorithm() {
    for info in algorithm::registry() {
        let r = benchmark_algorithm(info.id, 2).expect("benchmark runs");
        assert_eq!(r.algorithm, info.name);
        for t in [&r.keygen, &r.sign, &r.verify] {
            assert_eq!(t.samples, 2, "{}", info.name);
            assert!(t.min_us <= t.p50_us, "{}", info.name);
            assert!(t.p50_us <= t.p95_us, "{}", info.name);
            assert!(t.p95_us <= t.p99_us, "{}", info.name);
            assert!(t.p99_us <= t.max_us, "{}", info.name);
            assert!(
                t.min_us <= t.mean_us && t.mean_us <= t.max_us,
                "{}",
                info.name
            );
        }
        assert!(r.private_key_pkcs8_bytes > 0);
        assert!(r.certificate_der_bytes > r.signature_bytes);
        let expected_sig = match info.id {
            AlgorithmId::Ed25519 => Some(64),
            AlgorithmId::MlDsa44 => Some(2420),
            AlgorithmId::Rsa2048 => Some(256),
            AlgorithmId::Rsa4096 => Some(512),
            // ECDSA DER signatures vary by a few bytes.
            AlgorithmId::P256 => None,
        };
        match expected_sig {
            Some(n) => assert_eq!(r.signature_bytes, n, "{}", info.name),
            None => assert!((68..=72).contains(&r.signature_bytes), "{}", info.name),
        }
    }
}

#[test]
fn benchmark_rejects_out_of_range_iterations() {
    assert!(benchmark_algorithm(AlgorithmId::Ed25519, 0).is_err());
    assert!(benchmark_algorithm(AlgorithmId::Ed25519, algorithm_lab::MAX_ITERATIONS + 1).is_err());
}

#[test]
fn doctor_probes_pass_for_every_registered_algorithm() {
    let env = OpensslEnv::detect();
    for info in algorithm::registry() {
        let report = doctor_algorithm(info.id, &env);
        assert_eq!(report.algorithm, info.name);
        let names: Vec<_> = report.probes.iter().map(|p| p.probe).collect();
        assert_eq!(names, ["engine-self-test", "key-reload", "openssl-verify"]);
        assert_eq!(
            report.probes[0].status,
            ProbeStatus::Ok,
            "{}: {:?}",
            info.name,
            report.probes[0]
        );
        assert_eq!(
            report.probes[1].status,
            ProbeStatus::Ok,
            "{}: {:?}",
            info.name,
            report.probes[1]
        );
        // openssl is an independent validator: it must agree when it
        // can speak to the algorithm, and is skipped otherwise.
        let openssl = &report.probes[2];
        if env.version.is_none() || (info.id == AlgorithmId::MlDsa44 && !env.ml_dsa) {
            assert_eq!(
                openssl.status,
                ProbeStatus::Skipped,
                "{}: {openssl:?}",
                info.name
            );
        } else {
            assert_eq!(
                openssl.status,
                ProbeStatus::Ok,
                "{}: {openssl:?}",
                info.name
            );
        }
        assert!(report.ok());
    }
}

#[test]
fn selection_dedupes_and_defaults_to_the_registry() {
    let (all, warnings) = select_algorithms(None).unwrap();
    assert_eq!(all.len(), algorithm::registry().len());
    assert!(warnings.is_empty());
    let (two, _) = select_algorithms(Some("ed25519,ml-dsa-44,ed25519")).unwrap();
    let ids: Vec<_> = two.iter().map(|i| i.id).collect();
    assert_eq!(ids, [AlgorithmId::Ed25519, AlgorithmId::MlDsa44]);
    assert!(select_algorithms(Some("nope")).is_err());
}

#[test]
fn cli_verbs_exit_cleanly() {
    assert_eq!(
        fleet_cert::run(&args(&[
            "algorithms",
            "benchmark",
            "ed25519",
            "--iterations",
            "2",
            "--json"
        ])),
        0
    );
    assert_eq!(
        fleet_cert::run(&args(&["algorithms", "doctor", "ed25519,p256"])),
        0
    );
    // Usage errors exit 2.
    assert_eq!(
        fleet_cert::run(&args(&["algorithms", "benchmark", "--iterations", "0"])),
        2
    );
    assert_eq!(
        fleet_cert::run(&args(&["algorithms", "benchmark", "--iterations", "many"])),
        2
    );
    assert_eq!(fleet_cert::run(&args(&["algorithms", "bogus"])), 2);
}
