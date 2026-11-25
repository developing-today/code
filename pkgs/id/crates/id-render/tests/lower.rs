#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Behaviour tests for stages 4–5 (lower + emit, web target).
//! Each test names the decision-record behaviour it verifies:
//! #5851 supplies the mapping table, #5855 the pipeline rules.

use id_render::compose::{
    ComposedDocument, ComposedElement, ComposedNode, CompositionReport, ResolvedProperty,
    WinningLayer,
};
use id_render::lower::{WebNode, emit_json, lower_web};
use id_render::{Span, parse_and_validate, vocabulary};

/// The worked example from #5849 §2.1 (same source as
/// tests/pipeline.rs).
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

fn prop(property: &str, token: &str) -> ResolvedProperty {
    ResolvedProperty {
        property: property.to_string(),
        layer: WinningLayer::Assertion,
        token: Some(token.to_string()),
        raw_value: None,
        theme_value: None,
    }
}

const BUTTON_PATH: &str = "document/page[0]/form[0]/button[0]";

fn composed(button_properties: Vec<ResolvedProperty>) -> ComposedDocument {
    let mut button = el(BUTTON_PATH, "button", vec![text("Save")]);
    button.properties = button_properties;
    ComposedDocument {
        target: "web".to_string(),
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

fn find<'a>(
    node: &'a id_render::lower::WebElement,
    kind: &str,
) -> Option<&'a id_render::lower::WebElement> {
    if node.kind == kind {
        return Some(node);
    }
    for child in &node.children {
        if let WebNode::Element(child_el) = child {
            if let Some(found) = find(child_el, kind) {
                return Some(found);
            }
        }
    }
    None
}

#[test]
fn lower_emits_dom_tags_from_decision_5851_s2() {
    // #5851 §2: one fixed DOM element per registry kind, variant
    // rows resolved from the source attributes.
    let doc = parse_and_validate(EXAMPLE).expect("example validates");
    let (artefact, report) = lower_web(&doc, &composed(vec![])).expect("lowering succeeds");
    assert_eq!(artefact.root.tag, "html");
    assert_eq!(find(&artefact.root, "page").expect("page").tag, "main");
    assert_eq!(find(&artefact.root, "heading").expect("heading").tag, "h1");
    let fields: Vec<&id_render::lower::WebElement> = {
        let mut out = Vec::new();
        fn walk<'a>(
            n: &'a id_render::lower::WebElement,
            out: &mut Vec<&'a id_render::lower::WebElement>,
        ) {
            if n.kind == "field" {
                out.push(n);
            }
            for c in &n.children {
                if let WebNode::Element(e) = c {
                    walk(e, out);
                }
            }
        }
        walk(&artefact.root, &mut out);
        out
    };
    assert_eq!(fields[0].tag, "input");
    assert_eq!(
        fields[0].attributes,
        vec![("type".to_string(), "text".to_string())]
    );
    assert_eq!(fields[1].tag, "select");
    let alert = find(&artefact.root, "alert").expect("alert");
    assert_eq!(alert.tag, "div");
    assert_eq!(
        alert.attributes,
        vec![("role".to_string(), "alert".to_string())]
    );
    assert_eq!(
        find(&artefact.root, "button").expect("button").tag,
        "button"
    );
    assert!(
        report.element_defaults.is_empty(),
        "every kind in the example has a §2 row"
    );
}

#[test]
fn accessibility_role_survives_lowering_decision_5855_invariant_5() {
    // #5855 invariant 5: every artefact node exposes the
    // accessibility role its kind carries by construction.
    let doc = parse_and_validate(EXAMPLE).expect("example validates");
    let (artefact, _) = lower_web(&doc, &composed(vec![])).expect("lowering succeeds");
    for kind in ["page", "heading", "field", "button", "alert"] {
        let node = find(&artefact.root, kind).expect("node present");
        assert_eq!(
            node.role,
            vocabulary::lookup(kind).expect("registry kind").role
        );
        assert_ne!(node.role, "");
    }
}

