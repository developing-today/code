use super::*;

/// Error type for markdown conversion.
#[derive(Debug)]
pub enum ConversionError {
    /// The JSON structure is missing a required `type` field.
    MissingType,
    /// The JSON contains an unknown node type.
    UnknownType(String),
    /// A node appears where its kind cannot legally go (e.g. an inline
    /// node as a direct child of `doc`, a block node inside a
    /// `paragraph`). Converting it would build an ill-formed comrak AST
    /// that panics the formatter in debug builds and emits invalid
    /// markdown in release builds (#1043).
    MisplacedNode {
        /// The parent node type the child was found under.
        parent: String,
        /// The misplaced child node type.
        child: String,
    },
    /// Failed to format markdown output.
    FormatError(std::fmt::Error),
}

impl std::fmt::Display for ConversionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingType => write!(f, "JSON node missing 'type' field"),
            Self::UnknownType(t) => write!(f, "Unknown node type: {t}"),
            Self::MisplacedNode { parent, child } => {
                write!(f, "Misplaced node: {child} cannot appear inside {parent}")
            }
            Self::FormatError(e) => write!(f, "Markdown format error: {e}"),
        }
    }
}

impl std::error::Error for ConversionError {}

impl From<std::fmt::Error> for ConversionError {
    fn from(e: std::fmt::Error) -> Self {
        Self::FormatError(e)
    }
}

/// Get comrak options for `CommonMark` parsing.
pub(super) fn commonmark_options() -> Options<'static> {
    let mut options = Options::default();
    // Enable GFM extensions that map to ProseMirror marks/nodes
    options.extension.strikethrough = true;
    options.extension.tasklist = true;
    options.extension.table = true;
    options.extension.autolink = true; // #998: bare URLs/emails become links
    options.extension.footnotes = true;
    options.extension.front_matter_delimiter = Some("---".to_owned());
    options.parse.smart = false; // Don't convert quotes/dashes
    options.render.r#unsafe = true; // Allow HTML output (needed for sized images)
    // #954: the CommonMark renderer saves an info-less code block as an
    // indented block whenever it can (its fenced branch otherwise needs a
    // non-empty info string or a literal edge case). Force the fenced
    // spelling for every code block so info-less fences round-trip as
    // fences. Render-only: parsing is unaffected.
    options.render.prefer_fenced = true;
    options
}

/// Active marks being applied to inline content.
#[derive(Clone)]
pub(super) struct Mark {
    pub(super) mark_type: &'static str,
    pub(super) attrs: Option<Value>,
}

/// Canonical mark nesting order, outermost first (#944).
///
/// Both conversion directions agree on this order, so a marked text node
/// has exactly one markdown spelling: the parse direction sorts the marks
/// it emits into this order (see [`text_node_with_marks`]) and the save
/// direction sorts incoming marks into this order before wrapping (see
/// `create_marked_text`). Without the shared order, `**[text](url)` and
/// `[**text**](url)` converted into each other on every save — the parse
/// direction appends marks outermost-first (wrapper arms push as they
/// nest) while the save direction treated the array as innermost-first.
pub(super) fn mark_nesting_rank(mark_type: &str) -> u8 {
    match mark_type {
        "link" => 0,
        "strong" => 1,
        "em" => 2,
        "strikethrough" => 3,
        "code" => 4,
        _ => 5,
    }
}
