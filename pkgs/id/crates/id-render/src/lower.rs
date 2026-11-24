//! Stages 4 and 5 of the #5855 pipeline for the web target:
//! lower the resolved IR through the #5851 mapping table
//! ([`crate::web_table`], consumed as data) and emit the artefact
//! plus the machine-readable coverage report.
//!
//! Boundaries, stated rather than silent:
//!
//! * **Web only.** Native lowering (#5852) lives in
//!   [`crate::lower_native`] and custom-GUI lowering (#5853)
//!   consumes its own table through this same shape when it
//!   lands; nothing here branches on a target name — the web table
//!   is the input that makes this the web lowering.
//! * **Attributes come from the validated IR.** Stage 3's resolved
//!   IR carries each element's final assertion set but not its
//!   source attributes, and #5851 §2 rows are variant-driven
//!   (`heading` level, `field` kind, …). Lowering therefore walks
//!   the validated [`Document`](crate::Document) and the
//!   [`ComposedDocument`] together, matching elements by document
//!   path; a path mismatch is a build error, never a guess.
//! * **States are not lowered yet.** #5851 §3.6 state composition
//!   operates on state deltas, which #5850 §5 fixes as
//!   target-derived from the base roles — the `assert` slot names
//!   no per-state styling, so there is nothing state-shaped in
//!   the resolved properties to lower. Resolved properties
//!   are the assertion dimensions of #5850's closed v1 set as
//!   adopted by #5851 A3.
//! * **No silent fallback** (#5855 invariant 4, §3.3): an
//!   assertion with no row in the web table lowers to the layer
//!   default (no utilities) and is recorded in the report's
//!   `defaulted` list with the #5851 §3.7 warning text; an element
//!   kind with no §2 row lowers to `div` and is recorded in
//!   `element_defaults`. Absence from the report means it did
//!   not happen.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::compose::{ComposedDocument, ComposedElement, ComposedNode, WinningLayer};
use crate::vocabulary;
use crate::web_table;
use crate::{BuildError, Document, Element, Node};

/// The web artefact (stage 5): the lowered document plus the
/// generated class set, emitted explicitly because content
/// scanning cannot discover classes no author wrote (#5851 §3.8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebArtefact {
    /// Target name (the composed document's target).
    pub target: String,
    /// The lowered root element.
    pub root: WebElement,
    /// Every utility class the artefact uses, sorted and
    /// deduplicated — the safelist form of #5851 §3.8.
    pub class_set: Vec<String>,
}

/// One lowered element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebElement {
    /// Document path (as in the resolved IR).
    pub path: String,
    /// Element kind, as in the source IR.
    pub kind: String,
    /// DOM tag from the #5851 §2 table (or the recorded default).
    pub tag: String,
    /// Accessibility role the kind carries by construction
    /// (#5849 §6), carried through lowering unchanged
    /// (#5855 invariant 5).
    pub role: String,
    /// DOM attributes the lowering sets (role, type, …), sorted
    /// by name for determinism.
    pub attributes: Vec<(String, String)>,
    /// Utilities emitted for this element, order-normalised
    /// (sorted, deduplicated; #5855 §3.2).
    pub classes: Vec<String>,
    /// Raw-CSS declarations that won a property on this element
    /// (layer 5): (property, value), in property order. They are
    /// author CSS text, never classes (#5851 §5).
    pub raw_css: Vec<(String, String)>,
    /// The winning presentation override's name, when one
    /// applies; `classes` then describe the fallback standard
    /// rendering only (#5854 §2.1 layer 4).
    pub presentation: Option<String>,
    /// Lowered children, in source order.
    pub children: Vec<WebNode>,
}

/// One node of the web artefact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebNode {
    /// A lowered element.
    Element(WebElement),
    /// Text content, carried unchanged.
    Text(String),
    /// A raw HTML region, carried unchanged.
    RawHtml(String),
}

/// One assertion that found a row in the web table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappedAssertion {
    /// Element the assertion applies to.
    pub element_path: String,
    /// Assertion dimension.
    pub property: String,
    /// Role token that was mapped.
    pub token: String,
    /// Utilities emitted (empty for dimensions that resolve
    /// outside the class set, e.g. density, #5851 §3.3).
    pub classes: Vec<String>,
}

/// One assertion that had no row in the web table and lowered
/// to the layer default (#5855 §2.1 stage 4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefaultedAssertion {
    /// Element the assertion applies to.
    pub element_path: String,
    /// Assertion dimension.
    pub property: String,
    /// Role token that had no mapping.
    pub token: String,
    /// The default that was used: `none` (no utilities emitted).
    pub default: String,
    /// The #5851 §3.7 coverage-warning text.
    pub warning: String,
}

