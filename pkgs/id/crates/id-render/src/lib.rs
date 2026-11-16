//! Rendering pipeline compiler core for the id programme (issue #7918).
//!
//! The pipeline shape is settled by decision record #5855
//! (`doc/2026-11-08_decision_id5855_rendering_pipeline_conformance.md`):
//! five stages over one target-neutral IR — parse, validate, compose,
//! lower per target, emit artefact + report — with the mapping tables of
//! #5851/#5852/#5853 consumed as data inputs to lowering, never as code
//! branches in the pipeline.
//!
//! This crate implements the first stages:
//!
//! * **Stage 1 — parse** ([`parse`]): semantic source in the HTML-shaped
//!   authoring syntax of decision record #5849 §2 parses to the IR
//!   ([`Document`]). Parse errors are fatal, located, and never recovered
//!   by guessing (#5855 §2.1).
//! * **Stage 2 — validate, vocabulary half** ([`validate`]): the IR is
//!   checked against the #5849 element vocabulary (consumed as data from
//!   [`vocabulary`]): unknown element kinds, unknown attributes, missing
//!   required attributes, enum-valued attribute domains, the child and
//!   sibling rules of #5849 §4.5, and the names-not-expressions rule for
//!   binding slots (#5849 §8) are build errors with source locations.
//!
//! * **Stage 3 — compose** ([`compose`]): the #5854 cascade over
//!   data-supplied composition tables (theme layering, assertion and
//!   override resolution, the web-only raw-CSS layer), producing the
//!   resolved IR and composition report for one target. Conditional
//!   blocks await a #5849 vocabulary representation; see the module
//!   documentation for the stated boundary.
//!
//! The assertion half of stage 2 is deliberately not implemented yet:
//! the `assert` attribute's grammar and role vocabulary are #5850's,
//! and #5850 is not settled. The slot is parsed and carried opaquely,
//! exactly as #5855 assumption A2 prescribes (the pipeline is defined
//! over element kinds and assertion roles generically and consumes
//! both vocabularies as versioned data). Stages 3–5 land with the
//! mapping tables as follow-on work on #7918.

pub mod compose;
mod parse;
mod validate;
pub mod vocabulary;

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

/// Stage 2 (vocabulary half): validate a parsed document against the
/// #5849 vocabulary — element registry, attribute registry, enum
/// domains, child/sibling rules, and binding-slot name rules.
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
