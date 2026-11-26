//! Rendering pipeline compiler core for the id programme (issue #7918).
//!
//! The pipeline shape is settled by decision record #5855
//! (`doc/2026-11-08_decision_id5855_rendering_pipeline_conformance.md`):
//! five stages over one target-neutral IR — parse, validate, compose,
//! lower per target, emit artefact + report — with the mapping tables of
//! #5851/#5852/#5853 consumed as data inputs to lowering, never as code
//! branches in the pipeline.
//!
//! This crate implements all five stages:
//!
//! * **Stage 1 — parse** ([`parse`]): semantic source in the HTML-shaped
//!   authoring syntax of decision record #5849 §2 parses to the IR
//!   ([`Document`]). Parse errors are fatal, located, and never recovered
//!   by guessing (#5855 §2.1).
//! * **Stage 2 — validate** ([`validate`]): the IR is
//!   checked against the #5849 element vocabulary (consumed as data from
//!   [`vocabulary`]): unknown element kinds, unknown attributes, missing
//!   required attributes, enum-valued attribute domains, the child and
//!   sibling rules of #5849 §4.5, and the names-not-expressions rule for
//!   binding slots (#5849 §8) are build errors with source locations.
//!   The assertion half of the stage checks the `assert` slot against
//!   the #5850 assertion vocabulary (consumed as versioned data from
//!   [`assertion_vocabulary`]): an unknown dimension or role token
//!   fails with suggestions (the #5849 idiom), a raw value is rejected
//!   (#5850 invariant 1), and a dimension asserted twice in one slot
//!   is an error (#5854 §2.2).
//!
//! * **Stage 3 — compose** ([`compose`]): the #5854 cascade over
//!   the source `assert` slots (layer-3 assertions at instance
//!   specificity) and data-supplied composition tables (theme
//!   layering, further assertion and override resolution, the
//!   web-only raw-CSS layer), producing the resolved IR and
//!   composition report for one target. Conditional
//!   blocks (`<conditional>`, #5854 §5) are resolved for the
//!   target before the cascade; see the module documentation.
//!
//! * **Stages 4–5 — lower + emit, web target** ([`lower`]): the
//!   resolved IR is lowered through the #5851 mapping table
//!   ([`web_table`], consumed as data) to the web artefact — DOM
//!   tags per #5851 §2, utilities per §3 — and emitted with the
//!   machine-readable coverage report: assertions mapped,
//!   assertions defaulted (never silent, #5855 §3.3), element
//!   defaults, and reported raw-CSS overrides.
//!
//! * **Stages 4–5 — lower + emit, native targets**
//!   ([`lower_native`]): the resolved IR is lowered through the
//!   #5852 mapping tables ([`native_table`], consumed as data) to
//!   the native artefact for one platform (mac/windows/gnome) —
//!   the OS-standard control per element kind (#5852 §2), the
//!   control-kind sequence and accessibility tree of #5855 §3.2 —
//!   and emitted with the machine-readable coverage report:
//!   assertions mapped, assertions platform-defaulted (never
//!   silent, #5852 §3.1), element defaults, and presentation
//!   overrides excluded per #5853.
//!
//! * **Stages 4–5 — lower + emit, custom-GUI target**
//!   ([`lower_gui`]): the resolved IR is lowered through the
//!   #5853 presentation-override contract — override definitions
//!   consumed as target-side data ([`lower_gui::GuiData`]) — to
//!   the GUI artefact: the presentation choice per node (standard,
//!   asset, drawn, or the exercised standard fallback of §2.4)
//!   plus the accessibility tree, which is identical in every
//!   mode (#5853 §3 invariant 1). State completeness (§2.3) and
//!   renderer registration (§2.2) are build-time errors, never
//!   runtime gaps.
//!
//! The assertion half of stage 2 was the one deferral #5855
//! assumption A2 prescribed: the `assert` attribute's grammar and
//! role vocabulary are #5850's, and the slot was carried opaquely
//! until that record settled. #5850 is settled (its decision
//! record is the vocabulary v1 on the closed issue), so the
//! deferral is discharged: [`assertion_vocabulary`] consumes the
//! record as versioned data, stage 2 validates the slot against
//! it, and stage 3 composes the slot's assertions like any other
//! layer-3 input. What remains outside the pipeline is unchanged
//! and belongs to its own records: state-delta composition
//! (#5851 §3.6 — targets render state behaviour natively).
//! Conditional blocks, which #5849 v1 could not represent, are the
//! `<conditional>` element of [`vocabulary::CONDITIONAL`] (issue
//! #7920): validated in stage 2, resolved per target in stage 3.