/// One element kind that had no #5851 §2 row and lowered to the
/// default tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElementDefault {
    /// Element path.
    pub element_path: String,
    /// Element kind with no row.
    pub kind: String,
    /// The default tag that was used.
    pub default_tag: String,
}

/// The stage-5 coverage/conflict report for the web target
/// (#5855 §2.1 stage 5, §3.3): assertions mapped, assertions
/// defaulted, element defaults, and the composition report's
/// reported overrides carried through. Deterministic in content
/// and order; no timestamps (#5855 invariant 2).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LowerReport {
    /// Assertions with a web-table row, in document order.
    pub mapped: Vec<MappedAssertion>,
    /// Assertions defaulted for lack of a row, in document order.
    pub defaulted: Vec<DefaultedAssertion>,
    /// Element kinds defaulted for lack of a §2 row.
    pub element_defaults: Vec<ElementDefault>,
    /// Raw-CSS reported overrides, carried from the composition
    /// report (#5854 §6b) — reported, never silent.
    pub reported_overrides: Vec<crate::compose::ReportedOverride>,
}

/// Stage 4 + 5 for the web target: lower `composed` (whose
/// structure and attributes are read from the validated `doc`)
/// through the #5851 table and emit the artefact and report.
///
/// # Errors
///
/// Returns a build error when the composed document's target is
/// not the web target, or when the validated document and the
/// resolved IR disagree on an element path (the two inputs are
/// not the same document).
pub fn lower_web(
    doc: &Document,
    composed: &ComposedDocument,
) -> Result<(WebArtefact, LowerReport), Vec<BuildError>> {
    if composed.target != "web" {
        return Err(vec![BuildError::new(
            format!(
                "lower_web: composed document target is {:?}, not \"web\"; \
                 other targets lower through their own tables (#5852/#5853)",
                composed.target
            ),
            composed.root.span.line,
            composed.root.span.col,
        )]);
    }
    let mut source_by_path: BTreeMap<String, &Element> = BTreeMap::new();
    index_elements(&doc.root, "document", &mut source_by_path);

    let mut report = LowerReport {
        reported_overrides: composed.report.reported_overrides.clone(),
        ..LowerReport::default()
    };
    let root = lower_element(&composed.root, &source_by_path, &mut report)?;

    let mut class_set: Vec<String> = Vec::new();
    collect_classes(&root, &mut class_set);
    class_set.sort();
    class_set.dedup();

    Ok((
        WebArtefact {
            target: composed.target.clone(),
            root,
            class_set,
        },
        report,
    ))
}

/// Indexes the validated IR by document path, using the same
/// path derivation as composition.
fn index_elements<'a>(element: &'a Element, path: &str, out: &mut BTreeMap<String, &'a Element>) {
    out.insert(path.to_string(), element);
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for child in &element.children {
        if let Node::Element(child_el) = child {
            let index = {
                let entry = counts.entry(child_el.name.as_str()).or_insert(0);
                let i = *entry;
                *entry += 1;
                i
            };
            let child_path = crate::compose::element_path(path, &child_el.name, index);
            index_elements(child_el, &child_path, out);
        }
    }
}

fn lower_element(
    el: &ComposedElement,
    source_by_path: &BTreeMap<String, &Element>,
    report: &mut LowerReport,
) -> Result<WebElement, Vec<BuildError>> {
    let source = source_by_path.get(&el.path).ok_or_else(|| {
        vec![BuildError::new(
            format!(
                "lower_web: resolved element at {} has no counterpart in the \
                 validated document; the two inputs are not the same document",
                el.path
            ),
            el.span.line,
            el.span.col,
        )]
    })?;

    // #5851 §2: fixed row, variant row, or the recorded default.
    let variant = source.attr("kind").or_else(|| source.attr("level"));
    let (tag, mut attributes) = resolve_tag(el, source, variant, report);

    // The accessibility role survives lowering (#5855 invariant
    // 5): the artefact records the #5849 role for every node, and
    // §2's fixed role attributes land in the DOM attribute set.
    let role = vocabulary::lookup(&el.name).map_or("", |spec| spec.role);
    if let Some(row) = web_table::element_row(&el.name) {
        if let Some(role_attr) = row.role_attr {
            attributes.push(("role".to_string(), role_attr.to_string()));
        }
    }
    if el.name == "alert" {
        attributes.push((
            "role".to_string(),
            web_table::alert_role(source.attr("kind")).to_string(),
        ));
    }
    attributes.sort();
    attributes.dedup();

    let mut classes: Vec<String> = Vec::new();
    let mut raw_css: Vec<(String, String)> = Vec::new();
    for prop in &el.properties {
        if prop.layer == WinningLayer::RawCss {
            if let Some(value) = &prop.raw_value {
                raw_css.push((prop.property.clone(), value.clone()));
            }
            continue;
        }
        let Some(token) = &prop.token else {
            continue;
        };
        lower_assertion(el, prop.property.as_str(), token, &mut classes, report);
    }
    classes.sort();
    classes.dedup();

    let mut children = Vec::new();
    for child in &el.children {
        children.push(match child {
            ComposedNode::Element(child_el) => {
                WebNode::Element(lower_element(child_el, source_by_path, report)?)
            }
            ComposedNode::Text(text) => WebNode::Text(text.clone()),
            ComposedNode::RawHtml(raw) => WebNode::RawHtml(raw.content.clone()),
        });
    }

    Ok(WebElement {
        path: el.path.clone(),
        kind: el.name.clone(),
        tag,
        role: role.to_string(),
        attributes,
        classes,
        raw_css,
        presentation: el.presentation.clone(),
        children,
    })
}

