//! The semantic markup vocabulary v1, as data.
//!
//! Every entry is transcribed from decision record #5849
//! (`doc/2026-11-08_decision_id5849_semantic_markup_vocabulary_v1.md`):
//! the element registry of §4 (thirty-two elements in four groups, plus
//! the `raw` escape region of §3.1), the binding slots of §8, and the
//! field/preference constraint sets of §4.3.1/§4.4.1.
//!
//! Per #5855 invariant 6 ("tables are data"), a vocabulary change is a
//! change to this table, never a change to parser or validator code.
//! Two slots are universal and therefore not repeated per element:
//! every element accepts at most one `assert` attribute (#5849 §7, the
//! styling seam owned by #5850/#5854), and `show` applies to any
//! element (#5849 §8).

/// One registry element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ElementSpec {
    /// Element kind, e.g. `field`.
    pub name: &'static str,
    /// One-line purpose, used in unknown-element suggestions (#5849
    /// §3.2: the error lists the nearest registry names with their
    /// one-line purposes).
    pub purpose: &'static str,
    /// Accessibility role the kind carries by construction (#5849 §6).
    pub role: &'static str,
    /// Attributes that must be present and valued.
    pub required: &'static [&'static str],
    /// Further attributes the element accepts (beyond the universal
    /// `assert` and `show` slots).
    pub optional: &'static [&'static str],
    /// Attributes whose value must be one of a fixed set:
    /// `(attribute, allowed values)`.
    pub enums: &'static [(&'static str, &'static [&'static str])],
}

