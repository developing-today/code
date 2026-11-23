#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Behaviour tests for the assertion half of stage 2 and its
//! passage through stages 3–5: the `assert` slot consumed
//! against the #5850 assertion vocabulary (the discharge of
//! #5855 assumption A2). Each test names the decision-record
//! behaviour it verifies.

use std::collections::BTreeSet;

use id_render::assertion_vocabulary::{self, DIMENSIONS};
use id_render::compose::{AssertionInput, ComposeData, OverrideInput, Specificity, TargetSpec};
use id_render::lower_gui::{GuiData, GuiOverrideDef, PresentationKind};
use id_render::{
    compose_document, lower_document, lower_document_gui, lower_document_native, parse_and_validate,
};

const BUTTON_PATH: &str = "document/page[0]/form[0]/button[0]";
const CARD_PATH: &str = "document/page[0]/card[0]";

/// One assertion-bearing source rendered by the three-target
/// tests: the #5850 §2 examples made concrete (a primary
/// call-to-action carries emphasis primary and tone accent; a
/// card carries elevation raised and pad space-3).
const ASSERTING_DOC: &str = r#"<document version="1">
  <page title="Settings" assert="tone surface-base">
    <heading level="1" assert="text heading">Settings</heading>
    <card title="Profile" assert="elevation elevation-raised, pad space-3">
      <text>Your public profile.</text>
    </card>
    <form action="save">
      <button action="save" assert="tone accent, emphasis primary">Save</button>
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

fn find_composed<'a>(
    node: &'a id_render::compose::ComposedElement,
    path: &str,
) -> Option<&'a id_render::compose::ComposedElement> {
    if node.path == path {
        return Some(node);
    }
    for child in &node.children {
        if let id_render::compose::ComposedNode::Element(el) = child
            && let Some(found) = find_composed(el, path)
        {
            return Some(found);
        }
    }
    None
}

fn doc_with_assert(assert: &str) -> String {
    format!(
        r#"<document version="1"><page title="T"><form action="a"><button action="a" assert="{assert}">Go</button></form></page></document>"#
    )
}

#[test]
fn assert_slot_parses_dimension_token_pairs_decision_5850_s2() {
    // #5850 §2: an assertion is a (dimension, role-value) pair;
    // the slot is a comma-separated list of pairs.
    let pairs = assertion_vocabulary::parse_assert_slot("tone accent, emphasis primary")
        .expect("the §2 example pairs parse");
    assert_eq!(pairs.len(), 2);
    assert_eq!(pairs[0].dimension, "tone");
    assert_eq!(pairs[0].token, "accent");
    assert_eq!(pairs[1].dimension, "emphasis");
    assert_eq!(pairs[1].token, "primary");

    // And the full pipeline accepts the slot in source.
    let doc = parse_and_validate(&doc_with_assert("tone accent, emphasis primary"))
        .expect("a vocabulary-conforming slot validates");
    assert_eq!(doc.version(), Some("1"));
}

#[test]
fn every_vocabulary_token_validates_decision_5850_s2() {
    // The validator and the vocabulary table must agree exactly:
    // every (dimension, token) pair the #5850 data names is
    // accepted in the slot, on an ordinary element.
    for dimension in DIMENSIONS {
        for token in dimension.tokens {
            let src = doc_with_assert(&format!("{} {token}", dimension.name));
            assert!(
                parse_and_validate(&src).is_ok(),
                "{} {token} must validate",
                dimension.name
            );
        }
    }
}

#[test]
fn assert_slot_unknown_dimension_fails_with_suggestions_decision_5850_s2() {
    // The #5849 idiom, applied to the assertion vocabulary: an
    // unknown dimension is a build error naming the closed set
    // and the nearest dimensions.
    let errors = parse_and_validate(&doc_with_assert("tonee accent"))
        .expect_err("a typo'd dimension must fail validation");
    assert!(
        errors.iter().any(
            |e| e.message.contains("unknown assertion dimension `tonee`")
                && e.message.contains("tone")
        ),
        "{errors:?}"
    );
}

#[test]
fn assert_slot_unknown_token_fails_with_suggestions_decision_5850_s2() {
    let errors = parse_and_validate(&doc_with_assert("tone acceent"))
        .expect_err("a typo'd role token must fail validation");
    assert!(
        errors
            .iter()
            .any(|e| e.message.contains("unknown assertion token `acceent`")
                && e.message.contains("accent")),
        "{errors:?}"
    );
    // A token from another dimension's domain is equally unknown
    // here: domains are per-dimension.
    let errors = parse_and_validate(&doc_with_assert("tone heading"))
        .expect_err("a text role is not a tone role");
    assert!(
        errors
            .iter()
            .any(|e| e.message.contains("unknown assertion token `heading`")),
        "{errors:?}"
    );
}

