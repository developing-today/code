//! Stages 4 and 5 of the #5855 pipeline for the custom-GUI
//! target: lower the resolved IR through the #5853 presentation
//! override contract and emit the artefact plus the
//! machine-readable coverage report.
//!
//! The shape, from #5853 §2: a presentation override lives in the
//! target's override map, never in the semantic source. Stage 3
//! has already resolved which override applies to each element
//! (binding and specificity, §2.5) and recorded its name on the
//! resolved IR; this stage consumes the override **definitions**
//! as target-side data ([`GuiData`], the same data-input shape as
//! stage 3's `ComposeData`) and decides, per element, the
//! presentation choice the artefact carries:
//!
//! * **Standard** — no override applies; the element renders in
//!   its standard form (§2.1).
//! * **Asset / drawn override applied** — the definition's kind
//!   (§2.2), with its per-state assets or its registered
//!   renderer's per-state behaviours.
//! * **Fallback** — the override applies but cannot be produced
//!   (missing asset, or a registered renderer that failed to
//!   load): the standard rendering stands and the report records
//!   the exercised fallback with its warning (§2.4). An element
//!   never renders as nothing.
//!
//! Build-time errors, never runtime gaps (§2.2, §2.3):
//!
//! * a drawn presentation naming a renderer the override pack
//!   has not registered;
//! * an override whose covered states (explicit plus declared
//!   derivations) do not equal the element kind's full state set
//!   in [`crate::gui_table`] — the missing states are named;
//! * a composed element naming an override the pack carries no
//!   definition for.
//!
//! Semantics are presentation-proof (§3 invariant 1): role,
//! accessible name, value, and state come from the semantic
//! element, and the emitted accessibility tree is built from the
//! semantic document alone — identical with overrides applied,
//! with none applied, and in fallback mode. Essential content is
//! never in the asset (§3 invariant 5): the artefact carries
//! asset and renderer **names**, never content.
//!
//! Native targets never reach this code: they ignore
//! presentation overrides for standard controls (§4), which
//! [`crate::lower_native`] records as exclusions.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use crate::compose::{ComposedDocument, ComposedElement, ComposedNode};
use crate::gui_table;
use crate::lower_native::A11yNode;
use crate::vocabulary;
use crate::{BuildError, Document, Element, Node};

/// The two presentation kinds of #5853 §2.2. There is no third
/// kind in v1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentationKind {
    /// A referenced asset (or asset pack) with per-state assets.
    Asset,
    /// A renderer registered by name in the target's override
    /// pack, implementing the element's interaction contract.
    Drawn,
}

/// One presentation override definition: a target-side entry of
/// the GUI override pack, keyed by the name stage 3 recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuiOverrideDef {
    /// Override name (the name composition binds and records).
    pub name: String,
    /// Which of the two §2.2 kinds this override is.
    pub kind: PresentationKind,
    /// The asset the override renders, for the asset kind:
    /// a content-addressed asset id from the pack.
    pub asset: Option<String>,
    /// The registered renderer name, for the drawn kind.
    pub renderer: Option<String>,
    /// States with an explicit asset / renderer behaviour (§2.3).
    pub covered_states: Vec<String>,
    /// Declared derivations for the remaining states: (state,
    /// derivation), e.g. `("disabled", "default asset at the
    /// disabled token's opacity")` (§2.3).
    pub derivations: Vec<(String, String)>,
}

/// The GUI target's lowering data: the override pack stage 4
/// consumes. All of it is target-side; none of it appears in the
/// semantic source (§3 invariant 6).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GuiData {
    /// Override definitions, by name.
    pub overrides: Vec<GuiOverrideDef>,
    /// Renderer names the pack registers (§2.2: registration is
    /// the gate for drawn presentations).
    pub registered_renderers: Vec<String>,
    /// Asset ids the pack can produce at build time. An applied
    /// asset override naming an absent asset exercises the
    /// fallback (§2.4).
    pub available_assets: Vec<String>,
    /// Registered renderers that failed to load at build time:
    /// their overrides exercise the fallback (§2.4) — a load
    /// failure is not the §2.2 registration error.
    pub failed_renderers: Vec<String>,
}

impl GuiData {
    fn definition(&self, name: &str) -> Option<&GuiOverrideDef> {
        self.overrides.iter().find(|d| d.name == name)
    }
}

