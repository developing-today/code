#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Behaviour tests for stages 4–5 (lower + emit, native targets).
//! Each test names the decision-record behaviour it verifies:
//! #5852 supplies the mapping tables and the platform-default
//! contract, #5855 the pipeline rules, #5853 the native exclusion
//! of presentation overrides.

use id_render::compose::{
    ComposedDocument, ComposedElement, ComposedNode, CompositionReport, ResolvedProperty,
    WinningLayer,
};
use id_render::lower_native::{NativeNode, emit_native_json, lower_native};
use id_render::{Span, parse_and_validate, vocabulary};

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

fn composed(platform: &str, button_properties: Vec<ResolvedProperty>) -> ComposedDocument {
    let mut button = el(BUTTON_PATH, "button", vec![text("Save")]);
    button.properties = button_properties;
    ComposedDocument {
        target: platform.to_string(),
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
    node: &'a id_render::lower_native::NativeElement,
    kind: &str,
) -> Option<&'a id_render::lower_native::NativeElement> {
    if node.kind == kind {
        return Some(node);
    }
    for child in &node.children {
        if let NativeNode::Element(child_el) = child {
            if let Some(found) = find(child_el, kind) {
                return Some(found);
            }
        }
    }
    None
}

fn fields(
    root: &id_render::lower_native::NativeElement,
) -> Vec<&id_render::lower_native::NativeElement> {
    let mut out = Vec::new();
    fn walk<'a>(
        n: &'a id_render::lower_native::NativeElement,
        out: &mut Vec<&'a id_render::lower_native::NativeElement>,
    ) {
        if n.kind == "field" {
            out.push(n);
        }
        for c in &n.children {
            if let NativeNode::Element(e) = c {
                walk(e, out);
            }
        }
    }
    walk(root, &mut out);
    out
}

#[test]
fn lower_emits_os_standard_controls_from_decision_5852_s2() {
    // #5852 §2: one OS-standard control per element kind per
    // platform, variant rows resolved from the source attributes
    // (field kind text → text entry, choice → choice control).
    let doc = parse_and_validate(EXAMPLE).expect("example validates");
    let (mac, mac_report) = lower_native(&doc, &composed("mac", vec![])).expect("mac lowers");
    assert_eq!(
        find(&mac.root, "button").expect("button").control,
        "NSButton (standard push button)"
    );
    assert_eq!(fields(&mac.root)[0].control, "text field");
    assert_eq!(fields(&mac.root)[1].control, "pop-up button");
    assert_eq!(
        find(&mac.root, "form").expect("form").control,
        "grouped form layout"
    );
    assert_eq!(
        mac_report.element_defaults,
        [] as [id_render::lower_native::NativeElementDefault; 0]
    );

    let (windows, _) = lower_native(&doc, &composed("windows", vec![])).expect("windows lowers");
    assert_eq!(
        find(&windows.root, "button").expect("button").control,
        "Button"
    );
    assert_eq!(fields(&windows.root)[0].control, "TextBox");
    assert_eq!(fields(&windows.root)[1].control, "ComboBox");

    let (gnome, _) = lower_native(&doc, &composed("gnome", vec![])).expect("gnome lowers");
    assert_eq!(
        find(&gnome.root, "button").expect("button").control,
        "GtkButton"
    );
    assert_eq!(fields(&gnome.root)[0].control, "GtkEntry");
    assert_eq!(fields(&gnome.root)[1].control, "GtkDropDown");
    assert_eq!(
        find(&gnome.root, "form").expect("form").control,
        "AdwPreferencesGroup"
    );
}

#[test]
fn control_sequence_is_in_document_order_decision_5855_s3_2() {
    // #5855 §3.2: the native artefact's control-kind sequence is
    // the document-order list of selected controls.
    let doc = parse_and_validate(EXAMPLE).expect("example validates");
    let (artefact, _) = lower_native(&doc, &composed("mac", vec![])).expect("lowering succeeds");
    let kinds: Vec<&str> = artefact
        .control_sequence
        .iter()
        .map(|e| e.kind.as_str())
        .collect();
    assert_eq!(
        kinds,
        vec![
            "document", "page", "heading", "form", "field", "field", "alert", "button"
        ]
    );
    let button_entry = artefact.control_sequence.last().expect("last entry");
    assert_eq!(button_entry.path, BUTTON_PATH);
    assert_eq!(button_entry.control, "NSButton (standard push button)");
}

