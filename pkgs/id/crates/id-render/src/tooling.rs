//! Author tooling for the rendering pipeline — the one toolchain
//! surface decision record #5855 §4 settles and issue #7920 owns:
//! a linter over the semantic source, and the multi-target
//! preview. Both consume exactly the vocabularies and tables the
//! pipeline consumes; neither is a second implementation of the
//! rendering rules. The preview is the pipeline run in report
//! mode, and the linter's per-target rules read the pipeline's
//! own coverage reports.
//!
//! Lint rules shipped here, each expressed over pipeline inputs
//! only (#5855 §4):
//!
//! * `unknown-element` (error): an element kind outside the #5849
//!   vocabulary. Parsing already rejects unknown kinds, so over
//!   source this rule reports the parse failure; over the IR the
//!   kind is checked directly against [`crate::vocabulary`].
//! * `unknown-assertion` (error): an `assert` slot the #5850
//!   assertion vocabulary rejects — an unknown dimension or role
//!   token, a raw value, or a dimension asserted twice in one
//!   slot. Checked with
//!   [`crate::assertion_vocabulary::parse_assert_slot`], the same
//!   function stage 2 uses.
//! * `unmapped-assertion` (warning, per target): an assertion a
//!   target's mapping table has no row for, so it defaults there
//!   (#5855 §3.3: "defaulted" is a warning class, never silent).
//!   Read from the lowering report of a real pipeline run for
//!   that target. The custom-GUI target accounts assertions by
//!   disposition instead of by table row, so this rule does not
//!   fire there.
//! * `raw-css-override` (warning, web target): the listing of
//!   raw-CSS declarations that overrode an assertion-derived
//!   property (#5854 §6b reported overrides), carried from the
//!   composition report through the web lowering report.
//! * `duplicate-source` (error, multi-source): the same semantic
//!   document appears in more than one target folder. Source spans
//!   and attribute order are ignored; child order and raw content
//!   remain significant.
//!
//! The remaining #5855 §4 error rule — essential content inside
//! a conditional block — does not ship here, by §4's own gate: a
//! lint rule that cannot be expressed over the pipeline's inputs
//! does not ship. The #5849 v1 vocabulary has no conditional
//! element kind, so a conditional block has no source
//! representation to lint (the same stated boundary as stage 3;
//! see [`crate::compose`]). The rule lands with that vocabulary
//! addition, not before it.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use crate::assertion_vocabulary;
use crate::compose::{ComposeData, CompositionWarning, ReportedOverride, TargetSpec};
use crate::lower::{LowerReport, WebArtefact};
use crate::lower_gui::{GuiArtefact, GuiData, GuiReport};
use crate::lower_native::{NativeArtefact, NativeReport};
use crate::{BuildError, Document, Element, Node};

/// Lint rule name: element kind outside the #5849 vocabulary.
pub const RULE_UNKNOWN_ELEMENT: &str = "unknown-element";
/// Lint rule name: `assert` slot outside the #5850 vocabulary.
pub const RULE_UNKNOWN_ASSERTION: &str = "unknown-assertion";
/// Lint rule name: assertion with no mapping on a target.
pub const RULE_UNMAPPED_ASSERTION: &str = "unmapped-assertion";
/// Lint rule name: raw CSS overriding an assertion-derived
/// property on the web target.
pub const RULE_RAW_CSS_OVERRIDE: &str = "raw-css-override";
/// Lint rule name: the source does not parse.
pub const RULE_PARSE: &str = "parse";
/// Lint rule name: a stage-2 validation failure outside the
/// named lint rules (the linter never runs the pipeline on a
/// document that does not validate).
pub const RULE_VALIDATION: &str = "validation";
/// Lint rule name: composition failed for a target (a
/// same-specificity tie is the settled failure mode, #5854 §6a).
pub const RULE_COMPOSITION_CONFLICT: &str = "composition-conflict";
/// Lint rule name: lowering failed for a target.
pub const RULE_LOWERING: &str = "lowering";

/// Rule for the same semantic source found in two target folders
/// (#5855 §4).
pub const RULE_DUPLICATE_SOURCE: &str = "duplicate-source";

/// How serious a lint diagnostic is (#5855 §4: the lint rules
/// split into errors and warnings; nothing else exists).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// The author must fix this; the document does not build
    /// clean.
    Error,
    /// The document builds; the author decides whether the
    /// reported behaviour is acceptable.
    Warning,
}

