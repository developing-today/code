//! The style assertion vocabulary v1, as data.
//!
//! Every entry is transcribed from the decision record on issue
//! #5850 ("style assertion vocabulary v1", decided 2026-11-08,
//! comment 6719140810; the issue is closed and the record is the
//! settled vocabulary): the closed dimension set of §2, the colour
//! roles of §3, the typography roles of §4, and the spacing,
//! density, corner, and elevation roles of §6. The emphasis values
//! are the variant roles the parent design names (primary,
//! destructive, subtle — #5679's style-vocabulary line, exercised
//! in #5850 §2's examples and mapped by #5851 §3.5 / #5852 §3.4).
//!
//! Per #5855 invariant 6 ("tables are data"), a vocabulary change
//! is a change to this table, never a change to validator or
//! pipeline code. This module is the discharge of #5855 assumption
//! A2: stage 2 was defined to carry the `assert` slot opaquely
//! until #5850 settled; it has settled, so the slot is now parsed
//! and validated against this data (see [`crate::validate`]) and
//! composed as layer-3 assertions (see [`crate::compose`]).
//!
//! The slot grammar, transcribing #5850 §2's model (an assertion
//! is a (dimension, role-value) pair): the `assert` value is a
//! comma-separated list of `dimension token` pairs, e.g.
//! `assert="tone accent, emphasis primary"`. Whitespace around a
//! pair is insignificant. A dimension may appear at most once per
//! slot — an exact tie between two assertions in the source is an
//! authoring error, never resolved by order (#5854 §2.2).
//!
//! Deliberately not in the grammar: state-qualified assertions.
//! #5850 §5 fixes states as deltas the target derives from the
//! base roles, with a fixed precedence; authors never assert
//! per-state styling directly. The state roles themselves
//! (hover, focus-visible, active, selected, disabled, error,
//! loading) are consumed by the targets' state machinery (see
//! [`crate::gui_table`]), not by this slot.

/// The version of the assertion vocabulary this table transcribes
/// (#5850: "style assertion vocabulary v1"). A vocabulary version
/// change re-issues this table (#5850 invariant 2: the dimension
/// set is closed in v1; extension is by version change, never by
/// per-program custom dimensions).
pub const ASSERTION_VOCABULARY_VERSION: &str = "1";

/// One assertion dimension of #5850 §2 with its closed role
/// domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DimensionSpec {
    /// Dimension name, e.g. `tone`.
    pub name: &'static str,
    /// One-line purpose, used in error messages.
    pub purpose: &'static str,
    /// The #5850 section that settles this dimension's domain.
    pub section: &'static str,
    /// The closed set of role tokens the dimension accepts in v1.
    pub tokens: &'static [&'static str],
}

/// The spacing role keys of #5850 §6 (`space-0`…`space-8`),
/// shared by the `gap` and `pad` dimensions.
pub const SPACING_TOKENS: &[&str] = &[
    "space-0", "space-1", "space-2", "space-3", "space-4", "space-5", "space-6", "space-7",
    "space-8",
];

/// The full v1 dimension set: #5850 §2's closed list, in the
/// record's order.
pub const DIMENSIONS: &[DimensionSpec] = &[
    DimensionSpec {
        name: "tone",
        purpose: "Which colour role applies",
        section: "§3",
        tokens: &[
            // §3 surface roles.
            "surface-base",
            "surface-raised",
            "surface-sunken",
            "surface-overlay",
            "surface-inverse",
            // §3 content roles.
            "content-primary",
            "content-secondary",
            "content-muted",
            "content-inverse",
            "content-link",
            // §3 accent/identity roles.
            "accent",
            "accent-content",
            // §3 status roles.
            "success",
            "warning",
            "danger",
            "info",
            // §3 border roles.
            "border-subtle",
            "border-strong",
            "border-focus",
        ],
    },
    DimensionSpec {
        name: "emphasis",
        purpose: "Variant emphasis of the element",
        section: "§2",
        tokens: &["primary", "destructive", "subtle"],
    },
    DimensionSpec {
        name: "density",
        purpose: "Spacing/control density",
        section: "§6",
        tokens: &["comfortable", "compact"],
    },
    DimensionSpec {
        name: "text",
        purpose: "Typography role",
        section: "§4",
        tokens: &[
            "display",
            "heading",
            "body",
            "body-small",
            "caption",
            "label",
            "code",
            "link",
        ],
    },
    DimensionSpec {
        name: "gap",
        purpose: "Spacing role between children",
        section: "§6",
        tokens: SPACING_TOKENS,
    },
    DimensionSpec {
        name: "pad",
        purpose: "Spacing role inside the element",
        section: "§6",
        tokens: SPACING_TOKENS,
    },
    DimensionSpec {
        name: "corner",
        purpose: "Corner shape role",
        section: "§6",
        tokens: &["corner-square", "corner-rounded", "corner-full"],
    },
    DimensionSpec {
        name: "elevation",
        purpose: "Elevation (ordering) role",
        section: "§6",
        tokens: &["elevation-flat", "elevation-raised", "elevation-overlay"],
    },
];

