//! Probe: parse_records duplicate JSON keys.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use id::world::parse_records;

#[test]
fn probe_records_dup_keys() {
    // Control: a normal object parses, distinct keys kept.
    let ok = parse_records(r#"{"a":1,"b":2}"#).unwrap();
    assert_eq!(ok.len(), 2);

    // Control: a single oversized value is refused (MAX_RECORD_VALUE_BYTES = 8192).
    let big = format!(r#"{{"k":"{}"}}"#, "x".repeat(9000));
    assert!(parse_records(&big).is_err(), "single oversized value refused");

    // Case 1: duplicate key silently collapses, last wins.
    let dup = parse_records(r#"{"a":1,"a":2}"#);
    println!("dup result: {:?}", dup.as_ref().map(|r| r.get("a").cloned()));
    assert!(dup.is_ok(), "BUG-CHECK: duplicate key accepted");
    assert_eq!(dup.unwrap().get("a"), Some(&serde_json::json!(2)));

    // Case 2: oversized first value hidden behind a small duplicate passes the size cap.
    let hidden = format!(r#"{{"k":"{}","k":"small"}}"#, "x".repeat(9000));
    let res = parse_records(&hidden);
    println!("hidden-oversized result ok? {}", res.is_ok());
    assert!(res.is_ok(), "BUG-CHECK: oversized value hidden by duplicate accepted");

    // Case 3: record-count cap checked after collapse: 4097 raw entries, 1 dup -> 4096 kept.
    let mut text = String::from("{");
    for i in 0..4096 {
        text.push_str(&format!(r#""k{i}":1,"#));
    }
    text.push_str(r#""k0":2}"#);
    let res = parse_records(&text);
    println!("count-collapse result ok? {}", res.is_ok());
    assert!(res.is_ok(), "BUG-CHECK: raw count over cap collapsed under it");
}