#[test]
fn assert_slot_rejects_raw_values_decision_5850_invariant_1() {
    // Invariant 1: no hex colours, no sizes, no font names in
    // assertions — content names roles only.
    for bad in ["tone #bada55", "pad 12px", "text Arial"] {
        let errors = parse_and_validate(&doc_with_assert(bad))
            .expect_err("a raw value must fail validation");
        assert!(
            errors.iter().any(|e| e.message.contains("not a role name")),
            "{bad}: {errors:?}"
        );
    }
}

#[test]
fn assert_slot_rejects_malformed_pairs_decision_5850_s2() {
    for bad in ["accent", "tone", "tone accent extra", ""] {
        let errors = parse_and_validate(&doc_with_assert(bad))
            .expect_err("a malformed slot must fail validation");
        assert!(
            errors.iter().any(|e| e.message.contains("`assert`")),
            "{bad:?}: {errors:?}"
        );
    }
    // A bare `assert` attribute carries no expression at all.
    let src = r#"<document version="1"><page title="T"><form action="a"><button action="a" assert>Go</button></form></page></document>"#;
    let errors = parse_and_validate(src).expect_err("a bare assert must fail");
    assert!(
        errors
            .iter()
            .any(|e| e.message.contains("`assert` on <button> requires a value")),
        "{errors:?}"
    );
}

#[test]
fn assert_slot_duplicate_dimension_is_an_error_decision_5854_s2_2() {
    // #5854 §2.2: an exact tie between two assertions in the
    // source is an authoring error, never resolved by order.
    let errors = parse_and_validate(&doc_with_assert("tone accent, tone danger"))
        .expect_err("one dimension, asserted twice in one slot");
    assert!(
        errors.iter().any(|e| e.message.contains("asserted twice")),
        "{errors:?}"
    );
}

#[test]
fn assert_slot_on_raw_region_is_an_error_decision_5849_s3_1() {
    // A raw region is verbatim authored source, not a rendered
    // element: the slot cannot be honoured there, so it is an
    // error rather than a silent drop.
    let src = r#"<document version="1"><page title="T"><raw kind="html" assert="tone accent"><p>hi</p></raw></page></document>"#;
    let errors = parse_and_validate(src).expect_err("assert on raw must fail");
    assert!(
        errors
            .iter()
            .any(|e| e.message.contains("does not apply to a <raw> region")),
        "{errors:?}"
    );
}