#[test]
fn accessibility_tree_is_identical_across_platforms_decision_5852_invariant_5() {
    // #5852 invariant 5: role, accessible name, and structure
    // come from the semantic elements and are identical across
    // platforms; only the platform's presentation of the tree
    // (the controls) differs.
    let doc = parse_and_validate(EXAMPLE).expect("example validates");
    let (mac, _) = lower_native(&doc, &composed("mac", vec![])).expect("mac lowers");
    let (gnome, _) = lower_native(&doc, &composed("gnome", vec![])).expect("gnome lowers");
    assert_eq!(
        mac.a11y, gnome.a11y,
        "a11y trees must not fork per platform"
    );
    assert_ne!(
        mac.control_sequence, gnome.control_sequence,
        "controls are the platform-specific part"
    );
    // Roles are the vocabulary's, names come from label/text.
    fn a11y_find<'a>(
        node: &'a id_render::lower_native::A11yNode,
        kind: &str,
    ) -> Option<&'a id_render::lower_native::A11yNode> {
        if node.kind == kind {
            return Some(node);
        }
        node.children.iter().find_map(|c| a11y_find(c, kind))
    }
    for kind in ["page", "heading", "field", "button", "alert"] {
        let node = a11y_find(&mac.a11y, kind).expect("a11y node present");
        assert_eq!(
            node.role,
            vocabulary::lookup(kind).expect("registry kind").role
        );
    }
    assert_eq!(a11y_find(&mac.a11y, "button").expect("button").name, "Save");
    assert_eq!(
        a11y_find(&mac.a11y, "field").expect("field").name,
        "Display name"
    );
    assert_eq!(
        a11y_find(&mac.a11y, "heading").expect("heading").name,
        "Account settings"
    );
}

#[test]
fn mapped_assertions_record_platform_renderings_decision_5852_s3() {
    // #5852 §3.2/§3.3/§3.6/§3.7: mapped assertions record the
    // platform rendering the table gives — accent, text style,
    // spacing unit, elevation layering.
    let doc = parse_and_validate(EXAMPLE).expect("example validates");
    let (_, report) = lower_native(
        &doc,
        &composed(
            "mac",
            vec![
                prop("tone", "accent"),
                prop("text", "heading"),
                prop("pad", "space-3"),
                prop("elevation", "elevation-raised"),
            ],
        ),
    )
    .expect("lowering succeeds");
    assert_eq!(report.mapped.len(), 4);
    assert_eq!(
        report.defaulted,
        [] as [id_render::lower_native::DefaultedNativeAssertion; 0]
    );
    let rendering = |property: &str| {
        report
            .mapped
            .iter()
            .find(|m| m.property == property)
            .expect("mapped entry")
            .rendering
            .as_str()
    };
    assert_eq!(rendering("tone"), "controlAccentColor");
    assert_eq!(rendering("text"), "Dynamic Type headline");
    assert_eq!(rendering("pad"), "space-3 in points");
    assert_eq!(rendering("elevation"), "material layering (raised)");
}

#[test]
fn platform_default_assertions_are_reported_never_silent_decision_5852_s3_1() {
    // #5852 §3.1 + invariant 6: a platform-default entry lowers
    // to the OS rendering and is recorded with the §3.1 warning;
    // the same assertion may map on one platform and default on
    // another (§3.4 destructive on Windows, §3.5 density on
    // GNOME, §3.7 corner everywhere).
    let doc = parse_and_validate(EXAMPLE).expect("example validates");

    let (_, mac_report) = lower_native(
        &doc,
        &composed(
            "mac",
            vec![
                prop("corner", "corner-full"),
                prop("density", "compact"),
                prop("emphasis", "destructive"),
            ],
        ),
    )
    .expect("mac lowers");
    assert_eq!(mac_report.defaulted.len(), 1);
    assert_eq!(mac_report.defaulted[0].token, "corner-full");
    assert!(
        mac_report.defaulted[0]
            .warning
            .contains("no native mapping on mac"),
        "{}",
        mac_report.defaulted[0].warning
    );
    assert_eq!(
        mac_report.mapped.len(),
        2,
        "density and destructive map on mac"
    );

    let (_, gnome_report) =
        lower_native(&doc, &composed("gnome", vec![prop("density", "compact")]))
            .expect("gnome lowers");
    assert_eq!(gnome_report.defaulted.len(), 1);
    assert_eq!(gnome_report.defaulted[0].platform, "gnome");

    let (_, windows_report) = lower_native(
        &doc,
        &composed("windows", vec![prop("emphasis", "destructive")]),
    )
    .expect("windows lowers");
    assert_eq!(windows_report.defaulted.len(), 1);
    assert_eq!(windows_report.defaulted[0].token, "destructive");

    // An assertion with no row at all defaults the same way.
    let (_, report) = lower_native(&doc, &composed("mac", vec![prop("tone", "brand")]))
        .expect("lowering succeeds");
    assert_eq!(report.defaulted.len(), 1);
    assert_eq!(report.defaulted[0].token, "brand");
}