/// The presentation choice one element lowers to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuiPresentation {
    /// No override applies: the standard rendering (§2.1).
    Standard,
    /// An asset override applies: the asset, with the states it
    /// covers explicitly and by derivation, sorted by state.
    Asset {
        /// Override name.
        name: String,
        /// Asset id.
        asset: String,
        /// (state, how it is covered: `explicit` or the declared
        /// derivation text), sorted by state.
        states: Vec<(String, String)>,
    },
    /// A drawn override applies: the registered renderer, with
    /// its state coverage as in the asset form.
    Drawn {
        /// Override name.
        name: String,
        /// Registered renderer name.
        renderer: String,
        /// (state, coverage) pairs, sorted by state.
        states: Vec<(String, String)>,
    },
    /// An override applies but its presentation cannot be
    /// produced: the standard rendering stands (§2.4).
    Fallback {
        /// Override name.
        name: String,
        /// Why the fallback was exercised (the report carries
        /// the same fact as a warning).
        reason: String,
    },
}

/// One lowered GUI element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuiElement {
    /// Document path (as in the resolved IR).
    pub path: String,
    /// Element kind, as in the source IR.
    pub kind: String,
    /// Accessibility role the kind carries by construction
    /// (#5849 §6), unchanged by any override (#5853 §3
    /// invariant 1).
    pub role: String,
    /// The presentation choice for this element.
    pub presentation: GuiPresentation,
    /// Lowered children, in source order.
    pub children: Vec<GuiNode>,
}

/// One node of the GUI artefact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuiNode {
    /// A lowered element.
    Element(GuiElement),
    /// Text content, carried unchanged (essential content lives
    /// in the semantic layer, never in a presentation, §3
    /// invariant 5).
    Text(String),
    /// A raw HTML region, carried unchanged.
    RawHtml(String),
}

/// The GUI artefact (stage 5): the lowered document — the
/// presentation choice per node — plus the accessibility tree,
/// which is the standard element's tree in every mode (§3
/// conformance model).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuiArtefact {
    /// Target name (the composed document's target: `gui`).
    pub target: String,
    /// The lowered root element.
    pub root: GuiElement,
    /// The accessibility tree, built from the semantic document
    /// alone: identical with overrides applied, with none, and
    /// in fallback mode (§3 invariant 1).
    pub a11y: A11yNode,
}

/// One override that was applied to an element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedOverride {
    /// Element path.
    pub element_path: String,
    /// Override name.
    pub name: String,
    /// `asset` or `drawn` (§2.2).
    pub kind: String,
}

/// One exercised fallback (§2.4): the standard rendering stood
/// in for an override that could not be produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExercisedFallback {
    /// Element path.
    pub element_path: String,
    /// Override name.
    pub name: String,
    /// The build warning: what was missing and what stood in.
    pub warning: String,
}

/// The stage-5 report for the GUI target: overrides applied,
/// fallbacks exercised, and the composition report's reported
/// overrides carried through. Deterministic in content and
/// order; no timestamps (#5855 invariant 2).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GuiReport {
    /// Overrides applied, in document order.
    pub applied: Vec<AppliedOverride>,
    /// Fallbacks exercised, in document order — reported, never
    /// silent (§2.4).
    pub fallbacks: Vec<ExercisedFallback>,
    /// Raw-CSS reported overrides, carried from the composition
    /// report (#5854 §6b).
    pub reported_overrides: Vec<crate::compose::ReportedOverride>,
}