#[test]
fn source_assertions_compose_at_instance_specificity_decision_5854_s2_2() {
    let doc = parse_and_validate(&doc_with_assert("tone accent")).expect("validates");
    // A kind-default data assertion for the same property loses
    // to the source slot's instance assertion.
    let data = ComposeData {
        assertions: vec![AssertionInput {
            element_path: BUTTON_PATH.to_string(),
            property: "tone".to_string(),
            token: "danger".to_string(),
            specificity: Specificity::KindDefault,
        }],
        ..Default::default()
    };
    let composed = compose_document(&doc, &target("web"), &data).expect("composes");
    let button = find_composed(&composed.root, BUTTON_PATH).expect("button present");
    assert_eq!(button.properties.len(), 1);
    assert_eq!(button.properties[0].token.as_deref(), Some("accent"));

    // A data assertion at instance specificity with a different
    // token ties with the slot: a build error (§5854 §6a).
    let data = ComposeData {
        assertions: vec![AssertionInput {
            element_path: BUTTON_PATH.to_string(),
            property: "tone".to_string(),
            token: "danger".to_string(),
            specificity: Specificity::Instance,
        }],
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
fn assertion_bearing_document_lowers_on_web_decision_5855_s2_1() {
    // The worked example, web target (#5851): the source
    // assertions reach the artefact as utilities and are
    // accounted as mapped in the coverage report — none
    // defaulted, none silent.
    let doc = parse_and_validate(ASSERTING_DOC).expect("validates");
    let composed =
        compose_document(&doc, &target("web"), &ComposeData::default()).expect("composes");
    let (artefact, report) = lower_document(&doc, &composed).expect("lowers");
    assert!(artefact.class_set.contains(&"bg-accent".to_string()));
    assert!(artefact.class_set.contains(&"bg-surface-base".to_string()));
    assert!(artefact.class_set.contains(&"shadow".to_string()));
    assert!(artefact.class_set.contains(&"p-space-3".to_string()));
    assert!(artefact.class_set.contains(&"text-step-plus-2".to_string()));
    for (path, property, token) in [
        (BUTTON_PATH, "tone", "accent"),
        (BUTTON_PATH, "emphasis", "primary"),
        (CARD_PATH, "elevation", "elevation-raised"),
        (CARD_PATH, "pad", "space-3"),
    ] {
        assert!(
            report
                .mapped
                .iter()
                .any(|m| m.element_path == path && m.property == property && m.token == token),
            "report must account {property}: {token} on {path} as mapped"
        );
    }
    assert!(report.defaulted.is_empty(), "{:?}", report.defaulted);
}

#[test]
fn assertion_bearing_document_lowers_on_native_decision_5855_s2_1() {
    // The worked example, native target (mac, #5852): tone
    // accent maps to the platform rendering; corner — a
    // platform-default entry on every platform (§3.7) — is
    // accounted as defaulted, never silent (§3.1).
    let src = ASSERTING_DOC.replace(
        "assert=\"tone accent, emphasis primary\"",
        "assert=\"tone accent, emphasis primary, corner corner-rounded\"",
    );
    let doc = parse_and_validate(&src).expect("validates");
    let composed =
        compose_document(&doc, &target("mac"), &ComposeData::default()).expect("composes");
    let (_artefact, report) = lower_document_native(&doc, &composed).expect("lowers");
    assert!(
        report.mapped.iter().any(|m| m.element_path == BUTTON_PATH
            && m.property == "tone"
            && m.token == "accent"
            && m.rendering == "controlAccentColor"),
        "{:?}",
        report.mapped
    );
    assert!(
        report
            .defaulted
            .iter()
            .any(|d| d.element_path == BUTTON_PATH
                && d.property == "corner"
                && d.token == "corner-rounded"),
        "{:?}",
        report.defaulted
    );
}

#[test]
fn assertion_bearing_document_lowers_on_gui_decision_5855_s2_1() {
    // The worked example, custom-GUI target (#5853): with no
    // override, the source assertions govern the standard
    // rendering and the report says so; with an asset override
    // applied to the button, its assertions are accounted as
    // superseded-by-presentation — retained as the fallback
    // standard rendering (#5854 §2.1 layer 4), never dropped.
    let doc = parse_and_validate(ASSERTING_DOC).expect("validates");
    let composed =
        compose_document(&doc, &target("gui"), &ComposeData::default()).expect("composes");
    let (_artefact, report) =
        lower_document_gui(&doc, &composed, &GuiData::default()).expect("lowers");
    assert!(
        report
            .assertions
            .iter()
            .any(|a| a.element_path == BUTTON_PATH
                && a.property == "tone"
                && a.token == "accent"
                && a.disposition == "standard"),
        "{:?}",
        report.assertions
    );

    let data = ComposeData {
        overrides: vec![OverrideInput::Presentation {
            element_path: BUTTON_PATH.to_string(),
            presentation: "save-button-art".to_string(),
            specificity: Specificity::Instance,
        }],
        ..Default::default()
    };
    let composed = compose_document(&doc, &target("gui"), &data).expect("composes");
    let gui_data = GuiData {
        overrides: vec![GuiOverrideDef {
            name: "save-button-art".to_string(),
            kind: PresentationKind::Asset,
            asset: Some("save-art".to_string()),
            renderer: None,
            covered_states: ["default", "hover", "focus", "active", "disabled"]
                .iter()
                .map(ToString::to_string)
                .collect(),
            derivations: Vec::new(),
        }],
        available_assets: vec!["save-art".to_string()],
        ..Default::default()
    };
    let (_artefact, report) = lower_document_gui(&doc, &composed, &gui_data).expect("lowers");
    assert!(
        report
            .assertions
            .iter()
            .any(|a| a.element_path == BUTTON_PATH
                && a.property == "tone"
                && a.disposition == "superseded-by-presentation"),
        "{:?}",
        report.assertions
    );
    // Assertions on elements without an override still govern.
    assert!(
        report.assertions.iter().any(|a| a.element_path == CARD_PATH
            && a.property == "pad"
            && a.disposition == "standard"),
        "{:?}",
        report.assertions
    );
}
