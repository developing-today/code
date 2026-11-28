//! Web/terminal parity harness for #796 item 9-2 (issue #8340).
//!
//! The checked-in corpus at `pkgs/id/test-data/screen-parity-corpus.json`
//! is the oracle: its verdicts were generated from the real Rust
//! presenter (`ScreenSurface::text_projection` plus the per-cell
//! fg/bg/bold the terminal frame renders), never hand-written. This
//! test asserts the Rust presenter still produces exactly the corpus
//! verdicts; the vitest twin (`web/src/world-surface.parity.test.ts`)
//! asserts the web renderer produces the same verdicts from the same
//! corpus, so the two presenters cannot drift apart silently.
//!
//! Regenerate the corpus after an intentional presenter change with:
//! `SCREEN_PARITY_GENERATE=1 cargo test -p id-core --test screen_parity`
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use id_core::world_presentation::{ScreenCell, ScreenSurface, StyleToken};
use serde_json::{Value, json};

fn cell(symbol: &str, fg: u8, bg: u8, bold: bool, columns: u8) -> ScreenCell {
    ScreenCell {
        symbol: symbol.to_owned(),
        fg,
        bg,
        bold,
        columns,
        token: None,
    }
}

fn token_cell(symbol: &str, fg: u8, bg: u8, bold: bool, token: StyleToken) -> ScreenCell {
    ScreenCell {
        symbol: symbol.to_owned(),
        fg,
        bg,
        bold,
        columns: 1,
        token: Some(token),
    }
}

fn blank() -> ScreenCell {
    cell(" ", 0, 0, false, 1)
}

fn blanks(n: usize) -> Vec<ScreenCell> {
    (0..n).map(|_| blank()).collect()
}

/// The corpus cases: (name, width, height, input cells). Inputs only —
/// every verdict in the corpus file is presenter-generated.
fn cases() -> Vec<(String, usize, usize, Vec<ScreenCell>)> {
    let mut plain = vec![cell("H", 0, 0, false, 1), cell("i", 0, 0, false, 1)];
    plain.extend(blanks(6));
    plain.extend(blanks(2));
    plain.extend([cell("o", 0, 0, false, 1), cell("k", 0, 0, false, 1)]);
    plain.extend(blanks(4));

    let styled = vec![
        cell("R", 196, 0, true, 1),
        cell("G", 46, 0, false, 1),
        cell("B", 0, 21, false, 1),
        token_cell("W", 11, 0, true, StyleToken::Warning),
        token_cell("C", 14, 0, false, StyleToken::Code),
        blank(),
    ];

    let mut wide = vec![
        cell("a", 0, 0, false, 1),
        cell("\u{754c}", 0, 0, false, 2),
        cell("", 0, 0, false, 0),
        cell("b", 0, 0, false, 1),
    ];
    wide.extend(blanks(2));

    let emoji = vec![
        cell("\u{1f600}", 0, 0, false, 2),
        cell("", 0, 0, false, 0),
        cell("\u{2708}\u{fe0f}", 0, 0, false, 2),
        cell("", 0, 0, false, 0),
        cell("z", 0, 0, false, 1),
    ];

    let mut mixed = vec![
        cell("\u{754c}", 9, 0, false, 2),
        cell("", 9, 0, false, 0),
        cell("x", 0, 0, false, 1),
        cell("!", 10, 0, true, 1),
    ];
    mixed.extend(blanks(6));
    mixed.extend([
        token_cell("d", 10, 0, false, StyleToken::Success),
        cell("o", 0, 0, false, 1),
        cell("n", 0, 0, false, 1),
        cell("e", 0, 0, false, 1),
    ]);
    mixed.extend(blanks(6));

    vec![
        ("plain_text".to_owned(), 8, 2, plain),
        ("styled_row".to_owned(), 6, 1, styled),
        ("wide_glyph".to_owned(), 6, 1, wide),
        ("emoji_sequences".to_owned(), 5, 1, emoji),
        ("blank_screen".to_owned(), 4, 2, blanks(8)),
        ("mixed_rows".to_owned(), 10, 2, mixed),
    ]
}

/// The presenter's verdict for one surface: the text projection plus
/// every presented (non-continuation) cell's symbol and effective
/// style, in row-major order — exactly what the web test extracts
/// from the rendered DOM.
fn verdict(surface: &ScreenSurface) -> Value {
    let width = usize::from(surface.width);
    let mut cells = Vec::new();
    for (index, c) in surface.cells.iter().enumerate() {
        if c.columns == 0 {
            continue;
        }
        cells.push(json!({
            "symbol": c.symbol,
            "fg": c.fg,
            "bg": c.bg,
            "bold": c.bold,
            "columns": c.columns,
            "row": index / width,
            "col": index % width,
        }));
    }
    json!({ "text": surface.text_projection(), "cells": cells })
}

fn corpus_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test-data/screen-parity-corpus.json")
}

fn build_cases() -> Vec<Value> {
    cases()
        .into_iter()
        .map(|(name, width, height, cells)| {
            let surface =
                ScreenSurface::from_cells(width, height, cells).expect("corpus case must build");
            json!({
                "name": name,
                "width": width,
                "height": height,
                "cells": serde_json::to_value(&surface.cells).unwrap(),
                "verdict": verdict(&surface),
            })
        })
        .collect()
}

#[test]
fn screen_parity_corpus_matches_rust_presenter() {
    let built = build_cases();
    if std::env::var_os("SCREEN_PARITY_GENERATE").is_some() {
        let corpus = json!({
            "oracle": "Rust presenter id_core::world_presentation::ScreenSurface: text_projection + per-cell fg/bg/bold (#8340, #796 item 9-2). Generated by crates/id-core/tests/screen_parity.rs; do not hand-edit verdicts.",
            "cases": built,
        });
        let path = corpus_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut text = serde_json::to_string_pretty(&corpus).unwrap();
        text.push('\n');
        std::fs::write(&path, text).unwrap();
        return;
    }
    let raw = std::fs::read_to_string(corpus_path())
        .expect("screen-parity corpus must be checked in at pkgs/id/test-data/");
    let corpus: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(
        corpus["cases"],
        json!(built),
        "Rust presenter verdicts drifted from the checked-in parity corpus; \
         if the change is intentional, regenerate with SCREEN_PARITY_GENERATE=1 \
         and record the divergence on #8340"
    );
    // The presenter must also be able to frame every corpus screen.
    for (name, width, height, cells) in cases() {
        let surface = ScreenSurface::from_cells(width, height, cells).unwrap();
        assert!(surface.render_ansi().is_some(), "case {name} must render");
    }
}