/// Stage 4 + 5 for the GUI target: decide the presentation
/// choice per element of `composed` from the override pack
/// `data`, and emit the artefact and report.
///
/// # Errors
///
/// Returns every build error found (each located): an applied
/// override with no definition in the pack, a drawn override
/// naming an unregistered renderer (§2.2), an override with
/// incomplete state coverage (§2.3, missing states named), a
/// definition missing the field its kind requires, or a composed
/// document whose target is not `gui`.
pub fn lower_gui(
    doc: &Document,
    composed: &ComposedDocument,
    data: &GuiData,
) -> Result<(GuiArtefact, GuiReport), Vec<BuildError>> {
    if composed.target != "gui" {
        return Err(vec![BuildError::new(
            format!(
                "lower_gui: composed document target is {:?}, not \"gui\"; the web \
                 target lowers through lower_web (#5851), native targets through \
                 lower_native (#5852)",
                composed.target
            ),
            composed.root.span.line,
            composed.root.span.col,
        )]);
    }
    let mut source_by_path: BTreeMap<String, &Element> = BTreeMap::new();
    index_elements(&doc.root, "document", &mut source_by_path);

    let mut report = GuiReport {
        reported_overrides: composed.report.reported_overrides.clone(),
        ..GuiReport::default()
    };
    let mut errors: Vec<BuildError> = Vec::new();
    let root = lower_element(
        &composed.root,
        &source_by_path,
        data,
        &mut report,
        &mut errors,
    );
    if !errors.is_empty() {
        return Err(errors);
    }
    let root = root.expect("no errors means every element lowered");
    let a11y = a11y_tree(&composed.root, &source_by_path);
    Ok((
        GuiArtefact {
            target: composed.target.clone(),
            root,
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
    data: &GuiData,
    report: &mut GuiReport,
    errors: &mut Vec<BuildError>,
) -> Option<GuiElement> {
    if !source_by_path.contains_key(&el.path) {
        errors.push(BuildError::new(
            format!(
                "lower_gui: resolved element at {} has no counterpart in the \
                 validated document; the two inputs are not the same document",
                el.path
            ),
            el.span.line,
            el.span.col,
        ));
        return None;
    }

    let presentation = decide_presentation(el, data, report, errors)?;

    let mut children = Vec::new();
    for child in &el.children {
        children.push(match child {
            ComposedNode::Element(child_el) => GuiNode::Element(lower_element(
                child_el,
                source_by_path,
                data,
                report,
                errors,
            )?),
            ComposedNode::Text(text) => GuiNode::Text(text.clone()),
            ComposedNode::RawHtml(raw) => GuiNode::RawHtml(raw.content.clone()),
        });
    }

    Some(GuiElement {
        path: el.path.clone(),
        kind: el.name.clone(),
        role: vocabulary::lookup(&el.name)
            .map_or("", |spec| spec.role)
            .to_string(),
        presentation,
        children,
    })
}

/// Decides one element's presentation choice: standard, applied
/// override, or exercised fallback — recording errors, applied
/// overrides, and fallbacks as they are decided.
fn decide_presentation(
    el: &ComposedElement,
    data: &GuiData,
    report: &mut GuiReport,
    errors: &mut Vec<BuildError>,
) -> Option<GuiPresentation> {
    let Some(name) = &el.presentation else {
        return Some(GuiPresentation::Standard);
    };
    let Some(def) = data.definition(name) else {
        errors.push(BuildError::new(
            format!(
                "lower_gui: element at {} names presentation override {name:?}, but \
                 the gui override pack carries no definition for it; an override \
                 exists only as a pack entry (#5853 §2.1)",
                el.path
            ),
            el.span.line,
            el.span.col,
        ));
        return None;
    };

    // §2.3 state completeness, checked before any fallback
    // question: a partial override is a build error even if the
    // presentation would have fallen back anyway.
    let coverage = state_coverage(el, def, errors)?;

    match def.kind {
        PresentationKind::Asset => {
            let Some(asset) = &def.asset else {
                errors.push(BuildError::new(
                    format!(
                        "lower_gui: asset override {name:?} (element at {}) names no \
                         asset; an asset presentation is a referenced asset (#5853 §2.2)",
                        el.path
                    ),
                    el.span.line,
                    el.span.col,
                ));
                return None;
            };
            if !data.available_assets.iter().any(|a| a == asset) {
                let warning = format!(
                    "asset {asset:?} for override {name:?} is missing from the pack; \
                     standard rendering used for {} (#5853 §2.4)",
                    el.path
                );
                report.fallbacks.push(ExercisedFallback {
                    element_path: el.path.clone(),
                    name: name.clone(),
                    warning,
                });
                return Some(GuiPresentation::Fallback {
                    name: name.clone(),
                    reason: format!("asset {asset:?} missing from the pack"),
                });
            }
            report.applied.push(AppliedOverride {
                element_path: el.path.clone(),
                name: name.clone(),
                kind: "asset".to_string(),
            });
            Some(GuiPresentation::Asset {
                name: name.clone(),
                asset: asset.clone(),
                states: coverage,
            })
        }
        PresentationKind::Drawn => {
            let Some(renderer) = &def.renderer else {
                errors.push(BuildError::new(
                    format!(
                        "lower_gui: drawn override {name:?} (element at {}) names no \
                         renderer; a drawn presentation is a registered renderer \
                         (#5853 §2.2)",
                        el.path
                    ),
                    el.span.line,
                    el.span.col,
                ));
                return None;
            };
            if !data.registered_renderers.iter().any(|r| r == renderer) {
                errors.push(BuildError::new(
                    format!(
                        "lower_gui: drawn override {name:?} (element at {}) names \
                         renderer {renderer:?}, which the gui override pack has not \
                         registered; registration is the gate (#5853 §2.2)",
                        el.path
                    ),
                    el.span.line,
                    el.span.col,
                ));
                return None;
            }
            if data.failed_renderers.iter().any(|r| r == renderer) {
                let warning = format!(
                    "renderer {renderer:?} for override {name:?} failed to load; \
                     standard rendering used for {} (#5853 §2.4)",
                    el.path
                );
                report.fallbacks.push(ExercisedFallback {
                    element_path: el.path.clone(),
                    name: name.clone(),
                    warning,
                });
                return Some(GuiPresentation::Fallback {
                    name: name.clone(),
                    reason: format!("renderer {renderer:?} failed to load"),
                });
            }
            report.applied.push(AppliedOverride {
                element_path: el.path.clone(),
                name: name.clone(),
                kind: "drawn".to_string(),
            });
            Some(GuiPresentation::Drawn {
                name: name.clone(),
                renderer: renderer.clone(),
                states: coverage,
            })
        }
    }
}

/// The §2.3 completeness check for one applied override: the
/// union of explicit coverage and declared derivations must
/// equal the element kind's full state set. Returns the
/// (state, how-covered) list sorted by state, or records a
/// build error naming the missing states.
fn state_coverage(
    el: &ComposedElement,
    def: &GuiOverrideDef,
    errors: &mut Vec<BuildError>,
) -> Option<Vec<(String, String)>> {
    let required: BTreeSet<&str> = gui_table::state_set(&el.name).iter().copied().collect();
    let mut covered: BTreeMap<String, String> = BTreeMap::new();
    for state in &def.covered_states {
        covered.insert(state.clone(), "explicit".to_string());
    }
    for (state, derivation) in &def.derivations {
        covered
            .entry(state.clone())
            .or_insert_with(|| format!("derived: {derivation}"));
    }
    let missing: Vec<&str> = required
        .iter()
        .filter(|s| !covered.contains_key(**s))
        .copied()
        .collect();
    if !missing.is_empty() {
        errors.push(BuildError::new(
            format!(
                "lower_gui: presentation override {:?} on {} ({}) does not cover \
                 every interaction state; missing: {} (#5853 §2.3: a partial \
                 override is a build-time error)",
                def.name,
                el.path,
                el.name,
                missing.join(", ")
            ),
            el.span.line,
            el.span.col,
        ));
        return None;
    }
    Some(
        required
            .iter()
            .map(|s| {
                (
                    (*s).to_string(),
                    covered.get(*s).cloned().unwrap_or_default(),
                )
            })
            .collect(),
    )
}

/// Builds the accessibility tree from the composed (semantic)
/// document alone — overrides are never consulted, which is what
/// makes the tree identical in every mode (§3 invariant 1).
fn a11y_tree(el: &ComposedElement, source_by_path: &BTreeMap<String, &Element>) -> A11yNode {
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
                if let ComposedNode::Text(text) = child {
                    texts.push(text);
                }
            }
            texts.join(" ")
        });
    let mut children = Vec::new();
    for child in &el.children {
        if let ComposedNode::Element(child_el) = child {
            children.push(a11y_tree(child_el, source_by_path));
        }
    }
    A11yNode {
        path: el.path.clone(),
        kind: el.name.clone(),
        role: vocabulary::lookup(&el.name)
            .map_or("", |spec| spec.role)
            .to_string(),
        name,
        children,
    }
}

