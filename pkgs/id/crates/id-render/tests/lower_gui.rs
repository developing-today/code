#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Behaviour tests for stages 4–5 (lower + emit, custom-GUI
//! target). Each test names the decision-record behaviour it
//! verifies: #5853 supplies the presentation-override contract,
//! #5855 the pipeline rules.

use id_render::compose::{ComposedDocument, ComposedElement, ComposedNode, CompositionReport};
use id_render::lower_gui::{
    GuiData, GuiNode, GuiOverrideDef, GuiPresentation, PresentationKind, emit_gui_json, lower_gui,
};
use id_render::{Span, parse_and_validate};

/// The worked example from #5849 §2.1 (same source as
/// tests/lower.rs).
const EXAMPLE: &str = r#"<document version="1">
  <page title="Account settings">
    <heading level="1">Account settings</heading>
    <form action="save-account">
      <field kind="text" label="Display name" bind="account.name" required></field>
      <field kind="choice" label="Language" bind="account.language"
             options="state.languages"></field>
      <alert kind="error" show="account.errors.name">Name is required.</alert>
      <button action="save-account" confirm="Save account changes?">Save</button>
    </form>
  </page>
</document>"#;

fn el(path: &str, name: &str, children: Vec<ComposedNode>) -> ComposedElement {
    ComposedElement {
        path: path.to_string(),
        name: name.to_string(),
        span: Span::default(),
        presentation: None,
        properties: Vec::new(),
        children,
    }
}

fn text(value: &str) -> ComposedNode {
    ComposedNode::Text(value.to_string())
}

const BUTTON_PATH: &str = "document/page[0]/form[0]/button[0]";
const ALL_BUTTON_STATES: &[&str] = &["default", "hover", "focus", "active", "disabled"];

fn composed(button_presentation: Option<&str>) -> ComposedDocument {
    let mut button = el(BUTTON_PATH, "button", vec![text("Save")]);
    button.presentation = button_presentation.map(ToString::to_string);
    ComposedDocument {
        target: "gui".to_string(),
        root: el(
            "document",
            "document",
            vec![ComposedNode::Element(el(
                "document/page[0]",
                "page",
                vec![
                    ComposedNode::Element(el(
                        "document/page[0]/heading[0]",
                        "heading",
                        vec![text("Account settings")],
                    )),
                    ComposedNode::Element(el(
                        "document/page[0]/form[0]",
                        "form",
                        vec![
                            ComposedNode::Element(el(
                                "document/page[0]/form[0]/field[0]",
                                "field",
                                vec![],
                            )),
                            ComposedNode::Element(el(
                                "document/page[0]/form[0]/field[1]",
                                "field",
                                vec![],
                            )),
                            ComposedNode::Element(el(
                                "document/page[0]/form[0]/alert[0]",
                                "alert",
                                vec![text("Name is required.")],
                            )),
                            ComposedNode::Element(button),
                        ],
                    )),
                ],
            ))],
        ),
        report: CompositionReport::default(),
    }
}

fn asset_def(name: &str, asset: &str, covered: &[&str]) -> GuiOverrideDef {
    GuiOverrideDef {
        name: name.to_string(),
        kind: PresentationKind::Asset,
        asset: Some(asset.to_string()),
        renderer: None,
        covered_states: covered.iter().map(ToString::to_string).collect(),
        derivations: Vec::new(),
    }
}

fn drawn_def(name: &str, renderer: &str, covered: &[&str]) -> GuiOverrideDef {
    GuiOverrideDef {
        name: name.to_string(),
        kind: PresentationKind::Drawn,
        asset: None,
        renderer: Some(renderer.to_string()),
        covered_states: covered.iter().map(ToString::to_string).collect(),
        derivations: Vec::new(),
    }
}

fn find<'a>(
    node: &'a id_render::lower_gui::GuiElement,
    kind: &str,
) -> Option<&'a id_render::lower_gui::GuiElement> {
    if node.kind == kind {
        return Some(node);
    }
    for child in &node.children {
        if let GuiNode::Element(child_el) = child {
            if let Some(found) = find(child_el, kind) {
                return Some(found);
            }
        }
    }
    None
}

#[test]
fn element_without_override_renders_standard_decision_5853_s2_1() {
    // #5853 §2.1: the same source renders as a standard control
    // on a target with no override entry for the element.
    let doc = parse_and_validate(EXAMPLE).expect("example validates");
    let (artefact, report) =
        lower_gui(&doc, &composed(None), &GuiData::default()).expect("lowering succeeds");
    assert!(matches!(
        find(&artefact.root, "button").expect("button").presentation,
        GuiPresentation::Standard
    ));
    assert!(report.applied.is_empty() && report.fallbacks.is_empty());
}