#[test]
fn mapped_roles_emit_utilities_from_decision_5851_s3() {
    // #5851 §3.1/§3.2/§3.3/§3.4: tone, text, corner and spacing
    // roles emit their table utilities; the class set is the
    // order-normalised union (§5855 §3.2, #5851 §3.8).
    let doc = parse_and_validate(EXAMPLE).expect("example validates");
    let (artefact, report) = lower_web(
        &doc,
        &composed(vec![
            prop("tone", "surface-raised"),
            prop("text", "heading"),
            prop("corner", "corner-full"),
            prop("pad", "space-3"),
        ]),
    )
    .expect("lowering succeeds");
    let button = find(&artefact.root, "button").expect("button");
    for class in [
        "bg-surface-raised",
        "text-step-plus-2",
        "font-semibold",
        "rounded-full",
        "p-space-3",
    ] {
        assert!(
            button.classes.iter().any(|c| c == class),
            "missing {class}: {:?}",
            button.classes
        );
        assert!(artefact.class_set.iter().any(|c| c == class));
    }
    assert_eq!(report.mapped.len(), 4);
    assert_eq!(
        report.defaulted,
        [] as [id_render::lower::DefaultedAssertion; 0]
    );
    let mut sorted = artefact.class_set.clone();
    sorted.sort();
    assert_eq!(artefact.class_set, sorted, "class set is order-normalised");
}

#[test]
fn unmapped_assertion_defaults_and_is_reported_decision_5855_s3_3() {
    // #5855 §2.1 stage 4 + §3.3 and #5851 §3.7: an assertion with
    // no row in the target's table lowers to the layer default
    // and is recorded with the coverage warning — never silent.
    let doc = parse_and_validate(EXAMPLE).expect("example validates");
    let (artefact, report) =
        lower_web(&doc, &composed(vec![prop("tone", "brand")])).expect("lowering succeeds");
    let button = find(&artefact.root, "button").expect("button");
    assert!(button.classes.is_empty(), "the default emits no utilities");
    assert_eq!(report.defaulted.len(), 1);
    let defaulted = &report.defaulted[0];
    assert_eq!(defaulted.element_path, BUTTON_PATH);
    assert_eq!(defaulted.token, "brand");
    assert!(
        defaulted.warning.contains("no web mapping"),
        "{}",
        defaulted.warning
    );
}

#[test]
fn density_is_mapped_without_utilities_decision_5851_s3_3() {
    // #5851 §3.3: density resolves through the theme multiplier,
    // not utilities — mapped, not defaulted, no classes.
    let doc = parse_and_validate(EXAMPLE).expect("example validates");
    let (artefact, report) =
        lower_web(&doc, &composed(vec![prop("density", "compact")])).expect("lowering succeeds");
    assert_eq!(report.mapped.len(), 1);
    assert_eq!(report.mapped[0].classes, [] as [std::string::String; 0]);
    assert_eq!(
        report.defaulted,
        [] as [id_render::lower::DefaultedAssertion; 0]
    );
    assert_eq!(
        find(&artefact.root, "button").expect("button").classes,
        [] as [std::string::String; 0]
    );
}

#[test]
fn raw_css_winner_is_carried_as_text_not_classes_decision_5851_s5() {
    // #5851 §5 / #5854 layer 5: a raw-CSS winner is author CSS
    // text on the artefact node, never a generated class.
    let doc = parse_and_validate(EXAMPLE).expect("example validates");
    let raw = ResolvedProperty {
        property: "color".to_string(),
        layer: WinningLayer::RawCss,
        token: None,
        raw_value: Some("rebeccapurple".to_string()),
        theme_value: None,
    };
    let (artefact, report) = lower_web(&doc, &composed(vec![raw])).expect("lowering succeeds");
    let button = find(&artefact.root, "button").expect("button");
    assert_eq!(
        button.raw_css,
        vec![("color".to_string(), "rebeccapurple".to_string())]
    );
    assert_eq!(button.classes, [] as [std::string::String; 0]);
    assert!(report.mapped.is_empty() && report.defaulted.is_empty());
}

#[test]
fn emit_is_byte_deterministic_decision_5855_invariant_2() {
    // #5855 invariant 2: same inputs ⇒ byte-identical emit; the
    // JSON carries no timestamps.
    let doc = parse_and_validate(EXAMPLE).expect("example validates");
    let input = composed(vec![prop("text", "body"), prop("tone", "accent")]);
    let (a1, r1) = lower_web(&doc, &input).expect("lowering succeeds");
    let (a2, r2) = lower_web(&doc, &input).expect("lowering succeeds");
    let first = emit_json(&a1, &r1);
    assert_eq!(first, emit_json(&a2, &r2));
    assert!(first.contains("\"target\": \"web\""));
    assert!(first.contains("\"defaulted\""));
}

#[test]
fn lower_rejects_a_non_web_composed_document() {
    // The web table lowers web compositions only; other targets
    // lower through their own tables (#5852/#5853).
    let doc = parse_and_validate(EXAMPLE).expect("example validates");
    let mut input = composed(vec![]);
    input.target = "native".to_string();
    let errors = lower_web(&doc, &input).expect_err("non-web target must fail");
    assert!(errors[0].message.contains("not \"web\""), "{}", errors[0]);
}