/// One lint finding: a rule, a severity, a message, and the
/// source location when the finding is tied to one construct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// Error or warning (#5855 §4).
    pub severity: Severity,
    /// The lint rule that produced this diagnostic (one of the
    /// `RULE_*` constants).
    pub rule: &'static str,
    /// Human-readable description of the finding.
    pub message: String,
    /// 1-based line of the offending construct, when located.
    pub line: usize,
    /// 1-based column of the offending construct, when located.
    pub col: usize,
    /// The target the finding belongs to, for per-target rules.
    pub target: Option<String>,
    /// The document path of the element the finding belongs to,
    /// when it is tied to one element.
    pub element_path: Option<String>,
}

/// The outcome of one [`lint`] run: every diagnostic found, in
/// the order the rules produced them (source order for the
/// source rules, plan order for the per-target rules).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LintReport {
    /// All diagnostics, errors and warnings together.
    pub diagnostics: Vec<Diagnostic>,
}

impl LintReport {
    /// The error diagnostics only.
    pub fn errors(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
    }

    /// The warning diagnostics only.
    pub fn warnings(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Warning)
    }

    /// Whether the run found any error.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
    }
}

/// One target's inputs to a lint or preview run: the target
/// facts composition may consult, the per-target composition
/// data, and (for the custom-GUI target) the override pack.
#[derive(Debug, Clone)]
pub struct TargetPlan {
    /// Target facts (#5854 §5).
    pub spec: TargetSpec,
    /// Composition data for this target only (#5854 §2.4).
    pub compose: ComposeData,
    /// The custom-GUI override pack; consumed only when the
    /// target is `gui`.
    pub gui: GuiData,
}

impl TargetPlan {
    /// A plan for one target with default (empty) data inputs.
    #[must_use]
    pub fn new(name: &str) -> Self {
        Self {
            spec: TargetSpec {
                name: name.to_string(),
                form_factor: "desktop".to_string(),
                capabilities: BTreeSet::new(),
            },
            compose: ComposeData::default(),
            gui: GuiData::default(),
        }
    }
}

/// The standard multi-target plan set: the web target, the
/// three native platforms of the #5852 tables, and the
/// custom-GUI target — each with default data inputs. This is
/// the plan set the one-command preview renders by default.
#[must_use]
pub fn standard_plans() -> Vec<TargetPlan> {
    let mut plans = vec![TargetPlan::new("web")];
    for platform in crate::native_table::PLATFORMS {
        plans.push(TargetPlan::new(platform));
    }
    plans.push(TargetPlan::new("gui"));
    plans
}

/// One artefact of a preview run, tagged by the target family
/// that lowered it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Artefact {
    /// The web artefact (#5851).
    Web(WebArtefact),
    /// A native-platform artefact (#5852).
    Native(NativeArtefact),
    /// The custom-GUI artefact (#5853).
    Gui(GuiArtefact),
}

/// One assertion a target defaulted, unified across the web and
/// native coverage reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefaultedEntry {
    /// Element the assertion applies to.
    pub element_path: String,
    /// Assertion dimension.
    pub property: String,
    /// Role token that had no mapping.
    pub token: String,
    /// The coverage-warning text the lowering report carries.
    pub message: String,
}

/// One element kind a target defaulted, unified across the web
/// and native coverage reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElementDefaultEntry {
    /// Element the default applies to.
    pub element_path: String,
    /// Element kind with no table row.
    pub kind: String,
    /// The default that was used (tag or control).
    pub default: String,
}

/// The coverage half of the coverage/conflict report for one
/// target (#5855 §3.3), unified across the three lowering
/// reports: assertions mapped, assertions defaulted (never
/// silent), element defaults, and reported raw-CSS overrides.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Coverage {
    /// Assertions with a mapping on this target.
    pub mapped: usize,
    /// Assertions defaulted for lack of a mapping.
    pub defaulted: Vec<DefaultedEntry>,
    /// Element kinds defaulted for lack of a table row.
    pub element_defaults: Vec<ElementDefaultEntry>,
    /// Raw-CSS reported overrides (#5854 §6b).
    pub reported_overrides: Vec<ReportedOverride>,
}

impl Coverage {
    fn from_web(report: &LowerReport) -> Self {
        Self {
            mapped: report.mapped.len(),
            defaulted: report
                .defaulted
                .iter()
                .map(|d| DefaultedEntry {
                    element_path: d.element_path.clone(),
                    property: d.property.clone(),
                    token: d.token.clone(),
                    message: d.warning.clone(),
                })
                .collect(),
            element_defaults: report
                .element_defaults
                .iter()
                .map(|d| ElementDefaultEntry {
                    element_path: d.element_path.clone(),
                    kind: d.kind.clone(),
                    default: d.default_tag.clone(),
                })
                .collect(),
            reported_overrides: report.reported_overrides.clone(),
        }
    }