#[test]
fn asset_override_applies_with_per_state_assets_decision_5853_s2_2() {
    // #5853 §2.2: an asset presentation carries the asset plus
    // per-state coverage; the fallback is declared as standard.
    let doc = parse_and_validate(EXAMPLE).expect("example validates");
    let data = GuiData {
        overrides: vec![asset_def(
            "img-save",
            "assets/save-button",
            ALL_BUTTON_STATES,
        )],
        available_assets: vec!["assets/save-button".to_string()],
        ..GuiData::default()
    };
    let (artefact, report) =
        lower_gui(&doc, &composed(Some("img-save")), &data).expect("lowering succeeds");
    let button = find(&artefact.root, "button").expect("button");
    match &button.presentation {
        GuiPresentation::Asset {
            name,
            asset,
            states,
        } => {
            assert_eq!(name, "img-save");
            assert_eq!(asset, "assets/save-button");
            assert_eq!(states.len(), 5);
            assert!(states.iter().all(|(_, how)| how == "explicit"));
        }
        other => panic!("expected asset presentation, got {other:?}"),
    }
    assert_eq!(report.applied.len(), 1);
    assert_eq!(report.applied[0].kind, "asset");
    assert!(report.fallbacks.is_empty());
}

#[test]
fn drawn_override_applies_with_registered_renderer_decision_5853_s2_2() {
    // #5853 §2.2: a drawn presentation is a renderer registered
    // by name in the override pack.
    let doc = parse_and_validate(EXAMPLE).expect("example validates");
    let data = GuiData {
        overrides: vec![drawn_def(
            "drawn-save",
            "save_button_renderer",
            ALL_BUTTON_STATES,
        )],
        registered_renderers: vec!["save_button_renderer".to_string()],
        ..GuiData::default()
    };
    let (artefact, report) =
        lower_gui(&doc, &composed(Some("drawn-save")), &data).expect("lowering succeeds");
    match &find(&artefact.root, "button").expect("button").presentation {
        GuiPresentation::Drawn { renderer, .. } => {
            assert_eq!(renderer, "save_button_renderer");
        }
        other => panic!("expected drawn presentation, got {other:?}"),
    }
    assert_eq!(report.applied[0].kind, "drawn");
}

#[test]
fn unregistered_renderer_is_a_build_error_decision_5853_s2_2() {
    // #5853 §2.2: an unregistered renderer name is a build-time
    // error (a reference error — distinct from a load failure,
    // which exercises the fallback).
    let doc = parse_and_validate(EXAMPLE).expect("example validates");
    let data = GuiData {
        overrides: vec![drawn_def("drawn-save", "ghost_renderer", ALL_BUTTON_STATES)],
        ..GuiData::default()
    };
    let errors = lower_gui(&doc, &composed(Some("drawn-save")), &data)
        .expect_err("unregistered renderer must fail the build");
    assert!(
        errors[0].message.contains("ghost_renderer") && errors[0].message.contains("not"),
        "{}",
        errors[0]
    );
}

#[test]
fn partial_state_coverage_is_a_build_error_decision_5853_s2_3() {
    // #5853 §2.3: an override must cover every interaction state
    // of the element; anything less fails the build with the
    // missing states named.
    let doc = parse_and_validate(EXAMPLE).expect("example validates");
    let data = GuiData {
        overrides: vec![asset_def(
            "img-save",
            "assets/save-button",
            &["default", "hover"],
        )],
        available_assets: vec!["assets/save-button".to_string()],
        ..GuiData::default()
    };
    let errors = lower_gui(&doc, &composed(Some("img-save")), &data)
        .expect_err("partial coverage must fail the build");
    for missing in ["focus", "active", "disabled"] {
        assert!(
            errors[0].message.contains(missing),
            "missing state {missing} not named: {}",
            errors[0]
        );
    }
}

#[test]
fn declared_derivations_complete_state_coverage_decision_5853_s2_3() {
    // #5853 §2.3: a state may be satisfied by a declared
    // derivation instead of an explicit asset.
    let doc = parse_and_validate(EXAMPLE).expect("example validates");
    let mut def = asset_def(
        "img-save",
        "assets/save-button",
        &["default", "hover", "focus", "active"],
    );
    def.derivations = vec![(
        "disabled".to_string(),
        "default asset at the disabled token's opacity".to_string(),
    )];
    let data = GuiData {
        overrides: vec![def],
        available_assets: vec!["assets/save-button".to_string()],
        ..GuiData::default()
    };
    let (artefact, _) =
        lower_gui(&doc, &composed(Some("img-save")), &data).expect("lowering succeeds");
    match &find(&artefact.root, "button").expect("button").presentation {
        GuiPresentation::Asset { states, .. } => {
            let disabled = states
                .iter()
                .find(|(s, _)| s == "disabled")
                .expect("disabled state");
            assert!(
                disabled.1.contains("disabled token's opacity"),
                "{}",
                disabled.1
            );
        }
        other => panic!("expected asset presentation, got {other:?}"),
    }
}