/// Resolves the DOM tag for one element: variant row first for
/// variant-driven kinds, then the fixed table, then the recorded
/// default (`div`).
fn resolve_tag(
    el: &ComposedElement,
    source: &Element,
    variant: Option<&str>,
    report: &mut LowerReport,
) -> (String, Vec<(String, String)>) {
    let mut attributes: Vec<(String, String)> = Vec::new();
    if let Some(tag) = web_table::variant_tag(&el.name, variant) {
        if el.name == "field" {
            if let Some(input_type) = web_table::field_input_type(variant) {
                attributes.push(("type".to_string(), input_type.to_string()));
            }
        }
        return (tag.to_string(), attributes);
    }
    if let Some(row) = web_table::element_row(&el.name) {
        return (row.tag.to_string(), attributes);
    }
    let _ = source;
    report.element_defaults.push(ElementDefault {
        element_path: el.path.clone(),
        kind: el.name.clone(),
        default_tag: "div".to_string(),
    });
    ("div".to_string(), attributes)
}

/// Lowers one resolved assertion through the web table,
/// recording the outcome in the report either way.
fn lower_assertion(
    el: &ComposedElement,
    property: &str,
    token: &str,
    classes: &mut Vec<String>,
    report: &mut LowerReport,
) {
    if property == "density" {
        // #5851 §3.3: density is a container-level theme
        // multiplier; it emits no utilities and is not a default.
        report.mapped.push(MappedAssertion {
            element_path: el.path.clone(),
            property: property.to_string(),
            token: token.to_string(),
            classes: Vec::new(),
        });
        return;
    }
    if let Some(row) = web_table::role_row(property, token) {
        classes.extend(row.classes.iter().map(ToString::to_string));
        report.mapped.push(MappedAssertion {
            element_path: el.path.clone(),
            property: property.to_string(),
            token: token.to_string(),
            classes: row.classes.iter().map(ToString::to_string).collect(),
        });
        return;
    }
    if let Some(class) = web_table::spacing_class(property, token) {
        classes.push(class.clone());
        report.mapped.push(MappedAssertion {
            element_path: el.path.clone(),
            property: property.to_string(),
            token: token.to_string(),
            classes: vec![class],
        });
        return;
    }
    report.defaulted.push(DefaultedAssertion {
        element_path: el.path.clone(),
        property: property.to_string(),
        token: token.to_string(),
        default: "none".to_string(),
        warning: format!(
            "this assertion has no web mapping; platform default will be used \
             ({property}: {token} on {})",
            el.path
        ),
    });
}

fn collect_classes(el: &WebElement, out: &mut Vec<String>) {
    out.extend(el.classes.iter().cloned());
    for child in &el.children {
        if let WebNode::Element(child_el) = child {
            collect_classes(child_el, out);
        }
    }
}

/// Stage 5 emit: the artefact and report as one deterministic
/// JSON document (sorted keys in fixed schema order, no
/// timestamps — #5855 invariant 2). This is the machine-readable
/// report of #5855 §2.1 stage 5 in its web form.
#[must_use]
pub fn emit_json(artefact: &WebArtefact, report: &LowerReport) -> String {
    let mut out = String::new();
    out.push_str("{\n  \"target\": ");
    push_json_string(&mut out, &artefact.target);
    out.push_str(",\n  \"class_set\": ");
    push_json_string_list(&mut out, &artefact.class_set, 1);
    out.push_str(",\n  \"root\": ");
    push_element_json(&mut out, &artefact.root, 1);
    out.push_str(",\n  \"report\": ");
    push_report_json(&mut out, report, 1);
    out.push_str("\n}\n");
    out
}

