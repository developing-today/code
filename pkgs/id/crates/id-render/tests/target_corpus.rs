#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Cross-target structural snapshots for the #5855 conformance corpus.

use std::{collections::BTreeSet, fs, path::PathBuf};

use id_render::{
    compose::{ComposeData, TargetSpec},
    compose_document,
    lower::{emit_json, lower_web},
    lower_gui::{GuiData, emit_gui_json, lower_gui},
    lower_native::{emit_native_json, lower_native},
    parse_and_validate,
};

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/conformance/target-goldens")
}

fn target(name: &str) -> TargetSpec {
    TargetSpec {
        name: name.to_string(),
        form_factor: "desktop".to_string(),
        capabilities: BTreeSet::new(),
    }
}

#[test]
fn one_semantic_source_matches_each_target_golden() {
    let dir = corpus_dir();
    let source = fs::read_to_string(dir.join("basic.id")).expect("read target corpus source");
    let document = parse_and_validate(&source).expect("target corpus source validates");

    let web = compose_document(&document, &target("web"), &ComposeData::default())
        .expect("compose web target");
    let (artefact, report) = lower_web(&document, &web).expect("lower web target");
    let web_output = emit_json(&artefact, &report);
    assert_eq!(
        web_output,
        fs::read_to_string(dir.join("basic.web.golden")).expect("read web golden")
    );

    let native = compose_document(&document, &target("mac"), &ComposeData::default())
        .expect("compose native target");
    let (artefact, report) = lower_native(&document, &native).expect("lower native target");
    let native_output = emit_native_json(&artefact, &report);
    assert_eq!(
        native_output,
        fs::read_to_string(dir.join("basic.mac.golden")).expect("read native golden")
    );

    let gui = compose_document(&document, &target("gui"), &ComposeData::default())
        .expect("compose custom GUI target");
    let (artefact, report) =
        lower_gui(&document, &gui, &GuiData::default()).expect("lower custom GUI target");
    let gui_output = emit_gui_json(&artefact, &report);
    assert_eq!(
        gui_output,
        fs::read_to_string(dir.join("basic.gui.golden")).expect("read GUI golden")
    );
}