/// The full v1 registry: #5849 §4.1–§4.4 plus `raw` (§3.1), and the
/// `conditional` block of #5854 §5 (see [`CONDITIONAL`]).
pub const REGISTRY: &[ElementSpec] = &[
    // §4.1 Document and structure.
    ElementSpec {
        name: "document",
        purpose: "Root; declares version",
        role: "document",
        required: &["version"],
        optional: &[],
        enums: &[],
    },
    ElementSpec {
        name: "page",
        purpose: "One routable screen/window",
        role: "main region",
        required: &["title"],
        optional: &[],
        enums: &[],
    },
    ElementSpec {
        name: "section",
        purpose: "Thematic group",
        role: "region with label",
        required: &["title"],
        optional: &[],
        enums: &[],
    },
    ElementSpec {
        name: "nav",
        purpose: "Navigation group",
        role: "navigation",
        required: &["title"],
        optional: &[],
        enums: &[],
    },
    ElementSpec {
        name: "heading",
        purpose: "Heading at a level",
        role: "heading",
        required: &["level"],
        optional: &[],
        enums: &[],
    },
    ElementSpec {
        name: "card",
        purpose: "Grouped content with a subject",
        role: "group",
        required: &["title"],
        optional: &[],
        enums: &[],
    },
    ElementSpec {
        name: "list",
        purpose: "Ordered/unordered/description collection",
        role: "list",
        required: &["kind"],
        optional: &["items"],
        enums: &[("kind", &["unordered", "ordered", "description"])],
    },
    ElementSpec {
        name: "item",
        purpose: "One list entry",
        role: "listitem",
        required: &[],
        optional: &[],
        enums: &[],
    },
    ElementSpec {
        name: "table",
        purpose: "Tabular data",
        role: "table",
        required: &["title"],
        optional: &["items"],
        enums: &[],
    },
    ElementSpec {
        name: "row",
        purpose: "Table row",
        role: "row",
        required: &[],
        optional: &[],
        enums: &[],
    },
    ElementSpec {
        name: "cell",
        purpose: "Table cell",
        role: "columnheader/cell",
        required: &["kind"],
        optional: &[],
        enums: &[("kind", &["header", "data"])],
    },
    ElementSpec {
        name: "image",
        purpose: "Content image",
        role: "img",
        required: &["src", "alt"],
        optional: &["bind"],
        enums: &[],
    },
    ElementSpec {
        name: "link",
        purpose: "Navigation to an address",
        role: "link",
        required: &["to"],
        optional: &["bind", "disabled-when"],
        enums: &[],
    },
    ElementSpec {
        name: "master-detail",
        purpose: "Two-pane list/detail layout",
        role: "region with label",
        required: &["title"],
        optional: &[],
        enums: &[],
    },
    ElementSpec {
        name: "detail",
        purpose: "The detail pane of master-detail",
        role: "region",
        required: &[],
        optional: &[],
        enums: &[],
    },
    // §4.2 Inline text.
    ElementSpec {
        name: "text",
        purpose: "A paragraph of prose",
        role: "paragraph",
        required: &[],
        optional: &["bind"],
        enums: &[],
    },
    ElementSpec {
        name: "strong",
        purpose: "Strong importance inside text",
        role: "strong",
        required: &[],
        optional: &[],
        enums: &[],
    },
    ElementSpec {
        name: "emphasis",
        purpose: "Stress emphasis inside text",
        role: "emphasis",
        required: &[],
        optional: &[],
        enums: &[],
    },
    // §4.3 Forms and actions.
    ElementSpec {
        name: "form",
        purpose: "Submission boundary",
        role: "form",
        required: &["action"],
        optional: &["on-submit"],
        enums: &[],
    },
    ElementSpec {
        name: "field",
        purpose: "One labelled input",
        role: "per kind",
        required: &["kind", "label"],
        optional: &[
            "bind",
            "required",
            "min-length",
            "max-length",
            "min",
            "max",
            "options",
            "items",
            "multiple",
            "accept",
            "disabled-when",
            "on-change",
        ],
        enums: &[(
            "kind",
            &[
                "text", "password", "number", "checkbox", "choice", "date", "file",
            ],
        )],
    },
    ElementSpec {
        name: "alert",
        purpose: "Validation/status message",
        role: "alert (live region)",
        required: &["kind"],
        optional: &[],
        enums: &[("kind", &["error", "warning", "info", "success"])],
    },
    ElementSpec {
        name: "button",
        purpose: "Action trigger",
        role: "button",
        required: &["action"],
        optional: &["confirm", "disabled-when"],
        enums: &[],
    },
    ElementSpec {
        name: "dialog",
        purpose: "Modal confirmation/content",
        role: "dialog",
        required: &["title", "show"],
        optional: &["on-open", "on-close"],
        enums: &[],
    },
    // §4.4 Application chrome.
    ElementSpec {
        name: "menu",
        purpose: "Command menu",
        role: "menu",
        required: &["title"],
        optional: &["on-open", "on-close"],
        enums: &[],
    },
    ElementSpec {
        name: "menu-item",
        purpose: "One command",
        role: "menuitem",
        required: &["action"],
        optional: &["disabled-when"],
        enums: &[],
    },
    ElementSpec {
        name: "toolbar",
        purpose: "Group of frequent actions",
        role: "toolbar",
        required: &["title"],
        optional: &[],
        enums: &[],
    },
    ElementSpec {
        name: "sidebar",
        purpose: "Persistent secondary region",
        role: "complementary",
        required: &["title"],
        optional: &[],
        enums: &[],
    },
    ElementSpec {
        name: "tabs",
        purpose: "Tab set",
        role: "tablist",
        required: &["title"],
        optional: &["items"],
        enums: &[],
    },
    ElementSpec {
        name: "tab",
        purpose: "One tab and its panel",
        role: "tab",
        required: &["title"],
        optional: &[],
        enums: &[],
    },
    ElementSpec {
        name: "settings",
        purpose: "Settings/preferences group",
        role: "region with label",
        required: &["title"],
        optional: &[],
        enums: &[],
    },
    ElementSpec {
        name: "preference",
        purpose: "One setting",
        role: "per kind",
        required: &["kind", "label", "bind"],
        optional: &[
            "options",
            "min-length",
            "max-length",
            "min",
            "max",
            "multiple",
            "accept",
            "disabled-when",
            "on-change",
        ],
        enums: &[("kind", &["toggle", "choice", "text", "number"])],
    },
    ElementSpec {
        name: "note",
        purpose: "Non-essential annotation tied to nearby content",
        role: "note",
        required: &[],
        optional: &[],
        enums: &[],
    },
    // #5854 §5 conditional block (issue #7920 slice 3): the one
    // vocabulary addition after v1. It is not a rendered element —
    // composition resolves it per target before the cascade runs —
    // so it carries no role and accepts neither `assert` nor `show`.
    ElementSpec {
        name: "conditional",
        purpose: "Embellishment included only on the targets its predicate names",
        role: "none (resolved before rendering)",
        required: &[],
        optional: &["target", "form-factor", "capability"],
        enums: &[],
    },
    // §3.1 The one raw region.
    ElementSpec {
        name: "raw",
        purpose: "Raw HTML/CSS escape region (authored source only)",
        role: "none (raw)",
        required: &["kind"],
        optional: &[],
        enums: &[("kind", &["html", "css"])],
    },
];

/// Looks up an element kind in the registry.
#[must_use]
pub fn lookup(name: &str) -> Option<&'static ElementSpec> {
    REGISTRY.iter().find(|spec| spec.name == name)
}