#[test]
fn focus_border_is_platform_default_decision_5852_s3_2_and_s3_8() {
    // #5852 §3.2/§3.8: the focus ring is the platform's own, so
    // border-focus is a platform-default entry on every platform.
    let doc = parse_and_validate(EXAMPLE).expect("example validates");
    for platform in ["mac", "windows", "gnome"] {
        let (_, report) = lower_native(
            &doc,
            &composed(platform, vec![prop("tone", "border-focus")]),
        )
        .expect("lowering succeeds");
        assert_eq!(report.defaulted.len(), 1, "{platform}");
        assert_eq!(report.defaulted[0].platform, platform);
        assert!(report.mapped.is_empty(), "{platform}");
    }
}

#[test]
fn presentation_overrides_do_not_apply_to_native_controls_decision_5853() {
    // #5853 (followed by #5852): presentation overrides do not
    // apply to native standard controls — the element lowers to
    // its standard control and the exclusion is recorded.
    let doc = parse_and_validate(EXAMPLE).expect("example validates");
    let mut input = composed("gnome", vec![]);
    fn set_presentation(node: &mut ComposedElement) {
        if node.name == "button" {
            node.presentation = Some("fancy-card".to_string());
        }
        for child in &mut node.children {
            if let ComposedNode::Element(child_el) = child {
                set_presentation(child_el);
            }
        }
    }
    set_presentation(&mut input.root);
    let (artefact, report) = lower_native(&doc, &input).expect("lowering succeeds");
    assert_eq!(
        find(&artefact.root, "button").expect("button").control,
        "GtkButton",
        "the standard control stands"
    );
    assert_eq!(report.presentation_excluded.len(), 1);
    assert_eq!(report.presentation_excluded[0].element_path, BUTTON_PATH);
    assert_eq!(report.presentation_excluded[0].presentation, "fancy-card");
}

#[test]
fn raw_css_winner_is_a_platform_default_on_native_decision_5852_a3() {
    // #5852 A3: raw CSS does not exist on native targets, so a
    // raw-CSS winner cannot be carried — it is recorded as a
    // platform-default entry naming the value.
    let doc = parse_and_validate(EXAMPLE).expect("example validates");
    let raw = ResolvedProperty {
        property: "color".to_string(),
        layer: WinningLayer::RawCss,
        token: None,
        raw_value: Some("rebeccapurple".to_string()),
        theme_value: None,
    };
    let (_, report) = lower_native(&doc, &composed("mac", vec![raw])).expect("lowering succeeds");
    assert_eq!(
        report.mapped,
        [] as [id_render::lower_native::MappedNativeAssertion; 0]
    );
    assert_eq!(report.defaulted.len(), 1);
    assert!(
        report.defaulted[0].warning.contains("rebeccapurple"),
        "{}",
        report.defaulted[0].warning
    );
}

#[test]
fn emit_is_byte_deterministic_decision_5855_invariant_2() {
    // #5855 invariant 2: same inputs ⇒ byte-identical emit; the
    // JSON carries the control sequence and no timestamps.
    let doc = parse_and_validate(EXAMPLE).expect("example validates");
    let input = composed(
        "mac",
        vec![prop("tone", "accent"), prop("corner", "corner-full")],
    );
    let (a1, r1) = lower_native(&doc, &input).expect("lowering succeeds");
    let (a2, r2) = lower_native(&doc, &input).expect("lowering succeeds");
    let first = emit_native_json(&a1, &r1);
    assert_eq!(first, emit_native_json(&a2, &r2));
    assert!(first.contains("\"platform\": \"mac\""));
    assert!(first.contains("\"control_sequence\""));
    assert!(first.contains("\"a11y\""));
    assert!(first.contains("\"defaulted\""));
}

#[test]
fn lower_native_rejects_a_non_native_composed_document() {
    // The native tables lower native compositions only; the web
    // target lowers through lower_web (#5851).
    let doc = parse_and_validate(EXAMPLE).expect("example validates");
    let errors = lower_native(&doc, &composed("web", vec![])).expect_err("web target must fail");
    assert!(errors[0].message.contains("not a native"), "{}", errors[0]);
}