#[test]
fn missing_asset_exercises_the_standard_fallback_decision_5853_s2_4() {
    // #5853 §2.4: a missing asset falls back to the standard
    // rendering and warns at build time — the element never
    // renders as nothing.
    let doc = parse_and_validate(EXAMPLE).expect("example validates");
    let data = GuiData {
        overrides: vec![asset_def("img-save", "assets/gone", ALL_BUTTON_STATES)],
        ..GuiData::default()
    };
    let (artefact, report) =
        lower_gui(&doc, &composed(Some("img-save")), &data).expect("lowering succeeds");
    match &find(&artefact.root, "button").expect("button").presentation {
        GuiPresentation::Fallback { name, .. } => assert_eq!(name, "img-save"),
        other => panic!("expected fallback presentation, got {other:?}"),
    }
    assert_eq!(report.fallbacks.len(), 1);
    assert!(report.fallbacks[0].warning.contains("assets/gone"));
    assert!(report.applied.is_empty());
}

#[test]
fn renderer_load_failure_exercises_the_standard_fallback_decision_5853_s2_4() {
    // #5853 §2.4: a registered renderer that fails to load is
    // not an error — it exercises the fallback, with a warning.
    let doc = parse_and_validate(EXAMPLE).expect("example validates");
    let data = GuiData {
        overrides: vec![drawn_def(
            "drawn-save",
            "save_button_renderer",
            ALL_BUTTON_STATES,
        )],
        registered_renderers: vec!["save_button_renderer".to_string()],
        failed_renderers: vec!["save_button_renderer".to_string()],
        ..GuiData::default()
    };
    let (artefact, report) =
        lower_gui(&doc, &composed(Some("drawn-save")), &data).expect("lowering succeeds");
    assert!(matches!(
        find(&artefact.root, "button").expect("button").presentation,
        GuiPresentation::Fallback { .. }
    ));
    assert_eq!(report.fallbacks.len(), 1);
    assert!(report.fallbacks[0].warning.contains("failed to load"));
}

#[test]
fn accessibility_tree_is_identical_in_every_mode_decision_5853_invariant_1() {
    // #5853 §3 invariant 1: the accessibility tree is the same
    // with all overrides applied, with none applied, and in
    // fallback mode.
    let doc = parse_and_validate(EXAMPLE).expect("example validates");
    let (plain, _) = lower_gui(&doc, &composed(None), &GuiData::default()).expect("plain lowering");
    let applied_data = GuiData {
        overrides: vec![asset_def(
            "img-save",
            "assets/save-button",
            ALL_BUTTON_STATES,
        )],
        available_assets: vec!["assets/save-button".to_string()],
        ..GuiData::default()
    };
    let (applied, _) =
        lower_gui(&doc, &composed(Some("img-save")), &applied_data).expect("applied lowering");
    let fallback_data = GuiData {
        overrides: vec![asset_def("img-save", "assets/gone", ALL_BUTTON_STATES)],
        ..GuiData::default()
    };
    let (fallback, _) =
        lower_gui(&doc, &composed(Some("img-save")), &fallback_data).expect("fallback lowering");
    assert_eq!(plain.a11y, applied.a11y);
    assert_eq!(plain.a11y, fallback.a11y);
}

#[test]
fn undefined_override_name_is_a_build_error_decision_5853_s2_1() {
    // #5853 §2.1: an override exists only as an entry in the
    // target's override map; a name with no pack definition
    // cannot lower.
    let doc = parse_and_validate(EXAMPLE).expect("example validates");
    let errors = lower_gui(&doc, &composed(Some("nope")), &GuiData::default())
        .expect_err("undefined override must fail the build");
    assert!(errors[0].message.contains("nope"), "{}", errors[0]);
}

#[test]
fn emit_is_byte_deterministic_decision_5855_invariant_2() {
    // #5855 invariant 2: same inputs ⇒ byte-identical emit.
    let doc = parse_and_validate(EXAMPLE).expect("example validates");
    let data = GuiData {
        overrides: vec![asset_def(
            "img-save",
            "assets/save-button",
            ALL_BUTTON_STATES,
        )],
        available_assets: vec!["assets/save-button".to_string()],
        ..GuiData::default()
    };
    let input = composed(Some("img-save"));
    let (a1, r1) = lower_gui(&doc, &input, &data).expect("lowering succeeds");
    let (a2, r2) = lower_gui(&doc, &input, &data).expect("lowering succeeds");
    let first = emit_gui_json(&a1, &r1);
    assert_eq!(first, emit_gui_json(&a2, &r2));
    assert!(first.contains("\"target\": \"gui\""));
    assert!(first.contains("\"fallbacks\""));
}

#[test]
fn lower_gui_rejects_a_non_gui_composed_document() {
    // The GUI override contract lowers GUI compositions only;
    // web and native targets lower through their own tables.
    let doc = parse_and_validate(EXAMPLE).expect("example validates");
    let mut input = composed(None);
    input.target = "web".to_string();
    let errors = lower_gui(&doc, &input, &GuiData::default()).expect_err("web target must fail");
    assert!(errors[0].message.contains("not \"gui\""), "{}", errors[0]);
}