/// Attributes every element accepts in addition to its own registry
/// entry: the `assert` styling slot (#5849 §7) and the `show` binding
/// slot (#5849 §8, "any element").
pub const UNIVERSAL_ATTRS: &[&str] = &["assert", "show"];

/// Presentation attributes that do not exist anywhere in the registry
/// (#5849 §2.1). Using one is a build error that points at the `assert`
/// slot (#5849 §3.2).
pub const PRESENTATION_ATTRS: &[&str] = &["class", "style", "width", "color", "align"];

/// Binding slots whose values are names (dotted state paths or action
/// names), never expressions (#5849 §8 rule 1).
pub const NAME_VALUED_ATTRS: &[&str] = &[
    "bind",
    "items",
    "show",
    "disabled-when",
    "action",
    "options",
    "on-change",
    "on-submit",
    "on-open",
    "on-close",
];

/// Field constraint applicability (#5849 §4.3.1):
/// `(constraint attribute, field kinds it applies to)`.
pub const FIELD_CONSTRAINT_KINDS: &[(&str, &[&str])] = &[
    ("min-length", &["text", "password"]),
    ("max-length", &["text", "password"]),
    ("min", &["number", "date"]),
    ("max", &["number", "date"]),
    ("options", &["choice"]),
    ("multiple", &["choice", "file"]),
    ("accept", &["file"]),
];

/// Up to three registry names nearest to an unknown element name, for
/// the #5849 §3.2 error. Nearness is a deliberately simple heuristic:
/// shared prefix first, then shared first letter, in registry order —
/// the list is a reading aid, not a guess at intent (the parser never
/// recovers by guessing, #5855 §2.1).
#[must_use]
pub fn nearest_names(unknown: &str) -> Vec<&'static ElementSpec> {
    let first = unknown.chars().next();
    let mut out: Vec<&'static ElementSpec> = Vec::new();
    // Prefix neighbours first (a typo like `secton` still shares its
    // first letter with `section`; a truncation shares a prefix).
    for spec in REGISTRY {
        if spec.name.starts_with(unknown) || unknown.starts_with(spec.name) {
            out.push(spec);
        }
    }
    for spec in REGISTRY {
        if out.len() >= 3 {
            break;
        }
        if spec.name.chars().next() == first && !out.contains(&spec) {
            out.push(spec);
        }
    }
    out.truncate(3);
    out
}

/// The conditional-block element kind (#5854 §5). #5849 v1 had no
/// representation for a conditional block; this is it:
///
/// ```text
/// <conditional target="web">
///   <text>Tip: see the <link to="docs.account">account guide</link>.</text>
/// </conditional>
/// ```
///
/// A block is part of the one shared source. Its predicate names
/// target facts only — the three [`PREDICATE_ATTRS`], each a
/// space-separated list of names — and the block is included on a
/// target when every predicate attribute present lists a fact of
/// that target (any listed name matches within one attribute). A
/// block names at least one predicate attribute; blocks do not
/// nest; `assert` and `show` do not apply, because the block is
/// never rendered itself and its predicate is never state data.
/// The block is transparent to the child rules of #5849 §4.5: its
/// children are checked as children of the block's parent.
pub const CONDITIONAL: &str = "conditional";

/// The predicate attributes of a conditional block, each naming one
/// kind of target fact (#5854 §5): the target name, the
/// form-factor class, and a declared capability flag.
pub const PREDICATE_ATTRS: &[&str] = &["target", "form-factor", "capability"];

/// Element kinds that are embellishment, and so may sit inside a
/// conditional block (#5854 §5: "a block may carry embellishment
/// only"). Prose, inline emphasis, links, images and notes add to a
/// screen without carrying its function; everything else in the
/// registry — structure, forms, actions, chrome, bound inputs — is
/// essential content. A `<raw kind="css">` region is embellishment
/// too (it only restyles); raw HTML is not, since it can carry
/// anything. The #5855 §4 lint reads this table.
pub const CONDITIONAL_EMBELLISHMENT: &[&str] =
    &["text", "strong", "emphasis", "link", "image", "note"];

/// Whether `el` may sit inside a conditional block as embellishment
/// (see [`CONDITIONAL_EMBELLISHMENT`]).
#[must_use]
pub fn is_embellishment(el: &crate::Element) -> bool {
    CONDITIONAL_EMBELLISHMENT.contains(&el.name.as_str())
        || (el.name == "raw" && el.attr("kind") == Some("css"))
}
