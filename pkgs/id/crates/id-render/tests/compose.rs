#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Behaviour tests for stage 3 (compose). Each test names the
//! decision-record behaviour it verifies: the #5854 cascade as
//! executed by #5855 §2.1 stage 3.

use std::collections::{BTreeMap, BTreeSet};

use id_render::compose::{
    AssertionInput, ComposeData, OverrideInput, Specificity, TargetSpec, ThemeTables, WinningLayer,
};
use id_render::{compose_document, parse_and_validate};

const BUTTON_PATH: &str = "document/page[0]/form[0]/button[0]";

const FORM_DOC: &str = r#"<document version="1">
  <page title="Settings">
    <form action="save">
      <button action="save">Save</button>
    </form>
  </page>
</document>"#;

const FORM_DOC_WITH_CSS: &str = r#"<document version="1">
  <page title="Settings">
    <form action="save">
      <button action="save">Save<raw kind="css">button { colour-role: hotpink; letter-spacing: 0.02em; }</raw></button>
    </form>
  </page>
</document>"#;

fn target(name: &str) -> TargetSpec {
    TargetSpec {
        name: name.to_string(),
        form_factor: "desktop".to_string(),
        capabilities: BTreeSet::new(),
    }
}

fn theme(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn assertion(path: &str, property: &str, token: &str, specificity: Specificity) -> AssertionInput {
    AssertionInput {
        element_path: path.to_string(),
        property: property.to_string(),
        token: token.to_string(),
        specificity,
    }
}

fn find<'a>(
    node: &'a id_render::compose::ComposedElement,
    path: &str,
) -> Option<&'a id_render::compose::ComposedElement> {
    if node.path == path {
        return Some(node);
    }
    for child in &node.children {
        if let id_render::compose::ComposedNode::Element(el) = child {
            if let Some(found) = find(el, path) {
                return Some(found);
            }
        }
    }
    None
}

#[test]
fn cascade_style_override_beats_assertion_decision_5854_s2_1() {
    // Layers low→high: the layer-4a style override wins over the
    // layer-3 assertion for the same property; the token stays
    // role-named and its theme value is resolved alongside.
    let doc = parse_and_validate(FORM_DOC).expect("builds");
    let data = ComposeData {
        assertions: vec![assertion(
            BUTTON_PATH,
            "colour-role",
            "accent",
            Specificity::Instance,
        )],
        overrides: vec![OverrideInput::Style {
            element_path: BUTTON_PATH.to_string(),
            property: "colour-role".to_string(),
            token: "brand".to_string(),
            specificity: Specificity::KindDefault,
        }],
        theme: ThemeTables {
            base: theme(&[("accent", "green"), ("brand", "purple")]),
            ..Default::default()
        },
    };
    let composed = compose_document(&doc, &target("web"), &data).expect("composes");
    let button = find(&composed.root, BUTTON_PATH).expect("button present");
    assert_eq!(button.properties.len(), 1);
    let prop = &button.properties[0];
    assert_eq!(prop.layer, WinningLayer::StyleOverride);
    assert_eq!(prop.token.as_deref(), Some("brand"));
    assert_eq!(prop.theme_value.as_deref(), Some("purple"));
}

#[test]
fn specificity_within_assertions_and_tie_is_an_error_decision_5854_s2_2_s6a() {
    let doc = parse_and_validate(FORM_DOC).expect("builds");
    // Instance beats kind default within layer 3.
    let data = ComposeData {
        assertions: vec![
            assertion(
                BUTTON_PATH,
                "colour-role",
                "accent",
                Specificity::KindDefault,
            ),
            assertion(BUTTON_PATH, "colour-role", "brand", Specificity::Instance),
        ],
        theme: ThemeTables {
            base: theme(&[("accent", "green"), ("brand", "purple")]),
            ..Default::default()
        },
        ..Default::default()
    };
    let composed = compose_document(&doc, &target("web"), &data).expect("composes");
    let button = find(&composed.root, BUTTON_PATH).expect("button present");
    assert_eq!(button.properties[0].token.as_deref(), Some("brand"));

    // Two different tokens at the same specificity: a build
    // error, never resolved by declaration order.
    let data = ComposeData {
        assertions: vec![
            assertion(BUTTON_PATH, "colour-role", "accent", Specificity::Instance),
            assertion(BUTTON_PATH, "colour-role", "brand", Specificity::Instance),
        ],
        ..Default::default()
    };
    let errors = compose_document(&doc, &target("web"), &data).expect_err("tie must fail");
    assert!(
        errors
            .iter()
            .any(|e| e.message.contains("same-specificity tie")),
        "{errors:?}"
    );
}

