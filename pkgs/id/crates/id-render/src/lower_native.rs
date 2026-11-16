//! Stages 4 and 5 of the #5855 pipeline for the native targets:
//! lower the resolved IR through the #5852 mapping tables
//! ([`crate::native_table`], consumed as data) and emit the
//! artefact plus the machine-readable coverage report.
//!
//! Boundaries, stated rather than silent:
//!
//! * **Native only.** The composed document's target names the
//!   platform (`mac`, `windows`, `gnome`); the platform selects
//!   the table column, never a code branch — the same lowering
//!   walks all three. Web lowering is [`crate::lower`]; custom-GUI
//!   lowering (#5853) consumes its own table through this same
//!   shape when it lands.
//! * **Standard controls only** (#5852 invariant 1): every
//!   element lowers to the platform's OS-standard control from
//!   §2 (or a composition of them). A kind with no §2 row lowers
//!   to the platform's standard container and is recorded in
//!   `element_defaults` (§2 preamble) — never a drawn substitute.
//! * **No silent divergence** (#5852 invariant 6, §3.1): an
//!   assertion whose table entry is the platform-default marker,
//!   and an assertion with no row at all, both lower to the
//!   platform default and are recorded in the report's
//!   `defaulted` list with the §3.1 warning text. Absence from
//!   the report means it did not happen.
//! * **Presentation overrides do not apply natively** (#5853's
//!   native exclusion, followed by #5852): a composed element
//!   carrying a presentation override lowers to its standard
//!   control anyway, and the exclusion is recorded in
//!   `presentation_excluded`.
//! * **Raw CSS does not exist on native targets** (#5852 A3): a
//!   raw-CSS winner is recorded as a platform-default entry, not
//!   carried as CSS text.
//! * **States are not lowered.** As in the web lowering, #5851
//!   §3.6-style state composition operates on the #5850 state
//!   grammar, which reaches the pipeline through the `assert`
//!   slot stage 2 still carries opaquely. On native targets the
//!   standard control's own state behaviour is the rendering
//!   (#5852 §3.8), checked against the contract by conformance,
//!   not re-created here.
//! * **Attributes come from the validated IR**, matched by
//!   document path, exactly as in the web lowering: #5852 §2's
//!   `field` rows are variant-driven (`field` kind attribute).

use std::collections::BTreeMap;

use crate::compose::{ComposedDocument, ComposedElement, ComposedNode, WinningLayer};
use crate::native_table;
use crate::vocabulary;
use crate::{BuildError, Document, Element, Node};

/// The native artefact (stage 5, #5855 §3.2): the lowered
/// document as the control-kind sequence plus the accessibility
/// tree, with the lowered element tree they are derived from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeArtefact {
    /// Platform the artefact was lowered for (`mac`, `windows`,
    /// or `gnome`).
    pub platform: String,
    /// The lowered root element.
    pub root: NativeElement,
    /// Every element's selected control, in document order —
    /// the control-kind sequence of #5855 §3.2.
    pub control_sequence: Vec<ControlEntry>,
    /// The accessibility tree (#5852 invariant 5): role, name,
    /// and structure from the semantic elements, identical across
    /// platforms; only the controls differ.
    pub a11y: A11yNode,
}

/// One lowered element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeElement {
    /// Document path (as in the resolved IR).
    pub path: String,
    /// Element kind, as in the source IR.
    pub kind: String,
    /// OS-standard control from the #5852 §2 table (or the
    /// recorded standard-container default).
    pub control: String,
    /// Accessibility role the kind carries by construction
    /// (#5849 §6), carried through lowering unchanged
    /// (#5852 invariant 5).
    pub role: String,
    /// Lowered children, in source order.
    pub children: Vec<NativeNode>,
}

/// One node of the native artefact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeNode {
    /// A lowered element.
    Element(NativeElement),
    /// Text content, carried unchanged.
    Text(String),
    /// A raw HTML region, carried unchanged.
    RawHtml(String),
}

/// One entry of the control-kind sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlEntry {
    /// Document path of the element.
    pub path: String,
    /// Element kind.
    pub kind: String,
    /// Control selected for it on this platform.
    pub control: String,
}

/// One node of the accessibility tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct A11yNode {
    /// Document path of the element.
    pub path: String,
    /// Element kind.
    pub kind: String,
    /// Accessibility role (#5849 §6).
    pub role: String,
    /// Accessible name: the element's `label` or `title`
    /// attribute when it carries one, else its direct text
    /// content, else empty.
    pub name: String,
    /// Child elements, in source order (text is name source,
    /// not a tree node).
    pub children: Vec<A11yNode>,
}

