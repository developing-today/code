//! The web mapping table, as data.
//!
//! Every entry is transcribed from decision record #5851
//! (`doc/2026-11-09_decision_id5851_assertion_tailwind_mapping.md`):
//! the element-kind → DOM table of §2 and the assertion-role →
//! utility tables of §3. Per #5855 invariant 6 ("tables are data"),
//! a mapping change is a change to this table, never a change to
//! the lowering code in [`crate::lower`]. Dimension names are the
//! closed v1 set of #5850 as adopted by #5851 assumption A3:
//! tone, emphasis, density, text, gap, pad, corner, elevation.

/// One element-kind → DOM row of #5851 §2 whose tag does not depend
/// on a variant attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ElementRow {
    /// Element kind, e.g. `page`.
    pub kind: &'static str,
    /// DOM tag, e.g. `main`.
    pub tag: &'static str,
    /// Fixed `role` attribute the row carries, when §2 names one
    /// (`menu` → `menu`, `tab` → `tab`, …).
    pub role_attr: Option<&'static str>,
}

/// #5851 §2 rows with a fixed tag. Kinds whose tag depends on a
/// variant attribute (`heading`, `list`, `field`, `alert`, `cell`)
/// are resolved by [`variant_tag`]; kinds with no row at all lower
/// to the layer default and are reported (see [`crate::lower`]).
pub const ELEMENT_ROWS: &[ElementRow] = &[
    ElementRow {
        kind: "document",
        tag: "html",
        role_attr: None,
    },
    ElementRow {
        kind: "page",
        tag: "main",
        role_attr: None,
    },
    ElementRow {
        kind: "section",
        tag: "section",
        role_attr: None,
    },
    ElementRow {
        kind: "nav",
        tag: "nav",
        role_attr: None,
    },
    ElementRow {
        kind: "card",
        tag: "section",
        role_attr: None,
    },
    ElementRow {
        kind: "item",
        tag: "li",
        role_attr: None,
    },
    ElementRow {
        kind: "table",
        tag: "table",
        role_attr: None,
    },
    ElementRow {
        kind: "row",
        tag: "tr",
        role_attr: None,
    },
    ElementRow {
        kind: "image",
        tag: "img",
        role_attr: None,
    },
    ElementRow {
        kind: "link",
        tag: "a",
        role_attr: None,
    },
    ElementRow {
        kind: "master-detail",
        tag: "div",
        role_attr: None,
    },
    ElementRow {
        kind: "detail",
        tag: "div",
        role_attr: None,
    },
    ElementRow {
        kind: "text",
        tag: "p",
        role_attr: None,
    },
    ElementRow {
        kind: "strong",
        tag: "strong",
        role_attr: None,
    },
    ElementRow {
        kind: "emphasis",
        tag: "em",
        role_attr: None,
    },
    ElementRow {
        kind: "form",
        tag: "form",
        role_attr: None,
    },
    ElementRow {
        kind: "button",
        tag: "button",
        role_attr: None,
    },
    ElementRow {
        kind: "dialog",
        tag: "dialog",
        role_attr: None,
    },
    ElementRow {
        kind: "menu",
        tag: "div",
        role_attr: Some("menu"),
    },
    ElementRow {
        kind: "menu-item",
        tag: "div",
        role_attr: Some("menuitem"),
    },
    ElementRow {
        kind: "toolbar",
        tag: "div",
        role_attr: Some("toolbar"),
    },
    ElementRow {
        kind: "sidebar",
        tag: "aside",
        role_attr: None,
    },
    ElementRow {
        kind: "tabs",
        tag: "div",
        role_attr: Some("tablist"),
    },
    ElementRow {
        kind: "tab",
        tag: "button",
        role_attr: Some("tab"),
    },
    ElementRow {
        kind: "settings",
        tag: "section",
        role_attr: None,
    },
];

