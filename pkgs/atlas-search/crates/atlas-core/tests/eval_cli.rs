//! Oracles: hand-counted Precision@k ratios and the original report serializer.

use std::process::Command;

use atlas_core::eval::{EvalData, Split, evaluate};

#[test]
fn eval_cli_precision_and_original_output() {
    let input = serde_json::json!({
        "format": "atlas-eval/v1",
        "queries": [
            {"id": "dev", "class": "code", "split": "dev"},
            {"id": "empty", "class": "code", "split": "dev"},
            {"id": "test", "class": "code", "split": "test"}
        ],
        "qrels": [
            {"query": "dev", "doc": "hit", "grade": 1},
            {"query": "test", "doc": "hit", "grade": 1}
        ],
        "runs": [{"signal": "s", "results": {"dev": ["hit"], "test": ["hit"]}}]
    })
    .to_string();
    let path = std::env::temp_dir().join(format!("atlas-eval-cli-{}.json", std::process::id()));
    std::fs::write(&path, &input).unwrap();
    let binary = env!("CARGO_BIN_EXE_atlas-eval");

    // Oracle: the original command emits exactly the original report's
    // pretty JSON, with its trailing newline, and retains its verdict.
    let original = Command::new(binary).arg(&path).arg("dev").output().unwrap();
    assert!(original.status.success());
    let data = EvalData::from_json(&input).unwrap();
    let expected = format!(
        "{}\n",
        serde_json::to_string_pretty(&evaluate(&data, Split::Dev).unwrap()).unwrap()
    );
    assert_eq!(original.stdout, expected.as_bytes());

    // Oracle: dev contains one hit and one missing ranking; each has
    // denominator 3. The held-out query cannot affect the 1/6 dev mean.
    let precision = Command::new(binary)
        .args(["--precision-at-k", "3"])
        .arg(&path)
        .arg("dev")
        .output()
        .unwrap();
    assert!(precision.status.success());
    let report: serde_json::Value = serde_json::from_slice(&precision.stdout).unwrap();
    assert_eq!(report["format"], "atlas-eval-precision-report/v1");
    assert_eq!(report["k"], 3);
    let signal = &report["signals"][0];
    assert_eq!(signal["queries_total"], 2);
    assert_eq!(signal["precision"], 0.166667);
    assert_eq!(signal["per_class"]["code"], 0.166667);
    assert_eq!(signal["per_query"][0]["query"], "dev");
    assert_eq!(signal["per_query"][0]["hits"], 1);
    assert_eq!(signal["per_query"][0]["denominator"], 3);
    assert_eq!(signal["per_query"][1]["precision"], 0.0);

    // Oracle: invalid depths are usage errors, before reading the input.
    for k in ["0", "1001", "-1", "nope"] {
        let invalid = Command::new(binary)
            .args(["--precision-at-k", k, "missing.json", "dev"])
            .output()
            .unwrap();
        assert_eq!(invalid.status.code(), Some(2));
        assert!(invalid.stdout.is_empty());
    }
    std::fs::remove_file(path).unwrap();
}
