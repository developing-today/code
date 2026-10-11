//! Replays every checked-in regression case on each run.
//!
//! Each file under `tests/regressions/` is named for its area and holds one
//! case per line: `area<TAB>input<TAB>expected`. Blank lines and lines starting
//! with `#` are skipped. The input uses the area's own syntax; for `tuple` it is
//! two elements separated by a space (see `common::parse_value`), and for
//! `chainext` it is one `b:` element holding the event bytes (see
//! `common::chain`). The only expected result is `ok`: the property must hold
//! for that input. See `tests/regressions/README.md` for how a fuzz failure
//! becomes a case.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::path::{Path, PathBuf};

use common::chain::check_event_bytes;
use common::{Value, check_pair, parse_value};

/// Split one line into its three fields.
fn parse_line(line: &str) -> Result<(&str, &str, &str), String> {
    let fields: Vec<&str> = line.split('\t').collect();
    match fields.as_slice() {
        &[area, input, expected] => Ok((area, input, expected)),
        _ => Err(format!(
            "expected 3 tab-separated fields, found {}",
            fields.len()
        )),
    }
}

/// Run one case against the checker for its area.
fn replay_case(area: &str, input: &str, expected: &str) -> Result<(), String> {
    if expected != "ok" {
        return Err(format!(
            "unknown expected result {expected:?}; only \"ok\" is supported"
        ));
    }
    match area {
        "tuple" => {
            let (left, right) = input.split_once(' ').ok_or_else(|| {
                format!("tuple input {input:?} needs two elements separated by a space")
            })?;
            check_pair(&parse_value(left)?, &parse_value(right)?)
        }
        "chainext" => match parse_value(input)? {
            Value::Bytes(bytes) => check_event_bytes(&bytes),
            Value::Text(_) => Err(format!(
                "chainext input {input:?} must be one b:<escaped> element"
            )),
        },
        other => Err(format!("unknown area {other:?}")),
    }
}

fn regression_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/regressions")
}

#[test]
fn test_every_regression_case_still_passes() {
    let mut files: Vec<PathBuf> = std::fs::read_dir(regression_dir())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "txt"))
        .collect();
    files.sort();
    assert!(
        !files.is_empty(),
        "no regression files under tests/regressions"
    );

    let mut replayed = 0;
    let mut failures = Vec::new();
    for path in &files {
        let file_name = path.file_name().unwrap().to_string_lossy().into_owned();
        let area = path.file_stem().unwrap().to_string_lossy().into_owned();
        let text = std::fs::read_to_string(path).unwrap();
        for (index, line) in text.lines().enumerate() {
            let line = line.trim_end_matches('\r');
            if line.trim().is_empty() || line.starts_with('#') {
                continue;
            }
            replayed += 1;
            let outcome = parse_line(line).and_then(|(case_area, input, expected)| {
                if case_area != area {
                    return Err(format!("area {case_area:?} does not match the file name"));
                }
                replay_case(case_area, input, expected)
            });
            if let Err(error) = outcome {
                failures.push(format!("{file_name}:{}: {error}", index + 1));
            }
        }
    }

    assert!(replayed > 0, "the regression files have no cases");
    assert!(
        failures.is_empty(),
        "{} of {replayed} regression cases failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn test_replay_rejects_malformed_cases() {
    assert!(parse_line("tuple\ts:a s:a").is_err(), "two fields");
    assert!(parse_line("tuple\ts:a\tok\textra").is_err(), "four fields");
    assert!(replay_case("tuple", "s:a", "ok").is_err(), "one element");
    assert!(
        replay_case("tuple", "s:a s:a", "maybe").is_err(),
        "unknown result"
    );
    assert!(
        replay_case("tuple", "s:a s:a b", "ok").is_err(),
        "three elements"
    );
    assert!(
        replay_case("nope", "s:a s:a", "ok").is_err(),
        "unknown area"
    );
    assert!(
        replay_case("chainext", "s:a", "ok").is_err(),
        "chainext takes bytes, not a string"
    );
}

#[test]
fn test_replay_passes_a_good_case_and_rejects_a_bad_escape() {
    assert_eq!(replay_case("tuple", "s:a s:a\\0b", "ok"), Ok(()));
    assert!(
        replay_case("tuple", "s:a\\q s:a", "ok").is_err(),
        "bad escape"
    );
}