/// One assertion that found a mapping in the native table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappedNativeAssertion {
    /// Element the assertion applies to.
    pub element_path: String,
    /// Assertion dimension.
    pub property: String,
    /// Role token that was mapped.
    pub token: String,
    /// The platform rendering the table gives for the role.
    pub rendering: String,
}

/// One assertion that lowered to the platform default: its table
/// entry is the §3.1 platform-default marker, it has no row at
/// all, or it is a raw-CSS winner (which does not exist natively).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefaultedNativeAssertion {
    /// Element the assertion applies to.
    pub element_path: String,
    /// Assertion dimension.
    pub property: String,
    /// Role token that was defaulted (empty for raw-CSS winners,
    /// which carry a value, not a token).
    pub token: String,
    /// Platform the default applies on.
    pub platform: String,
    /// The §3.1 coverage-warning text.
    pub warning: String,
}

/// One element kind that had no #5852 §2 row and lowered to the
/// platform's standard container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeElementDefault {
    /// Element path.
    pub element_path: String,
    /// Element kind with no row.
    pub kind: String,
    /// Platform the default applies on.
    pub platform: String,
    /// The default control that was used.
    pub default_control: String,
}

/// One presentation override that did not apply because the
/// target is native (#5853's native exclusion).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresentationExcluded {
    /// Element path.
    pub element_path: String,
    /// The override's name.
    pub presentation: String,
}

/// The stage-5 coverage/conflict report for a native target
/// (#5855 §2.1 stage 5; #5852 §3.1, invariant 6): assertions
/// mapped, assertions platform-defaulted, element defaults,
/// presentation exclusions, and the composition report's
/// reported overrides carried through. Deterministic in content
/// and order; no timestamps (#5855 invariant 2).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NativeReport {
    /// Assertions with a native-table mapping, in document order.
    pub mapped: Vec<MappedNativeAssertion>,
    /// Assertions defaulted to the platform rendering, in
    /// document order.
    pub defaulted: Vec<DefaultedNativeAssertion>,
    /// Element kinds defaulted for lack of a §2 row.
    pub element_defaults: Vec<NativeElementDefault>,
    /// Presentation overrides excluded on this native target.
    pub presentation_excluded: Vec<PresentationExcluded>,
    /// Raw-CSS reported overrides, carried from the composition
    /// report (#5854 §6b) — reported, never silent.
    pub reported_overrides: Vec<crate::compose::ReportedOverride>,
}