#[test]
fn theme_layering_mode_beats_target_beats_base_and_mode_fallback_warns_decision_5854_s3() {
    let doc = parse_and_validate(FORM_DOC).expect("builds");
    let data = ComposeData {
        assertions: vec![
            assertion(BUTTON_PATH, "colour-role", "accent", Specificity::Instance),
            assertion(BUTTON_PATH, "emphasis", "primary", Specificity::Instance),
        ],
        theme: ThemeTables {
            base: theme(&[("accent", "base-green"), ("primary", "base-bold")]),
            target: theme(&[("accent", "target-green"), ("primary", "target-bold")]),
            mode: Some(("dark".to_string(), theme(&[("accent", "dark-green")]))),
        },
        ..Default::default()
    };
    let composed = compose_document(&doc, &target("web"), &data).expect("composes");
    let button = find(&composed.root, BUTTON_PATH).expect("button present");
    let colour = &button.properties[0];
    assert_eq!(colour.theme_value.as_deref(), Some("dark-green"));
    // `primary` is not named by the mode variant: it falls back
    // to the target value, with the §3 fallback warning.
    let emphasis = &button.properties[1];
    assert_eq!(emphasis.theme_value.as_deref(), Some("target-bold"));
    assert!(
        composed
            .report
            .warnings
            .iter()
            .any(|w| w.message.contains("mode variant `dark` supplies no value")),
        "{:?}",
        composed.report.warnings
    );
}

#[test]
fn unresolved_token_warns_and_stays_role_named_decision_5854_s6c() {
    let doc = parse_and_validate(FORM_DOC).expect("builds");
    let data = ComposeData {
        assertions: vec![assertion(
            BUTTON_PATH,
            "colour-role",
            "accent",
            Specificity::Instance,
        )],
        ..Default::default()
    };
    let composed = compose_document(&doc, &target("web"), &data).expect("composes");
    let button = find(&composed.root, BUTTON_PATH).expect("button present");
    assert_eq!(button.properties[0].theme_value, None);
    assert!(
        composed
            .report
            .warnings
            .iter()
            .any(|w| w.message.contains("has no value in any theme layer")),
        "{:?}",
        composed.report.warnings
    );
}

#[test]
fn raw_css_wins_on_web_and_is_a_reported_override_decision_5854_s6b() {
    // The parent's worked example shape (#5854 §8a): raw CSS on
    // web wins per property over the assertion-derived value and
    // is reported, never silent.
    let doc = parse_and_validate(FORM_DOC_WITH_CSS).expect("builds");
    let data = ComposeData {
        assertions: vec![assertion(
            BUTTON_PATH,
            "colour-role",
            "accent",
            Specificity::Instance,
        )],
        theme: ThemeTables {
            base: theme(&[("accent", "green")]),
            ..Default::default()
        },
        ..Default::default()
    };
    let composed = compose_document(&doc, &target("web"), &data).expect("composes");
    let button = find(&composed.root, BUTTON_PATH).expect("button present");
    let colour = button
        .properties
        .iter()
        .find(|p| p.property == "colour-role")
        .expect("colour resolved");
    assert_eq!(colour.layer, WinningLayer::RawCss);
    assert_eq!(colour.raw_value.as_deref(), Some("hotpink"));
    assert_eq!(composed.report.reported_overrides.len(), 1);
    assert_eq!(
        composed.report.reported_overrides[0].property,
        "colour-role"
    );
    // A raw-CSS-only property also resolves, without displacing
    // anything (no reported override for it).
    assert!(
        button
            .properties
            .iter()
            .any(|p| p.property == "letter-spacing" && p.layer == WinningLayer::RawCss)
    );
}