    fn from_native(report: &NativeReport) -> Self {
        Self {
            mapped: report.mapped.len(),
            defaulted: report
                .defaulted
                .iter()
                .map(|d| DefaultedEntry {
                    element_path: d.element_path.clone(),
                    property: d.property.clone(),
                    token: d.token.clone(),
                    message: d.warning.clone(),
                })
                .collect(),
            element_defaults: report
                .element_defaults
                .iter()
                .map(|d| ElementDefaultEntry {
                    element_path: d.element_path.clone(),
                    kind: d.kind.clone(),
                    default: d.default_control.clone(),
                })
                .collect(),
            reported_overrides: report.reported_overrides.clone(),
        }
    }

    fn from_gui(report: &GuiReport) -> Self {
        // The GUI report accounts every resolved assertion by
        // disposition; there is no table-row defaulting to
        // report, so every accounted assertion counts as mapped
        // and the defaulted list is empty by construction.
        Self {
            mapped: report.assertions.len(),
            defaulted: Vec::new(),
            element_defaults: Vec::new(),
            reported_overrides: report.reported_overrides.clone(),
        }
    }
}

/// One target's outcome in a preview: either its artefact plus
/// coverage, or the conflict that kept it from rendering (the
/// conflict half of the coverage/conflict report — a failed
/// target never fails the whole preview).
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(
    clippy::large_enum_variant,
    reason = "a preview outcome is a report value consumed by the author, not a hot path; the artefact is carried whole by design"
)]
pub enum TargetOutcome {
    /// The target rendered.
    Rendered {
        /// Target name.
        target: String,
        /// The target's artefact.
        artefact: Artefact,
        /// The target's coverage.
        coverage: Coverage,
        /// Composition warnings for this target (#5854 §6).
        composition_warnings: Vec<CompositionWarning>,
    },
    /// The target did not render; these errors are its conflict
    /// record (composition ties, lowering failures).
    Conflict {
        /// Target name.
        target: String,
        /// The errors that kept the target from rendering.
        errors: Vec<BuildError>,
    },
}

impl TargetOutcome {
    /// The target this outcome belongs to.
    #[must_use]
    pub fn target(&self) -> &str {
        match self {
            Self::Rendered { target, .. } | Self::Conflict { target, .. } => target,
        }
    }
}

/// The multi-target preview: every planned target's outcome,
/// artefacts presented together with the coverage/conflict
/// report (#5855 §4).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Preview {
    /// One outcome per planned target, in plan order.
    pub targets: Vec<TargetOutcome>,
}

impl Preview {
    /// The outcome for one target, by name.
    #[must_use]
    pub fn outcome(&self, target: &str) -> Option<&TargetOutcome> {
        self.targets.iter().find(|o| o.target() == target)
    }
}

/// Which pipeline stage a target run failed at.
enum Stage {
    Compose,
    Lower,
}

/// One target's successful pipeline run: its artefact, its
/// coverage, and its composition warnings.
struct TargetRun {
    artefact: Artefact,
    coverage: Coverage,
    composition_warnings: Vec<CompositionWarning>,
}

/// Runs stages 3–5 for one target plan against a validated
/// document: the single pipeline run both tooling surfaces use.
fn run_target(doc: &Document, plan: &TargetPlan) -> Result<TargetRun, (Stage, Vec<BuildError>)> {
    let composed = crate::compose_document(doc, &plan.spec, &plan.compose)
        .map_err(|errors| (Stage::Compose, errors))?;
    let warnings = composed.report.warnings.clone();
    let name = plan.spec.name.as_str();
    if name == "web" {
        let (artefact, report) =
            crate::lower_document(doc, &composed).map_err(|errors| (Stage::Lower, errors))?;
        Ok(TargetRun {
            artefact: Artefact::Web(artefact),
            coverage: Coverage::from_web(&report),
            composition_warnings: warnings,
        })
    } else if crate::native_table::is_platform(name) {
        let (artefact, report) = crate::lower_document_native(doc, &composed)
            .map_err(|errors| (Stage::Lower, errors))?;
        Ok(TargetRun {
            artefact: Artefact::Native(artefact),
            coverage: Coverage::from_native(&report),
            composition_warnings: warnings,
        })
    } else if name == "gui" {
        let (artefact, report) = crate::lower_document_gui(doc, &composed, &plan.gui)
            .map_err(|errors| (Stage::Lower, errors))?;
        Ok(TargetRun {
            artefact: Artefact::Gui(artefact),
            coverage: Coverage::from_gui(&report),
            composition_warnings: warnings,
        })
    } else {
        Err((
            Stage::Lower,
            vec![BuildError::new(
                format!(
                    "unknown target {name:?}: not the web target, a native \
                     platform, or the custom-GUI target"
                ),
                1,
                1,
            )],
        ))
    }
}