/// Looks up a dimension by name.
#[must_use]
pub fn lookup_dimension(name: &str) -> Option<&'static DimensionSpec> {
    DIMENSIONS.iter().find(|spec| spec.name == name)
}

/// One parsed assertion from an `assert` slot: a (dimension,
/// role-value) pair (#5850 §2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotAssertion {
    /// Dimension name (a member of [`DIMENSIONS`]).
    pub dimension: String,
    /// Role token (a member of the dimension's domain).
    pub token: String,
}

/// Whether `word` has the shape of a role name: a lowercase ASCII
/// letter followed by lowercase letters, digits, and hyphens (the
/// shape of every name in the #5849/#5850 vocabularies). A word
/// without this shape in the token position is a raw value — a
/// hex colour, a size, a font name — which #5850 invariant 1
/// forbids in assertions.
fn is_role_name(word: &str) -> bool {
    let mut chars = word.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Up to three names from `names` nearest to `unknown`, for
/// suggestion lists — the #5849 §3.2 idiom, with the same
/// deliberately simple heuristic as
/// [`crate::vocabulary::nearest_names`]: shared prefix first,
/// then shared first letter, in table order.
fn nearest<'a>(unknown: &str, names: &[&'a str]) -> Vec<&'a str> {
    let first = unknown.chars().next();
    let mut out: Vec<&'a str> = Vec::new();
    for name in names {
        if name.starts_with(unknown) || unknown.starts_with(name) {
            out.push(name);
        }
    }
    for name in names {
        if out.len() >= 3 {
            break;
        }
        if name.chars().next() == first && !out.contains(name) {
            out.push(name);
        }
    }
    out.truncate(3);
    out
}

/// The dimension names, for error messages.
fn dimension_names() -> Vec<&'static str> {
    DIMENSIONS.iter().map(|d| d.name).collect()
}

/// Parses and validates one `assert` slot value against the
/// vocabulary: the grammar above, the closed dimension set, each
/// dimension's closed token domain, the no-raw-values rule
/// (#5850 invariant 1), and the one-pair-per-dimension rule
/// (#5854 §2.2). Returns the parsed pairs, or every problem
/// found as a message string — the callers (stage 2 validation,
/// stage 3 composition) attach the element's source location,
/// exactly as for the #5849 attribute checks.
pub fn parse_assert_slot(value: &str) -> Result<Vec<SlotAssertion>, Vec<String>> {
    let mut problems: Vec<String> = Vec::new();
    let mut assertions: Vec<SlotAssertion> = Vec::new();
    if value.trim().is_empty() {
        problems.push(
            "the `assert` slot is empty: it names dimension/token pairs \
             (e.g. `tone accent, emphasis primary`) or is omitted (#5850 §2)"
                .to_string(),
        );
        return Err(problems);
    }
    for chunk in value.split(',') {
        let chunk = chunk.trim();
        let words: Vec<&str> = chunk.split_whitespace().collect();
        if words.len() != 2 {
            problems.push(format!(
                "`assert` entry `{chunk}` is not a dimension/token pair: an assertion \
                 is a (dimension, role-value) pair, e.g. `tone accent` (#5850 §2)"
            ));
            continue;
        }
        let (dimension, token) = (words[0], words[1]);
        let Some(spec) = lookup_dimension(dimension) else {
            let suggestions = nearest(dimension, &dimension_names());
            let hint = if suggestions.is_empty() {
                String::new()
            } else {
                format!("; nearest dimensions: {}", suggestions.join(", "))
            };
            problems.push(format!(
                "unknown assertion dimension `{dimension}` in `assert`: the v1 dimensions \
                 are [{}]{hint} (#5850 §2)",
                dimension_names().join(", ")
            ));
            continue;
        };
        if !is_role_name(token) {
            problems.push(format!(
                "`assert` token `{token}` is not a role name: assertions name roles, \
                 never raw values — no hex colours, sizes, font names, or target \
                 class names (#5850 invariant 1)"
            ));
            continue;
        }
        if !spec.tokens.contains(&token) {
            let suggestions = nearest(token, spec.tokens);
            let hint = if suggestions.is_empty() {
                String::new()
            } else {
                format!("; nearest roles: {}", suggestions.join(", "))
            };
            problems.push(format!(
                "unknown assertion token `{token}` for dimension `{dimension}` in \
                 `assert`: the {dimension} roles are [{}]{hint} (#5850 {})",
                spec.tokens.join(", "),
                spec.section
            ));
            continue;
        }
        if assertions.iter().any(|a| a.dimension == dimension) {
            problems.push(format!(
                "dimension `{dimension}` is asserted twice in one `assert` slot: an \
                 exact tie between two assertions in the source is an authoring \
                 error, never resolved by order (#5854 §2.2)"
            ));
            continue;
        }
        assertions.push(SlotAssertion {
            dimension: dimension.to_string(),
            token: token.to_string(),
        });
    }
    if problems.is_empty() {
        Ok(assertions)
    } else {
        Err(problems)
    }
}