pub(crate) fn push_json_string(out: &mut String, value: &str) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

pub(crate) fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn push_json_string_list(out: &mut String, values: &[String], depth: usize) {
    if values.is_empty() {
        out.push_str("[]");
        return;
    }
    out.push_str("[\n");
    for (i, value) in values.iter().enumerate() {
        indent(out, depth + 1);
        push_json_string(out, value);
        if i + 1 < values.len() {
            out.push(',');
        }
        out.push('\n');
    }
    indent(out, depth);
    out.push(']');
}

fn push_element_json(out: &mut String, el: &WebElement, depth: usize) {
    out.push_str("{\n");
    let field = |out: &mut String, name: &str| {
        indent(out, depth + 1);
        push_json_string(out, name);
        out.push_str(": ");
    };
    field(out, "path");
    push_json_string(out, &el.path);
    out.push_str(",\n");
    field(out, "kind");
    push_json_string(out, &el.kind);
    out.push_str(",\n");
    field(out, "tag");
    push_json_string(out, &el.tag);
    out.push_str(",\n");
    field(out, "role");
    push_json_string(out, &el.role);
    out.push_str(",\n");
    field(out, "attributes");
    out.push('{');
    for (i, (name, value)) in el.attributes.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        push_json_string(out, name);
        out.push_str(": ");
        push_json_string(out, value);
    }
    out.push('}');
    out.push_str(",\n");
    field(out, "classes");
    push_json_string_list(out, &el.classes, depth + 1);
    out.push_str(",\n");
    field(out, "children");
    if el.children.is_empty() {
        out.push_str("[]");
    } else {
        out.push_str("[\n");
        for (i, child) in el.children.iter().enumerate() {
            indent(out, depth + 2);
            match child {
                WebNode::Element(child_el) => push_element_json(out, child_el, depth + 2),
                WebNode::Text(text) => {
                    out.push_str("{\"text\": ");
                    push_json_string(out, text);
                    out.push('}');
                }
                WebNode::RawHtml(raw) => {
                    out.push_str("{\"raw_html\": ");
                    push_json_string(out, raw);
                    out.push('}');
                }
            }
            if i + 1 < el.children.len() {
                out.push(',');
            }
            out.push('\n');
        }
        indent(out, depth + 1);
        out.push(']');
    }
    out.push('\n');
    indent(out, depth);
    out.push('}');
}

fn push_report_json(out: &mut String, report: &LowerReport, depth: usize) {
    out.push_str("{\n");
    let field = |out: &mut String, name: &str| {
        indent(out, depth + 1);
        push_json_string(out, name);
        out.push_str(": ");
    };
    field(out, "mapped");
    out.push('[');
    for (i, m) in report.mapped.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str("{\"element\": ");
        push_json_string(out, &m.element_path);
        out.push_str(", \"property\": ");
        push_json_string(out, &m.property);
        out.push_str(", \"token\": ");
        push_json_string(out, &m.token);
        out.push_str(", \"classes\": ");
        push_json_string_list(out, &m.classes, 0);
        out.push('}');
    }
    out.push(']');
    out.push_str(",\n");
    field(out, "defaulted");
    out.push('[');
    for (i, d) in report.defaulted.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str("{\"element\": ");
        push_json_string(out, &d.element_path);
        out.push_str(", \"property\": ");
        push_json_string(out, &d.property);
        out.push_str(", \"token\": ");
        push_json_string(out, &d.token);
        out.push_str(", \"default\": ");
        push_json_string(out, &d.default);
        out.push_str(", \"warning\": ");
        push_json_string(out, &d.warning);
        out.push('}');
    }
    out.push(']');
    out.push_str(",\n");
    field(out, "element_defaults");
    out.push('[');
    for (i, d) in report.element_defaults.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str("{\"element\": ");
        push_json_string(out, &d.element_path);
        out.push_str(", \"kind\": ");
        push_json_string(out, &d.kind);
        out.push_str(", \"default_tag\": ");
        push_json_string(out, &d.default_tag);
        out.push('}');
    }
    out.push(']');
    out.push_str(",\n");
    field(out, "reported_overrides");
    out.push('[');
    for (i, o) in report.reported_overrides.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str("{\"element\": ");
        push_json_string(out, &o.element_path);
        out.push_str(", \"property\": ");
        push_json_string(out, &o.property);
        out.push_str(", \"selector\": ");
        push_json_string(out, &o.selector);
        out.push_str(", \"value\": ");
        push_json_string(out, &o.value);
        out.push('}');
    }
    out.push(']');
    out.push('\n');
    indent(out, depth);
    out.push('}');
}
