//! Stage 2 — validate (#5855 §2.1).
//!
//! Checks a parsed document against the #5849 vocabulary, consumed as
//! data from [`crate::vocabulary`]: required and allowed attributes,
//! enum domains, field/preference constraint applicability, the child
//! and sibling rules of #5849 §4.5, the document version rule of
//! §2.4, and the names-not-expressions rule for binding slots (§8).
//! Every violation is a build error with a source location; all
//! violations are collected before the build fails.
//!
//! The assertion half of the stage checks the `assert` slot against
//! the #5850 assertion vocabulary, consumed as versioned data from
//! [`crate::assertion_vocabulary`]: the slot's (dimension,
//! role-value) grammar, the closed dimension set and token domains,
//! the no-raw-values rule (#5850 invariant 1), and the
//! one-pair-per-dimension rule (#5854 §2.2). #5855 assumption A2
//! carried the slot opaquely until #5850 settled; it has settled,
//! so the slot is checked here like every other vocabulary input.

use crate::assertion_vocabulary;
use crate::vocabulary::{self, ElementSpec, FIELD_CONSTRAINT_KINDS};
use crate::{BuildError, Document, Element, Node};

/// Validates the document; see the module docs.
pub(crate) fn validate(doc: &Document) -> Result<(), Vec<BuildError>> {
    let mut errors = Vec::new();
    // §2.4: v1 parsers accept version="1" and reject any other value
    // with an error naming the supported versions.
    match doc.root.attr("version") {
        Some("1") => {}
        Some(other) => errors.push(BuildError::new(
            format!(
                "unsupported document version \"{other}\"; this pipeline supports version \"1\" (#5849 §2.4)"
            ),
            doc.root.span.line,
            doc.root.span.col,
        )),
        None => {}
    }
    validate_element(&doc.root, None, &[], false, &mut errors);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// `parent` is the nearest enclosing *rendered* element: a
/// conditional block is transparent to the #5849 §4.5 child rules,
/// so its children are validated as children of the block's parent,
/// and `siblings` is that parent's effective child list (see
/// [`effective_children`]). `in_conditional` is set inside a block,
/// where a second block is a nesting error (#5854 §5).
fn validate_element(
    el: &Element,
    parent: Option<&str>,
    siblings: &[&Node],
    in_conditional: bool,
    errors: &mut Vec<BuildError>,
) {
    let Some(spec) = vocabulary::lookup(&el.name) else {
        errors.push(BuildError::new(
            format!("unknown element <{}>", el.name),
            el.span.line,
            el.span.col,
        ));
        return;
    };

    if el.name == vocabulary::CONDITIONAL {
        check_attributes(el, spec, parent, siblings, errors);
        check_conditional(el, parent, in_conditional, errors);
        for child in &el.children {
            if let Node::Element(child_el) = child {
                validate_element(child_el, parent, siblings, true, errors);
            }
        }
        return;
    }

    check_attributes(el, spec, parent, siblings, errors);
    check_assert_slot(el, errors);
    check_enums(el, spec, errors);
    check_constraint_applicability(el, errors);
    check_child_rules(el, parent, errors);

    let children = effective_children(el);
    for child in &el.children {
        if let Node::Element(child_el) = child {
            validate_element(child_el, Some(&el.name), &children, in_conditional, errors);
        }
    }
}

/// An element's children with every conditional block replaced by
/// the block's own children: the child list the #5849 §4.5 rules
/// see, since a block is transparent to them.
pub(crate) fn effective_children(el: &Element) -> Vec<&Node> {
    let mut out = Vec::new();
    for child in &el.children {
        match child {
            Node::Element(child_el) if child_el.name == vocabulary::CONDITIONAL => {
                out.extend(effective_children(child_el));
            }
            other => out.push(other),
        }
    }
    out
}

/// The #5854 §5 rules on the block itself: a predicate of target
/// facts only (at least one predicate attribute, each a non-empty
/// list of names), no `assert` or `show` slot, no nesting, and never
/// the document root.
fn check_conditional(
    el: &Element,
    parent: Option<&str>,
    in_conditional: bool,
    errors: &mut Vec<BuildError>,
) {
    let at = |message: String| BuildError::new(message, el.span.line, el.span.col);
    if parent.is_none() {
        errors.push(at(
            "<conditional> cannot be the document root; it sits inside the one shared source (#5854 §5)"
                .to_string(),
        ));
    }
    if in_conditional {
        errors.push(at(
            "conditional blocks do not nest in v1; merge the predicates into one block (#5854 §5)"
                .to_string(),
        ));
    }
    for slot in vocabulary::UNIVERSAL_ATTRS {
        if el.has_attr(slot) {
            errors.push(at(format!(
                "the `{slot}` slot does not apply to <conditional>: a block is resolved per target before rendering, and its predicate names target facts only, never state (#5854 §5)"
            )));
        }
    }
    let mut any_predicate = false;
    for attr in vocabulary::PREDICATE_ATTRS {
        if !el.has_attr(attr) {
            continue;
        }
        any_predicate = true;
        let terms: Vec<&str> = el.attr(attr).unwrap_or("").split_whitespace().collect();
        if terms.is_empty() {
            errors.push(at(format!(
                "predicate `{attr}` on <conditional> needs at least one name (#5854 §5)"
            )));
        }
        for term in terms {
            if !is_name_path(term) || term.contains('.') {
                errors.push(at(format!(
                    "predicate `{attr}` on <conditional> lists names, not expressions: \"{term}\" (#5854 §5: no arbitrary expressions)"
                )));
            }
        }
    }
    if !any_predicate {
        errors.push(at(format!(
            "<conditional> needs a predicate: one or more of {} (#5854 §5)",
            vocabulary::PREDICATE_ATTRS
                .iter()
                .map(|a| format!("`{a}`"))
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
}

fn check_attributes(
    el: &Element,
    spec: &ElementSpec,
    _parent: Option<&str>,
    siblings: &[&Node],
    errors: &mut Vec<BuildError>,
) {
    for (name, value) in &el.attrs {
        if vocabulary::PRESENTATION_ATTRS.contains(&name.as_str()) {
            // #5849 §2.1/§3.2: no presentation attributes anywhere in
            // the registry; the error points at the assert slot.
            errors.push(BuildError::new(
                format!(
                    "<{}> has no `{name}` attribute: appearance is expressed through the `assert` slot, not presentation attributes (#5849 §2.1)",
                    el.name
                ),
                el.span.line,
                el.span.col,
            ));
            continue;
        }
        let known = spec.required.contains(&name.as_str())
            || spec.optional.contains(&name.as_str())
            || vocabulary::UNIVERSAL_ATTRS.contains(&name.as_str());
        if !known {
            errors.push(BuildError::new(
                format!(
                    "unknown attribute `{name}` on <{}>: only the attributes in the #5849 registry entry parse",
                    el.name
                ),
                el.span.line,
                el.span.col,
            ));
            continue;
        }
        if vocabulary::NAME_VALUED_ATTRS.contains(&name.as_str()) {
            match value {
                Some(v) if !is_name_path(v) => {
                    errors.push(BuildError::new(
                        format!(
                            "attribute `{name}` on <{}> must be a name (a dotted state path or an action name), not an expression: \"{v}\" (#5849 §8 rule 1)",
                            el.name
                        ),
                        el.span.line,
                        el.span.col,
                    ));
                }
                Some(_) => {}
                None => {
                    errors.push(BuildError::new(
                        format!("attribute `{name}` on <{}> requires a value", el.name),
                        el.span.line,
                        el.span.col,
                    ));
                }
            }
        }
    }

    for req in spec.required {
        // #5849 §4.1: a section's title may be omitted only if it is
        // the sole section of its page.
        if el.name == "section" && *req == "title" && is_sole_section(siblings) {
            continue;
        }
        if !el.has_attr(req) {
            errors.push(BuildError::new(
                format!("<{}> requires attribute `{req}` (#5849 §4)", el.name),
                el.span.line,
                el.span.col,
            ));
        } else if el.attr(req).is_none() {
            errors.push(BuildError::new(
                format!("attribute `{req}` on <{}> requires a value", el.name),
                el.span.line,
                el.span.col,
            ));
        }
    }

    // #5849 §4.4.1: options is required for a choice preference.
    if el.name == "preference" && el.attr("kind") == Some("choice") && !el.has_attr("options") {
        errors.push(BuildError::new(
            "<preference kind=\"choice\"> requires attribute `options` (#5849 §4.4.1)",
            el.span.line,
            el.span.col,
        ));
    }
}

/// The assertion half of stage 2: the `assert` slot checked
/// against the #5850 vocabulary (consumed as data from
/// [`crate::assertion_vocabulary`]). Unknown dimensions and tokens
/// fail with suggestions, in the #5849 idiom; raw values fail
/// under #5850 invariant 1; a dimension asserted twice in one
/// slot fails under #5854 §2.2. The slot does not apply to a
/// `<raw>` region: raw content is verbatim authored source, not
/// a rendered registry element (#5849 §3.1), so an assertion
/// there could never be honoured — it is an error, not a silent
/// drop.
fn check_assert_slot(el: &Element, errors: &mut Vec<BuildError>) {
    if !el.has_attr("assert") {
        return;
    }
    if el.name == "raw" {
        errors.push(BuildError::new(
            "the `assert` slot does not apply to a <raw> region: raw content is \
             verbatim authored source, not a rendered element (#5849 §3.1)",
            el.span.line,
            el.span.col,
        ));
        return;
    }
    let Some(value) = el.attr("assert") else {
        errors.push(BuildError::new(
            format!("attribute `assert` on <{}> requires a value", el.name),
            el.span.line,
            el.span.col,
        ));
        return;
    };
    if let Err(problems) = assertion_vocabulary::parse_assert_slot(value) {
        for problem in problems {
            errors.push(BuildError::new(problem, el.span.line, el.span.col));
        }
    }
}

fn is_sole_section(siblings: &[&Node]) -> bool {
    siblings
        .iter()
        .filter(|n| matches!(n, Node::Element(e) if e.name == "section"))
        .count()
        <= 1
}

fn check_enums(el: &Element, spec: &ElementSpec, errors: &mut Vec<BuildError>) {
    for (attr, allowed) in spec.enums {
        if let Some(value) = el.attr(attr) {
            if !allowed.contains(&value) {
                errors.push(BuildError::new(
                    format!(
                        "attribute `{attr}` on <{}> must be one of [{}], found \"{value}\" (#5849 §4)",
                        el.name,
                        allowed.join(", ")
                    ),
                    el.span.line,
                    el.span.col,
                ));
            }
        }
    }
    if el.name == "heading" {
        match el.attr("level") {
            Some(level) => {
                let ok = level.parse::<u32>().is_ok_and(|n| (1..=6).contains(&n));
                if !ok {
                    errors.push(BuildError::new(
                        format!(
                            "<heading> level must be an integer 1-6, found \"{level}\" (#5849 §4.1)"
                        ),
                        el.span.line,
                        el.span.col,
                    ));
                }
            }
            None => {}
        }
    }
}

/// #5849 §4.3.1/§4.4.1: a constraint attribute applies only to the
/// field/preference kinds listed for it.
fn check_constraint_applicability(el: &Element, errors: &mut Vec<BuildError>) {
    if el.name != "field" && el.name != "preference" {
        return;
    }
    let Some(kind) = el.attr("kind") else {
        return;
    };
    for (constraint, kinds) in FIELD_CONSTRAINT_KINDS {
        if !el.has_attr(constraint) {
            continue;
        }
        // Preference kinds reuse the field constraint set minus the
        // password-only kinds (§4.4.1): for a preference, min-length
        // and max-length apply to `text` only, min/max to `number`.
        let applies = kinds.contains(&kind);
        if !applies {
            errors.push(BuildError::new(
                format!(
                    "constraint `{constraint}` does not apply to <{} kind=\"{kind}\"> (#5849 §4.3.1)",
                    el.name
                ),
                el.span.line,
                el.span.col,
            ));
        }
    }
}

/// The child and sibling rules of #5849 §4.5, plus the parent rules
/// the registry tables imply (an `item` exists only inside a `list`,
/// and so on). A violation names the element, its parent, and the
/// allowed children.
fn check_child_rules(el: &Element, parent: Option<&str>, errors: &mut Vec<BuildError>) {
    // Parent rules for this element itself.
    let parent_rule: Option<(&str, &str)> = match el.name.as_str() {
        "page" => Some(("document", "<page> belongs directly under <document>")),
        "item" => Some(("list", "<item> belongs inside a <list>")),
        "tab" => Some(("tabs", "<tab> belongs inside <tabs>")),
        "menu-item" => Some(("menu", "<menu-item> belongs inside a <menu>")),
        "preference" => Some(("settings", "<preference> belongs inside <settings>")),
        "detail" => Some(("master-detail", "<detail> belongs inside <master-detail>")),
        "row" => Some(("table", "<row> belongs inside a <table>")),
        "cell" => Some(("row", "<cell> belongs inside a <row>")),
        "strong" | "emphasis" => Some(("text", "inline emphasis belongs inside <text>")),
        _ => None,
    };
    if let Some((want, hint)) = parent_rule {
        if parent != Some(want) {
            errors.push(BuildError::new(
                format!(
                    "{hint}, found parent <{}> (#5849 §4.5)",
                    parent.unwrap_or("(none)")
                ),
                el.span.line,
                el.span.col,
            ));
        }
    }

    // Allowed-children rules for this element's element children.
    let allowed: Option<&[&str]> = match el.name.as_str() {
        "document" => Some(&["page"]),
        "tabs" => Some(&["tab"]),
        "list" => Some(&["item"]),
        "menu" => Some(&["menu-item"]),
        "settings" => Some(&["preference"]),
        "toolbar" => Some(&["button", "menu"]),
        "form" => Some(&["field", "alert", "text", "button"]),
        "text" => Some(&["strong", "emphasis", "link"]),
        "table" => Some(&["row"]),
        "master-detail" => Some(&["list", "detail"]),
        _ => None,
    };
    let children = effective_children(el);
    if let Some(allowed) = allowed {
        for child in &children {
            if let Node::Element(child_el) = child {
                if !allowed.contains(&child_el.name.as_str()) {
                    errors.push(BuildError::new(
                        format!(
                            "<{}> may not contain <{}>; allowed children: {} (#5849 §4.5)",
                            el.name,
                            child_el.name,
                            allowed.join(", ")
                        ),
                        child_el.span.line,
                        child_el.span.col,
                    ));
                }
            }
        }
    }

    // master-detail contains exactly one list and exactly one detail.
    if el.name == "master-detail" {
        let lists = children
            .iter()
            .filter(|n| matches!(n, Node::Element(e) if e.name == "list"))
            .count();
        let details = children
            .iter()
            .filter(|n| matches!(n, Node::Element(e) if e.name == "detail"))
            .count();
        if lists != 1 || details != 1 {
            errors.push(BuildError::new(
                format!(
                    "<master-detail> contains exactly one <list> and exactly one <detail>; found {lists} list(s) and {details} detail(s) (#5849 §4.5)"
                ),
                el.span.line,
                el.span.col,
            ));
        }
    }
}

/// A binding-slot value is a name: dot-separated segments of letters,
/// digits, `_`, and `-`, each starting with a letter or `_` (#5849
/// §8 rule 1). `save-account` and `account.errors.name` are names;
/// `len(name) > 3` is an expression and a build error.
fn is_name_path(value: &str) -> bool {
    !value.is_empty()
        && value.split('.').all(|seg| {
            let mut chars = seg.chars();
            match chars.next() {
                Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
                _ => return false,
            }
            chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        })
}
