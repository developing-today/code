//! Coverage oracle for slice 9110-13 (issue #10177): every normal
//! retained-data path in `tests/operation-path-inventory.md` must map to
//! a fixed gate and a bypass test, and any unmapped path blocks the
//! completeness claim.
//!
//! The test parses the inventory's machine-readable table and asserts:
//! 1. The inventory's path-ID set equals the canonical list below — a
//!    new path must be added to both, in the same change.
//! 2. Every row carries every field (path, entry point, adapter, gate,
//!    hooks, bypass test, recovery semantics, status).
//! 3. Every named entry point still exists: the file is present and
//!    contains the named symbol.
//! 4. Every named bypass test file exists; a row whose bypass test is
//!    MISSING may not claim `gated` status.
//! 5. Completeness consistency: while any row is not `gated`, the
//!    inventory must state "Completeness: NOT COMPLETE".

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Canonical path IDs for the 9110-13 inventory. Keep in exact sync
/// with `tests/operation-path-inventory.md`; the test fails on any
/// divergence in either direction.
const CANONICAL_PATH_IDS: &[&str] = &[
    "P-BU-01",
    "P-ING-01",
    "P-ING-02",
    "P-ING-03",
    "P-ING-04",
    "P-ING-05",
    "P-ING-06",
    "P-PORT-01",
    "P-QY-01",
    "P-RD-01",
    "P-RD-02",
    "P-SY-01",
    "P-SY-02",
    "P-WR-01",
    "P-WR-02",
    "P-WR-03",
    "P-WR-04",
];

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn inventory_text() -> String {
    let path = manifest_dir().join("tests/operation-path-inventory.md");
    fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("inventory must exist at {}: {e}", path.display()))
}

struct Row {
    id: String,
    class: String,
    path: String,
    entry: String,
    adapter: String,
    gate: String,
    hooks: String,
    bypass: String,
    recovery: String,
    status: String,
}

fn parse_rows(text: &str) -> Vec<Row> {
    let mut rows = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with("| P-") {
            continue;
        }
        let cells: Vec<String> = line
            .trim_matches('|')
            .split('|')
            .map(|c| c.trim().to_string())
            .collect();
        assert_eq!(
            cells.len(),
            10,
            "inventory row must have exactly 10 cells: {line}"
        );
        rows.push(Row {
            id: cells[0].clone(),
            class: cells[1].clone(),
            path: cells[2].clone(),
            entry: cells[3].clone(),
            adapter: cells[4].clone(),
            gate: cells[5].clone(),
            hooks: cells[6].clone(),
            bypass: cells[7].clone(),
            recovery: cells[8].clone(),
            status: cells[9].clone(),
        });
    }
    rows
}

#[test]
fn inventory_covers_exactly_the_canonical_paths() {
    let rows = parse_rows(&inventory_text());
    let found: BTreeSet<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    let canonical: BTreeSet<&str> = CANONICAL_PATH_IDS.iter().copied().collect();
    assert_eq!(
        found.len(),
        rows.len(),
        "duplicate path IDs in the inventory"
    );
    assert_eq!(
        found, canonical,
        "inventory path IDs diverge from the canonical list: \
         add new paths to both the inventory and CANONICAL_PATH_IDS"
    );
}

#[test]
fn every_row_is_complete_and_anchored_to_source() {
    let root = manifest_dir();
    for row in parse_rows(&inventory_text()) {
        for (field, value) in [
            ("class", &row.class),
            ("path", &row.path),
            ("entry point", &row.entry),
            ("adapter", &row.adapter),
            ("gate", &row.gate),
            ("hooks", &row.hooks),
            ("bypass test", &row.bypass),
            ("recovery", &row.recovery),
            ("status", &row.status),
        ] {
            assert!(
                !value.is_empty(),
                "{}: field {field} must not be empty",
                row.id
            );
        }
        assert!(
            [
                "ingress", "read", "write", "sync", "query", "stream", "bucket"
            ]
            .contains(&row.class.as_str()),
            "{}: unknown path class {}",
            row.id,
            row.class
        );
        assert!(
            ["gated", "partial", "unmigrated"].contains(&row.status.as_str()),
            "{}: unknown status {}",
            row.id,
            row.status
        );

        // Entry point anchor: `relative/path.rs :: symbol`.
        let (file, symbol) = row
            .entry
            .split_once("::")
            .unwrap_or_else(|| panic!("{}: entry point must be `file :: symbol`", row.id));
        let file = file.trim();
        let symbol = symbol.trim();
        let source_path = root.join(file);
        let source = fs::read_to_string(&source_path).unwrap_or_else(|e| {
            panic!(
                "{}: entry point file {} must exist: {e}",
                row.id,
                source_path.display()
            )
        });
        assert!(
            source.contains(symbol),
            "{}: entry point symbol `{symbol}` not found in {file}",
            row.id
        );

        // Bypass test anchor: an existing test file, or MISSING — and a
        // MISSING bypass test forbids claiming the path is gated.
        if row.bypass == "MISSING" {
            assert_ne!(
                row.status, "gated",
                "{}: a path with no bypass test cannot claim gated status",
                row.id
            );
        } else {
            let test_path = root.join(&row.bypass);
            assert!(
                Path::new(&test_path).exists(),
                "{}: bypass test {} must exist",
                row.id,
                row.bypass
            );
        }
    }
}

#[test]
fn completeness_claim_matches_row_statuses() {
    let text = inventory_text();
    let rows = parse_rows(&text);
    let all_gated = rows.iter().all(|r| r.status == "gated");
    if all_gated {
        assert!(
            !text.contains("Completeness: NOT COMPLETE"),
            "all paths are gated but the inventory still blocks the completeness claim"
        );
    } else {
        assert!(
            text.contains("Completeness: NOT COMPLETE"),
            "unmigrated or partial paths exist, so the inventory must block \
             the completeness claim with `Completeness: NOT COMPLETE`"
        );
    }
}
