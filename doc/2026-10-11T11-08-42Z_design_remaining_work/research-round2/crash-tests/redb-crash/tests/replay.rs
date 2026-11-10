//! Replays the static crash corpus. Each line is one crash point. Lines start as failures the
//! sweep found, or as representative points kept as anchors. Each line must still produce its
//! recorded verdict. A line that changes verdict is a regression in redb, the harness, or the
//! fault model, and the test says which.

use redb_crash::faultfs::{Plan, Survive, Tear};
use redb_crash::harness::{Verdict, run_one};
use redb_crash::workload;

const CORPUS: &str = include_str!("../corpus/redb-crash-corpus.tsv");

fn parse_survive(s: &str) -> Survive {
    match s {
        "Drop" => Survive::Drop,
        "Keep" => Survive::Keep,
        "Random" => Survive::Random,
        "Reorder" => Survive::Reorder,
        other => panic!("unknown survive policy {other}"),
    }
}

fn parse_tear(s: &str) -> Tear {
    match s {
        "Off" => Tear::Off,
        "Byte" => Tear::Byte,
        "Sector" => Tear::Sector,
        other => panic!("unknown tear mode {other}"),
    }
}

#[test]
fn clean_run_reopens_with_every_transaction() {
    let models = workload::models();
    let (outcome, _) = run_one(None, false, &models);
    assert_eq!(outcome.verdict, Verdict::OkAcked, "{}", outcome.detail);
    assert_eq!(outcome.acked, workload::TXNS);
}

#[test]
fn corpus_replays_with_recorded_verdicts() {
    let models = workload::models();
    let mut replayed = 0;
    for line in CORPUS
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
    {
        let f: Vec<&str> = line.split('\t').collect();
        assert_eq!(f.len(), 8, "bad corpus line: {line}");
        let lying = f[1] == "1";
        let plan = Plan {
            event: f[2].parse().expect("event"),
            survive: parse_survive(f[3]),
            tear: parse_tear(f[4]),
            sync_completed: f[5] == "1",
            seed: f[6].parse().expect("seed"),
        };
        let (outcome, _) = run_one(Some(plan), lying, &models);
        // `ok` accepts either valid prefix. The in-flight commit may or may not be visible, and
        // the choice can differ between debug and release builds (see RESULTS.md).
        match f[7] {
            "ok" => assert!(
                matches!(outcome.verdict, Verdict::OkAcked | Verdict::OkAckedPlusOne),
                "{}: got {} ({}), expected a valid prefix",
                f[0],
                outcome.verdict.name(),
                outcome.detail
            ),
            name => {
                let expect = Verdict::from_name(name).unwrap_or_else(|| panic!("unknown verdict {name}"));
                assert_eq!(
                    outcome.verdict,
                    expect,
                    "{}: got {} ({}), expected {}",
                    f[0],
                    outcome.verdict.name(),
                    outcome.detail,
                    expect.name()
                );
            }
        }
        replayed += 1;
    }
    assert!(replayed >= 10, "corpus has only {replayed} entries");
}