/// The multi-target preview (#5855 §4): builds every planned
/// target's artefact from the source and presents them together
/// with the coverage/conflict report. This is the pipeline run
/// in report mode — a target whose composition conflicts is
/// reported as a conflict outcome, never a failed preview.
///
/// # Errors
///
/// Returns the parse/validation errors when the source does not
/// build at all; per-target failures are [`TargetOutcome::Conflict`]
/// values inside the preview, not errors of the preview itself.
pub fn preview(source: &str, plans: &[TargetPlan]) -> Result<Preview, Vec<BuildError>> {
    let doc = crate::parse_and_validate(source)?;
    Ok(preview_document(&doc, plans))
}

/// The preview over an already-validated document.
#[must_use]
pub fn preview_document(doc: &Document, plans: &[TargetPlan]) -> Preview {
    Preview {
        targets: plans
            .iter()
            .map(|plan| match run_target(doc, plan) {
                Ok(run) => TargetOutcome::Rendered {
                    target: plan.spec.name.clone(),
                    artefact: run.artefact,
                    coverage: run.coverage,
                    composition_warnings: run.composition_warnings,
                },
                Err((_, errors)) => TargetOutcome::Conflict {
                    target: plan.spec.name.clone(),
                    errors,
                },
            })
            .collect(),
    }
}

/// The source-level lint walks: element kinds against the #5849
/// vocabulary, `assert` slots against the #5850 vocabulary. Both
/// consume the same data stage 2 consumes.
fn lint_walk(el: &Element, path: &str, diagnostics: &mut Vec<Diagnostic>) {
    if crate::vocabulary::lookup(&el.name).is_none() {
        diagnostics.push(Diagnostic {
            severity: Severity::Error,
            rule: RULE_UNKNOWN_ELEMENT,
            message: format!("unknown element <{}>", el.name),
            line: el.span.line,
            col: el.span.col,
            target: None,
            element_path: Some(path.to_string()),
        });
    }
    if let Some(value) = el.attr("assert")
        && let Err(problems) = assertion_vocabulary::parse_assert_slot(value)
    {
        for problem in problems {
            diagnostics.push(Diagnostic {
                severity: Severity::Error,
                rule: RULE_UNKNOWN_ASSERTION,
                message: problem,
                line: el.span.line,
                col: el.span.col,
                target: None,
                element_path: Some(path.to_string()),
            });
        }
    }
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for child in &el.children {
        if let Node::Element(child_el) = child {
            let index = {
                let entry = counts.entry(child_el.name.as_str()).or_insert(0);
                let index = *entry;
                *entry += 1;
                index
            };
            let child_path = crate::compose::element_path(path, &child_el.name, index);
            lint_walk(child_el, &child_path, diagnostics);
        }
    }
}