/// Stage 4 + 5 for a native target: lower `composed` (whose
/// structure and attributes are read from the validated `doc`)
/// through the #5852 tables for the platform `composed` names,
/// and emit the artefact and report.
///
/// # Errors
///
/// Returns a build error when the composed document's target is
/// not one of the native platforms, or when the validated
/// document and the resolved IR disagree on an element path.
pub fn lower_native(
    doc: &Document,
    composed: &ComposedDocument,
) -> Result<(NativeArtefact, NativeReport), Vec<BuildError>> {
    let platform = composed.target.as_str();
    if !native_table::is_platform(platform) {
        return Err(vec![BuildError::new(
            format!(
                "lower_native: composed document target is {platform:?}, not a native \
                 platform (mac, windows, gnome); the web target lowers through \
                 lower_web (#5851) and custom GUI through its own table (#5853)"
            ),
            composed.root.span.line,
            composed.root.span.col,
        )]);
    }
    let mut source_by_path: BTreeMap<String, &Element> = BTreeMap::new();
    index_elements(&doc.root, "document", &mut source_by_path);

    let mut report = NativeReport {
        reported_overrides: composed.report.reported_overrides.clone(),
        ..NativeReport::default()
    };
    let mut control_sequence: Vec<ControlEntry> = Vec::new();
    let root = lower_element(
        &composed.root,
        &source_by_path,
        platform,
        &mut report,
        &mut control_sequence,
    )?;
    let a11y = a11y_tree(&root, &source_by_path);

    Ok((
        NativeArtefact {
            platform: platform.to_string(),
            root,
            control_sequence,
            a11y,
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
    platform: &str,
    report: &mut NativeReport,
    control_sequence: &mut Vec<ControlEntry>,
) -> Result<NativeElement, Vec<BuildError>> {
    let source = source_by_path.get(&el.path).ok_or_else(|| {
        vec![BuildError::new(
            format!(
                "lower_native: resolved element at {} has no counterpart in the \
                 validated document; the two inputs are not the same document",
                el.path
            ),
            el.span.line,
            el.span.col,
        )]
    })?;

    // #5852 §2: variant row, fixed row, or the recorded
    // standard-container default.
    let control = resolve_control(el, source, platform, report);
    control_sequence.push(ControlEntry {
        path: el.path.clone(),
        kind: el.name.clone(),
        control: control.clone(),
    });

    // Presentation overrides do not apply to native standard
    // controls (#5853): the exclusion is recorded, never silent.
    if let Some(presentation) = &el.presentation {
        report.presentation_excluded.push(PresentationExcluded {
            element_path: el.path.clone(),
            presentation: presentation.clone(),
        });
    }

    for prop in &el.properties {
        lower_assertion(el, prop, platform, report);
    }

    let role = vocabulary::lookup(&el.name).map_or("", |spec| spec.role);

    let mut children = Vec::new();
    for child in &el.children {
        children.push(match child {
            ComposedNode::Element(child_el) => NativeNode::Element(lower_element(
                child_el,
                source_by_path,
                platform,
                report,
                control_sequence,
            )?),
            ComposedNode::Text(text) => NativeNode::Text(text.clone()),
            ComposedNode::RawHtml(raw) => NativeNode::RawHtml(raw.content.clone()),
        });
    }

    Ok(NativeElement {
        path: el.path.clone(),
        kind: el.name.clone(),
        control,
        role: role.to_string(),
        children,
    })
}

/// Resolves the OS-standard control for one element: variant row
/// first for variant-driven kinds, then the fixed table, then
/// the recorded standard-container default (#5852 §2 preamble).
fn resolve_control(
    el: &ComposedElement,
    source: &Element,
    platform: &str,
    report: &mut NativeReport,
) -> String {
    if let Some(control) = native_table::variant_control(&el.name, source.attr("kind"), platform) {
        return control.to_string();
    }
    if let Some(row) = native_table::control_row(&el.name) {
        return row.control_on(platform).to_string();
    }
    report.element_defaults.push(NativeElementDefault {
        element_path: el.path.clone(),
        kind: el.name.clone(),
        platform: platform.to_string(),
        default_control: "standard container".to_string(),
    });
    "standard container".to_string()
}

/// Lowers one resolved assertion through the native table,
/// recording the outcome in the report either way.
fn lower_assertion(
    el: &ComposedElement,
    prop: &crate::compose::ResolvedProperty,
    platform: &str,
    report: &mut NativeReport,
) {
    if prop.layer == WinningLayer::RawCss {
        // Raw CSS does not exist on native targets (#5852 A3):
        // the winner cannot be carried; it is a platform-default
        // entry, recorded with its value in the warning.
        let value = prop.raw_value.as_deref().unwrap_or_default();
        report.defaulted.push(DefaultedNativeAssertion {
            element_path: el.path.clone(),
            property: prop.property.clone(),
            token: String::new(),
            platform: platform.to_string(),
            warning: format!(
                "raw CSS has no native mapping on {platform}; platform default used \
                 ({}: {value} on {})",
                prop.property, el.path
            ),
        });
        return;
    }
    let Some(token) = &prop.token else {
        return;
    };
    if let Some(rendering) = native_table::spacing_rendering(&prop.property, token, platform) {
        report.mapped.push(MappedNativeAssertion {
            element_path: el.path.clone(),
            property: prop.property.clone(),
            token: token.clone(),
            rendering,
        });
        return;
    }
    if let Some(row) = native_table::role_row(&prop.property, token) {
        match row.rendering_on(platform) {
            Some(rendering) => report.mapped.push(MappedNativeAssertion {
                element_path: el.path.clone(),
                property: prop.property.clone(),
                token: token.clone(),
                rendering: rendering.to_string(),
            }),
            None => report
                .defaulted
                .push(platform_default(el, prop, token, platform)),
        }
        return;
    }
    report
        .defaulted
        .push(platform_default(el, prop, token, platform));
}

/// The §3.1 platform-default record for one assertion.
fn platform_default(
    el: &ComposedElement,
    prop: &crate::compose::ResolvedProperty,
    token: &str,
    platform: &str,
) -> DefaultedNativeAssertion {
    DefaultedNativeAssertion {
        element_path: el.path.clone(),
        property: prop.property.clone(),
        token: token.to_string(),
        platform: platform.to_string(),
        warning: format!(
            "assertion has no native mapping on {platform}; platform default used \
             ({}: {token} on {})",
            prop.property, el.path
        ),
    }
}

/// Builds the accessibility tree from the lowered tree and the
/// validated IR (for accessible names).
fn a11y_tree(el: &NativeElement, source_by_path: &BTreeMap<String, &Element>) -> A11yNode {
    let name = source_by_path
        .get(&el.path)
        .map_or_else(String::new, |source| {
            if let Some(label) = source.attr("label") {
                return label.to_string();
            }
            if let Some(title) = source.attr("title") {
                return title.to_string();
            }
            let mut texts: Vec<&str> = Vec::new();
            for child in &el.children {
                if let NativeNode::Text(text) = child {
                    texts.push(text);
                }
            }
            texts.join(" ")
        });
    let mut children = Vec::new();
    for child in &el.children {
        if let NativeNode::Element(child_el) = child {
            children.push(a11y_tree(child_el, source_by_path));
        }
    }
    A11yNode {
        path: el.path.clone(),
        kind: el.kind.clone(),
        role: el.role.clone(),
        name,
        children,
    }
}

/// Stage 5 emit: the artefact and report as one deterministic
/// JSON document (sorted keys in fixed schema order, no
/// timestamps — #5855 invariant 2). This is the machine-readable
/// report of #5855 §2.1 stage 5 in its native form.
#[must_use]
pub fn emit_native_json(artefact: &NativeArtefact, report: &NativeReport) -> String {
    let mut out = String::new();
    out.push_str("{\n  \"target\": ");
    push_json_string(&mut out, &artefact.platform);
    out.push_str(",\n  \"platform\": ");
    push_json_string(&mut out, &artefact.platform);
    out.push_str(",\n  \"control_sequence\": ");
    push_control_sequence_json(&mut out, &artefact.control_sequence, 1);
    out.push_str(",\n  \"root\": ");
    push_element_json(&mut out, &artefact.root, 1);
    out.push_str(",\n  \"a11y\": ");
    push_a11y_json(&mut out, &artefact.a11y, 1);
    out.push_str(",\n  \"report\": ");
    push_report_json(&mut out, report, 1);
    out.push_str("\n}\n");
    out
}

fn push_json_string(out: &mut String, value: &str) {
    crate::lower::push_json_string(out, value);
}

fn indent(out: &mut String, depth: usize) {
    crate::lower::indent(out, depth);
}

fn push_control_sequence_json(out: &mut String, entries: &[ControlEntry], depth: usize) {
    if entries.is_empty() {
        out.push_str("[]");
        return;
    }
    out.push_str("[\n");
    for (i, entry) in entries.iter().enumerate() {
        indent(out, depth + 1);
        out.push_str("{\"path\": ");
        push_json_string(out, &entry.path);
        out.push_str(", \"kind\": ");
        push_json_string(out, &entry.kind);
        out.push_str(", \"control\": ");
        push_json_string(out, &entry.control);
        out.push('}');
        if i + 1 < entries.len() {
            out.push(',');
        }
        out.push('\n');
    }
    indent(out, depth);
    out.push(']');
}

fn push_element_json(out: &mut String, el: &NativeElement, depth: usize) {
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
    field(out, "control");
    push_json_string(out, &el.control);
    out.push_str(",\n");
    field(out, "role");
    push_json_string(out, &el.role);
    out.push_str(",\n");
    field(out, "children");
    if el.children.is_empty() {
        out.push_str("[]");
    } else {
        out.push_str("[\n");
        for (i, child) in el.children.iter().enumerate() {
            indent(out, depth + 2);
            match child {
                NativeNode::Element(child_el) => push_element_json(out, child_el, depth + 2),
                NativeNode::Text(text) => {
                    out.push_str("{\"text\": ");
                    push_json_string(out, text);
                    out.push('}');
                }
                NativeNode::RawHtml(raw) => {
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

fn push_a11y_json(out: &mut String, node: &A11yNode, depth: usize) {
    out.push_str("{\n");
    let field = |out: &mut String, name: &str| {
        indent(out, depth + 1);
        push_json_string(out, name);
        out.push_str(": ");
    };
    field(out, "path");
    push_json_string(out, &node.path);
    out.push_str(",\n");
    field(out, "kind");
    push_json_string(out, &node.kind);
    out.push_str(",\n");
    field(out, "role");
    push_json_string(out, &node.role);
    out.push_str(",\n");
    field(out, "name");
    push_json_string(out, &node.name);
    out.push_str(",\n");
    field(out, "children");
    if node.children.is_empty() {
        out.push_str("[]");
    } else {
        out.push_str("[\n");
        for (i, child) in node.children.iter().enumerate() {
            indent(out, depth + 2);
            push_a11y_json(out, child, depth + 2);
            if i + 1 < node.children.len() {
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

fn push_report_json(out: &mut String, report: &NativeReport, depth: usize) {
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
        out.push_str(", \"rendering\": ");
        push_json_string(out, &m.rendering);
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
        out.push_str(", \"platform\": ");
        push_json_string(out, &d.platform);
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
        out.push_str(", \"platform\": ");
        push_json_string(out, &d.platform);
        out.push_str(", \"default_control\": ");
        push_json_string(out, &d.default_control);
        out.push('}');
    }
    out.push(']');
    out.push_str(",\n");
    field(out, "presentation_excluded");
    out.push('[');
    for (i, p) in report.presentation_excluded.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str("{\"element\": ");
        push_json_string(out, &p.element_path);
        out.push_str(", \"presentation\": ");
        push_json_string(out, &p.presentation);
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