#[test]
fn raw_css_is_ignored_off_web_with_a_warning_decision_5854_s2_3_s6d() {
    let doc = parse_and_validate(FORM_DOC_WITH_CSS).expect("builds");
    let data = ComposeData {
        assertions: vec![assertion(
            BUTTON_PATH,
            "colour-role",
            "accent",
            Specificity::Instance,
        )],
        theme: ThemeTables {
            base: theme(&[("accent", "green")]),
            ..Default::default()
        },
        ..Default::default()
    };
    let composed = compose_document(&doc, &target("gui"), &data).expect("composes");
    let button = find(&composed.root, BUTTON_PATH).expect("button present");
    // The assertion-derived value stands; no raw-CSS property
    // appears in the resolved IR for a non-web target.
    assert_eq!(button.properties.len(), 1);
    assert_eq!(button.properties[0].layer, WinningLayer::Assertion);
    assert!(composed.report.reported_overrides.is_empty());
    assert!(
        composed
            .report
            .warnings
            .iter()
            .any(|w| w.message.contains("raw CSS on target `gui` is ignored")),
        "{:?}",
        composed.report.warnings
    );
}

#[test]
fn presentation_override_supersedes_appearance_but_keeps_fallback_decision_5854_s2_1() {
    let doc = parse_and_validate(FORM_DOC).expect("builds");
    let data = ComposeData {
        assertions: vec![assertion(
            BUTTON_PATH,
            "colour-role",
            "accent",
            Specificity::Instance,
        )],
        overrides: vec![OverrideInput::Presentation {
            element_path: BUTTON_PATH.to_string(),
            presentation: "image-button-pack".to_string(),
            specificity: Specificity::Instance,
        }],
        theme: ThemeTables {
            base: theme(&[("accent", "green")]),
            ..Default::default()
        },
        ..Default::default()
    };
    let composed = compose_document(&doc, &target("gui"), &data).expect("composes");
    let button = find(&composed.root, BUTTON_PATH).expect("button present");
    assert_eq!(button.presentation.as_deref(), Some("image-button-pack"));
    // The assertion set remains, as the fallback standard
    // rendering (#5853 §2.4 via #5854 §2.1 layer 4).
    assert_eq!(button.properties.len(), 1);
    assert_eq!(
        composed.report.presentation_overrides,
        vec![(BUTTON_PATH.to_string(), "image-button-pack".to_string())]
    );
}

#[test]
fn unused_data_entries_warn_decision_5854_s6e() {
    let doc = parse_and_validate(FORM_DOC).expect("builds");
    let data = ComposeData {
        assertions: vec![assertion(
            "document/page[0]/form[0]/button[9]",
            "colour-role",
            "accent",
            Specificity::Instance,
        )],
        overrides: vec![OverrideInput::Style {
            element_path: "document/page[3]".to_string(),
            property: "colour-role".to_string(),
            token: "brand".to_string(),
            specificity: Specificity::Instance,
        }],
        ..Default::default()
    };
    let composed = compose_document(&doc, &target("web"), &data).expect("composes");
    assert_eq!(composed.report.warnings.len(), 2);
    assert!(
        composed
            .report
            .warnings
            .iter()
            .all(|w| w.message.contains("binds no element")),
        "{:?}",
        composed.report.warnings
    );
}

#[test]
fn target_locality_other_targets_declarations_do_not_leak_decision_5854_s2_4() {
    // The same document composed for a target whose data carries
    // no declarations renders layers 1–3 only: no properties, no
    // presentation, no report entries.
    let doc = parse_and_validate(FORM_DOC).expect("builds");
    let composed =
        compose_document(&doc, &target("native"), &ComposeData::default()).expect("composes");
    let button = find(&composed.root, BUTTON_PATH).expect("button present");
    assert!(button.properties.is_empty());
    assert_eq!(button.presentation, None);
    assert!(composed.report.warnings.is_empty());
    assert!(composed.report.reported_overrides.is_empty());
}

#[test]
fn malformed_raw_css_warns_instead_of_dropping_silently_decision_5854_invariant_9() {
    let src = r#"<document version="1">
  <page title="Settings">
    <form action="save">
      <button action="save">Save<raw kind="css">button { colour-role }</raw></button>
    </form>
  </page>
</document>"#;
    let doc = parse_and_validate(src).expect("builds");
    let composed =
        compose_document(&doc, &target("web"), &ComposeData::default()).expect("composes");
    assert!(
        composed
            .report
            .warnings
            .iter()
            .any(|w| w.message.contains("unparseable raw CSS fragment")),
        "{:?}",
        composed.report.warnings
    );
}