/// Lints one semantic source document against the planned
/// targets (#5855 §4, issue #7920).
///
/// The source rules run first; when any of them (or stage 2)
/// reports an error, the per-target rules do not run — the
/// pipeline never runs on a document that does not build. When
/// the source is clean, each planned target is run through the
/// pipeline and its coverage report is read for the warning
/// rules; a target that fails to compose or lower contributes
/// its errors as diagnostics instead of stopping the lint.
#[must_use]
pub fn lint(source: &str, plans: &[TargetPlan]) -> LintReport {
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let doc = match crate::parse(source) {
        Ok(doc) => doc,
        Err(error) => {
            diagnostics.push(Diagnostic {
                severity: Severity::Error,
                rule: if error.message.starts_with("unknown element") {
                    RULE_UNKNOWN_ELEMENT
                } else {
                    RULE_PARSE
                },
                message: error.message.clone(),
                line: error.line,
                col: error.col,
                target: None,
                element_path: None,
            });
            return LintReport { diagnostics };
        }
    };
    lint_walk(&doc.root, "document", &mut diagnostics);
    if diagnostics.iter().any(|d| d.severity == Severity::Error) {
        return LintReport { diagnostics };
    }
    if let Err(errors) = crate::validate(&doc) {
        for error in errors {
            diagnostics.push(Diagnostic {
                severity: Severity::Error,
                rule: RULE_VALIDATION,
                message: error.message.clone(),
                line: error.line,
                col: error.col,
                target: None,
                element_path: None,
            });
        }
        return LintReport { diagnostics };
    }
    for plan in plans {
        let target = plan.spec.name.clone();
        match run_target(&doc, plan) {
            Ok(run) => {
                for defaulted in &run.coverage.defaulted {
                    diagnostics.push(Diagnostic {
                        severity: Severity::Warning,
                        rule: RULE_UNMAPPED_ASSERTION,
                        message: defaulted.message.clone(),
                        line: 0,
                        col: 0,
                        target: Some(target.clone()),
                        element_path: Some(defaulted.element_path.clone()),
                    });
                }
                if target == "web" {
                    for reported in &run.coverage.reported_overrides {
                        diagnostics.push(Diagnostic {
                            severity: Severity::Warning,
                            rule: RULE_RAW_CSS_OVERRIDE,
                            message: format!(
                                "raw CSS {}: {} overrides the assertion-derived \
                                 value on {} (reported override, #5854 §6b)",
                                reported.selector, reported.property, reported.element_path
                            ),
                            line: 0,
                            col: 0,
                            target: Some(target.clone()),
                            element_path: Some(reported.element_path.clone()),
                        });
                    }
                }
            }
            Err((stage, errors)) => {
                let rule = match stage {
                    Stage::Compose => RULE_COMPOSITION_CONFLICT,
                    Stage::Lower => RULE_LOWERING,
                };
                for error in errors {
                    diagnostics.push(Diagnostic {
                        severity: Severity::Error,
                        rule,
                        message: error.message.clone(),
                        line: error.line,
                        col: error.col,
                        target: Some(target.clone()),
                        element_path: None,
                    });
                }
            }
        }
    }
    LintReport { diagnostics }
}

/// Compare parsed documents by semantic content. Source positions are
/// diagnostics metadata, not part of the source's meaning. Element
/// attributes are key/value properties, so their order is ignored while
/// duplicate pairs are retained; child order and raw-region content remain
/// significant.
fn same_semantic_document(left: &Document, right: &Document) -> bool {
    same_semantic_element(&left.root, &right.root)
}

fn same_semantic_element(left: &Element, right: &Element) -> bool {
    if left.name != right.name || !same_semantic_attributes(&left.attrs, &right.attrs) {
        return false;
    }
    left.children.len() == right.children.len()
        && left
            .children
            .iter()
            .zip(&right.children)
            .all(|(left, right)| same_semantic_node(left, right))
}

fn same_semantic_attributes(
    left: &[(String, Option<String>)],
    right: &[(String, Option<String>)],
) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut left = left.to_vec();
    let mut right = right.to_vec();
    left.sort();
    right.sort();
    left == right
}

fn same_semantic_node(left: &Node, right: &Node) -> bool {
    match (left, right) {
        (Node::Element(left), Node::Element(right)) => same_semantic_element(left, right),
        (Node::Text(left), Node::Text(right)) => left == right,
        (Node::Raw(left), Node::Raw(right)) => {
            left.kind == right.kind && left.content == right.content
        }
        _ => false,
    }
}

/// Lint several target folders' sources together. Each source is
/// linted on its own with [`lint`], and every pair of sources whose
/// parsed documents are semantically equal is reported as a
/// `duplicate-source` error on the later folder (#5855 §4). Source
/// diagnostics retain their folder in the message; target diagnostics keep
/// their separate `target` field. Sources that fail to parse are not
/// compared; their own lint already reports the failure.
#[must_use]
pub fn lint_folders(sources: &[(&str, &str)], plans: &[TargetPlan]) -> LintReport {
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut parsed: Vec<(&str, Document)> = Vec::new();
    for (folder, source) in sources {
        let mut report = lint(source, plans);
        for diagnostic in &mut report.diagnostics {
            diagnostic.message = format!("{folder}: {}", diagnostic.message);
        }
        diagnostics.extend(report.diagnostics);
        if let Ok(doc) = crate::parse(source) {
            parsed.push((folder, doc));
        }
    }
    for (index, (folder, doc)) in parsed.iter().enumerate() {
        if let Some((first, _)) = parsed[..index]
            .iter()
            .find(|(_, other)| same_semantic_document(other, doc))
        {
            diagnostics.push(Diagnostic {
                severity: Severity::Error,
                rule: RULE_DUPLICATE_SOURCE,
                message: format!("semantic source in {folder} duplicates the one in {first}"),
                line: 0,
                col: 0,
                target: None,
                element_path: None,
            });
        }
    }
    LintReport { diagnostics }
}