/// Stage 5 emit: the artefact and report as one deterministic
/// JSON document (sorted keys in fixed schema order, no
/// timestamps — #5855 invariant 2). This is the machine-readable
/// report of #5855 §2.1 stage 5 in its GUI form.
#[must_use]
pub fn emit_gui_json(artefact: &GuiArtefact, report: &GuiReport) -> String {
    let mut out = String::new();
    out.push_str("{\n  \"target\": ");
    push_json_string(&mut out, &artefact.target);
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

fn push_presentation_json(out: &mut String, presentation: &GuiPresentation, depth: usize) {
    out.push_str("{\n");
    let field = |out: &mut String, name: &str| {
        indent(out, depth + 1);
        push_json_string(out, name);
        out.push_str(": ");
    };
    match presentation {
        GuiPresentation::Standard => {
            field(out, "choice");
            push_json_string(out, "standard");
        }
        GuiPresentation::Asset {
            name,
            asset,
            states,
        } => {
            field(out, "choice");
            push_json_string(out, "asset");
            out.push_str(",\n");
            field(out, "name");
            push_json_string(out, name);
            out.push_str(",\n");
            field(out, "asset");
            push_json_string(out, asset);
            out.push_str(",\n");
            field(out, "states");
            push_states_json(out, states, depth + 1);
            out.push_str(",\n");
            field(out, "fallback");
            push_json_string(out, "standard");
        }
        GuiPresentation::Drawn {
            name,
            renderer,
            states,
        } => {
            field(out, "choice");
            push_json_string(out, "drawn");
            out.push_str(",\n");
            field(out, "name");
            push_json_string(out, name);
            out.push_str(",\n");
            field(out, "renderer");
            push_json_string(out, renderer);
            out.push_str(",\n");
            field(out, "states");
            push_states_json(out, states, depth + 1);
            out.push_str(",\n");
            field(out, "fallback");
            push_json_string(out, "standard");
        }
        GuiPresentation::Fallback { name, reason } => {
            field(out, "choice");
            push_json_string(out, "fallback");
            out.push_str(",\n");
            field(out, "name");
            push_json_string(out, name);
            out.push_str(",\n");
            field(out, "reason");
            push_json_string(out, reason);
            out.push_str(",\n");
            field(out, "rendering");
            push_json_string(out, "standard");
        }
    }
    out.push('\n');
    indent(out, depth);
    out.push('}');
}

fn push_states_json(out: &mut String, states: &[(String, String)], depth: usize) {
    if states.is_empty() {
        out.push_str("[]");
        return;
    }
    out.push_str("[\n");
    for (i, (state, how)) in states.iter().enumerate() {
        indent(out, depth + 1);
        out.push_str("{\"state\": ");
        push_json_string(out, state);
        out.push_str(", \"coverage\": ");
        push_json_string(out, how);
        out.push('}');
        if i + 1 < states.len() {
            out.push(',');
        }
        out.push('\n');
    }
    indent(out, depth);
    out.push(']');
}

fn push_element_json(out: &mut String, el: &GuiElement, depth: usize) {
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
    field(out, "role");
    push_json_string(out, &el.role);
    out.push_str(",\n");
    field(out, "presentation");
    push_presentation_json(out, &el.presentation, depth + 1);
    out.push_str(",\n");
    field(out, "children");
    if el.children.is_empty() {
        out.push_str("[]");
    } else {
        out.push_str("[\n");
        for (i, child) in el.children.iter().enumerate() {
            indent(out, depth + 2);
            match child {
                GuiNode::Element(child_el) => push_element_json(out, child_el, depth + 2),
                GuiNode::Text(text) => {
                    out.push_str("{\"text\": ");
                    push_json_string(out, text);
                    out.push('}');
                }
                GuiNode::RawHtml(raw) => {
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

fn push_report_json(out: &mut String, report: &GuiReport, depth: usize) {
    out.push_str("{\n");
    let field = |out: &mut String, name: &str| {
        indent(out, depth + 1);
        push_json_string(out, name);
        out.push_str(": ");
    };
    field(out, "applied");
    out.push('[');
    for (i, a) in report.applied.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str("{\"element\": ");
        push_json_string(out, &a.element_path);
        out.push_str(", \"name\": ");
        push_json_string(out, &a.name);
        out.push_str(", \"kind\": ");
        push_json_string(out, &a.kind);
        out.push('}');
    }
    out.push(']');
    out.push_str(",\n");
    field(out, "fallbacks");
    out.push('[');
    for (i, f) in report.fallbacks.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str("{\"element\": ");
        push_json_string(out, &f.element_path);
        out.push_str(", \"name\": ");
        push_json_string(out, &f.name);
        out.push_str(", \"warning\": ");
        push_json_string(out, &f.warning);
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