//! * **Author tooling** ([`tooling`]): the one toolchain
//!   surface #5855 §4 settles — a linter over the source plus
//!   the vocabularies and tables the pipeline consumes, and
//!   the multi-target preview: the pipeline run in report
//!   mode, presenting every target's artefact together with
//!   the coverage/conflict report, including the essential-
//!   content rule for conditional blocks; see the module
//!   documentation.
//!
pub mod assertion_vocabulary;
pub mod compose;
pub mod gui_table;
pub mod lower;
pub mod lower_gui;
pub mod lower_native;
pub mod native_table;
mod parse;
pub mod tooling;
mod validate;
pub mod vocabulary;
pub mod web_table;

use std::fmt;

/// A located build error. Parse errors are fatal (parsing stops at the
/// first one, per #5855 §2.1 stage 1); validation collects every error
/// it can before failing the build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildError {
    /// Human-readable description of the violated rule.
    pub message: String,
    /// 1-based line of the offending construct.
    pub line: usize,
    /// 1-based column of the offending construct.
    pub col: usize,
}

impl BuildError {
    /// Creates an error at the given 1-based position.
    #[must_use]
    pub fn new(message: impl Into<String>, line: usize, col: usize) -> Self {
        Self {
            message: message.into(),
            line,
            col,
        }
    }
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.col, self.message)
    }
}

impl std::error::Error for BuildError {}

/// A source span: the 1-based line/column where a node starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    /// 1-based line.
    pub line: usize,
    /// 1-based column.
    pub col: usize,
}

/// One node of the target-neutral IR (#5855 §2.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    /// A registry element with attributes and children.
    Element(Element),
    /// Text content. Whitespace runs are collapsed and the ends trimmed
    /// at parse time (#5849 §2.1: whitespace inside `text` is collapsed;
    /// whitespace between elements is insignificant).
    Text(String),
    /// A `<raw>` region (#5849 §3.1): authored source passed through
    /// verbatim, never populated from a bound value.
    Raw(RawRegion),
}

/// A registry element: a kind from the #5849 vocabulary, its attributes
/// in source order, and its children.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    /// Element kind, e.g. `page` or `field`.
    pub name: String,
    /// Attributes in source order; a bare boolean attribute (e.g.
    /// `required`) has a `None` value.
    pub attrs: Vec<(String, Option<String>)>,
    /// Child nodes in source order (source order is reading order and
    /// focus order, #5849 §2.3/§6).
    pub children: Vec<Node>,
    /// Where the element's open tag starts.
    pub span: Span,
}

impl Element {
    /// Returns the value of the named attribute, or `None` when the
    /// attribute is absent or present as a bare boolean.
    #[must_use]
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(n, _)| n == name)
            .and_then(|(_, v)| v.as_deref())
    }

    /// Returns whether the named attribute is present at all (valued or
    /// bare boolean).
    #[must_use]
    pub fn has_attr(&self, name: &str) -> bool {
        self.attrs.iter().any(|(n, _)| n == name)
    }
}

/// A `<raw kind="html">` or `<raw kind="css">` region (#5849 §3.1). The
/// content is verbatim source between the open and close tags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawRegion {
    /// The region kind: `html` or `css`.
    pub kind: String,
    /// The verbatim content.
    pub content: String,
    /// Where the `<raw>` open tag starts.
    pub span: Span,
}

/// A parsed semantic document: the `<document>` root element. The root
/// is the only node [`parse`] returns; [`validate`] checks it against
/// the vocabulary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    /// The root `<document>` element.
    pub root: Element,
}