/// The DOM tag for a variant-driven kind (#5851 §2): `heading`
/// (`level` 1–6 → `h1`–`h6`), `list` (`kind` → `ul`/`ol`/`dl`),
/// `field` (`kind` → `input`/`select`), `alert` (`kind` → `div`,
/// with the live-region role from [`alert_role`]), `cell` (`kind`
/// → `th`/`td`). Returns `None` when the kind is not variant-driven
/// or the variant value has no row.
#[must_use]
pub fn variant_tag(kind: &str, variant: Option<&str>) -> Option<&'static str> {
    match (kind, variant) {
        ("heading", Some(level)) => match level {
            "1" => Some("h1"),
            "2" => Some("h2"),
            "3" => Some("h3"),
            "4" => Some("h4"),
            "5" => Some("h5"),
            "6" => Some("h6"),
            _ => None,
        },
        ("list", Some("unordered")) => Some("ul"),
        ("list", Some("ordered")) => Some("ol"),
        ("list", Some("description")) => Some("dl"),
        ("field", Some("text" | "password" | "number" | "date" | "checkbox" | "file")) => {
            Some("input")
        }
        ("field", Some("choice")) => Some("select"),
        ("alert", Some(_)) => Some("div"),
        ("cell", Some("header")) => Some("th"),
        ("cell", Some("data")) => Some("td"),
        _ => None,
    }
}

/// The `type` attribute a `field` variant carries on its `<input>`
/// (#5851 §2). `None` for non-`input` rows.
#[must_use]
pub fn field_input_type(variant: Option<&str>) -> Option<&'static str> {
    match variant {
        Some("text") => Some("text"),
        Some("password") => Some("password"),
        Some("number") => Some("number"),
        Some("date") => Some("date"),
        Some("checkbox") => Some("checkbox"),
        Some("file") => Some("file"),
        _ => None,
    }
}

/// The live-region role of an `alert` variant (#5851 §2: `error`
/// is assertive `alert`, every other kind is `status`).
#[must_use]
pub fn alert_role(variant: Option<&str>) -> &'static str {
    if variant == Some("error") {
        "alert"
    } else {
        "status"
    }
}

/// One assertion-role → utilities row of #5851 §3, keyed by the
/// #5850 dimension (`property`) and the role token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoleRow {
    /// Assertion dimension, e.g. `text`.
    pub property: &'static str,
    /// Role token, e.g. `heading`.
    pub token: &'static str,
    /// Utilities the role emits, in record order (the artefact
    /// order-normalises them, #5855 §3.2).
    pub classes: &'static [&'static str],
}

