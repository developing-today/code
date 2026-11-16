#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Behaviour tests for the compiler core. Each test names the
//! decision-record behaviour it verifies (#5855 pipeline stages over
//! the #5849 vocabulary).

use id_render::{Node, parse, parse_and_validate};

/// The worked example from #5849 §2.1.
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

#[test]
fn example_document_from_decision_5849_s2_1_parses_and_validates() {
    let doc = parse_and_validate(EXAMPLE).expect("the #5849 §2.1 example must build");
    assert_eq!(doc.version(), Some("1"));
    assert_eq!(doc.root.name, "document");
}

#[test]
fn parse_rejects_unknown_element_with_suggestions_decision_5849_s3_2() {
    // A typo'd tag is a build error listing nearest registry names,
    // never silently treated as raw HTML.
    let src =
        r#"<document version="1"><page title="T"><secton title="X">hi</secton></page></document>"#;
    let err = parse(src).expect_err("unknown element must fail the parse");
    assert!(err.message.contains("unknown element <secton>"), "{err}");
    assert!(err.message.contains("<section>"), "{err}");
}

#[test]
fn parse_rejects_unquoted_attribute_decision_5849_s2_1() {
    let src = r"<document version=1></document>";
    let err = parse(src).expect_err("unquoted values do not exist in the grammar");
    assert!(err.message.contains("double-quoted"), "{err}");
}

#[test]
fn parse_errors_are_located_decision_5855_s2_1() {
    let src = "<document version=\"1\">\n  <page title=\"T\">\n    <secton title=\"X\" />\n  </page>\n</document>";
    let err = parse(src).expect_err("must fail");
    assert_eq!(err.line, 3, "{err}");
}

#[test]
fn validate_rejects_presentation_attribute_pointing_at_assert_decision_5849_s3_2() {
    let src = r#"<document version="1"><page title="T"><button action="go" style="color: red">Go</button></page></document>"#;
    let errors = parse_and_validate(src).expect_err("style= is not in the registry");
    assert!(
        errors.iter().any(|e| e.message.contains("`assert` slot")),
        "{errors:?}"
    );
}

#[test]
fn validate_checks_document_version_decision_5849_s2_4() {
    let src = r#"<document version="2"><page title="T" /></document>"#;
    let errors = parse_and_validate(src).expect_err("version 2 is not supported");
    assert!(
        errors
            .iter()
            .any(|e| e.message.contains("supports version \"1\"")),
        "{errors:?}"
    );
}

#[test]
fn validate_enforces_child_rules_decision_5849_s4_5() {
    // A list contains items only.
    let src = r#"<document version="1"><page title="T"><list kind="unordered"><text>nope</text></list></page></document>"#;
    let errors = parse_and_validate(src).expect_err("text inside list violates §4.5");
    assert!(
        errors
            .iter()
            .any(|e| e.message.contains("allowed children")),
        "{errors:?}"
    );
}

#[test]
fn validate_master_detail_needs_one_list_and_one_detail_decision_5849_s4_5() {
    let src = r#"<document version="1"><page title="T"><master-detail title="M"><list kind="unordered"><item>x</item></list></master-detail></page></document>"#;
    let errors = parse_and_validate(src).expect_err("missing detail pane");
    assert!(
        errors.iter().any(|e| e.message.contains("exactly one")),
        "{errors:?}"
    );
}

#[test]
fn validate_rejects_expression_in_binding_slot_decision_5849_s8() {
    let src = r#"<document version="1"><page title="T"><alert kind="error" show="len(name) > 3">Bad</alert></page></document>"#;
    let errors = parse_and_validate(src).expect_err("expressions are not names");
    assert!(
        errors
            .iter()
            .any(|e| e.message.contains("not an expression")),
        "{errors:?}"
    );
}

#[test]
fn validate_rejects_missing_required_and_bad_enum() {
    let src = r#"<document version="1"><page title="T"><alert kind="fatal">Boom</alert></page></document>"#;
    let errors = parse_and_validate(src).expect_err("fatal is not an alert kind");
    assert!(
        errors.iter().any(|e| e.message.contains("must be one of")),
        "{errors:?}"
    );

    let src = r#"<document version="1"><page><text>x</text></page></document>"#;
    let errors = parse_and_validate(src).expect_err("page requires title");
    assert!(
        errors
            .iter()
            .any(|e| e.message.contains("requires attribute `title`")),
        "{errors:?}"
    );
}

#[test]
fn validate_field_constraint_applicability_decision_5849_s4_3_1() {
    // min-length applies to text/password fields, not to number.
    let src = r#"<document version="1"><page title="T"><form action="a"><field kind="number" label="N" min-length="2" /></form></page></document>"#;
    let errors = parse_and_validate(src).expect_err("min-length on a number field");
    assert!(
        errors.iter().any(|e| e.message.contains("does not apply")),
        "{errors:?}"
    );
}

#[test]
fn raw_region_passes_through_verbatim_decision_5849_s3_1() {
    let src = r#"<document version="1"><page title="T"><raw kind="html"><div class="ascii-art">…</div></raw></page></document>"#;
    let doc = parse_and_validate(src).expect("raw regions are authored source");
    let page = match &doc.root.children[0] {
        Node::Element(el) => el,
        other => panic!("expected page element, got {other:?}"),
    };
    let raw = match &page.children[0] {
        Node::Element(el) => el,
        other => panic!("expected raw element, got {other:?}"),
    };
    match &raw.children[0] {
        Node::Raw(region) => {
            assert_eq!(region.kind, "html");
            assert!(region.content.contains("ascii-art"), "{region:?}");
        }
        other => panic!("expected raw node, got {other:?}"),
    }
}

#[test]
fn comments_are_stripped_and_may_not_contain_elements_decision_5849_s2_1() {
    let src = "<document version=\"1\"><!-- a note --><page title=\"T\" /></document>";
    assert!(parse_and_validate(src).is_ok());
    let src = "<document version=\"1\"><!-- <page> --><page title=\"T\" /></document>";
    assert!(parse(src).is_err());
}

#[test]
fn text_whitespace_collapses_decision_5849_s2_1() {
    let src = "<document version=\"1\"><page title=\"T\"><text>hello\n   wide   world</text></page></document>";
    let doc = parse_and_validate(src).expect("valid");
    let page = match &doc.root.children[0] {
        Node::Element(el) => el,
        other => panic!("expected page, got {other:?}"),
    };
    let text = match &page.children[0] {
        Node::Element(el) => el,
        other => panic!("expected text, got {other:?}"),
    };
    assert_eq!(
        text.children,
        vec![Node::Text("hello wide world".to_string())]
    );
}

#[test]
fn sole_section_may_omit_title_decision_5849_s4_1() {
    let src = r#"<document version="1"><page title="T"><section><text>x</text></section></page></document>"#;
    assert!(parse_and_validate(src).is_ok());
    let src = r#"<document version="1"><page title="T"><section><text>x</text></section><section title="B"><text>y</text></section></page></document>"#;
    let errors = parse_and_validate(src).expect_err("one of two sections lacks a title");
    assert!(
        errors
            .iter()
            .any(|e| e.message.contains("requires attribute `title`")),
        "{errors:?}"
    );
}