impl Document {
    /// The declared vocabulary version (`version` attribute of the
    /// root). v1 parsers accept only `"1"` (#5849 §2.4); the check
    /// itself lives in [`validate`].
    #[must_use]
    pub fn version(&self) -> Option<&str> {
        self.root.attr("version")
    }
}

/// Stage 1 of the #5855 pipeline: parse semantic source to the IR.
///
/// # Errors
///
/// Returns the first fatal parse error (located). Unknown elements are
/// parse errors, never silently treated as raw HTML (#5849 §3.2).
pub fn parse(source: &str) -> Result<Document, BuildError> {
    parse::Parser::new(source).parse_document()
}

/// Stage 2: validate a parsed document against the vocabularies —
/// the #5849 element registry, attribute registry, enum domains,
/// child/sibling rules, and binding-slot name rules, plus the
/// `assert` slot against the #5850 assertion vocabulary.
///
/// # Errors
///
/// Returns every validation error found (each located), or an empty
/// vector wrapped in `Ok` semantics by returning `Ok(())` — see the
/// signature: `Err` carries the full list.
pub fn validate(doc: &Document) -> Result<(), Vec<BuildError>> {
    validate::validate(doc)
}

/// Stage 3: compose a validated document for one target.
/// See [`compose::compose`] for the cascade and its data inputs.
///
/// # Errors
///
/// Returns every same-specificity tie found (each located).
pub fn compose_document(
    doc: &Document,
    target: &compose::TargetSpec,
    data: &compose::ComposeData,
) -> Result<compose::ComposedDocument, Vec<BuildError>> {
    compose::compose(doc, target, data)
}

/// Stages 4 + 5 (web target): lower a composed document through
/// the #5851 mapping table and emit the artefact plus coverage
/// report. See [`lower::lower_web`] for the stated boundaries
/// (attributes are read from the validated document, matched by
/// document path).
///
/// # Errors
///
/// Returns a build error for a non-web composed document or a
/// path mismatch between the two inputs.
pub fn lower_document(
    doc: &Document,
    composed: &compose::ComposedDocument,
) -> Result<(lower::WebArtefact, lower::LowerReport), Vec<BuildError>> {
    lower::lower_web(doc, composed)
}

/// Stages 4 + 5 (native targets): lower a composed document
/// through the #5852 mapping tables for the platform the
/// composed document names (mac/windows/gnome) and emit the
/// artefact plus coverage report. See
/// [`lower_native::lower_native`] for the stated boundaries.
///
/// # Errors
///
/// Returns a build error for a non-native composed document or a
/// path mismatch between the two inputs.
pub fn lower_document_native(
    doc: &Document,
    composed: &compose::ComposedDocument,
) -> Result<(lower_native::NativeArtefact, lower_native::NativeReport), Vec<BuildError>> {
    lower_native::lower_native(doc, composed)
}

/// Stages 4 + 5 (custom-GUI target): lower a composed document
/// through the #5853 presentation-override contract, consuming
/// the override pack as data, and emit the artefact plus report.
/// See [`lower_gui::lower_gui`] for the stated boundaries.
///
/// # Errors
///
/// Returns every build error found: a non-GUI composed document,
/// an undefined or unregistered override, or incomplete state
/// coverage.
pub fn lower_document_gui(
    doc: &Document,
    composed: &compose::ComposedDocument,
    data: &lower_gui::GuiData,
) -> Result<(lower_gui::GuiArtefact, lower_gui::GuiReport), Vec<BuildError>> {
    lower_gui::lower_gui(doc, composed, data)
}

/// Stages 1 + 2: parse a source document and validate it against the
/// vocabulary. This is the compiler core's current entry point; the
/// output is the validated IR the compose stage will consume.
///
/// # Errors
///
/// A parse failure returns the single fatal parse error; a validation
/// failure returns every validation error found.
pub fn parse_and_validate(source: &str) -> Result<Document, Vec<BuildError>> {
    let doc = parse(source).map_err(|e| vec![e])?;
    validate(&doc)?;
    Ok(doc)
}
