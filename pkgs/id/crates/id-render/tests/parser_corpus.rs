#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Parser corpus snapshots for the #5855 conformance suite.
//!
//! Each `.id` source has a sibling `.golden` file containing a stable,
//! target-neutral rendering of the parsed IR. The corpus is append-only:
//! update a golden only when the parser contract intentionally changes.

use std::{fmt::Write as _, fs, path::PathBuf};

use id_render::{Document, Element, Node, parse};

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/conformance/parser")
}

fn golden_document(document: &Document) -> String {
    let mut output = String::new();
    write_element(&mut output, &document.root, 0);
    output
}

fn write_element(output: &mut String, element: &Element, depth: usize) {
    let indent = "  ".repeat(depth);
    let _ = writeln!(
        output,
        "{indent}element <{}> @{}:{}",
        element.name, element.span.line, element.span.col
    );
    for (name, value) in &element.attrs {
        match value {
            Some(value) => {
                let _ = writeln!(output, "{indent}  @{name}={value:?}");
            }
            None => {
                let _ = writeln!(output, "{indent}  @{name}");
            }
        }
    }
    for child in &element.children {
        match child {
            Node::Element(child) => write_element(output, child, depth + 1),
            Node::Text(text) => {
                let _ = writeln!(output, "{indent}  text {text:?}");
            }
            Node::Raw(raw) => {
                let _ = writeln!(
                    output,
                    "{indent}  raw kind={:?} @{}:{} content={:?}",
                    raw.kind, raw.span.line, raw.span.col, raw.content
                );
            }
        }
    }
    let _ = writeln!(output, "{indent}end <{}>", element.name);
}

#[test]
fn parser_corpus_matches_checked_in_goldens() {
    let dir = corpus_dir();
    let mut sources = fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("read parser corpus {}: {error}", dir.display()))
        .map(|entry| entry.expect("valid corpus directory entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "id"))
        .collect::<Vec<_>>();
    sources.sort();
    assert!(
        !sources.is_empty(),
        "parser corpus must contain at least one .id case"
    );

    for source_path in sources {
        let case = source_path
            .file_stem()
            .expect("case has a file stem")
            .to_string_lossy();
        let source = fs::read_to_string(&source_path)
            .unwrap_or_else(|error| panic!("read corpus case {}: {error}", source_path.display()));
        let golden_path = source_path.with_extension("golden");
        let expected = fs::read_to_string(&golden_path).unwrap_or_else(|error| {
            panic!(
                "read golden for {case} at {}: {error}",
                golden_path.display()
            )
        });
        let document =
            parse(&source).unwrap_or_else(|error| panic!("parse corpus case {case}: {error}"));
        assert_eq!(
            golden_document(&document),
            expected,
            "parser golden mismatch: {case}"
        );
    }
}