/// #5851 §3.1 (tone), §3.2 (text), §3.4 (corner/elevation) and
/// §3.5 (emphasis). Spacing (§3.3) is generative — `pad`/`gap` over
/// the `space-0`…`space-8` keys — and lives in [`spacing_class`];
/// density (§3.3) emits no utilities at all (it is a theme
/// multiplier), which [`crate::lower`] records as mapped.
pub const ROLE_ROWS: &[RoleRow] = &[
    // §3.1 tone — surface roles emit their bg- utility.
    RoleRow {
        property: "tone",
        token: "surface-base",
        classes: &["bg-surface-base"],
    },
    RoleRow {
        property: "tone",
        token: "surface-raised",
        classes: &["bg-surface-raised"],
    },
    RoleRow {
        property: "tone",
        token: "surface-sunken",
        classes: &["bg-surface-sunken"],
    },
    RoleRow {
        property: "tone",
        token: "surface-overlay",
        classes: &["bg-surface-overlay"],
    },
    RoleRow {
        property: "tone",
        token: "surface-inverse",
        classes: &["bg-surface-inverse"],
    },
    // §3.1 tone — content roles emit their text- utility.
    RoleRow {
        property: "tone",
        token: "content-primary",
        classes: &["text-content-primary"],
    },
    RoleRow {
        property: "tone",
        token: "content-secondary",
        classes: &["text-content-secondary"],
    },
    RoleRow {
        property: "tone",
        token: "content-muted",
        classes: &["text-content-muted"],
    },
    RoleRow {
        property: "tone",
        token: "content-inverse",
        classes: &["text-content-inverse"],
    },
    RoleRow {
        property: "tone",
        token: "content-link",
        classes: &["text-content-link"],
    },
    // §3.1 tone — accent pair and status roles.
    RoleRow {
        property: "tone",
        token: "accent",
        classes: &["bg-accent", "text-accent-content"],
    },
    RoleRow {
        property: "tone",
        token: "accent-content",
        classes: &["text-accent-content"],
    },
    RoleRow {
        property: "tone",
        token: "success",
        classes: &["text-success", "bg-success-surface", "border-success"],
    },
    RoleRow {
        property: "tone",
        token: "warning",
        classes: &["text-warning", "bg-warning-surface", "border-warning"],
    },
    RoleRow {
        property: "tone",
        token: "danger",
        classes: &["text-danger", "bg-danger-surface", "border-danger"],
    },
    RoleRow {
        property: "tone",
        token: "info",
        classes: &["text-info", "bg-info-surface", "border-info"],
    },
    RoleRow {
        property: "tone",
        token: "border-subtle",
        classes: &["border-border-subtle"],
    },
    RoleRow {
        property: "tone",
        token: "border-strong",
        classes: &["border-border-strong"],
    },
    RoleRow {
        property: "tone",
        token: "border-focus",
        classes: &["focus-visible:outline-border-focus"],
    },
    // §3.2 text roles.
    RoleRow {
        property: "text",
        token: "display",
        classes: &["text-step-plus-5", "leading-tight", "font-bold"],
    },
    RoleRow {
        property: "text",
        token: "heading",
        classes: &["text-step-plus-2", "leading-tight", "font-semibold"],
    },
    RoleRow {
        property: "text",
        token: "body",
        classes: &["text-step-0", "leading-normal"],
    },
    RoleRow {
        property: "text",
        token: "body-small",
        classes: &["text-step-minus-1", "leading-normal"],
    },
    RoleRow {
        property: "text",
        token: "caption",
        classes: &[
            "text-step-minus-1",
            "leading-tight",
            "text-content-secondary",
        ],
    },
    RoleRow {
        property: "text",
        token: "label",
        classes: &["text-step-0", "leading-tight", "font-medium"],
    },
    RoleRow {
        property: "text",
        token: "code",
        classes: &["font-mono", "text-step-minus-1", "leading-normal"],
    },
    RoleRow {
        property: "text",
        token: "link",
        classes: &["text-content-link", "underline"],
    },
    // §3.4 corner and elevation roles.
    RoleRow {
        property: "corner",
        token: "corner-square",
        classes: &["rounded-none"],
    },
    RoleRow {
        property: "corner",
        token: "corner-rounded",
        classes: &["rounded"],
    },
    RoleRow {
        property: "corner",
        token: "corner-full",
        classes: &["rounded-full"],
    },
    RoleRow {
        property: "elevation",
        token: "elevation-flat",
        classes: &["shadow-none"],
    },
    RoleRow {
        property: "elevation",
        token: "elevation-raised",
        classes: &["shadow"],
    },
    RoleRow {
        property: "elevation",
        token: "elevation-overlay",
        classes: &["shadow-lg"],
    },
    // §3.5 emphasis bundles. `destructive` also tints with
    // `bg-danger-surface` where the element has a surface; the
    // lowerer cannot know that from the assertion alone, so the
    // unconditional part of the bundle is what the table carries.
    RoleRow {
        property: "emphasis",
        token: "primary",
        classes: &["bg-accent", "text-accent-content"],
    },
    RoleRow {
        property: "emphasis",
        token: "destructive",
        classes: &["text-danger", "border-danger"],
    },
    RoleRow {
        property: "emphasis",
        token: "subtle",
        classes: &["bg-transparent"],
    },
];

/// The utility a spacing role emits (#5851 §3.3): `pad` over
/// `space-N` emits `p-space-N`, `gap` emits `gap-space-N`, for the
/// theme spacing keys `space-0`…`space-8`. `None` when the token
/// is not a spacing key.
#[must_use]
pub fn spacing_class(property: &str, token: &str) -> Option<String> {
    let n: u32 = token.strip_prefix("space-")?.parse().ok()?;
    if n > 8 {
        return None;
    }
    match property {
        "pad" => Some(format!("p-space-{n}")),
        "gap" => Some(format!("gap-space-{n}")),
        _ => None,
    }
}

/// Looks up one assertion in the role table.
#[must_use]
pub fn role_row(property: &str, token: &str) -> Option<&'static RoleRow> {
    ROLE_ROWS
        .iter()
        .find(|r| r.property == property && r.token == token)
}

/// Looks up one element kind in the fixed-tag table.
#[must_use]
pub fn element_row(kind: &str) -> Option<&'static ElementRow> {
    ELEMENT_ROWS.iter().find(|r| r.kind == kind)
}
